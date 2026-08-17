"""Tests for the WebSocket routing layer (ConnectionManager + _handle).

We exercise the handler directly with FakeWebSocket sockets rather than
spinning up a live uvicorn, so these are fast and synchronous-in-spirit.
"""
from __future__ import annotations

import json

import pytest

from app import db, pairing
from app.ws import _flush_outbox, _handle, manager


class FakeWS:
    def __init__(self) -> None:
        self.sent: list[str] = []

    async def accept(self) -> None:
        pass

    async def send_text(self, s: str) -> None:
        self.sent.append(s)

    async def close(self, code: int = 1000, reason: str = "") -> None:
        pass

    def last(self) -> dict:
        return json.loads(self.sent[-1]) if self.sent else {}


@pytest.fixture
async def paired(tmp_path, monkeypatch):
    monkeypatch.setattr("app.config.DB_PATH", str(tmp_path / "t.db"))
    await db.init_db()
    a_code, a_secret = await pairing.register_device("dev-a")
    _, b_secret = await pairing.register_device("dev-b")
    await pairing.set_profile("dev-a", a_secret, "Taylor")
    await pairing.pair("dev-b", b_secret, a_code)
    yield
    manager.active.clear()


async def _count_outbox(to_id: str | None = None) -> int:
    async with db.connect() as conn:
        if to_id is None:
            async with conn.execute("SELECT COUNT(*) c FROM outbox") as cur:
                row = await cur.fetchone()
        else:
            async with conn.execute("SELECT COUNT(*) c FROM outbox WHERE to_id = ?", (to_id,)) as cur:
                row = await cur.fetchone()
    return row["c"]


async def test_chat_delivered_when_partner_online(paired):
    fa, fb = FakeWS(), FakeWS()
    manager.active["dev-a"] = fa
    manager.active["dev-b"] = fb
    await _handle("dev-a", {"type": "chat", "id": "m1", "text": "hi"})
    partner_msg = fb.last()
    assert partner_msg["type"] == "chat" and partner_msg["text"] == "hi" and partner_msg["from"] == "dev-a"
    ack = fa.last()
    assert ack["type"] == "ack" and ack["id"] == "m1" and ack["delivered"] is True
    assert await _count_outbox() == 0


async def test_chat_buffered_when_partner_offline(paired):
    fa = FakeWS()
    manager.active["dev-a"] = fa
    await _handle("dev-a", {"type": "chat", "id": "m2", "text": "hello"})
    ack = fa.last()
    assert ack["type"] == "ack" and ack["id"] == "m2" and ack["delivered"] is False
    assert await _count_outbox("dev-b") == 1


async def test_outbox_flushed_on_reconnect(paired):
    fa = FakeWS()
    manager.active["dev-a"] = fa
    await _handle("dev-a", {"type": "chat", "id": "m3", "text": "hi"})
    assert await _count_outbox("dev-b") == 1
    fb = FakeWS()
    manager.active["dev-b"] = fb
    await _flush_outbox("dev-b")
    # B received the buffered chat...
    assert any(json.loads(s).get("type") == "chat" for s in fb.sent)
    assert await _count_outbox("dev-b") == 0
    # ...and A received a late ack that the offline message finally landed.
    late_ack = [json.loads(s) for s in fa.sent if json.loads(s).get("type") == "ack"]
    assert any(a["id"] == "m3" and a["delivered"] is True for a in late_ack), fa.sent


# --- Image attachments -------------------------------------------------------
# A small compressed JPEG rides inline in the `chat` envelope as a `data:image/`
# URL. The relay forwards/buffers it verbatim; these lock that the field
# survives both the live forward and the outbox round-trip, and that the
# relay-side validator drops a malformed (non-data-URL) image.
JPEG = "data:image/jpeg;base64,/9j/4AAQSkLdrhEAAQ=="


async def test_chat_image_forwarded_to_partner(paired):
    fa, fb = FakeWS(), FakeWS()
    manager.active["dev-a"] = fa
    manager.active["dev-b"] = fb
    await _handle("dev-a", {"type": "chat", "id": "img1", "text": "look", "image": JPEG})
    partner_msg = fb.last()
    assert (
        partner_msg["type"] == "chat"
        and partner_msg["text"] == "look"
        and partner_msg["from"] == "dev-a"
        and partner_msg["image"] == JPEG
    ), partner_msg
    ack = fa.last()
    assert ack["type"] == "ack" and ack["id"] == "img1" and ack["delivered"] is True
    assert await _count_outbox() == 0


async def test_chat_image_buffered_and_flushed_on_reconnect(paired):
    fa = FakeWS()
    manager.active["dev-a"] = fa
    # B offline → the image chat is buffered (must survive the JSON encode/decode).
    await _handle("dev-a", {"type": "chat", "id": "img2", "text": "", "image": JPEG})
    ack = fa.last()
    assert ack["type"] == "ack" and ack["id"] == "img2" and ack["delivered"] is False
    assert await _count_outbox("dev-b") == 1
    # B reconnects → flush delivers the payload with the image intact + late ack.
    fb = FakeWS()
    manager.active["dev-b"] = fb
    await _flush_outbox("dev-b")
    flushed = next(json.loads(s) for s in fb.sent if json.loads(s).get("type") == "chat")
    assert flushed["image"] == JPEG and flushed["text"] == "", flushed
    late_ack = [json.loads(s) for s in fa.sent if json.loads(s).get("type") == "ack"]
    assert any(a["id"] == "img2" and a["delivered"] is True for a in late_ack), fa.sent


