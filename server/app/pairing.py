"""Pairing code generation and the register / pair / profile flows.

A device registers once (receiving a code to share + a secret to keep), then
links to its partner by presenting the partner's code. Codes are single-use:
both are retired the moment a pair succeeds. The WebSocket handshake then
authenticates with (device_id, device_secret) — a guessed device_id is useless
without the 256-bit secret.
"""
from __future__ import annotations

import secrets
import sqlite3
import time

from . import db, security

# Unambiguous alphabet: excludes 0, O, 1, I so codes are easy to read/share aloud.
SAFE_ALPHABET = "ABCDEFGHJKLMNPQRSTUVWXYZ23456789"
CODE_PREFIX = "HARBOR"


class PairError(Exception):
    """Business error carrying an HTTP-ish status code."""

    def __init__(self, detail: str, status: int = 400) -> None:
        self.detail = detail
        self.status = status
        super().__init__(detail)


def _rand_group(n: int = 4) -> str:
    return "".join(secrets.choice(SAFE_ALPHABET) for _ in range(n))


def generate_code() -> str:
    return f"{CODE_PREFIX}-{_rand_group()}-{_rand_group()}"


async def _fresh_code(conn) -> str:
    """Generate a pairing code not already outstanding in the DB.

    Used by register_device (via its own INSERT retry) and by unpair (which
    UPDATEs an existing row, so it must check the DB rather than rely on the
    UNIQUE constraint to surface a collision mid-transaction).
    """
    for _ in range(5):
        code = generate_code()
        if await db.find_by_code(conn, code) is None:
            return code
    raise PairError("could not allocate a unique code", 500)


async def register_device(
    device_id: str, public_key: str | None = None, avatar: str | None = None
) -> tuple[str, str]:
    """Create or refresh a device row; return a fresh pairing code + secret.

    Calling /register on an existing device re-issues its secret and code (the
    "I lost my local store" path); its partner link is preserved but the old
    secret stops authenticating. For the normal flow this is called once at
    first launch, with the client-generated X25519 public key so the relay can
    later hand it to a partner (see pair / get_partner). `avatar` is an optional
    base64 JPEG data URL (profile photo) published at register so a future
    partner can render it — optional so pre-avatar clients/scripts keep working.
    """
    secret = security.issue_secret()
    now = time.time()
    for _ in range(5):
        code = generate_code()
        try:
            async with db.connect() as conn:
                await conn.execute(
                    "INSERT INTO devices(id, secret, pairing_code, public_key, avatar, presence, created_at) "
                    "VALUES (?, ?, ?, ?, ?, 'offline', ?) "
                    "ON CONFLICT(id) DO UPDATE SET "
                    "  secret = excluded.secret, pairing_code = excluded.pairing_code, "
                    "  public_key = excluded.public_key, avatar = excluded.avatar",
                    (device_id, secret, code, public_key, avatar, now),
                )
            return code, secret
        except sqlite3.IntegrityError:
            # pairing_code collided with another device's outstanding code — retry.
            continue
    raise PairError("could not allocate a unique code", 500)


async def pair(caller_id: str, caller_secret: str, partner_code: str) -> dict:
    if not partner_code:
        raise PairError("missing partner code", 400)
    async with db.connect() as conn:
        caller = await db.get_device(conn, caller_id)
        if caller is None or not security.verify_secret(caller["secret"], caller_secret):
            raise PairError("unauthorized", 401)
        partner_id = await db.find_by_code(conn, partner_code)
        if partner_id is None:
            raise PairError("invalid or expired code", 404)
        if partner_id == caller_id:
            raise PairError("cannot pair with self", 400)
        # Retire both codes (single-use) and cross-link partners — atomic via commit.
        await conn.execute(
            "UPDATE devices SET pairing_code = NULL WHERE id IN (?, ?)",
            (caller_id, partner_id),
        )
        await conn.execute(
            "UPDATE devices SET partner_id = ? WHERE id = ?", (partner_id, caller_id)
        )
        await conn.execute(
            "UPDATE devices SET partner_id = ? WHERE id = ?", (caller_id, partner_id)
        )
        partner = await db.get_device(conn, partner_id)
    return {
        "partner_device_id": partner_id,
        "partner_name": partner["display_name"] if partner else None,
        "partner_public_key": partner["public_key"] if partner else None,
        "partner_avatar": partner["avatar"] if partner else None,
    }


