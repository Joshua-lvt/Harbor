# Harbor Backend Migration — FastAPI → Cloudflare Workers + Durable Objects

This documents the migration of Harbor's relay from a local Python **FastAPI**
server (`server/`, run with `uvicorn`) to the **`harbor-cloud`** Cloudflare
Worker + Durable Object project. The goal stated in the plan: a faithful 1:1
port — no protocol breaks, E2E crypto untouched on the client — so the two
devices reach each other through a globally addressable, always-on edge service
instead of coupling the operator's machine to the app's liveness.

FastAPI stays on disk as **LEGACY** (see `server/LEGACY.md`) until the new
backend is proven with two real Tauri clients, then the `127.0.0.1:8000` /
uvicorn references are removed.

> The authoritative design lives in `cosmic-tumbling-conway.md` (the plan). This
> page is the durable *implementation* record: what was built, where to find it,
> how to run/prove it, and how to deploy.

---

## Target architecture

**Two SQLite-backed Durable Object classes + one Worker router.** A faithful
1:1 port of the FastAPI relay's behavior; the *only* intentional behavior change
is mandated by the prompt: offline presence is pushed after a short grace
window (not instantly, as FastAPI did).

### `HarborRegistry` — singleton DO (`idFromName("harbor-registry")`)

Owns the **global** concerns a per-pair DO cannot: an arbitrary
`pairing_code → device` lookup and `device → pair` routing. Holds a SQLite
`devices` table mirroring FastAPI's `devices` table **minus** `presence` /
`last_seen` (those are live data, owned by `HarborPair`):

```
devices(id TEXT PK, secret TEXT, pairing_code TEXT UNIQUE NULLABLE,
        partner_id TEXT NULLABLE, pair_key TEXT NULLABLE,
        display_name TEXT NULLABLE, public_key TEXT NULLABLE,
        avatar TEXT NULLABLE, created_at REAL)
```

**Secrets live only here.** The Worker authenticates every WS upgrade via
`Registry.verifyDevice` *before* forwarding the upgraded socket to
`HarborPair`, which then trusts the connection — so `HarborPair` holds no
secrets and no E2E private keys.

### `HarborPair` — one DO per pair (`idFromName(pair_key)`)