async def test_chat_rejects_non_data_url_image(paired):
    fa, fb = FakeWS(), FakeWS()
    manager.active["dev-a"] = fa
    manager.active["dev-b"] = fb
    # A bogus `image` (not a data:image/* URL) is dropped by the relay validator;
    # the text still forwards, but no `image` key reaches the partner.
    await _handle("dev-a", {"type": "chat", "id": "bad1", "text": "hi", "image": "https://x/y.png"})
    partner_msg = fb.last()
    assert partner_msg["type"] == "chat" and partner_msg["text"] == "hi"
    assert "image" not in partner_msg, partner_msg


# --- End-to-end encrypted `enc` payloads ------------------------------------
# Under E2E, the wire chat envelope carries an opaque base64 `enc` string (the
# client seals text+image with libsodium crypto_box_seal). The relay is FULLY
# key-blind: it forwards `enc` verbatim with zero content validation and never
# sees the text/image structure inside. These lock that passthrough online +
# buffered (the outbox stores the `enc` JSON and replays it unchanged).


async def test_chat_enc_forwarded_to_partner(paired):
    fa, fb = FakeWS(), FakeWS()
    manager.active["dev-a"] = fa
    manager.active["dev-b"] = fb
    await _handle("dev-a", {"type": "chat", "id": "e1", "enc": "CIPHER123"})
    partner_msg = fb.last()
    # The relay forwards enc verbatim and must NOT inject a `text` key — the
    # encrypted payload is structurally opaque (no text/image leak).
    assert (
        partner_msg["type"] == "chat"
        and partner_msg["enc"] == "CIPHER123"
        and partner_msg["from"] == "dev-a"
    ), partner_msg
    assert "text" not in partner_msg, partner_msg
    assert "image" not in partner_msg, partner_msg
    ack = fa.last()
    assert ack["type"] == "ack" and ack["id"] == "e1" and ack["delivered"] is True
    assert await _count_outbox() == 0


async def test_chat_enc_buffered_and_flushed_on_reconnect(paired):
    fa = FakeWS()
    manager.active["dev-a"] = fa
    # B offline → the enc payload is buffered (must survive JSON encode/decode).
    await _handle("dev-a", {"type": "chat", "id": "e2", "enc": "CIPHER456"})
    ack = fa.last()
    assert ack["type"] == "ack" and ack["id"] == "e2" and ack["delivered"] is False
    assert await _count_outbox("dev-b") == 1
    # B reconnects → flush delivers the enc payload intact (verbatim) + late ack.
    fb = FakeWS()
    manager.active["dev-b"] = fb
    await _flush_outbox("dev-b")
    flushed = next(json.loads(s) for s in fb.sent if json.loads(s).get("type") == "chat")
    assert flushed["enc"] == "CIPHER456", flushed
    assert "text" not in flushed and "image" not in flushed, flushed
    late_ack = [json.loads(s) for s in fa.sent if json.loads(s).get("type") == "ack"]
    assert any(a["id"] == "e2" and a["delivered"] is True for a in late_ack), fa.sent


async def test_presence_forwarded_to_partner(paired):
    fa, fb = FakeWS(), FakeWS()
    manager.active["dev-a"] = fa
    manager.active["dev-b"] = fb
    await _handle("dev-a", {"type": "presence", "state": "away"})
    msg = fb.last()
    assert msg["type"] == "presence" and msg["state"] == "away" and msg["device_id"] == "dev-a"
    async with db.connect() as conn:
        a = await db.get_device(conn, "dev-a")
    assert a["presence"] == "away"


async def test_presence_ignores_unknown_state(paired):
    fa, fb = FakeWS(), FakeWS()
    manager.active["dev-a"] = fa
    manager.active["dev-b"] = fb
    await _handle("dev-a", {"type": "presence", "state": "warp"})
    assert fb.sent == []  # nothing forwarded for an invalid state


async def test_typing_forwarded(paired):
    fa, fb = FakeWS(), FakeWS()
    manager.active["dev-a"] = fa
    manager.active["dev-b"] = fb
    await _handle("dev-a", {"type": "typing", "state": "start"})
    msg = fb.last()
    assert msg["type"] == "typing" and msg["state"] == "start"


@pytest.fixture
async def unpaired(tmp_path, monkeypatch):
    monkeypatch.setattr("app.config.DB_PATH", str(tmp_path / "t.db"))
    await db.init_db()
    code, secret = await pairing.register_device("dev-solo")
    yield code, secret
    manager.active.clear()


async def test_unpaired_chat_acks_not_delivered_and_unbuffered(unpaired):
    fa = FakeWS()
    manager.active["dev-solo"] = fa
    await _handle("dev-solo", {"type": "chat", "id": "solo1", "text": "anyone there?"})
    ack = fa.last()
    assert ack["type"] == "ack" and ack["id"] == "solo1" and ack["delivered"] is False
    assert await _count_outbox() == 0


async def test_heartbeat_refreshes_last_seen_only(paired):
    fa = FakeWS()
    manager.active["dev-a"] = fa
    await _handle("dev-a", {"type": "heartbeat"})
    async with db.connect() as conn:
        a = await db.get_device(conn, "dev-a")
    # last_seen advances, but presence is NOT touched (the client owns online/away).
    assert a["last_seen"] is not None
    assert a["presence"] == "offline"


async def test_heartbeat_preserves_away(paired):
    fa, fb = FakeWS(), FakeWS()
    manager.active["dev-a"] = fa
    manager.active["dev-b"] = fb
    await _handle("dev-a", {"type": "presence", "state": "away"})
    await _handle("dev-a", {"type": "heartbeat"})
    async with db.connect() as conn:
        a = await db.get_device(conn, "dev-a")
    assert a["presence"] == "away"
    assert a["last_seen"] is not None
