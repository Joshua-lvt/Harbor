# Harbor

**Seu porto seguro digital.** A Windows 11 desktop app for couples — a quiet,
constant sense of presence: a small always-on widget, a private two-person chat,
real-time presence, native notifications, and auto-start on boot. No accounts,
no email, no passwords — pair two devices with a shareable code and you're done.

> *Mesmo longe, você ainda está por perto.*

## Repo layout

- `client/` — Tauri v2 (Rust) desktop app: React + TypeScript + Tailwind v4.
  Multi-window (main chat + always-on-top widget), system tray, native
  notifications, autostart. OS-wide idle (Windows `GetLastInputInfo`) drives
  "away".
- `harbor-cloud/` — **Cloudflare Worker + Durable Objects** relay (the current
  backend): private WebSocket coordination for exactly two paired devices, over
  two SQLite-backed Durable Objects (`HarborRegistry`, `HarborPair`).
- `server/` — **LEGACY** Python FastAPI relay: the 1:1 source `harbor-cloud` was
  ported from. Dormant reference; see `server/LEGACY.md`. Kept until the Worker
  is proven with two real clients.
- `docs/cloudflare-migration.md` — the authoritative migration record
  (architecture, protocol, DO design, flows, security, two-client verification).

## Prerequisites (one-time)

- Node 20+ and Python 3.10+ — already present on this machine.
- **Rust toolchain** (rustup, `stable-msvc`) + **MSVC "Desktop development with C++"
  Build Tools** (~6 GB) — required to build the Tauri client. Install via
  https://rustup.rs and the Visual Studio Build Tools installer.
- For the Cloudflare backend: a Cloudflare account + `wrangler` (bundled as a
  devDependency in `harbor-cloud/`; `npx wrangler login` once for deploy).
- git (recommended).

## Quick start

### Relay (Cloudflare backend — local)
```sh
cd harbor-cloud
npm install
npx wrangler dev          # http://localhost:8787 (in-memory Durable Objects)
npm run test:run          # vitest — 62 tests
```
For deploy + the two-client proof, see `harbor-cloud/README.md` and
`docs/cloudflare-migration.md`. The legacy FastAPI relay in `server/` still runs
via `uvicorn app.main:app` if you need it (`server/README.md`); it's not the
backend the client dials by default anymore.

### Client
```sh
cd client
npm install
npm run tauri dev     # requires Rust + MSVC; dials the default relay URL
```
The client's default relay URL is `ws://localhost:8787` (local `wrangler dev`),
overridable to `wss://<worker>.workers.dev` at build time via
`VITE_RELAY_URL=... npm run tauri build` (one wired-in place — see
`client/src/lib/types.ts`'s `DEFAULT_RELAY_URL`). Existing installs on the old
`8000` default are auto-migrated to it.

## Architecture notes

- The relay is now a **Cloudflare Worker** (always-on, globally addressable). The
  legacy FastAPI server is kept as reference only. The client's relay URL is
  configurable in Settings; the built-in default moved to the Worker.
- No public/social servers, no accounts. Two devices pair via a single-use
  `HARBOR-XXXX-XXXX` code.
- E2E encryption is **client-side** (libsodium `crypto_box_seal`); the Worker only
  forwards opaque `enc` ciphertext and never decrypts. See
  `docs/cloudflare-migration.md` § Security model.
- "Away" uses OS-wide idle (`GetLastInputInfo` via a Rust command) so it reflects
  true machine idleness, not just whether a Harbor window has focus.

See the implementation plan in `.claude/plans/` for the full phased breakdown.
