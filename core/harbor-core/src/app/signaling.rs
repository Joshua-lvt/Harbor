//! Signaling legs of direct calls.
//!
//! The core resolves every paired peer and relays opaque SDP/ICE strings
//! through the control-plane server, which routes them without interpreting
//! them. One pinned TLS connection is reused across the idle poll loop; any
//! failure drops it and backs off, because a signaling hiccup must never fail
//! an unrelated local request or surface as user-visible noise.
//!
//! Signals are JSON of the shape `{"call_id", "signal": {..}}`. The server
//! only ever sees an opaque string; both ends unwrap them against their own
//! call identifiers, so the two sides' `call_id`s never need to match.
//!
//! Each pair session also carries a `device_hello`: the endpoint kind each
//! side runs. The server relays it opaquely like any other signal — it never
//! parses device claims. Hellos feed the companion registry (mode, blocked,
//! call routing); a claim that does not match the authenticated sender or
//! the paired record is dropped, never stored.

use std::path::Path;
use std::time::{Duration, Instant};

use super::mobile_link::LinkInvite;
use crate::device::{DeviceType, compatibility};
use harbor_protocol::{Envelope, ProtocolError};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::{
    LocalIdentity, ServerClient, ServerPin, load_or_create, load_server_pin, rfc3339_now,
};

use super::{CoreState, call_error, identity_unavailable_error, now_seconds};

/// Mirrors the server's per-signal byte cap so an oversized relay fails here
/// instead of traveling to be rejected.
const MAX_RELAY_SIGNAL_BYTES: usize = 64 * 1024;
/// Trusted window for the `contacts.list` answer that names the call peer.
const PEER_CACHE_TTL: Duration = Duration::from_secs(30);
/// Partner-presence polls per healthy tick cadence: a 5 s read on the 1 s
/// loop. The 75 s offline tolerance comfortably covers the gaps.
const PARTNER_PRESENCE_POLL_TICKS: u64 = 5;
/// Quiet period between failed connection attempts. A blackholed route burns
/// the whole 10 s dial timeout on the pump's single thread, so without this
/// every 5 s backoff tick would wedge request handling for 10 s at a time.
/// Failed ticks stay fast; the server is retried once per window instead.
const CONNECT_RETRY_COOLDOWN: Duration = Duration::from_secs(30);

/// What one polling step concluded. Offers surface only here; the pump
/// presents them as an explicit INCOMING state — a call is never answered
/// without this device's user approving it first.
pub enum SignalingTick {
    Quiet,
    IncomingOffer {
        offer_call_id: String,
        sdp: String,
        /// Which paired peer sent the offer: answers and declines route
        /// back through this leg's session.
        peer: Uuid,
        /// A sibling endpoint taking over this identity's call media.
        takeover: bool,
    },
}

/// One paired peer as the control plane reports it: addressing plus the
/// public identity facts the link challenge needs later.
#[derive(Debug, Clone)]
pub(crate) struct PeerInfo {
    pub(crate) device_id: Uuid,
    pub(crate) harbor_id: String,
    pub(crate) public_key: String,
}

/// One pair session leg: the session id plus what the peer's `device_hello`
/// taught us. The device kind starts unknown and stays unknown until a
/// validated hello names it — never defaulted, never guessed.
#[derive(Debug, Clone)]
pub(crate) struct PeerLeg {
    pub(crate) peer: Uuid,
    pub(crate) session_id: Option<Uuid>,
    pub(crate) device: Option<DeviceType>,
    pub(crate) hello_sent: bool,
}

/// Transport + relay state for the call signaling loop. Deliberately the only
/// place a `ServerClient` lives between exchanges, so at most one pinned
/// connection exists for calls at any time.
#[derive(Default)]
pub struct Signaling {
    client: Option<ServerClient>,
    identity: Option<LocalIdentity>,
    peers: Vec<PeerInfo>,
    peer_checked_at: Option<Instant>,
    legs: Vec<PeerLeg>,
    failures: u32,
    /// Last `ServerClient::connect` attempt, success or failure. Gates the
    /// retry cooldown that keeps a dead route from wedging the pump.
    last_connect_attempt: Option<Instant>,
    /// Healthy ticks since the last partner-presence poll.
    presence_ticks: u64,
}

impl Signaling {
    /// One leg per known peer, in contacts order. Unknown until helloed.
    pub(crate) fn legs(&self) -> &[PeerLeg] {
        &self.legs
    }

    /// Paired peers with their learned device kinds, for the endpoint
    /// snapshot. Absent kinds stay absent.
    pub(crate) fn peer_snapshot(&self) -> Vec<(Uuid, String, Option<DeviceType>)> {
        self.peers
            .iter()
            .map(|peer| {
                let device = self
                    .legs
                    .iter()
                    .find(|leg| leg.peer == peer.device_id)
                    .and_then(|leg| leg.device);
                (peer.device_id, peer.harbor_id.clone(), device)
            })
            .collect()
    }

    /// Paired peers with the public facts the link worker needs.
    pub(crate) fn peer_infos(&self) -> Vec<(Uuid, String, String)> {
        self.peers
            .iter()
            .map(|peer| (peer.device_id, peer.harbor_id.clone(), peer.public_key.clone()))
            .collect()
    }

    /// The paired harbor identity a peer device belongs to, if paired.
    pub(crate) fn peer_harbor(&self, peer: &Uuid) -> Option<&str> {
        self.peers
            .iter()
            .find(|info| &info.device_id == peer)
            .map(|info| info.harbor_id.as_str())
    }

