/**
 * HarborPair WebSocket integration tests.
 *
 * Exercises the WS Hibernation surface (`src/pair.ts`) end-to-end through the
 * real Worker router (`src/index.ts`) running in a Miniflare isolate:
 * connect/presence, chat online ack + offline buffer + reconnect flush + late
 * ack, the grace-window offline push, voice signaling, activity, typing,
 * malformed/oversized frames (socket kept), the unpaired push, duplicate
 * connection (close-old-accept-new), and heartbeat-touches-last_seen-only.
 *
 * The grace-window behavior is tested deterministically with
 * `runDurableObjectAlarm(stub)`, which fires a scheduled DO alarm immediately
 * instead of the real 30s wait — so the tests are fast and non-flaky. The DO's
 * `alarm()` is idempotent and branches on its stored intent, so a manual fire
 * behaves exactly like a real one.
 */
import { afterEach, describe, expect, it } from "vitest";
import { SELF, reset, runDurableObjectAlarm, runInDurableObject } from "cloudflare:test";
import { env } from "cloudflare:workers";
import { pairKey } from "../src/util";

afterEach(async () => {
  await reset();
});

const BASE = "http://harbor.test";
const newId = () =>
  typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `dev-${Math.random().toString(36).slice(2)}-${Math.random().toString(36).slice(2)}`;

/* ─────────────────────── HTTP + WS helpers ──────────────────────── */

