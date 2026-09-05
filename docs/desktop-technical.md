# Harbor — technical overview (desktop)

Harbor 2.0 is a Qt 6/QML desktop application foundation for a calm, shared space between two people. It preserves the complete visual language, navigation, interaction states, responsive layouts, English/Brazilian Portuguese localization, and deterministic mock scenarios while a local Rust control core is introduced behind a typed C++ façade.

## Current boundary

The application starts a supervised `harbor-core` child process beside the Qt executable. Its local IPC uses the versioned, bounded control protocol described in [`docs/control-protocol-v1.md`](docs/control-protocol-v1.md). The core supports a version-negotiated readiness handshake, controlled shutdown, identity, durable settings, configured-server state, and pairing. `identity.get` creates or reads an Ed25519 device identity from a private local fallback store; its private signing seed is not returned through IPC. Settings live in the same private store (atomic writes, `0600` files under `XDG_STATE_HOME`/`~/.local/state/harbor`).

While the core is ready, typed C++/QML bridges feed authoritative settings and local activity snapshots into `AppState` and forward user edits back through the core, so settings survive restarts and the real device `harbor_id` replaces its fixture. Pairing follows the same pattern: `HarborPairingBridge` mirrors the mock provider's contract and drives host code + approval, peer code entry, decline, and error flows against the configured control-plane server; `MockController` remains the deterministic provider for tests and previews. The control plane's durable paired-peer snapshot (safe `{deviceId, harborId}` pairs only) also gates the first-run experience: with a resolved snapshot and no pair, a single-screen pairing onboarding opens before Home — your Harbor ID, their six-digit code, connect, plus an explicit session-scoped "continue without pairing" bypass — and Call and Chat state their unpaired unavailability instead of pretending a channel exists. No server address, fingerprint, or TLS detail is ever shown; the server stays invisible infrastructure.

Local activity is real on Linux: `harbor-core` observes `/proc`, classifies and debounces process launches, keeps raw PID/path/command-line material private, and exposes only sanitized keyed timeline records plus seven-day totals through `activity.state` and `activity.updated`. `activitySharing` and `gameVisibility` are enforced in Rust before serialization. When sharing is on and a call's direct path is live, those sanitized records are delivered to the paired peer over the control DataChannel lane, validated against the shareable schema on arrival, and rendered as the remote lane in `ActivityView.qml` — the Harbor Server never sees activity.

The public profile travels the same direct path, peer-to-peer, never through the server. Each core publishes its durable local profile (display name, status message, avatar) over the control DataChannel lane as a versioned, revision-stamped frame — small avatars ride inline, larger ones follow as peer-paced, hash-verified chunks — and applies the peer's frames only when their revision is newer, so edits converge without redial and replays are ignored. The validated partner snapshot persists locally across restarts, GIF avatars stay animated on every surface, and the UI reads it through `profile.state` and `profile.updated`, which carry public fields only.