    fn leg_mut(&mut self, peer: &Uuid) -> &mut PeerLeg {
        if !self.legs.iter().any(|leg| &leg.peer == peer) {
            self.legs.push(PeerLeg {
                peer: *peer,
                session_id: None,
                device: None,
                hello_sent: false,
            });
        }
        self.legs
            .iter_mut()
            .find(|leg| &leg.peer == peer)
            .expect("leg just ensured")
    }

    /// Preferred call target: the first peer that is not a proven
    /// Mobile↔Mobile refusal. Unknown devices stay dialable — the hello in
    /// flight decides, and a hostile answer ends the call with its reason
    /// on the record.
    pub(crate) fn preferred_peer(&self, own: DeviceType) -> Option<Uuid> {
        self.peers.iter().find_map(|peer| {
            let device = self
                .legs
                .iter()
                .find(|leg| leg.peer == peer.device_id)
                .and_then(|leg| leg.device);
            match device {
                Some(remote) if compatibility(own, remote).is_err() => None,
                _ => Some(peer.device_id),
            }
        })
    }

    /// Test seam: one paired peer with an empty public key.
    #[cfg(test)]
    pub(crate) fn seed_peer(&mut self, device_id: Uuid, harbor_id: &str) {
        self.peers.push(PeerInfo {
            device_id,
            harbor_id: harbor_id.into(),
            public_key: String::new(),
        });
        self.leg_mut(&device_id);
    }

    /// Records a validated hello. Returns true when the kind was newly
    /// learned (the endpoint snapshot must re-render).
    pub(crate) fn note_peer_device(&mut self, peer: &Uuid, device: DeviceType) -> bool {
        let leg = self.leg_mut(peer);
        if leg.device == Some(device) {
            return false;
        }
        leg.device = Some(device);
        true
    }
}

/// This install's endpoint kind for hello and routing decisions. Missing
/// settings degrade to desktop — the historically only kind — rather than
/// refusing calls over a persistence hiccup.
pub(crate) fn own_device_type(state: &CoreState) -> DeviceType {
    state
        .settings
        .as_ref()
        .and_then(|settings| DeviceType::parse(settings.values().device_type.as_str()))
        .unwrap_or(DeviceType::Desktop)
}

impl Signaling {
    /// How long the pump should wait before the next polling step.
    pub fn interval(&self) -> Duration {
        if self.failures == 0 {
            Duration::from_secs(1)
        } else {
            Duration::from_secs(5)
        }
    }

    fn fail(&mut self) {
        self.failures = self.failures.saturating_add(1);
    }

    fn load_identity(&mut self, directory: &Path) -> Option<LocalIdentity> {
        if self.identity.is_none() {
            self.identity = load_or_create(directory, now_seconds()).ok();
        }
        self.identity.clone()
    }

    /// Ensures the reused pinned connection and local identity exist. Both
    /// are cheap to rebuild, which is exactly what happens after a failure.
    fn open(&mut self, pin: &ServerPin, directory: &Path) -> Result<(), ()> {
        if self.load_identity(directory).is_none() {
            return Err(());
        }
        if self.client.is_none() {
            // A blackholed route burns the whole dial timeout on the pump's
            // thread: after a failure, stay quiet until the cooldown lapses
            // instead of wedging every backoff tick behind a fresh dial.
            if self
                .last_connect_attempt
                .is_some_and(|at| at.elapsed() < CONNECT_RETRY_COOLDOWN)
            {
                return Err(());
            }
            self.last_connect_attempt = Some(Instant::now());
            self.client = ServerClient::connect(pin).ok();
        }
        if self.client.is_none() {
            return Err(());
        }
        Ok(())
    }

    /// One signed request/response against the reused connection. A transport
    /// failure poisons the connection (it is rebuilt on the next `open`); a
    /// server refusal arrives as a normal error variant.
    fn exchanged(
        &mut self,
        identity: &LocalIdentity,
        message_type: &str,
        payload: Value,
    ) -> Result<Value, ExchangeError> {
        let Some(client) = self.client.as_mut() else {
            return Err(ExchangeError::Transport);
        };
        let request = Envelope::request(message_type, payload, rfc3339_now());
        match client.exchange(request, identity) {
            Ok(response) if response.error.is_none() => Ok(response.payload),
            Ok(_) => Err(ExchangeError::Refused),
            Err(_) => {
                self.client = None;
                Err(ExchangeError::Transport)
            }
        }
    }

    /// Resolves every paired peer. `force` skips the cache: a user who
    /// just paired must be able to call immediately. A companion identity
    /// contributes one entry per device — that is the normal case now, not
    /// an ambiguity. Records that fail validation are skipped, never stored.
    fn resolve_peers(
        &mut self,
        identity: &LocalIdentity,
        force: bool,
    ) -> Result<Vec<PeerInfo>, PeerLookup> {
        if !force {
            if let Some(checked) = self.peer_checked_at {
                if checked.elapsed() <= PEER_CACHE_TTL && !self.peers.is_empty() {
                    return Ok(self.peers.clone());
                }
            }
        }
        let payload = self
            .exchanged(identity, "contacts.list", json!({}))
            .map_err(|_| PeerLookup::Transport)?;
        let peers = payload
            .get("peers")
            .and_then(Value::as_array)
            .ok_or(PeerLookup::Missing)?;
        let mut resolved = Vec::new();
        for entry in peers {
            let device_id = entry
                .get("device_id")
                .and_then(Value::as_str)
                .and_then(|id| Uuid::parse_str(id).ok());
            let harbor_id = entry
                .get("harbor_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let (Some(device_id), harbor_id) = (device_id, harbor_id.trim()) else {
                continue;
            };
            if harbor_id.is_empty() {
                continue;
            }
            resolved.push(PeerInfo {
                device_id,
                harbor_id: harbor_id.to_owned(),
                public_key: entry
                    .get("public_key")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            });
        }
        if resolved.is_empty() {
            return Err(PeerLookup::Missing);
        }
        self.peers = resolved.clone();
        self.peer_checked_at = Some(Instant::now());
        self.legs
            .retain(|leg| resolved.iter().any(|peer| peer.device_id == leg.peer));
        Ok(resolved)
    }