async function post(path: string, body: unknown) {
  const res = await SELF.fetch(BASE + path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  return { status: res.status, body: (await res.json()) as Record<string, unknown> };
}

async function get(path: string) {
  const res = await SELF.fetch(BASE + path);
  return { status: res.status, body: (await res.json()) as Record<string, unknown> };
}

/** Register two devices, pair them (B presents A's code), and return their ids +
 *  secrets + the deterministic pair_key (used to address the HarborPair DO). */
async function makePair(): Promise<{
  aid: string;
  bid: string;
  aSec: string;
  bSec: string;
  pk: string;
}> {
  const aid = newId();
  const bid = newId();
  const a = await post("/register", { device_id: aid });
  const b = await post("/register", { device_id: bid });
  await post("/pair", {
    device_id: bid,
    device_secret: b.body.device_secret as string,
    partner_code: a.body.pairing_code as string,
  });
  return {
    aid,
    bid,
    aSec: a.body.device_secret as string,
    bSec: b.body.device_secret as string,
    pk: pairKey(aid, bid),
  };
}

/**
 * Per-socket inbound buffer. Messages (and the close code) that arrive as
 * connect-time side effects — presence-on-connect, outbox flush on reconnect,
 * the grace-window offline push, the `unpaired` push — land BEFORE the test
 * attaches a transient listener, which would drop them. We attach a PERMANENT
 * collector at `openWs` time so nothing is ever lost; `recvJson`/`drain` then
 * drain the buffer (or wait for the next pushed frame).
 */
interface WsState {
  queue: Record<string, unknown>[];
  waiters: Array<{ resolve: (m: Record<string, unknown>) => void; timer: ReturnType<typeof setTimeout> }>;
  closeCode: number | null;
  closeWaiters: Array<{ resolve: (c: number) => void; timer: ReturnType<typeof setTimeout> }>;
}
const states = new WeakMap<WebSocket, WsState>();

/** Upgrade to a WS at /ws, accept the client end, and attach the permanent buffer. */
async function openWs(deviceId: string, secret: string): Promise<WebSocket> {
  const res = await SELF.fetch(`${BASE}/ws?device_id=${deviceId}&secret=${secret}`, {
    headers: { upgrade: "websocket" },
  });
  if (res.status !== 101) throw new Error(`ws upgrade failed: ${res.status}`);
  const ws = res.webSocket as WebSocket;
  ws.accept();
  const state: WsState = { queue: [], waiters: [], closeCode: null, closeWaiters: [] };
  states.set(ws, state);
  ws.addEventListener("message", (ev) => {
    let msg: Record<string, unknown>;
    try {
      msg = JSON.parse((ev as MessageEvent).data as string) as Record<string, unknown>;
    } catch {
      return; // ignore unparseable frames at the buffer layer
    }
    const w = state.waiters.shift();
    if (w) {
      clearTimeout(w.timer);
      w.resolve(msg);
    } else {
      state.queue.push(msg);
    }
  });
  ws.addEventListener("close", (ev) => {
    state.closeCode = (ev as CloseEvent).code;
    for (const w of state.waiters) clearTimeout(w.timer);
    state.waiters = [];
    for (const cw of state.closeWaiters) {
      clearTimeout(cw.timer);
      cw.resolve(state.closeCode);
    }
    state.closeWaiters = [];
  });
  return ws;
}

/** Resolve to the next decoded JSON message (buffered or future), or reject past `ms`.
 *  A timeout removes its own waiter entry so a later message never resolves a
 *  dead (already-rejected) promise and starves the next real consumer. */
function recvJson(ws: WebSocket, ms = 3000): Promise<Record<string, unknown>> {
  const state = states.get(ws);
  if (!state) throw new Error("recv on unbuffered socket");
  if (state.queue.length) return Promise.resolve(state.queue.shift() as Record<string, unknown>);
  if (state.closeCode !== null) return Promise.reject(new Error("socket closed"));
  return new Promise((resolve, reject) => {
    const entry: { resolve: (m: Record<string, unknown>) => void; timer: ReturnType<typeof setTimeout> } = {
      resolve,
      timer: undefined as unknown as ReturnType<typeof setTimeout>,
    };
    entry.timer = setTimeout(() => {
      const i = state.waiters.indexOf(entry);
      if (i >= 0) state.waiters.splice(i, 1);
      reject(new Error("recv timeout"));
    }, ms);
    state.waiters.push(entry);
  });
}

/** Collect every already-buffered message, plus (if any) frames that arrive
 *  within `ms` of the LAST received one — then return them. Returns instantly
 *  with whatever is buffered if nothing new lands within `ms` after the last. */
async function drain(ws: WebSocket, ms = 400): Promise<Record<string, unknown>[]> {
  const out: Record<string, unknown>[] = [];
  const state = states.get(ws);
  if (!state) throw new Error("drain on unbuffered socket");
  // First, flush anything already queued (instant).
  while (state.queue.length) out.push(state.queue.shift() as Record<string, unknown>);
  // Then keep grabbing frames that arrive within `ms` of the previous one. The
  // first MISS (timeout) ends the drain — so connect-time pings cluster and
  // exit fast, while a quiet socket returns immediately.
  for (;;) {
    try {
      out.push(await recvJson(ws, ms));
    } catch {
      return out;
    }
  }
}

/** Wait for no message to arrive within `ms` (negative assertion helper). */
async function assertNoMessage(ws: WebSocket, ms = 500): Promise<void> {
  await expect(drain(ws, ms)).resolves.toEqual([]);
}

/** Resolve to the close code, or 0 if the socket didn't close within `ms`. */
function onClose(ws: WebSocket, ms = 3000): Promise<number> {
  const state = states.get(ws);
  if (!state) throw new Error("onClose on unbuffered socket");
  if (state.closeCode !== null) return Promise.resolve(state.closeCode);
  return new Promise((resolve) => {
    const timer = setTimeout(() => resolve(0), ms);
    state.closeWaiters.push({ resolve, timer });
  });
}

/** The HarborPair DO stub for a pair_key (drives runDurableObjectAlarm). */
const pairStub = (pk: string) => env.HARBOR_PAIR.get(env.HARBOR_PAIR.idFromName(pk));

/* ─────────────────────────── tests ─────────────────────────── */

describe("WS /ws handshake", () => {
  it("rejects an upgrade with a bad secret (no member push)", async () => {
    const { aid } = await makePair();
    const res = await SELF.fetch(`${BASE}/ws?device_id=${aid}&secret=wrong`, {
      headers: { upgrade: "websocket" },
    });
    // Worker returns 401 before forwarding the upgrade (no 101).
    expect(res.status).toBe(401);
  });

  it("accepts an authenticated device and keeps the socket open", async () => {
    const { aid, aSec } = await makePair();
    const ws = await openWs(aid, aSec);
    // Send a heartbeat — the DO touches last_seen; nothing comes back. A round
    // trip without throwing confirms the socket is live and accepted.
    ws.send(JSON.stringify({ type: "heartbeat" }));
    await assertNoMessage(ws, 300);
    ws.close();
  });
});

describe("presence online (#7)", () => {
  it("B connecting after A forwards B's online presence to A", async () => {
    const { aid, bid, aSec, bSec } = await makePair();
    const wa = await openWs(aid, aSec);
    const wb = await openWs(bid, bSec);
    // drain A: it should have received B's connect-time online ping.
    const aMsgs = await drain(wa, 400);
    const pres = aMsgs.find((m) => m.type === "presence");
    expect(pres).toBeTruthy();
    expect(pres?.state).toBe("online");
    expect(pres?.device_id).toBe(bid);
    wb.close();
    wa.close();
  });
});

describe("presence offline + grace window (#8)", () => {
  it("B closing does NOT immediately push offline to A (suppressed until the grace alarm)", async () => {
    const { aid, bid, aSec, bSec, pk } = await makePair();
    const wa = await openWs(aid, aSec);
    const wb = await openWs(bid, bSec);
    await drain(wa, 300); // B's online ping to A

    wb.close();
    // Within the 30s grace window, A must not receive an offline push yet.
    await assertNoMessage(wa, 400);
    wa.close();
    void pk; // pair_key used in the next test
  });

  it("firing the grace alarm pushes offline to A with last_seen", async () => {
    const { aid, bid, aSec, bSec, pk } = await makePair();
    const wa = await openWs(aid, aSec);
    const wb = await openWs(bid, bSec);
    await drain(wa, 300);

    wb.close();
    await new Promise((r) => setTimeout(r, 100)); // let close settle
    // The DO scheduled the grace 30s out. runDurableObjectAlarm fires it at the
    // real "now", so backdate the member's pending grace into the past first —
    // a faithful simulation of "the grace window has elapsed".
    await runInDurableObject(pairStub(pk), (_inst, state) => {
      state.storage.sql.exec(
        "UPDATE members SET pending_grace_until = ? WHERE device_id = ?",
        Date.now() / 1000 - 60,
        bid,
      );
    });

    const ran = await runDurableObjectAlarm(pairStub(pk));
    expect(ran).toBe(true);

    const off = await recvJson(wa, 3000);
    expect(off.type).toBe("presence");
    expect(off.state).toBe("offline");
    expect(off.device_id).toBe(bid);
    expect(off.last_seen).not.toBeUndefined();
    wa.close();
  });

  it("B reconnecting before the grace fires cancels the offline flap (#9 reconnect)", async () => {
    const { aid, bid, aSec, bSec, pk } = await makePair();
    const wa = await openWs(aid, aSec);
    const wb = await openWs(bid, bSec);
    await drain(wa, 300);

    wb.close();
    // B reconnects within the grace window.
    const wb2 = await openWs(bid, bSec);
    await drain(wa, 300); // B's new online ping to A

    // Now fire whatever alarm is pending — the grace is cancelled (member
    // re-online), so A must NOT receive an offline push.
    await runDurableObjectAlarm(pairStub(pk));
    await assertNoMessage(wa, 400);
    wb2.close();
    wa.close();
  });
});

describe("chat forwarding + ack + offline buffer (#9, #10)", () => {
  it("delivers an encrypted `enc` chat verbatim (no text/image injected) + ack to sender", async () => {
    const { aid, bid, aSec, bSec } = await makePair();
    const wa = await openWs(aid, aSec);
    const wb = await openWs(bid, bSec);
    await drain(wa, 300);
    await drain(wb, 300);

    wa.send(JSON.stringify({ type: "chat", id: "m1", enc: "SEALED-BOX-BASE64" }));
    const gotB = await recvJson(wb, 1000);
    const gotA = await recvJson(wa, 1000);

    expect(gotB.type).toBe("chat");
    expect(gotB.id).toBe("m1");
    expect(gotB.from).toBe(aid);
    expect(gotB.enc).toBe("SEALED-BOX-BASE64");
    expect("text" in gotB).toBe(false);
    expect("image" in gotB).toBe(false);

    expect(gotA.type).toBe("ack");
    expect(gotA.id).toBe("m1");
    expect(gotA.delivered).toBe(true);
    wb.close();
    wa.close();
  });

  it("forwards a plaintext chat with a data:image/ attachment", async () => {
    const { aid, bid, aSec, bSec } = await makePair();
    const wa = await openWs(aid, aSec);
    const wb = await openWs(bid, bSec);
    await drain(wa, 300);
    await drain(wb, 300);

    wa.send(
      JSON.stringify({
        type: "chat",
        id: "m2",
        text: "look",
        image: "data:image/jpeg;base64,AAAA",
      }),
    );
    const gotB = await recvJson(wb, 1000);
    await recvJson(wa, 1000); // ack
    expect(gotB.text).toBe("look");
    expect(gotB.image).toBe("data:image/jpeg;base64,AAAA");
    wb.close();
    wa.close();
  });

  it("buffers chat to an offline partner, then flushes + late-acks on reconnect", async () => {
    const { aid, bid, aSec, bSec } = await makePair();
    const wa = await openWs(aid, aSec);
    // B is offline. A sends → buffer, ack delivered:false.
    wa.send(JSON.stringify({ type: "chat", id: "m3", enc: "BUF-ENC" }));
    const ack = await recvJson(wa, 1000);
    expect(ack.type).toBe("ack");
    expect(ack.delivered).toBe(false);

    // B reconnects → flush delivers the buffered chat, then the late ack to A.
    const wb = await openWs(bid, bSec);
    const flushed = await recvJson(wb, 3000);
    expect(flushed.type).toBe("chat");
    expect(flushed.id).toBe("m3");
    expect(flushed.enc).toBe("BUF-ENC");
    expect(flushed.from).toBe(aid);

    // A receives B's reconnect online ping FIRST, then the late ack for m3 —
    // read until the ack lands (the ping may precede it).
    let lateAck: Record<string, unknown> | undefined;
    for (let i = 0; i < 5 && !lateAck; i++) {
      const m = await recvJson(wa, 3000);
      if (m.type === "ack" && m.id === "m3") lateAck = m;
    }
    expect(lateAck).toBeDefined();
    expect(lateAck?.delivered).toBe(true);
    wb.close();
    wa.close();
  });
});

describe("transient forwards — voice / activity / typing (#11–14)", () => {
  it("forwards each voice_signal kind and never buffers (partner online)", async () => {
    const { aid, bid, aSec, bSec } = await makePair();
    const wa = await openWs(aid, aSec);
    const wb = await openWs(bid, bSec);
    await drain(wa, 300);
    await drain(wb, 300);

    for (const kind of ["offer", "answer", "ice"] as const) {
      wa.send(JSON.stringify({ type: "voice_signal", kind, data: `sdp-${kind}` }));
      const got = await recvJson(wb, 1000);
      expect(got.type).toBe("voice_signal");
      expect(got.kind).toBe(kind);
      expect(got.device_id).toBe(aid);
    }
    // Nothing routed back to A for signaling.
    await assertNoMessage(wa, 300);
    wb.close();
    wa.close();
  });

  it("forwards an activity event (transient, not persisted)", async () => {
    const { aid, bid, aSec, bSec } = await makePair();
    const wa = await openWs(aid, aSec);
    const wb = await openWs(bid, bSec);
    await drain(wa, 300);
    await drain(wb, 300);

    wa.send(JSON.stringify({ type: "activity", app: "discord.exe" }));
    const got = await recvJson(wb, 1000);
    expect(got.type).toBe("activity");
    expect(got.app).toBe("discord.exe");
    expect(got.device_id).toBe(aid);
    wb.close();
    wa.close();
  });

  it("forwards a profile_update push to the partner (additive, verbatim)", async () => {
    const { aid, bid, aSec, bSec } = await makePair();
    const wa = await openWs(aid, aSec);
    const wb = await openWs(bid, bSec);
    await drain(wa, 300);
    await drain(wb, 300);

    wa.send(
      JSON.stringify({
        type: "profile_update",
        display_name: "Taylor",
        avatar: "data:image/jpeg;base64,AAAA",
      }),
    );
    const got = await recvJson(wb, 1000);
    expect(got.type).toBe("profile_update");
    expect(got.display_name).toBe("Taylor");
    expect(got.avatar).toBe("data:image/jpeg;base64,AAAA");
    expect(got.device_id).toBe(aid);
    expect(typeof got.ts).toBe("number");
    // Nothing routed back to the sender.
    await assertNoMessage(wa, 300);
    wb.close();
    wa.close();
  });

  it("forwards an activity_icon push to the partner (additive, verbatim)", async () => {
    const { aid, bid, aSec, bSec } = await makePair();
    const wa = await openWs(aid, aSec);
    const wb = await openWs(bid, bSec);
    await drain(wa, 300);
    await drain(wb, 300);

    wa.send(
      JSON.stringify({
        type: "activity_icon",
        app: "chrome.exe",
        icon: "data:image/png;base64,iVBOR",
      }),
    );
    const got = await recvJson(wb, 1000);
    expect(got.type).toBe("activity_icon");
    expect(got.app).toBe("chrome.exe");
    expect(got.icon).toBe("data:image/png;base64,iVBOR");
    expect(got.device_id).toBe(aid);
    expect(typeof got.ts).toBe("number");
    await assertNoMessage(wa, 300);
    wb.close();
    wa.close();
  });

  it("forwards an activity_icon push with a null icon (fallback confirm)", async () => {
    const { aid, bid, aSec, bSec } = await makePair();
    const wa = await openWs(aid, aSec);
    const wb = await openWs(bid, bSec);
    await drain(wa, 300);
    await drain(wb, 300);

    wa.send(JSON.stringify({ type: "activity_icon", app: "obscure.exe", icon: null }));
    const got = await recvJson(wb, 1000);
    expect(got.type).toBe("activity_icon");
    expect(got.app).toBe("obscure.exe");
    expect(got.icon).toBeNull();
    wb.close();
    wa.close();
  });

  it("forwards a typing start event, defaulting a missing state to start", async () => {
    const { aid, bid, aSec, bSec } = await makePair();
    const wa = await openWs(aid, aSec);
    const wb = await openWs(bid, bSec);
    await drain(wa, 300);
    await drain(wb, 300);

    wa.send(JSON.stringify({ type: "typing" })); // no state → defaults "start"
    const got = await recvJson(wb, 1000);
    expect(got.type).toBe("typing");
    expect(got.state).toBe("start");
    expect(got.device_id).toBe(aid);
    wb.close();
    wa.close();
  });
});

describe("resilience — malformed, oversized, heartbeat (#15, extras)", () => {
  it("replies {type:error} for an unknown-but-valid message type and keeps the socket", async () => {
    const { aid, bid, aSec, bSec } = await makePair();
    const wa = await openWs(aid, aSec);
    const wb = await openWs(bid, bSec);
    await drain(wa, 300);
    await drain(wb, 300);

    wa.send(JSON.stringify({ type: "bogus" }));
    const err = await recvJson(wa, 1000);
    expect(err.type).toBe("error");
    expect(typeof err.reason).toBe("string");

    // Socket is still alive — a subsequent chat round-trips normally.
    wa.send(JSON.stringify({ type: "chat", id: "after", enc: "x" }));
    const ack = await recvJson(wa, 1000);
    expect(ack.type).toBe("ack");
    expect(ack.id).toBe("after");
    wb.close();
    wa.close();
  });

  it("silently drops invalid-JSON frames (faithful to ws.py:266)", async () => {
    const { aid, bid, aSec, bSec } = await makePair();
    const wa = await openWs(aid, aSec);
    const wb = await openWs(bid, bSec);
    await drain(wa, 300);
    await drain(wb, 300);

    // Two garbage frames, then a real chat — the chat must still land.
    wa.send("not json{");
    wa.send("");
    wa.send(JSON.stringify({ type: "chat", id: "after-garbage", enc: "y" }));
    const ack = await recvJson(wa, 1000);
    expect(ack.type).toBe("ack");
    expect(ack.id).toBe("after-garbage");
    wb.close();
    wa.close();
  });

  it("rejects an oversized frame with {type:error} and keeps the socket", async () => {
    const { aid, bid, aSec, bSec } = await makePair();
    const wa = await openWs(aid, aSec);
    const wb = await openWs(bid, bSec);
    await drain(wa, 300);
    await drain(wb, 300);

    // A string longer than HARBOR_MAX_FRAME_BYTES (256 KB). Send as a chat
    // whose `enc` is oversized.
    const big = "x".repeat(300_000);
    wa.send(JSON.stringify({ type: "chat", id: "big", enc: big }));
    const err = await recvJson(wa, 3000);
    expect(err.type).toBe("error");
    expect(err.reason).toBe("frame_too_large");

    // Socket survived: a normal chat still round-trips.
    wa.send(JSON.stringify({ type: "chat", id: "after-big", enc: "z" }));
    const ack = await recvJson(wa, 1000);
    expect(ack.type).toBe("ack");
    expect(ack.id).toBe("after-big");
    wb.close();
    wa.close();
  });

  it("heartbeat touches last_seen only — no presence push to the partner", async () => {
    const { aid, bid, aSec, bSec } = await makePair();
    const wa = await openWs(aid, aSec);
    const wb = await openWs(bid, bSec);
    await drain(wa, 300);
    await drain(wb, 300);

    // A heartbeats; the partner must not see a presence change from it.
    wa.send(JSON.stringify({ type: "heartbeat" }));
    await assertNoMessage(wb, 400);

    // And /partner still reads A as online (presence unchanged by the beat).
    const partner = await get(`/partner?device_id=${bid}&secret=${bSec}`);
    expect(partner.body.presence).toBe("online");
    wb.close();
    wa.close();
  });
});

describe("duplicate connection — close-old-accept-new (#6)", () => {
  it("the newest connection wins: the old socket is closed (4409), chat routes to the new one", async () => {
    const { aid, bid, aSec, bSec } = await makePair();
    const wb = await openWs(bid, bSec);
    await drain(wb, 300); // B's online ping to A (A offline yet → dropped)

    // A opens a first socket, then a second (duplicate). The first should get
    // closed with 4409; the second stays and receives chat.
    const wa1 = await openWs(aid, aSec);
    const wa2 = await openWs(aid, aSec);

    // wa1 should receive a close (4409 duplicate_connection).
    const closed = await onClose(wa1, 3000);
    expect(closed).toBe(4409);

    // A chat from B must now route to wa2 (the surviving socket). wa2's own
    // connect-time presence ping went to B (its partner), so wa2's queue is
    // empty — no drain needed before sending.
    wb.send(JSON.stringify({ type: "chat", id: "dup", enc: "dup-enc" }));
    const got = await recvJson(wa2, 3000);
    expect(got.type).toBe("chat");
    expect(got.id).toBe("dup");
    wa2.close();
    wb.close();
  });
});

describe("last_seen query", () => {
  it("replies to the sender with the partner's presence + last_seen", async () => {
    const { aid, bid, aSec, bSec } = await makePair();
    const wa = await openWs(aid, aSec);
    const wb = await openWs(bid, bSec);
    await drain(wa, 300);
    await drain(wb, 300);

    wa.send(JSON.stringify({ type: "last_seen" }));
    const r = await recvJson(wa, 1000);
    expect(r.type).toBe("last_seen");
    expect(r.device_id).toBe(bid);
    expect(r.presence).toBe("online");
    expect(typeof r.last_seen).toBe("number");
    wb.close();
    wa.close();
  });
});

describe("unpair pushes {type:unpaired} to the live ex-partner", () => {
  it("B sees its own freshly-issued code arrive over WS when A unpairs", async () => {
    const { aid, bid, aSec, bSec } = await makePair();
    const wa = await openWs(aid, aSec);
    const wb = await openWs(bid, bSec);
    await drain(wa, 300);
    await drain(wb, 300);

    await post("/unpair", { device_id: aid, device_secret: aSec });
    const got = await recvJson(wb, 1000);
    expect(got.type).toBe("unpaired");
    expect(typeof got.pairing_code).toBe("string");
    expect(/^HARBOR-/.test(got.pairing_code as string)).toBe(true);
    // The code delivered to B matches the fresh code B now sees via /me.
    const me = await get(`/me?device_id=${bid}&secret=${bSec}`);
    expect(me.body.pairing_code).toBe(got.pairing_code);
    wb.close();
    wa.close();
  });
});

/* ─────────────────────── small local helpers ─────────────────────── */

// (helpers folded into the buffer layer above; this section kept for future
//  local utilities without polluting the test bodies.)
