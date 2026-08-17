/**
 * Manual end-to-end smoke harness for harbor-cloud — a faithful port of
 * `server/scripts/ws_smoke.py`, retargeted at the local Cloudflare Worker.
 *
 *   1. Start the Worker:   npx wrangler dev            (serves :8787)
 *   2. Run this script:    node scripts/ws_smoke.mjs
 *
 * Registers A + B, pairs them, then exercises the happy path the two real
 * Tauri clients rely on: online chat with a delivered ack, offline buffering
 * with a not-delivered ack, and reconnect flush + late ack. Prints PASS/FAIL
 * per step and a final ALL PASS / FAILED line. Exits 0 on success, 1 on failure.
 *
 * Plain ESM JS (no TS, no deps) so it runs on Node ≥18 with the global fetch /
 * WebSocket it ships — no ts-node, tsx, or flags. Override the target with
 * HARBOR_SMOKE_HTTP / HARBOR_SMOKE_WS.
 *
 * Runs against wrangler dev's in-memory Durable Objects (HARBOR_OFFLINE_GRACE_MS
 * etc. from wrangler.jsonc apply). The 30s offline-grace window means A receives
 * NO offline push when B closes mid-test — unlike the FastAPI relay, which
 * pushed offline instantly — so the harness never contends with an incidental
 * presence-between-ack sequence on A's side, only on B's reconnect (A then sees
 * the online ping followed by the late ack, which the ack-skip handles).
 */

// Override the relay URL from the shell if needed; otherwise wrangler dev.
const BASE = process.env.HARBOR_SMOKE_HTTP ?? "http://localhost:8787";
const WS = process.env.HARBOR_SMOKE_WS ?? "ws://localhost:8787";

async function post(path, body) {
  const res = await fetch(BASE + path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  return await res.json();
}

/**
 * Per-socket inbound buffer. Node's raw EventTarget does NOT reliably deliver
 * a `message` whose listener was transiently removed across awaits (the event
 * can dispatch to nothing mid-`recv` and be lost). We attach ONE permanent
 * collector at open time that queues every frame; `recv`/`waitFor`/`drain` then
 * read from the queue (or wait for the next push). Nothing is ever dropped.
 */
const buffers = new WeakMap();

/** Resolve on `open`, reject on `error`. Attaches the permanent collector. */
function openSocket(url) {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url);
    const queue = [];
    const waiters = [];
    buffers.set(ws, { queue, waiters });
    ws.addEventListener("message", (ev) => {
      let msg;
      try {
        msg = JSON.parse(ev.data);
      } catch {
        return;
      }
      const w = waiters.shift();
      if (w) {
        clearTimeout(w.timer);
        w.resolve(msg);
      } else {
        queue.push(msg);
      }
    });
    ws.addEventListener("open", () => resolve(ws), { once: true });
    ws.addEventListener("error", () => reject(new Error("ws open error")), { once: true });
  });
}

/** Resolve to the next decoded message (buffered or future), or reject past `ms`.
 *  A timeout splices its own waiter out so a later message never resolves a
 *  dead promise and starves the next real consumer. */
function recv(ws, ms = 5000) {
  const buf = buffers.get(ws);
  if (buf.queue.length) return Promise.resolve(buf.queue.shift());
  return new Promise((resolve, reject) => {
    const entry = { resolve, timer: undefined };
    entry.timer = setTimeout(() => {
      const i = buf.waiters.indexOf(entry);
      if (i >= 0) buf.waiters.splice(i, 1);
      reject(new Error("recv timeout"));
    }, ms);
    buf.waiters.push(entry);
  });
}

/** Wait for a message whose `match` is true, skipping incidental frames
 *  (connect-time presence pings). Rejects on timeout. */
async function waitFor(ws, match, ms = 5000) {
  for (;;) {
    const m = await recv(ws, ms);
    if (match(m)) return m;
  }
}

/** Absorb every buffered frame, plus frames that arrive within `ms` of the last
 *  one (connect-time pings cluster and exit fast; a quiet socket returns fast). */
async function drain(ws, ms = 400) {
  const buf = buffers.get(ws);
  while (buf.queue.length) buf.queue.shift();
  for (;;) {
    try {
      await recv(ws, ms);
    } catch {
      return;
    }
  }
}

function closeQuiet(ws) {
  try {
    ws.close();
  } catch {
    /* already closed */
  }
}

let failures = 0;
function step(name, ok) {
  console.log(`[${ok ? "PASS" : "FAIL"}] ${name}`);
  if (!ok) failures++;
}

async function run() {
  const stamp = `${Date.now()}`;
  const aid = `smoke-a-${stamp}`;
  const bid = `smoke-b-${stamp}`;
  const a = await post("/register", { device_id: aid });
  const b = await post("/register", { device_id: bid });
  const aSec = a.device_secret;
  const bSec = b.device_secret;
  await post("/pair", {
    device_id: bid,
    device_secret: bSec,
    partner_code: a.pairing_code,
  });

  // Two live sockets. drain absorbs the connect-time online pings.
  const wa = await openSocket(`${WS}/ws?device_id=${aid}&secret=${aSec}`);
  const wb = await openSocket(`${WS}/ws?device_id=${bid}&secret=${bSec}`);
  await drain(wa);
  await drain(wb);

  // [1/3] online chat -> delivered ack.
  wa.send(JSON.stringify({ type: "chat", id: "m1", text: "hi" }));
  const chatB = await waitFor(wb, (m) => m.type === "chat" && m.id === "m1");
  const ackA1 = await waitFor(wa, (m) => m.type === "ack" && m.id === "m1");
  step("[1/3] online chat delivered + ack delivered:true", chatB.text === "hi" && ackA1.delivered === true);

  // [2/3] B offline -> A's chat is buffered, ack delivered:false.
  wb.close();
  wa.send(JSON.stringify({ type: "chat", id: "m2", text: "while you were away" }));
  const ackA2 = await waitFor(wa, (m) => m.type === "ack" && m.id === "m2");
  step("[2/3] offline chat buffered, ack delivered:false", ackA2.delivered === false);

  // [3/3] B reconnects -> buffered m2 flushed + late ack back to A.
  const wb2 = await openSocket(`${WS}/ws?device_id=${bid}&secret=${bSec}`);
  const chatB2 = await waitFor(wb2, (m) => m.type === "chat" && m.id === "m2");
  const lateAckA2 = await waitFor(wa, (m) => m.type === "ack" && m.id === "m2" && m.delivered === true);
  step("[3/3] offline buffer flushed on reconnect", chatB2.text === "while you were away");
  step("[3/3] late ack delivered:true on reconnect", lateAckA2.delivered === true);

  closeQuiet(wa);
  closeQuiet(wb2);

  console.log(failures === 0 ? "\nALL PASS" : `\n${failures} step(s) FAILED`);
  return failures === 0 ? 0 : 1;
}

run()
  .then((code) => {
    if (code !== 0) process.exit(code);
  })
  .catch((e) => {
    console.error("smoke crashed:", e instanceof Error ? e.message : String(e));
    process.exit(1);
  });
