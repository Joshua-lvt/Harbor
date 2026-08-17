"""WebSocket endpoint and the two-device ConnectionManager.

Routing rule: a message from device A is forwarded only to A's partner
(looked up from the devices table). If the partner is offline, chat payloads
are buffered in the outbox and flushed when the partner reconnects. Presence
changes ('online'/'away'/'offline') are echoed to the partner; typing is
transient and never persisted.

Persistence is done inline in the WS loop (awaiting the aiosqlite write) so
ordering is preserved; BackgroundTasks would only be appropriate for
post-disconnect cleanup, which here we do directly in the finally block.
"""
from __future__ import annotations

import json
import time
from typing import Any, Optional

from fastapi import APIRouter, Query, WebSocket, WebSocketDisconnect

from . import db, security

router = APIRouter()


class ConnectionManager:
    """Maps a device_id to its live WebSocket. Two-person routing, nothing more."""

    def __init__(self) -> None:
        self.active: dict[str, WebSocket] = {}

    def connect(self, device_id: str, ws: WebSocket) -> None:
        self.active[device_id] = ws

    def disconnect(self, device_id: str) -> None:
        self.active.pop(device_id, None)

    async def forward(self, device_id: str, payload: dict) -> bool:
        ws = self.active.get(device_id)
        if ws is None:
            return False
        try:
            await ws.send_text(json.dumps(payload))
            return True
        except Exception:
            # Socket is gone — drop it so presence logic can react.
            self.disconnect(device_id)
            return False


manager = ConnectionManager()


async def _set_presence(device_id: str, state: str) -> None:
    async with db.connect() as conn:
        await conn.execute(
            "UPDATE devices SET presence = ?, last_seen = ? WHERE id = ?",
            (state, time.time(), device_id),
        )


async def _touch_last_seen(device_id: str) -> None:
    """Refresh last_seen only — never touch `presence`.

    The client sends a heartbeat every 25s regardless of its online/away state
    (the heartbeat keeps the socket alive; idle-driven presence is a separate,
    explicit `presence` message). Forcing `presence='online'` here would stomp
    an `away` set by the client and corrupt /partner lookups on cold start.
    """
    async with db.connect() as conn:
        await conn.execute(
            "UPDATE devices SET last_seen = ? WHERE id = ?",
            (time.time(), device_id),
        )


async def _flush_outbox(device_id: str) -> None:
    """Deliver buffered messages addressed to this device, then delete them.

    Delivered rows are removed immediately (no chat history is retained by the
    relay — only undelivered payloads are buffered, with a startup TTL sweep).
    On successful delivery we also send a late `ack {delivered: true}` back to
    the ORIGINAL SENDER, so their client can flip the bubble from "sending" to
    "delivered" — otherwise a message sent while the partner was offline is
    genuinely delivered yet shows as perpetually sending on the sender's side.
    """
    async with db.connect() as conn:
        async with conn.execute(
            "SELECT id, from_id, to_id, payload FROM outbox "
            "WHERE to_id = ? AND delivered = 0 ORDER BY ts ASC",
            (device_id,),
        ) as cur:
            rows = await cur.fetchall()
        for r in rows:
            payload = json.loads(r["payload"])
            ok = await manager.forward(device_id, payload)
            if ok:
                await conn.execute("DELETE FROM outbox WHERE id = ?", (r["id"],))
                # Late-ack the original sender that the buffered message landed.
                if "id" in payload:
                    await manager.forward(
                        r["from_id"],
                        {"type": "ack", "id": payload["id"], "delivered": True},
                    )


async def _buffer(partner_id: str, sender_id: str, payload: dict, delivered: bool) -> None:
    if delivered:
        return
    async with db.connect() as conn:
        await conn.execute(
            "INSERT INTO outbox(from_id, to_id, payload, ts) VALUES (?, ?, ?, ?)",
            (sender_id, partner_id, json.dumps(payload), payload["ts"]),
        )


