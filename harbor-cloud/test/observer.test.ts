/**
 * Harbor Mobile-observer integration tests.
 *
 * Exercises the V0.1 mobile flow end-to-end through the real Worker router
 * (`src/index.ts`): a mobile device binds as a receive-only OBSERVER of a PC via
 * `/mobile_code` + `/connect_mobile`, then watches the PC's `presence`/`activity`
 * over `/ws`. Core assertions:
 *
 *   - the observer receives the PC's activity/presence fan-out (BOTH to it and to a
 *     paired peer — non-regression of the existing peer route);
 *   - the observer is receive-only: sending presence/activity from it never reaches
 *     the PC or the peer, and never corrupts the PC's persisted presence;
 *   - a paired PC's partner link stays 1:1 (the observer never displaces the real
 *     partner);
 *   - offline (grace) reaches the observer just like it reaches a peer;
 *   - /connect_mobile is single-use (consumes the code) and unauthenticated bind
 *     attempts are rejected.
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
const MOBILE_CODE_RE = /^[A-Z2-9]{4}-[A-Z2-9]{4}$/;

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

/** Register two devices, pair them (B presents A's code), return ids/secrets/pk. */
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
  return { aid, bid, aSec: a.body.device_secret as string, bSec: b.body.device_secret as string, pk: pairKey(aid, bid) };
}

/** Register a mobile device and return its id + secret. */
async function makeMobile(): Promise<{ mid: string; mSec: string }> {
  const mid = newId();
  const r = await post("/register", { device_id: mid });
  return { mid, mSec: r.body.device_secret as string };
}

// ── WS buffer helpers (same protocol as pair.test.ts) ───────────────────────

interface WsState {
  queue: Record<string, unknown>[];
  waiters: Array<{ resolve: (m: Record<string, unknown>) => void; timer: ReturnType<typeof setTimeout> }>;
  closeCode: number | null;
  closeWaiters: Array<{ resolve: (c: number) => void; timer: ReturnType<typeof setTimeout> }>;
}
const states = new WeakMap<WebSocket, WsState>();

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
      return;
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

async function drain(ws: WebSocket, ms = 400): Promise<Record<string, unknown>[]> {
  const out: Record<string, unknown>[] = [];
  const state = states.get(ws);
  if (!state) throw new Error("drain on unbuffered socket");
  while (state.queue.length) out.push(state.queue.shift() as Record<string, unknown>);
  for (;;) {
    try {
      out.push(await recvJson(ws, ms));
    } catch {
      return out;
    }
  }
}

async function assertNoMessage(ws: WebSocket, ms = 500): Promise<void> {
  await expect(drain(ws, ms)).resolves.toEqual([]);
}

const pairStub = (pk: string) => env.HARBOR_PAIR.get(env.HARBOR_PAIR.idFromName(pk));

/* ─────────────────────────── tests ─────────────────────────── */

describe("POST /mobile_code", () => {
  it("mints a short readable mobile code for an authenticated caller", async () => {
    const aid = newId();
    const a = await post("/register", { device_id: aid });
    const r = await post("/mobile_code", {
      device_id: aid,
      device_secret: a.body.device_secret as string,
    });
    expect(r.status).toBe(200);
    expect(typeof r.body.mobile_code).toBe("string");
    expect(MOBILE_CODE_RE.test(r.body.mobile_code as string)).toBe(true);
    expect(typeof r.body.expires).toBe("number");
    expect(r.body.expires).toBeGreaterThan(Date.now() / 1000);
  });

  it("rotates the code on re-mint (single-use prior one invalidated)", async () => {
    const aid = newId();
    const a = await post("/register", { device_id: aid });
    const r1 = await post("/mobile_code", { device_id: aid, device_secret: a.body.device_secret as string });
    const r2 = await post("/mobile_code", { device_id: aid, device_secret: a.body.device_secret as string });
    expect(r2.body.mobile_code).not.toBe(r1.body.mobile_code);
    const { mid, mSec } = await makeMobile();
    const oldAttempt = await post("/connect_mobile", {
      device_id: mid,
      device_secret: mSec,
      mobile_code: r1.body.mobile_code as string,
    });
    expect(oldAttempt.status).toBe(404);
  });

  it("401s on a bad secret", async () => {
    const aid = newId();
    await post("/register", { device_id: aid });
    const r = await post("/mobile_code", { device_id: aid, device_secret: "nope" });
    expect(r.status).toBe(401);
  });
});