The coordination atom the app requires ("exactly two devices"). Built on the
**WebSocket Hibernation API** so the DO accrues no GB-s while the two devices
sit idle (Harbor's common state — the widget is "always on" but mostly quiet).
Owns only live/restart-relevant state in SQLite:

```
members(device_id TEXT PK, pair_key TEXT, last_presence TEXT,
        last_seen REAL, pending_grace_until REAL)
outbox(seq INTEGER PK AUTOINCREMENT, from_id TEXT, to_id TEXT,
       payload TEXT, ts REAL)
_schema_migrations(version INTEGER PK, applied_at REAL)
```

Live presence is **derived** from `ctx.getWebSockets(device_id)` (not
persisted): connected → `members.last_presence` (`online`|`away`);
disconnected → `offline`. `webSocketClose` schedules a **~30s grace alarm**; if
the same device reconnects before it fires, the alarm cancels and never pushes
offline. An idempotent **outbox-TTL alarm** (7d) sweeps stale rows, replacing
FastAPI's startup-only sweep.

### Why two DOs (a deliberate, minimal choice)

`/register` and `/pair` happen *before* a pair exists and need a global code
lookup the per-pair DO can't provide. Secrets stay in the Registry; the
Worker authenticates every WS upgrade via `Registry.verifyDevice` **before**
forwarding the upgraded socket to `HarborPair`. The singleton is an
acknowledged trade-off (a global code lookup can't be sharded by `device_id`);
for a private 2-human app the volume is negligible, and it keeps the whole
stack inside Workers+DO (no D1) as scoped.

### Routing

- **HTTP** (`/health` `/register` `/pair` `/profile` `/partner` `/unpair`
  `/me`) → Worker → `HarborRegistry` RPC; `/partner` additionally calls a
  `HarborPair.getPresence` RPC; `/unpair` additionally calls `HarborPair` to
  push `unpaired` + clear outbox.
- **WS** (`GET /ws?device_id&secret`) → Worker `Registry.verifyDevice` → on ok,
  forward the raw request to `HarborPair.fetch` via
  `env.HARBOR_PAIR.idFromName(pair_key).get().fetch(request)`. HarborPair does
  the upgrade (`new WebSocketPair` → `ctx.acceptWebSocket(server, [device_id])`
  → returns `Response(null,{status:101, webSocket: client})`). On auth failure
  the Worker returns 401 (HTTP).

### WebSocket Hibernation correctness

- **Upgrade handoff**: Worker returns the DO's `Response(null,{status:101,
  webSocket: client})` upstream — the documented current pattern.
- **Hibernated-partner forward**: tags + `serializeAttachment` survive
  eviction; on `webSocketMessage`, `ctx.getWebSockets(partner_id)` returns the
  hibernating-but-still-connected partner socket and `.send()` delivers. Empty
  → buffer.
- **RPC wakes a hibernated pair**: `getPresence` / `notifyUnpaired` /
  `clearOutboxForPair` reach a quiet `HarborPair` via RPC; the call
  re-instantiates the DO (cheap ctor), reads live data from SQLite, returns.
  This is why presence/last_seen live in SQLite, not attachments.
- **Grace alarm vs hibernation**: a pending 30s alarm keeps the DO awake 30s
  post-close — negligible for 2 devices, and the only timer surviving eviction
  (`setTimeout` blocks hibernation entirely).

---

## File map — `harbor-cloud`

### Created (`src/`)
- `src/protocol.ts` — discriminated unions (`ClientMessage`, `ServerMessage`)
  + HTTP request/response types + `validateClientMessage`. Single source of
  truth for wire shapes; no free-form event strings anywhere.
- `src/util.ts` — `generateCode()`, `newSecret()`, `pairKey(a,b)`,
  constant-time `verifySecret`, `parseFrame`, `nowTs()`, `DEFAULT_MAX_FRAME_BYTES`.
- `src/registry.ts` — `HarborRegistry extends DurableObject<Env>` (SQLite +
  ctor `blockConcurrencyWhile` schema ensure; `register`/`pair`/`setProfile`/
  `getPartnerInfo`/`getMe`/`unpair` + `verifyDevice` WS-auth RPC).
- `src/pair.ts` — `HarborPair extends DurableObject<Env>` (Hibernation WS
  handlers, SQLite `members`/`outbox`, grace + TTL alarms, `bootstrap`/
  `getPresence`/`notifyUnpaired`/`clearOutboxForPair` RPC).
- `src/index.ts` — the Worker router: route table, CORS, JSON validation,
  Registry RPC for HTTP, `verifyDevice`→forward upgrade for `/ws`, and the
  `PairError`→HTTP-status mapping.

### Created (other)
- `test/protocol.test.ts` — validator unit tests (no I/O).
- `test/registry.test.ts` — HTTP-path integration via `SELF.fetch` (register,
  pair, single-use, profile/clear, /me, /partner, /unpair, auth).
- `test/pair.test.ts` — HarborPair WS integration via Miniflare (presence
  online/offline+grace, duplicate connection, chat enc/text + ack + offline
  buffer + reconnect flush + late ack, voice/activity/typing, malformed/
  oversized-keep-socket, heartbeat, unpaired push, last_seen).
- `scripts/ws_smoke.mjs` — manual two-client harness (port of
  `server/scripts/ws_smoke.py`): register→pair→online chat→offline buffer→
  reconnect flush+late-ack. Runs against `npx wrangler dev` (`node
  scripts/ws_smoke.mjs`, no deps, Node ≥18).
- `vitest.config.mts` — Vitest config wiring `@cloudflare/vitest-pool-workers`.

### Modified
- `wrangler.jsonc` — uses the declarative `exports` field (mutually exclusive
  with `migrations`); two `sqlite` DOs; bindings `HARBOR_REGISTRY`,
  `HARBOR_PAIR`; `vars` (`HARBOR_OFFLINE_MSG_TTL_DAYS: 7`,
  `HARBOR_OFFLINE_GRACE_MS: 30000`, `HARBOR_MAX_FRAME_BYTES: 262144`).
- `package.json` — `test`/`test:run`/`cf-typegen` scripts; devDeps `vitest`,
  `@cloudflare/vitest-pool-workers`.

### Marked legacy (not deleted)
- `server/` — `server/LEGACY.md` marker; code left for reference.
- `testharbor.bat`, `server.zip`, `client/setup-*` scripts — left; noted here
  as legacy.

---

## HTTP contract (matches FastAPI exactly — the client depends on it)

| Method | Path | Body / Query | Response |
|---|---|---|---|
| GET | `/health` | — | `{status:"ok"}` |
| POST | `/register` | `{device_id, public_key?, avatar?}` | `{pairing_code, device_secret}` |
| POST | `/pair` | `{device_id, device_secret, partner_code}` | `{partner_device_id, partner_name?, partner_public_key?, partner_avatar?}` |
| POST | `/profile` | `{device_id, device_secret, display_name?, public_key?, avatar?}` (`""`=clear, `null`/omitted=skip) | `{ok:true}` |
| GET | `/partner` | `?device_id&secret` | `{partner_device_id, partner_name?, presence, last_seen?, partner_public_key?, partner_avatar?}` |
| GET | `/me` | `?device_id&secret` | `{pairing_code?, partner_id?, display_name?}` |
| POST | `/unpair` | `{device_id, device_secret}` | `{ok:true, pairing_code}` |
| GET | `/ws` | `?device_id&secret` | WS upgrade → protocol below |

## WS protocol (verbatim types, client ↔ server)

Outbound (client→server): `heartbeat`, `presence{state}`, `typing{state?}`,
`activity{app}`, `voice_signal{kind,data}`, `chat{id, enc | text+image?}`,
`last_seen`.

Inbound (server→client): `presence{device_id,state,ts,last_seen?}`,
`chat{id,from,ts, enc | text+image?}`, `ack{id,delivered}`,
`typing{device_id,state,ts}`, `last_seen{device_id,last_seen,presence}`,
`activity{device_id,app,ts}`, `voice_signal{device_id,kind,data,ts}`,
`unpaired{pairing_code,ts}`, `error{reason}`.

---

## Security model (unchanged from FastAPI)

- **E2E stays client-side**: libsodium `crypto_box_seal` (X25519 +
  XSalsa20-Poly1305). The server only ever forwards opaque base64 `enc`
  ciphertext; private keys never leave the client (`identity.json`).
  Worker/DOs never decrypt.
- **No secrets in source**: `device_secret` is per-device data in the Registry
  DO's SQLite, not a deploy secret; wrangler has no secrets needed.
- **Server sees only routing metadata** + public keys + opaque ciphertext.
  Logs carry no plaintext chat, no tokens, no secrets.
- **Audio stays P2P**; Cloudflare is signaling-only (`voice_signal`); no
  SFU/relay/recording. STUN/TURN remain client-side.

---

## Dev, test, and two-client proof

### Local dev (no Cloudflare account needed)
```sh
cd harbor-cloud
npx wrangler dev        # in-memory DOs on http://localhost:8787
```

### Automated tests (Vitest + Miniflare)
```sh
cd harbor-cloud
npm run test            # watch  (or `npm run test:run` for one-shot)
```
Covers: protocol validation, the full HTTP surface, and the WS Hibernation
surface — presence online/offline+grace, duplicate connection, chat ack +
offline buffer + reconnect flush + late ack, voice/activity/typing, malformed
and oversized frames (socket kept), heartbeat, the `unpaired` push, and
`last_seen`. 62 tests across three files.

### Manual smoke (two fake clients over the live Worker)
```sh
# terminal 1
cd harbor-cloud && npx wrangler dev
# terminal 2
cd harbor-cloud && node scripts/ws_smoke.mjs
```
Exercises the happy path the two real Tauri clients rely on: online chat with
a delivered ack, offline buffering with a not-delivered ack, and reconnect
flush + late ack. Prints `[PASS]`/`[FAIL]` per step and `ALL PASS`.

### Two real Tauri clients (the final proof — gated by REGRA 11)
After `wrangler dev` is up, point two real clients at `ws://localhost:8787`
(see the client cutover below): connect A + B → mutual online, chat A→B→A,
voice offer/answer/ICE + P2P audio, A closes → B sees offline (after grace),
A reconnects → presence restored + chat resumes.

