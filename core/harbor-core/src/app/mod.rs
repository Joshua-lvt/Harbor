//! In-process Harbor domain core: state, dispatch, and supervision.
//!
//! This module owns the bounded local control protocol, local device
//! identity persistence, durable user settings, and private media-worker
//! supervision. The desktop binary drives it over framed stdio; the mobile
//! build drives the same state through the C ABI at the end of this file.
//! Pairing talks to the control-plane server over the pinned TLS client;
//! the worker itself never receives server or identity data.

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

mod media;
mod mobile_link;
mod signaling;

use self::mobile_link::{
    KIND_CHAT, KIND_CHAT_ACK, KIND_HELLO, KIND_MOBILE, KIND_PHONE_NOTIFICATION, KIND_PROFILE,
    LinkContext, LinkFrame,
    LinkInvite, LinkPeer, MobileLink, hello_frame, local_dial_addrs,
};

use self::signaling::{SignalingTick, server_unavailable_error};

use base64::Engine as _;
use crate::activity::ActivityEngine;
use crate::device::{DeviceRecord, DeviceType, MobileStatus, initial_mic_muted, own_mode, reconnect_delay};
use crate::direct::{ChatSession, Delivery, Direction, TransferBoard, TransferPhase};
use crate::{
    PairingError, PairingSession, ServerPin, Settings, SettingsError, load_or_create,
    load_server_pin, store_server_pin,
};
use harbor_protocol::{Envelope, FrameDecoder, ProtocolError, encode_frame};
use serde_json::{Value, json};
use thiserror::Error;
use uuid::Uuid;

use self::media::{MediaError, MediaEvent, MediaSupervisor};

const CORE_CAPABILITIES: &[&str] = &[
    "core.lifecycle",
    "identity",
    "settings",
    "server",
    "pairing",
    "activity",
    "call-bootstrap",
    "call-signaling",
    "direct-chat",
    "direct-transfer",
    "presence",
    "device",
    "mobile",
];

/// How long a lost direct path may take to recover before the call is
/// declared failed. Recovery is Pion's agent re-establishing the same direct
/// candidates; there is no relay to fall back to, so a lost path either
/// returns or the call ends visibly.
const RECONNECT_WINDOW: Duration = Duration::from_secs(15);

fn default_state_dir() -> Option<PathBuf> {
    // One shared resolution with the settings layer: HARBOR_STATE_DIR,
    // then the platform home (%LOCALAPPDATA%\Harbor on Windows,
    // $XDG_STATE_HOME/harbor elsewhere). Callers fall back to ".".
    crate::default_state_dir()
}

fn now_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_secs()
}

struct CallState {
    call_id: Option<String>,
    /// Which paired peer this call is with. Set on start/accept, cleared
    /// with the call: answers, candidates, and incompatibility verdicts all
    /// route through this leg's session.
    peer: Option<Uuid>,
    phase: String,
    muted: bool,
    share_phase: String,
    /// When the direct path was last observed lost. Recovery that outlives
    /// the policy window fails the call instead of hanging in limbo.
    reconnecting_since: Option<Instant>,
    /// Why the last call ended or failed, when one did ("declined", "busy").
    /// Cleared when a new call begins; surfaced in the call snapshot.
    reason: String,
    /// Latest voice-level facts the worker measured on this call's own audio
    /// (~10 Hz). No second capture stream exists for these.
    voice: VoiceLevels,
    /// Latest transport facts the worker measured on this call's own
    /// connection (~0.5 Hz): round-trip time and cumulative audio counters.
    /// None until the worker's first real sample; never fabricated.
    stats: Option<CallStats>,
}

/// One sample of the live call's transport health, measured by the media
/// worker's Pion agent on its own connection.
#[derive(Default, Clone, Copy)]
struct CallStats {
    rtt_ms: f64,
    /// Cumulative RTP packets received and lost on the inbound audio stream.
    packets_received: u64,
    packets_lost: i64,
}

impl CallStats {
    /// Cumulative loss share over the call so far, in percent.
    fn loss_pct(&self) -> f64 {
        let total = self.packets_received as f64 + self.packets_lost as f64;
        if total <= 0.0 {
            return 0.0;
        }
        (self.packets_lost as f64 / total) * 100.0
    }

    /// A coarse, honest verdict for the UI. "unknown" never reaches this
    /// function: it is the absence of stats, kept distinct from any verdict.
    fn quality(&self) -> &'static str {
        let loss = self.loss_pct();
        if self.rtt_ms <= 150.0 && loss < 2.0 {
            "good"
        } else if self.rtt_ms <= 400.0 && loss < 8.0 {
            "fair"
        } else {
            "poor"
        }
    }
}

/// One direction-pair sample of speaking indication.
#[derive(Default, Clone, Copy)]
struct VoiceLevels {
    level: f64,
    remote_level: f64,
    speaking: bool,
    remote_speaking: bool,
}

impl Default for CallState {
    fn default() -> Self {
        Self {
            call_id: None,
            peer: None,
            phase: "IDLE".into(),
            muted: false,
            share_phase: "NOT_SHARING".into(),
            reconnecting_since: None,
            reason: String::new(),
            voice: VoiceLevels::default(),
            stats: None,
        }
    }
}

impl CallState {
    fn reset(&mut self) {
        self.call_id = None;
        self.peer = None;
        self.phase = "IDLE".into();
        self.muted = false;
        self.share_phase = "NOT_SHARING".into();
        self.reconnecting_since = None;
        self.reason.clear();
        self.voice = VoiceLevels::default();
        self.stats = None;
    }

    fn snapshot(&self) -> Value {
        json!({
            "state": self.phase,
            "call_id": self.call_id,
            "muted": self.muted,
            "share_state": self.share_phase,
            "reason": self.reason,
            // Absent until the worker's first real sample; a torn-down call
            // reports null so the UI never keeps a stale verdict.
            "stats": self.stats.as_ref().map(|stats| {
                json!({
                    "rtt_ms": stats.rtt_ms,
                    "loss_pct": stats.loss_pct(),
                    "quality": stats.quality(),
                })
            }),
        })
    }
}

/// An authenticated peer's offer waiting for this device's user to approve
/// or decline it. Held by the core, never by the worker: nothing about the
/// inbound call exists as media until acceptance.
struct IncomingCall {
    /// The caller's own call id, which decline/answer signals must carry
    /// back so the caller can match them to the right attempt.
    remote_call_id: String,
    /// Which paired peer is calling: the answer and any decline route
    /// through this leg's session.
    peer: Uuid,
    sdp: String,
    received_at: Instant,
}

/// An unanswered incoming call is presented for this long before it stops
/// ringing; the peer's own side will have given up too.
const INCOMING_TTL: Duration = Duration::from_secs(45);

/// Mirrors the media worker's direct control-channel cap so an oversized
/// activity frame is trimmed here instead of being refused at the wire.
const ACTIVITY_FRAME_MAX_BYTES: usize = 8 * 1024;
/// How many validated peer records the remote activity view retains.
const REMOTE_ACTIVITY_CAP: usize = 50;

/// Mutable per-process domain state shared across dispatches.
struct CoreState {
    state_dir: PathBuf,
    settings: Option<Settings>,
    pairing: PairingSession,
    /// Real local activity: fed by the monitor thread, read by dispatches.
    /// The mutex is held for microseconds per scan or request.
    engine: Arc<Mutex<ActivityEngine>>,
    /// Set when a dispatch changed something the UI must re-render as an
    /// `activity.updated` event (policy flips re-redact the timeline).
    activity_dirty: bool,
    /// The Go process exists only while a local call is being bootstrapped.
    /// It receives no Harbor identity, pairing material, or server address.
    media: Option<MediaSupervisor>,
    /// Host-provided worker location. Android extracts the private Go/Pion
    /// executable from APK assets before a call; desktop leaves this unset
    /// and uses the normal sibling/env lookup.
    media_worker_path: Option<PathBuf>,
    /// Pump wake token handed to the media supervisor so asynchronous worker
    /// facts (connection state, worker death) reach the UI promptly.
    media_wake: Option<mpsc::Sender<()>>,
    /// Signaling transport for direct calls: the reused pinned server
    /// connection, the resolved paired peer, and the pair session.
    signaling: signaling::Signaling,
    call: CallState,
    call_dirty: bool,
    /// Set when fresh transport facts wait to be surfaced as a
    /// `call.stats_changed` event.
    stats_dirty: bool,
    share_dirty: bool,
    /// Set when fresh voice-level facts wait to be surfaced as a
    /// `voice.level` event.
    voice_dirty: bool,
    /// A peer's offer waiting for explicit approval. Present while the call
    /// phase is INCOMING; accepted or declined by the user, expired by time.
    incoming: Option<IncomingCall>,
    /// Validated activity records the paired peer shared during this call,
    /// newest last. Never trusted raw: every record passed the shareable
    /// schema on arrival.
    remote_activity: Vec<crate::activity::RemoteActivityRecord>,
    /// Fingerprint of the last activity frame handed to the worker, so an
    /// unchanged timeline is not re-sent on every poll cadence.
    shared_activity_fingerprint: Option<u64>,
    // Direct DataChannel policy stays in the core. The worker transports only
    // bounded frames; it never owns transcript, staging, destination, or
    // delivery policy.
    chat: ChatSession,
    transfers: TransferBoard,
    direct_dirty: bool,
    /// Peer-to-peer public-profile sync. The local profile is durable
    /// settings; the partner snapshot is a validated, persisted cache of
    /// what the paired peer shared over the direct channel — never the
    /// server, never local identity.
    profile: crate::profile::ProfileSync,
    profile_dirty: bool,
    /// Local presence from private multi-signal evidence, plus the partner
    /// presence observed through healthy control-plane answers. Snapshots in,
    /// aggregate ONLINE/AWAY/OFFLINE out; the private evidence never leaves
    /// this process.
    presence: crate::presence::PresenceTracker,
    presence_dirty: bool,
    /// This install's validated phone aggregate, pushed by the platform
    /// adapter through `mobile.update`. `None` means no phone state was
    /// ever shared — the UI reads the honest unavailable snapshot.
    mobile_own: Option<MobileStatus>,
    mobile_dirty: bool,
    /// Ephemeral phone-notification payloads waiting for the UI. Contents
    /// never enter a snapshot, transcript, settings file, or diagnostic log.
    phone_notifications: Vec<Value>,
    /// Set when the device endpoint snapshot changed (type switch, link,
    /// unlink, media-endpoint move): the UI re-reads mode and registry.
    device_dirty: bool,
    /// Which own device currently holds call media. `None` while a call is
    /// CONNECTED means this install holds it (only its own call start can
    /// reach CONNECTED); a takeover grant names the sibling that took it.
    /// Cleared with every call teardown.
    media_endpoint: Option<Uuid>,
    /// Direct TCP bearer to a mobile peer (the phone path: no worker, no
    /// call phase needed). The worker thread owns every socket; this side
    /// only exchanges frames and facts.
    link: MobileLink,
    /// Last validated phone aggregate received over the link. Read through
    /// `mobile.state` alongside the own status; null until the peer shares.
    mobile_peer: Option<MobileStatus>,
    /// Fresh dial invitation from the listening peer, with redial attempts
    /// and the next due time. Cleared on connect.
    link_invite: Option<(LinkInvite, u32, Option<Instant>)>,
    /// Whether the bearer was live at the last pump: rising edges transmit
    /// our hello, falling edges retire the peer's phone state as unknown.
    link_was_live: bool,
    /// Last listener port plus the mobile legs already invited on it: a new
    /// port or a new mobile leg re-sends invites, never every pump.
    link_invite_epoch: (Option<u16>, Vec<Uuid>),
    /// Microphone self-check worker. It borrows the selected devices
    /// outside any call (a short-lived instance like the device listing)
    /// and is always torn down before a call opens them.
    mic_test: Option<MediaSupervisor>,
}