describe("observer fan-out — paired PC", () => {
  it("observer receives presence+activity from PC, AND so does the peer (non-regression)", async () => {
    const { aid, bid, aSec, bSec } = await makePair();
    const { mid, mSec } = await makeMobile();

    // PC A mints a code; mobile binds as its observer.
    const codeRes = await post("/mobile_code", { device_id: aid, device_secret: aSec });
    const mobileCode = codeRes.body.mobile_code as string;
    const bind = await post("/connect_mobile", { device_id: mid, device_secret: mSec, mobile_code: mobileCode });
    expect(bind.status).toBe(200);
    expect(bind.body.target_id).toBe(aid);

    const wm = await openWs(mid, mSec);
    await drain(wm, 200); // observer boots with a presence snapshot

    const wa = await openWs(aid, aSec);
    const wb = await openWs(bid, bSec);
    await drain(wm, 200); // A's connect online fan-out to the observer
    await drain(wa, 200);
    await drain(wb, 200);

    // A sends activity + presence-from-away.
    wa.send(JSON.stringify({ type: "presence", state: "away" }));
    wa.send(JSON.stringify({ type: "activity", app: "discord.exe" }));

    // Observer gets both.
    const mObserved = await drain(wm, 400);
    expect(mObserved.some((m) => m.type === "presence" && m.device_id === aid && m.state === "away")).toBe(true);
    expect(mObserved.some((m) => m.type === "activity" && m.device_id === aid && m.app === "discord.exe")).toBe(true);

    // Peer B also gets both (peer routing unchanged).
    const bObserved = await drain(wb, 400);
    expect(bObserved.some((m) => m.type === "activity" && m.device_id === aid && m.app === "discord.exe")).toBe(true);

    // The observer is scoped to A: B's activity must not leak through the pair.
    wb.send(JSON.stringify({ type: "activity", app: "private-b.exe" }));
    const noLeak = await drain(wm, 400);
    expect(noLeak.some((m) => m.type === "activity" && m.device_id === bid)).toBe(false);

    wm.close();
    wa.close();
    wb.close();
  });

  it("observer is receive-only: sending presence/activity from it never reaches A or B", async () => {
    const { aid, bid, aSec, bSec } = await makePair();
    const { mid, mSec } = await makeMobile();
    const codeRes = await post("/mobile_code", { device_id: aid, device_secret: aSec });
    await post("/connect_mobile", { device_id: mid, device_secret: mSec, mobile_code: codeRes.body.mobile_code as string });

    const wm = await openWs(mid, mSec);
    const wa = await openWs(aid, aSec);
    const wb = await openWs(bid, bSec);
    await drain(wm, 200);
    await drain(wa, 300);
    await drain(wb, 300);

    // The frame limit is checked before the observer branch, so oversized input
    // cannot be parsed or smuggled through the receive-only path.
    wm.send("x".repeat(262_145));
    const frameError = await recvJson(wm, 1000);
    expect(frameError).toMatchObject({ type: "error", reason: "frame_too_large" });

    // Mobile tries to push presence/activity — must be silently ignored.
    wm.send(JSON.stringify({ type: "presence", state: "away" }));
    wm.send(JSON.stringify({ type: "activity", app: "malware.exe" }));
    wm.send(JSON.stringify({ type: "chat", id: "x", text: "hi" }));
    await assertNoMessage(wa, 400);
    await assertNoMessage(wb, 400);

    // And the PC's persisted presence is untouched (still online from its connect).
    const partner = await get(`/partner?device_id=${bid}&secret=${bSec}`);
    expect(partner.body.presence).toBe("online");

    wm.close();
    wa.close();
    wb.close();
  });

  it("mobile observer does NOT displace the real partner (peer link stays 1:1)", async () => {
    const { aid, bid, aSec, bSec } = await makePair();
    const { mid, mSec } = await makeMobile();
    const codeRes = await post("/mobile_code", { device_id: aid, device_secret: aSec });
    await post("/connect_mobile", { device_id: mid, device_secret: mSec, mobile_code: codeRes.body.mobile_code as string });

    const wa = await openWs(aid, aSec);
    const wb = await openWs(bid, bSec);
    await drain(wa, 300);
    await drain(wb, 300);

    // A messages B, and B must still receive it (partner = B, not the observer).
    wa.send(JSON.stringify({ type: "activity", app: "chrome.exe" }));
    const got = await drain(wb, 400);
    expect(got.some((m) => m.type === "activity" && m.device_id === aid && m.app === "chrome.exe")).toBe(true);

    // /partner for B still points at A (not at the observer).
    const partner = await get(`/partner?device_id=${bid}&secret=${bSec}`);
    expect(partner.body.partner_device_id).toBe(aid);

    wb.close();
    wa.close();
  });
});

describe("observer lifecycle", () => {
  it("rehomes a solo observer when its PC later pairs with another PC", async () => {
    const aid = newId();
    const bid = newId();
    const a = await post("/register", { device_id: aid });
    const b = await post("/register", { device_id: bid });
    const { mid, mSec } = await makeMobile();

    const code = await post("/mobile_code", {
      device_id: aid,
      device_secret: a.body.device_secret as string,
    });
    await post("/connect_mobile", {
      device_id: mid,
      device_secret: mSec,
      mobile_code: code.body.mobile_code as string,
    });

    // A still has its normal pairing code; pairing must move the observer from
    // pairKey(A,A) to the new A:B Pair DO instead of orphaning the mobile.
    const paired = await post("/pair", {
      device_id: bid,
      device_secret: b.body.device_secret as string,
      partner_code: a.body.pairing_code as string,
    });
    expect(paired.status).toBe(200);

    const wm = await openWs(mid, mSec);
    const wa = await openWs(aid, a.body.device_secret as string);
    await drain(wm, 300);
    await drain(wa, 300);
    wa.send(JSON.stringify({ type: "activity", app: "after-pair.exe" }));
    const observed = await drain(wm, 500);
    expect(observed.some((m) => m.type === "activity" && m.device_id === aid && m.app === "after-pair.exe")).toBe(true);
    expect(observed.some((m) => m.device_id === bid)).toBe(false);
    wm.close();
    wa.close();
  });
});

