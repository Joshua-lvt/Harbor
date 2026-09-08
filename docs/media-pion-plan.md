# Harbor Direct Media Plan — Pion (Fase 6)

This document is the implementation contract for real voice and screen share. The private Go/Pion worker, its bounded Rust supervisor, and the full signaling leg now exist: the core resolves its single paired peer, holds an idempotent pair session open, and relays opaque SDP/ICE through `session.connect` / `session.signal` / `session.signal_poll` on one reused pinned connection; the worker creates local offers, **answers inbound offers** (`call.accept`), and applies remote answers and relayed candidates (`call.remote_signal`). An incoming offer rings for explicit approval in `CallView.qml` and is never auto-answered; a decline answers `busy` at the signaling layer, and glare is resolved by both sides ignoring the inbound offer. A direct call between two cores through the real control plane is proven end-to-end by a core test (offer → answer → relayed candidates → `CONNECTED` on both sides), and the worker's stdout is demultiplexed by a dedicated reader thread so asynchronous Pion facts never strand in the pipe. Voice, reconnection, and screen share are now implemented end to end. The worker captures and plays real audio through miniaudio with Opus coding (deterministic silent boundary via `HARBOR_MEDIA_AUDIO=silent`), mutes the outgoing track, and refuses a call honestly when devices are unavailable. A lost direct path moves the call to `RECONNECTING` while Pion re-probes; recovery that outlives the core's 15 s window — or a Pion `failed` report — tears the worker down and fails the call through one shared path. Screen share captures the X11 root screen, encodes with libvpx (VP8, 30 fps, keyframes every 5 s), and writes into a video track negotiated in the call's initial offer/answer, so start/stop never renegotiates audio; start/stop live only in `CallView.qml`, share dies with the call but not the reverse, and Wayland sessions are refused with a localized unavailable state. The share round-trip is proven at the core level with a scripted fake worker and over a live direct call with real VP8 packets. Real input/output device enumeration, per-device volume, push-to-talk, and speaking levels are wired through the core into `CallView.qml`. Still not wired: a screen-share preview surface and capture outside X11 — all refused or shown as explicit notices rather than simulated. Settings carries a real microphone self-check: outside any call, the worker opens the selected devices, loops capture back to playback with a short delay, and reports live levels and the measured peak (`audio.loopback_*`); it refuses honestly while a call owns the devices or when no capture device exists. During a live call the worker samples its own Pion transport every 2 s and reports the nominated ICE pair's round-trip time and cumulative audio packet counters (`media.call_stats`); the core derives a loss share and an honest good/fair/poor verdict and surfaces them as `call.stats_changed` events and in every `call.state_changed` snapshot, which `CallView.qml` renders in place of the preview health card.

## What this phase delivers

