# Harbor Mobile 1.0

Android companion/standalone client for Harbor 2.0. Same identity, same
partner, same conversation — on the phone.

## What it is

- **UI**: Qt 6 / QML (touch layouts, bottom navigation). No Kotlin UI.
- **Authority**: the shared Rust core (`harbor-core` + `harbor-protocol`).
  Android code only feeds platform signals; it never owns identity,
  pairing, chat, presence, or session state.
- **Android APIs** live behind adapters: Java (`android/`) → C++ facade
  (`Harbor-Mobile/native/`) → QML. QML never touches Android directly.

## Two modes

- **Companion**: phone + own PC share one Harbor identity. Same chat,
  same pairing relation, one media endpoint at a time (takeover).
- **Standalone**: phone only, paired with a Harbor PC. Persistent call
  defaults ON and can be switched OFF.

## Compatibility

```
Desktop ↔ Desktop   ✅          Mobile + Desktop ↔ Desktop             ✅
Mobile  ↔ Desktop   ✅          Desktop ↔ Mobile + Desktop             ✅
Desktop ↔ Mobile    ✅          Mobile + Desktop ↔ Mobile + Desktop    ✅
Mobile  ↔ Mobile    ❌  (refused in UI, pairing, core, and protocol)
```

A session needs a participating Desktop. Identity presence is the OR of
its authorized devices: one device alive ⇒ identity ONLINE.

## Layout

```
Harbor-Mobile/
  README.md            this file
  docs/architecture.md device/identity model, protocol additions, flows
  docs/android-apis.md required API research per Android version (§74)
  qml/                 mobile shell (touch-first, reuses Harbor tokens)
  android/             Java adapters (linted here, built with Qt Android kit)
  tests/               mobile core + QML tests
```

## Android rendering diagnostics

Production uses Qt's Android default renderer and the Material control theme.
The shell deliberately draws its primary surfaces/buttons as bounded, flat
items rather than relying on style-level layers.

If a GPU or emulator build shows triangles, torn controls, or bad clipping,
reproduce with diagnostics (not production settings):

```sh
adb shell setprop debug.harbor_disable_effects 1
adb shell setprop debug.harbor_render_backend software  # or opengl/vulkan/default
adb shell am force-stop org.harbor.mobile
adb shell am start -n org.harbor.mobile/.HarborMobileActivity
```
(The dotted aliases `debug.harbor.disable_effects` /
`debug.harbor.render_backend` are accepted too.)

`HARBOR_DISABLE_EFFECTS=1`/`HARBOR_RENDER_BACKEND=...` environment variables
are available for desktop/mobile-native launches. `disable_effects` selects the
flat Basic control style, turns off custom button transforms, and asks Qt not to
allocate scene-graph depth/stencil attachments. `render_backend` is never set
in production; it exists only to isolate RHI/backend behavior.

Source contracts fail if Mobile QML reintroduces `ShaderEffect`, `MultiEffect`,
layer effects, Canvas/Shape masks, particles, fixed desktop dimensions, or the
Material `ToolBar`/`ToolButton` surfaces that previously triggered the invalid
Android layer/elevation path.

### Live preview fidelity

Horizontal "scanlines" seen in a desktop capture can be introduced by the
preview transport rather than Harbor's scene graph. Verify the source with:

```sh
adb exec-out screencap -p > /tmp/harbor-framebuffer.png
```

Do not diagnose a compressed or non-integer-scaled scrcpy window as an app
rendering defect. For a clean preview on a 1080p desktop, let scrcpy use a
1080-high source and display it 1:1 in fullscreen:

```sh
scrcpy -s emulator-5554 --no-audio --stay-awake --fullscreen \
  --max-size=1080 --video-bit-rate=30M --max-fps=60 \
  --window-title='Harbor Mobile Clean'
```

On the local emulator this creates a 486x1080 video texture and avoids the
fractional desktop downscale. The important invariants are: capture the
Android framebuffer before declaring a QML defect, preserve an integer scale
from source pixels to the preview window, and leave production rendering
defaults unchanged. These settings are for observation only; Harbor does not
force a preview-safe backend.

## Network (no manual setup)

The app ships pre-pointed at the Harbor network and configures it on
first run when no server pin exists yet. An existing pin is always
respected and never overwritten. The user never sees or edits addresses
or fingerprints — pairing just works.

- Default endpoint: `100.114.220.46:9091` (Tailnet client path, the same
  endpoint the desktop client uses)
- Pinned fingerprint:
  `b9846aed2e97bd741ae5a2a3de9ab37c1831d2372ca67f26f538bd279dd7271f`
  (public pinning material, not a secret)

Requirement: the phone must be on the Harbor Tailnet (Tailscale logged
in on the device), otherwise pairing reports the server as unreachable.
Settings carries no server section by design — the desktop runbook
records the same product rule: the UI never shows addresses,
fingerprints, or TLS details.

## Appearance (same personalizations as the desktop)

`qml/components/MobileTheme.qml` mirrors the desktop tokens with the
same defaults and the same palettes: appearance mode (dark/light/system,
system follows the OS scheme), 7 accent presets plus custom `#RRGGBB`
with strength, 8 ocean variants, soft/medium corners, comfortable/compact
density, higher contrast, reduced motion plus animation strength. Every
choice persists in the shared core settings, so it is durable and shared
with the desktop meaning of each key.

