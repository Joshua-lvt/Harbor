# harbor-cloud

The **Cloudflare Workers + Durable Objects** backend for [Harbor](../) — the
private two-person desktop relay. It is a faithful 1:1 port of the legacy
[FastAPI relay](../server/) (`server/app/*`): the same HTTP routes, the same
WebSocket wire protocol, the same E2E-crypto-blind forwarding. Only the process
moved — from a single `uvicorn` you keep running on one operator's machine to a
globally addressable, always-on edge service.

> The legacy FastAPI relay in `../server/` stays on disk as a **LEGACY**
> reference (see `../server/LEGACY.md`) until this Worker is proven with two real
> clients, then those references are removed. The client's relay URL is the only
> thing that flips: `ws://localhost:8000` (FastAPI) → `ws://localhost:8787`
> (local `wrangler dev`) → `wss://<worker>.workers.dev` (prod).

## Architecture

Two SQLite-backed **Durable Object** classes + one **Worker** router. Full
design, protocol, and flows in [`docs/cloudflare-migration.md`](docs/cloudflare-migration.md).

- **`HarborRegistry`** (`src/registry.ts`) — singleton DO
  (`idFromName("harbor-registry")`). Owns the *global* concerns a per-pair DO
  cannot: arbitrary `pairing_code → device` lookup and `device → pair` routing.
  Holds a SQLite `devices` table. **Secrets live only here.** Exposes
  `register`, `pair`, `setProfile`, `getPartnerInfo`, `getMe`, `unpair`, and
  `verifyDevice` (WS auth) RPC.
- **`HarborPair`** (`src/pair.ts`) — one DO per pair (`idFromName(pair_key)`),
  the "exactly-two-devices" coordination atom. Uses the **WebSocket Hibernation
  API** (no GB-s while idle; clients survive eviction). Owns live/restart-relevant
  state in SQLite (`members`, `outbox`). Forwards chat/presence/typing/activity/
  voice_signal; buffers offline chat to `outbox`; flushes on reconnect with a
  late `ack`. Pushes `offline` presence after a ~30s grace window (not instantly
  — the one intentional behavior change from FastAPI).
- **Worker router** (`src/index.ts`) — stateless: authenticates via the Registry
  (HTTP mutations + `/ws` upgrade) and routes HTTP to Registry RPC, and the
  upgraded `/ws` socket to `HarborPair.fetch`. Holds no secrets, no per-connection
  state. CORS is permissive (FastAPI allowed all; the Tauri client's WebView
  origin varies).

Files: `src/protocol.ts` (wire unions + `validateClientMessage` — the single
source of truth for shapes), `src/util.ts` (code/secret/pair-key helpers ported
from `server/app/security.py` + `pairing.py`).

## Security model

Unchanged from FastAPI. E2E stays **client-side**: libsodium `crypto_box_seal`
(X25519 + XSalsa20-Poly1305); the Worker/DOs only ever route metadata + public
keys + opaque base64 `enc` ciphertext and **never decrypt**. Private keys never
leave the client (`identity.json`). `device_secret` is per-device data in the
Registry DO's SQLite, not a deploy secret — `wrangler` needs **no secrets**.
Audio stays P2P (`voice_signal` is signaling only; no SFU/relay/processing).

## Develop

```sh
npm install
npx wrangler dev          # local Worker at http://localhost:8787 (in-memory DOs)
npm run test              # vitest watch  — or `npm run test:run` for a single run
node scripts/ws_smoke.mjs # manual two-client WS harness against the local Worker
```

`wrangler dev` materializes the DO bindings + vars from `wrangler.jsonc`
in-memory. `.wrangler/state/v3/do/...` is created on disk for persistence across
restarts in dev. `npm run cf-typegen` regenerates `worker-configuration.d.ts`
after changing `wrangler.jsonc` (already committed; rerun if you edit bindings).

### Tests

The vitest suite (`@cloudflare/vitest-pool-workers`) runs each test inside a
Miniflare isolate with the real `wrangler.jsonc` bindings — tests exercise the
actual Worker router + DOs in-memory, no deploy needed. Storage reset per case
via `cloudflare:test`'s `reset()`.

- `test/protocol.test.ts` — `validateClientMessage()` unit (enc/plaintext paths,
  `data:image/` filtering, unknown types).
- `test/registry.test.ts` — HTTP paths via `SELF.fetch` against the real
  Registry: `/register`, `/pair` (single-use, self-pair, bad secret, 404),
  `/profile` (set/clear avatar+pubkey), `/me`, `/partner`, `/unpair`.
- `test/pair.test.ts` — WS Hibernation integration: handshake/auth, presence
  online/offline + the grace window (via `runDurableObjectAlarm`), chat forwards
  + `ack` + offline buffer + reconnect flush + late-ack, voice signaling,
  activity/typing, malformed/oversized frames (socket survives), duplicate
  connection (4409), `last_seen`, unpair push, heartbeat.

```sh
npm run test:run   # 3 files, 62 tests — all green as of this commit
```

## Deploy

**Deploy is a gated final step.** The implementation deliverable stops at
"ready to deploy"; `wrangler deploy` runs only after code+test review and a
two-client smoke. Locally the backend is already proven (vitest + the WS smoke
harness below).

```sh
npx wrangler deploy                    # live at https://harbor-cloud.<acct>.workers.dev
npx wrangler deploy --name <name>      # or a custom Worker name
```

After deploy, capture the `https://…workers.dev` URL and point the client at
it by building the client with the production relay baked in:

```sh
cd ../client
VITE_RELAY_URL=wss://harbor-cloud.<acct>.workers.dev npm run tauri build
```

`VITE_RELAY_URL` is read in **one place** (`client/src/lib/types.ts`'s
`DEFAULT_RELAY_URL`) and seeded into `DEFAULT_SETTINGS.relay_url`; everything
else derives. Existing installs on a prior default (`ws://localhost:8000` or
`ws://localhost:8787`) auto-migrate to it on next load; real custom URLs are
preserved. Production over `wss://` is implicit (Worker serves TLS).

## Vars (`wrangler.jsonc`)

| Var | Default | Purpose |
|---|---|---|
| `HARBOR_OFFLINE_GRACE_MS` | `30000` | Grace window before an offline push (anti-flap). |
| `HARBOR_OFFLINE_MSG_TTL_DAYS` | `7` | Outbox row age at which the TTL sweep drops them. |
| `HARBOR_MAX_FRAME_BYTES` | `262144` | Inbound WS frame cap; oversized → `{type:"error"}`, socket kept. |

See [`docs/cloudflare-migration.md`](docs/cloudflare-migration.md) for the full
architecture, protocol, DO design, flows, security, and two-client verification.
