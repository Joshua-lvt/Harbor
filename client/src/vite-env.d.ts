/// <reference types="vite/client" />

/* Environment variables statically substituted by Vite at build time.
 *
 * The production Cloudflare Worker relay URL is injected here so a production
 * build dials the deployed `harbor-cloud` Worker instead of the local
 * `wrangler dev` default (`ws://localhost:8787`). The prod URL is NOT hardcoded
 * in source — it is read from a local, non-versioned `client/.env` file at build
 * time (see `client/.env.example`). See `DEFAULT_RELAY_URL` in `lib/types.ts`.
 *
 * Dev:  unset → defaults to the local wrangler-dev Worker (`ws://localhost:8787`).
 * Prod: set `VITE_RELAY_URL=wss://harbor-cloud.<acct>.workers.dev` in the
 *        `.env` file (or inline before the build command).
 */
interface ImportMetaEnv {
  readonly VITE_RELAY_URL?: string;
}
interface ImportMeta {
  readonly env: ImportMetaEnv;
}