async def set_profile(
    caller_id: str,
    caller_secret: str,
    display_name: str | None,
    public_key: str | None = None,
    avatar: str | None = None,
) -> dict:
    async with db.connect() as conn:
        me = await db.get_device(conn, caller_id)
        if me is None or not security.verify_secret(me["secret"], caller_secret):
            raise PairError("unauthorized", 401)
        if public_key is not None and avatar is not None:
            # Both keys + name in one update (avatar is a base64 data URL; an
            # empty string "" clears it, distinct from None = don't touch).
            await conn.execute(
                "UPDATE devices SET display_name = ?, public_key = ?, avatar = ? WHERE id = ?",
                (display_name, public_key, avatar, caller_id),
            )
        elif public_key is not None:
            # Publishing/updating the device's public key. This is the upgrade
            # path for already-paired installs that predate E2E (re-running
            # /register would re-issue the device_secret and force a WS
            # re-auth; /profile keeps the secret stable).
            await conn.execute(
                "UPDATE devices SET display_name = ?, public_key = ? WHERE id = ?",
                (display_name, public_key, caller_id),
            )
        elif avatar is not None:
            # Updating the avatar alone (empty string "" clears it).
            await conn.execute(
                "UPDATE devices SET display_name = ?, avatar = ? WHERE id = ?",
                (display_name, avatar, caller_id),
            )
        else:
            await conn.execute(
                "UPDATE devices SET display_name = ? WHERE id = ?", (display_name, caller_id)
            )
    return {"ok": True}


async def get_partner(caller_id: str, caller_secret: str) -> dict:
    async with db.connect() as conn:
        me = await db.get_device(conn, caller_id)
        if me is None or not security.verify_secret(me["secret"], caller_secret):
            raise PairError("unauthorized", 401)
        if not me["partner_id"]:
            raise PairError("not paired", 404)
        p = await db.get_device(conn, me["partner_id"])
        if p is None:
            raise PairError("partner not found", 404)
    return {
        "partner_device_id": p["id"],
        "partner_name": p["display_name"],
        "presence": p["presence"],
        "last_seen": p["last_seen"],
        "partner_public_key": p["public_key"],
        "partner_avatar": p["avatar"],
    }


async def get_me(caller_id: str, caller_secret: str) -> dict:
    """Return this device's own state — its current pairing code, partner link,
    and display name. Used on cold start to recover a fresh code after the
    partner unpaired us while we were offline (our local store still held the
    old partner_id, but the relay already broke the link and reissued a code).
    """
    async with db.connect() as conn:
        me = await db.get_device(conn, caller_id)
        if me is None or not security.verify_secret(me["secret"], caller_secret):
            raise PairError("unauthorized", 401)
    return {
        "pairing_code": me["pairing_code"],
        "partner_id": me["partner_id"],
        "display_name": me["display_name"],
    }


async def unpair(caller_id: str, caller_secret: str) -> tuple[str, str | None, str | None]:
    """Break the pairing from my side. Bilateral + atomic.

    - authenticate the caller (constant-time secret compare, like the other
      mutations);
    - 404 if the caller wasn't actually paired;
    - clear `partner_id` on BOTH devices and reissue a fresh single-use
      `pairing_code` to EACH (so both can pair with someone new right away —
      this is the "connect with another person" path);
    - drop any undelivered outbox rows between the two (the pairing is gone, so
      buffered messages to a now-ex-partner should never deliver later).

    Returns ``(my_new_code, partner_id, partner_new_code)``. The route layer uses
    ``partner_id`` + ``partner_new_code`` to notify the ex-partner in real time
    over the WebSocket (a ``{type: "unpaired", pairing_code}`` envelope) so their
    client returns to the pairing screen instead of clinging to a dead link.
    """
    async with db.connect() as conn:
        me = await db.get_device(conn, caller_id)
        if me is None or not security.verify_secret(me["secret"], caller_secret):
            raise PairError("unauthorized", 401)
        partner_id = me["partner_id"]
        if not partner_id:
            raise PairError("not paired", 404)

        code_me = await _fresh_code(conn)
        code_them = await _fresh_code(conn)
        # Clear the link + reissue codes for both, atomically.
        await conn.execute(
            "UPDATE devices SET partner_id = NULL, pairing_code = ? WHERE id = ?",
            (code_me, caller_id),
        )
        await conn.execute(
            "UPDATE devices SET partner_id = NULL, pairing_code = ? WHERE id = ?",
            (code_them, partner_id),
        )
        # Drop undelivered messages buffered between these two — the pairing is
        # gone, so a later flush to the now-ex-partner would be wrong.
        await conn.execute(
            "DELETE FROM outbox WHERE (from_id = ? AND to_id = ?) OR (from_id = ? AND to_id = ?)",
            (caller_id, partner_id, partner_id, caller_id),
        )
    return code_me, partner_id, code_them
