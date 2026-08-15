"""Solo test harness — be the "partner" so one person can test Harbor alone.

Harbor pairs exactly two *distinct* devices, and the relay forbids pairing a
device with itself (`pairing.py`: "cannot pair with self"). Two `tauri dev`
windows on one machine also share the same Tauri store (`identity.json`) →
same `device_id` → same pairing code, so you can't open a second identity that
way either.

This script is the second identity. It is a real relay client:

  1. Registers a fresh solo device  →  prints a `HARBOR-XXXX-XXXX` code.
  2. Opens its WebSocket (relay now sees the device as `online`).
  3. You open the real Harbor app, paste THIS script's code, click Conectar.
     The app calls `/pair`; the relay cross-links both devices.
  4. Both sides go online. You chat from the app; replies typed here are sent
     to the app. Presence (online/away) and acks flow live in both directions.

So the actual Tauri/React client is exercised end-to-end on one side, and this
terminal plays the partner on the other — all by yourself.

E2E encryption note: under Harbor's end-to-end encryption, the real client seals
each chat body to the partner's X25519 public key (libsodium crypto_box_seal),
sending an opaque `enc` string the relay forwards verbatim. This harness registers
WITHOUT a public key, so the client falls back to sending it PLAINTEXT (visibly
marked insecure on the app side) — that is the intended "partner hasn't published
a key" fallback and is exactly what exercises the plaintext path live. It also
cannot decrypt an incoming sealed box (no shared key), so a `chat` carrying `enc`
prints as `[chat cifrado] <…truncated>`. This keeps the harness a relay +
presence + pairing smoke test; a full E2E chat round-trip needs two real clients
on two machines (a single box can't host two Tauri identities — same store, same
device_id). `ws_smoke.py` covers the relay's `enc` passthrough at the protocol
level without needing keys.

Run the relay first:
    server\\.venv\\Scripts\\python.exe -m uvicorn app.main:app --host 127.0.0.1 --port 8000 --reload
Then:
    server\\.venv\\Scripts\\python.exe scripts\\solo_partner.py
    (optional arg: host:port, default 127.0.0.1:8000)

Commands typed at the prompt:
    <text>           send a chat message to the app
    /away            announce presence "away" (app should show 🌙)
    /online          announce presence "online"
    /typing          send a typing-start event (then auto-stops after ~2s)
    /quit            close and exit
"""
from __future__ import annotations

import asyncio
import json
import sys
import time
import urllib.request

import websockets

DEFAULT_TARGET = "127.0.0.1:8000"