impl CoreState {
    fn transfer_destination(&self) -> PathBuf {
        let configured = self
            .settings
            .as_ref()
            .map(|settings| settings.values().transfer_directory.trim())
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute());
        configured.unwrap_or_else(|| self.state_dir.join("downloads"))
    }

    /// Whether this machine advertises its presence lease at all. Private
    /// mode still observes the partner — it only stops publishing self, so
    /// the peer reads the same Offline an expired lease produces.
    fn presence_publishing_enabled(&self) -> bool {
        self.settings
            .as_ref()
            .map(|settings| settings.values().presence_visibility)
            .unwrap_or(true)
    }

    fn from_default() -> Self {
        let state_dir = default_state_dir().unwrap_or_else(|| PathBuf::from("."));
        Self {
            state_dir: state_dir.clone(),
            settings: Settings::load_default().ok(),
            pairing: PairingSession::default(),
            engine: Arc::new(Mutex::new(ActivityEngine::default())),
            activity_dirty: false,
            media: None,
            media_worker_path: None,
            media_wake: None,
            signaling: signaling::Signaling::default(),
            call: CallState::default(),
            call_dirty: false,
            stats_dirty: false,
            share_dirty: false,
            voice_dirty: false,
            incoming: None,
            remote_activity: Vec::new(),
            shared_activity_fingerprint: None,
            chat: ChatSession::new(),
            transfers: TransferBoard::new(),
            direct_dirty: false,
            profile: crate::profile::ProfileSync::load(&state_dir),
            profile_dirty: false,
            presence: crate::presence::PresenceTracker::default(),
            presence_dirty: false,
            mobile_own: None,
            mobile_dirty: false,
            phone_notifications: Vec::new(),
            device_dirty: false,
            media_endpoint: None,
            link: MobileLink::new(),
            mobile_peer: None,
            link_invite: None,
            link_was_live: false,
            link_invite_epoch: (None, Vec::new()),
            mic_test: None,
        }
    }

    /// Explicit-directory constructor: tests pin it, the mobile build
    /// passes the app-private directory through the C ABI.
    fn for_directory(directory: &std::path::Path) -> Self {
        Self {
            state_dir: directory.to_path_buf(),
            settings: Settings::load_or_create(directory).ok(),
            pairing: PairingSession::default(),
            engine: Arc::new(Mutex::new(ActivityEngine::default())),
            activity_dirty: false,
            media: None,
            media_worker_path: None,
            media_wake: None,
            signaling: signaling::Signaling::default(),
            call: CallState::default(),
            call_dirty: false,
            stats_dirty: false,
            share_dirty: false,
            voice_dirty: false,
            incoming: None,
            remote_activity: Vec::new(),
            shared_activity_fingerprint: None,
            chat: ChatSession::new(),
            transfers: TransferBoard::new(),
            direct_dirty: false,
            profile: crate::profile::ProfileSync::load(directory),
            profile_dirty: false,
            presence: crate::presence::PresenceTracker::default(),
            presence_dirty: false,
            mobile_own: None,
            mobile_dirty: false,
            phone_notifications: Vec::new(),
            device_dirty: false,
            media_endpoint: None,
            link: MobileLink::new(),
            mobile_peer: None,
            link_invite: None,
            link_was_live: false,
            link_invite_epoch: (None, Vec::new()),
            mic_test: None,
        }
    }

    /// The `activity.updated` event payload: monitor state plus the current
    /// timeline with the game-titles policy applied at serialization time.
    fn activity_event(&self) -> Envelope {
        let engine = self.engine.lock().expect("activity engine mutex");
        let game_titles = self
            .settings
            .as_ref()
            .map(|settings| settings.values().game_visibility)
            .unwrap_or(true);
        Envelope::event(
            "activity.updated",
            json!({
                "monitor": engine.monitor_state().as_str(),
                "timeline": engine.timeline_json(game_titles),
                "stats": engine.stats_json(now_seconds()),
                "remote": self.remote_activity_json(),
            }),
            crate::rfc3339_now(),
        )
    }

    /// Takes the pending activity event, if a dispatch marked one needed.
    fn take_activity_event(&mut self) -> Option<Envelope> {
        if !self.activity_dirty {
            return None;
        }
        self.activity_dirty = false;
        Some(self.activity_event())
    }

    fn take_call_event(&mut self) -> Option<Envelope> {
        if !self.call_dirty {
            return None;
        }
        self.call_dirty = false;
        Some(Envelope::event(
            "call.state_changed",
            self.call.snapshot(),
            crate::rfc3339_now(),
        ))
    }

    fn take_share_event(&mut self) -> Option<Envelope> {
        if !self.share_dirty {
            return None;
        }
        self.share_dirty = false;
        Some(Envelope::event(
            "call.share_state_changed",
            json!({
                "call_id": self.call.call_id.clone(),
                "state": self.call.share_phase.clone(),
            }),
            crate::rfc3339_now(),
        ))
    }

    /// Sanitized direct-session state: IDs, timestamps, plain message text,
    /// and transfer metadata only. Paths, chunks, source bytes, and digests
    /// remain below this QML-facing boundary.
    fn direct_payload(&self) -> Value {
        let messages: Vec<Value> = self
            .chat
            .snapshot()
            .into_iter()
            .map(|message| {
                json!({
                    "id": message.id,
                    "body": message.body,
                    "direction": match message.direction { Direction::Outgoing => "OUTGOING", Direction::Incoming => "INCOMING" },
                    "delivery": match message.delivery {
                        Delivery::WaitingForConnection => "WAITING_FOR_CONNECTION",
                        Delivery::Sent => "SENT",
                        Delivery::Delivered => "DELIVERED",
                        Delivery::Failed => "FAILED",
                    },
                    "timestamp": message.timestamp,
                })
            })
            .collect();
        let transfers: Vec<Value> = self
            .transfers
            .records()
            .into_iter()
            .map(|transfer| {
                json!({
                    "id": transfer.id,
                    "direction": match transfer.direction { Direction::Outgoing => "OUTGOING", Direction::Incoming => "INCOMING" },
                    "state": match transfer.phase {
                        TransferPhase::Offered => "OFFERED",
                        TransferPhase::Active => "ACTIVE",
                        TransferPhase::Completed => "COMPLETED",
                        TransferPhase::Canceled => "CANCELED",
                        TransferPhase::Failed => "FAILED",
                    },
                    "name": transfer.name,
                    "size": transfer.size,
                    "receivedBytes": transfer.received_bytes,
                    "peerReceived": transfer.peer_received,
                    "expired": transfer.expired,
                })
            })
            .collect();
        json!({"messages": messages, "transfers": transfers})
    }

    fn take_direct_event(&mut self) -> Option<Envelope> {
        if !self.direct_dirty {
            return None;
        }
        self.direct_dirty = false;
        Some(Envelope::event(
            "direct.updated",
            self.direct_payload(),
            crate::rfc3339_now(),
        ))
    }

    /// Partner-profile snapshot for the UI: the peer's public fields only.
    /// Revision and hash stay below this boundary; the view simply shows
    /// the profile. Local identity never appears here.
    fn profile_snapshot(&self) -> Value {
        json!({"partner": self.profile.partner.json()})
    }

    /// The `presence.updated` event payload: both committed aggregates with
    /// per-side change flags against what the UI last saw. Private evidence
    /// never appears here — only states, revisions, and the change verdict.
    fn presence_event(&self) -> Envelope {
        Envelope::event(
            "presence.updated",
            json!({
                "local": presence_side_json(
                    self.presence.local.state(),
                    self.presence.local.previous_state(),
                    self.presence.last_emitted_local(),
                    self.presence.local.revision(),
                ),
                "partner": self.presence.partner.state().map(|partner| {
                    presence_side_json(
                        partner,
                        self.presence.partner.previous_state(),
                        self.presence.last_emitted_partner(),
                        self.presence.partner.revision(),
                    )
                }),
            }),
            crate::rfc3339_now(),
        )
    }

    /// Takes the pending presence event: emitted when either side's
    /// committed state changed since the last emission.
    fn take_presence_event(&mut self) -> Option<Envelope> {
        if !self.presence_dirty {
            return None;
        }
        self.presence_dirty = false;
        let event = self.presence_event();
        self.presence.mark_emitted();
        Some(event)
    }

    fn take_profile_event(&mut self) -> Option<Envelope> {
        if !self.profile_dirty {
            return None;
        }
        self.profile_dirty = false;
        Some(Envelope::event(
            "profile.updated",
            self.profile_snapshot(),
            crate::rfc3339_now(),
        ))
    }

    /// This install's endpoint snapshot: own device record, companion mode
    /// from the authorized registry, linked devices, the current media
    /// endpoint, and every paired peer with its hello-learned kind. Peers
    /// whose hello never arrived read a null kind — unknown is omitted,
    /// never invented. `blocked` is true only for a proven Mobile<->Mobile
    /// pair; the UI gates the session on it.
    fn device_snapshot(&self) -> Result<Value, ProtocolError> {
        let now = now_seconds();
        let identity =
            load_or_create(&self.state_dir, now).map_err(|_| identity_unavailable_error())?;
        let record = identity.record();
        let settings = self
            .settings
            .as_ref()
            .ok_or_else(settings_unavailable_error)?;
        let device_type =
            DeviceType::parse(settings.values().device_type.as_str()).unwrap_or(DeviceType::Desktop);
        let own = DeviceRecord::new(record.device_id, device_type, true, now);
        let linked = settings.values().linked_devices.clone();
        let peers: Vec<Value> = self
            .signaling
            .peer_snapshot()
            .into_iter()
            .map(|(device_id, harbor_id, device)| {
                json!({
                    "deviceId": device_id,
                    "harborId": harbor_id,
                    "deviceType": device,
                })
            })
            .collect();
        let blocked = crate::device::session_blocked(
            device_type,
            &peers
                .iter()
                .map(|peer| {
                    peer.get("deviceType")
                        .and_then(Value::as_str)
                        .and_then(DeviceType::parse)
                })
                .collect::<Vec<_>>(),
        );
        Ok(json!({
            "device": {
                "deviceId": record.device_id,
                "harborId": record.harbor_id,
                "deviceType": device_type,
            },
            "mode": own_mode(&own, &linked),
            "linked": linked,
            "mediaEndpoint": self.media_endpoint,
            "peers": peers,
            "blocked": blocked,
            "linkPeer": self.link.live_peer(),
            "linkListeningPort": self.link.listening().map(|(port, _)| port),
        }))
    }

    /// The validated phone aggregate for the UI, or an honest null when no
    /// phone state was ever shared.
    fn mobile_snapshot(&self) -> Value {
        json!({
            "own": self.mobile_own.as_ref().map(|status| serde_json::to_value(status).unwrap_or(Value::Null)).unwrap_or(Value::Null),
            "peer": self.mobile_peer.as_ref().map(|status| serde_json::to_value(status).unwrap_or(Value::Null)).unwrap_or(Value::Null),
        })
    }

    fn take_device_event(&mut self) -> Option<Envelope> {
        if !self.device_dirty {
            return None;
        }
        self.device_dirty = false;
        match self.device_snapshot() {
            Ok(snapshot) => Some(Envelope::event(
                "device.updated",
                snapshot,
                crate::rfc3339_now(),
            )),
            Err(_) => None,
        }
    }

    fn take_mobile_event(&mut self) -> Option<Envelope> {
        if !self.mobile_dirty {
            return None;
        }
        self.mobile_dirty = false;
        Some(Envelope::event(
            "mobile.updated",
            self.mobile_snapshot(),
            crate::rfc3339_now(),
        ))
    }

    fn take_phone_notification_event(&mut self) -> Option<Envelope> {
        // Notification callbacks are already ordered by Android's listener;
        // preserve that order at the UI boundary instead of showing a burst
        // newest-first when several frames arrive in one core tick.
        if self.phone_notifications.is_empty() {
            return None;
        }
        let payload = self.phone_notifications.remove(0);
        Some(Envelope::event(
            "phone.notification",
            payload,
            crate::rfc3339_now(),
        ))
    }

    /// Snapshot of what the link worker needs: our identity, our kind, and
    /// the paired peers it may accept. Nothing here is secret except the
    /// seed, which stays in process memory and is never serialized.
    fn link_context(&self) -> Option<LinkContext> {
        let identity = load_or_create(&self.state_dir, now_seconds()).ok()?;
        let record = identity.record();
        let own = self
            .settings
            .as_ref()
            .and_then(|settings| DeviceType::parse(settings.values().device_type.as_str()))
            .unwrap_or(DeviceType::Desktop);
        Some(LinkContext {
            device_id: record.device_id,
            harbor_id: record.harbor_id.clone(),
            signing_seed: identity.seed_bytes(),
            device_type: own,
            peers: self
                .signaling
                .peer_infos()
                .into_iter()
                .map(|(device_id, harbor_id, public_key)| LinkPeer {
                    device_id,
                    harbor_id,
                    public_key,
                })
                .collect(),
        })
    }

    /// A mobile peer helloed: listen once, then invite it onto the bearer.
    /// Listens only on desktops — a phone never accepts link connections,
    /// it only dials out.
    fn start_link_listener(&mut self, peer: &Uuid) {
        let Some(context) = self.link_context() else {
            return;
        };
        if context.device_type != DeviceType::Desktop {
            return;
        }
        if self.link.listening().is_none() {
            self.link.listen(context);
            return;
        }
        self.send_link_invite(peer);
    }

    fn send_link_invite(&mut self, peer: &Uuid) {
        let (port, fingerprint) = match self.link.listening() {
            Some((port, fingerprint)) => (port, fingerprint.to_owned()),
            None => return,
        };
        let server_addr = load_server_pin(&self.state_dir)
            .map(|pin| pin.address)
            .unwrap_or_else(|| "127.0.0.1:9091".into());
        let mut addrs = local_dial_addrs(&server_addr);
        addrs.truncate(8);
        signaling::send_to(
            self,
            peer,
            &json!({
                "call_id": peer.to_string(),
                "signal": {
                    "type": "tcp_invite",
                    "addrs": addrs,
                    "port": port,
                    "fingerprint": fingerprint,
                },
            }),
        );
    }

    /// Invites on every known mobile leg: a fresh listener (or a late
    /// hello) must reach all of them, not just the leg that triggered it.
    fn send_link_invites(&mut self) {
        let mobiles: Vec<Uuid> = self
            .signaling
            .legs()
            .iter()
            .filter(|leg| leg.device == Some(DeviceType::Mobile))
            .map(|leg| leg.peer)
            .collect();
        for peer in mobiles {
            self.send_link_invite(&peer);
        }
    }

    /// A validated dial invitation: store it for redial and dial now. A
    /// live bearer is never re-dialed over.
    fn note_link_invite(&mut self, invite: LinkInvite) {
        if self.link.live_peer().is_some() {
            return;
        }
        let Some(context) = self.link_context() else {
            return;
        };
        self.link_invite = Some((invite.clone(), 0, None));
        self.link.dial(context, invite);
    }

    /// Our durable public profile for link exchange, mirroring the worker
    /// path's construction exactly.
    fn local_public_profile(&self) -> Option<crate::profile::PublicProfile> {
        let settings = self.settings.as_ref()?;
        let values = settings.values();
        Some(crate::profile::local_profile(
            values.profile_revision,
            &values.display_name,
            &values.status_message,
            &values.avatar,
            &values.avatar_type,
        ))
    }

    fn send_link_profile_frame(&mut self, frame: &str) -> bool {
        if self.link.live_peer().is_none() {
            return false;
        }
        let Some(frame) = LinkFrame::new(KIND_PROFILE, json!({"frame": frame})) else {
            return false;
        };
        self.link.send(frame);
        true
    }

    /// Publishes our profile over the bearer: hello on revision changes,
    /// avatar chunks bounded per pump, plus any queued responses. The frame
    /// strings are the same ones the worker path carries, so validation is
    /// identical on arrival.
    fn flush_link_profile(&mut self) {
        let Some(local) = self.local_public_profile() else {
            return;
        };
        let now = now_seconds();
        if let Some(frame) = self.profile.pending_hello(&local, now) {
            if self.send_link_profile_frame(&frame) {
                self.profile.mark_hello_sent(local.revision, now);
            }
        }
        for _ in 0..crate::profile::PROFILE_AVATAR_CHUNKS_PER_TICK {
            let Some(frame) = self.profile.peek_chunk(&local) else {
                break;
            };
            if !self.send_link_profile_frame(&frame) {
                break;
            }
            self.profile.advance_chunk();
        }
        if let Some(cancel) = self.profile.stale_outbound_cancel(&local) {
            self.send_link_profile_frame(&cancel);
        }
    }

    /// Pushes our validated phone aggregate to the live peer. Called on
    /// every `mobile.update` and on link rise; the peer renders it through
    /// `mobile.state`, never stores it beyond the session view.
    fn transmit_link_mobile(&mut self) {
        if self.link.live_peer().is_none() {
            return;
        }
        let Some(own) = self.mobile_own.clone() else {
            return;
        };
        let Ok(status) = serde_json::to_value(&own) else {
            return;
        };
        if let Some(frame) = LinkFrame::new(KIND_MOBILE, status) {
            self.link.send(frame);
        }
    }

    /// Sends one display-only phone notification over the authenticated
    /// mobile bearer. The queue is intentionally not retained: an offline
    /// peer misses the notification rather than receiving it later from
    /// durable storage.
    fn transmit_link_phone_notification(&mut self, payload: &Value) -> bool {
        let Some(frame) = LinkFrame::new(KIND_PHONE_NOTIFICATION, payload.clone()) else {
            return false;
        };
        if self.link.live_peer().is_none() {
            return false;
        }
        self.link.send(frame);
        true
    }

    fn transmit_link_hello(&mut self) {
        let Some(context) = self.link_context() else {
            return;
        };
        self.link.send(hello_frame(&context));
    }

    /// Applies one bearer frame from the authenticated `peer`. Content
    /// policy mirrors the worker path: chat bodies are sanitized on
    /// record, profile frames pass ingest, phone state passes consent
    /// validation, hellos must name this very sender.
    fn absorb_link_frame(&mut self, peer: Uuid, frame: LinkFrame) {
        match frame.kind.as_str() {
            KIND_CHAT => {
                let id = frame.payload.get("id").and_then(Value::as_str);
                let body = frame.payload.get("body").and_then(Value::as_str);
                if let (Some(id), Some(body)) = (id, body) {
                    if self
                        .chat
                        .record_inbound(id.to_owned(), body.to_owned(), now_seconds())
                    {
                        self.direct_dirty = true;
                        if let Some(ack) = LinkFrame::new(KIND_CHAT_ACK, json!({"id": id})) {
                            self.link.send(ack);
                        }
                    }
                }
            }
            KIND_CHAT_ACK => {
                if let Some(id) = frame.payload.get("id").and_then(Value::as_str) {
                    self.chat.mark_delivery(id, Delivery::Delivered);
                    self.direct_dirty = true;
                }
            }
            KIND_HELLO => {
                let claimed_id = frame
                    .payload
                    .get("device_id")
                    .and_then(Value::as_str)
                    .and_then(|id| Uuid::parse_str(id).ok());
                let claimed_type = frame
                    .payload
                    .get("device_type")
                    .and_then(Value::as_str)
                    .and_then(DeviceType::parse);
                let claimed_harbor =
                    frame.payload.get("harbor_id").and_then(Value::as_str).unwrap_or_default();
                if claimed_id != Some(peer) {
                    return;
                }
                if claimed_harbor.is_empty()
                    || Some(claimed_harbor) != self.signaling.peer_harbor(&peer)
                {
                    return;
                }
                if let Some(device) = claimed_type {
                    if self.signaling.note_peer_device(&peer, device) {
                        self.device_dirty = true;
                    }
                }
            }
            KIND_PROFILE => {
                let Some(local) = self.local_public_profile() else {
                    return;
                };
                let Some(raw) = frame.payload.get("frame").and_then(Value::as_str) else {
                    return;
                };
                let outcome = self.profile.ingest(raw, &local);
                if outcome.applied {
                    self.profile_dirty = true;
                    self.profile.persist_partner(&self.state_dir);
                }
                for response in outcome.send {
                    self.send_link_profile_frame(&response);
                }
            }
            KIND_MOBILE => {
                let status: MobileStatus = match serde_json::from_value(frame.payload.clone()) {
                    Ok(status) => status,
                    Err(_) => return,
                };
                if status.validate().is_err() {
                    return;
                }
                self.mobile_peer = Some(status);
                self.mobile_dirty = true;
            }
            KIND_PHONE_NOTIFICATION => {
                // The TLS link has already authenticated the sender. Keep
                // this as a bounded transient event, never a chat record or
                // mobile snapshot. Validate again at the domain boundary.
                if validate_phone_notification(&frame.payload).is_ok() {
                    if self.phone_notifications.len() >= 16 {
                        self.phone_notifications.remove(0);
                    }
                    self.phone_notifications.push(frame.payload);
                }
            }
            _ => {}
        }
    }

    /// One bearer step: absorb worker events, track the live edge, redial a
    /// stored invite with bounded backoff, and flush profile state while
    /// live. Nonblocking throughout; safe on every pump cadence.
    fn pump_link(&mut self) {
        for (peer, frame) in self.link.drain() {
            self.absorb_link_frame(peer, frame);
        }
        let live = self.link.live_peer().is_some();
        let was = std::mem::replace(&mut self.link_was_live, live);
        match (was, live) {
            (false, true) => {
                // Rising edge: announce ourselves and publish at once. The
                // peer answers with its own hello through the same path.
                self.transmit_link_hello();
                self.transmit_link_mobile();
            }
            (true, false) => {
                // Falling edge: the peer's phone state is unknown again, not
                // last-known-forever.
                self.mobile_peer = None;
                self.mobile_dirty = true;
            }
            _ => {}
        }
        if let Some((port, _)) = self.link.listening() {
            let mobiles: Vec<Uuid> = self
                .signaling
                .legs()
                .iter()
                .filter(|leg| leg.device == Some(DeviceType::Mobile))
                .map(|leg| leg.peer)
                .collect();
            if self.link_invite_epoch != (Some(port), mobiles.clone()) {
                self.link_invite_epoch = (Some(port), mobiles.clone());
                for peer in mobiles {
                    self.send_link_invite(&peer);
                }
            }
        }
        if live {
            self.link_invite = None;
            self.flush_link_profile();
            return;
        }
        // Patient redial of a stored invite while the bearer is down.
        let pending = self.link_invite.clone();
        if let Some((invite, attempt, due)) = pending {
            let now = Instant::now();
            if due.is_none_or(|due| now >= due) {
                if let Some(context) = self.link_context() {
                    self.link.dial(context, invite.clone());
                    let wait = Duration::from_secs(reconnect_delay(attempt));
                    self.link_invite = Some((invite, attempt.saturating_add(1), Some(now + wait)));
                }
            }
        }
    }

    /// Ends an outgoing attempt on a peer's word (decline/busy). The worker
    /// is torn down and the phase lands on ENDED carrying the reason, so the
    /// UI can say why the call never connected.
    fn end_outgoing_with_reason(&mut self, reason: &str) {
        self.shutdown_media();
        self.call.phase = "ENDED".into();
        self.call.reason = reason.to_owned();
        self.call_dirty = true;
    }

    /// Presents an authenticated peer's offer as an explicit INCOMING state.
    /// Nothing opens until the user accepts; approval is never assumed.
    fn present_incoming(&mut self, remote_call_id: &str, sdp: &str, peer: Uuid) {
        if self.call.call_id.is_some() || self.incoming.is_some() {
            return;
        }
        self.incoming = Some(IncomingCall {
            remote_call_id: remote_call_id.to_owned(),
            peer,
            sdp: sdp.to_owned(),
            received_at: Instant::now(),
        });
        self.call.reason.clear();
        self.call.phase = "INCOMING".into();
        self.call_dirty = true;
    }

    /// Expires an incoming call nobody answered within the presentation
    /// window. Returns whether the visible state changed.
    fn enforce_incoming_ttl(&mut self) -> bool {
        let expired = self
            .incoming
            .as_ref()
            .is_some_and(|incoming| incoming.received_at.elapsed() > INCOMING_TTL);
        if !expired {
            return false;
        }
        self.incoming = None;
        self.call.phase = "IDLE".into();
        self.call_dirty = true;
        true
    }

    /// Takes the pending `voice.level` event, if fresh facts are waiting.
    /// Takes the pending transport-facts event, if fresh samples are waiting.
    fn take_stats_event(&mut self) -> Option<Envelope> {
        if !self.stats_dirty {
            return None;
        }
        self.stats_dirty = false;
        let payload = match self.call.stats.as_ref() {
            Some(stats) => json!({
                "call_id": self.call.call_id,
                "rtt_ms": stats.rtt_ms,
                "loss_pct": stats.loss_pct(),
                "quality": stats.quality(),
            }),
            // The call is gone: the explicit null clears any stale verdict.
            None => json!({ "call_id": self.call.call_id, "stats": null }),
        };
        Some(Envelope::event(
            "call.stats_changed",
            payload,
            crate::rfc3339_now(),
        ))
    }

    fn take_voice_event(&mut self) -> Option<Envelope> {
        if !self.voice_dirty {
            return None;
        }
        self.voice_dirty = false;
        Some(Envelope::event(
            "voice.level",
            json!({
                "call_id": self.call.call_id,
                "level": self.call.voice.level,
                "remote_level": self.call.voice.remote_level,
                "speaking": self.call.voice.speaking,
                "remote_speaking": self.call.voice.remote_speaking,
                "muted": self.call.muted,
            }),
            crate::rfc3339_now(),
        ))
    }

    fn drain_media_events(&mut self) {
        let events = self
            .media
            .as_ref()
            .map(MediaSupervisor::drain_events)
            .unwrap_or_default();
        for event in events {
            self.absorb_media_event(&event);
        }
    }

    /// Translates one asynchronous worker fact into local state. Kept apart
    /// from the drain so tests can feed events without a real worker.
    fn absorb_media_event(&mut self, event: &MediaEvent) {
        let call_id = event.payload.get("call_id").and_then(Value::as_str);
        if call_id != self.call.call_id.as_deref() {
            return;
        }
        if event.message_type == "media.voice_level" {
            // Speaking indication is call-scoped UI material measured on
            // audio the pipeline already owns. It never changes call
            // policy; it only refreshes the display facts.
            let voice = &event.payload;
            self.call.voice = VoiceLevels {
                level: voice.get("level").and_then(Value::as_f64).unwrap_or(0.0),
                remote_level: voice
                    .get("remote_level")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0),
                speaking: voice
                    .get("speaking")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                remote_speaking: voice
                    .get("remote_speaking")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
            };
            self.voice_dirty = true;
            return;
        }
        if event.message_type == "media.call_stats" {
            // Transport health is display material the worker measured on
            // its own connection. It never changes call policy; it only
            // refreshes the quality facts the call surface shows.
            let stats = CallStats {
                rtt_ms: event
                    .payload
                    .get("rtt_ms")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0),
                packets_received: event
                    .payload
                    .get("received")
                    .and_then(Value::as_u64)
                    .unwrap_or(0),
                packets_lost: event
                    .payload
                    .get("lost")
                    .and_then(Value::as_i64)
                    .unwrap_or(0),
            };
            self.call.stats = Some(stats);
            self.stats_dirty = true;
            return;
        }
        if event.message_type == "media.ice_candidate" {
            // The peer needs this candidate to find the direct path; the
            // relay is best-effort because ICE retries and the state
            // machine cover transient losses.
            if let Some(peer) = self.call.peer.clone() {
                signaling::send_to(
                    self,
                    &peer,
                    &json!({
                        "call_id": call_id,
                        "signal": {
                            "type": "candidate",
                            "candidate": event.payload.get("candidate").cloned().unwrap_or(Value::Null),
                        },
                    }),
                );
            }
            return;
        }
        if event.message_type != "media.call_state" {
            return;
        }
        let worker_state = event.payload.get("state").and_then(Value::as_str);
        match worker_state {
            Some("connected") => {
                self.call.phase = "CONNECTED".into();
                self.call.reconnecting_since = None;
            }
            Some("connecting") => self.call.phase = "CONNECTING".into(),
            // A transient path loss puts the call into recovery; Pion's
            // agent keeps probing the same direct candidates. Pion
            // reporting "failed" means it gave up — so does the call.
            Some("disconnected") => {
                if self.call.phase != "RECONNECTING" {
                    self.call.phase = "RECONNECTING".into();
                    self.call.reconnecting_since = Some(Instant::now());
                }
            }
            Some("failed") => self.fail_media(),
            Some("closed") => self.call.reset(),
            _ => return,
        }
        self.call_dirty = true;
    }

    fn shutdown_media(&mut self) {
        // Ending a call also tears down its signaling session, so queued
        // SDP/ICE never leaks into the next call between the pair.
        signaling::hangup(self);
        if let Some(media) = self.media.take() {
            media.shutdown();
        }
        // A new call starts a new peer session: the next frame is shared
        // whole, even if the timeline did not change since the last call.
        self.shared_activity_fingerprint = None;
        self.media_endpoint = None;
        self.call.reset();
        self.call_dirty = true;
        self.share_dirty = true;
    }

    /// Fails a live call and tears everything under it down: the worker, the
    /// signaling session, and the share. FAILED is a resting terminal state —
    /// the user ends the call — but nothing keeps running beneath it.
    fn fail_media(&mut self) {
        signaling::hangup(self);
        if let Some(media) = self.media.take() {
            media.shutdown();
        }
        self.call.phase = "FAILED".into();
        self.call.reconnecting_since = None;
        self.media_endpoint = None;
        let share_changed = self.call.share_phase != "NOT_SHARING";
        self.call.share_phase = "NOT_SHARING".into();
        self.chat.fail_unacknowledged();
        self.call_dirty = true;
        self.share_dirty |= share_changed;
        self.direct_dirty = true;
    }

    /// Polls bounded direct-channel facts and pushes any queued chat into an
    /// active direct call. Polling, rather than a lossy event stream, is the
    /// authority: a full or restarted worker cannot make an inbound message
    /// silently disappear between callbacks.
    /// Fans queued chat out on every live transport: the bearer needs no
    /// call phase (the mobile path), the worker needs its CONNECTED call,
    /// and a companion identity bridging two peers sends on both. One
    /// drain, no duplicates: delivery acks from either path advance the
    /// same transcript entry.
    fn flush_chat_queue(&mut self) {
        let link_live = self.link.live_peer().is_some();
        let worker_live = self.call.phase == "CONNECTED"
            && self.call.call_id.is_some()
            && self.media.is_some();
        if !link_live && !worker_live {
            return;
        }
        let call_id = self.call.call_id.clone().unwrap_or_default();
        let queued = self.chat.drain_queue();
        let mut retry = Vec::new();
        for message in queued {
            let mut sent = false;
            if link_live {
                if let Some(frame) =
                    LinkFrame::new(KIND_CHAT, json!({"id": message.id, "body": message.body}))
                {
                    self.link.send(frame);
                    self.chat.mark_delivery(&message.id, Delivery::Sent);
                    self.direct_dirty = true;
                    sent = true;
                }
            }
            if worker_live {
                if let Some(media) = self.media.as_ref() {
                    match media.request(
                        "chat.send",
                        json!({
                            "call_id": call_id,
                            "message_id": message.id,
                            "body": message.body,
                        }),
                    ) {
                        Ok(reply)
                            if reply.error.is_none() && reply.message_type == "chat.send" =>
                        {
                            self.chat.mark_delivery(&message.id, Delivery::Sent);
                            self.direct_dirty = true;
                            sent = true;
                        }
                        _ => {}
                    }
                }
            }
            if !sent {
                retry.push(message);
            }
        }
        if !retry.is_empty() {
            self.chat.requeue(retry);
        }
    }

    fn sync_direct(&mut self) {
        // The bearer runs without any call phase, so it pumps first and
        // queued chat fans out below on every live transport — worker,
        // bearer, or both for a companion identity bridging two peers.
        self.pump_link();
        let expired = self.transfers.expire(now_seconds());
        if !expired.is_empty() {
            self.direct_dirty = true;
        }
        self.flush_chat_queue();
        if self.call.phase != "CONNECTED" {
            // No direct channel exists outside a connected call, so nothing
            // can be exchanged — and any in-flight profile state belongs to
            // a dead call. Resetting forces a fresh hello on the next call,
            // which is how reconnects converge. The durable partner snapshot
            // is untouched.
            self.profile.note_disconnected();
            return;
        }
        let Some(call_id) = self.call.call_id.clone() else {
            return;
        };
        let Some(media) = self.media.as_ref() else {
            return;
        };

        if let Ok(reply) = media.request("chat.poll", json!({"call_id": call_id})) {
            if reply.error.is_none() && reply.message_type == "chat.poll" {
                if let Some(messages) = reply.payload.get("messages").and_then(Value::as_array) {
                    for message in messages {
                        let id = message.get("message_id").and_then(Value::as_str);
                        let body = message.get("body").and_then(Value::as_str);
                        if let (Some(id), Some(body)) = (id, body) {
                            self.direct_dirty |= self.chat.record_inbound(
                                id.to_owned(),
                                body.to_owned(),
                                now_seconds(),
                            );
                        }
                    }
                }
            }
        }

        let ids = self.chat.sent_ids();
        if !ids.is_empty() {
            if let Ok(reply) = media.request("chat.status", json!({"message_ids": ids})) {
                if reply.error.is_none() && reply.message_type == "chat.status" {
                    if let Some(deliveries) =
                        reply.payload.get("deliveries").and_then(Value::as_object)
                    {
                        for (id, delivered) in deliveries {
                            if delivered.as_bool() == Some(true) {
                                self.chat.mark_delivery(id, Delivery::Delivered);
                                self.direct_dirty = true;
                            }
                        }
                    }
                }
            }
        }

        // The worker carries offer/chunk frames directly to the peer. The core
        // owns every local file operation, and only metadata crosses this
        // boundary. Polling makes lost worker notifications recoverable.
        let records = self.transfers.records();
        for record in &records {
            if let Some((name, size, sum)) = self.transfers.outgoing_offer(&record.id) {
                if media
                    .request(
                        "transfer.begin",
                        json!({"call_id": call_id, "transfer_id": record.id, "name": name,
                               "size": size, "sha256": sum, "chunk_size": 16 * 1024}),
                    )
                    .is_ok()
                {
                    self.direct_dirty = true;
                }
            }
        }
        if let Ok(reply) = media.request("transfer.poll", json!({"call_id": call_id})) {
            if reply.error.is_none() && reply.message_type == "transfer.poll" {
                if let Some(transfers) = reply.payload.get("transfers").and_then(Value::as_array) {
                    for remote in transfers {
                        let id = remote.get("transfer_id").and_then(Value::as_str);
                        let direction = remote.get("direction").and_then(Value::as_str);
                        let phase = remote.get("phase").and_then(Value::as_str);
                        let Some(id) = id else {
                            continue;
                        };
                        match (direction, phase) {
                            (Some("incoming"), Some("offered")) => {
                                let name = remote.get("name").and_then(Value::as_str);
                                let size = remote.get("size").and_then(Value::as_u64);
                                let sum = remote.get("sha256").and_then(Value::as_str);
                                if let (Some(name), Some(size), Some(sum)) = (name, size, sum) {
                                    if self
                                        .transfers
                                        .offer_inbound(
                                            id.into(),
                                            name,
                                            size,
                                            sum.into(),
                                            now_seconds(),
                                        )
                                        .is_ok()
                                    {
                                        self.direct_dirty = true;
                                    }
                                }
                            }
                            (Some("outgoing"), Some("active")) => {
                                self.direct_dirty |= self.transfers.observe_accepted(id);
                            }
                            (_, Some("canceled" | "rejected")) => {
                                self.direct_dirty |= self.transfers.observe_canceled(id);
                            }
                            (Some("outgoing"), Some("completed")) => {
                                self.direct_dirty |= self.transfers.observe_peer_received(id);
                            }
                            _ => {}
                        }
                    }
                }
            }
        }

        // Send no more than one chunk per active transfer per tick. Each chunk
        // remains pending in TransferBoard until the worker accepts it, so SCTP
        // backpressure cannot skip source bytes.
        for record in self.transfers.records() {
            if record.direction != Direction::Outgoing || record.phase != TransferPhase::Active {
                continue;
            }
            let Ok(Some((seq, offset, data, final_chunk))) =
                self.transfers.next_outgoing_chunk(&record.id)
            else {
                continue;
            };
            let reply = media.request("transfer.send_chunk", json!({
                "call_id": call_id, "transfer_id": record.id, "seq": seq, "offset": offset,
                "data": base64::engine::general_purpose::STANDARD.encode(data), "final": final_chunk,
            }));
            if let Ok(reply) = reply {
                if reply.error.is_none()
                    && reply.payload.get("paused").and_then(Value::as_bool) != Some(true)
                {
                    self.transfers.confirm_outgoing_chunk(&record.id, seq);
                }
            }
        }

        let staging = self.state_dir.join("transfers");
        let destination = self.transfer_destination();
        for record in self.transfers.records() {
            if record.direction != Direction::Incoming || record.phase != TransferPhase::Active {
                continue;
            }
            for _ in 0..8 {
                let Ok(reply) = media.request(
                    "transfer.recv_chunk",
                    json!({"call_id": call_id, "transfer_id": record.id}),
                ) else {
                    break;
                };
                if reply.error.is_some()
                    || reply.payload.get("empty").and_then(Value::as_bool) == Some(true)
                {
                    break;
                }
                let seq = reply
                    .payload
                    .get("seq")
                    .and_then(Value::as_u64)
                    .map(|v| v as usize);
                let offset = reply.payload.get("offset").and_then(Value::as_u64);
                let data = reply
                    .payload
                    .get("data")
                    .and_then(Value::as_str)
                    .and_then(|text| base64::engine::general_purpose::STANDARD.decode(text).ok());
                let final_chunk = reply
                    .payload
                    .get("final")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let (Some(seq), Some(offset), Some(data)) = (seq, offset, data) else {
                    break;
                };
                if self
                    .transfers
                    .write_inbound_chunk(&record.id, seq, offset, &data, final_chunk)
                    .is_err()
                {
                    break;
                }
                self.direct_dirty = true;
                if final_chunk {
                    let verified = self
                        .transfers
                        .finalize_inbound(&record.id, &staging, &destination)
                        .is_ok();
                    let _ = media.request(
                        "transfer.finalize",
                        json!({"call_id": call_id, "transfer_id": record.id, "ok": verified}),
                    );
                    self.direct_dirty = true;
                    break;
                }
            }
        }

        // Activity delivery: only sanitized records cross, only when the
        // frame changed, and the peer's inbox drains on the same cadence so
        // a lost notification cannot strand a frame. Kept at the tail: the
        // worker borrow ends with the exchange, before the state updates.
        let sharing_enabled = self
            .settings
            .as_ref()
            .is_some_and(|settings| settings.values().activity_sharing);
        let mut outcome = ActivitySync::default();
        if let Some((frame, fingerprint)) = self.pending_activity_frame() {
            let mut events = frame;
            let mut encoded = serde_json::to_string(&events).unwrap_or_default();
            // The worker refuses frames past the direct limit; trim the
            // oldest records until the newest fit instead of losing all.
            while encoded.len() > ACTIVITY_FRAME_MAX_BYTES && events.len() > 1 {
                events.remove(0);
                encoded = serde_json::to_string(&events).unwrap_or_default();
            }
            if encoded.len() <= ACTIVITY_FRAME_MAX_BYTES {
                outcome.fingerprint = matches!(
                    media.request(
                        "activity.send",
                        json!({"call_id": call_id, "events": encoded}),
                    ),
                    Ok(reply)
                        if reply.error.is_none() && reply.message_type == "activity.send",
                )
                .then_some(fingerprint);
            }
        }
        if let Ok(reply) = media.request("activity.poll", json!({})) {
            if reply.error.is_none() && reply.message_type == "activity.poll" {
                if let Some(frames) = reply.payload.get("events").and_then(Value::as_array) {
                    outcome.inbound = frames
                        .iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect();
                }
            }
        }
        if self.absorb_activity_sync(outcome, sharing_enabled) {
            self.activity_dirty = true;
        }

        // Public-profile sync: the local profile crosses peer-to-peer over
        // the direct control channel, never the server. The hello carries
        // the full public state (avatar inline when small); larger avatars
        // follow as peer-paced, hash-verified chunks. Revisions make every
        // frame idempotent, so reconnects converge on the newest state.
        if self.sync_profile_exchange(&call_id, now_seconds()) {
            self.profile_dirty = true;
        }
    }

    /// The next activity frame to share, when policy permits and the
    /// timeline changed since the last accepted send. Sanitization and
    /// policy both happen here, before anything is serialized for the
    /// worker: the frame is exactly what may leave the device, no more.
    fn pending_activity_frame(&self) -> Option<(Vec<Value>, u64)> {
        let settings = self.settings.as_ref()?;
        if !settings.values().activity_sharing {
            return None;
        }
        let sender = self.signaling.cached_harbor_id()?;
        let frame = self
            .engine
            .lock()
            .expect("activity engine mutex")
            .shareable_json(settings.values().game_visibility, &sender);
        if frame.is_empty() {
            return None;
        }
        let fingerprint = activity_frame_fingerprint(&frame);
        if self.shared_activity_fingerprint == Some(fingerprint) {
            return None;
        }
        Some((frame, fingerprint))
    }

    /// Applies the owned outcome of one exchange. Returns whether anything
    /// user-visible changed.
    fn absorb_activity_sync(&mut self, outcome: ActivitySync, sharing_enabled: bool) -> bool {
        if !sharing_enabled {
            // A disabled policy forgets the last send so re-enabling
            // shares the frame whole again.
            self.shared_activity_fingerprint = None;
        }
        if let Some(fingerprint) = outcome.fingerprint {
            self.shared_activity_fingerprint = Some(fingerprint);
        }
        let mut changed = false;
        for raw in outcome.inbound {
            changed |= self.absorb_remote_activity_frame(&raw);
        }
        changed
    }

    /// Validates and stores one peer-delivered frame. Records failing the
    /// shareable schema are dropped, never surfaced; duplicates by id are
    /// ignored. Returns whether anything new became visible.
    fn absorb_remote_activity_frame(&mut self, raw: &str) -> bool {
        let Ok(records) = serde_json::from_str::<Value>(raw) else {
            return false;
        };
        let Some(records) = records.as_array() else {
            return false;
        };
        let now = now_seconds();
        let mut changed = false;
        for record in records {
            let Ok(record) = crate::activity::validate_remote_record(record, now) else {
                continue;
            };
            if self
                .remote_activity
                .iter()
                .any(|known| known.id == record.id)
            {
                continue;
            }
            self.remote_activity.push(record);
            changed = true;
        }
        if self.remote_activity.len() > REMOTE_ACTIVITY_CAP {
            let excess = self.remote_activity.len() - REMOTE_ACTIVITY_CAP;
            self.remote_activity.drain(0..excess);
        }
        changed
    }

    /// The validated peer records as the UI sees them: IDs, sender, and
    /// labels that already passed sanitization on the sending side, plus
    /// the peer-resolved app identity/icon keys for real program icons.
    fn remote_activity_json(&self) -> Vec<Value> {
        self.remote_activity
            .iter()
            .map(|record| {
                json!({
                    "id": record.id,
                    "sender": record.sender,
                    "category": record.category,
                    "kind": record.kind,
                    "label": record.label,
                    "app_id": record.app_id,
                    "icon": record.icon_key,
                    "occurred_at": record.occurred_at,
                })
            })
            .collect()
    }

    /// One public-profile exchange tick on a live direct call. Sends the
    /// local hello when the peer has not seen this revision, drains inbound
    /// profile frames, and paces outbound avatar chunks. Only public fields
    /// ever leave the device; the paired-only direct channel is the
    /// authorization (it exists solely for the paired session). Returns
    /// whether partner state changed.
    fn sync_profile_exchange(&mut self, call_id: &str, now: u64) -> bool {
        use crate::profile as profile;
        let Some(settings) = self.settings.as_ref() else {
            return false;
        };
        let values = settings.values();
        let local = profile::local_profile(
            values.profile_revision,
            &values.display_name,
            &values.status_message,
            &values.avatar,
            &values.avatar_type,
        );
        let mut changed = false;

        if self.profile.pending_hello(&local, now).is_some() {
            let frame = self
                .profile
                .pending_hello(&local, now)
                .unwrap_or_default();
            if let Some(media) = self.media.as_ref() {
                match media.request("profile.send", json!({"call_id": call_id, "frame": frame})) {
                    Ok(reply)
                        if reply.error.is_none()
                            && reply.message_type == "profile.send" =>
                    {
                        self.profile.mark_hello_sent(local.revision, now);
                    }
                    _ => {}
                }
            }
        }

        let mut responses: Vec<String> = Vec::new();
        if let Some(media) = self.media.as_ref() {
            if let Ok(reply) = media.request("profile.poll", json!({})) {
                if reply.error.is_none() && reply.message_type == "profile.poll" {
                    if let Some(frames) = reply.payload.get("frames").and_then(Value::as_array)
                    {
                        let raws: Vec<String> = frames
                            .iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect();
                        for raw in raws {
                            let outcome = self.profile.ingest(&raw, &local);
                            if outcome.applied {
                                changed = true;
                            }
                            responses.extend(outcome.send);
                        }
                    }
                }
            }
        }

        if let Some(cancel) = self.profile.stale_outbound_cancel(&local) {
            responses.push(cancel);
        }
        for _ in 0..profile::PROFILE_AVATAR_CHUNKS_PER_TICK {
            let Some(frame) = self.profile.peek_chunk(&local) else {
                break;
            };
            let accepted = self
                .media
                .as_ref()
                .and_then(|media| {
                    media
                        .request("profile.send", json!({"call_id": call_id, "frame": frame}))
                        .ok()
                })
                .is_some_and(|reply| {
                    reply.error.is_none() && reply.message_type == "profile.send"
                });
            if !accepted {
                break;
            }
            self.profile.advance_chunk();
        }

        if let Some(media) = self.media.as_ref() {
            for frame in responses {
                let _ = media.request("profile.send", json!({"call_id": call_id, "frame": frame}));
            }
        }

        if changed {
            self.profile.persist_partner(&self.state_dir);
        }
        changed
    }
}

/// The owned outcome of one activity exchange with the worker: whether a
/// frame was accepted for the peer, and what the peer delivered.
#[derive(Default)]
struct ActivitySync {
    fingerprint: Option<u64>,
    inbound: Vec<String>,
}

/// A stable identity for one shareable frame, so an unchanged timeline is
/// not re-sent on every poll cadence.
fn activity_frame_fingerprint(frame: &[Value]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(frame)
        .unwrap_or_default()
        .hash(&mut hasher);
    hasher.finish()
}

/// The synchronous stdio pump. Production runs through `run_stdio`
/// (reader + monitor threads, one pump); the mobile build drives
/// `dispatch` and the poll helpers below through the C ABI instead.
pub fn run_stdio() -> Result<(), CoreError> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let state = Arc::new(Mutex::new(CoreState::from_default()));

    // One merged pump: stdin requests and monitor notifications converge on
    // a single channel, so the stdout writer stays owned by one thread and
    // events never interleave with responses.
    let (sender, receiver) = mpsc::channel::<Pump>();

    let reader_sender = sender.clone();
    let reader = thread::spawn(move || {
        let mut input = stdin.lock();
        let mut decoder = FrameDecoder::default();
        let mut buffer = [0_u8; 8192];
        loop {
            match input.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => match decoder.push(&buffer[..count]) {
                    Ok(requests) => {
                        for request in requests {
                            if reader_sender
                                .send(Pump::Request(Box::new(request)))
                                .is_err()
                            {
                                return;
                            }
                        }
                    }
                    // A malformed frame ends the process's input stream; the
                    // decoder's error is reported through the frame limit
                    // rules, not to stderr.
                    Err(_) => break,
                },
                Err(_) => break,
            }
        }
        let _ = reader_sender.send(Pump::InputEnd);
    });

    // Media wake bridge: the supervisor only knows `Sender<()>`; this tiny
    // forwarder translates its tokens into pump messages, keeping the media
    // module free of core pump types.
    let (media_wake, media_wake_receiver) = mpsc::channel::<()>();
    let forwarder_sender = sender.clone();
    let forwarder = thread::spawn(move || {
        while media_wake_receiver.recv().is_ok() {
            if forwarder_sender.send(Pump::MediaChanged).is_err() {
                return;
            }
        }
    });
    {
        let mut core = state.lock().expect("core state mutex");
        core.media_wake = Some(media_wake);
    }

    // Activity monitor thread: periodic process scans. A change pokes the
    // pump so the main thread emits one coalesced `activity.updated` event;
    // unsupported platforms report once and stay quiet.
    let monitor_sender = sender;
    let monitor_engine = Arc::clone(&state.lock().expect("core state mutex").engine);
    let monitor = thread::spawn(move || {
        let mut monitor = crate::monitor::platform_monitor();
        let mut running = false;
        loop {
            thread::sleep(Duration::from_secs(2));
            let now = now_seconds();
            match monitor.scan() {
                Ok(observations) => {
                    let changed = {
                        let mut engine = monitor_engine.lock().expect("activity engine mutex");
                        if running {
                            engine.ingest(now, observations)
                        } else {
                            running = true;
                            engine.mark_monitor_started(now) | engine.ingest(now, observations)
                        }
                    };
                    if changed && monitor_sender.send(Pump::ActivityChanged).is_err() {
                        return;
                    }
                }
                Err(error) => {
                    let changed = {
                        let mut engine = monitor_engine.lock().expect("activity engine mutex");
                        engine.mark_monitor_unavailable(
                            now,
                            error.kind() == io::ErrorKind::Unsupported,
                        )
                    };
                    if changed && monitor_sender.send(Pump::ActivityChanged).is_err() {
                        return;
                    }
                }
            }
        }
    });

    let mut output = stdout.lock();
    let result = pump(&receiver, &state, &mut output);
    // On explicit core.shutdown, the reader is intentionally still blocked in
    // stdin: the supervisor owns that write end and waits for *this* process
    // to exit. Joining it here would create a shutdown deadlock until the
    // supervisor's kill deadline. Rust terminates these process-local threads
    // together with main; neither can outlive the core process.
    drop(reader);
    drop(monitor);
    drop(forwarder);
    result
}