describe("observer offline via grace", () => {
  it("PC dropping fires the grace alarm → observer receives offline, peer too", async () => {
    const { aid, bid, aSec, bSec, pk } = await makePair();
    const { mid, mSec } = await makeMobile();
    const codeRes = await post("/mobile_code", { device_id: aid, device_secret: aSec });
    await post("/connect_mobile", { device_id: mid, device_secret: mSec, mobile_code: codeRes.body.mobile_code as string });

    const wm = await openWs(mid, mSec);
    const wb = await openWs(bid, bSec);
    const wa = await openWs(aid, aSec);
    await drain(wm, 300);
    await drain(wb, 300);

    wa.close();
    await new Promise((r) => setTimeout(r, 100));
    // Backdate A's pending grace into the past, then fire the alarm.
    await runInDurableObject(pairStub(pk), (_inst, state) => {
      state.storage.sql.exec(
        "UPDATE members SET pending_grace_until = ? WHERE device_id = ?",
        Date.now() / 1000 - 60,
        aid,
      );
    });
    expect(await runDurableObjectAlarm(pairStub(pk))).toBe(true);

    // Observer sees A offline with last_seen.
    const mOff = await recvJson(wm, 3000);
    expect(mOff.type).toBe("presence");
    expect(mOff.device_id).toBe(aid);
    expect(mOff.state).toBe("offline");

    // Peer B sees it too.
    const bOff = await recvJson(wb, 3000);
    expect(bOff.type).toBe("presence");
    expect(bOff.device_id).toBe(aid);
    expect(bOff.state).toBe("offline");

    wm.close();
    wb.close();
  });
});

describe("/connect_mobile — single-use & auth", () => {
  it("retries the same mobile binding idempotently after a lost response", async () => {
    const { aid, aSec } = await makePair();
    const { mid, mSec } = await makeMobile();
    const codeRes = await post("/mobile_code", { device_id: aid, device_secret: aSec });
    const body = { device_id: mid, device_secret: mSec, mobile_code: codeRes.body.mobile_code as string };
    expect((await post("/connect_mobile", body)).status).toBe(200);
    const retry = await post("/connect_mobile", body);
    expect(retry.status).toBe(200);
    expect(retry.body.target_id).toBe(aid);
  });

  it("consumes the code: a second bind with a different mobile → 404", async () => {
    const { aid, aSec } = await makePair();
    const { mid, mSec } = await makeMobile();
    const codeRes = await post("/mobile_code", { device_id: aid, device_secret: aSec });
    const mobileCode = codeRes.body.mobile_code as string;

    const r1 = await post("/connect_mobile", { device_id: mid, device_secret: mSec, mobile_code: mobileCode });
    expect(r1.status).toBe(200);

    // A second mobile tries the same (now-consumed) code.
    const { mid: mid2, mSec: mSec2 } = await makeMobile();
    const r2 = await post("/connect_mobile", { device_id: mid2, device_secret: mSec2, mobile_code: mobileCode });
    expect(r2.status).toBe(404);
  });

  it("rejects a mobile that is already a PC peer", async () => {
    const { aid, aSec } = await makePair();
    const mid = newId();
    const otherId = newId();
    const mobile = await post("/register", { device_id: mid });
    const other = await post("/register", { device_id: otherId });
    const peerBind = await post("/pair", {
      device_id: otherId,
      device_secret: other.body.device_secret as string,
      partner_code: mobile.body.pairing_code as string,
    });
    expect(peerBind.status).toBe(200);
    const targetCode = await post("/mobile_code", { device_id: aid, device_secret: aSec });
    const result = await post("/connect_mobile", {
      device_id: mid,
      device_secret: mobile.body.device_secret as string,
      mobile_code: targetCode.body.mobile_code as string,
    });
    expect(result.status).toBe(409);
  });

  it("401s when the mobile presents a bad secret", async () => {
    const { aid, aSec } = await makePair();
    const mid = newId();
    await post("/register", { device_id: mid });
    const codeRes = await post("/mobile_code", { device_id: aid, device_secret: aSec });
    const r = await post("/connect_mobile", {
      device_id: mid,
      device_secret: "wrong",
      mobile_code: codeRes.body.mobile_code as string,
    });
    expect(r.status).toBe(401);
  });
});