def post(base: str, path: str, body: dict) -> dict:
    req = urllib.request.Request(
        f"{base}{path}",
        data=json.dumps(body).encode(),
        headers={"Content-Type": "application/json"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(req, timeout=10) as r:
            return json.loads(r.read())
    except urllib.error.HTTPError as e:
        detail = e.read().decode(errors="replace")
        raise RuntimeError(f"POST {path} -> {e.code}: {detail}") from None


def describe(msg: dict) -> str:
    """Human-readable form of a server envelope, for printing in the terminal."""
    t = msg.get("type")
    if t == "presence":
        return f"[presenca] parceiro: {msg.get('state')}"
    if t == "chat":
        enc = msg.get("enc")
        if isinstance(enc, str) and enc:
            # Under E2E the real client seals chat to the partner's pubkey, so
            # what arrives here is an opaque base64 sealed-box string — this
            # harness has no matching key and cannot decrypt it. Print it
            # truncated; the relay/presence/pairing plumbing is still fully
            # exercisable, only the chat body is opaque (see header note above).
            shown = enc if len(enc) <= 24 else enc[:24] + "…"
            return f"[chat cifrado] {shown}  (id={msg.get('id')})"
        return f"[chat recebido] {msg.get('text')}  (id={msg.get('id')})"
    if t == "ack":
        state = "entregue" if msg.get("delivered") else "nao entregue (offline/ausente)"
        return f"[ack] {msg.get('id')}: {state}"
    if t == "typing":
        return f"[digitando] parceiro: {msg.get('state')}"
    if t == "last_seen":
        return f"[last_seen] parceiro: {msg.get('presence')} (last_seen={msg.get('last_seen')})"
    return f"[?] {msg}"


async def heartbeat(ws: websockets.WebSocketClientProtocol, stop: asyncio.Event) -> None:
    """Mirror the client's 25s heartbeat so the relay keeps us from timing out."""
    while not stop.is_set():
        try:
            await ws.send(json.dumps({"type": "heartbeat"}))
        except Exception:
            return
        try:
            await asyncio.wait_for(stop.wait(), timeout=25)
        except asyncio.TimeoutError:
            continue


async def recv_loop(ws: websockets.WebSocketClientProtocol, stop: asyncio.Event) -> None:
    while not stop.is_set():
        try:
            raw = await ws.recv()
        except websockets.ConnectionClosed:
            print("\n[conexao fechada]", file=sys.stderr)
            stop.set()
            return
        try:
            msg = json.loads(raw)
        except (json.JSONDecodeError, TypeError):
            continue
        print(describe(msg))
        # When the app comes online (i.e. it just paired + connected), nudge
        # it with our presence so its UI flips to "online" without waiting for
        # its next refresh. Cheap and event-driven, not a polling loop.
        if msg.get("type") == "presence" and msg.get("state") == "online":
            try:
                await ws.send(json.dumps({"type": "presence", "state": "online"}))
            except Exception:
                pass


async def stdin_loop(ws: websockets.WebSocketClientProtocol, stop: asyncio.Event) -> None:
    loop = asyncio.get_running_loop()
    while not stop.is_set():
        # input() blocks a thread, not the event loop.
        line = await loop.run_in_executor(None, sys.stdin.readline)
        if not line:
            # EOF (Ctrl-Z+Enter on Windows) — quit.
            stop.set()
            return
        cmd = line.strip()
        if not cmd:
            continue
        try:
            if cmd == "/quit":
                stop.set()
                return
            if cmd == "/away":
                await ws.send(json.dumps({"type": "presence", "state": "away"}))
                print("[voce] away")
                continue
            if cmd == "/online":
                await ws.send(json.dumps({"type": "presence", "state": "online"}))
                print("[voce] online")
                continue
            if cmd == "/typing":
                await ws.send(json.dumps({"type": "typing", "state": "start"}))
                print("[voce] digitando...")
                await asyncio.sleep(2)
                await ws.send(json.dumps({"type": "typing", "state": "stop"}))
                continue
            # Default: a chat message. Use a unique id so we can match the ack.
            mid = f"solo-{int(time.time() * 1000)}"
            await ws.send(json.dumps({"type": "chat", "id": mid, "text": cmd}))
            print(f"[voce enviado] {cmd}  (id={mid})")
        except websockets.ConnectionClosed:
            stop.set()
            return
        except Exception as e:
            print(f"[erro ao enviar] {e}", file=sys.stderr)


async def main(target: str) -> int:
    base = f"http://{target}"
    ws_url = f"ws://{target}/ws"

    device_id = f"solo-{int(time.time() * 1000)}"
    reg = post(base, "/register", {"device_id": device_id})
    code = reg["pairing_code"]
    secret = reg["device_secret"]

    print("=" * 60)
    print(" HARBOR - parceiro solo")
    print("=" * 60)
    print(f" Relay:        {base}")
    print(f" device_id:    {device_id}")
    print()
    print(f" SEU CODIGO:   {code}")
    print()
    print(" 1. Abra o Harbor (npm run tauri dev).")
    print(" 2. Na tela de pairing, cole o codigo acima em")
    print('    "Cole o codigo do parceiro" e clique Conectar.')
    print(" 3. Digite mensagens aqui para envia-las ao app.")
    print("    Comandos: /away  /online  /typing  /quit")
    print("=" * 60)
    print()

    stop = asyncio.Event()
    async with websockets.connect(f"{ws_url}?device_id={device_id}&secret={secret}") as ws:
        # Announce online so the relay stores presence (it does this on accept
        # too, but re-asserting is harmless and matches the client's behavior).
        await ws.send(json.dumps({"type": "presence", "state": "online"}))

        hb = asyncio.create_task(heartbeat(ws, stop))
        rx = asyncio.create_task(recv_loop(ws, stop))
        tx = asyncio.create_task(stdin_loop(ws, stop))

        # Finish when any one loop signals stop (Ctrl-C, /quit, EOF, or close).
        done, pending = await asyncio.wait(
            {asyncio.create_task(stop.wait()), hb, rx, tx},
            return_when=asyncio.FIRST_COMPLETED,
        )
        stop.set()
        for t in pending:
            t.cancel()
    return 0


if __name__ == "__main__":
    target = sys.argv[1] if len(sys.argv) > 1 else DEFAULT_TARGET
    try:
        raise SystemExit(asyncio.run(main(target)))
    except KeyboardInterrupt:
        print("\n[saindo]")
        raise SystemExit(0)