    /// One polling step: keep the peer view fresh, hold the pair session
    /// open, and drain whatever the peer signaled. All failures are quiet —
    /// the tick simply backs off and tries again.
    fn tick_inner(&mut self, state: &mut CoreState) -> SignalingTick {
        let Some(pin) = load_server_pin(&state.state_dir) else {
            return SignalingTick::Quiet;
        };
        if self.open(&pin, &state.state_dir).is_err() {
            self.fail();
            return SignalingTick::Quiet;
        }
        let Some(identity) = self.load_identity(&state.state_dir) else {
            return SignalingTick::Quiet;
        };
        let peers = match self.resolve_peers(&identity, false) {
            Ok(peers) => peers,
            Err(_) => {
                self.fail();
                return SignalingTick::Quiet;
            }
        };
        // One pair session per peer device. A companion identity holds one
        // leg per endpoint; each leg hello exchanged below teaches the core
        // which endpoint kind sits at the other end.
        for peer in &peers {
            let session = match self.exchanged(&identity, "session.connect", json!({"peer": peer.device_id})) {
                Ok(payload) => payload,
                Err(_) => {
                    self.fail();
                    return SignalingTick::Quiet;
                }
            };
            let leg = self.leg_mut(&peer.device_id);
            leg.session_id = session
                .get("session_id")
                .and_then(Value::as_str)
                .and_then(|id| Uuid::parse_str(id).ok())
                .or(leg.session_id);
        }
        self.failures = 0;
        // Presence rides this healthy tick. The local lease refreshes well
        // ahead of its 45 s expiry (and not at all in private mode); the
        // partner's lease is read only through healthy answers, because a
        // transport failure says nothing about them — it gets the same quiet
        // backoff as every other exchange, never a presence verdict.
        let now = now_seconds();
        if state.presence_publishing_enabled() && state.presence.publish_due(now) {
            let lease = state.presence.publishable_state().lease_value();
            match self.exchanged(&identity, "presence.publish", json!({ "state": lease })) {
                Ok(_) => state.presence.mark_published(now),
                Err(_) => {
                    self.fail();
                    return SignalingTick::Quiet;
                }
            }
        }
        self.presence_ticks = self.presence_ticks.wrapping_add(1);
        if self.presence_ticks % PARTNER_PRESENCE_POLL_TICKS == 0 {
            // Identity presence is the OR of its devices' healthy answers:
            // one ONLINE anywhere keeps the identity ONLINE. A tick with no
            // healthy answer at all says nothing and feeds no verdict.
            let mut reports = Vec::new();
            for peer in &peers {
                match self.exchanged(&identity, "presence.status", json!({ "peer": peer.device_id })) {
                    Ok(payload) => {
                        let reported = match payload.get("state").and_then(Value::as_str) {
                            Some("ONLINE") => Some(crate::presence::PresenceState::Online),
                            Some("IDLE") => Some(crate::presence::PresenceState::Away),
                            Some("OFFLINE") => Some(crate::presence::PresenceState::Offline),
                            _ => None,
                        };
                        reports.extend(reported);
                    }
                    Err(_) => {
                        self.fail();
                        return SignalingTick::Quiet;
                    }
                }
            }
            if !reports.is_empty() {
                let aggregate = if reports.contains(&crate::presence::PresenceState::Online) {
                    crate::presence::PresenceState::Online
                } else if reports.contains(&crate::presence::PresenceState::Away) {
                    crate::presence::PresenceState::Away
                } else {
                    crate::presence::PresenceState::Offline
                };
                if state.presence.partner.observe(aggregate, now_seconds()) {
                    state.presence_dirty = true;
                }
            }
        }
        for peer in &peers {
            let Some(session_id) = self
                .legs
                .iter()
                .find(|leg| leg.peer == peer.device_id)
                .and_then(|leg| leg.session_id)
            else {
                continue;
            };
            // Our kind, promptly: the peer routes calls and takeovers on it.
            if !self
                .legs
                .iter()
                .any(|leg| leg.peer == peer.device_id && leg.hello_sent)
            {
                let own = own_device_type(state);
                let hello = json!({
                    "call_id": peer.device_id.to_string(),
                    "signal": {
                        "type": "device_hello",
                        "device_id": self
                            .identity
                            .as_ref()
                            .map(|identity| identity.record().device_id.to_string())
                            .unwrap_or_default(),
                        "device_type": own.as_str(),
                        "harbor_id": self.cached_harbor_id().unwrap_or_default(),
                    },
                });
                if self.send_inner(&identity, session_id, &hello) {
                    self.leg_mut(&peer.device_id).hello_sent = true;
                }
            }
            let drained = match self.exchanged(
                &identity,
                "session.signal_poll",
                json!({"session_id": session_id}),
            ) {
                Ok(payload) => payload,
                Err(_) => {
                    self.fail();
                    return SignalingTick::Quiet;
                }
            };
            let Some(entries) = drained.get("signals").and_then(Value::as_array) else {
                continue;
            };
            for entry in entries {
                let Some(raw) = entry.get("signal").and_then(Value::as_str) else {
                    continue;
                };
                if let Some(tick) = self.inbound(state, raw, peer.device_id) {
                    return tick;
                }
            }
        }
        SignalingTick::Quiet
    }

