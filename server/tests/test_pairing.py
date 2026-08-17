"""Tests for code generation, register, pair, profile, and partner lookup."""
from __future__ import annotations

import re

import pytest

from app import db, pairing

CODE_RE = re.compile(r"^HARBOR-[A-Z2-9]{4}-[A-Z2-9]{4}$")

# The non-prefix groups use the safe alphabet (no 0, O, 1, I). HARBOR itself is
# the brand prefix and legitimately contains 'O', so check the random part only.
_GROUP_RE = re.compile(r"^[A-Z2-9]{4}$")


@pytest.fixture
async def fresh_db(tmp_path, monkeypatch):
    monkeypatch.setattr("app.config.DB_PATH", str(tmp_path / "t.db"))
    await db.init_db()
    yield


def test_code_format():
    for _ in range(200):
        c = pairing.generate_code()
        assert CODE_RE.match(c), c
        _, g1, g2 = c.split("-")
        for g in (g1, g2):
            assert _GROUP_RE.match(g), g
            for ch in "01OI":
                assert ch not in g


async def test_register_issues_codes_and_secrets(fresh_db):
    a_code, a_secret = await pairing.register_device("dev-a")
    b_code, b_secret = await pairing.register_device("dev-b")
    assert CODE_RE.match(a_code) and CODE_RE.match(b_code)
    assert a_code != b_code
    assert a_secret and b_secret and a_secret != b_secret


async def test_register_is_idempotent_for_device_id(fresh_db):
    code1, _ = await pairing.register_device("dev-a")
    code2, _ = await pairing.register_device("dev-a")
    # Re-registering the same device re-issues a fresh code.
    assert code1 != code2


async def test_pair_links_and_retires_codes(fresh_db):
    a_code, _ = await pairing.register_device("dev-a")
    _, b_secret = await pairing.register_device("dev-b")
    res = await pairing.pair("dev-b", b_secret, a_code)
    assert res["partner_device_id"] == "dev-a"
    async with db.connect() as conn:
        ad = await db.get_device(conn, "dev-a")
        bd = await db.get_device(conn, "dev-b")
    assert ad["pairing_code"] is None and bd["pairing_code"] is None
    assert ad["partner_id"] == "dev-b" and bd["partner_id"] == "dev-a"


async def test_pair_wrong_secret(fresh_db):
    a_code, _ = await pairing.register_device("dev-a")
    await pairing.register_device("dev-b")
    with pytest.raises(pairing.PairError) as ei:
        await pairing.pair("dev-b", "wrong-secret", a_code)
    assert ei.value.status == 401


async def test_pair_unknown_caller(fresh_db):
    a_code, _ = await pairing.register_device("dev-a")
    with pytest.raises(pairing.PairError) as ei:
        await pairing.pair("ghost", "whatever", a_code)
    assert ei.value.status == 401


async def test_pair_invalid_code(fresh_db):
    _, b_secret = await pairing.register_device("dev-b")
    with pytest.raises(pairing.PairError) as ei:
        await pairing.pair("dev-b", b_secret, "HARBOR-AAAA-AAAA")
    assert ei.value.status == 404


async def test_pair_self_rejected(fresh_db):
    a_code, a_secret = await pairing.register_device("dev-a")
    with pytest.raises(pairing.PairError):
        await pairing.pair("dev-a", a_secret, a_code)


async def test_code_is_single_use(fresh_db):
    a_code, _ = await pairing.register_device("dev-a")
    _, b_secret = await pairing.register_device("dev-b")
    await pairing.pair("dev-b", b_secret, a_code)
    # A third device cannot reuse the retired code.
    _, c_secret = await pairing.register_device("dev-c")
    with pytest.raises(pairing.PairError) as ei:
        await pairing.pair("dev-c", c_secret, a_code)
    assert ei.value.status == 404


async def test_set_profile_and_get_partner_unauthorized(fresh_db):
    await pairing.register_device("dev-a")
    with pytest.raises(pairing.PairError) as ei:
        await pairing.get_partner("dev-a", "wrong")
    assert ei.value.status == 401


async def test_set_profile_then_get_partner(fresh_db):
    a_code, a_secret = await pairing.register_device("dev-a")
    _, b_secret = await pairing.register_device("dev-b")
    await pairing.set_profile("dev-a", a_secret, "Taylor")
    await pairing.pair("dev-b", b_secret, a_code)
    info = await pairing.get_partner("dev-b", b_secret)
    assert info["partner_device_id"] == "dev-a"
    assert info["partner_name"] == "Taylor"
    assert info["presence"] == "offline"


