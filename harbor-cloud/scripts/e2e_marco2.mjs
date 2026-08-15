/**
 * MARCO 2 — manual E2E smoke against the LIVE local Worker (ws://localhost:8787).
 *
 * Drives the REAL pairing + E2E flow that the two Tauri clients run, with REAL
 * libsodium crypto (crypto_box_seal / crypto_box_seal_open) — no mock, no Tauri:
 *
 *   1. Register A + B, each publishing a real X25519 pubkey (libsodium genKeypair).
 *   2. Active pair: B POSTs /pair with A's code → learns A's pubkey (partner_public_key).
 *   3. Passive detect: A GETs /me (sees partner_id=B) → GETs /partner → learns B's pubkey.
 *      Both sides now hold own keypair + partner_pubkey = ChatScreen's `encrypted` flag.
 *   4. Open two live WS. A→B and B→A: seal { text } to partner's pubkey, send the
 *      `{type:"chat",id,enc}` frame (the E2E wire shape — NO text/image). B opens the
 *      received `enc` with its own keypair → recovers the plaintext. Acks delivered:true.
 *      We INSPECT every forwarded frame to prove the relay carried ciphertext-only.
 *   5. Reconnect: close A → B queries /last_seen → presence "offline" (Worker's
 *      authoritative state; the offline PUSH is grace-delayed ~30s by design). Reopen A
 *      → B receives instant presence state "online". New encrypted message both ways,
 *      decrypts without restarting any client.
 *
 * Plain ESM JS (no TS) like ws_smoke.mjs. libsodium-wrappers resolves from the
 * client's node_modules via an absolute specifier (harbor-cloud has no sodium dep).
 *
 *   Start the Worker first:  cd harbor-cloud && npx wrangler dev   (serves :8787)
 *   Run this harness:        node scripts/e2e_marco2.mjs
 *
 * Prints a structured, step-by-step trace and a final MARCO 2 PASSOU / FALHOU line.
 *
 * NOTE on E2E readiness: this harness imports the SAME libsodium primitives the
 * client's crypto.ts uses (genKeypair→crypto_box_keypair, seal→crypto_box_seal,
 * open→crypto_box_seal_open), so a green round trip here is a faithful proxy for
 * the client sealing/opening with the keys the (fixed) pairing flow persists.
 */
// @ts-nocheck

const BASE = process.env.HARBOR_SMOKE_HTTP ?? "http://localhost:8787";
const WS = process.env.HARBOR_SMOKE_WS ?? "ws://localhost:8787";
// Resolve libsodium-wrappers from the client app's node_modules (harbor-cloud has none).
const SODIUM_SPEC =
  process.env.HARBOR_SODIUM_SPEC ??
  "file:///D:/Projetos/Projeto%20Harbor/client/node_modules/libsodium-wrappers/dist/modules-esm/libsodium-wrappers.mjs";