    /// Routes one relayed signal from the authenticated `from_peer`.
    /// Answers and candidates feed the active worker; an offer is presented
    /// for approval while idle and answered with an explicit busy decline
    /// while not; a decline ends this side's outgoing attempt with a visible
    /// reason. A `device_hello` teaches the endpoint registry and may end a
    /// call the policy forbids, with its reason on the record.
    fn inbound(
        &mut self,
        state: &mut CoreState,
        raw: &str,
        from_peer: Uuid,
    ) -> Option<SignalingTick> {
        let parsed: Value = serde_json::from_str(raw).ok()?;
        let signal = parsed.get("signal")?;
        let offer_call_id = parsed
            .get("call_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        match signal.get("type").and_then(Value::as_str)? {
            "device_hello" => {
                self.inbound_hello(state, signal, from_peer);
                None
            }
            // A dial invitation from the listening desktop. Only a mobile
            // endpoint ever dials; anything else ignores it.
            "tcp_invite" => {
                if own_device_type(state) == DeviceType::Mobile {
                    if let Some(invite) = LinkInvite::parse(signal) {
                        state.note_link_invite(invite);
                    }
                }
                None
            }
            "offer" => {
                let from_device = parsed
                    .get("from_device")
                    .and_then(Value::as_str)
                    .and_then(|id| Uuid::parse_str(id).ok())
                    .unwrap_or(from_peer);
                let takeover = parsed.get("takeover").and_then(Value::as_bool) == Some(true)
                    && from_device == from_peer;
                // A proven Mobile<->Mobile pair never rings: the refusal is
                // explicit and localizable, not a silence.
                let known_incompatible = matches!(
                    self.leg_device(&from_peer),
                    Some(remote)
                        if compatibility(own_device_type(state), remote).is_err()
                );
                if !takeover && known_incompatible {
                    decline_remote(state, &from_peer, &offer_call_id, "incompatible");
                    return None;
                }
                if takeover
                    && state.call.call_id.is_some()
                    && self.same_identity_as_call(state, &from_peer)
                {
                    // Sibling takeover: drop this media first, then present
                    // the new offer. Never two endpoints of one identity.
                    state.shutdown_media();
                    state.call.reason = "takeover".into();
                    state.call_dirty = true;
                } else if state.call.call_id.is_some() || state.incoming.is_some() {
                    // One call at a time: a second offer (glare, or a peer who
                    // dialed while we are busy) is refused honestly with a busy
                    // decline instead of being silently swallowed.
                    decline_remote(state, &from_peer, &offer_call_id, "busy");
                    return None;
                }
                Some(SignalingTick::IncomingOffer {
                    offer_call_id,
                    sdp: signal.get("sdp").and_then(Value::as_str)?.to_owned(),
                    peer: from_peer,
                    takeover,
                })
            }
            "decline" => {
                // Only an outgoing attempt in progress can be declined; the
                // peer's decline is addressed to its own call id, which we
                // never match against our local one.
                if state.call.call_id.is_some()
                    && (state.call.phase == "CONNECTING" || state.call.phase == "OUTGOING")
                {
                    state.end_outgoing_with_reason(
                        signal
                            .get("reason")
                            .and_then(Value::as_str)
                            .unwrap_or("declined"),
                    );
                }
                None
            }
            "answer" | "candidate" => {
                let media = state.media.as_ref()?;
                let call_id = state.call.call_id.clone()?;
                let _ = media.request(
                    "call.remote_signal",
                    json!({"call_id": call_id, "signal": signal}),
                );
                None
            }
            _ => None,
        }
    }

    /// The learned kind of one peer leg, still unknown until helloed.
    fn leg_device(&self, peer: &Uuid) -> Option<DeviceType> {
        self.legs
            .iter()
            .find(|leg| &leg.peer == peer)
            .and_then(|leg| leg.device)
    }

    /// Whether `peer` belongs to the same harbor identity as the current
    /// call's peer: the takeover test. Contacts group devices by harbor id.
    fn same_identity_as_call(&self, state: &CoreState, peer: &Uuid) -> bool {
        let Some(call_peer) = state.call.peer.as_ref() else {
            return false;
        };
        match (self.peer_harbor(call_peer), self.peer_harbor(peer)) {
            (Some(first), Some(second)) => first == second,
            _ => false,
        }
    }

