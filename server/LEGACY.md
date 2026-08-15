# `server/` — LEGACY FastAPI relay

> **⚠ This is the LEGACY backend. It is kept on disk as a dormant reference only.**
>
> Harbor's relay has been migrated to **`harbor-cloud`** — a Cloudflare Worker +
> Durable Objects project that is a faithful 1:1 port of the code in this folder
> (same HTTP routes, same WebSocket wire protocol, same E2E-crypto-blind
> forwarding). See:
> - `../harbor-cloud/README.md` — how to run/deploy the new backend.
> - `../docs/cloudflare-migration.md` — the authoritative migration record
>   (architecture, protocol, DO design, flows, security, two-client verification).
>
> **Do not start new work here.** This directory served as the source the Worker
> was ported from; it stays so the migration can be audited against it and as a
> fallback until the Worker is proven with two real clients. Once that proof
> lands, the `127.0.0.1:8000` / `uvicorn` references in the client and in this
> README are removed and this folder can be deleted.
>
> **Nothing has been renamed here** — its import paths and CLI
> (`uvicorn app.main:app`) are unchanged so an operator can still run it ad hoc if
> needed. The legacy `pytest` suite (`server/tests/`) is intentionally **left
> green and untouched** as a behavioral spec the port was validated against.

## Legacy artifacts also kept on disk (not deleted)

- `testharbor.bat` — top-level dev helper for the FastAPI relay flow.
- `server.zip` — an archived snapshot of the relay.
- `client/setup-client.ps1` — the pre-Cloudflare client bootstrap helper.

These are noted as legacy in `../docs/cloudflare-migration.md`. They are not used
by the Cloudflare backend or the migrated client build and can be removed
alongside this folder once the Worker is production-proven.