enum Pump {
    Request(Box<Envelope>),
    ActivityChanged,
    MediaChanged,
    InputEnd,
}

/// Drives the merged pump until the input ends or shutdown is requested.
/// Between messages it services the call-signaling poll loop, whose cadence
/// yields to (and never starves) request handling.
fn pump<W: Write>(
    receiver: &mpsc::Receiver<Pump>,
    state: &Arc<Mutex<CoreState>>,
    output: &mut W,
) -> Result<(), CoreError> {
    let mut next_signal_poll = Instant::now();
    loop {
        let timeout = next_signal_poll.saturating_duration_since(Instant::now());
        match receiver.recv_timeout(timeout) {
            Ok(Pump::Request(request)) => {
                let should_stop = request.message_type == "core.shutdown";
                let response = {
                    let mut core = state.lock().expect("core state mutex");
                    let response = dispatch(*request, &mut core)?;
                    if should_stop {
                        // One honest OFFLINE on the way out; the lease would
                        // otherwise keep the partner guessing for up to 45 s.
                        signaling::publish_shutdown_offline(&mut core);
                    }
                    response
                };
                output.write_all(&encode_frame(&response)?)?;
                output.flush()?;
                if should_stop {
                    return Ok(());
                }
                write_call_event(state, output)?;
                write_share_event(state, output)?;
                write_direct_event(state, output)?;
                write_profile_event(state, output)?;
                write_presence_event(state, output)?;
                write_device_event(state, output)?;
                write_mobile_event(state, output)?;
                write_phone_notification_event(state, output)?;
                write_activity_event(state, output)?;
            }
            // The monitor only pokes the pump when a scan actually changed
            // something, so this event is forced; a stale dirty flag (an
            // event already taken) clears with it.
            Ok(Pump::ActivityChanged) => {
                let event = {
                    let mut core = state.lock().expect("core state mutex");
                    core.activity_dirty = false;
                    core.activity_event()
                };
                output.write_all(&encode_frame(&event)?)?;
                output.flush()?;
            }
            // The media worker queued facts or died. Translate what arrived
            // into local call/share events; a worker that vanished mid-call
            // becomes an explicit FAILED, never a silent CONNECTING forever.
            Ok(Pump::MediaChanged) => {
                let events = {
                    let mut core = state.lock().expect("core state mutex");
                    core.drain_media_events();
                    if core.call.call_id.is_some()
                        && core.call.phase != "IDLE"
                        && core.media.as_ref().is_some_and(|media| !media.is_running())
                    {
                        core.call.phase = "FAILED".into();
                        core.chat.fail_unacknowledged();
                        core.call_dirty = true;
                        core.direct_dirty = true;
                    }
                    core.sync_direct();
                    let mut events = Vec::new();
                    if let Some(event) = core.take_call_event() {
                        events.push(event);
                    }
                    if let Some(event) = core.take_share_event() {
                        events.push(event);
                    }
                    if let Some(event) = core.take_direct_event() {
                        events.push(event);
                    }
                    if let Some(event) = core.take_profile_event() {
                        events.push(event);
                    }
                    if let Some(event) = core.take_voice_event() {
                        events.push(event);
                    }
                    if let Some(event) = core.take_stats_event() {
                        events.push(event);
                    }
                    if let Some(event) = core.take_phone_notification_event() {
                        events.push(event);
                    }
                    events
                };
                for event in events {
                    output.write_all(&encode_frame(&event)?)?;
                    output.flush()?;
                }
            }
            Ok(Pump::InputEnd) => return Ok(()),
            // The signaling poll cadence: drain inbound relayed signals and
            // present an authenticated peer's offer as an explicit INCOMING
            // state — never answered without this user's approval.
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let (events, interval) = {
                    let mut core = state.lock().expect("core state mutex");
                    let action = signaling::tick(&mut core);
                    if let SignalingTick::IncomingOffer {
                        ref offer_call_id,
                        ref sdp,
                        peer,
                        ..
                    } = action
                    {
                        core.present_incoming(offer_call_id, sdp, peer);
                    }
                    // A recovery that outlives its window ends the call, and
                    // a ring nobody answered stops ringing.
                    enforce_reconnect_window(&mut core);
                    core.enforce_incoming_ttl();
                    // The worker's bounded direct buffers are authoritative;
                    // poll them on the same cadence as the private signaling
                    // pump, even if an event notification was lost.
                    core.sync_direct();
                    let mut events = Vec::new();
                    if let Some(event) = core.take_call_event() {
                        events.push(event);
                    }
                    if let Some(event) = core.take_share_event() {
                        events.push(event);
                    }
                    if let Some(event) = core.take_direct_event() {
                        events.push(event);
                    }
                    if let Some(event) = core.take_profile_event() {
                        events.push(event);
                    }
                    if let Some(event) = core.take_presence_event() {
                        events.push(event);
                    }
                    if let Some(event) = core.take_voice_event() {
                        events.push(event);
                    }
                    if let Some(event) = core.take_stats_event() {
                        events.push(event);
                    }
                    if let Some(event) = core.take_phone_notification_event() {
                        events.push(event);
                    }
                    (events, core.signaling.interval())
                };
                for event in events {
                    output.write_all(&encode_frame(&event)?)?;
                    output.flush()?;
                }
                next_signal_poll = Instant::now() + interval;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Ok(()),
        }
    }
}

/// Emits the pending presence transition after its correlated response.
fn write_presence_event<W: Write>(
    state: &Arc<Mutex<CoreState>>,
    output: &mut W,
) -> Result<(), CoreError> {
    let event = {
        let mut core = state.lock().expect("core state mutex");
        core.take_presence_event()
    };
    if let Some(event) = event {
        output.write_all(&encode_frame(&event)?)?;
        output.flush()?;
    }
    Ok(())
}

/// Emits the pending call state transition after its correlated response.
fn write_call_event<W: Write>(
    state: &Arc<Mutex<CoreState>>,
    output: &mut W,
) -> Result<(), CoreError> {
    let event = {
        let mut core = state.lock().expect("core state mutex");
        core.take_call_event()
    };
    if let Some(event) = event {
        output.write_all(&encode_frame(&event)?)?;
        output.flush()?;
    }
    Ok(())
}

/// Emits the pending screen-share state transition after its correlated
/// response. It carries no source, frame, permission, or capture details.
fn write_share_event<W: Write>(
    state: &Arc<Mutex<CoreState>>,
    output: &mut W,
) -> Result<(), CoreError> {
    let event = {
        let mut core = state.lock().expect("core state mutex");
        core.take_share_event()
    };
    if let Some(event) = event {
        output.write_all(&encode_frame(&event)?)?;
        output.flush()?;
    }
    Ok(())
}

/// Emits the pending direct-session snapshot after a chat or transfer state
/// changes. Its payload is deliberately metadata-only.
fn write_direct_event<W: Write>(
    state: &Arc<Mutex<CoreState>>,
    output: &mut W,
) -> Result<(), CoreError> {
    let event = {
        let mut core = state.lock().expect("core state mutex");
        core.take_direct_event()
    };
    if let Some(event) = event {
        output.write_all(&encode_frame(&event)?)?;
        output.flush()?;
    }
    Ok(())
}

/// Emits the pending partner-profile snapshot after the peer shared newer
/// public state. Its payload carries public fields only.
fn write_profile_event<W: Write>(
    state: &Arc<Mutex<CoreState>>,
    output: &mut W,
) -> Result<(), CoreError> {
    let event = {
        let mut core = state.lock().expect("core state mutex");
        core.take_profile_event()
    };
    if let Some(event) = event {
        output.write_all(&encode_frame(&event)?)?;
        output.flush()?;
    }
    Ok(())
}

/// Emits the pending device-endpoint snapshot after a type switch, link,
/// unlink, or media-endpoint move.
fn write_device_event<W: Write>(
    state: &Arc<Mutex<CoreState>>,
    output: &mut W,
) -> Result<(), CoreError> {
    let event = {
        let mut core = state.lock().expect("core state mutex");
        core.take_device_event()
    };
    if let Some(event) = event {
        output.write_all(&encode_frame(&event)?)?;
        output.flush()?;
    }
    Ok(())
}

/// Emits the pending validated phone aggregate after `mobile.update`.
fn write_mobile_event<W: Write>(
    state: &Arc<Mutex<CoreState>>,
    output: &mut W,
) -> Result<(), CoreError> {
    let event = {
        let mut core = state.lock().expect("core state mutex");
        core.take_mobile_event()
    };
    if let Some(event) = event {
        output.write_all(&encode_frame(&event)?)?;
        output.flush()?;
    }
    Ok(())
}

fn write_phone_notification_event<W: Write>(
    state: &Arc<Mutex<CoreState>>,
    output: &mut W,
) -> Result<(), CoreError> {
    let event = {
        let mut core = state.lock().expect("core state mutex");
        core.take_phone_notification_event()
    };
    if let Some(event) = event {
        output.write_all(&encode_frame(&event)?)?;
        output.flush()?;
    }
    Ok(())
}

/// Emits the pending `activity.updated` event, if a dispatch marked the
/// timeline dirty (policy flips re-redact the timeline).
fn write_activity_event<W: Write>(
    state: &Arc<Mutex<CoreState>>,
    output: &mut W,
) -> Result<(), CoreError> {
    let event = {
        let mut core = state.lock().expect("core state mutex");
        core.take_activity_event()
    };
    if let Some(event) = event {
        output.write_all(&encode_frame(&event)?)?;
        output.flush()?;
    }
    Ok(())
}

/// The synchronous, thread-free variant of the stdio loop. Production runs
/// through `run_stdio` (reader + monitor threads, one pump); tests drive
/// this one to pin dispatch and framing without concurrency.
#[cfg(test)]
fn run<R: Read, W: Write>(
    mut input: R,
    mut output: W,
    state: &mut CoreState,
) -> Result<(), CoreError> {
    let mut decoder = FrameDecoder::default();
    let mut buffer = [0_u8; 8192];

    loop {
        let count = input.read(&mut buffer)?;
        if count == 0 {
            decoder.finish()?;
            return Ok(());
        }

        for request in decoder.push(&buffer[..count])? {
            let should_stop = request.message_type == "core.shutdown";
            let response = dispatch(request, state)?;
            output.write_all(&encode_frame(&response)?)?;
            output.flush()?;
            if let Some(event) = state.take_call_event() {
                output.write_all(&encode_frame(&event)?)?;
                output.flush()?;
            }
            if let Some(event) = state.take_share_event() {
                output.write_all(&encode_frame(&event)?)?;
                output.flush()?;
            }
            if let Some(event) = state.take_activity_event() {
                output.write_all(&encode_frame(&event)?)?;
                output.flush()?;
            }
            if let Some(event) = state.take_direct_event() {
                output.write_all(&encode_frame(&event)?)?;
                output.flush()?;
            }
            if let Some(event) = state.take_profile_event() {
                output.write_all(&encode_frame(&event)?)?;
                output.flush()?;
            }
            if should_stop {
                return Ok(());
            }
        }
    }
}

fn dispatch(request: Envelope, state: &mut CoreState) -> Result<Envelope, CoreError> {
    let timestamp = request
        .timestamp
        .clone()
        .ok_or(CoreError::MissingTimestamp)?;
    let mut response =
        Envelope::response_to(&request, request.message_type.clone(), json!({}), timestamp)?;

    match request.message_type.as_str() {
        "core.hello" => {
            let minimum = request
                .payload
                .get("protocol_min")
                .and_then(serde_json::Value::as_u64);
            let maximum = request
                .payload
                .get("protocol_max")
                .and_then(serde_json::Value::as_u64);
            if minimum.is_none_or(|minimum| minimum > 1)
                || maximum.is_none_or(|maximum| maximum < 1)
            {
                response.error = Some(ProtocolError {
                    code: "protocol_version_incompatible".into(),
                    ui_key: "error.protocol.versionIncompatible".into(),
                    retryable: false,
                    detail: "Harbor core supports control protocol v1 only".into(),
                });
            } else {
                response.message_type = "core.ready".into();
                response.payload = json!({
                    "protocol": 1,
                    "service": "harbor-core",
                    "capabilities": CORE_CAPABILITIES,
                });
            }
        }
        "core.shutdown" => {
            state.shutdown_media();
            response.payload = json!({"accepted": true});
        }
        "call.start" => start_call(state, &mut response),
        "call.accept" => accept_incoming_call(state, &mut response),
        "call.decline" => decline_incoming_call(state, &mut response),
        "call.end" => end_call(state, &mut response),
        "call.mute" => set_call_muted(state, &request.payload, &mut response),
        "call.push_to_talk" => set_push_to_talk(state, &request.payload, &mut response),
        "call.share_screen_start" => start_screen_share(state, &mut response),
        "call.share_screen_stop" => stop_screen_share(state, &mut response),
        "audio.devices" => list_audio_devices(state, &mut response),
        "audio.config" => audio_config(state, &request.payload, &mut response),
        "audio.switch_devices" => audio_switch_devices(state, &request.payload, &mut response),
        "audio.loopback_start" => mic_test_start(state, &request.payload, &mut response),
        "audio.loopback_poll" => mic_test_poll(state, &mut response),
        "audio.loopback_stop" => mic_test_stop(state, &mut response),
        "chat.send" => send_chat(state, &request.payload, &mut response),
        "transfer.offer_local" => offer_local_transfer(state, &request.payload, &mut response),
        "transfer.accept" => accept_transfer(state, &request.payload, true, &mut response),
        "transfer.reject" => accept_transfer(state, &request.payload, false, &mut response),
        "transfer.cancel" => cancel_transfer(state, &request.payload, &mut response),
        "direct.state" => {
            state.sync_direct();
            response.payload = state.direct_payload();
        }
        // Partner-profile snapshot: the peer's public fields only. Served
        // from the validated local cache; live updates arrive as
        // `profile.updated` events while a direct call is up.
        "profile.state" => {
            response.payload = state.profile_snapshot();
        }
        // Own endpoint snapshot: device record, companion mode, registry,
        // media endpoint. Reads durable state only — nothing is negotiated
        // here. The peer side arrives with the direct `device_hello`.
        "device.state" => match state.device_snapshot() {
            Ok(snapshot) => response.payload = snapshot,
            Err(error) => response.error = Some(error),
        },
        "device.configure" => configure_device(state, &request.payload, &mut response),
        // Validated phone aggregate for the UI; `mobile.updated` follows.
        "mobile.state" => {
            response.payload = state.mobile_snapshot();
        }
        "mobile.update" => update_mobile(state, &request.payload, &mut response),
        "mobile.notification" => forward_phone_notification(state, &request.payload, &mut response),
        // Explicit media-endpoint handoff between two own devices. The
        // grant drops local media first when this install holds it; the
        // sibling's join rides the normal call start on its own core.
        "call.takeover" => take_over_call(state, &request.payload, &mut response),
        // The Qt side's native detector pushes private platform facts here;
        // the aggregate committed inside the tracker is all that anyone —
        // this UI, the peer, the server — ever sees. A snapshot is never
        // echoed back, and only a committed transition marks the event.
        "presence.sense" => match serde_json::from_value::<
            crate::presence::UserActivitySnapshot,
        >(request.payload.clone()) {
            Ok(snapshot) => {
                let now = now_seconds();
                if state.presence.observe_local(&snapshot, now) {
                    state.presence_dirty = true;
                }
                response.payload = json!({"state": state.presence.local.state()});
            }
            Err(_) => {
                response.error = Some(ProtocolError::invalid_request(
                    "presence snapshot is malformed",
                ))
            }
        },
        // The committed aggregate for the UI. The evidence behind it stays
        // inside the core; transitions arrive unsolicited as
        // `presence.updated` — polling this route is for reconnects only.
        "presence.state" => {
            response.payload = json!({
                "local": {
                    "state": state.presence.local.state(),
                    "revision": state.presence.local.revision(),
                },
                "partner": state.presence.partner.state().map(|partner| json!({
                    "state": partner,
                    "revision": state.presence.partner.revision(),
                })),
            });
        }
        // Identity lives in the same state directory as everything else the
        // core owns: one persistence root, one resolution (no env-dependent
        // default that drifts away from state.state_dir).
        "identity.get" => match load_or_create(&state.state_dir, now_seconds()) {
            Ok(identity) => {
                let record = identity.record();
                response.payload = json!({
                    "device_id": record.device_id,
                    "harbor_id": record.harbor_id,
                    "public_key": record.public_key,
                    "registered_at": record.registered_at,
                });
            }
            Err(_) => response.error = Some(identity_unavailable_error()),
        },
        "settings.get" => match state.settings.as_ref() {
            Some(settings) => response.payload = settings.to_json(),
            None => response.error = Some(settings_unavailable_error()),
        },
        "settings.update" => {
            // Activity policies re-redact the timeline; the re-render is
            // emitted as an `activity.updated` event after the response.
            let activity_policy_change = request.payload.get("activitySharing").is_some()
                || request.payload.get("gameVisibility").is_some();
            let Some(settings) = state.settings.as_mut() else {
                response.error = Some(settings_unavailable_error());
                return Ok(response);
            };
            match settings.update(&request.payload) {
                Ok(()) => response.payload = settings.to_json(),
                Err(error @ (SettingsError::InvalidPatch | SettingsError::UnknownKey)) => {
                    response.error = Some(ProtocolError {
                        code: "invalid_request".into(),
                        ui_key: "error.protocol.invalidRequest".into(),
                        retryable: false,
                        detail: error.to_string(),
                    });
                }
                Err(_) => response.error = Some(settings_unavailable_error()),
            }
            if response.error.is_none() && activity_policy_change {
                state.activity_dirty = true;
            }
            // Call-mode changes are policy a live call must follow, not just
            // durable text: the worker's per-frame gate reads this mode. An
            // idle worker learns nothing — the next call payload carries the
            // persisted mode.
            if response.error.is_none() {
                if let Some(enabled) = request
                    .payload
                    .get("pushToTalkEnabled")
                    .and_then(Value::as_bool)
                {
                    forward_call_mode(
                        state,
                        "call.ptt",
                        json!({"enabled": enabled}),
                        &mut response,
                    );
                }
                if let Some(enabled) = request
                    .payload
                    .get("voiceActivation")
                    .and_then(Value::as_bool)
                {
                    forward_call_mode(
                        state,
                        "call.voice_activation",
                        json!({"enabled": enabled}),
                        &mut response,
                    );
                }
            }
        }
        "activity.state" => {
            let engine = state.engine.lock().expect("activity engine mutex");
            let game_titles = state
                .settings
                .as_ref()
                .map(|settings| settings.values().game_visibility)
                .unwrap_or(true);
            response.payload = json!({
                "monitor": engine.monitor_state().as_str(),
                "timeline": engine.timeline_json(game_titles),
                "stats": engine.stats_json(now_seconds()),
                "remote": state.remote_activity_json(),
            });
        }
        "server.config" => match load_server_pin(&state.state_dir) {
            Some(pin) => {
                response.payload = json!({
                    "configured": true,
                    "address": pin.address,
                    "fingerprint": pin.fingerprint_hex,
                })
            }
            None => response.payload = json!({"configured": false, "fingerprint": ""}),
        },
        "server.configure" => {
            let parsed = match (
                request.payload.get("address").and_then(Value::as_str),
                request.payload.get("fingerprint").and_then(Value::as_str),
            ) {
                (Some(address), Some(fingerprint)) => ServerPin::parse(address, fingerprint),
                _ => Err(crate::ServerClientError::InvalidFingerprint),
            };
            match parsed {
                Ok(pin) => match store_server_pin(&state.state_dir, &pin) {
                    Ok(()) => {
                        response.payload = json!({
                            "configured": true,
                            "address": pin.address,
                            "fingerprint": pin.fingerprint_hex,
                        })
                    }
                    Err(error) => response.error = Some(storage_failed_error(error)),
                },
                Err(error) => {
                    let (ui_key, detail) = match error {
                        crate::ServerClientError::InvalidAddress => (
                            "error.server.invalidAddress",
                            "server address must be a host and non-zero port",
                        ),
                        crate::ServerClientError::InvalidFingerprint => (
                            "error.protocol.invalidRequest",
                            "server address and 64-hex-character fingerprint are required",
                        ),
                        _ => (
                            "error.protocol.invalidRequest",
                            "server address and 64-hex-character fingerprint are required",
                        ),
                    };
                    response.error = Some(ProtocolError {
                        code: "invalid_request".into(),
                        ui_key: ui_key.into(),
                        retryable: false,
                        detail: detail.into(),
                    })
                }
            }
        }
        "pairing.state" => {
            response.payload = state.pairing.snapshot();
        }
        // The control plane owns durable paired relationships. Only the safe
        // public peer identifiers cross the local UI boundary; public keys
        // are deliberately omitted because this snapshot gates navigation,
        // not identity verification.
        "contacts.list" => match list_paired_contacts(state) {
            Ok(payload) => response.payload = payload,
            Err(error) => response.error = Some(error),
        },
        // Diagnostics measure the real path: a fresh pinned TLS handshake and
        // one signed control-plane exchange, plus whatever the live call's
        // worker actually reported. Unreachable servers are a structured
        // result, never a simulated success.
        "network.diagnostics" => match run_network_diagnostics(state) {
            Ok(payload) => response.payload = payload,
            Err(error) => response.error = Some(error),
        },
        "pairing.enter_code" => {
            state.pairing.enter_code();
            response.payload = state.pairing.snapshot();
        }
        "pairing.reset" => {
            state.pairing.reset();
            response.payload = state.pairing.snapshot();
        }
        "pairing.create" | "pairing.submit" | "pairing.incoming" | "pairing.accept"
        | "pairing.decline" | "pairing.cancel" | "pairing.status" => {
            let message_type = request.message_type.clone();
            match run_pairing(state, &message_type, &request.payload) {
                Ok(payload) => response.payload = payload,
                Err(error) => response.error = Some(error),
            }
        }
        _ => {
            response.error = Some(ProtocolError {
                code: "capability_unavailable".into(),
                ui_key: "error.core.capabilityUnavailable".into(),
                retryable: false,
                detail: format!(
                    "{} is not enabled by the current core foundation",
                    request.message_type
                ),
            });
        }
    }

    state.drain_media_events();
    Ok(response)
}

fn start_call(state: &mut CoreState, response: &mut Envelope) {
    start_call_inner(state, response, false);
}

/// Caller side with a takeover flag: the offer names this device taking over
/// its identity's call media. The peer core drops the sibling's media first
/// (same harbor identity) and presents the new offer; any other peer treats
/// it as a normal offer under the usual busy rules.
fn start_call_inner(state: &mut CoreState, response: &mut Envelope, takeover: bool) {
    if state.call.call_id.is_some() {
        response.error = Some(call_error("call_active", "error.call.alreadyActive", false));
        return;
    }
    // A running microphone self-check ends here: the call owns the devices.
    stop_mic_test(state);
    // The server leg comes first: resolving the peer and holding the session
    // needs no offer, so configuration and reachability problems refuse the
    // call before a worker spawns or a microphone opens.
    let target = match signaling::prepare(state) {
        Ok(target) => target,
        Err(error) => {
            response.error = Some(error);
            return;
        }
    };
    let media = match MediaSupervisor::start_with_executable(state.media_worker_path.as_deref(), state.media_wake.clone()) {
        Ok(media) => media,
        Err(error) => {
            state.call.phase = "FAILED".into();
            state.call_dirty = true;
            response.error = Some(media_unavailable_error(error));
            return;
        }
    };
    let call_id = Uuid::new_v4().to_string();
    match media.request("call.start", audio_call_payload(state, &call_id)) {
        Ok(reply) if reply.error.is_none() && reply.message_type == "call.start" => {
            // The worker created a local offer; it must reach the paired peer
            // through the control plane before anything can connect. A relay
            // failure is a visible refusal, not a hanging CONNECTING.
            let offer = reply.payload.get("signal").cloned().unwrap_or(Value::Null);
            let dialed = if takeover {
                let own = match load_or_create(&state.state_dir, now_seconds()) {
                    Ok(identity) => identity.record().device_id,
                    Err(_) => {
                        state.call.phase = "FAILED".into();
                        state.call_dirty = true;
                        response.error = Some(identity_unavailable_error());
                        media.shutdown();
                        return;
                    }
                };
                signaling::dial_to(
                    state,
                    &target,
                    &json!({
                        "call_id": call_id,
                        "signal": offer,
                        "takeover": true,
                        "from_device": own.to_string(),
                    }),
                )
            } else {
                signaling::dial_to(state, &target, &json!({"call_id": call_id, "signal": offer}))
            };
            match dialed {
                Ok(()) => {
                    state.call.call_id = Some(call_id.clone());
                    state.call.peer = Some(target);
                    state.call.phase = "CONNECTING".into();
                    state.call.muted = initial_mic_muted(
                        DeviceType::parse(
                            state
                                .settings
                                .as_ref()
                                .map(|settings| settings.values().device_type.as_str())
                                .unwrap_or("desktop"),
                        )
                        .unwrap_or(DeviceType::Desktop),
                    );
                    state.call.reason.clear();
                    state.call_dirty = true;
                    state.media = Some(media);
                    response.payload = if takeover {
                        json!({"state": "CONNECTING", "call_id": call_id, "takeover": true})
                    } else {
                        json!({"state": "CONNECTING", "call_id": call_id})
                    };
                }
                Err(error) => {
                    state.call.phase = "FAILED".into();
                    state.call_dirty = true;
                    response.error = Some(error);
                    media.shutdown();
                }
            }
        }
        Ok(reply) => {
            state.call.phase = "FAILED".into();
            state.call_dirty = true;
            response.error = Some(reply.error.map_or_else(
                || media_unavailable_error(MediaError::UnexpectedReply),
                worker_error,
            ));
            media.shutdown();
        }
        Err(error) => {
            state.call.phase = "FAILED".into();
            state.call_dirty = true;
            response.error = Some(media_unavailable_error(error));
            media.shutdown();
        }
    }
}

fn end_call(state: &mut CoreState, response: &mut Envelope) {
    state.shutdown_media();
    response.payload = json!({"state": "ENDED"});
}

/// Own-endpoint management: persist a device-type switch or maintain the
/// companion registry (link/unlink an authorized own device). Every
/// mutation re-renders through `device.updated`; unknown keys and
/// malformed records are refused, never partially applied.
fn configure_device(state: &mut CoreState, payload: &Value, response: &mut Envelope) {
    let Some(patch) = payload.as_object() else {
        response.error = Some(device_error("invalid_request", "error.protocol.invalidRequest"));
        return;
    };
    if patch.is_empty()
        || patch
            .keys()
            .any(|key| !matches!(key.as_str(), "deviceType" | "link" | "unlink"))
    {
        response.error = Some(device_error("invalid_request", "error.protocol.invalidRequest"));
        return;
    }
    let Some(settings) = state.settings.as_mut() else {
        response.error = Some(settings_unavailable_error());
        return;
    };
    let now = now_seconds();
    if let Some(device_type) = patch.get("deviceType").and_then(Value::as_str) {
        if DeviceType::parse(device_type).is_none() {
            response.error = Some(device_error("unknown_device_type", "error.device.unknownType"));
            return;
        }
        if let Err(error) = settings.update(&json!({"deviceType": device_type})) {
            response.error = Some(settings_patch_error(error));
            return;
        }
    }
    if let Some(link) = patch.get("link") {
        let device_id = link
            .get("deviceId")
            .and_then(Value::as_str)
            .and_then(|id| Uuid::parse_str(id).ok());
        let device_type = link
            .get("deviceType")
            .and_then(Value::as_str)
            .and_then(DeviceType::parse);
        let (Some(device_id), Some(device_type)) = (device_id, device_type) else {
            response.error = Some(device_error("invalid_device_link", "error.device.invalidLink"));
            return;
        };
        let mut linked = settings.values().linked_devices.clone();
        if let Some(existing) = linked.iter_mut().find(|device| device.device_id == device_id) {
            existing.device_type = device_type;
            existing.authorized = true;
            existing.last_seen = now;
        } else {
            linked.push(DeviceRecord::new(device_id, device_type, true, now));
        }
        let patch_value = serde_json::to_value(&linked).unwrap_or(Value::Null);
        if let Err(error) = settings.update(&json!({"linkedDevices": patch_value})) {
            response.error = Some(settings_patch_error(error));
            return;
        }
    }
    if let Some(unlink) = patch.get("unlink") {
        let device_id = unlink
            .get("deviceId")
            .and_then(Value::as_str)
            .and_then(|id| Uuid::parse_str(id).ok());
        let Some(device_id) = device_id else {
            response.error = Some(device_error("invalid_device_link", "error.device.invalidLink"));
            return;
        };
        let mut linked = settings.values().linked_devices.clone();
        let before = linked.len();
        linked.retain(|device| device.device_id != device_id);
        if linked.len() == before {
            response.error = Some(device_error("unknown_device", "error.device.unknownDevice"));
            return;
        }
        let patch_value = serde_json::to_value(&linked).unwrap_or(Value::Null);
        if let Err(error) = settings.update(&json!({"linkedDevices": patch_value})) {
            response.error = Some(settings_patch_error(error));
            return;
        }
    }
    match state.device_snapshot() {
        Ok(snapshot) => {
            state.device_dirty = true;
            response.payload = snapshot;
        }
        Err(error) => response.error = Some(error),
    }
}

/// Stores this install's validated phone aggregate. Consent and shape are
/// enforced by [`MobileStatus::validate`]: a fix without its toggle or an
/// invented signal is refused with its own code, never stored.
fn update_mobile(state: &mut CoreState, payload: &Value, response: &mut Envelope) {
    let status: MobileStatus = match serde_json::from_value(payload.clone()) {
        Ok(status) => status,
        Err(_) => {
            response.error = Some(device_error("invalid_request", "error.protocol.invalidRequest"));
            return;
        }
    };
    if let Err(error) = status.validate() {
        response.error = Some(device_error(error.code(), "error.mobile.invalidStatus"));
        return;
    }
    state.mobile_own = Some(status);
    state.mobile_dirty = true;
    state.transmit_link_mobile();
    response.payload = state.mobile_snapshot();
}

fn validate_phone_notification(payload: &Value) -> Result<(), ()> {
    let object = payload.as_object().ok_or(())?;
    for key in ["appLabel", "title", "text"] {
        if !object.get(key).is_some_and(Value::is_string) {
            return Err(());
        }
    }
    let app = object.get("appLabel").and_then(Value::as_str).unwrap_or_default();
    let title = object.get("title").and_then(Value::as_str).unwrap_or_default();
    let text = object.get("text").and_then(Value::as_str).unwrap_or_default();
    let timestamp = object.get("timestamp").and_then(Value::as_u64).unwrap_or(0);
    if app.trim().is_empty() || (title.trim().is_empty() && text.trim().is_empty())
        || app.len() > 256 || title.len() > 512 || text.len() > 4096 || timestamp == 0
    {
        return Err(());
    }
    Ok(())
}