    /// Absorbs a `device_hello`. The claim must name the authenticated
    /// sender and the paired harbor record; anything else is dropped, never
    /// stored. A newly learned kind re-renders the endpoint snapshot, and a
    /// hello that proves Mobile<->Mobile ends the local call with its
    /// reason on the record.
    fn inbound_hello(&mut self, state: &mut CoreState, signal: &Value, from_peer: Uuid) {
        let claimed_id = signal
            .get("device_id")
            .and_then(Value::as_str)
            .and_then(|id| Uuid::parse_str(id).ok());
        let claimed_type = signal
            .get("device_type")
            .and_then(Value::as_str)
            .and_then(DeviceType::parse);
        let claimed_harbor = signal.get("harbor_id").and_then(Value::as_str).unwrap_or_default();
        if claimed_id != Some(from_peer) {
            return;
        }
        if claimed_harbor.is_empty() || Some(claimed_harbor) != self.peer_harbor(&from_peer) {
            return;
        }
        let Some(device) = claimed_type else {
            return;
        };
        if self.note_peer_device(&from_peer, device) {
            state.device_dirty = true;
        }
        // A mobile peer means bearer duty for a desktop: listen once, then
        // invite it onto the direct TCP path.
        if device == DeviceType::Mobile {
            state.start_link_listener(&from_peer);
        }
        if compatibility(own_device_type(state), device).is_err()
            && state.call.peer == Some(from_peer)
        {
            if state.call.phase == "CONNECTED" {
                state.shutdown_media();
            } else {
                state.end_outgoing_with_reason("incompatible");
            }
            state.call.reason = "incompatible".into();
            state.call_dirty = true;
        }
    }

    fn prepare_inner(&mut self, state: &mut CoreState) -> Result<Uuid, ProtocolError> {
        let Some(pin) = load_server_pin(&state.state_dir) else {
            return Err(ProtocolError {
                code: "server_unconfigured".into(),
                ui_key: "error.server.unconfigured".into(),
                retryable: false,
                detail: "No control-plane server is configured for calls".into(),
            });
        };
        if self.open(&pin, &state.state_dir).is_err() {
            return Err(server_unavailable_error());
        }
        let Some(identity) = self.load_identity(&state.state_dir) else {
            return Err(identity_unavailable_error());
        };
        self.resolve_peers(&identity, true).map_err(|lookup| match lookup {
            PeerLookup::Transport => server_unavailable_error(),
            PeerLookup::Missing => call_error("no_peer", "error.call.noPeer", false),
        })?;
        let own = own_device_type(state);
        let Some(target) = self.preferred_peer(own) else {
            // Every known peer is a proven Mobile while we are one: the
            // session between the identities stands, media does not.
            return Err(call_error(
                "mobile_to_mobile",
                "error.device.mobileToMobile",
                false,
            ));
        };
        let session = self
            .exchanged(&identity, "session.connect", json!({"peer": target}))
            .map_err(|error| match error {
                ExchangeError::Transport => server_unavailable_error(),
                ExchangeError::Refused => call_error("no_peer", "error.call.noPeer", false),
            })?;
        let leg = self.leg_mut(&target);
        leg.session_id = session
            .get("session_id")
            .and_then(Value::as_str)
            .and_then(|id| Uuid::parse_str(id).ok());
        if leg.session_id.is_none() {
            return Err(call_error("no_peer", "error.call.noPeer", false));
        }
        // Our kind rides out with the session, not a tick later: the peer
        // routes this very call on it.
        let session_id = leg.session_id;
        let hello_due = !leg.hello_sent;
        if hello_due {
            let hello = json!({
                "call_id": target.to_string(),
                "signal": {
                    "type": "device_hello",
                    "device_id": identity.record().device_id.to_string(),
                    "device_type": own.as_str(),
                    "harbor_id": identity.record().harbor_id.clone(),
                },
            });
            if let Some(session_id) = session_id {
                if self.send_inner(&identity, session_id, &hello) {
                    self.leg_mut(&target).hello_sent = true;
                }
            }
        }
        Ok(target)
    }

    fn send_inner(&mut self, identity: &LocalIdentity, session_id: Uuid, signal: &Value) -> bool {
        let body = signal.to_string();
        if body.is_empty() || body.len() > MAX_RELAY_SIGNAL_BYTES {
            return false;
        }
        self.exchanged(
            identity,
            "session.signal",
            json!({"session_id": session_id, "signal": body}),
        )
        .is_ok()
    }

    /// The local identity's harbor_id when one is loaded. Activity delivery
    /// labels frames with it without touching storage again.
    pub fn cached_harbor_id(&self) -> Option<String> {
        self.identity
            .as_ref()
            .map(|identity| identity.record().harbor_id.clone())
    }

    /// Stores an identity loaded elsewhere (tests seed the label cache this
    /// way) so the cache stays the single owner of the loaded identity.
    #[cfg(test)]
    pub fn cache_identity(&mut self, identity: LocalIdentity) {
        self.identity = Some(identity);
    }
}

enum ExchangeError {
    Transport,
    Refused,
}

enum PeerLookup {
    Transport,
    Missing,
}

pub(crate) fn server_unavailable_error() -> ProtocolError {
    ProtocolError {
        code: "server_unavailable".into(),
        ui_key: "error.server.unavailable".into(),
        retryable: true,
        detail: "The control-plane server is unreachable".into(),
    }
}

/// Runs one polling step; the take/restore dance keeps the borrow checker
/// happy about `state.signaling` being driven from inside a free function.
pub fn tick(state: &mut CoreState) -> SignalingTick {
    let mut signaling = std::mem::take(&mut state.signaling);
    let outcome = signaling.tick_inner(state);
    state.signaling = signaling;
    outcome
}

