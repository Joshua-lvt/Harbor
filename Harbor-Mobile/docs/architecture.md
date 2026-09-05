# Harbor Mobile — architecture

Identity-first: the Harbor identity is the person; devices are endpoints.

```
Identity "Joshua" (harbor_id)
├── Device: Desktop  (device_id, DeviceType::Desktop)
└── Device: Mobile   (device_id, DeviceType::Mobile)
```

## Reuse (nothing reimplemented)

| Concern | Owner (existing) | Mobile use |
|---|---|---|
| Identity, pairing, session, signaling | `harbor-core`, `harbor-control`, server | unchanged; pairing authorizes identities, devices attach to it |
| Chat transcript, dedup, delivery | `direct.rs:ChatSession` | same conversation; `record_inbound` drops duplicate IDs |
| Presence verdict | `presence.rs` | same ONLINE/AWAY/OFFLINE machine; devices feed it |
| Profile sync | `profile.rs` | same Local/PartnerProfile, GIF bytes preserved |
| Activity schema | `activity.rs` | phone activity maps onto shareable fields only |
| Notifications | NotificationService + widget | same types; persistent bar is a new surface, not a new system |
| Media transport | Go/Pion worker | same worker; one endpoint per identity (takeover) |

## New core surface (`harbor-core/src/device.rs`)

- `DeviceType { Desktop, Mobile }` — per-install, local-only, persisted
  in settings (`deviceType`, default `desktop`).
- `compatibility(local, remote)` — `Mobile ↔ Mobile` is
  `RefusedMobileToMobile`, everywhere: pairing accept, session connect,
  direct-channel `device_hello`, call signaling.
- `IdentityDevices` — companion registry: own + peer devices, identity
  presence = OR over authorized devices, single media endpoint with
  explicit takeover ordering (PC→Mobile, Mobile→PC).
- `MobileStatus` — the only phone aggregate that crosses IPC/P2P:
  `deviceType, batteryPercent, charging, phoneActivity, lastActiveAt,
  currentApp?, locationSharingEnabled, location?, locationUpdatedAt?,
  notificationSharingEnabled`. Location/currentApp present only when
  their share toggle is ON; no history, no notification contents, no
  keystrokes, no screen data — enforced by validation in Rust.
- Call policy: a Mobile endpoint joins with mic **muted**; unmute is an
  explicit tap. `persistentCall` defaults ON (standalone eternal call),
  OFF means explicit connect.

## Protocol additions (local IPC only)

`device.state` / `device.updated` / `device.configure`,
`mobile.state` / `mobile.updated` / `mobile.update`,
`call.takeover`. None are valid on the server transport: the server
stays control-plane (identity, pairing, presence leases, SDP/ICE
relay). Device claims between cores travel inside core-to-core
payloads (direct control-channel `device_hello`, call signaling
envelopes the server relays opaquely), never as server-parsed fields.

## Flows

**Companion chat**: both own devices hold sessions with the same peer
identity; each core's transcript converges by message ID. No separate
"mobile chat".

**Presence bar (Android)**: a foreground-service notification renders
the committed partner aggregate (avatar, name, ONLINE/AWAY/OFFLINE,
current app when shared). It is a surface on `presence.changed`, not a
second presence definition.

**Call takeover**: entering from the second own device sends
`call.takeover`; the first endpoint leaves media before the second
joins. Never two mics of one identity at once.

**Phone → PC mirroring** (battery, activity, location, phone
notifications): Mobile → P2P DataChannel → peer. Opt-in per toggle
(default OFF except presence), each with an Android permission gate
and a visible OFF state. Phone notification *contents* are display-only
on the peer, never stored. The desktop renders all of it in its Phone
tab (`MobileView`, fed by `HarborMobileBridge` from the core's
`mobile.state`/`mobile.updated` snapshot): a battery card, an activity
card, an offline position schematic (Canvas grid + accuracy disc, no
tiles, no network), and the transient notice list — each degrading to
its honest empty copy when its toggle is off.

## Privacy invariants

- Server sees no location, notifications, chat, files, audio, or screen.
- Toggles are intents persisted in the core; grants are platform facts
  reported by adapters. Sharing needs both.
- Denied/unavailable permission ⇒ feature reports unavailable, UI says
  so, no fallback invention.