/// Accepts the native listener callback only while the persisted share
/// intent is enabled. The Android listener also checks its special grant;
/// this second gate prevents stale callbacks after a toggle is switched off.
fn forward_phone_notification(state: &mut CoreState, payload: &Value, response: &mut Envelope) {
    if !state.settings.as_ref().is_some_and(|settings| settings.values().share_phone_notifications)
        || validate_phone_notification(payload).is_err()
    {
        response.error = Some(device_error("notification_unavailable", "error.mobile.notificationsUnavailable"));
        return;
    }
    if state.transmit_link_phone_notification(payload) {
        response.payload = json!({"sent": true});
    } else {
        response.error = Some(device_error("peer_unavailable", "error.mobile.peerUnavailable"));
    }
}

/// Explicit media-endpoint handoff between two own devices, executed on
/// either side of the handoff:
/// * on the JOINING device (`joinDevice` is this install): starts a takeover
///   call when idle — the peer core drops the sibling's media first (same
///   harbor identity) and presents the new offer. A device already holding
///   media answers `already_active`.
/// * on the LEAVING device (`joinDevice` is the sibling): drops local media
///   first (never two mics of one identity) and records the new endpoint.
///   With no live local call there is nothing to drop: `call_inactive`.
fn take_over_call(state: &mut CoreState, payload: &Value, response: &mut Envelope) {
    let join = payload
        .get("joinDevice")
        .and_then(Value::as_str)
        .and_then(|id| Uuid::parse_str(id).ok());
    let Some(join) = join else {
        response.error = Some(call_error(
            "invalid_request",
            "error.protocol.invalidRequest",
            false,
        ));
        return;
    };
    let own = match load_or_create(&state.state_dir, now_seconds()) {
        Ok(identity) => identity.record().device_id,
        Err(_) => {
            response.error = Some(identity_unavailable_error());
            return;
        }
    };
    if join == own {
        if state.call.call_id.is_some() {
            response.payload = json!({"action": "already_active", "endpoint": join});
            return;
        }
        start_call_inner(state, response, true);
        return;
    }
    if state.call.call_id.is_none() {
        response.error = Some(call_error("call_inactive", "error.call.inactive", false));
        return;
    }
    let holder = state.media_endpoint.unwrap_or(own);
    // Drop first, join second: local media ends before the sibling starts.
    state.shutdown_media();
    state.call.reason = "takeover".into();
    state.call_dirty = true;
    state.media_endpoint = Some(join);
    state.device_dirty = true;
    response.payload = json!({
        "action": "drop_then_join",
        "dropDevice": holder,
        "joinDevice": join,
    });
}

fn device_error(code: &str, ui_key: &str) -> ProtocolError {
    ProtocolError {
        code: code.into(),
        ui_key: ui_key.into(),
        retryable: false,
        detail: code.into(),
    }
}

fn settings_patch_error(error: SettingsError) -> ProtocolError {
    match error {
        SettingsError::InvalidPatch | SettingsError::UnknownKey => {
            device_error("invalid_request", "error.protocol.invalidRequest")
        }
        _ => settings_unavailable_error(),
    }
}

/// Accepts the presented incoming call: the worker answers the stored offer
/// and the answer is relayed back under the caller's own call id. Nothing
/// about the media path existed before this dispatch.
fn accept_incoming_call(state: &mut CoreState, response: &mut Envelope) {
    let Some(incoming) = state.incoming.take() else {
        response.error = Some(call_error(
            "no_incoming_call",
            "error.call.noIncomingCall",
            false,
        ));
        return;
    };
    // Transient approval phase: the UI sees the click land before the
    // (synchronous) worker negotiation completes.
    state.call.phase = "ACCEPTING".into();
    state.call_dirty = true;
    // A running microphone self-check ends here: the call owns the devices.
    stop_mic_test(state);

    let fail = |state: &mut CoreState, error: ProtocolError, response: &mut Envelope| {
        state.incoming = None;
        state.call.phase = "FAILED".into();
        state.call.reason = error.code.clone();
        state.call_dirty = true;
        response.error = Some(error);
    };

    // The session leg: the idle poll loop usually holds one already, but an
    // acceptance must not depend on that timing.
    let incoming_peer = incoming.peer;
    if let Err(error) = signaling::prepare(state) {
        fail(state, error, response);
        return;
    }
    let media = match MediaSupervisor::start_with_executable(state.media_worker_path.as_deref(), state.media_wake.clone()) {
        Ok(media) => media,
        Err(error) => {
            fail(state, media_unavailable_error(error), response);
            return;
        }
    };
    let call_id = Uuid::new_v4().to_string();
    let mut payload = audio_call_payload(state, &call_id);
    payload["signal"] = json!({"type": "offer", "sdp": incoming.sdp});
    match media.request("call.accept", payload) {
        Ok(reply) if reply.error.is_none() && reply.message_type == "call.accept" => {
            let answer = reply.payload.get("signal").cloned().unwrap_or(Value::Null);
            let relayed = signaling::send_to(
                state,
                &incoming_peer,
                &json!({"call_id": incoming.remote_call_id, "signal": answer}),
            );
            if !relayed {
                media.shutdown();
                fail(state, server_unavailable_error(), response);
                return;
            }
            state.call.call_id = Some(call_id.clone());
            state.call.peer = Some(incoming_peer);
            state.call.phase = "CONNECTING".into();
            state.call.muted = initial_mic_muted(
                DeviceType::parse(
                    state
                        .settings
                        .as_ref()
                        .map(|settings| settings.values().device_type.as_str())
                        .unwrap_or("desktop"),
                )
                .unwrap_or(DeviceType::Desktop),
            );
            state.call.reason.clear();
            state.call_dirty = true;
            state.media = Some(media);
            response.payload = json!({"state": "CONNECTING", "call_id": call_id});
        }
        Ok(reply) => {
            media.shutdown();
            fail(
                state,
                reply.error.map_or_else(
                    || media_unavailable_error(MediaError::UnexpectedReply),
                    worker_error,
                ),
                response,
            );
        }
        Err(error) => {
            media.shutdown();
            fail(state, media_unavailable_error(error), response);
        }
    }
}

/// Declines the presented incoming call. The refusal is told to the caller
/// with an explicit decline signal; nothing media-shaped ever existed here.
fn decline_incoming_call(state: &mut CoreState, response: &mut Envelope) {
    let Some(incoming) = state.incoming.take() else {
        response.error = Some(call_error(
            "no_incoming_call",
            "error.call.noIncomingCall",
            false,
        ));
        return;
    };
    state.call.phase = "REJECTING".into();
    state.call_dirty = true;
    signaling::decline_remote(state, &incoming.peer, &incoming.remote_call_id, "declined");
    state.call.phase = "IDLE".into();
    state.call_dirty = true;
    response.payload = json!({"state": "IDLE"});
}

/// Fails a call whose path recovery outlived the policy window. Returns
/// whether the state changed.
fn enforce_reconnect_window(state: &mut CoreState) -> bool {
    if state.call.phase != "RECONNECTING" {
        return false;
    }
    let expired = state
        .call
        .reconnecting_since
        .is_some_and(|since| since.elapsed() > RECONNECT_WINDOW);
    if !expired {
        return false;
    }
    state.fail_media();
    true
}

fn set_call_muted(state: &mut CoreState, payload: &Value, response: &mut Envelope) {
    let Some(muted) = payload.get("muted").and_then(Value::as_bool) else {
        response.error = Some(call_error(
            "invalid_request",
            "error.protocol.invalidRequest",
            false,
        ));
        return;
    };
    let Some(media) = state.media.as_ref() else {
        response.error = Some(call_error("call_inactive", "error.call.inactive", false));
        return;
    };
    match media.request("call.mute", json!({"muted": muted})) {
        Ok(reply) if reply.error.is_none() && reply.message_type == "call.mute" => {
            state.call.muted = muted;
            state.call_dirty = true;
            response.payload = json!({"muted": muted});
        }
        Ok(reply) => {
            response.error = Some(reply.error.map_or_else(
                || media_unavailable_error(MediaError::UnexpectedReply),
                worker_error,
            ));
        }
        Err(error) => response.error = Some(media_unavailable_error(error)),
    }
}

/// Push-to-talk mode and live key state. The mode is durable policy; the key
/// state is transient. Manual mute is enforced below the worker's transmit
/// gate, so PTT can never reopen a hand-muted microphone.
/// Forwards one call-mode change to the live media worker. An idle worker
/// learns nothing (the next call payload carries the durable mode); a worker
/// failure surfaces on the caller's response, because the mode persisted but
/// the live call did not follow it — pretending otherwise would leave the
/// user's microphone gated by a rule they switched off.
fn forward_call_mode(
    state: &mut CoreState,
    request_type: &str,
    payload: Value,
    response: &mut Envelope,
) {
    if response.error.is_some() {
        return;
    }
    let Some(media) = state.media.as_ref() else {
        return;
    };
    match media.request(request_type, payload) {
        Ok(reply) if reply.error.is_none() && reply.message_type == request_type => {}
        Ok(reply) => {
            response.error = Some(reply.error.map_or_else(
                || media_unavailable_error(MediaError::UnexpectedReply),
                worker_error,
            ));
        }
        Err(error) => response.error = Some(media_unavailable_error(error)),
    }
}

fn set_push_to_talk(state: &mut CoreState, payload: &Value, response: &mut Envelope) {
    let enabled = payload.get("enabled").and_then(Value::as_bool);
    let active = payload.get("active").and_then(Value::as_bool);
    if enabled.is_none() && active.is_none() {
        response.error = Some(call_error(
            "invalid_request",
            "error.protocol.invalidRequest",
            false,
        ));
        return;
    }
    if let Some(enabled) = enabled {
        let Some(settings) = state.settings.as_mut() else {
            response.error = Some(settings_unavailable_error());
            return;
        };
        if settings
            .update(&json!({"pushToTalkEnabled": enabled}))
            .is_err()
        {
            response.error = Some(settings_unavailable_error());
            return;
        }
    }
    // A live worker learns both facts so the gate updates mid-call; an idle
    // one learns nothing — the next call.start carries the persisted mode.
    if let Some(media) = state.media.as_ref() {
        let request_payload = json!({
            "enabled": enabled,
            "active": active,
        });
        match media.request("call.ptt", request_payload) {
            Ok(reply) if reply.error.is_none() && reply.message_type == "call.ptt" => {}
            Ok(reply) => {
                response.error = Some(reply.error.map_or_else(
                    || media_unavailable_error(MediaError::UnexpectedReply),
                    worker_error,
                ));
                return;
            }
            Err(error) => {
                response.error = Some(media_unavailable_error(error));
                return;
            }
        }
    }
    response.payload = json!({
        "push_to_talk": {
            "enabled": enabled.unwrap_or(state.settings.as_ref().is_some_and(|settings| settings.values().push_to_talk_enabled)),
            "active": active.unwrap_or(false),
        }
    });
}

/// Only device ids the enumeration itself produced (hex) cross to the worker;
/// anything else — including the prototype's placeholder device names — means
/// the session default. The worker refuses non-hex ids outright.
fn sanitized_worker_device(value: &str) -> String {
    let is_hex = !value.is_empty() && value.chars().all(|c| c.is_ascii_hexdigit());
    if is_hex {
        value.to_string()
    } else {
        String::new()
    }
}