# --- unpair ---------------------------------------------------------------
# Unpairing is the "connect with someone else" path: it breaks the link on
# BOTH sides atomically and reissues a single-use code to each device, so each
# can pair again right away. A message buffered for the now-ex-partner must be
# dropped (delivering to an ex is wrong), and the bilateral clearing must be
# atomic (no half-unpaired state).


async def test_unpair_breaks_link_and_reissues_codes(fresh_db):
    a_code, a_secret = await pairing.register_device("dev-a")
    _, b_secret = await pairing.register_device("dev-b")
    await pairing.pair("dev-b", b_secret, a_code)

    my_code, partner_id, partner_code = await pairing.unpair("dev-b", b_secret)
    assert partner_id == "dev-a"
    # Both got a fresh, distinct, well-formed code.
    assert CODE_RE.match(my_code) and CODE_RE.match(partner_code)
    assert my_code != partner_code
    assert my_code != a_code  # differs from the original (now retired) code

    async with db.connect() as conn:
        ad = await db.get_device(conn, "dev-a")
        bd = await db.get_device(conn, "dev-b")
    assert ad["partner_id"] is None and bd["partner_id"] is None
    assert ad["pairing_code"] == partner_code and bd["pairing_code"] == my_code

    # Each can immediately pair with a fresh third device using the new codes.
    _, c_secret = await pairing.register_device("dev-c")
    res = await pairing.pair("dev-c", c_secret, my_code)
    assert res["partner_device_id"] == "dev-b"


async def test_unpair_wrong_secret(fresh_db):
    a_code, a_secret = await pairing.register_device("dev-a")
    _, b_secret = await pairing.register_device("dev-b")
    await pairing.pair("dev-b", b_secret, a_code)
    with pytest.raises(pairing.PairError) as ei:
        await pairing.unpair("dev-b", "wrong-secret")
    assert ei.value.status == 401
    # Failed auth leaves the pairing intact.
    async with db.connect() as conn:
        bd = await db.get_device(conn, "dev-b")
    assert bd["partner_id"] == "dev-a"


async def test_unpair_not_paired(fresh_db):
    _, a_secret = await pairing.register_device("dev-a")
    with pytest.raises(pairing.PairError) as ei:
        await pairing.unpair("dev-a", a_secret)
    assert ei.value.status == 404


async def test_unpair_clears_outbox(fresh_db):
    a_code, a_secret = await pairing.register_device("dev-a")
    _, b_secret = await pairing.register_device("dev-b")
    await pairing.pair("dev-b", b_secret, a_code)

    # Buffer a message as if it couldn't be delivered (b -> a).
    async with db.connect() as conn:
        await conn.execute(
            "INSERT INTO outbox(from_id, to_id, payload, ts) VALUES (?, ?, ?, ?)",
            ("dev-b", "dev-a", '{"type":"chat"}', 0.0),
        )
    await pairing.unpair("dev-a", a_secret)
    async with db.connect() as conn:
        async with conn.execute("SELECT COUNT(*) AS n FROM outbox") as cur:
            row = await cur.fetchone()
    assert row["n"] == 0


# --- get_me ----------------------------------------------------------------


async def test_get_me_unauthorized(fresh_db):
    await pairing.register_device("dev-a")
    with pytest.raises(pairing.PairError) as ei:
        await pairing.get_me("dev-a", "wrong")
    assert ei.value.status == 401


async def test_get_me_returns_current_state(fresh_db):
    a_code, a_secret = await pairing.register_device("dev-a")
    me = await pairing.get_me("dev-a", a_secret)
    assert me["pairing_code"] == a_code
    assert me["partner_id"] is None
    assert me["display_name"] is None

    # After pairing, partner_id reflects it; after unpairing, a fresh code returns.
    b_code, b_secret = await pairing.register_device("dev-b")
    await pairing.pair("dev-a", a_secret, b_code)
    me = await pairing.get_me("dev-a", a_secret)
    assert me["partner_id"] == "dev-b"
    assert me["pairing_code"] is None  # retired at pair time

    new_code, _, _ = await pairing.unpair("dev-a", a_secret)
    me = await pairing.get_me("dev-a", a_secret)
    assert me["partner_id"] is None
    assert me["pairing_code"] == new_code


# --- E2E public key exchange -----------------------------------------------
# Devices publish an X25519 public key at register (or later via /profile, the
# upgrade path). /pair hands the partner's pubkey back to the caller; the
# passive partner retrieves the caller's key via /partner. The relay stores
# ONLY public keys (never private), so this is E2E-safe.


