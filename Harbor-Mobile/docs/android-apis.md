# Harbor Mobile — Android API research (§74)

Target: `targetSdk 35`, `minSdk 28` (Android 9.0 — notification channels
and adaptive behavior baseline). Built against compile SDK 36 while targeting
API 35.
NDK 28, JDK 17, foreground-service types as of Android 14/15.

Rule: verify before use; if a version/device refuses an API, report
unavailable — never invent.

## Qt 6 on Android

- Qt for Android ships prebuilt `android_arm64_v8a` kits (online installer).
  This workspace currently has the Qt Android kits and produces an installable
  x86_64 and arm64 debug APKs; physical-device validation is still pending.
- App entry is `QtActivity` (or subclass) via `androiddeployqt`;
  `AndroidManifest.xml` declares activities, services, permissions.
- QML lifecycle follows the activity: `onPause/onResume` pause render;
  process death is real — restore from persisted core state, reconnect
  with backoff, never spin.
- JNI bridge: Java adapters expose static methods / signals to C++
  (`QJniObject`), C++ facade owns typed properties for QML.
- Background rendering/audio needs a foreground service; a cached
  activity alone is killable at any time (Doze/App Standby below).

## Battery — `BatteryManager` (API 21+, sticky intent)

- `BATTERY_PROPERTY_CAPACITY` (21+) + `isCharging()` via sticky
  `ACTION_BATTERY_CHANGED` — no permission needed.
- Live updates: manifest receiver for `ACTION_BATTERY_CHANGED` is
  sticky-broadcast; register at runtime instead.
- Fallback: if the property reads `Integer.MIN_VALUE`, report battery
  unavailable.

## App usage — `UsageStatsManager` (API 22+)

- Needs `PACKAGE_USAGE_STATS` — a special Settings grant
  (`Settings.ACTION_USAGE_ACCESS_SETTINGS`), not a runtime dialog.
  Check with `AppOpsManager.checkOpNoThrow(OPSTR_GET_USAGE_STATS)`.
- `queryUsageStats(INTERVAL_DAILY…)` gives aggregates incl.
  `getLastTimeUsed()`/`getLastTimeVisible()`; `queryEvents()` gives
  foreground/background/move-to-foreground events for "current app".
- Foreground-app confidence: only from recent `MOVE_TO_FOREGROUND`
  events; otherwise report `Phone active`, never a guessed package.
  Package → label via `PackageManager`; never exfiltrate the raw
  package list beyond the single current label.
- No grant ⇒ `Share phone activity` stays OFF with setup guidance.

## Notifications — `NotificationListenerService` (API 18+, 21+ sane)

- Special access: `ACTION_NOTIFICATION_LISTENER_SETTINGS`; bound only
  after explicit user enablement. Check via
  `NotificationManagerCompat.getEnabledListenerPackages()`.
- Receives `StatusBarNotification`: mirror **app label + title/text**
  only while `Share phone notifications` is ON (default OFF).
- Never persist contents; skip categories `CATEGORY_SYSTEM`,
  ongoing media/OTP heuristics stay display-only; honor
  `EXTRA_CONTAINS_CUSTOM_VIEW`? No — plain text extras only.
- Revoked ⇒ mirroring stops immediately and the toggle reads OFF-effective.

## Location (API 26–36)

- Foreground: `ACCESS_FINE_LOCATION` or `COARSE` runtime permission. The
  package uses `LocationManager` with the network provider first and GPS as
  an explicit fallback because Google Play services is not bundled in the
  Qt APK. Updates use a 60 s / 50 m significant-change policy; no fix means
  no coordinates are sent.
- Background (API 29+): separate `ACCESS_BACKGROUND_LOCATION` grant;
  API 30+ requires foreground grant first, then a settings-page request
  with visible rationale. Without it, updates stop when backgrounded —
  say so.
- Foreground-service type `location` (API 29+; enforced 34+) with a
  persistent system notification while tracking.
- Payload: `{lat, lon, accuracyM, updatedAt}` only; no history; `Share
  location` OFF ⇒ provider never started.

## Presence bar — foreground service + `Notification` (API 26+ channels)

- Persistent partner bar = ongoing notification (`setOngoing(true)`,
  channel `IMPORTANCE_LOW`, no sound) from a `foregroundServiceType`
  service (`specialUse`? prefer `mediaPlayback` only while in call;
  otherwise `dataSync`-free persistent presence via `START_STICKY`
  service on API <34 constraints, or short-service). Exact type is
  validated at APK build time against API 35 enforcement.
- Content = committed presence aggregate only; tap opens Harbor Chat.
- Call audio while backgrounded: `microphone` + `mediaPlayback`
  foreground-service types (API 34 enforcement) + `AudioRecord` focus
  handling; mic starts MUTED and shows a system privacy indicator.

## Power — Doze / App Standby (API 23+, 26+ limits)

- No exact timers; use FCM-free design: control-plane socket + backoff
  reconnect; `setAndAllowWhileIdle` only for presence-lease keepalive,
  batched.
- Battery-optimization exemption (`REQUEST_IGNORE_BATTERY_OPTIMIZATIONS`)
  is ask-only with rationale; refusal ⇒ longer reconnect intervals,
  honestly surfaced.
- Jobs/WorkManager for deferred transcript convergence, not for realtime.

## Permissions UX (just-in-time, per feature)

Each share toggle explains → requests → reflects grant state. Denial
keeps the toggle intent stored but the feature effectively OFF with the
reason shown. Basic Harbor (pairing, chat, presence, call) never
requires these grants.
