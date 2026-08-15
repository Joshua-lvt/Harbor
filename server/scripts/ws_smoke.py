"""Manual end-to-end smoke test for the relay — two fake devices over a live WS.

Run the relay first:  uvicorn app.main:app --port 8000
Then:                 python scripts/ws_smoke.py

Registers two devices, pairs them, opens both WebSockets, sends a chat A->B
(asserts delivery + ack), takes B offline, sends A->B again (asserts buffering),
reconnects B (asserts flushed delivery), and prints PASS/FAIL lines.

Requires the `websockets` library:  pip install websockets
"""
from __future__ import annotations

import asyncio
import json
import sys
import time
import urllib.request

import websockets

BASE = "http://localhost:8000"
WS = "ws://localhost:8000"


def post(path: str, body: dict) -> dict:
    req = urllib.request.Request(
        BASE + path,
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    with urllib.request.urlopen(req) as r:
        return json.loads(r.read())


async def drain(ws, timeout: float = 0.5) -> list[str]:
    """Best-effort drain of any buffered frames (e.g. initial presence pings).

    Presence is transient/best-effort on the relay — it is never buffered for
    an offline partner — so exactly one side receives the other's connect-time
    'online' ping (whichever device connected second; the first's ping was
    dropped because the partner wasn't online yet). We must tolerate either
    socket having zero or one buffered frames, hence tolerate TimeoutError.
    """
    msgs: list[str] = []
    while True:
        try:
            msgs.append(await asyncio.wait_for(ws.recv(), timeout=timeout))
        except asyncio.TimeoutError:
            break
    return msgs


async def run() -> int:
    aid = f"smoke-a-{int(time.time() * 1000)}"
    bid = f"smoke-b-{int(time.time() * 1000)}"
    a = post("/register", {"device_id": aid})
    b = post("/register", {"device_id": bid})
    post("/pair", {"device_id": bid, "device_secret": b["device_secret"], "partner_code": a["pairing_code"]})

    async with websockets.connect(f"{WS}/ws?device_id={aid}&secret={a['device_secret']}") as wa, \
               websockets.connect(f"{WS}/ws?device_id={bid}&secret={b['device_secret']}") as wb:
        # Drain any pre-existing connect-time presence pings (best-effort).
        await asyncio.gather(drain(wa), drain(wb))

        await wa.send(json.dumps({"type": "chat", "id": "m1", "text": "hi"}))
        got_b = json.loads(await asyncio.wait_for(wb.recv(), timeout=3))
        got_a = json.loads(await asyncio.wait_for(wa.recv(), timeout=3))
        assert got_b["type"] == "chat" and got_b["text"] == "hi", got_b
        assert got_a["type"] == "ack" and got_a["delivered"] is True, got_a
        print("[1/3] chat delivered online: PASS")

    # B offline: A sends, relay buffers (ack delivered=false)
    async with websockets.connect(f"{WS}/ws?device_id={aid}&secret={a['device_secret']}") as wa:
        await wa.send(json.dumps({"type": "chat", "id": "m2", "text": "while you were away"}))
        got_a = json.loads(await asyncio.wait_for(wa.recv(), timeout=3))
        assert got_a["type"] == "ack" and got_a["delivered"] is False, got_a
        print("[2/3] chat buffered offline: PASS")

        # B reconnects: the buffered m2 is flushed on connect.
        async with websockets.connect(f"{WS}/ws?device_id={bid}&secret={b['device_secret']}") as wb:
            delivered = False
            for _ in range(5):
                msg = json.loads(await asyncio.wait_for(wb.recv(), timeout=3))
                if msg.get("type") == "chat" and msg.get("id") == "m2":
                    delivered = True
                    break
            assert delivered, "buffered m2 not delivered on reconnect"
            print("[3/3] offline buffer flushed on reconnect: PASS")
    print("\nALL PASS")
    return 0


if __name__ == "__main__":
    sys.exit(asyncio.run(run()))