async def test_register_with_public_key_persists(fresh_db):
    """A pubkey passed to register_device is stored on the device row."""
    await pairing.register_device("dev-a", public_key="PUBA")
    async with db.connect() as conn:
        ad = await db.get_device(conn, "dev-a")
    assert ad["public_key"] == "PUBA"
    # Re-registering the same device overwrites the key (lost-store recovery).
    await pairing.register_device("dev-a", public_key="PUBA2")
    async with db.connect() as conn:
        ad = await db.get_device(conn, "dev-a")
    assert ad["public_key"] == "PUBA2"


async def test_register_without_public_key_is_null(fresh_db):
    """Pre-E2E clients / scripts register without a pubkey — the column is NULL."""
    await pairing.register_device("dev-a")
    async with db.connect() as conn:
        ad = await db.get_device(conn, "dev-a")
    assert ad["public_key"] is None


async def test_pair_returns_partner_public_key(fresh_db):
    """The caller learns the partner's pubkey from the /pair response."""
    a_code, a_secret = await pairing.register_device("dev-a", public_key="PUBA")
    _, b_secret = await pairing.register_device("dev-b", public_key="PUBB")
    res = await pairing.pair("dev-b", b_secret, a_code)
    assert res["partner_device_id"] == "dev-a"
    assert res["partner_public_key"] == "PUBA"  # B learns A's key


async def test_pair_returns_null_partner_key_when_partner_lacks_one(fresh_db):
    """If the partner hasn't published a key yet, the caller gets None and
    must fall back to plaintext (visibly marked insecure on the client)."""
    a_code, _ = await pairing.register_device("dev-a")  # no pubkey
    _, b_secret = await pairing.register_device("dev-b", public_key="PUBB")
    res = await pairing.pair("dev-b", b_secret, a_code)
    assert res["partner_public_key"] is None


async def test_get_partner_returns_partner_public_key(fresh_db):
    """The passive partner retrieves the caller's pubkey via /partner — the
    path used on cold start, since only the caller hits /pair."""
    a_code, a_secret = await pairing.register_device("dev-a", public_key="PUBA")
    _, b_secret = await pairing.register_device("dev-b", public_key="PUBB")
    await pairing.pair("dev-b", b_secret, a_code)
    # B (the caller) -> get_partner returns A's key.
    info_b = await pairing.get_partner("dev-b", b_secret)
    assert info_b["partner_public_key"] == "PUBA"
    # A (the passive partner) -> get_partner returns B's key.
    info_a = await pairing.get_partner("dev-a", a_secret)
    assert info_a["partner_public_key"] == "PUBB"


async def test_set_profile_updates_public_key(fresh_db):
    """The upgrade-publish path: an already-paired client that predates E2E
    publishes its key via /profile (avoids re-/register's secret shuffle)."""
    a_code, a_secret = await pairing.register_device("dev-a")
    _, b_secret = await pairing.register_device("dev-b")
    await pairing.pair("dev-b", b_secret, a_code)
    # A publishes its pubkey after pairing, via set_profile.
    await pairing.set_profile("dev-a", a_secret, "Taylor", public_key="PUBA")
    async with db.connect() as conn:
        ad = await db.get_device(conn, "dev-a")
    assert ad["public_key"] == "PUBA"
    assert ad["display_name"] == "Taylor"
    # B can now retrieve A's freshly-published key.
    info = await pairing.get_partner("dev-b", b_secret)
    assert info["partner_public_key"] == "PUBA"
    assert info["partner_name"] == "Taylor"


# --- Profile photo (avatar) ------------------------------------------------
# Avatars follow the same publish/propagate contract as public keys: an optional
# base64 JPEG data URL published at register (or later via /profile, the upgrade
# path that keeps the device_secret stable), then handed to the partner via /pair
# (caller) and /partner (passive partner, cold start). An empty string "" clears
# the stored avatar (distinct from None/omitted = don't touch). The relay stores
# only the avatar string — like the pubkey, it is opaque bytes to the relay.

AVATAR_A = "data:image/jpeg;base64,/9j/4AAQSkLdrhEAAQAvatarA=="
AVATAR_B = "data:image/jpeg;base64,/9j/4AAQSkLdrhEAAQAvatarB=="