/// As the core exits, hand the server one honest OFFLINE lease so the
/// partner reads an immediate departure instead of a lease decaying over
/// its remaining life. Best effort by design: reusing the live connection
/// when there is one, never blocking shutdown on transport trouble.
pub fn publish_shutdown_offline(state: &mut CoreState) {
    let mut signaling = std::mem::take(&mut state.signaling);
    let _ = (|| {
        if !state.presence_publishing_enabled() {
            return Err(());
        }
        let identity = signaling.load_identity(&state.state_dir).ok_or(())?;
        let now = now_seconds();
        if state.presence.local.mark_offline(now) {
            state.presence_dirty = true;
        }
        signaling
            .exchanged(&identity, "presence.publish", json!({"state": "OFFLINE"}))
            .map_err(|_| ())?;
        Ok(())
    })();
    state.signaling = signaling;
}

/// Caller side of a new call, step one: resolve the paired peers, pick the
/// preferred target, and hold the pair session before any media resource is
/// touched. None of this needs the offer, so an unconfigured or unreachable
/// server refuses a call before a worker spawns or a microphone opens. Every
/// failure is structured and localizable — never a half-open call. Returns
/// the chosen peer: the call is addressed to a device, not an identity.
pub fn prepare(state: &mut CoreState) -> Result<Uuid, ProtocolError> {
    let mut signaling = std::mem::take(&mut state.signaling);
    let result = signaling.prepare_inner(state);
    state.signaling = signaling;
    result
}

/// Caller side, step two: relay the worker's offer to one peer's session,
/// the one `prepare` established.
pub fn dial_to(state: &mut CoreState, peer: &Uuid, signal: &Value) -> Result<(), ProtocolError> {
    let mut signaling = std::mem::take(&mut state.signaling);
    let result = (|| {
        let Some(identity) = signaling.load_identity(&state.state_dir) else {
            return Err(identity_unavailable_error());
        };
        let Some(session_id) = signaling
            .legs
            .iter()
            .find(|leg| &leg.peer == peer)
            .and_then(|leg| leg.session_id)
        else {
            return Err(call_error("no_peer", "error.call.noPeer", false));
        };
        if !signaling.send_inner(&identity, session_id, signal) {
            return Err(server_unavailable_error());
        }
        Ok(())
    })();
    state.signaling = signaling;
    result
}

/// Queues one opaque signal for one peer's session. Returns false on any
/// transport problem; callers treat relay hiccups according to their own
/// stakes.
pub fn send_to(state: &mut CoreState, peer: &Uuid, signal: &Value) -> bool {
    let mut signaling = std::mem::take(&mut state.signaling);
    let Some(identity) = signaling.load_identity(&state.state_dir) else {
        state.signaling = signaling;
        return false;
    };
    let sent = signaling
        .legs
        .iter()
        .find(|leg| &leg.peer == peer)
        .and_then(|leg| leg.session_id)
        .is_some_and(|session_id| signaling.send_inner(&identity, session_id, signal));
    state.signaling = signaling;
    sent
}

/// Sends an explicit decline for a caller's offer on its own leg. The signal
/// string is opaque to the server, so no protocol change is needed to carry
/// it; the caller's core translates it into an ended attempt with a reason.
pub fn decline_remote(state: &mut CoreState, peer: &Uuid, remote_call_id: &str, reason: &str) {
    send_to(
        state,
        peer,
        &json!({
            "call_id": remote_call_id,
            "signal": {"type": "decline", "reason": reason},
        }),
    );
}

