# Harbor Control Protocol v1

The Harbor control protocol carries **only control-plane traffic** between the Qt façade, the Rust core, the Harbor Server, and the internal media component. It is length-framed JSON for the initial implementation.

## Security boundary

This protocol must not carry audio, video, screen-share frames, chat bodies, DataChannel payloads, or file chunks. The Harbor Server accepts identity, pairing, presence, authorization, and SDP/ICE signaling control records only. Direct peer media and data use WebRTC once that phase is implemented; its process boundaries and state machines are planned in [`media-pion-plan.md`](media-pion-plan.md).

## Frame

Each local IPC record is a four-byte unsigned big-endian payload length followed by UTF-8 JSON. The maximum payload is 1 MiB. A receiver rejects truncated, empty, oversized, malformed, or unknown-version frames before dispatching them.

## Envelope

```json
{
  "v": 1,
  "type": "core.hello",
  "request_id": "e0a1b7f3-5d89-4d5d-bd32-9bea68fd9a80",
  "timestamp": "2026-08-31T20:00:00Z",
  "payload": {
    "client": "harbor-ui",
    "protocol_min": 1,
    "protocol_max": 1,
    "capabilities": ["pairing", "presence"]
  }
}
```

`v`, `type`, and `payload` are required. Commands require `request_id`; events require `event_id`. `reply_to` correlates a response. Structured errors contain a stable `code`, localization key `ui_key`, `retryable`, and a safe detail string. Stack traces and secrets are never serialized.

## Authenticated envelopes

Server-bound requests are wrapped in an authenticated envelope:

```json
{
  "signer_id": "0d7f45a2-8e1c-4c0f-9a6b-2f19c3fbd710",
  "envelope": { "...": "..." },
  "signature": "<base64-nopad Ed25519 signature over (signer_id, envelope)>"
}
```

The signature is Ed25519 over the deterministic serialization of `signer_id` plus the envelope, encoded as unpadded Base64. Registration (`identity.update`) requires proof of possession: the payload's `device_id` must equal `signer_id` and the signature must verify against the submitted public key. Every later request is verified against the registered public key of `signer_id`; an unknown signer or a bad signature is answered with `unauthorized` and never reaches the state machine.

## Initial message families

- Core lifecycle: `core.hello`, `core.ready`, `core.shutdown`
- Identity/settings: `identity.*`, `settings.*`
- Pairing: `pairing.create`, `pairing.submit`, `pairing.cancel`, `pairing.incoming`, `pairing.accept`, `pairing.decline`, `pairing.status`
- Presence/session: `presence.publish`, `presence.changed`, `session.connect`, `session.disconnect`, `session.signal`, `session.signal_poll`
- Contacts: `contacts.list` — the server answers with the caller's accepted relationships as registered identity records (`device_id`, `harbor_id`, public key, registration time). The supervised core filters this down to safe `{deviceId, harborId}` pairs before anything crosses the local IPC, so public keys never reach the UI: the list exists to gate peer-only surfaces on the control plane's durable truth, not to hand out key material.
- **Private local profile only**: `profile.state` and the unsolicited `profile.updated` event
- **Private local activity only**: `activity.state` and the unsolicited `activity.updated` event
- **Private local device/mobile only**: `device.state` + `device.updated` (own endpoint, companion mode, linked-device registry, media endpoint; `device.configure` manages the type switch and links), `mobile.state` + `mobile.updated` (the validated phone aggregate via `mobile.update`), and `call.takeover` (explicit media-endpoint handoff between two own devices). None are valid on the server transport. The peer side of the device picture arrives with the direct-channel `device_hello`; until it does, `Mobile ↔ Mobile` refusal is enforced by the shared `compatibility` policy wherever both endpoint kinds are known (pairing accept, session connect, call signaling), with the UI gating first.

The allowlist is expanded only in the corresponding implementation phase. `session.signal` is restricted to SDP/ICE control material; it is not a generic relay channel.

### Local activity boundary

`activity.state` and `activity.updated` are valid only between the Qt façade and its supervised local core. They carry a sanitized local snapshot: monitor state, keyed timeline records, and rolling-week totals. They never carry PID, executable path, command line, window title, or other raw observation material. The Harbor Server has no activity route and rejects even an authenticated `activity.state` request; activity is not published to peers in protocol v1.

### Local profile boundary

`profile.state` and `profile.updated` are valid only between the Qt façade and its supervised local core. They carry the peer's public profile as validated by the core — display name, status message, avatar type, and avatar bytes — and never identity material: no device IDs, keys, revisions, or hashes cross this boundary.

The public profile itself travels peer-to-peer, never through the server. While a direct call is connected, each core publishes its local profile over the direct control channel as a versioned, revision-stamped frame (`hello`, plus peer-paced `avatar_offer`/`avatar_request`/`avatar_chunk`/`avatar_cancel` for avatars too large to ride inline) and applies the peer's frames only when their revision is newer than the stored one. Avatars travel as `data:` URLs (rebuilt from bytes, never from paths), hash-verified before publication, with GIF bytes preserved; the validated partner snapshot persists locally so a restart does not blank the partner. The Harbor Server has no profile route: its traffic stays signed control-plane envelopes only, asserted byte-level by tests.