Direct peer-to-peer calls now carry real voice, reconnection, and screen share. A supervised Go/Pion worker (`harbor-media`, private framed IPC, no UI and no server access) creates and answers WebRTC offers, and `harbor-core` relays opaque SDP/ICE between paired peers through the control plane's `session.*` messages — a direct call between two cores is proven end-to-end by tests, with VP8 screen-share packets crossing the same direct link in tests. An incoming call rings for explicit approval in `CallView.qml` and is never auto-answered; a decline answers `busy` at the signaling layer. The worker captures and plays real audio through miniaudio/Opus (a deterministic silent boundary stands in under `HARBOR_MEDIA_AUDIO=silent`), a lost direct path moves the call to `RECONNECTING` and fails it visibly when recovery outlives the core's 15 s window, and screen share captures the X11 root screen into a VP8 track negotiated with the call — started and stopped only from `CallView.qml`, torn down with the call, and refused honestly on Wayland. Real audio-device enumeration, per-device volume, mute, push-to-talk, voice activation (the worker's own speech detector gates transmission frame by frame — a closed activation gate still measures the mic so speech can reopen it), and speaking levels feed the same call surface through the core; call-mode toggles persist through settings and reach a live call without redialing. The only deliberate media deferral is screen capture outside X11 (a Wayland portal adapter is a later, explicit addition), documented in [`docs/media-pion-plan.md`](docs/media-pion-plan.md).

The `harbor-server` crate builds a standalone control-plane service with a real TLS listener (see the Transport section of [`docs/control-protocol-v1.md`](docs/control-protocol-v1.md)): framed signed requests only, a 256 KiB frame cap, a ±300 s replay window, an 8-connection bound, and a persistent pinned-by-fingerprint server certificate. It is strictly control plane — its message allowlist makes media, screen-share, chat, and file transfer unrepresentable. It is deployed as the production control plane on the LG K11+; operation and validation are documented in [`docs/k11-runbook.md`](docs/k11-runbook.md).

Direct chat and file transfer ride the same direct path on three Pion DataChannels (`control`, `chat`, `file`) created with each call. Policy lives in the core: 4 KiB message bodies with control-character sanitization, a 32-message outbound queue with a 200-message transcript, delivery states that only a peer acknowledgement can advance to `DELIVERED`, and — for files — chunked streaming (16 KiB chunks, backpressure on buffered amount, no whole-file RAM residency) bounded only by disk space and transport stability, strict chunk ordering, private staging before acceptance, streaming SHA-256 digest verification before publication, collision-safe destination names, bilateral cancellation, and ten-minute offer expiry. The Go worker transports bounded frames only; the server sees none of it. `ChatView.qml` is the single transfer surface: a file picker and drag-and-drop make offers, cards track OFFERED → ACTIVE → COMPLETED with chunk-accurate progress, incoming offers are accepted or declined before any byte moves, completed images render inline, the destination directory is chosen in the view, and everything mirrors the typed facade's sanitized `direct.updated` snapshot.

`harbor-core` contains the matching client: a TLS `ServerClient` that pins the server by certificate fingerprint and exchanges signed, correlated requests. Pairing now runs over it end to end — the core's IPC surface serves `server.config`/`server.configure` and the full `pairing.*` family, the session machine lives in the core (its phases drive the pairing UI through the typed facade), and tests prove a host and a peer core completing pairing over real TLS — including through the deployed K11+ server, where a wrong fingerprint is refused before any state machine runs.

Window, application, and core lifecycle are now real on Linux. Closing the window hides while a way back exists — the system tray when close-to-tray is enabled and a tray is actually present, or the visible companion widget (whose click reopens the shell); with neither, the close is an honest quit, so the app is never left running invisibly — the in-app tray preview is a test-only affordance and never stands in for the real icon in production. The tray icon (`native/HarborTray.cpp`) and the XDG autostart entry (`native/HarborAutostart.cpp`, written atomically to `autostart/harbor.desktop` under `XDG_CONFIG_HOME`) are native adapters behind typed context properties — QML never touches `QSystemTrayIcon`, a confinement the source-contract test enforces. Explicit Quit ends the call, screen share, and transfers, and tears down the supervised core with no orphaned processes. A small desktop companion window (`qml/components/HarborWidget.qml`, same process, no polling, independent top-level so hiding the shell never takes it down) mirrors partner presence, the current activity, and an optional joined-call symbol; it follows Settings → General, survives close-to-tray, and dies on Quit. Windows ships real autostart (Startup-folder shortcut), app-icon extraction, presence detection (idle/lock/session) and tray balloons; Job Objects, DPAPI and signed bundles/installers remain honest no-ops.

Harbor currently does **not** implement:

- screen capture outside X11 (a Wayland session is an honest refusal until a portal adapter is deliberately added);
- account services or pairing without a configured control-plane server (pairing requires `server.configure` with the server address and certificate fingerprint);
- Windows lifecycle adapters and signed bundles/installers (the system tray and desktop notifications are real native adapters on Linux via the freedesktop D-Bus notification service; where the session bus has none, Harbor reports notifications unavailable rather than faking them).

Traffic, QR artwork, and the mock-state controller remain deterministic test affordances; the developer panel and tray preview never mount in production. The real system-tray icon and event notifications (incoming call, new message, file offer/receipt) are native, gated by the notification settings, and keep message contents off the lock screen. During a live call the voice strip reports only essential states (speaking, muted) instead of transport metrics; the core's measured channel facts stay in diagnostics and logs. The activity view shows the paired peer's shared history whenever the core is ready; its week aggregates and simulation button exist only with the mock provider. Network and Devices pages are compiled for diagnostics and tests but are intentionally unreachable from the product shell, whose navigation is Home, Call, Chat, Activity, Phone, and Settings. The Phone tab renders the paired peer's shared phone state (battery, coarse activity, a consented offline position schematic, and display-only mirrored notifications) from the core's MobileStatus snapshot; everything there is session-only, per-toggle, and returns to honest empty states when sharing stops.

## Requirements

- CMake 3.21+
- Qt 6.5+ with Qt Core, Qt Quick, Qt Quick Controls, and Qt Widgets (tray and lifecycle adapters)
- A C++17 compiler
- Rust 1.85+ and Cargo (the CMake build compiles `harbor-core`)
- Qt Quick Test and Qt Test when building tests

## Build and run

```bash
cmake -S . -B build -DCMAKE_BUILD_TYPE=Debug
cmake --build build --parallel
./build/harbor
```

The production build compiles `harbor-core`, places it beside the Qt executable, and links the application to Qt Core, Qt Quick, and Qt Widgets. Test-only dependencies are enabled through `BUILD_TESTING`.

## Tests

```bash
cmake -S . -B build -DBUILD_TESTING=ON
cmake --build build --parallel
ctest --test-dir build --output-on-failure
```

If available in the Qt installation, run the generated QML lint target as well:

```bash
cmake --build build --target all_qmllint
```

## Interface controls

- **Ctrl+Shift+D** — open the deterministic mock-state controller
- **Ctrl+K** — open demo pairing
- **Ctrl+N** — open the in-app notification center
- **Escape** — close the topmost overlay

The developer panel previews connection, presence, call, push-to-talk, pairing, notification, device, content, theme, language, contrast, and motion states without contacting any service or operating-system API.

## Localization

The interface can switch at runtime between:

- English
- Português (Brasil)

Language selection is available in onboarding and Settings. It applies only for the current session.

## Accessibility and motion

Harbor includes keyboard navigation, visible focus, semantic control roles, text labels alongside state colors, 44-pixel minimum targets, accessible chart summaries, and a reduced-motion mode. Reduced motion stops decorative particles, pulses, drift, and chart/map interpolation while preserving deterministic functional state transitions.

## Responsive targets

The shell and page layouts are designed for:

- 1024×640 minimum preview size
- 1280×720 and 1366×768 compact desktop layouts
- 1600×900 and 1920×1080 wide layouts
- 2560×1440 and ultrawide displays with a centered maximum-width content channel

The Rust core boundary remains intentionally narrow: identity, persistence, configured-server state, pairing, session, call approval, local activity monitoring and peer delivery, direct chat/transfer policy, and window/system lifecycle (tray, autostart, notifications, supervised teardown) are real; screen capture outside X11 and Windows-specific adapters are not implemented rather than simulated as production functionality.