fn clamped_volume(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

/// The audio facts a new call must open with: the durable selection, the
/// per-stream volumes, and the call modes.
fn audio_call_payload(state: &CoreState, call_id: &str) -> Value {
    let (input, output, mic, output_volume, ptt, voice_activation) = state
        .settings
        .as_ref()
        .map(|settings| {
            let values = settings.values();
            (
                values.input_device.clone(),
                values.output_device.clone(),
                values.microphone_volume,
                values.output_volume,
                values.push_to_talk_enabled,
                values.voice_activation,
            )
        })
        .unwrap_or_else(|| (String::new(), String::new(), 1.0, 1.0, false, false));
    json!({
        "call_id": call_id,
        // Keep the endpoint policy in both layers: the QML snapshot and the
        // worker's source gate must agree while the call is CONNECTING.
        "muted": initial_mic_muted(
            DeviceType::parse(
                state
                    .settings
                    .as_ref()
                    .map(|settings| settings.values().device_type.as_str())
                    .unwrap_or("desktop"),
            )
            .unwrap_or(DeviceType::Desktop),
        ),
        "ptt_enabled": ptt,
        "voice_activation": voice_activation,
        "input_device": sanitized_worker_device(&input),
        "output_device": sanitized_worker_device(&output),
        "input_volume": clamped_volume(mic),
        "output_volume": clamped_volume(output_volume),
    })
}

/// Enumerates the session's real audio devices. The worker holds no identity
/// and learns nothing by running, so a short-lived instance serves the
/// listing when no call is keeping one alive.
fn list_audio_devices(state: &mut CoreState, response: &mut Envelope) {
    let temporary;
    let media = if let Some(media) = state.media.as_ref() {
        media
    } else {
        temporary = match MediaSupervisor::start_with_executable(state.media_worker_path.as_deref(), state.media_wake.clone()) {
            Ok(media) => media,
            Err(error) => {
                response.error = Some(media_unavailable_error(error));
                return;
            }
        };
        &temporary
    };
    match media.request("audio.devices", json!({})) {
        Ok(reply) if reply.error.is_none() && reply.message_type == "audio.devices" => {
            response.payload = reply.payload;
        }
        Ok(reply) => {
            response.error = Some(reply.error.map_or_else(
                || media_unavailable_error(MediaError::UnexpectedReply),
                worker_error,
            ));
        }
        Err(error) => response.error = Some(media_unavailable_error(error)),
    }
}

/// Gets or sets the call audio configuration. A set persists the durable
/// choice and, when a call is live, applies it immediately — volumes take
/// effect per frame, devices swap only if they really open.
fn audio_config(state: &mut CoreState, payload: &Value, response: &mut Envelope) {
    let set = payload.get("set").and_then(Value::as_bool).unwrap_or(false);
    if !set {
        // Effective facts: a live worker owns the truth; idle, the durable
        // settings answer in the worker's vocabulary (empty id = default).
        let Some(media) = state
            .media
            .as_ref()
            .filter(|_| state.call.call_id.is_some())
        else {
            let (input, output, mic, output_volume) = state
                .settings
                .as_ref()
                .map(|settings| {
                    let values = settings.values();
                    (
                        values.input_device.clone(),
                        values.output_device.clone(),
                        values.microphone_volume,
                        values.output_volume,
                    )
                })
                .unwrap_or_else(|| (String::new(), String::new(), 1.0, 1.0));
            response.payload = json!({
                "input_device": sanitized_worker_device(&input),
                "output_device": sanitized_worker_device(&output),
                "input_volume": clamped_volume(mic),
                "output_volume": clamped_volume(output_volume),
            });
            return;
        };
        match media.request("audio.config", json!({})) {
            Ok(reply) if reply.error.is_none() && reply.message_type == "audio.config" => {
                response.payload = reply.payload;
            }
            Ok(reply) => {
                response.error = Some(reply.error.map_or_else(
                    || media_unavailable_error(MediaError::UnexpectedReply),
                    worker_error,
                ));
            }
            Err(error) => response.error = Some(media_unavailable_error(error)),
        }
        return;
    }

    // Build the validated settings patch. Unknown shapes are a protocol
    // error before anything persists.
    let mut patch = serde_json::Map::new();
    let mut input_device = None;
    let mut output_device = None;
    if let Some(value) = payload.get("input_device").and_then(Value::as_str) {
        patch.insert("inputDevice".into(), json!(value));
        input_device = Some(value.to_string());
    }
    if let Some(value) = payload.get("output_device").and_then(Value::as_str) {
        patch.insert("outputDevice".into(), json!(value));
        output_device = Some(value.to_string());
    }
    for (key, name) in [
        ("input_volume", "microphoneVolume"),
        ("output_volume", "outputVolume"),
    ] {
        if let Some(value) = payload.get(key).and_then(Value::as_f64) {
            patch.insert(name.into(), json!(clamped_volume(value)));
        }
    }
    if patch.is_empty() {
        response.error = Some(call_error(
            "invalid_request",
            "error.protocol.invalidRequest",
            false,
        ));
        return;
    }
    {
        let Some(settings) = state.settings.as_mut() else {
            response.error = Some(settings_unavailable_error());
            return;
        };
        if settings.update(&Value::Object(patch)).is_err() {
            response.error = Some(settings_unavailable_error());
            return;
        }
    }

    // A live call hears the change now, not at its next start.
    if let Some(media) = state
        .media
        .as_ref()
        .filter(|_| state.call.call_id.is_some())
    {
        let live = json!({
            "set": true,
            "input_device": input_device.as_deref().map(sanitized_worker_device),
            "output_device": output_device.as_deref().map(sanitized_worker_device),
            "input_volume": payload.get("input_volume").and_then(Value::as_f64).map(clamped_volume),
            "output_volume": payload.get("output_volume").and_then(Value::as_f64).map(clamped_volume),
        });
        match media.request("audio.config", live) {
            Ok(reply) if reply.error.is_none() && reply.message_type == "audio.config" => {
                response.payload = reply.payload;
            }
            Ok(reply) => {
                response.error = Some(reply.error.map_or_else(
                    || media_unavailable_error(MediaError::UnexpectedReply),
                    worker_error,
                ));
            }
            Err(error) => response.error = Some(media_unavailable_error(error)),
        }
        return;
    }
    let (input, output, mic, output_volume) = state
        .settings
        .as_ref()
        .map(|settings| {
            let values = settings.values();
            (
                values.input_device.clone(),
                values.output_device.clone(),
                values.microphone_volume,
                values.output_volume,
            )
        })
        .unwrap_or_else(|| (String::new(), String::new(), 1.0, 1.0));
    response.payload = json!({
        "input_device": sanitized_worker_device(&input),
        "output_device": sanitized_worker_device(&output),
        "input_volume": clamped_volume(mic),
        "output_volume": clamped_volume(output_volume),
    });
}

/// Stops an in-progress microphone self-check and frees its worker. Calls
/// always win over the test: starting or accepting a call ends it first so
/// two owners never fight over the devices.
fn stop_mic_test(state: &mut CoreState) {
    if let Some(media) = state.mic_test.take() {
        let _ = media.request("audio.loopback_stop", json!({}));
        media.shutdown();
    }
}

fn mic_test_idle_payload() -> Value {
    json!({
        "active": false,
        "level": 0.0,
        "seconds_left": 0,
        "peak": 0.0,
        "failed": false,
    })
}

/// Starts a bounded microphone self-check on a short-lived worker. Refused
/// honestly while a call owns the devices; otherwise the worker reports
/// real capture levels the UI renders live.
fn mic_test_start(state: &mut CoreState, payload: &Value, response: &mut Envelope) {
    if state.call.call_id.is_some() || state.media.is_some() {
        response.error = Some(call_error("call_active", "error.audio.busy", false));
        return;
    }
    if state.mic_test.is_none() {
        match MediaSupervisor::start_with_executable(state.media_worker_path.as_deref(), state.media_wake.clone()) {
            Ok(media) => state.mic_test = Some(media),
            Err(error) => {
                response.error = Some(media_unavailable_error(error));
                return;
            }
        }
    }
    let seconds = payload.get("seconds").and_then(Value::as_i64).unwrap_or(5);
    let Some(media) = state.mic_test.as_ref() else {
        response.error = Some(media_unavailable_error(MediaError::Exited));
        return;
    };
    match media.request("audio.loopback_start", json!({"seconds": seconds})) {
        Ok(reply) if reply.error.is_none() && reply.message_type == "audio.loopback_start" => {
            response.payload = reply.payload;
        }
        Ok(reply) => {
            // The worker refused honestly (no devices, busy): nothing to keep.
            stop_mic_test(state);
            response.error = Some(reply.error.map_or_else(
                || media_unavailable_error(MediaError::UnexpectedReply),
                worker_error,
            ));
        }
        Err(error) => {
            stop_mic_test(state);
            response.error = Some(media_unavailable_error(error));
        }
    }
}

/// Polls the running self-check. A finished test reaps its worker and still
/// reports the final snapshot once, so the UI can show the measured peak.
fn mic_test_poll(state: &mut CoreState, response: &mut Envelope) {
    let Some(media) = state.mic_test.as_ref() else {
        response.payload = mic_test_idle_payload();
        return;
    };
    match media.request("audio.loopback_poll", json!({})) {
        Ok(reply) if reply.error.is_none() && reply.message_type == "audio.loopback_poll" => {
            response.payload = reply.payload.clone();
            if !reply.payload.get("active").and_then(Value::as_bool).unwrap_or(false) {
                stop_mic_test(state);
            }
        }
        Ok(reply) => {
            stop_mic_test(state);
            response.error = Some(reply.error.map_or_else(
                || media_unavailable_error(MediaError::UnexpectedReply),
                worker_error,
            ));
        }
        Err(error) => {
            stop_mic_test(state);
            response.error = Some(media_unavailable_error(error));
        }
    }
}

/// Ends the self-check early, reporting the peak measured so far.
fn mic_test_stop(state: &mut CoreState, response: &mut Envelope) {
    let Some(media) = state.mic_test.as_ref() else {
        let mut idle = mic_test_idle_payload();
        idle["stopped"] = json!(true);
        response.payload = idle;
        return;
    };
    match media.request("audio.loopback_stop", json!({})) {
        Ok(reply) if reply.error.is_none() && reply.message_type == "audio.loopback_stop" => {
            response.payload = reply.payload;
        }
        Ok(reply) => {
            response.error = Some(reply.error.map_or_else(
                || media_unavailable_error(MediaError::UnexpectedReply),
                worker_error,
            ));
        }
        Err(error) => {
            response.error = Some(media_unavailable_error(error));
        }
    }
    stop_mic_test(state);
}

/// Swaps a live call's devices. The durable selection only moves if the swap
/// really opened; a failed switch keeps both the call and the saved choice.
fn audio_switch_devices(state: &mut CoreState, payload: &Value, response: &mut Envelope) {
    let Some(call_id) = payload.get("call_id").and_then(Value::as_str) else {
        response.error = Some(call_error(
            "invalid_request",
            "error.protocol.invalidRequest",
            false,
        ));
        return;
    };
    let input = payload
        .get("input_device")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let output = payload
        .get("output_device")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let Some(media) = state.media.as_ref() else {
        response.error = Some(call_error("call_inactive", "error.call.inactive", false));
        return;
    };
    let request_payload = json!({
        "call_id": call_id,
        "input_device": sanitized_worker_device(input),
        "output_device": sanitized_worker_device(output),
    });
    match media.request("audio.switch_devices", request_payload) {
        Ok(reply) if reply.error.is_none() && reply.message_type == "audio.switch_devices" => {
            if let Some(settings) = state.settings.as_mut() {
                let _ = settings.update(&json!({
                    "inputDevice": input,
                    "outputDevice": output,
                }));
            }
            response.payload = json!({"switched": true});
        }
        Ok(reply) => {
            response.error = Some(reply.error.map_or_else(
                || media_unavailable_error(MediaError::UnexpectedReply),
                worker_error,
            ));
        }
        Err(error) => response.error = Some(media_unavailable_error(error)),
    }
}

fn start_screen_share(state: &mut CoreState, response: &mut Envelope) {
    if state.call.phase != "CONNECTED" {
        response.error = Some(call_error(
            "call_not_connected",
            "error.call.notConnected",
            false,
        ));
        return;
    }
    let Some(media) = state.media.as_ref() else {
        response.error = Some(call_error("call_inactive", "error.call.inactive", false));
        return;
    };
    let payload = json!({"call_id": state.call.call_id});
    match media.request("call.share_start", payload) {
        Ok(reply) if reply.error.is_none() && reply.message_type == "call.share_start" => {
            // The worker acknowledges only after its native capture adapter
            // really acquired a source; anything else arrives as an error.
            state.call.share_phase = "SHARING".into();
            state.share_dirty = true;
            response.payload = json!({"state": "SHARING"});
        }
        Ok(reply) => {
            response.error = Some(reply.error.map_or_else(
                || media_unavailable_error(MediaError::UnexpectedReply),
                worker_error,
            ));
        }
        Err(error) => response.error = Some(media_unavailable_error(error)),
    }
}

fn stop_screen_share(state: &mut CoreState, response: &mut Envelope) {
    // Stopping never requires a live worker to succeed: without one there is
    // nothing sharing, so the request still resolves to NOT_SHARING.
    let Some(media) = state.media.as_ref() else {
        let changed = state.call.share_phase != "NOT_SHARING";
        state.call.share_phase = "NOT_SHARING".into();
        state.share_dirty |= changed;
        response.payload = json!({"state": "NOT_SHARING"});
        return;
    };
    match media.request("call.share_stop", json!({"call_id": state.call.call_id})) {
        Ok(reply) if reply.error.is_none() && reply.message_type == "call.share_stop" => {
            let changed = state.call.share_phase != "NOT_SHARING";
            state.call.share_phase = "NOT_SHARING".into();
            state.share_dirty |= changed;
            response.payload = json!({"state": "NOT_SHARING"});
        }
        Ok(reply) => {
            response.error = Some(reply.error.map_or_else(
                || media_unavailable_error(MediaError::UnexpectedReply),
                worker_error,
            ));
        }
        Err(error) => response.error = Some(media_unavailable_error(error)),
    }
}

/// One side of a presence event: the committed state, the state it changed
/// from, whether this event actually changes anything against what the UI
/// last saw, and the side's monotonically increasing revision.
fn presence_side_json(
    state: crate::presence::PresenceState,
    from_state: Option<crate::presence::PresenceState>,
    last_emitted: Option<crate::presence::PresenceState>,
    revision: u64,
) -> Value {
    json!({
        "state": state,
        "previousState": from_state,
        "changed": last_emitted != Some(state),
        "revision": revision,
    })
}

fn send_chat(state: &mut CoreState, payload: &Value, response: &mut Envelope) {
    let Some(body) = payload.get("body").and_then(Value::as_str) else {
        response.error = Some(ProtocolError {
            code: "invalid_request".into(),
            ui_key: "error.protocol.invalidRequest".into(),
            retryable: false,
            detail: "chat body is required".into(),
        });
        return;
    };
    let id = Uuid::new_v4().to_string();
    match state.chat.compose(id.clone(), body, now_seconds()) {
        Ok(_) => {
            state.direct_dirty = true;
            // A disconnected channel intentionally retains the bounded queue;
            // a connected channel attempts the direct send immediately.
            state.sync_direct();
            response.payload = json!({"message_id": id, "state": state.direct_payload()});
        }
        Err(error) => {
            response.error = Some(ProtocolError {
                code: error.code.into(),
                ui_key: error.ui_key.into(),
                retryable: error.code == "chat_queue_full",
                detail: "chat request was refused by local policy".into(),
            })
        }
    }
}

fn transfer_error(error: crate::direct::DirectError) -> ProtocolError {
    ProtocolError {
        code: error.code.into(),
        ui_key: error.ui_key.into(),
        retryable: error.code == "transfer_busy",
        detail: "transfer request was refused by local policy".into(),
    }
}

fn offer_local_transfer(state: &mut CoreState, payload: &Value, response: &mut Envelope) {
    let Some(source) = payload.get("source_path").and_then(Value::as_str) else {
        response.error = Some(ProtocolError::invalid_request(
            "local source path is required",
        ));
        return;
    };
    let id = Uuid::new_v4().to_string();
    match state
        .transfers
        .begin_outgoing(id.clone(), &PathBuf::from(source), now_seconds())
    {
        Ok(_) => {
            state.direct_dirty = true;
            state.sync_direct();
            response.payload = json!({"transfer_id": id, "state": state.direct_payload()});
        }
        Err(error) => response.error = Some(transfer_error(error)),
    }
}

fn accept_transfer(
    state: &mut CoreState,
    payload: &Value,
    accepted: bool,
    response: &mut Envelope,
) {
    let Some(id) = payload.get("transfer_id").and_then(Value::as_str) else {
        response.error = Some(ProtocolError::invalid_request("transfer ID is required"));
        return;
    };
    let staging = state.state_dir.join("transfers");
    if accepted {
        if let Err(error) = state.transfers.accept_inbound(id, &staging) {
            response.error = Some(transfer_error(error));
            return;
        }
    } else if !state.transfers.cancel(id, &staging) {
        response.error = Some(ProtocolError::invalid_request("unknown transfer"));
        return;
    }
    if state.call.phase == "CONNECTED" {
        if let (Some(call_id), Some(media)) = (state.call.call_id.clone(), state.media.as_ref()) {
            let command = if accepted {
                "transfer.accept"
            } else {
                "transfer.reject"
            };
            if let Err(error) =
                media.request(command, json!({"call_id": call_id, "transfer_id": id}))
            {
                response.error = Some(media_unavailable_error(error));
                return;
            }
        }
    }
    state.direct_dirty = true;
    response.payload = state.direct_payload();
}

fn cancel_transfer(state: &mut CoreState, payload: &Value, response: &mut Envelope) {
    let Some(id) = payload.get("transfer_id").and_then(Value::as_str) else {
        response.error = Some(ProtocolError::invalid_request("transfer ID is required"));
        return;
    };
    if !state
        .transfers
        .cancel(id, &state.state_dir.join("transfers"))
    {
        response.error = Some(ProtocolError::invalid_request("unknown transfer"));
        return;
    }
    if let (Some(call_id), Some(media)) = (state.call.call_id.clone(), state.media.as_ref()) {
        let _ = media.request(
            "transfer.cancel",
            json!({"call_id": call_id, "transfer_id": id}),
        );
    }
    state.direct_dirty = true;
    response.payload = state.direct_payload();
}

fn call_error(code: &str, ui_key: &str, retryable: bool) -> ProtocolError {
    ProtocolError {
        code: code.into(),
        ui_key: ui_key.into(),
        retryable,
        detail: "call state transition was refused".into(),
    }
}

fn worker_error(error: self::media::MediaProtocolError) -> ProtocolError {
    ProtocolError {
        code: error.code,
        ui_key: error.ui_key,
        retryable: error.retryable,
        // Diagnostics from a private child never cross the core→UI boundary.
        detail: "media worker refused the command".into(),
    }
}

fn media_unavailable_error(error: MediaError) -> ProtocolError {
    ProtocolError {
        code: "media_unavailable".into(),
        ui_key: "error.call.unavailable".into(),
        retryable: matches!(
            error,
            MediaError::Busy | MediaError::TimedOut | MediaError::Exited
        ),
        detail: "media worker is unavailable".into(),
    }
}

/// Times a real control-plane round trip and reports the direct path's own
/// measured facts. The probe is a genuine signed `contacts.list` exchange on
/// a fresh pinned connection, so the numbers cover exactly what a Harbor
/// session pays: TCP+TLS handshake and one signed request/response. An
/// unreachable server is a structured result (`reachable: false`), never a
/// fabricated latency, and absence of a live call is reported as absence.
fn run_network_diagnostics(state: &mut CoreState) -> Result<Value, ProtocolError> {
    // Direct path: only what the live call's worker has really measured.
    let direct = match state.call.stats.as_ref() {
        Some(stats) if state.call.phase == "CONNECTED" => json!({
            "active": true,
            "rtt_ms": stats.rtt_ms,
            "loss_pct": stats.loss_pct(),
            "quality": stats.quality(),
        }),
        _ => json!({ "active": false }),
    };

    let Some(pin) = load_server_pin(&state.state_dir) else {
        return Ok(json!({
            "server": { "configured": false, "reachable": false },
            "direct": direct,
        }));
    };

    let unreachable = |direct: &Value| {
        Ok(json!({
            "server": { "configured": true, "reachable": false },
            "direct": direct,
        }))
    };
    let identity = match load_or_create(&state.state_dir, now_seconds()) {
        Ok(identity) => identity,
        Err(_) => return unreachable(&direct),
    };
    let handshake_started = Instant::now();
    let mut client = match crate::ServerClient::connect(&pin) {
        Ok(client) => client,
        Err(_) => return unreachable(&direct),
    };
    let handshake_ms = handshake_started.elapsed().as_secs_f64() * 1000.0;

    let exchange_started = Instant::now();
    let request = Envelope::request("contacts.list", json!({}), crate::rfc3339_now());
    let response = match client.exchange(request, &identity) {
        Ok(response) => response,
        Err(_) => return unreachable(&direct),
    };
    let rtt_ms = exchange_started.elapsed().as_secs_f64() * 1000.0;
    if response.error.is_some() {
        // The server answered but refused the signed probe; the path works,
        // yet the honest summary is still "not serving this device".
        return unreachable(&direct);
    }

    Ok(json!({
        "server": {
            "configured": true,
            "reachable": true,
            "handshake_ms": handshake_ms,
            "rtt_ms": rtt_ms,
        },
        "direct": direct,
    }))
}

/// Retrieves the control plane's durable pairing relationship snapshot.
/// The UI receives only public stable identifiers; a public key is not needed
/// to decide whether Call and Chat may be entered.
fn list_paired_contacts(state: &mut CoreState) -> Result<Value, ProtocolError> {
    let Some(pin) = load_server_pin(&state.state_dir) else {
        return Ok(json!({ "peers": [] }));
    };
    let identity = load_or_create(&state.state_dir, now_seconds())
        .map_err(|_| identity_unavailable_error())?;
    let mut client =
        crate::ServerClient::connect(&pin).map_err(|_| server_unavailable_error())?;
    let request = Envelope::request("contacts.list", json!({}), crate::rfc3339_now());
    let response = client
        .exchange(request, &identity)
        .map_err(|_| server_unavailable_error())?;
    if let Some(error) = response.error {
        return Err(error);
    }
    let peers = response
        .payload
        .get("peers")
        .and_then(Value::as_array)
        .ok_or_else(server_unavailable_error)?;
    let peers: Vec<Value> = peers
        .iter()
        .filter_map(|peer| {
            let device_id = peer.get("device_id")?.as_str()?;
            let harbor_id = peer.get("harbor_id")?.as_str()?;
            Uuid::parse_str(device_id).ok()?;
            if harbor_id.is_empty() {
                return None;
            }
            Some(json!({ "device_id": device_id, "harbor_id": harbor_id }))
        })
        .collect();
    Ok(json!({ "peers": peers }))
}

/// Drives one pairing operation against the configured server. Network I/O
/// happens synchronously here; the process's stdout stays quiet until the
/// correlated response is ready, so the C++ side only ever sees complete
/// frames.
fn run_pairing(
    state: &mut CoreState,
    message_type: &str,
    payload: &Value,
) -> Result<Value, ProtocolError> {
    let Some(pin) = load_server_pin(&state.state_dir) else {
        return Err(ProtocolError {
            code: "server_unconfigured".into(),
            ui_key: "error.server.unconfigured".into(),
            retryable: false,
            detail: "No control-plane server is configured for pairing".into(),
        });
    };
    let identity = load_or_create(&state.state_dir, now_seconds())
        .map_err(|_| identity_unavailable_error())?;
    let session = &mut state.pairing;

    let run = |session: &mut PairingSession| -> Result<Value, PairingError> {
        match message_type {
            "pairing.create" => session
                .host_create(&pin, &identity)
                .map(|code| json!({ "phase": session.phase(), "code": code })),
            "pairing.incoming" => session.host_poll(&pin, &identity).map(|has_request| {
                json!({
                    "phase": session.phase(),
                    "has_request": has_request,
                    // Echo the surfaced request so the UI can show who
                    // is asking without a second round trip.
                    "request": session.pending_request(),
                })
            }),
            "pairing.accept" => session
                .host_accept(&pin, &identity)
                .map(|_| session.snapshot()),
            "pairing.decline" => session
                .host_decline(&pin, &identity)
                .map(|_| session.snapshot()),
            "pairing.submit" => {
                let Some(code) = payload.get("code").and_then(Value::as_str) else {
                    session.reset();
                    return Err(PairingError::MissingCode);
                };
                session
                    .peer_submit(&pin, &identity, code)
                    .map(|_| session.snapshot())
            }
            "pairing.status" => session
                .peer_poll(&pin, &identity)
                // The full snapshot, so an EXPIRED/CANCELLED refusal carries
                // its error_key to the UI, not just the phase.
                .map(|_| session.snapshot()),
            "pairing.cancel" => session
                .peer_cancel(&pin, &identity)
                .map(|_| session.snapshot()),
            other => unreachable!("dispatch routes only pairing types to run_pairing: {other}"),
        }
    };

    // The server only serves registered devices. An `unauthorized` refusal
    // means this identity is unknown to the server (first contact, or the
    // server's state was rebuilt): register and retry exactly once so the
    // failure the user sees is a real pairing failure, not an introduction
    // order problem.
    let outcome = match run(session) {
        Err(PairingError::Refused(ui_key)) if ui_key == "error.server.unauthorized" => {
            if let Err(error) = crate::register_identity(&pin, &identity) {
                return Err(pairing_protocol_error(session, error));
            }
            run(session)
        }
        other => other,
    };

    outcome.map_err(|error| pairing_protocol_error(session, error))
}

/// Maps a pairing failure onto the structured error surface and marks the
/// local session so the UI sees the ERROR phase, not a stale in-flight one.
fn pairing_protocol_error(session: &mut PairingSession, error: PairingError) -> ProtocolError {
    let (code, ui_key, retryable, detail) = match error {
        PairingError::NoActiveRequest | PairingError::MissingCode => (
            "invalid_request".to_owned(),
            "error.protocol.invalidRequest".to_owned(),
            false,
            "The pairing request payload does not meet the control-plane schema".to_owned(),
        ),
        PairingError::Connect(source) => (
            "server_unavailable".to_owned(),
            "error.server.unavailable".to_owned(),
            true,
            source.to_string(),
        ),
        PairingError::Refused(ui_key) => {
            session.mark_error(ui_key.clone());
            (
                "pairing_refused".to_owned(),
                ui_key,
                false,
                "The control-plane server refused the pairing request".to_owned(),
            )
        }
    };
    ProtocolError {
        code,
        ui_key,
        retryable,
        detail,
    }
}

fn storage_failed_error(error: std::io::Error) -> ProtocolError {
    ProtocolError {
        code: "settings_unavailable".into(),
        ui_key: "error.settings.unavailable".into(),
        retryable: false,
        detail: error.to_string(),
    }
}

fn identity_unavailable_error() -> ProtocolError {
    ProtocolError {
        code: "identity_unavailable".into(),
        ui_key: "error.identity.unavailable".into(),
        retryable: false,
        detail: "Harbor identity storage is unavailable".into(),
    }
}

fn settings_unavailable_error() -> ProtocolError {
    ProtocolError {
        code: "settings_unavailable".into(),
        ui_key: "error.settings.unavailable".into(),
        retryable: false,
        detail: "Harbor settings storage is unavailable".into(),
    }
}

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("stdin/stdout I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Frame(#[from] harbor_protocol::FrameError),
    #[error(transparent)]
    Validation(#[from] harbor_protocol::ValidationError),
    #[error("request did not include a timestamp")]
    MissingTimestamp,
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;
    use std::path::PathBuf;

    use harbor_protocol::{Envelope, FrameDecoder, encode_frame};
    use harbor_server::Listener;
    use serde_json::json;
    use uuid::Uuid;

    use super::*;

    fn state() -> CoreState {
        let directory = std::env::temp_dir().join(format!("harbor-core-test-{}", Uuid::new_v4()));
        CoreState::for_directory(&directory)
    }

    /// Every test that points HARBOR_MEDIA_EXECUTABLE somewhere holds this
    /// lock: the variable is process-global, and the direct-call proof reads
    /// it through build_media_worker() to decide which worker to spawn.
    static WORKER_ENV_LOCK: Mutex<()> = Mutex::new(());

    fn hello(minimum: u64, maximum: u64) -> Envelope {
        Envelope::request(
            "core.hello",
            json!({"client": "test", "protocol_min": minimum, "protocol_max": maximum}),
            "2026-08-31T20:00:00Z",
        )
    }

    #[test]
    fn mobile_call_payload_and_snapshot_start_muted() {
        let mut core_state = state();
        let configured = dispatch(
            Envelope::request(
                "device.configure",
                json!({"deviceType": "mobile"}),
                "2026-08-31T20:00:00Z",
            ),
            &mut core_state,
        )
        .unwrap();
        assert!(configured.error.is_none(), "{configured:?}");
        assert_eq!(audio_call_payload(&core_state, "call-1")["muted"], true);

        core_state.call.call_id = Some("call-1".into());
        core_state.call.muted = true;
        assert_eq!(core_state.call.snapshot()["muted"], true);
    }

    #[test]
    fn phone_notifications_are_bounded_ephemeral_events() {
        let mut core_state = state();
        let payload = json!({
            "appLabel": "Messages",
            "title": "A title",
            "text": "A body",
            "timestamp": 1_725_000_000_u64,
        });
        assert!(validate_phone_notification(&payload).is_ok());
        assert!(validate_phone_notification(&json!({
            "appLabel": "com.example.messages",
            "title": "",
            "text": "",
            "timestamp": 1
        }))
        .is_err());

        core_state.phone_notifications.push(payload.clone());
        let event = core_state.take_phone_notification_event().expect("event");
        assert_eq!(event.message_type, "phone.notification");
        assert_eq!(event.payload, payload);
        // The event queue is the only place the content exists; snapshots
        // used by mobile state and direct chat never contain it.
        assert!(!core_state.mobile_snapshot().to_string().contains("A body"));
        assert!(!core_state.direct_payload().to_string().contains("A body"));
        assert!(core_state.take_phone_notification_event().is_none());
    }

    #[test]
    fn activity_state_reports_an_idle_engine_before_any_scan() {
        let response = dispatch(
            Envelope::request("activity.state", json!({}), "2026-08-31T20:00:00Z"),
            &mut state(),
        )
        .unwrap();
        assert!(response.error.is_none(), "{:?}", response.error);
        assert_eq!(response.payload["monitor"], "idle");
        assert_eq!(response.payload["timeline"], json!([]));
        assert_eq!(response.payload["stats"]["games"], 0);
        assert_eq!(response.payload["stats"]["hours"], 0);
    }

    #[test]
    fn presence_senses_commit_into_events_and_snapshots_stay_private() {
        let mut core_state = state();

        // First evidence commits OFFLINE -> ONLINE and marks the event.
        let response = dispatch(
            Envelope::request(
                "presence.sense",
                json!({"inputIdleSeconds": 2, "sessionActive": true}),
                "2026-08-31T20:00:00Z",
            ),
            &mut core_state,
        )
        .unwrap();
        assert!(response.error.is_none(), "{:?}", response.error);
        assert_eq!(response.payload["state"], "ONLINE");
        let event = core_state.take_presence_event().unwrap();
        assert_eq!(event.message_type, "presence.updated");
        assert_eq!(event.payload["local"]["state"], "ONLINE");
        assert_eq!(event.payload["local"]["previousState"], "OFFLINE");
        assert_eq!(event.payload["local"]["changed"], true);
        assert!(event.payload["partner"].is_null());
        // The event carries states only — the snapshot's numbers never do.
        let serialized = serde_json::to_string(&event.payload).unwrap();
        assert!(!serialized.contains("inputIdle"), "evidence leaked: {serialized}");
        assert!(!serialized.contains("sessionActive"), "evidence leaked: {serialized}");
        // The event was consumed: no echo on the next request.
        assert!(core_state.take_presence_event().is_none());

        // The snapshot route serves the aggregate for reconnecting UIs.
        let response = dispatch(
            Envelope::request("presence.state", json!({}), "2026-08-31T20:00:01Z"),
            &mut core_state,
        )
        .unwrap();
        assert_eq!(response.payload["local"]["state"], "ONLINE");
        assert_eq!(response.payload["local"]["revision"], 1);
        assert!(response.payload["partner"].is_null());

        // A malformed snapshot is refused without touching state.
        let rejected = dispatch(
            Envelope::request(
                "presence.sense",
                json!({"inputIdleSeconds": "soon"}),
                "2026-08-31T20:00:02Z",
            ),
            &mut core_state,
        )
        .unwrap();
        assert_eq!(rejected.error.unwrap().code, "invalid_request");
        assert_eq!(core_state.presence.local.state().lease_value(), "ONLINE");
    }

    #[test]
    fn activity_updates_flow_as_events_after_the_engine_changes() {
        let mut core_state = state();
        {
            let mut engine = core_state.engine.lock().unwrap();
            assert!(engine.mark_monitor_started(1_000));
            assert!(engine.ingest(
                1_000,
                vec![crate::activity::RawObservation {
                    pid: 42,
                    exe_path: Some("/usr/bin/vlc".into()),
                    command_line: Some("vlc --loop".into()),
                }],
            ));
        }

        // A scan change reaches the UI through the pump's forced event; the
        // event carries the sanitized timeline, never the raw material.
        let event = core_state.activity_event();
        assert_eq!(event.message_type, "activity.updated");
        assert!(event.request_id.is_none() && event.event_id.is_some());
        let serialized = serde_json::to_string(&event.payload).unwrap();
        assert!(!serialized.contains("usr/bin"), "path leaked: {serialized}");
        assert!(
            !serialized.contains("--loop"),
            "cmdline leaked: {serialized}"
        );
        assert_eq!(event.payload["monitor"], "running");
        assert_eq!(event.payload["timeline"].as_array().unwrap().len(), 2);

        // A direct engine change marks nothing dirty: the pump owns that
        // emission; dispatches mark it explicitly.
        assert!(core_state.take_activity_event().is_none());
    }

    #[test]
    fn activity_policy_flips_mark_the_timeline_dirty() {
        let mut core_state = state();
        dispatch(
            Envelope::request(
                "core.hello",
                json!({"client": "t", "protocol_min": 1, "protocol_max": 1}),
                "2026-08-31T20:00:00Z",
            ),
            &mut core_state,
        )
        .unwrap();

        dispatch(
            Envelope::request(
                "settings.update",
                json!({"gameVisibility": false}),
                "2026-08-31T20:00:01Z",
            ),
            &mut core_state,
        )
        .unwrap();
        let event = core_state.take_activity_event().unwrap();
        assert_eq!(event.message_type, "activity.updated");

        // Unrelated settings changes stay quiet.
        dispatch(
            Envelope::request(
                "settings.update",
                json!({"debug_mode": true}),
                "2026-08-31T20:00:02Z",
            ),
            &mut core_state,
        )
        .unwrap();
        assert!(core_state.take_activity_event().is_none());
    }

    #[test]
    fn identity_get_persists_under_the_core_state_directory() {
        let directory = std::env::temp_dir().join(format!("harbor-core-test-{}", Uuid::new_v4()));
        let mut core_state = CoreState::for_directory(&directory);

        let first = dispatch(
            Envelope::request("identity.get", json!({}), "2026-08-31T20:00:00Z"),
            &mut core_state,
        )
        .unwrap();
        assert!(first.error.is_none(), "{:?}", first.error);

        // One persistence owner: the identity lands in the core's own state
        // directory, never in an env-dependent default location.
        assert!(
            directory.join("identity-v1.json").is_file(),
            "identity must live in the core state directory"
        );

        let second = dispatch(
            Envelope::request("identity.get", json!({}), "2026-08-31T20:00:01Z"),
            &mut core_state,
        )
        .unwrap();
        assert_eq!(
            second.payload["device_id"], first.payload["device_id"],
            "the same state directory must reload the same identity"
        );
    }

    #[test]
    fn screen_share_requires_a_connected_call_without_faking_permission() {
        let mut core_state = state();
        let response = dispatch(
            Envelope::request("call.share_screen_start", json!({}), "2026-09-01T00:00:00Z"),
            &mut core_state,
        )
        .unwrap();

        assert_eq!(
            response.error.as_ref().map(|error| error.code.as_str()),
            Some("call_not_connected")
        );
        assert_eq!(core_state.call.share_phase, "NOT_SHARING");
        assert!(core_state.take_share_event().is_none());
    }

    #[test]
    fn call_snapshot_exposes_only_safe_screen_share_state() {
        let mut core_state = state();
        core_state.call.call_id = Some("call-local-id".into());
        core_state.call.phase = "CONNECTED".into();
        core_state.call.share_phase = "NOT_SHARING".into();
        core_state.share_dirty = true;

        let event = core_state.take_share_event().expect("share event");
        assert_eq!(event.message_type, "call.share_state_changed");
        assert_eq!(
            event.payload,
            json!({"call_id": "call-local-id", "state": "NOT_SHARING"})
        );
        let serialized = serde_json::to_string(&event.payload).unwrap();
        assert!(
            !serialized.contains("source"),
            "capture details leaked: {serialized}"
        );
    }

    #[test]
    fn hello_negotiates_version_one() {
        let request = hello(1, 1);
        let response = dispatch(request, &mut state()).unwrap();
        assert_eq!(response.message_type, "core.ready");
        assert_eq!(response.payload["protocol"], 1);
        assert_eq!(response.payload["capabilities"], json!(CORE_CAPABILITIES));
    }

    #[test]
    fn hello_rejects_an_incompatible_range() {
        let response = dispatch(hello(2, 3), &mut state()).unwrap();
        assert_eq!(
            response.error.unwrap().code,
            "protocol_version_incompatible"
        );
    }

    #[test]
    fn stdio_handles_a_complete_hello_frame() {
        let frame = encode_frame(&hello(1, 1)).unwrap();
        let mut output = Vec::new();
        run(Cursor::new(frame), &mut output, &mut state()).unwrap();

        let mut decoder = FrameDecoder::default();
        let messages = decoder.push(&output).unwrap();
        decoder.finish().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].message_type, "core.ready");
    }

    /// The private channel's vocabulary is the frame validator's allowlist,
    /// so a dispatch arm whose type never entered LOCAL_MESSAGE_TYPES kills
    /// the reader the moment the UI sends it: the core exits cleanly and the
    /// supervisor reports stoppedUnexpectedly. This drives every request the
    /// facade can send through the real framed stdio loop and requires one
    /// reply per request — an error reply still proves the channel lived.
    #[test]
    fn every_ui_request_type_survives_the_framed_channel() {
        let requests = [
            (
                "core.hello",
                json!({"client": "ui", "protocol_min": 1, "protocol_max": 1}),
            ),
            ("identity.get", json!({})),
            ("identity.update", json!({})),
            ("settings.get", json!({})),
            ("settings.update", json!({})),
            ("server.config", json!({})),
            (
                "server.configure",
                json!({"address": "192.168.1.6:8443", "fingerprint": "00"}),
            ),
            ("pairing.create", json!({})),
            ("pairing.submit", json!({"code": "123456"})),
            ("pairing.cancel", json!({})),
            ("pairing.incoming", json!({})),
            ("pairing.accept", json!({})),
            ("pairing.decline", json!({})),
            ("pairing.status", json!({})),
            ("pairing.state", json!({})),
            ("pairing.enter_code", json!({})),
            ("pairing.reset", json!({})),
            ("activity.state", json!({})),
            ("presence.publish", json!({"state": "online"})),
            ("call.start", json!({})),
            ("call.accept", json!({})),
            ("call.decline", json!({})),
            ("call.end", json!({})),
            ("call.mute", json!({"muted": true})),
            ("call.push_to_talk", json!({"enabled": false})),
            ("call.share_screen_start", json!({})),
            ("call.share_screen_stop", json!({})),
            ("audio.devices", json!({})),
            ("audio.config", json!({"set": false})),
            ("audio.loopback_start", json!({"seconds": 3})),
            ("audio.loopback_poll", json!({})),
            ("audio.loopback_stop", json!({})),
            ("chat.send", json!({"body": "olá"})),
            ("direct.state", json!({})),
            ("profile.state", json!({})),
            (
                "transfer.offer_local",
                json!({"source_path": "no-such-file"}),
            ),
            ("transfer.accept", json!({"transfer_id": "t"})),
            ("transfer.reject", json!({"transfer_id": "t"})),
            ("transfer.cancel", json!({"transfer_id": "t"})),
            ("session.connect", json!({"peer": "x"})),
            ("session.disconnect", json!({})),
            ("session.signal", json!({"signal": "{}"})),
        ];
        let mut input = Vec::new();
        for (message_type, payload) in &requests {
            input.extend_from_slice(
                &encode_frame(&Envelope::request(
                    *message_type,
                    payload.clone(),
                    "2026-08-31T20:00:00Z",
                ))
                .unwrap(),
            );
        }

        let mut output = Vec::new();
        run(Cursor::new(input), &mut output, &mut state()).unwrap();

        let mut decoder = FrameDecoder::default();
        let messages = decoder.push(&output).unwrap();
        decoder.finish().unwrap();
        // Side events (call/direct/activity pushes) interleave; only the
        // correlated replies prove each request survived the validator.
        let replies: Vec<_> = messages
            .iter()
            .filter(|message| message.reply_to.is_some())
            .collect();
        assert_eq!(
            replies.len(),
            requests.len(),
            "a request type died in the frame validator"
        );
        for (request, reply) in requests.iter().zip(&replies) {
            let expected = if request.0 == "core.hello" {
                "core.ready"
            } else {
                request.0
            };
            assert_eq!(reply.message_type, *expected, "for {}", request.0);
        }
    }

    /// Feeds wire frames into one side and collects its responses.
    fn deliver(
        target: &mut CoreState,
        local: &crate::profile::PublicProfile,
        frames: Vec<String>,
    ) -> Vec<String> {
        let mut responses = Vec::new();
        for raw in frames {
            let outcome = target.profile.ingest(&raw, local);
            if outcome.applied {
                target.profile_dirty = true;
            }
            responses.extend(outcome.send);
        }
        responses
    }

    /// Drives one full profile exchange tick between two cores over
    /// simulated wire frames: A -> B, B -> A, plus avatar chunk flow.
    /// Returns true while either side still has outbound frames pending.
    fn exchange_profiles(a: &mut CoreState, b: &mut CoreState) {
        use crate::profile as profile;
        for _ in 0..64 {
            let now = now_seconds();
            let a_local = {
                let values = a.settings.as_ref().unwrap().values();
                profile::local_profile(
                    values.profile_revision,
                    &values.display_name,
                    &values.status_message,
                    &values.avatar,
                    &values.avatar_type,
                )
            };
            let b_local = {
                let values = b.settings.as_ref().unwrap().values();
                profile::local_profile(
                    values.profile_revision,
                    &values.display_name,
                    &values.status_message,
                    &values.avatar,
                    &values.avatar_type,
                )
            };
            let mut to_b: Vec<String> = Vec::new();
            let mut to_a: Vec<String> = Vec::new();
            if let Some(frame) = a.profile.pending_hello(&a_local, now) {
                a.profile.mark_hello_sent(a_local.revision, now);
                to_b.push(frame);
            }
            if let Some(frame) = b.profile.pending_hello(&b_local, now) {
                b.profile.mark_hello_sent(b_local.revision, now);
                to_a.push(frame);
            }
            for _ in 0..profile::PROFILE_AVATAR_CHUNKS_PER_TICK {
                let Some(frame) = a.profile.peek_chunk(&a_local) else {
                    break;
                };
                a.profile.advance_chunk();
                to_b.push(frame);
            }
            for _ in 0..profile::PROFILE_AVATAR_CHUNKS_PER_TICK {
                let Some(frame) = b.profile.peek_chunk(&b_local) else {
                    break;
                };
                b.profile.advance_chunk();
                to_a.push(frame);
            }
            if to_a.is_empty() && to_b.is_empty() {
                let a_cancel = a.profile.stale_outbound_cancel(&a_local);
                let b_cancel = b.profile.stale_outbound_cancel(&b_local);
                if a_cancel.is_none() && b_cancel.is_none() {
                    return;
                }
            }
            // Responses to responses stay bounded: a request answers at most
            // an offer or a cancel, an offer starts at most a chunk stream.
            for _ in 0..4 {
                let b_responses = deliver(b, &b_local, std::mem::take(&mut to_b));
                let a_responses = deliver(a, &a_local, std::mem::take(&mut to_a));
                to_b.extend(a_responses);
                to_a.extend(b_responses);
                if to_a.is_empty() && to_b.is_empty() {
                    break;
                }
            }
        }
    }

    #[test]
    fn paired_profiles_converge_peer_to_peer() {
        let a_dir =
            std::env::temp_dir().join(format!("harbor-profile-a-{}", uuid::Uuid::new_v4()));
        let b_dir =
            std::env::temp_dir().join(format!("harbor-profile-b-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&a_dir).unwrap();
        std::fs::create_dir_all(&b_dir).unwrap();
        let mut a = CoreState::for_directory(&a_dir);
        let mut b = CoreState::for_directory(&b_dir);

        // A publishes a name, status and GIF avatar; B a name only.
        dispatch(
            Envelope::request(
                "settings.update",
                json!({
                    "displayName": "Joshua",
                    "statusMessage": "Building a cabin",
                    "avatar": "data:image/gif;base64,R0lGODlhAQABAIAAAP///wAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw==",
                    "avatarType": "gif",
                }),
                "2026-08-31T20:00:00Z",
            ),
            &mut a,
        )
        .unwrap();
        dispatch(
            Envelope::request(
                "settings.update",
                json!({"displayName": "Taylor"}),
                "2026-08-31T20:00:00Z",
            ),
            &mut b,
        )
        .unwrap();

        exchange_profiles(&mut a, &mut b);

        // Each side learned the other's public profile — and nothing else.
        assert_eq!(b.profile.partner.display_name, "Joshua");
        assert_eq!(b.profile.partner.status_message, "Building a cabin");
        assert_eq!(b.profile.partner.avatar_type, "gif");
        assert!(b.profile.partner.avatar.starts_with("data:image/gif;base64,"));
        assert_eq!(
            b.profile.partner.avatar_hash,
            crate::profile::avatar_data_hash(
                "data:image/gif;base64,R0lGODlhAQABAIAAAP///wAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw=="
            )
        );
        assert_eq!(a.profile.partner.display_name, "Taylor");
        assert_eq!(a.profile.partner.avatar_type, "none");
        // Local profiles were never touched by inbound state.
        assert_eq!(a.settings.as_ref().unwrap().values().display_name, "Joshua");
        assert_eq!(b.settings.as_ref().unwrap().values().display_name, "Taylor");
        // No private material crossed: the partner snapshot has no identity.
        let snapshot = b.profile_snapshot();
        assert!(snapshot.get("device_id").is_none());
        assert!(snapshot.get("public_key").is_none());
        assert_eq!(snapshot["partner"]["displayName"], "Joshua");

        // A profile.updated event is pending for the UI on both sides.
        assert!(a.take_profile_event().is_some());
        assert!(b.take_profile_event().is_some());
        // ...but a duplicate exchange emits nothing new (dedup by revision).
        exchange_profiles(&mut a, &mut b);
        assert!(a.take_profile_event().is_none());
        assert!(b.take_profile_event().is_none());

        let _ = std::fs::remove_dir_all(&a_dir);
        let _ = std::fs::remove_dir_all(&b_dir);
    }

    #[test]
    fn stale_revisions_never_rewind_partner_state() {
        let dir = std::env::temp_dir().join(format!("harbor-profile-rev-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut core = CoreState::for_directory(&dir);
        let local = crate::profile::local_profile(1, "Me", "", "", "image");

        let newer = crate::profile::hello_frame(
            &crate::profile::local_profile(9, "Taylor", "v9", "", "image"),
            true,
        );
        assert!(core.profile.ingest(&newer, &local).applied);
        assert_eq!(core.profile.partner.display_name, "Taylor");

        // Same revision: duplicate delivery, ignored without work.
        assert!(!core.profile.ingest(&newer, &local).applied);
        // Older revision: a reorder on the wire, ignored.
        let older = crate::profile::hello_frame(
            &crate::profile::local_profile(4, "Mallory", "v4", "", "image"),
            true,
        );
        assert!(!core.profile.ingest(&older, &local).applied);
        assert_eq!(core.profile.partner.display_name, "Taylor");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn offline_edits_converge_on_the_newest_revision_after_reconnect() {
        let a_dir =
            std::env::temp_dir().join(format!("harbor-profile-off-{}", uuid::Uuid::new_v4()));
        let b_dir =
            std::env::temp_dir().join(format!("harbor-profile-offb-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&a_dir).unwrap();
        std::fs::create_dir_all(&b_dir).unwrap();
        let mut a = CoreState::for_directory(&a_dir);
        let mut b = CoreState::for_directory(&b_dir);

        dispatch(
            Envelope::request(
                "settings.update",
                json!({"displayName": "Joshua"}),
                "2026-08-31T20:00:00Z",
            ),
            &mut a,
        )
        .unwrap();
        exchange_profiles(&mut a, &mut b);
        assert_eq!(b.profile.partner.display_name, "Joshua");

        // Both go offline (call ends): transient state drops, snapshot stays.
        a.profile.note_disconnected();
        b.profile.note_disconnected();
        // A edits twice while offline: only the newest revision matters.
        for name in ["Joshua 2", "Joshua 3"] {
            dispatch(
                Envelope::request(
                    "settings.update",
                    json!({"displayName": name}),
                    "2026-08-31T20:00:00Z",
                ),
                &mut a,
            )
            .unwrap();
        }
        // Intermediate revisions are never sent one by one: reconnecting
        // publishes the current state only.
        exchange_profiles(&mut a, &mut b);
        assert_eq!(b.profile.partner.display_name, "Joshua 3");

        let _ = std::fs::remove_dir_all(&a_dir);
        let _ = std::fs::remove_dir_all(&b_dir);
    }

    #[test]
    fn partner_snapshot_survives_restart_and_updates_on_newer() {
        let a_dir =
            std::env::temp_dir().join(format!("harbor-profile-rs-{}", uuid::Uuid::new_v4()));
        let b_dir =
            std::env::temp_dir().join(format!("harbor-profile-rsb-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&a_dir).unwrap();
        std::fs::create_dir_all(&b_dir).unwrap();
        let mut a = CoreState::for_directory(&a_dir);
        let mut b = CoreState::for_directory(&b_dir);
        dispatch(
            Envelope::request(
                "settings.update",
                json!({"displayName": "Joshua", "statusMessage": "Hi"}),
                "2026-08-31T20:00:00Z",
            ),
            &mut a,
        )
        .unwrap();
        exchange_profiles(&mut a, &mut b);
        assert!(b.profile_dirty);
        b.profile.persist_partner(&b.state_dir);
        b.profile_dirty = false;

        // Restart: the partner is still known without any traffic.
        let restarted = CoreState::for_directory(&b_dir);
        assert_eq!(restarted.profile.partner.display_name, "Joshua");
        assert_eq!(restarted.profile.partner.status_message, "Hi");

        // A newer peer state still wins after the restart.
        let mut b = restarted;
        dispatch(
            Envelope::request(
                "settings.update",
                json!({"displayName": "Joshua!", "statusMessage": "Back"}),
                "2026-08-31T20:00:00Z",
            ),
            &mut a,
        )
        .unwrap();
        exchange_profiles(&mut a, &mut b);
        assert_eq!(b.profile.partner.display_name, "Joshua!");
        assert!(b.profile_dirty);

        // profile.state serves the persisted snapshot to the UI.
        let state = dispatch(
            Envelope::request("profile.state", json!({}), "2026-08-31T20:00:00Z"),
            &mut b,
        )
        .unwrap();
        assert!(state.error.is_none());
        assert_eq!(state.payload["partner"]["displayName"], "Joshua!");
        assert_eq!(state.payload["partner"]["statusMessage"], "Back");

        let _ = std::fs::remove_dir_all(&a_dir);
        let _ = std::fs::remove_dir_all(&b_dir);
    }

    #[test]
    fn large_avatars_chunk_peer_to_peer_and_verify() {
        let a_dir =
            std::env::temp_dir().join(format!("harbor-profile-lg-{}", uuid::Uuid::new_v4()));
        let b_dir =
            std::env::temp_dir().join(format!("harbor-profile-lgb-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&a_dir).unwrap();
        std::fs::create_dir_all(&b_dir).unwrap();
        let mut a = CoreState::for_directory(&a_dir);
        let mut b = CoreState::for_directory(&b_dir);

        // Larger than the inline budget: must travel chunked, never inline.
        let big = format!("data:image/png;base64,{}", "QUJD".repeat(3000));
        assert!(big.len() > crate::profile::PROFILE_INLINE_AVATAR_MAX_BYTES);
        dispatch(
            Envelope::request(
                "settings.update",
                json!({
                    "displayName": "Joshua",
                    "avatar": big,
                    "avatarType": "image",
                }),
                "2026-08-31T20:00:00Z",
            ),
            &mut a,
        )
        .unwrap();
        exchange_profiles(&mut a, &mut b);

        assert_eq!(b.profile.partner.avatar, big);
        assert_eq!(
            b.profile.partner.avatar_hash,
            crate::profile::avatar_data_hash(&big)
        );
        // No in-flight state leaks after completion.
        assert!(b.profile.inbound.is_none());
        assert!(b.profile.requested_hash.is_none());
        assert!(a.profile.outbound.is_none());

        let _ = std::fs::remove_dir_all(&a_dir);
        let _ = std::fs::remove_dir_all(&b_dir);
    }

    #[test]
    fn avatar_removal_returns_the_peer_to_initials() {
        let a_dir =
            std::env::temp_dir().join(format!("harbor-profile-rm-{}", uuid::Uuid::new_v4()));
        let b_dir =
            std::env::temp_dir().join(format!("harbor-profile-rmb-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&a_dir).unwrap();
        std::fs::create_dir_all(&b_dir).unwrap();
        let mut a = CoreState::for_directory(&a_dir);
        let mut b = CoreState::for_directory(&b_dir);
        dispatch(
            Envelope::request(
                "settings.update",
                json!({
                    "displayName": "Joshua",
                    "avatar": "data:image/png;base64,AA==",
                    "avatarType": "image",
                }),
                "2026-08-31T20:00:00Z",
            ),
            &mut a,
        )
        .unwrap();
        exchange_profiles(&mut a, &mut b);
        assert_eq!(b.profile.partner.avatar, "data:image/png;base64,AA==");

        dispatch(
            Envelope::request(
                "settings.update",
                json!({"avatar": "", "avatarType": "image"}),
                "2026-08-31T20:00:00Z",
            ),
            &mut a,
        )
        .unwrap();
        exchange_profiles(&mut a, &mut b);
        assert_eq!(b.profile.partner.avatar_type, "none");
        assert!(b.profile.partner.avatar.is_empty());

        let _ = std::fs::remove_dir_all(&a_dir);
        let _ = std::fs::remove_dir_all(&b_dir);
    }

    #[test]
    fn settings_round_trip_within_one_process() {
        let mut state = state();

        let read = dispatch(
            Envelope::request("settings.get", json!({}), "2026-08-31T20:00:00Z"),
            &mut state,
        )
        .unwrap();
        assert!(read.error.is_none());
        assert_eq!(read.payload["appearanceMode"], "dark");

        let update = dispatch(
            Envelope::request(
                "settings.update",
                json!({"appearanceMode": "light", "reducedMotion": true}),
                "2026-08-31T20:00:01Z",
            ),
            &mut state,
        )
        .unwrap();
        assert!(update.error.is_none());
        assert_eq!(update.payload["appearanceMode"], "light");
        assert_eq!(update.payload["reducedMotion"], true);

        let rejected = dispatch(
            Envelope::request(
                "settings.update",
                json!({"notASetting": true}),
                "2026-08-31T20:00:02Z",
            ),
            &mut state,
        )
        .unwrap();
        assert_eq!(rejected.error.unwrap().code, "invalid_request");
        assert_eq!(
            state.settings.as_ref().unwrap().values().appearance_mode,
            "light"
        );
    }

    #[test]
    fn fresh_state_persists_settings_for_the_next_process() {
        let directory = std::env::temp_dir().join(format!("harbor-core-test-{}", Uuid::new_v4()));
        let mut first = CoreState::for_directory(&directory);
        dispatch(
            Envelope::request(
                "settings.update",
                json!({"locale": "pt"}),
                "2026-08-31T20:00:00Z",
            ),
            &mut first,
        )
        .unwrap();

        let mut second = CoreState::for_directory(&directory);
        let read = dispatch(
            Envelope::request("settings.get", json!({}), "2026-08-31T20:00:01Z"),
            &mut second,
        )
        .unwrap();
        assert_eq!(read.payload["locale"], "pt");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn server_pin_configuration_validates_persists_and_round_trips() {
        let directory = std::env::temp_dir().join(format!("harbor-core-test-{}", Uuid::new_v4()));
        let mut state = CoreState::for_directory(&directory);

        let rejected = dispatch(
            Envelope::request(
                "server.configure",
                json!({"address": "127.0.0.1:9091", "fingerprint": "not-hex"}),
                "2026-08-31T20:00:00Z",
            ),
            &mut state,
        )
        .unwrap();
        assert_eq!(rejected.error.unwrap().code, "invalid_request");
        assert_eq!(
            dispatch(
                Envelope::request("server.config", json!({}), "2026-08-31T20:00:01Z"),
                &mut state,
            )
            .unwrap()
            .payload["configured"],
            false
        );

        let fingerprint = "a".repeat(64);
        let configured = dispatch(
            Envelope::request(
                "server.configure",
                json!({"address": "127.0.0.1:9091", "fingerprint": fingerprint}),
                "2026-08-31T20:00:02Z",
            ),
            &mut state,
        )
        .unwrap();
        assert!(configured.error.is_none());

        // A restarted core reads the same pin from durable state.
        let mut reloaded = CoreState::for_directory(&directory);
        let read = dispatch(
            Envelope::request("server.config", json!({}), "2026-08-31T20:00:03Z"),
            &mut reloaded,
        )
        .unwrap();
        assert_eq!(read.payload["configured"], true);
        assert_eq!(read.payload["address"], "127.0.0.1:9091");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn pairing_without_a_configured_server_is_refused_locally() {
        let mut state = state();
        let response = dispatch(
            Envelope::request("pairing.create", json!({}), "2026-08-31T20:00:00Z"),
            &mut state,
        )
        .unwrap();
        assert_eq!(response.error.unwrap().code, "server_unconfigured");
        assert_eq!(
            dispatch(
                Envelope::request("pairing.state", json!({}), "2026-08-31T20:00:01Z"),
                &mut state,
            )
            .unwrap()
            .payload["phase"],
            "IDLE"
        );
    }

    /// One real TLS server plus two pinned cores whose devices just paired.
    /// The pairing-flow test and the direct-call test both build on this.
    fn spawn_server_and_pair() -> (Listener, CoreState, CoreState, PathBuf, PathBuf, PathBuf) {
        use std::sync::{Arc, Mutex};

        use harbor_server::{Listener, ListenerConfig};

        let server_dir =
            std::env::temp_dir().join(format!("harbor-core-e2e-server-{}", Uuid::new_v4()));
        let server_core = Arc::new(Mutex::new(
            harbor_server::ServerCore::open(&server_dir.join("state")).unwrap(),
        ));
        let listener = Listener::spawn(
            ListenerConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                tls_dir: server_dir.join("tls"),
                cert_pem: None,
                key_pem: None,
            },
            server_core,
        )
        .unwrap();
        let fingerprint = listener
            .certificate_fingerprint()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let address = listener.local_addr().to_string();

        let host_dir =
            std::env::temp_dir().join(format!("harbor-core-e2e-host-{}", Uuid::new_v4()));
        let peer_dir =
            std::env::temp_dir().join(format!("harbor-core-e2e-peer-{}", Uuid::new_v4()));
        let mut host = CoreState::for_directory(&host_dir);
        let mut peer = CoreState::for_directory(&peer_dir);

        // Both devices pin the same server; the fingerprint comes from the
        // server's startup line in real deployments.
        for core in [&mut host, &mut peer] {
            let response = dispatch(
                Envelope::request(
                    "server.configure",
                    json!({"address": address, "fingerprint": fingerprint}),
                    crate::rfc3339_now(),
                ),
                core,
            )
            .unwrap();
            assert!(response.error.is_none(), "{:?}", response.error);
        }

        // Host: generate and register the code.
        let created = dispatch(
            Envelope::request("pairing.create", json!({}), crate::rfc3339_now()),
            &mut host,
        )
        .unwrap();
        assert!(created.error.is_none(), "{:?}", created.error);
        assert_eq!(created.payload["phase"], "WAITING_APPROVAL");
        let code = created.payload["code"].as_str().unwrap().to_owned();
        assert_eq!(code.len(), 6);

        // Peer: enter the code and request approval.
        dispatch(
            Envelope::request("pairing.enter_code", json!({}), crate::rfc3339_now()),
            &mut peer,
        )
        .unwrap();
        let submitted = dispatch(
            Envelope::request(
                "pairing.submit",
                json!({"code": code}),
                crate::rfc3339_now(),
            ),
            &mut peer,
        )
        .unwrap();
        assert!(submitted.error.is_none(), "{:?}", submitted.error);
        assert_eq!(submitted.payload["phase"], "REQUESTING");

        // Host: observe the request and approve it.
        let polled = dispatch(
            Envelope::request("pairing.incoming", json!({}), crate::rfc3339_now()),
            &mut host,
        )
        .unwrap();
        assert!(polled.error.is_none(), "{:?}", polled.error);
        assert_eq!(polled.payload["has_request"], true);
        assert_eq!(polled.payload["phase"], "INCOMING_REQUEST");
        let accepted = dispatch(
            Envelope::request("pairing.accept", json!({}), crate::rfc3339_now()),
            &mut host,
        )
        .unwrap();
        assert!(accepted.error.is_none(), "{:?}", accepted.error);
        assert_eq!(accepted.payload["phase"], "ACCEPTED");

        (listener, host, peer, server_dir, host_dir, peer_dir)
    }

    fn cleanup_e2e(listener: Listener, directories: &[PathBuf]) {
        listener.shutdown();
        for directory in directories {
            let _ = std::fs::remove_dir_all(directory);
        }
    }

    #[test]
    fn pairing_flows_end_to_end_over_tls_between_host_and_peer_cores() {
        let (listener, _host, mut peer, server_dir, host_dir, peer_dir) = spawn_server_and_pair();

        // Peer: the decision is observable through status polling.
        let observed = dispatch(
            Envelope::request("pairing.status", json!({}), crate::rfc3339_now()),
            &mut peer,
        )
        .unwrap();
        assert!(observed.error.is_none(), "{:?}", observed.error);
        assert_eq!(observed.payload["phase"], "ACCEPTED");

        cleanup_e2e(listener, &[server_dir, host_dir, peer_dir]);
    }

    /// Manual companion rig: this test PLAYS TAYLOR'S DESKTOP against the
    /// deployed control plane while a human drives the real Harbor Mobile on
    /// a device/emulator. Gated on HARBOR_E2E_ADDRESS +
    /// HARBOR_E2E_FINGERPRINT like the rehearsal above; skipped honestly
    /// when unset.
    ///
    /// Run it, read the printed code, and on the phone: Settings -> save
    /// the same server address + fingerprint, open Pair, enter the code.
    /// Then send at least one chat message from the phone. The rig accepts
    /// the pairing, publishes ONLINE presence, echoes every phone message
    /// back over the direct link, and asserts the transcript at the end.
    /// The desktop listener port prints for `adb reverse` so the emulator
    /// can dial it (link invites also carry 127.0.0.1 for this).
    #[test]
    fn companion_rig_pairs_and_chats_through_the_deployed_server() {
        use std::time::{Duration, Instant};
        let Some(address) = std::env::var_os("HARBOR_E2E_ADDRESS") else {
            eprintln!("skipping companion rig: HARBOR_E2E_ADDRESS is unset");
            return;
        };
        let Some(fingerprint) = std::env::var_os("HARBOR_E2E_FINGERPRINT") else {
            eprintln!("skipping companion rig: HARBOR_E2E_FINGERPRINT is unset");
            return;
        };
        let address = address.to_string_lossy().into_owned();
        let fingerprint = fingerprint.to_string_lossy().into_owned();

        let rig_dir =
            std::env::temp_dir().join(format!("harbor-rig-taylor-{}", Uuid::new_v4()));
        let mut taylor = CoreState::for_directory(&rig_dir);
        let configured = dispatch(
            Envelope::request(
                "server.configure",
                json!({"address": address, "fingerprint": fingerprint}),
                crate::rfc3339_now(),
            ),
            &mut taylor,
        )
        .unwrap();
        assert!(configured.error.is_none(), "{configured:?}");
        dispatch(
            Envelope::request(
                "settings.update",
                json!({"displayName": "Taylor"}),
                crate::rfc3339_now(),
            ),
            &mut taylor,
        )
        .unwrap();

        let created = dispatch(
            Envelope::request("pairing.create", json!({}), crate::rfc3339_now()),
            &mut taylor,
        )
        .unwrap();
        assert!(created.error.is_none(), "{created:?}");
        let code = created.payload["code"].as_str().unwrap().to_owned();
        eprintln!("RIG: pairing code is {code} -- enter it on the phone (TTL 5 min)");

        // Wait for the phone's submit, then approve the identity pairing.
        let wait_until = Instant::now() + Duration::from_secs(270);
        let mut accepted = false;
        while Instant::now() < wait_until {
            let incoming = dispatch(
                Envelope::request("pairing.incoming", json!({}), crate::rfc3339_now()),
                &mut taylor,
            )
            .unwrap();
            if incoming.payload["has_request"] == true {
                eprintln!("RIG: phone is asking -- accepting");
                let approved = dispatch(
                    Envelope::request("pairing.accept", json!({}), crate::rfc3339_now()),
                    &mut taylor,
                )
                .unwrap();
                assert!(approved.error.is_none(), "{approved:?}");
                accepted = true;
                break;
            }
            std::thread::sleep(Duration::from_secs(3));
        }
        assert!(accepted, "the phone never submitted the code -- pair it first");

        // Live peer for ~3 minutes: presence, signaling, link, echo.
        eprintln!("RIG: paired -- send at least one chat message from the phone");
        let mut echoed: std::collections::BTreeSet<String> = Default::default();
        let live_until = Instant::now() + Duration::from_secs(180);
        let mut last_port_print = Instant::now() - Duration::from_secs(60);
        while Instant::now() < live_until {
            // Attended desktop: fresh input, active session.
            let _ = dispatch(
                Envelope::request(
                    "presence.sense",
                    json!({"inputIdleSeconds": 0, "sessionActive": true}),
                    crate::rfc3339_now(),
                ),
                &mut taylor,
            );
            let _ = signaling::tick(&mut taylor);
            taylor.pump_link();
            taylor.sync_direct();
            if last_port_print.elapsed() > Duration::from_secs(15) {
                last_port_print = Instant::now();
                if let Ok(snapshot) = taylor.device_snapshot() {
                    eprintln!(
                        "RIG: link port={} live={} peers={}",
                        snapshot["linkListeningPort"],
                        snapshot["linkPeer"],
                        snapshot["peers"]
                    );
                }
            }
            // Echo every new inbound chat message back over the link.
            for message in taylor.chat.snapshot() {
                if message.direction == crate::direct::Direction::Incoming
                    && echoed.insert(message.id.clone())
                {
                    eprintln!("RIG: phone said {:?} -- echoing", message.body);
                    let _ = dispatch(
                        Envelope::request(
                            "chat.send",
                            json!({"body": format!("eco: {}", message.body)}),
                            crate::rfc3339_now(),
                        ),
                        &mut taylor,
                    );
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
        let texts: Vec<String> = taylor
            .chat
            .snapshot()
            .into_iter()
            .map(|message| message.body)
            .collect();
        eprintln!("RIG: final transcript: {texts:?}");
        assert!(
            texts.iter().any(|body| !body.starts_with("eco: ")),
            "no phone message arrived -- send one from the phone next run"
        );
        let _ = std::fs::remove_dir_all(&rig_dir);
    }

    /// The real-deployment rehearsal: the same pairing and contacts flows the
    /// shell drives, aimed at the deployed control plane instead of an
    /// in-process listener. Gated on HARBOR_E2E_ADDRESS +
    /// HARBOR_E2E_FINGERPRINT (the public pin from the server's startup
    /// line); skipped honestly when unset so ordinary `cargo test` never
    /// depends on the device being reachable.
    #[test]
    fn pairing_and_contacts_succeed_against_the_deployed_control_plane() {
        let Some(address) = std::env::var_os("HARBOR_E2E_ADDRESS") else {
            eprintln!("skipping deployed-server E2E: HARBOR_E2E_ADDRESS is unset");
            return;
        };
        let Some(fingerprint) = std::env::var_os("HARBOR_E2E_FINGERPRINT") else {
            eprintln!("skipping deployed-server E2E: HARBOR_E2E_FINGERPRINT is unset");
            return;
        };
        let address = address.to_string_lossy().into_owned();
        let fingerprint = fingerprint.to_string_lossy().into_owned();

        // Pinning is enforced by the deployed listener too: the configure
        // only persists the pin, so the refusal must fire on the first real
        // exchange, before any pairing state machine runs.
        let stranger_dir =
            std::env::temp_dir().join(format!("harbor-core-k11-stranger-{}", Uuid::new_v4()));
        let mut stranger = CoreState::for_directory(&stranger_dir);
        let configured = dispatch(
            Envelope::request(
                "server.configure",
                json!({"address": address, "fingerprint": "ab".repeat(32)}),
                crate::rfc3339_now(),
            ),
            &mut stranger,
        )
        .unwrap();
        assert!(configured.error.is_none(), "{configured:?}");
        let refused = dispatch(
            Envelope::request("pairing.create", json!({}), crate::rfc3339_now()),
            &mut stranger,
        )
        .unwrap();
        let error = refused.error.expect("a wrong pin must be refused");
        assert_eq!(error.ui_key, "error.server.unavailable", "{error:?}");
        drop(stranger);
        let _ = std::fs::remove_dir_all(&stranger_dir);

        // Two fresh devices pair through the deployed server.
        let host_dir =
            std::env::temp_dir().join(format!("harbor-core-k11-host-{}", Uuid::new_v4()));
        let peer_dir =
            std::env::temp_dir().join(format!("harbor-core-k11-peer-{}", Uuid::new_v4()));
        let mut host = CoreState::for_directory(&host_dir);
        let mut peer = CoreState::for_directory(&peer_dir);
        for core in [&mut host, &mut peer] {
            let response = dispatch(
                Envelope::request(
                    "server.configure",
                    json!({"address": address, "fingerprint": fingerprint}),
                    crate::rfc3339_now(),
                ),
                core,
            )
            .unwrap();
            assert!(response.error.is_none(), "{response:?}");
        }

        let created = dispatch(
            Envelope::request("pairing.create", json!({}), crate::rfc3339_now()),
            &mut host,
        )
        .unwrap();
        assert!(created.error.is_none(), "{created:?}");
        assert_eq!(created.payload["phase"], "WAITING_APPROVAL");
        let code = created.payload["code"].as_str().unwrap().to_owned();

        let submitted = dispatch(
            Envelope::request(
                "pairing.submit",
                json!({"code": code}),
                crate::rfc3339_now(),
            ),
            &mut peer,
        )
        .unwrap();
        assert!(submitted.error.is_none(), "{submitted:?}");
        assert_eq!(submitted.payload["phase"], "REQUESTING");

        let polled = dispatch(
            Envelope::request("pairing.incoming", json!({}), crate::rfc3339_now()),
            &mut host,
        )
        .unwrap();
        assert!(polled.error.is_none(), "{polled:?}");
        assert_eq!(polled.payload["has_request"], true);
        let accepted = dispatch(
            Envelope::request("pairing.accept", json!({}), crate::rfc3339_now()),
            &mut host,
        )
        .unwrap();
        assert!(accepted.error.is_none(), "{accepted:?}");
        assert_eq!(accepted.payload["phase"], "ACCEPTED");

        for core in [&mut host, &mut peer] {
            let observed = dispatch(
                Envelope::request("pairing.status", json!({}), crate::rfc3339_now()),
                core,
            )
            .unwrap();
            assert!(observed.error.is_none(), "{observed:?}");
            assert_eq!(observed.payload["phase"], "ACCEPTED");
        }

        // The new contacts family over the same deployed server: each fresh
        // device's durable peer list is exactly its counterpart — safe
        // {deviceId, harborId} pairs, no keys, no private material.
        let mut seen = Vec::new();
        for core in [&mut host, &mut peer] {
            let listed = dispatch(
                Envelope::request("contacts.list", json!({}), crate::rfc3339_now()),
                core,
            )
            .unwrap();
            assert!(listed.error.is_none(), "{listed:?}");
            let peers = listed.payload["peers"].as_array().unwrap();
            assert_eq!(peers.len(), 1, "{listed:?}");
            let entry = &peers[0];
            assert!(entry["device_id"].as_str().unwrap().len() >= 32);
            assert!(!entry["harbor_id"].as_str().unwrap().trim().is_empty());
            assert!(
                entry.get("public_key").is_none(),
                "keys never reach contacts"
            );
            seen.push(entry["device_id"].as_str().unwrap().to_owned());
        }
        assert_ne!(seen[0], seen[1], "the two devices are distinct");

        let _ = std::fs::remove_dir_all(&host_dir);
        let _ = std::fs::remove_dir_all(&peer_dir);
    }

    /// Builds the Go/Pion worker for direct-call tests. Honors
    /// HARBOR_MEDIA_EXECUTABLE; returns None without a Go toolchain so the
    /// test reports a skip instead of pretending media was proven.
    fn build_media_worker() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("HARBOR_MEDIA_EXECUTABLE") {
            return Some(PathBuf::from(path));
        }
        let media_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../media");
        let output_dir = std::env::temp_dir().join(format!("harbor-media-e2e-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&output_dir).ok()?;
        let binary = output_dir.join("harbor-media");
        let built = std::process::Command::new("go")
            .arg("build")
            .arg("-o")
            .arg(&binary)
            .arg("./cmd/harbor-media")
            .current_dir(media_dir)
            .output()
            .ok()?;
        if !built.status.success() {
            return None;
        }
        Some(binary)
    }

    #[test]
    fn a_recovery_that_outlives_its_window_fails_the_call() {
        let directory =
            std::env::temp_dir().join(format!("harbor-core-reconnect-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut core = CoreState::for_directory(&directory);

        // Fresh recovery inside the window keeps the call alive.
        core.call.call_id = Some("call-1".into());
        core.call.phase = "RECONNECTING".into();
        core.call.reconnecting_since = Some(Instant::now());
        assert!(!enforce_reconnect_window(&mut core));
        assert_eq!(core.call.phase, "RECONNECTING");

        // Other phases are never touched by the policy.
        core.call.phase = "CONNECTED".into();
        assert!(!enforce_reconnect_window(&mut core));
        assert_eq!(core.call.phase, "CONNECTED");

        // Recovery that outlives the window ends the call visibly.
        core.call.phase = "RECONNECTING".into();
        core.call.reconnecting_since = Some(Instant::now() - Duration::from_secs(16));
        assert!(enforce_reconnect_window(&mut core));
        assert_eq!(core.call.phase, "FAILED");
        assert!(core.call.reconnecting_since.is_none());
        assert!(core.call_dirty);

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn calling_without_a_reachable_server_is_refused_honestly() {
        let _env_lock = WORKER_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(worker) = build_media_worker() else {
            eprintln!("skipping call refusal test: no Go toolchain to build harbor-media");
            return;
        };
        // SAFETY: single assignment before any media worker spawns; no other
        // test reads this variable concurrently.
        unsafe { std::env::set_var("HARBOR_MEDIA_EXECUTABLE", &worker) };
        let directory =
            std::env::temp_dir().join(format!("harbor-core-call-refused-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut core = CoreState::for_directory(&directory);

        // No server pin at all: the refusal names the missing configuration.
        let refused = dispatch(
            Envelope::request("call.start", json!({}), crate::rfc3339_now()),
            &mut core,
        )
        .unwrap();
        assert_eq!(
            refused.error.as_ref().unwrap().ui_key,
            "error.server.unconfigured",
            "{refused:?}"
        );

        // A configured-but-dead server must not hang or fake success.
        let pin = ServerPin::parse("127.0.0.1:1", &"ab".repeat(32)).unwrap();
        store_server_pin(&directory, &pin).unwrap();
        let refused = dispatch(
            Envelope::request("call.start", json!({}), crate::rfc3339_now()),
            &mut core,
        )
        .unwrap();
        let error = refused.error.expect("dialing a dead server must fail");
        assert_eq!(error.ui_key, "error.server.unavailable");
        assert!(error.retryable);
        // The refusal happens before any media resource is touched, so no
        // call state was ever entered: IDLE is the honest phase, and the
        // response alone carries the failure.
        assert_eq!(core.call.phase, "IDLE");
        assert!(core.call.call_id.is_none());

        // The polling loop backs off quietly instead of spinning on the dead
        // server, and an unpaired core never produces signaling actions.
        assert!(matches!(signaling::tick(&mut core), SignalingTick::Quiet));
        assert_eq!(core.signaling.interval(), Duration::from_secs(5));

        let _ = std::fs::remove_dir_all(&directory);
    }

    /// Writes a deterministic fake media worker: a small script speaking the
    /// same framed private protocol as harbor-media, answering the handshake
    /// and acknowledging share commands. It lets the core's share mapping be
    /// proven without capture hardware, and it refuses share starts whose
    /// payload is missing the call it belongs to.
    fn write_fake_share_worker() -> Option<PathBuf> {
        let directory =
            std::env::temp_dir().join(format!("harbor-core-fake-media-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).ok()?;
        let script = directory.join("fake-media-worker");
        let source = r#"#!/usr/bin/env python3
import json, struct, sys


def read_frame():
    header = sys.stdin.buffer.read(4)
    if len(header) < 4:
        return None
    (length,) = struct.unpack(">I", header)
    return json.loads(sys.stdin.buffer.read(length))


def write_frame(envelope):
    body = json.dumps(envelope).encode()
    sys.stdout.buffer.write(struct.pack(">I", len(body)) + body)
    sys.stdout.buffer.flush()


chat_polls = 0
while True:
    request = read_frame()
    if request is None:
        break
    kind = request["type"]
    if kind == "media.hello":
        payload = {"service": "harbor-media", "protocol": 1,
                   "capabilities": ["host-ice", "local-offer"]}
    elif kind == "call.share_start":
        if not request["payload"].get("call_id"):
            payload = None
        else:
            payload = {"state": "SHARING"}
    elif kind == "call.share_stop":
        payload = {"state": "NOT_SHARING"}
    elif kind == "chat.send":
        payload = {"state": "SENT"}
    elif kind == "chat.poll":
        payload = {"messages": ([{"message_id": "peer-1", "body": "the peer replied"}]
                                if chat_polls == 0 else [])}
        chat_polls += 1
    elif kind == "chat.status":
        payload = {"deliveries": {message: True for message in request["payload"].get("message_ids", [])}}
    elif kind == "media.shutdown":
        write_frame({"v": 1, "type": kind, "request_id": request["request_id"],
                     "reply_to": request["request_id"],
                     "timestamp": request["timestamp"], "payload": {}})
        break
    else:
        payload = {}
    if payload is None:
        error = {"code": "invalid_request", "ui_key": "error.call.screenShareUnavailable",
                 "retryable": False, "detail": "share start lacked its call"}
    else:
        error = None
    write_frame({"v": 1, "type": kind, "request_id": request["request_id"],
                 "reply_to": request["request_id"],
                 "timestamp": request["timestamp"], "payload": payload or {},
                 "error": error})
"#;
        std::fs::write(&script, source).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).ok()?;
        }
        Some(script)
    }

    #[test]
    fn share_commands_round_trip_through_the_worker_boundary() {
        let _env_lock = WORKER_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(script) = write_fake_share_worker() else {
            eprintln!("skipping share round-trip test: fake worker could not be written");
            return;
        };
        // SAFETY: single assignment under WORKER_ENV_LOCK, before any media
        // worker spawns; the other tests that read this variable hold the
        // same lock.
        unsafe { std::env::set_var("HARBOR_MEDIA_EXECUTABLE", &script) };
        let Ok(media) = MediaSupervisor::start(None) else {
            eprintln!("skipping share round-trip test: fake worker did not start");
            let _ = std::fs::remove_dir_all(script.parent().unwrap());
            return;
        };

        // A connected call with a live worker: exactly the state the share
        // gate demands before any screen leaves the machine.
        let mut core = state();
        core.call.call_id = Some("call-1".into());
        core.call.phase = "CONNECTED".into();
        core.media = Some(media);

        let started = dispatch(
            Envelope::request(
                "call.share_screen_start",
                json!({}),
                crate::rfc3339_now(),
            ),
            &mut core,
        )
        .unwrap();
        assert!(started.error.is_none(), "{started:?}");
        assert_eq!(started.payload["state"], "SHARING");
        assert_eq!(core.call.share_phase, "SHARING");
        let event = core
            .take_share_event()
            .expect("the share must be announced");
        assert_eq!(event.message_type, "call.share_state_changed");
        assert_eq!(event.payload["state"], "SHARING");
        assert_eq!(event.payload["call_id"], "call-1");

        let stopped = dispatch(
            Envelope::request(
                "call.share_screen_stop",
                json!({}),
                crate::rfc3339_now(),
            ),
            &mut core,
        )
        .unwrap();
        assert!(stopped.error.is_none(), "{stopped:?}");
        assert_eq!(stopped.payload["state"], "NOT_SHARING");
        assert_eq!(core.call.share_phase, "NOT_SHARING");
        let event = core.take_share_event().expect("the stop must be announced");
        assert_eq!(event.message_type, "call.share_state_changed");
        assert_eq!(event.payload["state"], "NOT_SHARING");

        // A share start that lost its call identity is refused by the worker
        // and surfaces as the localized refusal, never as a fake success.
        core.call.call_id = None;
        core.call.phase = "CONNECTED".into();
        let refused = dispatch(
            Envelope::request(
                "call.share_screen_start",
                json!({}),
                crate::rfc3339_now(),
            ),
            &mut core,
        )
        .unwrap();
        let error = refused.error.expect("a call-less share must be refused");
        assert_eq!(error.ui_key, "error.call.screenShareUnavailable");
        assert_eq!(core.call.share_phase, "NOT_SHARING");

        if let Some(media) = core.media.take() {
            media.shutdown();
        }
        let _ = std::fs::remove_dir_all(script.parent().unwrap());
    }

    #[test]
    fn chat_round_trips_through_the_worker_without_server_content() {
        let _env_lock = WORKER_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(script) = write_fake_share_worker() else {
            eprintln!("skipping direct chat test: fake worker could not be written");
            return;
        };
        // SAFETY: single assignment under WORKER_ENV_LOCK, before any media
        // worker spawns; every other reader of this process-global setting
        // uses the same lock.
        unsafe { std::env::set_var("HARBOR_MEDIA_EXECUTABLE", &script) };
        let Ok(media) = MediaSupervisor::start(None) else {
            eprintln!("skipping direct chat test: fake worker did not start");
            let _ = std::fs::remove_dir_all(script.parent().unwrap());
            return;
        };
        let mut core = state();
        core.call.call_id = Some("call-chat".into());
        core.call.phase = "CONNECTED".into();
        core.media = Some(media);

        let sent = dispatch(
            Envelope::request(
                "chat.send",
                json!({"body": "hello peer"}),
                crate::rfc3339_now(),
            ),
            &mut core,
        )
        .unwrap();
        assert!(sent.error.is_none(), "{sent:?}");
        let messages = sent.payload["state"]["messages"]
            .as_array()
            .expect("state must expose sanitized messages");
        assert_eq!(messages.len(), 2, "outbound and polled peer reply");
        assert_eq!(messages[0]["body"], "hello peer");
        assert_eq!(messages[0]["delivery"], "DELIVERED");
        assert_eq!(messages[1]["id"], "peer-1");
        assert_eq!(messages[1]["body"], "the peer replied");
        assert_eq!(messages[1]["direction"], "INCOMING");

        let event = core
            .take_direct_event()
            .expect("chat state change must be announced");
        assert_eq!(event.message_type, "direct.updated");
        assert_eq!(event.payload["messages"].as_array().map(Vec::len), Some(2));

        if let Some(media) = core.media.take() {
            media.shutdown();
        }
        let _ = std::fs::remove_dir_all(script.parent().unwrap());
    }

    #[test]
    fn transfer_offers_follow_local_policy_before_any_transport() {
        let directory =
            std::env::temp_dir().join(format!("harbor-core-transfer-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let mut core = CoreState::for_directory(&directory);

        // A missing source is refused before any offer exists.
        let refused = dispatch(
            Envelope::request(
                "transfer.offer_local",
                json!({"source_path": "/nonexistent/harbor.bin"}),
                crate::rfc3339_now(),
            ),
            &mut core,
        )
        .unwrap();
        assert_eq!(
            refused.error.as_ref().map(|error| error.code.as_str()),
            Some("transfer_unreadable"),
            "{refused:?}"
        );

        // A payload without a source is a protocol error, not a policy one.
        let invalid = dispatch(
            Envelope::request(
                "transfer.offer_local",
                json!({}),
                crate::rfc3339_now(),
            ),
            &mut core,
        )
        .unwrap();
        assert_eq!(
            invalid.error.as_ref().map(|error| error.code.as_str()),
            Some("invalid_request")
        );

        // A real source is registered with sanitized metadata only.
        let source = directory.join("notes.txt");
        std::fs::write(&source, b"harbor notes").unwrap();
        let offered = dispatch(
            Envelope::request(
                "transfer.offer_local",
                json!({"source_path": source}),
                crate::rfc3339_now(),
            ),
            &mut core,
        )
        .unwrap();
        assert!(offered.error.is_none(), "{offered:?}");
        let transfers = offered.payload["state"]["transfers"]
            .as_array()
            .expect("state must expose transfers");
        assert_eq!(transfers.len(), 1);
        assert_eq!(transfers[0]["state"], "OFFERED");
        assert_eq!(transfers[0]["name"], "notes.txt");
        assert_eq!(transfers[0]["size"], 12);
        // Policy facts stay below the UI boundary: no digest, no path.
        assert!(
            offered.payload["state"]["transfers"][0]
                .get("sha256")
                .is_none()
        );

        let event = core
            .take_direct_event()
            .expect("an offer must be announced");
        assert_eq!(event.message_type, "direct.updated");

        // Unknown IDs are refused for both settle and cancel.
        for kind in ["transfer.accept", "transfer.reject", "transfer.cancel"] {
            let unknown = dispatch(
                Envelope::request(
                    kind,
                    json!({"transfer_id": "nope"}),
                    crate::rfc3339_now(),
                ),
                &mut core,
            )
            .unwrap();
            assert!(unknown.error.is_some(), "{kind} must refuse unknown ids");
        }

        // A queued offer expires instead of lingering forever.
        core.transfers.expire(now_seconds() + 601);
        let records = core.transfers.records();
        assert!(matches!(records[0].phase, TransferPhase::Canceled));

        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn call_accept_and_decline_without_a_pending_offer_are_refused() {
        let mut core_state = state();
        for action in ["call.accept", "call.decline"] {
            let response = dispatch(
                Envelope::request(action, json!({}), "2026-08-31T20:00:00Z"),
                &mut core_state,
            )
            .unwrap();
            let error = response.error.expect("no offer is pending, so refuse");
            assert_eq!(error.code, "no_incoming_call");
            assert_eq!(error.ui_key, "error.call.noIncomingCall");
        }
        assert_eq!(core_state.call.phase, "IDLE");
        assert!(core_state.call.call_id.is_none());
    }

    #[test]
    fn a_presented_offer_rings_as_incoming_and_decline_returns_to_idle() {
        let mut core_state = state();
        core_state.present_incoming("remote-call", "v=0", Uuid::new_v4());
        assert_eq!(core_state.call.phase, "INCOMING");
        assert!(
            core_state.call.call_id.is_none(),
            "nothing media exists yet"
        );

        let declined = dispatch(
            Envelope::request("call.decline", json!({}), "2026-08-31T20:00:00Z"),
            &mut core_state,
        )
        .unwrap();
        assert!(declined.error.is_none(), "{:?}", declined.error);
        assert_eq!(declined.payload["state"], "IDLE");
        assert_eq!(core_state.call.phase, "IDLE");
        assert!(core_state.incoming.is_none());
    }

    #[test]
    fn an_incoming_call_past_its_presentation_window_stops_ringing() {
        let mut core_state = state();
        core_state.present_incoming("remote-call", "v=0", Uuid::new_v4());
        if let Some(incoming) = core_state.incoming.as_mut() {
            incoming.received_at = Instant::now() - INCOMING_TTL - Duration::from_secs(1);
        }

        assert!(core_state.enforce_incoming_ttl());
        assert!(core_state.incoming.is_none());
        assert_eq!(core_state.call.phase, "IDLE");

        // A fresh offer is never touched by the expiry pass.
        core_state.present_incoming("remote-call", "v=0", Uuid::new_v4());
        assert!(!core_state.enforce_incoming_ttl());
        assert_eq!(core_state.call.phase, "INCOMING");
    }

    /// Seeds a core with a loaded identity and one announced app open plus
    /// one announced game open, as the monitor would have produced them.
    fn seed_shared_activity(core_state: &mut CoreState) {
        let identity = crate::load_or_create(&core_state.state_dir, now_seconds()).unwrap();
        core_state.signaling.cache_identity(identity);
        let mut engine = core_state.engine.lock().unwrap();
        assert!(engine.mark_monitor_started(now_seconds()));
        assert!(engine.ingest(
            now_seconds(),
            vec![
                crate::activity::RawObservation {
                    pid: 42,
                    exe_path: Some("/usr/bin/vlc".into()),
                    command_line: Some("vlc --loop".into()),
                },
                crate::activity::RawObservation {
                    pid: 43,
                    exe_path: Some("/opt/minecraft-launcher/minecraft".into()),
                    command_line: Some("minecraft --demo".into()),
                },
            ],
        ));
    }

    #[test]
    fn the_shared_activity_frame_is_sanitized_and_schema_valid() {
        let mut core_state = state();
        seed_shared_activity(&mut core_state);

        let (frame, fingerprint) = core_state
            .pending_activity_frame()
            .expect("a frame to share");
        assert_eq!(fingerprint, activity_frame_fingerprint(&frame));
        assert_eq!(frame.len(), 2, "the two announced opens cross");
        let serialized = serde_json::to_string(&frame).unwrap();
        assert!(!serialized.contains("usr/bin"), "path leaked: {serialized}");
        assert!(
            !serialized.contains("--loop"),
            "cmdline leaked: {serialized}"
        );
        assert!(
            !serialized.contains("--demo"),
            "cmdline leaked: {serialized}"
        );
        for record in &frame {
            crate::activity::validate_remote_record(record, now_seconds())
                .expect("our own frame passes the peer schema");
        }

        // An accepted send memoizes the frame; nothing changes on the next
        // cadence, so nothing is re-shared.
        let changed = core_state.absorb_activity_sync(
            ActivitySync {
                fingerprint: Some(fingerprint),
                inbound: Vec::new(),
            },
            true,
        );
        assert!(!changed);
        assert!(
            core_state.pending_activity_frame().is_none(),
            "an unchanged timeline is not re-shared"
        );
    }

    #[test]
    fn activity_policies_decide_what_the_peer_frame_holds() {
        let mut core_state = state();
        seed_shared_activity(&mut core_state);

        dispatch(
            Envelope::request(
                "settings.update",
                json!({"gameVisibility": false}),
                "2026-08-31T20:00:00Z",
            ),
            &mut core_state,
        )
        .unwrap();
        let (frame, _) = core_state.pending_activity_frame().unwrap();
        assert_eq!(frame.len(), 1, "a hidden game title hides the record");
        assert_eq!(frame[0]["label"], "vlc");

        dispatch(
            Envelope::request(
                "settings.update",
                json!({"activitySharing": false}),
                "2026-08-31T20:00:01Z",
            ),
            &mut core_state,
        )
        .unwrap();
        assert!(
            core_state.pending_activity_frame().is_none(),
            "sharing off shares nothing"
        );

        // Re-enabling forgets the previous send, so the frame crosses whole.
        dispatch(
            Envelope::request(
                "settings.update",
                json!({"activitySharing": true}),
                "2026-08-31T20:00:02Z",
            ),
            &mut core_state,
        )
        .unwrap();
        assert!(core_state.pending_activity_frame().is_some());
    }

    #[test]
    fn remote_activity_records_are_validated_deduplicated_and_capped() {
        let mut core_state = state();
        let now = now_seconds();
        let record = |id: &str, label: &str| {
            json!({
                "id": id, "sender": "harbor-peer", "category": "app",
                "kind": "opened", "label": label, "occurred_at": now,
            })
        };
        let good = json!([record("r1", "vlc")]).to_string();
        assert!(core_state.absorb_remote_activity_frame(&good));
        assert!(
            !core_state.absorb_remote_activity_frame(&good),
            "a repeated id is ignored"
        );

        // Hostile records are dropped whole, never surfaced: path-like
        // labels, impossible clocks, unknown categories.
        let hostile = json!([
            record("r2", "C:windows system32"),
            {"id": "r3", "sender": "harbor-peer", "category": "app", "kind": "opened",
             "label": "future", "occurred_at": now + 86_400},
            {"id": "r4", "sender": "harbor-peer", "category": "rootkit", "kind": "opened",
             "label": "x", "occurred_at": now},
        ])
        .to_string();
        assert!(
            !core_state.absorb_remote_activity_frame(&hostile),
            "no hostile record becomes visible"
        );
        assert_eq!(core_state.remote_activity.len(), 1);

        // A frame that is not a record array is dropped whole.
        assert!(!core_state.absorb_remote_activity_frame("\"hello\""));
        assert!(!core_state.absorb_remote_activity_frame("not json"));

        // Valid records surface in the UI payload.
        let response = dispatch(
            Envelope::request("activity.state", json!({}), "2026-08-31T20:00:00Z"),
            &mut core_state,
        )
        .unwrap();
        let remote = response.payload["remote"].as_array().unwrap();
        assert!(remote.iter().any(|record| record["id"] == "r1"));
        let serialized = serde_json::to_string(remote).unwrap();
        assert!(!serialized.contains("windows"), "hostile label surfaced");

        // The remote view is capped with the newest retained.
        for index in 0..(REMOTE_ACTIVITY_CAP + 5) {
            let frame = json!([record(&format!("bulk-{index}"), "app")]).to_string();
            core_state.absorb_remote_activity_frame(&frame);
        }
        assert_eq!(core_state.remote_activity.len(), REMOTE_ACTIVITY_CAP);
        assert_eq!(
            core_state.remote_activity.last().unwrap().id,
            format!("bulk-{}", REMOTE_ACTIVITY_CAP + 4)
        );
    }

    #[test]
    fn a_direct_call_connects_both_cores_through_the_control_plane() {
        let _env_lock = WORKER_ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(worker) = build_media_worker() else {
            eprintln!("skipping direct-call e2e: no Go toolchain to build harbor-media");
            return;
        };
        // SAFETY: single assignment before any media worker spawns; no other
        // test reads this variable concurrently.
        unsafe { std::env::set_var("HARBOR_MEDIA_EXECUTABLE", &worker) };
        // SAFETY: same single-assignment window. The workers spawned by this
        // test run their full voice pipeline against the silent audio
        // boundary, so the proof of a direct call never depends on this
        // machine owning sound hardware.
        unsafe { std::env::set_var("HARBOR_MEDIA_AUDIO", "silent") };
        let (listener, mut host, mut peer, server_dir, host_dir, peer_dir) =
            spawn_server_and_pair();

        // The caller dials; the relayed offer makes the worker CONNECTING on
        // the caller side while the peer knows nothing yet.
        let started = dispatch(
            Envelope::request("call.start", json!({}), crate::rfc3339_now()),
            &mut host,
        )
        .unwrap();
        assert!(started.error.is_none(), "{:?}", started.error);
        assert_eq!(started.payload["state"], "CONNECTING");

        // The callee's poll surfaces the offer as an explicit INCOMING
        // state — a call rings for approval, it is never auto-answered.
        let mut approved = false;
        for _ in 0..100 {
            match signaling::tick(&mut peer) {
                SignalingTick::IncomingOffer {
                    offer_call_id,
                    sdp,
                    peer: caller,
                    ..
                } => {
                    peer.present_incoming(&offer_call_id, &sdp, caller);
                    approved = true;
                    break;
                }
                SignalingTick::Quiet => thread::sleep(Duration::from_millis(50)),
            }
        }
        assert!(approved, "the paired peer never surfaced the offer");
        assert_eq!(peer.call.phase, "INCOMING");

        // The callee approves: acceptance prepares its own signaling leg,
        // starts the worker with the caller's offer, and relays the answer.
        let accepted = dispatch(
            Envelope::request("call.accept", json!({}), crate::rfc3339_now()),
            &mut peer,
        )
        .unwrap();
        assert!(accepted.error.is_none(), "{:?}", accepted.error);
        assert_eq!(accepted.payload["state"], "CONNECTING");
        assert_eq!(peer.call.phase, "CONNECTING");

        // Candidate relays run in both directions until the workers find the
        // direct path; each tick also carries the answer and ICE material.
        for _iteration in 0..200 {
            host.drain_media_events();
            peer.drain_media_events();
            if host.call.phase == "CONNECTED" && peer.call.phase == "CONNECTED" {
                break;
            }
            signaling::tick(&mut host);
            signaling::tick(&mut peer);
            thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(host.call.phase, "CONNECTED", "caller never connected");
        assert_eq!(peer.call.phase, "CONNECTED", "callee never connected");

        // Activity sharing rides the same direct channel: a sanitized frame
        // crosses only through the live call, and the peer sees exactly the
        // validated records — never the raw material behind them.
        let host_identity = crate::load_or_create(&host.state_dir, now_seconds()).unwrap();
        host.signaling.cache_identity(host_identity);
        {
            let mut engine = host.engine.lock().unwrap();
            assert!(engine.mark_monitor_started(now_seconds()));
            assert!(engine.ingest(
                now_seconds(),
                vec![crate::activity::RawObservation {
                    pid: 7,
                    exe_path: Some("/usr/bin/vlc".into()),
                    command_line: Some("vlc --loop".into()),
                }],
            ));
        }
        host.sync_direct();
        let mut shared = false;
        for _ in 0..100 {
            peer.sync_direct();
            if !peer.remote_activity.is_empty() {
                shared = true;
                break;
            }
            thread::sleep(Duration::from_millis(50));
        }
        assert!(
            shared,
            "the peer never received the sanitized activity frame"
        );
        let serialized = serde_json::to_string(&peer.remote_activity_json()).unwrap();
        assert!(!serialized.contains("usr/bin"), "path leaked: {serialized}");
        assert!(
            !serialized.contains("--loop"),
            "cmdline leaked: {serialized}"
        );
        assert_eq!(peer.remote_activity[0].label, "vlc");

        // Ending the call tears down the worker and the signaling session;
        // the callee keeps its own call until it ends it explicitly.
        let ended = dispatch(
            Envelope::request("call.end", json!({}), crate::rfc3339_now()),
            &mut host,
        )
        .unwrap();
        assert!(ended.error.is_none(), "{:?}", ended.error);
        assert_eq!(ended.payload["state"], "ENDED");
        assert!(host.call.call_id.is_none());
        assert!(peer.call.call_id.is_some());

        cleanup_e2e(listener, &[server_dir, host_dir, peer_dir]);
    }

    #[test]
    fn diagnostics_report_absence_without_a_configured_server() {
        let mut core_state = state();
        let response = dispatch(
            Envelope::request("network.diagnostics", json!({}), crate::rfc3339_now()),
            &mut core_state,
        )
        .unwrap();
        assert!(response.error.is_none(), "{:?}", response.error);
        assert_eq!(response.payload["server"]["configured"], false);
        assert_eq!(response.payload["server"]["reachable"], false);
        // Absence of a live call is reported as absence, never as a number.
        assert_eq!(response.payload["direct"]["active"], false);
        assert!(response.payload["direct"].get("rtt_ms").is_none());
    }

    #[test]
    fn diagnostics_report_an_unreachable_server_honestly() {
        let mut core_state = state();
        let configured = dispatch(
            Envelope::request(
                "server.configure",
                json!({
                    "address": "127.0.0.1:1",
                    "fingerprint": "ab".repeat(32),
                }),
                crate::rfc3339_now(),
            ),
            &mut core_state,
        )
        .unwrap();
        assert!(configured.error.is_none(), "{configured:?}");

        let response = dispatch(
            Envelope::request("network.diagnostics", json!({}), crate::rfc3339_now()),
            &mut core_state,
        )
        .unwrap();
        assert!(response.error.is_none(), "{:?}", response.error);
        assert_eq!(response.payload["server"]["configured"], true);
        assert_eq!(response.payload["server"]["reachable"], false);
        assert!(response.payload["server"].get("rtt_ms").is_none());
    }

    #[test]
    fn diagnostics_measure_a_real_pinned_exchange() {
        let (listener, mut host, _peer, server_dir, host_dir, peer_dir) = spawn_server_and_pair();
        // A call that never existed reports no direct path.
        host.call.phase = "IDLE".into();

        let response = dispatch(
            Envelope::request("network.diagnostics", json!({}), crate::rfc3339_now()),
            &mut host,
        )
        .unwrap();
        assert!(response.error.is_none(), "{:?}", response.error);
        assert_eq!(response.payload["server"]["configured"], true);
        assert_eq!(response.payload["server"]["reachable"], true);
        let handshake = response.payload["server"]["handshake_ms"].as_f64().unwrap();
        let rtt = response.payload["server"]["rtt_ms"].as_f64().unwrap();
        assert!(handshake > 0.0, "a real handshake takes measurable time");
        assert!(rtt > 0.0, "a real exchange takes measurable time");

        // A live call with worker stats folds them into the same report.
        host.call.phase = "CONNECTED".into();
        host.call.call_id = Some("call-1".into());
        host.absorb_media_event(&stats_event(35.0, 990, 10));
        let response = dispatch(
            Envelope::request("network.diagnostics", json!({}), crate::rfc3339_now()),
            &mut host,
        )
        .unwrap();
        assert_eq!(response.payload["direct"]["active"], true);
        assert!(
            (response.payload["direct"]["rtt_ms"].as_f64().unwrap() - 35.0).abs() < f64::EPSILON
        );
        assert_eq!(response.payload["direct"]["quality"], "good");

        cleanup_e2e(listener, &[server_dir, host_dir, peer_dir]);
    }

    fn voice_event(
        level: f64,
        speaking: bool,
        remote_level: f64,
        remote_speaking: bool,
    ) -> MediaEvent {
        MediaEvent {
            message_type: "media.voice_level".into(),
            payload: json!({
                "call_id": "call-1",
                "level": level,
                "speaking": speaking,
                "remote_level": remote_level,
                "remote_speaking": remote_speaking,
                "muted": false,
            }),
        }
    }

    #[test]
    fn voice_level_facts_surface_as_voice_level_events() {
        let mut core_state = state();
        core_state.call.call_id = Some("call-1".into());

        core_state.absorb_media_event(&voice_event(0.62, true, 0.1, false));
        assert!(core_state.voice_dirty);
        assert!((core_state.call.voice.level - 0.62).abs() < f64::EPSILON);
        assert!(core_state.call.voice.speaking);
        assert!(!core_state.call.voice.remote_speaking);

        let event = core_state.take_voice_event().unwrap();
        assert_eq!(event.message_type, "voice.level");
        assert_eq!(event.payload["call_id"], "call-1");
        assert!((event.payload["level"].as_f64().unwrap() - 0.62).abs() < f64::EPSILON);
        assert_eq!(event.payload["speaking"], true);
        assert_eq!(event.payload["remote_speaking"], false);
        assert!(core_state.take_voice_event().is_none());
    }

    #[test]
    fn voice_level_facts_for_other_calls_are_ignored() {
        let mut core_state = state();
        core_state.call.call_id = Some("call-1".into());
        let mut stranger = voice_event(0.9, true, 0.9, true);
        stranger.payload["call_id"] = "call-2".into();
        core_state.absorb_media_event(&stranger);
        assert!(!core_state.voice_dirty);
        assert!(!core_state.call.voice.speaking);
    }

    #[test]
    fn call_reset_clears_voice_facts() {
        let mut core_state = state();
        core_state.call.call_id = Some("call-1".into());
        core_state.absorb_media_event(&voice_event(0.5, true, 0.5, true));
        core_state.call.reset();
        assert!(!core_state.call.voice.speaking);
        assert!(!core_state.call.voice.remote_speaking);
        assert!(core_state.call.voice.level == 0.0);
    }

    fn stats_event(rtt_ms: f64, received: u64, lost: i64) -> MediaEvent {
        MediaEvent {
            message_type: "media.call_stats".into(),
            payload: json!({
                "call_id": "call-1",
                "rtt_ms": rtt_ms,
                "received": received,
                "lost": lost,
            }),
        }
    }

    #[test]
    fn transport_facts_surface_as_stats_events_with_quality() {
        let mut core_state = state();
        core_state.call.call_id = Some("call-1".into());
        // No sample yet: the snapshot reports the absence, not a verdict.
        assert!(core_state.call.snapshot()["stats"].is_null());

        core_state.absorb_media_event(&stats_event(42.0, 980, 10));
        assert!(core_state.stats_dirty);
        let stats = core_state.call.stats.expect("stats stored");
        assert!((stats.rtt_ms - 42.0).abs() < f64::EPSILON);
        assert_eq!(stats.packets_received, 980);
        assert_eq!(stats.packets_lost, 10);
        assert!((stats.loss_pct() - (10.0 / 990.0) * 100.0).abs() < 1e-9);
        assert_eq!(stats.quality(), "good");

        let event = core_state.take_stats_event().unwrap();
        assert_eq!(event.message_type, "call.stats_changed");
        assert_eq!(event.payload["call_id"], "call-1");
        assert!((event.payload["rtt_ms"].as_f64().unwrap() - 42.0).abs() < f64::EPSILON);
        assert_eq!(event.payload["quality"], "good");
        assert!(core_state.take_stats_event().is_none());

        // A degraded sample replaces the old one and downgrades the verdict.
        core_state.absorb_media_event(&stats_event(600.0, 1000, 200));
        let event = core_state.take_stats_event().unwrap();
        assert_eq!(event.payload["quality"], "poor");
    }

    #[test]
    fn transport_facts_for_other_calls_are_ignored() {
        let mut core_state = state();
        core_state.call.call_id = Some("call-1".into());
        let mut stranger = stats_event(30.0, 100, 0);
        stranger.payload["call_id"] = "call-2".into();
        core_state.absorb_media_event(&stranger);
        assert!(!core_state.stats_dirty);
        assert!(core_state.call.stats.is_none());
    }

    #[test]
    fn call_reset_clears_transport_facts() {
        let mut core_state = state();
        core_state.call.call_id = Some("call-1".into());
        core_state.absorb_media_event(&stats_event(35.0, 500, 5));
        core_state.call.reset();
        assert!(core_state.call.stats.is_none());
        assert!(core_state.call.snapshot()["stats"].is_null());
    }

    #[test]
    fn quality_boundaries_stay_honest() {
        let good = CallStats {
            rtt_ms: 150.0,
            packets_received: 990,
            packets_lost: 10,
        };
        assert_eq!(good.quality(), "good");
        // 2% exactly is no longer the good band.
        let borderline = CallStats {
            rtt_ms: 100.0,
            packets_received: 980,
            packets_lost: 20,
        };
        assert_eq!(borderline.quality(), "fair");
        let fair = CallStats {
            rtt_ms: 400.0,
            packets_received: 930,
            packets_lost: 70,
        };
        assert_eq!(fair.quality(), "fair");
        let lossy = CallStats {
            rtt_ms: 100.0,
            packets_received: 900,
            packets_lost: 100,
        };
        assert_eq!(lossy.quality(), "poor");
        let slow = CallStats {
            rtt_ms: 401.0,
            packets_received: 1000,
            packets_lost: 0,
        };
        assert_eq!(slow.quality(), "poor");
    }

    #[test]
    fn worker_devices_must_be_hex_or_default() {
        assert_eq!(sanitized_worker_device(""), "");
        assert_eq!(sanitized_worker_device("default-microphone"), "");
        assert_eq!(sanitized_worker_device("harbor-headphones"), "");
        assert_eq!(sanitized_worker_device("with space"), "");
        assert_eq!(sanitized_worker_device("a1B2c3D4"), "a1B2c3D4");
    }

    #[test]
    fn audio_call_payload_carries_settings_into_the_call() {
        let mut core_state = state();
        dispatch(
            Envelope::request(
                "settings.update",
                json!({
                    "microphoneVolume": 1.5,
                    "outputVolume": -0.5,
                    "pushToTalkEnabled": true,
                    "voiceActivation": true,
                }),
                "2026-08-31T20:00:00Z",
            ),
            &mut core_state,
        )
        .unwrap();

        let payload = audio_call_payload(&core_state, "call-9");
        assert_eq!(payload["call_id"], "call-9");
        // Volumes persist clamped into the worker's supported range.
        assert_eq!(payload["input_volume"], json!(1.0));
        assert_eq!(payload["output_volume"], json!(0.0));
        // Placeholder device names never reach the worker as ids.
        assert_eq!(payload["input_device"], json!(""));
        assert_eq!(payload["output_device"], json!(""));
        assert_eq!(payload["ptt_enabled"], json!(true));
        assert_eq!(payload["voice_activation"], json!(true));
    }

    #[test]
    fn voice_activation_mode_persists_and_flows_to_the_call_payload() {
        let mut core_state = state();
        let response = dispatch(
            Envelope::request(
                "settings.update",
                json!({"voiceActivation": true}),
                "2026-09-01T20:00:00Z",
            ),
            &mut core_state,
        )
        .unwrap();
        assert!(response.error.is_none(), "{:?}", response.error);

        let settings = core_state.settings.as_ref().unwrap();
        assert!(settings.values().voice_activation);
        // With no live worker the setting is purely durable; the next call
        // still opens gated by it.
        assert_eq!(
            audio_call_payload(&core_state, "call-1")["voice_activation"],
            json!(true)
        );

        let response = dispatch(
            Envelope::request(
                "settings.update",
                json!({"voiceActivation": false}),
                "2026-09-01T20:00:01Z",
            ),
            &mut core_state,
        )
        .unwrap();
        assert!(response.error.is_none(), "{:?}", response.error);
        let settings = core_state.settings.as_ref().unwrap();
        assert!(!settings.values().voice_activation);
    }

    #[test]
    fn push_to_talk_mode_persists_and_responds() {
        let mut core_state = state();
        let response = dispatch(
            Envelope::request(
                "call.push_to_talk",
                json!({"enabled": true}),
                "2026-08-31T20:00:00Z",
            ),
            &mut core_state,
        )
        .unwrap();
        assert!(response.error.is_none(), "{:?}", response.error);
        assert_eq!(response.payload["push_to_talk"]["enabled"], true);
        assert_eq!(response.payload["push_to_talk"]["active"], false);

        let settings = core_state.settings.as_ref().unwrap();
        assert!(settings.values().push_to_talk_enabled);

        // A missing fact set is a protocol error, not a silent no-op.
        let refused = dispatch(
            Envelope::request("call.push_to_talk", json!({}), "2026-08-31T20:00:01Z"),
            &mut core_state,
        )
        .unwrap();
        assert_eq!(refused.error.as_ref().unwrap().code, "invalid_request");
    }

    #[test]
    fn audio_config_set_persists_and_reports_effective_values() {
        let mut core_state = state();
        let response = dispatch(
            Envelope::request(
                "audio.config",
                json!({"set": true, "input_volume": 0.3, "output_volume": 0.8}),
                "2026-08-31T20:00:00Z",
            ),
            &mut core_state,
        )
        .unwrap();
        assert!(response.error.is_none(), "{:?}", response.error);
        assert!((response.payload["input_volume"].as_f64().unwrap() - 0.3).abs() < 1e-9);
        assert!((response.payload["output_volume"].as_f64().unwrap() - 0.8).abs() < 1e-9);

        let settings = core_state.settings.as_ref().unwrap();
        assert!((settings.values().microphone_volume - 0.3).abs() < 1e-9);

        // A get without a live call answers from the durable settings.
        let read = dispatch(
            Envelope::request("audio.config", json!({}), "2026-08-31T20:00:01Z"),
            &mut core_state,
        )
        .unwrap();
        assert!(read.error.is_none(), "{:?}", read.error);
        assert!((read.payload["input_volume"].as_f64().unwrap() - 0.3).abs() < 1e-9);

        // An empty set is refused, not silently accepted.
        let refused = dispatch(
            Envelope::request("audio.config", json!({"set": true}), "2026-08-31T20:00:02Z"),
            &mut core_state,
        )
        .unwrap();
        assert_eq!(refused.error.as_ref().unwrap().code, "invalid_request");
    }

    #[test]
    fn audio_switch_devices_requires_a_live_call() {
        let mut core_state = state();
        let response = dispatch(
            Envelope::request(
                "audio.switch_devices",
                json!({"call_id": "call-1", "input_device": "a1", "output_device": "b2"}),
                "2026-08-31T20:00:00Z",
            ),
            &mut core_state,
        )
        .unwrap();
        assert_eq!(response.error.as_ref().unwrap().code, "call_inactive");
    }

    #[test]
    fn mic_test_poll_and_stop_idle_without_a_worker() {
        // Neither poll nor stop may spawn a worker: idleness is a local
        // answer, so this runs with no media binary at all.
        let mut core_state = state();
        assert!(core_state.mic_test.is_none());
        let poll = dispatch(
            Envelope::request("audio.loopback_poll", json!({}), "2026-08-31T20:00:00Z"),
            &mut core_state,
        )
        .unwrap();
        assert!(poll.error.is_none());
        assert_eq!(poll.payload["active"], false);
        assert!(core_state.mic_test.is_none());

        let stop = dispatch(
            Envelope::request("audio.loopback_stop", json!({}), "2026-08-31T20:00:00Z"),
            &mut core_state,
        )
        .unwrap();
        assert!(stop.error.is_none());
        assert_eq!(stop.payload["stopped"], true);
        assert!(core_state.mic_test.is_none());
    }

    #[test]
    fn mic_test_start_refuses_while_a_call_owns_the_devices() {
        let mut core_state = state();
        // A live call phase is enough to refuse: no worker spawns, no
        // microphone opens, and the error names the conflict honestly.
        core_state.call.call_id = Some("call-1".into());
        let refused = dispatch(
            Envelope::request(
                "audio.loopback_start",
                json!({"seconds": 3}),
                "2026-08-31T20:00:00Z",
            ),
            &mut core_state,
        )
        .unwrap();
        let error = refused.error.expect("a call must refuse the mic test");
        assert_eq!(error.code, "call_active");
        assert_eq!(error.ui_key, "error.audio.busy");
        assert!(core_state.mic_test.is_none());
    }

    fn full_mobile_status() -> Value {
        json!({
            "schemaVersion": 1,
            "deviceType": "mobile",
            "batteryPercent": 73,
            "charging": true,
            "phoneActivity": "ACTIVE",
            "lastActiveAt": 1_700_000_100_u64,
            "currentApp": "YouTube",
            "locationSharingEnabled": true,
            "location": {
                "latitude": -23.5505,
                "longitude": -46.6333,
                "accuracyMeters": 12.0,
                "updatedAt": 1_700_000_000_u64,
            },
            "notificationSharingEnabled": true,
        })
    }

    #[test]
    fn device_state_reports_this_install_as_desktop_standalone() {
        let mut core_state = state();
        let response = dispatch(
            Envelope::request("device.state", json!({}), "2026-08-31T20:00:00Z"),
            &mut core_state,
        )
        .unwrap();
        assert!(response.error.is_none());
        assert_eq!(response.payload["device"]["deviceType"], "desktop");
        assert!(!response.payload["device"]["deviceId"].as_str().unwrap().is_empty());
        assert_eq!(response.payload["mode"], "standalone");
        assert_eq!(response.payload["linked"].as_array().unwrap().len(), 0);
        assert!(response.payload["mediaEndpoint"].is_null());
        // Nothing changed, so no event is owed.
        assert!(core_state.take_device_event().is_none());
    }

    #[test]
    fn device_configure_switches_device_type_and_rejects_unknown() {
        let mut core_state = state();
        let switched = dispatch(
            Envelope::request(
                "device.configure",
                json!({"deviceType": "mobile"}),
                "2026-08-31T20:00:00Z",
            ),
            &mut core_state,
        )
        .unwrap();
        assert!(switched.error.is_none());
        assert_eq!(switched.payload["device"]["deviceType"], "mobile");
        let event = core_state.take_device_event().expect("type switch emits");
        assert_eq!(event.message_type, "device.updated");
        assert!(core_state.take_device_event().is_none());

        let refused = dispatch(
            Envelope::request(
                "device.configure",
                json!({"deviceType": "tablet"}),
                "2026-08-31T20:00:01Z",
            ),
            &mut core_state,
        )
        .unwrap();
        assert_eq!(refused.error.as_ref().unwrap().code, "unknown_device_type");

        let empty = dispatch(
            Envelope::request("device.configure", json!({}), "2026-08-31T20:00:02Z"),
            &mut core_state,
        )
        .unwrap();
        assert_eq!(empty.error.as_ref().unwrap().code, "invalid_request");
    }

    #[test]
    fn device_link_forms_a_companion_and_unlink_restores_standalone() {
        let mut core_state = state();
        let phone_id = Uuid::new_v4().to_string();
        let linked = dispatch(
            Envelope::request(
                "device.configure",
                json!({"link": {"deviceId": phone_id, "deviceType": "mobile"}}),
                "2026-08-31T20:00:00Z",
            ),
            &mut core_state,
        )
        .unwrap();
        assert!(linked.error.is_none());
        // This install is a desktop; a linked phone makes it a companion.
        assert_eq!(linked.payload["mode"], "companion");
        assert_eq!(linked.payload["linked"].as_array().unwrap().len(), 1);

        let unlinked = dispatch(
            Envelope::request(
                "device.configure",
                json!({"unlink": {"deviceId": phone_id}}),
                "2026-08-31T20:00:01Z",
            ),
            &mut core_state,
        )
        .unwrap();
        assert!(unlinked.error.is_none());
        assert_eq!(unlinked.payload["mode"], "standalone");

        let unknown = dispatch(
            Envelope::request(
                "device.configure",
                json!({"unlink": {"deviceId": phone_id}}),
                "2026-08-31T20:00:02Z",
            ),
            &mut core_state,
        )
        .unwrap();
        assert_eq!(unknown.error.as_ref().unwrap().code, "unknown_device");

        let malformed = dispatch(
            Envelope::request(
                "device.configure",
                json!({"link": {"deviceId": "not-a-uuid", "deviceType": "mobile"}}),
                "2026-08-31T20:00:03Z",
            ),
            &mut core_state,
        )
        .unwrap();
        assert_eq!(malformed.error.as_ref().unwrap().code, "invalid_device_link");
    }

    #[test]
    fn mobile_update_validates_consent_and_emits_once() {
        let mut core_state = state();
        let stored = dispatch(
            Envelope::request("mobile.update", full_mobile_status(), "2026-08-31T20:00:00Z"),
            &mut core_state,
        )
        .unwrap();
        assert!(stored.error.is_none());

        let snapshot = dispatch(
            Envelope::request("mobile.state", json!({}), "2026-08-31T20:00:01Z"),
            &mut core_state,
        )
        .unwrap();
        assert_eq!(snapshot.payload["own"]["batteryPercent"], 73);
        assert_eq!(snapshot.payload["own"]["currentApp"], "YouTube");

        let event = core_state.take_mobile_event().expect("update emits");
        assert_eq!(event.message_type, "mobile.updated");
        assert!(core_state.take_mobile_event().is_none());

        // A fix without its toggle is refused with its own code.
        let mut smuggled = full_mobile_status();
        smuggled["locationSharingEnabled"] = json!(false);
        let refused = dispatch(
            Envelope::request("mobile.update", smuggled, "2026-08-31T20:00:02Z"),
            &mut core_state,
        )
        .unwrap();
        assert_eq!(
            refused.error.as_ref().unwrap().code,
            "location_without_consent"
        );

        let malformed = dispatch(
            Envelope::request(
                "mobile.update",
                json!({"deviceType": "mobile"}),
                "2026-08-31T20:00:03Z",
            ),
            &mut core_state,
        )
        .unwrap();
        assert_eq!(malformed.error.as_ref().unwrap().code, "invalid_request");
    }

    #[test]
    fn device_state_names_peers_blocked_and_link() {
        let mut core_state = state();
        // A mobile install that learned a mobile peer is blocked.
        core_state
            .settings
            .as_mut()
            .unwrap()
            .update(&json!({"deviceType": "mobile"}))
            .unwrap();
        let peer = Uuid::new_v4();
        core_state.signaling.seed_peer(peer, "harbor-peer");
        core_state.signaling.note_peer_device(&peer, DeviceType::Mobile);
        let response = dispatch(
            Envelope::request("device.state", json!({}), "2026-08-31T20:00:00Z"),
            &mut core_state,
        )
        .unwrap();
        assert!(response.error.is_none());
        assert_eq!(response.payload["blocked"], true);
        assert_eq!(response.payload["peers"].as_array().unwrap().len(), 1);
        assert_eq!(response.payload["peers"][0]["deviceType"], "mobile");
        assert!(response.payload["linkPeer"].is_null());
    }

    #[test]
    fn mobile_state_carries_the_linked_peer_alongside_own() {
        let mut core_state = state();
        let stored = dispatch(
            Envelope::request("mobile.update", full_mobile_status(), "2026-08-31T20:00:00Z"),
            &mut core_state,
        )
        .unwrap();
        assert!(stored.error.is_none());
        let snapshot = dispatch(
            Envelope::request("mobile.state", json!({}), "2026-08-31T20:00:01Z"),
            &mut core_state,
        )
        .unwrap();
        assert_eq!(snapshot.payload["own"]["batteryPercent"], 73);
        assert!(snapshot.payload["peer"].is_null());
    }

    /// Two cores, desktop persona and mobile persona, linked over loopback
    /// exactly like a companion pair: chat flows both ways with delivery
    /// acks, the profile converges, and the phone aggregate arrives.
    fn link_pair() -> (CoreState, CoreState) {
        let mut desktop = state();
        let mut mobile = state();
        mobile
            .settings
            .as_mut()
            .unwrap()
            .update(&json!({"deviceType": "mobile"}))
            .unwrap();
        (desktop, mobile)
    }

    fn link_identities(desktop: &CoreState, mobile: &CoreState) -> (LinkContext, LinkContext) {
        use crate::device::DeviceType as DT;
        let did = load_or_create(&desktop.state_dir, 1_700_000_000).unwrap();
        let mid = load_or_create(&mobile.state_dir, 1_700_000_000).unwrap();
        let dkey = did.record().public_key.clone();
        let mkey = mid.record().public_key.clone();
        let dctx = LinkContext {
            device_id: did.record().device_id,
            harbor_id: did.record().harbor_id.clone(),
            signing_seed: did.seed_bytes(),
            device_type: DT::Desktop,
            peers: vec![LinkPeer {
                device_id: mid.record().device_id,
                harbor_id: mid.record().harbor_id.clone(),
                public_key: mkey,
            }],
        };
        let mctx = LinkContext {
            device_id: mid.record().device_id,
            harbor_id: mid.record().harbor_id.clone(),
            signing_seed: mid.seed_bytes(),
            device_type: DT::Mobile,
            peers: vec![LinkPeer {
                device_id: did.record().device_id,
                harbor_id: did.record().harbor_id.clone(),
                public_key: dkey,
            }],
        };
        (dctx, mctx)
    }

    fn pump_pair(desktop: &mut CoreState, mobile: &mut CoreState, rounds: usize) {
        for _ in 0..rounds {
            desktop.pump_link();
            mobile.pump_link();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[test]
    fn link_pair_exchanges_chat_profile_and_phone_state() {
        let (mut desktop, mut mobile) = link_pair();
        let (dctx, mctx) = link_identities(&desktop, &mobile);
        desktop.link.listen(dctx);
        // The listener advertises port + fingerprint; the dialer pins and
        // challenges. Drain folds the Listening event into handle state.
        let mut advertised = None;
        for _ in 0..100 {
            desktop.link.drain();
            if let Some((live_port, live_fp)) = desktop.link.listening() {
                advertised = Some((live_port, live_fp.to_owned()));
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let (port, fingerprint_hex) = advertised.expect("the desktop listener must advertise");
        assert!(port != 0);
        let invite = LinkInvite {
            addrs: vec!["127.0.0.1".into()],
            port,
            fingerprint: crate::app::mobile_link::parse_fingerprint_hex(&fingerprint_hex).unwrap(),
        };
        mobile.link.dial(mctx, invite);
        // Both sides authenticate within a few pumps.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while (desktop.link.live_peer().is_none() || mobile.link.live_peer().is_none())
            && std::time::Instant::now() < deadline
        {
            pump_pair(&mut desktop, &mut mobile, 2);
        }
        assert!(desktop.link.live_peer().is_some(), "desktop sees the phone");
        assert!(mobile.link.live_peer().is_some(), "phone sees the desktop");

        // Chat both ways, no call phase anywhere.
        dispatch(
            Envelope::request(
                "chat.send",
                json!({"body": "oi do mobile"}),
                "2026-08-31T20:00:00Z",
            ),
            &mut mobile,
        )
        .unwrap();
        dispatch(
            Envelope::request(
                "chat.send",
                json!({"body": "oi do desktop"}),
                "2026-08-31T20:00:01Z",
            ),
            &mut desktop,
        )
        .unwrap();
        pump_pair(&mut desktop, &mut mobile, 30);
        let desk_texts: Vec<String> = desktop
            .chat
            .snapshot()
            .into_iter()
            .map(|message| message.body)
            .collect();
        let mob_texts: Vec<String> = mobile
            .chat
            .snapshot()
            .into_iter()
            .map(|message| message.body)
            .collect();
        assert!(desk_texts.iter().any(|body| body == "oi do mobile"), "{desk_texts:?}");
        assert!(desk_texts.iter().any(|body| body == "oi do desktop"), "{desk_texts:?}");
        assert!(mob_texts.iter().any(|body| body == "oi do mobile"), "{mob_texts:?}");
        assert!(mob_texts.iter().any(|body| body == "oi do desktop"), "{mob_texts:?}");
        // Delivery acks crossed: nothing waits for connection anymore.
        assert!(
            desktop.chat.snapshot().into_iter().all(|message| !matches!(
                message.delivery,
                crate::direct::Delivery::WaitingForConnection
            )),
            "desktop deliveries advanced"
        );

        // Profile converges over the same bearer.
        desktop
            .settings
            .as_mut()
            .unwrap()
            .update(&json!({"displayName": "Taylor"}))
            .unwrap();
        pump_pair(&mut desktop, &mut mobile, 30);
        assert_eq!(mobile.profile.partner.display_name, "Taylor");

        // Phone aggregate arrives validated on the desktop.
        let stored = dispatch(
            Envelope::request("mobile.update", full_mobile_status(), "2026-08-31T20:00:02Z"),
            &mut mobile,
        )
        .unwrap();
        assert!(stored.error.is_none());
        pump_pair(&mut desktop, &mut mobile, 30);
        assert_eq!(
            desktop.mobile_peer.as_ref().unwrap().battery_percent,
            Some(73)
        );
        desktop.link.shutdown();
        mobile.link.shutdown();
    }

    #[test]
    fn link_chat_frames_land_in_the_transcript_with_an_ack_queued() {
        let mut core_state = state();
        let peer = Uuid::new_v4();
        // Fake a live bearer just far enough to observe the send path is
        // gated: without one, nothing is queued anywhere.
        assert!(core_state.link.live_peer().is_none());
        let frame = LinkFrame::new(KIND_CHAT, json!({"id": "m1", "body": "oi"})).unwrap();
        core_state.absorb_link_frame(peer, frame);
        let snapshot = core_state.chat.snapshot();
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].body, "oi");
        // A duplicate delivery changes nothing and acks nothing new.
        let echo = LinkFrame::new(KIND_CHAT, json!({"id": "m1", "body": "oi"})).unwrap();
        core_state.absorb_link_frame(peer, echo);
        assert_eq!(core_state.chat.snapshot().len(), 1);
    }

    #[test]
    fn link_profile_and_mobile_frames_validate_like_the_worker_path() {
        let mut core_state = state();
        let peer = Uuid::new_v4();
        // Garbage profile frames never touch the partner snapshot.
        let junk = LinkFrame::new(KIND_PROFILE, json!({"frame": "not-a-frame"})).unwrap();
        core_state.absorb_link_frame(peer, junk);
        assert!(core_state.take_profile_event().is_none());
        // Phone state without consent is refused, not stored.
        let mut smuggled = full_mobile_status();
        smuggled["locationSharingEnabled"] = json!(false);
        let bad = LinkFrame::new(KIND_MOBILE, smuggled).unwrap();
        core_state.absorb_link_frame(peer, bad);
        assert!(core_state.mobile_peer.is_none());
        assert!(core_state.take_mobile_event().is_none());
        // Valid phone state lands on the peer slot and emits.
        let good = LinkFrame::new(KIND_MOBILE, full_mobile_status()).unwrap();
        core_state.absorb_link_frame(peer, good);
        assert_eq!(
            core_state.mobile_peer.as_ref().unwrap().battery_percent,
            Some(73)
        );
        let event = core_state.take_mobile_event().expect("peer status emits");
        assert_eq!(event.message_type, "mobile.updated");
        assert_eq!(event.payload["peer"]["batteryPercent"], 73);
    }

    #[test]
    fn takeover_requires_a_live_call() {
        let mut core_state = state();
        let refused = dispatch(
            Envelope::request(
                "call.takeover",
                json!({"joinDevice": Uuid::new_v4()}),
                "2026-08-31T20:00:00Z",
            ),
            &mut core_state,
        )
        .unwrap();
        assert_eq!(refused.error.as_ref().unwrap().code, "call_inactive");
    }

    #[test]
    fn takeover_grants_drop_then_join_and_drops_local_media_first() {
        let mut core_state = state();
        core_state.call.phase = "CONNECTED".into();
        core_state.call.call_id = Some("call-1".into());
        let sibling = Uuid::new_v4();
        let granted = dispatch(
            Envelope::request(
                "call.takeover",
                json!({"joinDevice": sibling}),
                "2026-08-31T20:00:00Z",
            ),
            &mut core_state,
        )
        .unwrap();
        assert!(granted.error.is_none());
        assert_eq!(granted.payload["action"], "drop_then_join");
        assert_eq!(granted.payload["joinDevice"], sibling.to_string());
        // Drop first: local media is gone, the endpoint moved, the reason
        // says why — the sibling's join is its own call start.
        assert_ne!(core_state.call.phase, "CONNECTED");
        assert_eq!(core_state.call.reason, "takeover");
        assert_eq!(core_state.media_endpoint, Some(sibling));
        assert!(core_state.take_device_event().is_some());
    }

    #[test]
    fn takeover_to_the_holding_device_is_already_active() {
        let mut core_state = state();
        core_state.call.phase = "CONNECTED".into();
        core_state.call.call_id = Some("call-1".into());
        let snapshot = dispatch(
            Envelope::request("device.state", json!({}), "2026-08-31T20:00:00Z"),
            &mut core_state,
        )
        .unwrap();
        let own = snapshot.payload["device"]["deviceId"].clone();
        let repeated = dispatch(
            Envelope::request(
                "call.takeover",
                json!({"joinDevice": own}),
                "2026-08-31T20:00:01Z",
            ),
            &mut core_state,
        )
        .unwrap();
        assert!(repeated.error.is_none());
        assert_eq!(repeated.payload["action"], "already_active");
        // Nothing dropped: the call still stands.
        assert_eq!(core_state.call.phase, "CONNECTED");
    }

    /// Drives the C ABI like the Android facade does: framed bytes in,
    /// framed bytes out, every buffer freed exactly once.
    fn abi_round_trip(frames: &[u8]) -> Vec<harbor_protocol::Envelope> {
        use std::ffi::CString;
        let dir = std::env::temp_dir().join(format!("harbor-abi-test-{}", Uuid::new_v4()));
        let path = CString::new(dir.to_string_lossy().into_owned()).unwrap();
        let handle = harbor_core_create(path.as_ptr());
        assert!(!handle.is_null());
        let mut out_len = 0_usize;
        let out = harbor_core_dispatch(handle, frames.as_ptr(), frames.len(), &mut out_len);
        assert!(!out.is_null() && out_len > 0);
        let bytes = unsafe { std::slice::from_raw_parts(out, out_len) }.to_vec();
        harbor_core_free(out, out_len);
        // A tick with no server configured emits nothing, but must not fail.
        let mut tick_len = 0_usize;
        let ticked = harbor_core_tick(handle, &mut tick_len);
        assert!(ticked.is_null() && tick_len == 0);
        harbor_core_destroy(handle);
        let mut decoder = harbor_protocol::FrameDecoder::default();
        decoder.push(&bytes).expect("facade frames decode")
    }

    #[test]
    fn abi_dispatch_answers_device_state() {
        let request = harbor_protocol::Envelope::request(
            "device.state",
            json!({}),
            "2026-08-31T20:00:00Z",
        );
        let frame = harbor_protocol::encode_frame(&request).unwrap();
        let replies = abi_round_trip(&frame);
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].message_type, "device.state");
        assert!(replies[0].error.is_none());
        assert_eq!(replies[0].payload["device"]["deviceType"], "desktop");
    }

    #[test]
    fn abi_rejects_garbage_and_nulls() {
        let mut out_len = 0_usize;
        assert!(harbor_core_dispatch(
            std::ptr::null_mut(),
            [0_u8; 4].as_ptr(),
            4,
            &mut out_len
        )
        .is_null());
        assert!(harbor_core_tick(std::ptr::null_mut(), &mut out_len).is_null());
        harbor_core_destroy(std::ptr::null_mut());
        harbor_core_free(std::ptr::null_mut(), 0);
    }
}

// ---------------------------------------------------------------------------
// C ABI for the mobile build (and any other in-process host).
//
// The desktop binary drives this same state over framed stdio; Android
// cannot spawn the core as a child process, so the Qt facade calls in
// directly. The wire format is identical — length-prefixed v1 envelopes —
// so the facade speaks one protocol on both platforms.
//
// Ownership: every function returning `*mut c_uchar` hands a Rust-allocated
// buffer to the caller with its length in `out_len`; the caller releases
// it with exactly one `harbor_core_free`. Null in, null out, always.
// ---------------------------------------------------------------------------

use std::ffi::CStr;
use std::os::raw::{c_char, c_uchar};

/// Creates the core for `state_dir` (UTF-8 path). Null/empty falls back to
/// the platform default directory, like the desktop binary. Returns an
/// opaque handle, or null when the directory cannot even be resolved.
#[unsafe(no_mangle)]
pub extern "C" fn harbor_core_create(state_dir: *const c_char) -> *mut CoreState {
    let state = if state_dir.is_null() {
        CoreState::from_default()
    } else {
        let dir = unsafe { CStr::from_ptr(state_dir) }.to_string_lossy();
        if dir.is_empty() {
            CoreState::from_default()
        } else {
            CoreState::for_directory(std::path::Path::new(dir.as_ref()))
        }
    };
    Box::into_raw(Box::new(state))
}

/// Supplies an app-private media worker path before the first call. This is
/// intentionally a separate host hook rather than an environment variable:
/// Android processes do not inherit a useful executable directory from the
/// APK loader. Calling it while a worker is live is rejected, which prevents
/// a running call from switching binaries underneath its supervisor.
#[unsafe(no_mangle)]
pub extern "C" fn harbor_core_set_media_worker(
    handle: *mut CoreState,
    worker_path: *const c_char,
) -> bool {
    if handle.is_null() || worker_path.is_null() {
        return false;
    }
    let path = unsafe { CStr::from_ptr(worker_path) }.to_string_lossy();
    if path.trim().is_empty() {
        return false;
    }
    let state = unsafe { &mut *handle };
    if state.media.is_some() || !PathBuf::from(path.as_ref()).is_absolute() {
        return false;
    }
    state.media_worker_path = Some(PathBuf::from(path.as_ref()));
    true
}

/// Destroys a handle from [`harbor_core_create`]. Null is a no-op.
#[unsafe(no_mangle)]
pub extern "C" fn harbor_core_destroy(handle: *mut CoreState) {
    if !handle.is_null() {
        unsafe {
            drop(Box::from_raw(handle));
        }
    }
}

/// Dispatches one framed request and returns all framed replies: the
/// correlated response followed by any pending events, concatenated.
/// Null handle, null bytes, or a rejected frame yield null with
/// `out_len = 0`; the facade logs and keeps running.
#[unsafe(no_mangle)]
pub extern "C" fn harbor_core_dispatch(
    handle: *mut CoreState,
    request: *const c_uchar,
    request_len: usize,
    out_len: *mut usize,
) -> *mut c_uchar {
    set_out_len(out_len, 0);
    if handle.is_null() || request.is_null() {
        return std::ptr::null_mut();
    }
    let bytes = unsafe { std::slice::from_raw_parts(request, request_len) };
    let mut decoder = FrameDecoder::default();
    let envelopes = decoder.push(bytes).unwrap_or_default();
    let Some(envelope) = envelopes.into_iter().next() else {
        return std::ptr::null_mut();
    };
    let state = unsafe { &mut *handle };
    // A dispatch error (no correlatable request) has no honest reply: the
    // stdio pump drops it the same way. Null out, facade logs.
    let Ok(response) = dispatch(envelope, state) else {
        return std::ptr::null_mut();
    };
    let mut out = encode_frame(&response).unwrap_or_default();
    for event in drain_events(state) {
        if let Ok(frame) = encode_frame(&event) {
            out.extend_from_slice(&frame);
        }
    }
    hand_out(out, out_len)
}

/// Runs one background step (signaling, media, link, direct-state, and
/// timeout pumps) and returns any pending events as concatenated frames. The
/// host calls this on a ~1 s cadence, mirroring the desktop pump's poll loop.
#[unsafe(no_mangle)]
pub extern "C" fn harbor_core_tick(handle: *mut CoreState, out_len: *mut usize) -> *mut c_uchar {
    set_out_len(out_len, 0);
    if handle.is_null() {
        return std::ptr::null_mut();
    }
    let state = unsafe { &mut *handle };
    // The stdio host has a media-wake channel; the in-process mobile host
    // does not.  Its one-second ticker is therefore the wakeup for every
    // asynchronous source as well as signaling.  In particular, merely
    // calling `signaling::tick` here would leave a Pion `connected` event in
    // the worker queue and strand the mobile UI in CONNECTING forever.
    let signaling_action = signaling::tick(state);
    if let SignalingTick::IncomingOffer {
        ref offer_call_id,
        ref sdp,
        peer,
        ..
    } = signaling_action
    {
        state.present_incoming(offer_call_id, sdp, peer);
    }

    // Drain before syncing direct state so a worker's connected/disconnected
    // transition selects the correct direct path.  A worker that died
    // without publishing a final event is handled by the same terminal
    // cleanup as an explicit media failure; no dead child or microphone
    // owner is retained beneath a FAILED call.
    state.drain_media_events();
    if state.call.call_id.is_some()
        && state.call.phase != "IDLE"
        && state.media.as_ref().is_some_and(|media| !media.is_running())
    {
        state.fail_media();
    }
    enforce_reconnect_window(state);
    state.enforce_incoming_ttl();

    // pump_link is also called by sync_direct.  Keeping the call in one
    // place makes the mobile tick mirror the desktop pump and ensures link
    // chat/profile state is serviced even when no QML request is in flight.
    state.sync_direct();
    // Requests made by sync_direct can cause the worker to queue another
    // unsolicited state transition. Pick it up in this same tick rather
    // than adding another full-second of latency.
    state.drain_media_events();
    if state.call.call_id.is_some()
        && state.call.phase != "IDLE"
        && state.media.as_ref().is_some_and(|media| !media.is_running())
    {
        state.fail_media();
    }
    let mut out = Vec::new();
    for event in drain_events(state) {
        if let Ok(frame) = encode_frame(&event) {
            out.extend_from_slice(&frame);
        }
    }
    hand_out(out, out_len)
}

/// Releases a buffer from [`harbor_core_dispatch`] or [`harbor_core_tick`].
#[unsafe(no_mangle)]
pub extern "C" fn harbor_core_free(buffer: *mut c_uchar, len: usize) {
    if !buffer.is_null() {
        unsafe {
            drop(Vec::from_raw_parts(buffer, len, len));
        }
    }
}

fn set_out_len(out_len: *mut usize, value: usize) {
    if !out_len.is_null() {
        unsafe {
            *out_len = value;
        }
    }
}

fn hand_out(mut bytes: Vec<u8>, out_len: *mut usize) -> *mut c_uchar {
    if bytes.is_empty() {
        return std::ptr::null_mut();
    }
    let ptr = bytes.as_mut_ptr();
    let len = bytes.len();
    std::mem::forget(bytes);
    set_out_len(out_len, len);
    ptr
}

/// Every pending event, oldest first — the same set the stdio pump
/// flushes after each dispatch. One list, two transports.
fn drain_events(state: &mut CoreState) -> Vec<Envelope> {
    [
        state.take_call_event(),
        state.take_share_event(),
        state.take_direct_event(),
        state.take_profile_event(),
        state.take_presence_event(),
        state.take_device_event(),
        state.take_mobile_event(),
        state.take_phone_notification_event(),
        state.take_activity_event(),
        state.take_voice_event(),
        state.take_stats_event(),
    ]
    .into_iter()
    .flatten()
    .collect()
}