### Legacy tests (untouched)
```sh
cd server && pytest      # still green — FastAPI code is dormant reference
```

---

## Env / config (`wrangler.jsonc` `vars`)

| Var | Default | Meaning |
|---|---|---|
| `HARBOR_OFFLINE_MSG_TTL_DAYS` | `7` | Outbox row TTL; swept by the HarborPair alarm. |
| `HARBOR_OFFLINE_GRACE_MS` | `30000` | Grace before a transient drop is pushed as `offline`. |
| `HARBOR_MAX_FRAME_BYTES` | `262144` | Largest inbound WS frame; oversize → `{type:"error","frame_too_large"}`, socket kept. |

No `secrets` are required — `device_secret` is stored in the Registry's
SQLite, not as a Worker secret.

---

## Deploy (final, gated step — run only on explicit confirmation)

harbor-cloud is **not yet deployed**. After the code + test review (REGRA 11)
and the two-client proof:

```sh
cd harbor-cloud
npx wrangler types      # regenerate worker-configuration.d.ts
npx wrangler deploy
```

Capture the Worker URL (`https://harbor-cloud.<account>.workers.dev`), then
cut the client over (production relay URL via
`import.meta.env.VITE_RELAY_URL`) and run the smoke harness against prod with:

```sh
HARBOR_SMOKE_HTTP=https://harbor-cloud.<acct>.workers.dev \
HARBOR_SMOKE_WS=wss://harbor-cloud.<acct>.workers.dev \
node scripts/ws_smoke.mjs
```