## Control-plane state foundation

`harbor-control` owns the transport-independent pairing, relationship, presence lease, and logical-session transitions. A pairing code is exactly six ASCII digits, expires after five minutes, and authorizes peers only after the target explicitly accepts the request. Presence is a 45-second lease and becomes `OFFLINE` after expiry. A logical session requires an accepted relationship, and SDP/ICE records are capped at 64 KiB.

The state machine deliberately does **not** authenticate a network caller. The `harbor-server` dispatcher applies that boundary today: only signed, verified envelopes from registered devices reach the pairing, presence, and session transitions. `session.signal` is validated (membership, active session, 64 KiB cap) but actual delivery to the remote peer is the transport's job.

## Transport

`harbor-server` exposes exactly one listener: a TLS endpoint that speaks the same 4-byte big-endian length-prefixed JSON framing as the local IPC, with one `AuthenticatedEnvelope` request per frame and one correlated response envelope per request.

- **TLS**: rustls 0.23 (ring provider), TLS 1.3, no client certificates. The server identity is a persistent self-signed certificate (CN `harbor-server`, ECDSA P-256 via rcgen) stored as `tls/cert.pem` + `tls/key.pem` under the server state directory; the key is written atomically (temp file + rename, mode 0600). The certificate is not the trust anchor — the **SHA-256 fingerprint of the certificate DER** is. A Harbor core pins that fingerprint out-of-band and refuses any server whose served certificate does not match. Operator-provided PEM files (`HARBOR_SERVER_CERT`/`HARBOR_SERVER_KEY`) replace the generated identity; a half-written generated pair is regenerated, never trusted.
- **Frame limit**: the network cap is 256 KiB (`MAX_NETWORK_FRAME_BYTES`), tighter than the local 1 MiB maximum. The largest legitimate payload is a 64 KiB session signal (which can roughly double under JSON escaping); a peer announcing a longer frame is disconnected before any parse. Frames that are not a signed request close the connection; oversized frames, unparseable frames, and garbage bytes never take the listener down.
- **Replay window**: the server refuses any request whose RFC 3339 timestamp is more than ±300 seconds from its own clock (`stale_timestamp`, retryable) *before* dispatching it, bounding how long a captured signed request stays replayable. Clients whose clock drifts beyond the window receive retryable errors until resynchronized.
- **Concurrency and lifetime**: at most 8 concurrent connections; idle connections (including an unfinished handshake) are dropped after 30 s. Every response ends the connection only on protocol violation; otherwise the connection serves further requests until idle timeout or clean close, and every close sends TLS `close_notify` so a client read sees clean EOF, not truncation.
- **Control-plane only**: the message-type allowlist has no media, screen-share, chat, file, or DataChannel entries, and the 256 KiB frame cap plus 8-connection bound keep bulk transfer physically impossible through this listener. Signaling relay to the remote peer is the next layer, not a data path.
- **Endpoints**: the listener binds whatever `SocketAddr` `HARBOR_SERVER_BIND` carries, including dual-stack `[::]:9091` (production) and loopback defaults. Clients address it as `host:port`, bracketed IPv6 (`[2001:db8::1]:9091`), or a DNS name; at connect time every resolved address is dialed in order with a bounded per-address timeout, so one hostname can carry both AAAA and A records and the client uses whatever the network actually routes. The pinned fingerprint is certificate-derived and therefore transport-agnostic: the same pin secures IPv4, IPv6, and hostname endpoints, and SNI is always the constant `harbor-server` (the custom verifier ignores the name). Dial failures are fail-fast and surface as retryable `error.server.unavailable`, never as hangs.

Configuration is environment-only: `HARBOR_SERVER_BIND` (default `127.0.0.1:9091`), `HARBOR_SERVER_STATE_DIR` (default `$XDG_STATE_HOME/harbor-server`, falling back through `$HOME/.local/state`), and the optional certificate overrides above. The startup line prints the bind address and the certificate fingerprint (public pinning material, not a secret); no key material is ever logged.

## Durable server state

`harbor-server` persists only the minimal control-plane metadata: identity records and accepted pairing relationships (`control-state-v1.json`, schema version 1, atomic writes, private directory and file modes on Unix). Pending pairings, presence leases, and logical sessions are intentionally transient: a restarted server must never resurrect a live pairing code, a stale presence lease, or a phantom call. Relationships whose endpoints are no longer registered are discarded on restore. The generated TLS identity is durable by design — a restart must keep the same fingerprint so pinned clients keep trusting the server.

The listener is deployed on the LG K11+ (Termux/Android) as the production control plane; operation, upgrade, and validation are documented in [`k11-runbook.md`](k11-runbook.md).