- `harbor-media`, a Go process embedding [Pion](https://github.com/pion/webrtc), providing **direct peer-to-peer voice and screen share** between two paired devices.
- Real call audio: mute, volume, push-to-talk, speaking indicator, device selection, reconnection.
- Screen share as an **exclusive child of an active call**, controlled only from `CallView.qml`.
- Zero media through the Harbor Server: it keeps routing SDP/ICE signaling only, exactly as documented in [`control-protocol-v1.md`](control-protocol-v1.md).

## Non-negotiable boundaries

- **One application.** `harbor-media` has no UI, no window, no tray presence, and is never launched by the user. It exists only as a supervised child of `harbor-core`.
- **No direct dialing.** The Go process cannot contact the Harbor Server, cannot read durable state, holds no identity and no policy. Every decision belongs to the Rust core; the process only executes media work the core requests.
- **Direct media only.** The current worker gathers host candidates only; future server-reflexive candidates must still remain direct. There is no Harbor TURN, no relay, no HarborNet, no TUN, and no silent fallback. If a direct path cannot be established, the call fails visibly (`FAILED` / `unavailable`); it never degrades through infrastructure we own.
- **No custom crypto.** Media security is WebRTC's (DTLS-SRTP, as implemented by Pion). The Harbor identity (Ed25519 keys) signs control-plane envelopes at the core; the media layer's DTLS certificates are ephemeral and deliberately unrelated. The two trust domains never merge.
- **Screen share never escapes the call.** No `ScreenSharePage`, no sidebar entry, no standalone view. Capture permission denied ⇒ no capture process, no partial state. Stopping share never ends the call; ending the call always ends share, voice, tracks, and the process.

## Process architecture

```text
Harbor Qt/QML (single visible app)
  └─ HarborFacade (typed C++)
       └─ harbor-core (Rust, supervisor, policy, durable state)
            ├─ Harbor Server ⇄ signaling: SDP/ICE + authorization only
            └─ harbor-media (Go + Pion child process, private IPC)
                 └─ direct WebRTC to the peer (voice tracks, screen track)
```

- The Rust core spawns `harbor-media` on demand (first call attempt), monitors it, and tears it down with a deadline. The Go process is killed, never orphaned; a crash maps to `FAILED` for the current call, and a fresh process is started only for a later call.
- The private core⇄media IPC reuses the established frame discipline: 4-byte big-endian length + versioned JSON envelopes over the child's stdio, with bounded queues and a hard message cap. The media process speaks **no other protocol**.
- Health events from Go to Rust are limited to call-scoped facts (connection state, ICE state, track activity, last ping/pong timing). No logs, paths, or system details cross this boundary.

## Control plane additions

The server's allowlist gains only what Fase 3 already reserved, and nothing media-shaped:

- `session.connect` / `session.disconnect` / `session.signal` — opaque SDP descriptions and ICE candidates between paired peers, plus per-session authorization checks and lease expiry.
- `call.*` is private UI↔core state only and is deliberately excluded from signed server envelopes. A peer call request must use an authorized `session.*` control message without media bodies.
- The server still **cannot represent** media frames, DataChannel payloads, chat, files, screen captures, or local call state; the existing allowlist tests keep proving it.

## State machines (user-visible via the facade)

Voice (`AppState.callState` vocabulary stays; the facade maps):

`IDLE → OUTGOING → CONNECTING → CONNECTED`, with `INCOMING` as the peer-side entry, `RECONNECTING` on ICE interruption, and terminal `ENDED` / `FAILED`. Every transition is driven by a core event, never by a QML timer.

Screen share (new, private to `CallView.qml`):

Implemented as the honest live machine `NOT_SHARING ⇄ SHARING`: the supported capture path has no permission prompt or source picker, so no such states exist to fake. Starting requires `CONNECTED`; ending the call (or losing it) tears the share down from any state, while stopping the share never ends the call. Capture and encoding run outside the UI thread through native adapters — no Qt Multimedia in QML.

## Media pipeline

- **Voice:** Opus via Pion's media engine, one sending and one receiving audio track per call. Input/output device enumeration and capture belong to platform adapters compiled into the Go process.
- **Screen share:** one VP8 video track, negotiated in the call's initial offer/answer so start and stop write frames without any renegotiation; an idle share track sends nothing.
- **Reconnection:** a lost direct path moves the call to `RECONNECTING` while Pion's agent re-probes the same direct candidates; if recovery outlives the core's 15 s policy window — or Pion reports `failed` — the call becomes `FAILED` and the worker is torn down. There is no relay to fall back to, so a lost path either returns or ends visibly. An explicit ICE restart command becomes meaningful only alongside server-reflexive candidates (see boundaries above); with host candidates it would re-run the same already-probing agent.

## Testing and proof obligations

1. **Go:** Pion loopback (two peer connections in one test) covers offer/answer, tracks, mute, ICE restart, and deterministic teardown.
2. **Core:** media process supervision — spawn, crash ⇒ `FAILED`, deadline kill, no orphans after quit; every IPC envelope schema-validated.
3. **Integration:** two cores with a fake signaling server complete a full call; the server's recorded traffic contains **only** SDP/ICE/session records — asserted byte-level, not by absence of failures.
4. **QML:** call states and share states render from facade signals via the deterministic provider; `CallView.qml` remains the only surface with share controls.
5. **Real hardware:** two desktops, then a desktop and a device scenario, including network conditions where a direct path must fail *visibly*.

## Explicitly out of scope for this phase

Chat and file transfer (DataChannel, Fase 7), TURN or any relay, presence changes, multi-party calls, recording, and any second executable the user can see or launch. Screen share permission flows are limited to what the platform adapters genuinely support; unsupported platforms report `unavailable` rather than pretending.