`wrangler.jsonc` uses the declarative `exports` field and two `sqlite` DOs
with no `migrations` array — a clean first deploy (no prod namespace to
migrate). Once proven with two real clients against Cloudflare, the FastAPI
`server/` and its `uvicorn` references are removed.

---

## Client change set (minimal — wire protocol is unchanged)

The migration is **URL-only** on the client: WS send/receive `type` strings,
payload shapes, and HTTP routes are identical to FastAPI, so no client logic
changes. Changes:

- `client/src/lib/types.ts` — default relay URL changed from
  `ws://localhost:8000` (uvicorn) to `ws://localhost:8787` (wrangler dev). All
  network paths derive HTTP from WS (`relay.ts:httpBase`) and append `/ws`.
- **Production URL** via build-time `import.meta.env.VITE_RELAY_URL` (the
  client's `vite.config.ts` already exposes `VITE_*` via `envPrefix`), read in
  the **one place** that sets the default (`DEFAULT_RELAY_URL` in `types.ts`,
  seeded into `DEFAULT_SETTINGS.relay_url`). Dev falls back to
  `ws://localhost:8787`. A `client/src/vite-env.d.ts` now declares
  `VITE_RELAY_URL` so the `import.meta.env` read typechecks (Vite's
  `vite/client` types are referenced there).
- `client/src/lib/identity.ts` — **one-time existing-install migration**: on
  load, if the persisted `relay_url` is a *known stale built-in default* it is
  rewritten to the current default / `VITE_RELAY_URL`; anything else (a real user
  override) is left untouched. Two values count as a stale default —
  `ws://localhost:8000` (the pre-migration FastAPI relay) **and**
  `ws://localhost:8787` (the wrangler-dev default): a brand-new install created
  *during* the dev/migration phase persists `8787`, so on a prod build it must
  be carried forward like a pre-migration install or the cut-over strands it on
  the dev Worker. The `!== stored` guard makes each rewrite fire once (and never
  when already current); both `loadIdentity` and `loadSettings` perform it (the
  two persisted locations of the relay URL), so the Settings field and the WS
  connection source stay coherent.

**Nothing else** changes client-side: `relay.ts`, `ws.ts`, all `features/*`,
Rust commands, WebRTC (Google STUN stays; TURN is an optional *client* concern,
out of scope), crypto (`crypto.ts` libsodium `crypto_box_seal`, untouched),
local SQLite history, stores.

---

## Open decisions (resolved)

- **Singleton Registry vs D1** — singleton DO; faithful, minimal, in-scope
  (prompt scoped to Workers+DO; harbor-cloud has no D1 binding). Documented as
  a known trade-off.
- **Auto-deploy** — no. Implement + prove locally; deploy is a gated final
  step after code+test review.
- **Grace value** — 30s, configurable via `HARBOR_OFFLINE_GRACE_MS`.
- **Existing-install URL rewrite** — both stale built-in defaults
  (`ws://localhost:8000` *and* `ws://localhost:8787`) migrate to the current
  default; real custom URLs are preserved. Migrating `8787` too is required so
  installs created during the dev/migration phase reach prod on a prod build;
  the only cost is that a prod build run against local wrangler dev (a
  self-contradictory setup — use `npm run tauri dev` to develop locally) would
  have its `8787` rewritten. An accepted trade-off for a 2-human app.

---

## The six closing questions

- **FastAPI ainda é necessário?** During migration — yes (LEGACY reference).
  After the two-client proof against Cloudflare — **no**.
- **Uvicorn ainda é necessário?** **No** after migration.
- **Qual URL o Harbor Client deve usar?** `https://<worker>.workers.dev`
  (prod) / `http://localhost:8787` (dev); WS = `wss://`/`ws://` on `/ws`.
- **O WebRTC continua P2P?** **Yes.** Cloudflare is signaling-only.
- **A E2E encryption continua no cliente?** **Yes**, unchanged (server only
  forwards opaque `enc`).
- **O servidor consegue ler mensagens privadas?** **No.** Only public keys +
  opaque ciphertext + routing metadata; never decrypts.

---

## Known limitations

- **Singleton Registry** — a global code lookup can't be sharded by
  `device_id`; negligible for a 2-human app, an accepted trade-off.
- **STUN-only, no TURN** — unchanged from the current client; NAT traversal
  holes require TURN, a future *client* concern (out of scope per the prompt).
- **Local at-rest chat storage unencrypted** — unchanged from the current
  client; E2E is transport-only here.