async def _handle(device_id: str, msg: dict) -> None:
    mtype = msg.get("type")

    if mtype == "heartbeat":
        await _touch_last_seen(device_id)
        return

    # Re-read partner each message so a pairing that happened mid-session routes correctly.
    async with db.connect() as conn:
        me = await db.get_device(conn, device_id)
    partner_id: Optional[str] = me["partner_id"] if me else None

    if mtype == "presence":
        state = msg.get("state")
        if state in ("online", "away"):
            await _set_presence(device_id, state)
            if partner_id:
                await manager.forward(
                    partner_id,
                    {"type": "presence", "device_id": device_id, "state": state, "ts": time.time()},
                )
        return

    if mtype == "typing":
        if partner_id:
            await manager.forward(
                partner_id,
                {"type": "typing", "device_id": device_id, "state": msg.get("state", "start"), "ts": time.time()},
            )
        return

    if mtype == "activity":
        # Foreground-app indicator (Discord-style "using …"). Transient and
        # forward-only — never persisted (no DB column), never buffered. The
        # receiver maps the exe to a friendly name + game client-side. When the
        # partner is offline the activity simply isn't shown; on reconnect the
        # sender's next poll re-advertises its current foreground app.
        if partner_id:
            await manager.forward(
                partner_id,
                {"type": "activity", "device_id": device_id, "app": msg.get("app"), "ts": time.time()},
            )
        return

    if mtype == "voice_signal":
        # WebRTC signaling (SDP offer/answer + ICE candidates) for the
        # permanent P2P voice call. Transient and forward-only — mirroring
        # `typing`: signaling only matters when both peers are live (audio
        # itself goes P2P, never through the relay), so we neither buffer nor
        # persist. The caller routes `kind` through untouched.
        kind = msg.get("kind")
        if partner_id and kind in ("offer", "answer", "ice"):
            await manager.forward(
                partner_id,
                {"type": "voice_signal", "device_id": device_id, "kind": kind, "data": msg.get("data"), "ts": time.time()},
            )
        return

    if mtype == "chat":
        mid = msg.get("id") or f"{int(time.time() * 1000)}"
        ts = time.time()
        # Two wire shapes, mutually exclusive. With an `enc` payload the relay
        # is FULLY key-blind: it forwards an opaque base64 sealed-box string
        # (see the client's lib/crypto.ts — libsodium crypto_box_seal) and
        # never sees the text/image structure it wraps, so there is no content
        # validation at all. Without `enc`, the legacy plaintext path runs
        # (text + an optional `image` data URL, prefix-validated as before)
        # for scripts (ws_smoke/solo_partner), pre-E2E installs, and any
        # partner who hasn't published a public key yet.
        enc = msg.get("enc")
        if isinstance(enc, str) and enc:
            payload: dict[str, Any] = {
                "type": "chat",
                "id": mid,
                "from": device_id,
                "enc": enc,
                "ts": ts,
            }
        else:
            payload = {
                "type": "chat",
                "id": mid,
                "from": device_id,
                "text": str(msg.get("text", "")),
                "ts": ts,
            }
            # Optional image attachment: a small compressed JPEG as a base64 data
            # URL (no binary transport here — attachments ride inside the JSON
            # envelope). Pass it through verbatim; the client compresses before
            # sending so this stays a few tens of KB. (Plaintext path only —
            # E2E images are sealed inside `enc` along with the text.)
            image = msg.get("image")
            if isinstance(image, str) and image.startswith("data:image/"):
                payload["image"] = image
        if partner_id:
            delivered = await manager.forward(partner_id, payload)
            await _buffer(partner_id, device_id, payload, delivered)
            await manager.forward(device_id, {"type": "ack", "id": mid, "delivered": delivered})
        else:
            # No partner linked yet — nothing to route or buffer.
            await manager.forward(device_id, {"type": "ack", "id": mid, "delivered": False})
        return

    if mtype == "last_seen":
        if partner_id:
            async with db.connect() as conn:
                p = await db.get_device(conn, partner_id)
            await manager.forward(
                device_id,
                {
                    "type": "last_seen",
                    "device_id": partner_id,
                    "last_seen": p["last_seen"] if p else None,
                    "presence": p["presence"] if p else "offline",
                },
            )
        return


@router.websocket("/ws")
async def ws_endpoint(
    websocket: WebSocket,
    device_id: str = Query(...),
    secret: str = Query(...),
):
    async with db.connect() as conn:
        dev = await db.get_device(conn, device_id)
    if dev is None or not security.verify_secret(dev["secret"], secret):
        # Accept-then-close (closing before accept is not supported by Starlette).
        await websocket.accept()
        await websocket.close(code=4401, reason="unauthorized")
        return

    await websocket.accept()
    manager.connect(device_id, websocket)
    partner_id = dev["partner_id"]
    await _set_presence(device_id, "online")
    if partner_id:
        await manager.forward(
            partner_id,
            {"type": "presence", "device_id": device_id, "state": "online", "ts": time.time()},
        )
    await _flush_outbox(device_id)

    try:
        while True:
            raw = await websocket.receive_text()
            try:
                msg = json.loads(raw)
            except (json.JSONDecodeError, TypeError):
                continue
            if isinstance(msg, dict):
                try:
                    await _handle(device_id, msg)
                except Exception:
                    # A single bad message must not kill the socket.
                    pass
    except WebSocketDisconnect:
        pass
    except Exception:
        pass
    finally:
        manager.disconnect(device_id)
        await _set_presence(device_id, "offline")
        if partner_id:
            await manager.forward(
                partner_id,
                {
                    "type": "presence",
                    "device_id": device_id,
                    "state": "offline",
                    "ts": time.time(),
                    "last_seen": time.time(),
                },
            )