async def test_register_with_avatar_persists(fresh_db):
    """An avatar passed to register_device is stored on the device row."""
    await pairing.register_device("dev-a", avatar=AVATAR_A)
    async with db.connect() as conn:
        ad = await db.get_device(conn, "dev-a")
    assert ad["avatar"] == AVATAR_A
    # Re-registering the same device overwrites the avatar (lost-store recovery).
    await pairing.register_device("dev-a", avatar="data:image/jpeg;base64,/9j/4AAQNEW==")
    async with db.connect() as conn:
        ad = await db.get_device(conn, "dev-a")
    assert ad["avatar"] == "data:image/jpeg;base64,/9j/4AAQNEW=="


async def test_register_without_avatar_is_null(fresh_db):
    """Pre-avatar clients register without one — the column is NULL."""
    await pairing.register_device("dev-a")
    async with db.connect() as conn:
        ad = await db.get_device(conn, "dev-a")
    assert ad["avatar"] is None


async def test_pair_returns_partner_avatar(fresh_db):
    """The caller learns the partner's avatar from the /pair response."""
    a_code, _ = await pairing.register_device("dev-a", avatar=AVATAR_A)
    _, b_secret = await pairing.register_device("dev-b")
    res = await pairing.pair("dev-b", b_secret, a_code)
    assert res["partner_device_id"] == "dev-a"
    assert res["partner_avatar"] == AVATAR_A


async def test_pair_returns_null_partner_avatar_when_partner_lacks_one(fresh_db):
    """If the partner hasn't set an avatar yet, the caller gets None and the
    client falls back to the shark mascot."""
    a_code, _ = await pairing.register_device("dev-a")  # no avatar
    _, b_secret = await pairing.register_device("dev-b", avatar=AVATAR_B)
    res = await pairing.pair("dev-b", b_secret, a_code)
    assert res["partner_avatar"] is None


async def test_get_partner_returns_partner_avatar(fresh_db):
    """The passive partner retrieves the caller's avatar via /partner — the
    cold-start path, since only the caller hits /pair."""
    a_code, a_secret = await pairing.register_device("dev-a", avatar=AVATAR_A)
    _, b_secret = await pairing.register_device("dev-b", avatar=AVATAR_B)
    await pairing.pair("dev-b", b_secret, a_code)
    # B (the caller) -> get_partner returns A's avatar.
    info_b = await pairing.get_partner("dev-b", b_secret)
    assert info_b["partner_avatar"] == AVATAR_A
    # A (the passive partner) -> get_partner returns B's avatar.
    info_a = await pairing.get_partner("dev-a", a_secret)
    assert info_a["partner_avatar"] == AVATAR_B


async def test_set_profile_updates_avatar(fresh_db):
    """The upgrade-publish path: an already-paired client that predates avatars
    publishes its photo via /profile (avoids re-/register's secret shuffle)."""
    a_code, a_secret = await pairing.register_device("dev-a")
    _, b_secret = await pairing.register_device("dev-b")
    await pairing.pair("dev-b", b_secret, a_code)
    # A publishes its avatar after pairing, via set_profile.
    await pairing.set_profile("dev-a", a_secret, "Taylor", avatar=AVATAR_A)
    async with db.connect() as conn:
        ad = await db.get_device(conn, "dev-a")
    assert ad["avatar"] == AVATAR_A
    assert ad["display_name"] == "Taylor"
    # B can now retrieve A's freshly-published avatar.
    info = await pairing.get_partner("dev-b", b_secret)
    assert info["partner_avatar"] == AVATAR_A


async def test_set_profile_avatar_none_does_not_clear(fresh_db):
    """avatar=None means 'don't touch' — a previously set avatar survives an
    unrelated set_profile call that omits avatar. (Distinct from "" = clear.)"""
    a_code, a_secret = await pairing.register_device("dev-a", avatar=AVATAR_A)
    _, b_secret = await pairing.register_device("dev-b")
    await pairing.pair("dev-b", b_secret, a_code)
    # Change only the name, pass avatar=None — the stored avatar must persist.
    await pairing.set_profile("dev-a", a_secret, "Renamed")
    info = await pairing.get_partner("dev-b", b_secret)
    assert info["partner_avatar"] == AVATAR_A  # still set
    assert info["partner_name"] == "Renamed"


async def test_set_profile_avatar_empty_clears_it(fresh_db):
    """avatar='' clears the stored photo, mirroring the 'clear my photo' UI path."""
    a_code, a_secret = await pairing.register_device("dev-a", avatar=AVATAR_A)
    _, b_secret = await pairing.register_device("dev-b")
    await pairing.pair("dev-b", b_secret, a_code)
    await pairing.set_profile("dev-a", a_secret, "Taylor", avatar="")
    info = await pairing.get_partner("dev-b", b_secret)
    assert info["partner_avatar"] == ""  # cleared (empty string, not None)
