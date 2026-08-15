# Harbor Relay

> **LEGACY backend.** Harbor's relay now lives in [`../harbor-cloud`](../harbor-cloud)
> (Cloudflare Worker + Durable Objects). This folder is a dormant 1:1 source the
> Worker was ported from; see [`LEGACY.md`](LEGACY.md) and
> [`../docs/cloudflare-migration.md`](../docs/cloudflare-migration.md). Don't start
> new work here.

Private WebSocket relay that brokers pairing, presence, and chat between exactly
two paired devices. Run it once somewhere both devices can reach (a private VPS, a
home server, or a Tailscale node) — it is **not** a public platform.

## Run (dev)

```sh
python -m venv .venv
.venv\Scripts\activate          # Windows
pip install -e ".[dev]"
uvicorn app.main:app --host 0.0.0.0 --port 8000 --reload
```

## Tests

```sh
pytest
```

## Routes

| Method | Path | Body / Query | Returns |
|---|---|---|---|
| `POST` | `/register` | `{device_id}` | `{pairing_code, device_secret}` |
| `POST` | `/pair` | `{device_id, device_secret, partner_code}` | `{partner_device_id, partner_name}` |
| `POST` | `/profile` | `{device_id, device_secret, display_name}` | `{ok}` |
| `GET`  | `/partner` | `?device_id=&secret=` | `{partner_device_id, partner_name, presence, last_seen}` |
| `GET`  | `/health` | | `{status: ok}` |
| `WS`   | `/ws` | `?device_id=&secret=` | presence / typing / chat / ack / last_seen envelopes |

## WebSocket envelopes (JSON)

- `{"type":"chat","id","text"}` → relay echoes `{"type":"chat",...,"from","ts"}` to the partner or buffers it; sender gets `{"type":"ack","id","delivered":bool}`.
- `{"type":"presence","state":"online|away"}` → forwarded to partner.
- `{"type":"typing","state":"start|stop"}` → forwarded (transient, not stored).
- `{"type":"heartbeat"}` → refreshes last_seen/online.
- `{"type":"last_seen"}` → reply with partner's current presence + last_seen.

## Production / TLS

Run behind a TLS terminator (Caddy auto-cert recommended) so clients use `wss://`.
The relay **must** run with `--workers 1` — the in-memory `ConnectionManager` is
per-process. Delivered messages are deleted immediately; undelivered ones are
swept after `HARBOR_OFFLINE_MSG_TTL_DAYS` (default 7).

> Encryption: MVP payloads are **plaintext** to the relay (it routes/buffers
> them). End-to-end encryption (client-side sealed with a pairing-time key) is
> the next milestone. Keep the relay private and behind TLS until then.