/* ───────────────────── HTTP helpers (REST endpoints) ───────────────────── */
async function post(path, body) {
  const res = await fetch(BASE + path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  return res.json();
}
async function get(path) {
  const res = await fetch(BASE + path);
  return res.json();
}

/* ───────────────────── libsodium (real crypto) ───────────────────── */
let sodium;
async function loadSodium() {
  const mod = await import(SODIUM_SPEC);
  // Mirror the client's crypto.ts loader: the sodium object is on `mod.default`
  // under ESM (mod may also surface it bare as a CJS interop); await `.ready`.
  const s = mod.default ?? mod;
  await s.ready;
  sodium = s;
  return sodium;
}
/** Generate a real X25519 keypair, base64 — mirrors client crypto.genKeypair. */
function genKeypair() {
  const kp = sodium.crypto_box_keypair();
  return {
    pub: sodium.to_base64(kp.publicKey),
    priv: sodium.to_base64(kp.privateKey),
  };
}
/** Seal a plaintext string to partner's base64 pubkey → base64 sealed box. */
function sealTo(partnerPubB64, plaintext) {
  const cipher = sodium.crypto_box_seal(
    sodium.from_string(plaintext),
    sodium.from_base64(partnerPubB64),
  );
  return sodium.to_base64(cipher);
}
/** Open a base64 sealed box with own base64 keypair → plaintext or null. */
function openFrom(myPrivB64, myPubB64, cipherB64) {
  try {
    const plain = sodium.crypto_box_seal_open(
      sodium.from_base64(cipherB64),
      sodium.from_base64(myPubB64),
      sodium.from_base64(myPrivB64),
    );
    return sodium.to_string(plain);
  } catch {
    return null;
  }
}

/* ───────────────────── per-socket buffer machinery ───────────────────── */
/* Copied from ws_smoke.mjs: one permanent collector per socket so no frame is
 * dropped between awaits; recv/waitFor/drain read from the queue. */
const buffers = new WeakMap();
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
function recv(ws, ms = 6000) {
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
async function waitFor(ws, match, ms = 6000) {
  for (;;) {
    const m = await recv(ws, ms);
    if (match(m)) return m;
  }
}
async function drain(ws, ms = 500) {
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

/* ───────────────────── structured trace + verdict ───────────────────── */
const steps = [];
function step(name, ok, detail = "") {
  const tag = ok ? "PASS" : "FAIL";
  const line = detail ? `[${tag}] ${name} — ${detail}` : `[${tag}] ${name}`;
  console.log(line);
  steps.push({ name, ok, detail });
  return ok;
}
function section(title) {
  console.log(`\n── ${title} ──`);
}

async function run() {
  await loadSodium();
  const stamp = `${Date.now()}`;
  const aid = `marco2-a-${stamp}`;
  const bid = `marco2-b-${stamp}`;

  /* ───── 1. Register both devices with REAL X25519 pubkeys ───── */
  section("1. /register (real X25519 keypairs, pubkeys published)");
  const keyA = genKeypair();
  const keyB = genKeypair();
  const aReg = await post("/register", { device_id: aid, public_key: keyA.pub });
  const bReg = await post("/register", { device_id: bid, public_key: keyB.pub });
  const aSec = aReg.device_secret;
  const bSec = bReg.device_secret;
  const codeA = aReg.pairing_code; // A's code → B will pair with it (B active)
  step(
    "A registered (pubkey published)",
    typeof aSec === "string" && typeof codeA === "string",
    `device_id=${aid}, code=${codeA}`,
  );
  step(
    "B registered (pubkey published)",
    typeof bSec === "string",
    `device_id=${bid}, code=${bReg.pairing_code}`,
  );

  /* ───── 2. Active pair: B pastes A's code → B learns A's pubkey ───── */
  section("2. /pair (B active) → B learns A's partner_public_key");
  const pairRes = await post("/pair", {
    device_id: bid,
    device_secret: bSec,
    partner_code: codeA,
  });
  const bLearnedAPubkey = pairRes.partner_public_key === keyA.pub;
  step(
    "B pairs actively with A's code",
    pairRes.partner_device_id === aid,
    `partner=${pairRes.partner_device_id}`,
  );
  step("B learned A's X25519 pubkey from /pair", bLearnedAPubkey, "partner_public_key == A.pub");

  /* ───── 3. Passive detect: A polls /me then /partner → A learns B's pubkey ───── */
  section("3. /me + /partner (A passive) → A learns B's partner_public_key");
  const meA = await get(
    `/me?device_id=${encodeURIComponent(aid)}&secret=${encodeURIComponent(aSec)}`,
  );
  step("A's /me detects partner_id=B", meA.partner_id === bid, `partner_id=${meA.partner_id}`);
  const partnerA = await get(
    `/partner?device_id=${encodeURIComponent(aid)}&secret=${encodeURIComponent(aSec)}`,
  );
  const aLearnedBPubkey = partnerA.partner_public_key === keyB.pub;
  step(
    "A's /partner returns B's X25519 pubkey",
    aLearnedBPubkey,
    `partner_public_key == B.pub, presence=${partnerA.presence}`,
  );

  /* Both identities now carry: own keypair + partner pubkey. This is precisely
   * ChatScreen's `encrypted = !!(device_privkey && partner_pubkey)` flag. */
  const encryptedA = !!(keyA.priv && /*partner_pubkey:*/ aLearnedBPubkey);
  const encryptedB = !!(keyB.priv && /*partner_pubkey:*/ bLearnedAPubkey);
  step("encrypted === true on A", encryptedA, "device_privkeyA + partner_pubkeyB present");
  step("encrypted === true on B", encryptedB, "device_privkeyB + partner_pubkeyA present");

  /* ───── 4. Open two live WS; drain connect-time presence pings ───── */
  section("4. open two WS (A, B), drain connect-time pings");
  const wa = await openSocket(`${WS}/ws?device_id=${aid}&secret=${aSec}`);
  const wb = await openSocket(`${WS}/ws?device_id=${bid}&secret=${bSec}`);
  await drain(wa);
  await drain(wb);
  step("A & B sockets open", true, "both connected");

  /* ───── 5. Bidirectional E2E: seal → enc frame → relay forwards → open ───── */
  section("5. bidirectional E2E chat (seal → enc-only frame → open → plaintext)");

  // A → B
  const msgAB = "Oi do A 🔒 (msg 1)";
  const wireAB = sealTo(keyB.pub, JSON.stringify({ text: msgAB }));
  wa.send(JSON.stringify({ type: "chat", id: "m-ab-1", enc: wireAB }));
  const gotB = await waitFor(wb, (m) => m.type === "chat" && m.id === "m-ab-1");
  const ackAB = await waitFor(wa, (m) => m.type === "ack" && m.id === "m-ab-1");
  // Inspect the frame the RELAY delivered to B — must be enc-only, no text.
  const relayedABHasEnc = typeof gotB.enc === "string" && gotB.enc.length > 0;
  const relayedABNoText = gotB.text === undefined;
  const relayedABPayload = JSON.stringify({ type: gotB.type, id: gotB.id, from: gotB.from, enc_present: relayedABHasEnc, text_present: !relayedABNoText, enc_len: gotB.enc?.length });
  const openedAB = openFrom(keyB.priv, keyB.pub, gotB.enc);
  const decodedAB = openedAB !== null ? JSON.parse(openedAB).text : null;
  step(
    "A→B: relay forwarded enc-only frame (no text)",
    relayedABHasEnc && relayedABNoText,
    `frame=${relayedABPayload}`,
  );
  step("A→B: B decrypted with own keypair", decodedAB === msgAB, `recovered="${decodedAB}"`);
  step("A→B: ack delivered:true", ackAB.delivered === true);

  // B → A
  const msgBA = "Recebi! Resposta do B 🔒 (msg 2)";
  const wireBA = sealTo(keyA.pub, JSON.stringify({ text: msgBA }));
  wb.send(JSON.stringify({ type: "chat", id: "m-ba-2", enc: wireBA }));
  const gotA = await waitFor(wa, (m) => m.type === "chat" && m.id === "m-ba-2");
  const ackBA = await waitFor(wb, (m) => m.type === "ack" && m.id === "m-ba-2");
  const relayedBAHasEnc = typeof gotA.enc === "string" && gotA.enc.length > 0;
  const relayedBANoText = gotA.text === undefined;
  const relayedBAPayload = JSON.stringify({ type: gotA.type, id: gotA.id, from: gotA.from, enc_present: relayedBAHasEnc, text_present: !relayedBANoText, enc_len: gotA.enc?.length });
  const openedBA = openFrom(keyA.priv, keyA.pub, gotA.enc);
  const decodedBA = openedBA !== null ? JSON.parse(openedBA).text : null;
  step(
    "B→A: relay forwarded enc-only frame (no text)",
    relayedBAHasEnc && relayedBANoText,
    `frame=${relayedBAPayload}`,
  );
  step("B→A: A decrypted with own keypair", decodedBA === msgBA, `recovered="${decodedBA}"`);
  step("B→A: ack delivered:true", ackBA.delivered === true);

  /* Ciphertext-only confirmation: the two frames the relay stamped+forwarded
   * carried `enc` and were devoid of `text`. The relay never sees plaintext. */
  const relayCiphertextOnly =
    relayedABHasEnc && relayedABNoText && relayedBAHasEnc && relayedBANoText;
  const e2eImmediateDecrypt = decodedAB === msgAB && decodedBA === msgBA;
  step(
    "relay confirmed ciphertext-only (enc on both frames, text absent)",
    relayCiphertextOnly,
  );
  step(
    "immediate decryption both directions (no restart)",
    e2eImmediateDecrypt,
  );

  /* ───── 6. Reconnect test ───── */
  section("6. reconnect: close A → B vê offline → reopen A → B vê online → new enc msg");

  // 6a. Close A's WS (clean close).
  closeQuiet(wa);
  // Give the Worker a moment to register the close + set last_seen.
  await new Promise((r) => setTimeout(r, 600));

  // 6b. B vê offline — query the Worker's authoritative presence via /last_seen.
  //     (The offline PUSH is grace-delayed ~30s on the Worker by design; the
  //     last_seen RPC returns `presence:"offline"` immediately from isOnline().)
  wb.send(JSON.stringify({ type: "last_seen" }));
  const lsAfterClose = await waitFor(
    wb,
    (m) => m.type === "last_seen" && m.device_id === aid,
  );
  const bSawOffline = lsAfterClose.presence === "offline";
  step(
    "B vê A offline (last_seen RPC, authoritative)",
    bSawOffline,
    `presence=${lsAfterClose.presence}, last_seen=${lsAfterClose.last_seen}`,
  );

  // 6c. Reopen A — its reconnect pushes instant presence state="online" to B.
  const wa2 = await openSocket(`${WS}/ws?device_id=${aid}&secret=${aSec}`);
  const bSawOnline = await waitFor(
    wb,
    (m) => m.type === "presence" && m.device_id === aid && m.state === "online",
    8000,
  );
  step(
    "B vê A online (presence push on A's reconnect)",
    !!bSawOnline,
    bSawOnline ? `state=${bSawOnline.state}, ts=${bSawOnline.ts}` : "no online push received",
  );

  // 6d. New encrypted message after reconnect — both directions, decrypt,
  //     without restarting any client (A reused its same keypair; B stayed open).
  const msgReconAB = "Após reconectar 🔗 (msg 3)";
  const wireReconAB = sealTo(keyB.pub, JSON.stringify({ text: msgReconAB }));
  wa2.send(JSON.stringify({ type: "chat", id: "m-recon-3", enc: wireReconAB }));
  const gotB3 = await waitFor(wb, (m) => m.type === "chat" && m.id === "m-recon-3");
  const openedReconAB = openFrom(keyB.priv, keyB.pub, gotB3.enc);
  const decodedReconAB = openedReconAB !== null ? JSON.parse(openedReconAB).text : null;
  step(
    "reconnect: new A→B encrypted msg decrypted",
    decodedReconAB === msgReconAB,
    `recovered="${decodedReconAB}"`,
  );

  const msgReconBA = "Resposta pós-reconexão do B 🔗 (msg 4)";
  const wireReconBA = sealTo(keyA.pub, JSON.stringify({ text: msgReconBA }));
  wb.send(JSON.stringify({ type: "chat", id: "m-recon-4", enc: wireReconBA }));
  const gotA4 = await waitFor(wa2, (m) => m.type === "chat" && m.id === "m-recon-4");
  const openedReconBA = openFrom(keyA.priv, keyA.pub, gotA4.enc);
  const decodedReconBA = openedReconBA !== null ? JSON.parse(openedReconBA).text : null;
  step(
    "reconnect: new B→A encrypted msg decrypted",
    decodedReconBA === msgReconBA,
    `recovered="${decodedReconBA}"`,
  );

  closeQuiet(wa2);
  closeQuiet(wb);

  /* ───── Verdict ───── */
  const failed = steps.filter((s) => !s.ok);
  console.log(`\n${"═".repeat(60)}`);
  if (failed.length === 0) {
    console.log("MARCO 2 PASSOU — all steps green");
  } else {
    console.log(`MARCO 2 FALHOU — ${failed.length} step(s) failed:`);
    for (const f of failed) console.log(`  ✗ ${f.name}${f.detail ? " — " + f.detail : ""}`);
  }
  console.log(`${"═".repeat(60)}`);
  return failed.length === 0 ? 0 : 1;
}

run()
  .then((code) => {
    if (code !== 0) process.exit(code);
  })
  .catch((e) => {
    console.error("smoke crashed:", e instanceof Error ? e.message : String(e));
    if (e instanceof Error && e.stack) console.error(e.stack);
    process.exit(1);
  });