/// Best-effort session teardown across every leg: the server drops any
/// still-queued signals, so a finished call never leaks stale SDP into the
/// next one.
pub fn hangup(state: &mut CoreState) {
    let mut signaling = std::mem::take(&mut state.signaling);
    let sessions: Vec<Uuid> = signaling
        .legs
        .iter_mut()
        .filter_map(|leg| leg.session_id.take())
        .collect();
    if sessions.is_empty() {
        state.signaling = signaling;
        return;
    }
    if let Some(identity) = signaling.load_identity(&state.state_dir) {
        for session_id in sessions {
            let _ = signaling.exchanged(
                &identity,
                "session.disconnect",
                json!({"session_id": session_id}),
            );
        }
    }
    state.signaling = signaling;
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::CoreState;

    /// A signaling test's transport is dead by construction, so the relay
    /// legs of these scenarios quietly no-op; what is under test is the
    /// state routing, not the wire.
    fn state() -> CoreState {
        let directory =
            std::env::temp_dir().join(format!("harbor-signaling-test-{}", Uuid::new_v4()));
        CoreState::for_directory(&directory)
    }

    fn relayed(call_id: &str, signal: Value) -> String {
        json!({"call_id": call_id, "signal": signal}).to_string()
    }

    #[test]
    fn an_offer_while_idle_becomes_an_incoming_offer_tick() {
        let mut core_state = state();
        let mut signaling = Signaling::default();
        let tick = signaling.inbound(
            &mut core_state,
            &relayed("remote-call", json!({"type": "offer", "sdp": "v=0"})),
            Uuid::new_v4(),
        );
        match tick {
            Some(SignalingTick::IncomingOffer {
                offer_call_id,
                sdp,
                ..
            }) => {
                assert_eq!(offer_call_id, "remote-call");
                assert_eq!(sdp, "v=0");
            }
            _ => panic!("an idle peer must surface the offer for approval"),
        }
        std::fs::remove_dir_all(&core_state.state_dir).ok();
    }

    #[test]
    fn a_second_offer_while_a_call_is_live_is_refused_not_swallowed() {
        let mut core_state = state();
        core_state.call.call_id = Some("local-call".into());
        core_state.call.phase = "CONNECTED".into();
        let mut signaling = Signaling::default();
        let tick = signaling.inbound(
            &mut core_state,
            &relayed("other-call", json!({"type": "offer", "sdp": "v=0"})),
            Uuid::new_v4(),
        );
        assert!(tick.is_none(), "a busy peer never rings");
        assert!(core_state.incoming.is_none());
        assert_eq!(core_state.call.call_id.as_deref(), Some("local-call"));
        assert_eq!(core_state.call.phase, "CONNECTED");
        std::fs::remove_dir_all(&core_state.state_dir).ok();
    }

    #[test]
    fn a_second_offer_while_one_rings_is_refused_and_the_first_stays() {
        let mut core_state = state();
        core_state.present_incoming("first", "v=0", Uuid::new_v4());
        let mut signaling = Signaling::default();
        let tick = signaling.inbound(
            &mut core_state,
            &relayed("second", json!({"type": "offer", "sdp": "v=0"})),
            Uuid::new_v4(),
        );
        assert!(tick.is_none());
        let incoming = core_state.incoming.as_ref().expect("the first offer stays");
        assert_eq!(incoming.remote_call_id, "first");
        assert_eq!(core_state.call.phase, "INCOMING");
        std::fs::remove_dir_all(&core_state.state_dir).ok();
    }

    #[test]
    fn a_peer_decline_ends_the_outgoing_attempt_with_its_reason() {
        let mut core_state = state();
        core_state.call.call_id = Some("local-call".into());
        core_state.call.phase = "OUTGOING".into();
        let mut signaling = Signaling::default();
        let tick = signaling.inbound(
            &mut core_state,
            &relayed(
                "remote-call",
                json!({"type": "decline", "reason": "declined"}),
            ),
            Uuid::new_v4(),
        );
        assert!(tick.is_none());
        assert!(core_state.call.call_id.is_none());
        assert_eq!(core_state.call.phase, "ENDED");
        assert_eq!(core_state.call.reason, "declined");
        std::fs::remove_dir_all(&core_state.state_dir).ok();
    }

    fn seed_peer(signaling: &mut Signaling, device_id: Uuid, harbor: &str) {
        signaling.peers.push(PeerInfo {
            device_id,
            harbor_id: harbor.into(),
            public_key: String::new(),
        });
        signaling.leg_mut(&device_id);
    }

    fn hello_from(device_id: Uuid, device_type: &str, harbor: &str) -> String {
        json!({
            "call_id": device_id.to_string(),
            "signal": {
                "type": "device_hello",
                "device_id": device_id.to_string(),
                "device_type": device_type,
                "harbor_id": harbor,
            },
        })
        .to_string()
    }

    #[test]
    fn a_validated_hello_learns_the_peer_kind() {
        let mut core_state = state();
        let mut signaling = Signaling::default();
        let peer = Uuid::new_v4();
        seed_peer(&mut signaling, peer, "harbor-abc");
        let tick = signaling.inbound(&mut core_state, &hello_from(peer, "mobile", "harbor-abc"), peer);
        assert!(tick.is_none());
        assert_eq!(signaling.leg_device(&peer), Some(DeviceType::Mobile));
        assert!(core_state.device_dirty);
        std::fs::remove_dir_all(&core_state.state_dir).ok();
    }

    #[test]
    fn a_spoofed_hello_is_dropped_never_stored() {
        let mut core_state = state();
        let mut signaling = Signaling::default();
        let peer = Uuid::new_v4();
        let stranger = Uuid::new_v4();
        seed_peer(&mut signaling, peer, "harbor-abc");
        // Wrong sender id.
        let tick = signaling.inbound(
            &mut core_state,
            &hello_from(stranger, "mobile", "harbor-abc"),
            peer,
        );
        assert!(tick.is_none());
        // Wrong harbor record.
        let tick = signaling.inbound(
            &mut core_state,
            &hello_from(peer, "mobile", "harbor-evil"),
            peer,
        );
        assert!(tick.is_none());
        // Unknown device kind.
        let tick = signaling.inbound(
            &mut core_state,
            &hello_from(peer, "tablet", "harbor-abc"),
            peer,
        );
        assert!(tick.is_none());
        assert_eq!(signaling.leg_device(&peer), None);
        assert!(!core_state.device_dirty);
        std::fs::remove_dir_all(&core_state.state_dir).ok();
    }

    #[test]
    fn a_proven_mobile_to_mobile_offer_is_declined_incompatible() {
        let mut core_state = state();
        core_state
            .settings
            .as_mut()
            .unwrap()
            .update(&serde_json::json!({"deviceType": "mobile"}))
            .unwrap();
        let mut signaling = Signaling::default();
        let peer = Uuid::new_v4();
        seed_peer(&mut signaling, peer, "harbor-abc");
        assert!(signaling.note_peer_device(&peer, DeviceType::Mobile));
        let tick = signaling.inbound(
            &mut core_state,
            &relayed("remote-call", json!({"type": "offer", "sdp": "v=0"})),
            peer,
        );
        assert!(tick.is_none(), "a mobile never rings for a mobile");
        assert!(core_state.incoming.is_none());
        std::fs::remove_dir_all(&core_state.state_dir).ok();
    }

    #[test]
    fn a_sibling_takeover_drops_local_media_then_rings() {
        let mut core_state = state();
        let sibling = Uuid::new_v4();
        let taylor = Uuid::new_v4();
        // This core is in a call with Taylor's desktop...
        core_state.call.call_id = Some("local-call".into());
        core_state.call.phase = "CONNECTED".into();
        core_state.call.peer = Some(taylor);
        let mut signaling = Signaling::default();
        // ...and both Taylor endpoints belong to one harbor identity.
        seed_peer(&mut signaling, taylor, "harbor-taylor");
        seed_peer(&mut signaling, sibling, "harbor-taylor");
        let offer = json!({
            "call_id": "sibling-call",
            "takeover": true,
            "from_device": sibling.to_string(),
            "signal": {"type": "offer", "sdp": "v=0"},
        })
        .to_string();
        let tick = signaling.inbound(&mut core_state, &offer, sibling);
        match tick {
            Some(SignalingTick::IncomingOffer { takeover, .. }) => assert!(takeover),
            _ => panic!("a sibling takeover must ring after dropping local media"),
        }
        assert!(core_state.call.call_id.is_none());
        assert_eq!(core_state.call.reason, "takeover");
        std::fs::remove_dir_all(&core_state.state_dir).ok();
    }

    #[test]
    fn a_takeover_from_another_identity_is_just_busy() {
        let mut core_state = state();
        core_state.call.call_id = Some("local-call".into());
        core_state.call.phase = "CONNECTED".into();
        core_state.call.peer = Some(Uuid::new_v4());
        let mut signaling = Signaling::default();
        let stranger = Uuid::new_v4();
        seed_peer(&mut signaling, stranger, "harbor-stranger");
        let offer = json!({
            "call_id": "stranger-call",
            "takeover": true,
            "from_device": stranger.to_string(),
            "signal": {"type": "offer", "sdp": "v=0"},
        })
        .to_string();
        let tick = signaling.inbound(&mut core_state, &offer, stranger);
        assert!(tick.is_none());
        assert_eq!(core_state.call.call_id.as_deref(), Some("local-call"));
        std::fs::remove_dir_all(&core_state.state_dir).ok();
    }

    #[test]
    fn a_mobile_core_dials_a_validated_invite_and_desktops_ignore_it() {
        let mut core_state = state();
        core_state
            .settings
            .as_mut()
            .unwrap()
            .update(&serde_json::json!({"deviceType": "mobile"}))
            .unwrap();
        let peer = Uuid::new_v4();
        let invite = json!({
            "call_id": peer.to_string(),
            "signal": {
                "type": "tcp_invite",
                "addrs": ["127.0.0.1"],
                "port": 9,
                "fingerprint": "ab".repeat(32),
            },
        })
        .to_string();
        let tick = signaling_inbound(&mut core_state, &invite, peer);
        assert!(tick.is_none());
        assert!(
            core_state.link_invite.is_some(),
            "a validated invite dials"
        );
        // A desktop never dials out on the bearer.
        let mut desktop = state();
        let tick = signaling_inbound(&mut desktop, &invite, peer);
        assert!(tick.is_none());
        assert!(desktop.link_invite.is_none());
        // Malformed invites change nothing anywhere.
        let junk = json!({
            "call_id": peer.to_string(),
            "signal": {"type": "tcp_invite", "addrs": [], "port": 0},
        })
        .to_string();
        let tick = signaling_inbound(&mut core_state, &junk, peer);
        assert!(tick.is_none());
        std::fs::remove_dir_all(&core_state.state_dir).ok();
        std::fs::remove_dir_all(&desktop.state_dir).ok();
    }

    fn signaling_inbound(
        core_state: &mut CoreState,
        raw: &str,
        peer: Uuid,
    ) -> Option<SignalingTick> {
        Signaling::default().inbound(core_state, raw, peer)
    }

    #[test]
    fn preferred_peer_skips_proven_mobile_pairs() {
        let mut signaling = Signaling::default();
        let mobile_peer = Uuid::new_v4();
        let desktop_peer = Uuid::new_v4();
        seed_peer(&mut signaling, mobile_peer, "harbor-a");
        seed_peer(&mut signaling, desktop_peer, "harbor-b");
        signaling.note_peer_device(&mobile_peer, DeviceType::Mobile);
        // Unknown stays dialable; proven mobile is skipped for mobiles.
        assert_eq!(
            signaling.preferred_peer(DeviceType::Mobile),
            Some(desktop_peer)
        );
        signaling.note_peer_device(&desktop_peer, DeviceType::Mobile);
        assert_eq!(signaling.preferred_peer(DeviceType::Mobile), None);
        assert_eq!(
            signaling.preferred_peer(DeviceType::Desktop),
            Some(mobile_peer)
        );
        std::fs::remove_dir_all(&std::env::temp_dir().join("harbor-unused")).ok();
    }

    #[test]
    fn a_peer_decline_cannot_touch_an_established_call() {
        let mut core_state = state();
        core_state.call.call_id = Some("local-call".into());
        core_state.call.phase = "CONNECTED".into();
        let mut signaling = Signaling::default();
        let tick = signaling.inbound(
            &mut core_state,
            &relayed(
                "remote-call",
                json!({"type": "decline", "reason": "declined"}),
            ),
            Uuid::new_v4(),
        );
        assert!(tick.is_none());
        assert_eq!(core_state.call.call_id.as_deref(), Some("local-call"));
        assert_eq!(core_state.call.phase, "CONNECTED");
        std::fs::remove_dir_all(&core_state.state_dir).ok();
    }
}