Deliberate mobile omissions: surfaces are flat solid colors sampled from
the same ocean stops (the render-safety contract bans scene-graph
effects, so there is no blur/transparency layer), there are no
background particles, and copy is English-only (`qsTr` without catalogs).

## Persistent presence bar

While paired, `HarborPresenceBar` (foreground service, `specialUse`)
shows an ongoing low-importance notification with the partner name, the
committed ONLINE/AWAY/OFFLINE state (icon follows the state), and the
shared current app when there is one. Tapping opens Harbor. Unpaired,
the service stops instead of pinning a stale offline row. A presence
change arriving while the app is backgrounded falls back to a plain
ongoing notification rather than crashing on the background
foreground-service start refusal; the next foreground update
re-establishes the service form.

## Build status (honest)

- Rust policy (`core/harbor-core/src/device.rs`): 24 unit tests pass with
  the host toolchain; `aarch64-linux-android` `cargo check` passes with
  the NDK 28 toolchain.
- Core IPC (`device.*`, `mobile.*`, `call.takeover`): 7 dispatch tests
  pass; the full `harbor-core` library suite is green (189 tests).
- Java adapters: compile against the configured Android SDK (API 36) with
  `targetSdk` 35; the Qt Android
  deployment adds the required `androidx.core` runtime dependency.
- QML shell: `all_qmllint` completes and the mobile QML tests pass. The
  Android smoke build also caught and removed a release-blocking diagnostic
  touch probe that used an invalid `TapHandler.z` property.
- Clean `./build-android.sh x64` builds produce a debug APK containing the
  ABI-matched `harbor-media` worker in an app-private resource. The worker is
  extracted only for calls, and a missing/unexecutable worker fails as
  `media_unavailable`; it is never replaced by a simulated call. The x86_64
  APK was installed on the local emulator and stayed alive after launch. This
  proves packaging/startup only — it is not an end-to-end pairing, chat, or
  call proof. An arm64 device run and full Android ↔ Desktop coverage still
  remain.
- Peer-side fan-out (direct-channel `device_hello`, multi-endpoint
  sessions, session/pairing guards consuming the learned peer type) is
  implemented in the core and covered by host tests. Live proof
  (2026-09-05, emulator → K11+ over Tailnet): real pairing submit on the
  phone + accept on the rig, session established, chat message sent from
  the phone (rendered outgoing bubble). Still open: P2P chat delivery
  into the emulator — the direct link never came up there (`linkPeer`
  null), so the rig saw an empty transcript. Suspect is the emulator
  path (SLiRP NAT, ~500 ms RTT), not the protocol: the same flows pass
  in host core tests. Needs a physical arm64 device run (or adb-reverse
  host forwarding) before calling transport finished.
- The mobile host now sends valid, consent-gated current location fixes and
  last-active timestamps when the corresponding Android facts are available.
- Nothing is mocked in production paths: an unavailable platform API
  surfaces as unavailable, never as invented data.

## Remaining validation before calling Mobile finished

- Android calls now use the packaged private worker, real miniaudio Android
  capture/playback (OpenSL ES backend), just-in-time microphone permission,
  and a microphone/media-playback foreground service. The call flow still
  needs a two-endpoint audio proof on a physical arm64 device.
- Phone notification mirroring now has a registered JNI callback and a
  bounded, display-only core event. Desktop rendering is transient and never
  enters `AppState` history. A device-level listener-permission test remains.
- Location has a foreground service, foreground/background permission states,
  restart-safe intent handling, and a documented `LocationManager` fallback
  when fused services are unavailable. A device-level location smoke test
  remains.
- `HarborCoreAdapter` dispatches QML requests through a dedicated core-owner
  thread with request IDs and queued completion signals. The synchronous
  `send()` entry point remains only for native compatibility tests.
- Runtime permission UX for `POST_NOTIFICATIONS` and `RECORD_AUDIO` is
  just-in-time; process-death recovery and Android ↔ Desktop end-to-end tests
  are still required.

## Docs

- [`docs/architecture.md`](docs/architecture.md)
- [`docs/android-apis.md`](docs/android-apis.md)
- Desktop runbook: [`../docs/k11-runbook.md`](../docs/k11-runbook.md)
- Control protocol: [`../docs/control-protocol-v1.md`](../docs/control-protocol-v1.md)

## Release build

The default build is a debuggable package (`./build-android.sh x64`). A
release package is arm64-only and requires credentials from the environment;
the keystore and passwords are never checked in:

```sh
HARBOR_BUILD_TYPE=Release \
HARBOR_ANDROID_KEYSTORE=/secure/harbor-upload.jks \
HARBOR_ANDROID_KEY_ALIAS=harbor \
HARBOR_ANDROID_STORE_PASSWORD='…' \
HARBOR_ANDROID_KEY_PASSWORD='…' \
./build-android.sh arm64
```

Treat the generated build directory as sensitive because Android Gradle
signing configuration contains release metadata. The release script produces
both a v3-signed APK and AAB; CI should use the same external-keystore policy
after the device smoke tests pass. Passwords are exported only for the
androiddeployqt invocation and are not written to CMakeCache.txt.
