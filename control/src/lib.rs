//! Authoritative, transport-independent control-plane state.
//!
//! This crate deliberately contains no listener, media path, or file/message
//! payload. A deployed server must authenticate the caller before invoking these
//! transitions; unverified network input must never select an identity here.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

pub const PAIRING_CODE_LENGTH: usize = 6;
pub const PAIRING_TTL_SECONDS: u64 = 5 * 60;
pub const PRESENCE_LEASE_SECONDS: u64 = 45;
pub const MAX_SIGNAL_BYTES: usize = 64 * 1024;
/// Bounded routing queue per logical session. Signaling is relay-only and
/// short-lived: a full queue refuses new signals instead of dropping older
/// ones, because SDP ordering (offer before its candidates) is meaningful.
pub const MAX_SESSION_SIGNALS: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityRecord {
    pub device_id: Uuid,
    pub harbor_id: String,
    pub public_key: String,
    pub registered_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PairingState {
    PendingCode,
    WaitingApproval,
    Accepted,
    Declined,
    Cancelled,
    Expired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairingSnapshot {
    pub pairing_id: Uuid,
    pub requester: Option<Uuid>,
    pub target: Uuid,
    pub state: PairingState,
    pub expires_at: u64,
}

#[derive(Debug, Clone)]
struct PairingRequest {
    snapshot: PairingSnapshot,
    code: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Presence {
    Online,
    Idle,
    Offline,
}

#[derive(Debug, Clone, Copy)]
struct PresenceLease {
    state: Presence,
    expires_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SessionState {
    Connecting,
    Connected,
    Reconnecting,
    Disconnected,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: Uuid,
    pub first_peer: Uuid,
    pub second_peer: Uuid,
    pub state: SessionState,
    pub updated_at: u64,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ControlError {
    #[error("identity is not registered")]
    UnknownIdentity,
    #[error("the pairing code must be exactly six ASCII digits")]
    InvalidPairingCode,
    #[error("a pairing code is already active")]
    DuplicatePairingCode,
    #[error("a device cannot pair with itself")]
    SelfPairing,
    #[error("pairing request was not found")]
    UnknownPairing,
    #[error("pairing request is no longer active")]
    InactivePairing,
    #[error("caller is not authorized for this operation")]
    Unauthorized,
    #[error("the peers are not paired")]
    PeersNotPaired,
    #[error("session was not found")]
    UnknownSession,
    #[error("session signal exceeds {MAX_SIGNAL_BYTES} bytes")]
    OversizedSignal,
    #[error("the session signal queue is full")]
    SignalQueueFull,
    #[error("identity fields are invalid")]
    InvalidIdentity,
}

/// One relayed signaling message awaiting its recipient. The signal body is
/// opaque to the control plane: it is routed, sized, and dropped — never
/// parsed, interpreted, or stored beyond the routing queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedSignal {
    pub from_peer: Uuid,
    pub signal: String,
    pub enqueued_at: u64,
}

/// Durable subset of the control-plane state.
///
/// Pending pairings, presence leases, and logical sessions are intentionally
/// excluded: they are transient, so a restarted server must re-establish them
/// instead of resurrecting stale secrets or leases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlSnapshot {
    pub identities: Vec<IdentityRecord>,
    pub relationships: Vec<(Uuid, Uuid)>,
}

#[derive(Debug, Default)]
pub struct ControlPlane {
    identities: BTreeMap<Uuid, IdentityRecord>,
    pairings: BTreeMap<Uuid, PairingRequest>,
    relationships: BTreeSet<(Uuid, Uuid)>,
    presence: BTreeMap<Uuid, PresenceLease>,
    sessions: BTreeMap<(Uuid, Uuid), SessionRecord>,
    /// Transient relay queues keyed by session id; a disconnected session
    /// drops its queue. Never part of the durable snapshot.
    signals: BTreeMap<Uuid, Vec<QueuedSignal>>,
}

impl ControlPlane {
    pub fn register_identity(
        &mut self,
        device_id: Uuid,
        harbor_id: String,
        public_key: String,
        now: u64,
    ) -> Result<IdentityRecord, ControlError> {
        if harbor_id.trim().is_empty()
            || harbor_id.len() > 64
            || public_key.trim().is_empty()
            || public_key.len() > 4096
        {
            return Err(ControlError::InvalidIdentity);
        }

        let identity = IdentityRecord {
            device_id,
            harbor_id,
            public_key,
            registered_at: now,
        };
        self.identities.insert(device_id, identity.clone());
        self.presence.insert(
            device_id,
            PresenceLease {
                state: Presence::Offline,
                expires_at: now,
            },
        );
        Ok(identity)
    }

    pub fn identity(&self, device_id: Uuid) -> Option<&IdentityRecord> {
        self.identities.get(&device_id)
    }

    /// Exports the durable state for persistence. Pending pairings, presence
    /// leases, and sessions are dropped by design.
    pub fn snapshot(&self) -> ControlSnapshot {
        ControlSnapshot {
            identities: self.identities.values().cloned().collect(),
            relationships: self.relationships.iter().copied().collect(),
        }
    }

    /// Rebuilds a control plane from durable state. Relationships that no
    /// longer have two registered endpoints are discarded instead of trusted.
    pub fn restore(snapshot: ControlSnapshot) -> Self {
        let mut control = Self::default();
        for identity in snapshot.identities {
            let _ = control.register_identity(
                identity.device_id,
                identity.harbor_id,
                identity.public_key,
                identity.registered_at,
            );
        }
        for (first, second) in snapshot.relationships {
            if control.identities.contains_key(&first) && control.identities.contains_key(&second) {
                control.relationships.insert(peer_key(first, second));
            }
        }
        control
    }

    pub fn create_pairing(
        &mut self,
        target: Uuid,
        code: String,
        now: u64,
    ) -> Result<PairingSnapshot, ControlError> {
        self.expire(now);
        self.require_identity(target)?;
        if !is_pairing_code(&code) {
            return Err(ControlError::InvalidPairingCode);
        }
        if self.pairings.values().any(|request| {
            request.code == code
                && matches!(
                    request.snapshot.state,
                    PairingState::PendingCode | PairingState::WaitingApproval
                )
        }) {
            return Err(ControlError::DuplicatePairingCode);
        }

        let pairing_id = Uuid::new_v4();
        let snapshot = PairingSnapshot {
            pairing_id,
            requester: None,
            target,
            state: PairingState::PendingCode,
            expires_at: now + PAIRING_TTL_SECONDS,
        };
        self.pairings.insert(
            pairing_id,
            PairingRequest {
                snapshot: snapshot.clone(),
                code,
            },
        );
        Ok(snapshot)
    }

    pub fn submit_pairing(
        &mut self,
        requester: Uuid,
        code: &str,
        now: u64,
    ) -> Result<PairingSnapshot, ControlError> {
        self.expire(now);
        self.require_identity(requester)?;
        let request = self
            .pairings
            .values_mut()
            .find(|request| {
                request.code == code && request.snapshot.state == PairingState::PendingCode
            })
            .ok_or(ControlError::UnknownPairing)?;
        if request.snapshot.target == requester {
            return Err(ControlError::SelfPairing);
        }

        request.snapshot.requester = Some(requester);
        request.snapshot.state = PairingState::WaitingApproval;
        Ok(request.snapshot.clone())
    }

    pub fn incoming_pairings(
        &mut self,
        target: Uuid,
        now: u64,
    ) -> Result<Vec<PairingSnapshot>, ControlError> {
        self.expire(now);
        self.require_identity(target)?;
        Ok(self
            .pairings
            .values()
            .filter(|request| {
                request.snapshot.target == target
                    && request.snapshot.state == PairingState::WaitingApproval
            })
            .map(|request| request.snapshot.clone())
            .collect())
    }

    pub fn accept_pairing(
        &mut self,
        target: Uuid,
        pairing_id: Uuid,
        now: u64,
    ) -> Result<PairingSnapshot, ControlError> {
        self.expire(now);
        let request = self
            .pairings
            .get_mut(&pairing_id)
            .ok_or(ControlError::UnknownPairing)?;
        if request.snapshot.target != target {
            return Err(ControlError::Unauthorized);
        }
        if request.snapshot.state != PairingState::WaitingApproval {
            return Err(ControlError::InactivePairing);
        }
        let requester = request
            .snapshot
            .requester
            .ok_or(ControlError::InactivePairing)?;
        request.snapshot.state = PairingState::Accepted;
        self.relationships.insert(peer_key(requester, target));
        Ok(request.snapshot.clone())
    }

    pub fn decline_pairing(
        &mut self,
        target: Uuid,
        pairing_id: Uuid,
        now: u64,
    ) -> Result<PairingSnapshot, ControlError> {
        self.expire(now);
        let request = self
            .pairings
            .get_mut(&pairing_id)
            .ok_or(ControlError::UnknownPairing)?;
        if request.snapshot.target != target {
            return Err(ControlError::Unauthorized);
        }
        if request.snapshot.state != PairingState::WaitingApproval {
            return Err(ControlError::InactivePairing);
        }
        request.snapshot.state = PairingState::Declined;
        Ok(request.snapshot.clone())
    }

    pub fn cancel_pairing(
        &mut self,
        requester: Uuid,
        pairing_id: Uuid,
        now: u64,
    ) -> Result<PairingSnapshot, ControlError> {
        self.expire(now);
        let request = self
            .pairings
            .get_mut(&pairing_id)
            .ok_or(ControlError::UnknownPairing)?;
        if request.snapshot.requester != Some(requester) {
            return Err(ControlError::Unauthorized);
        }
        if !matches!(
            request.snapshot.state,
            PairingState::PendingCode | PairingState::WaitingApproval
        ) {
            return Err(ControlError::InactivePairing);
        }
        request.snapshot.state = PairingState::Cancelled;
        Ok(request.snapshot.clone())
    }

    /// Read-only pairing lookup for either endpoint. The requester needs this
    /// to observe the host's accept/decline; expiry is reflected in the
    /// returned snapshot without mutating in-memory state.
    pub fn pairing_status(
        &self,
        caller: Uuid,
        pairing_id: Uuid,
        now: u64,
    ) -> Result<PairingSnapshot, ControlError> {
        let request = self
            .pairings
            .get(&pairing_id)
            .ok_or(ControlError::UnknownPairing)?;
        if request.snapshot.target != caller && request.snapshot.requester != Some(caller) {
            return Err(ControlError::Unauthorized);
        }
        let mut snapshot = request.snapshot.clone();
        if now >= snapshot.expires_at {
            snapshot.state = PairingState::Expired;
        }
        Ok(snapshot)
    }

    pub fn publish_presence(
        &mut self,
        device_id: Uuid,
        state: Presence,
        now: u64,
    ) -> Result<(), ControlError> {
        self.require_identity(device_id)?;
        let expires_at = if state == Presence::Offline {
            now
        } else {
            now + PRESENCE_LEASE_SECONDS
        };
        self.presence
            .insert(device_id, PresenceLease { state, expires_at });
        Ok(())
    }

    pub fn presence_of(
        &self,
        observer: Uuid,
        target: Uuid,
        now: u64,
    ) -> Result<Presence, ControlError> {
        self.require_identity(observer)?;
        self.require_identity(target)?;
        if !self.relationships.contains(&peer_key(observer, target)) {
            return Err(ControlError::PeersNotPaired);
        }
        Ok(self
            .presence
            .get(&target)
            .filter(|lease| lease.expires_at > now)
            .map_or(Presence::Offline, |lease| lease.state))
    }

    pub fn connect_session(
        &mut self,
        first_peer: Uuid,
        second_peer: Uuid,
        now: u64,
    ) -> Result<SessionRecord, ControlError> {
        self.require_identity(first_peer)?;
        self.require_identity(second_peer)?;
        let key = peer_key(first_peer, second_peer);
        if !self.relationships.contains(&key) {
            return Err(ControlError::PeersNotPaired);
        }

        let session = self.sessions.entry(key).or_insert_with(|| SessionRecord {
            session_id: Uuid::new_v4(),
            first_peer: key.0,
            second_peer: key.1,
            state: SessionState::Connecting,
            updated_at: now,
        });
        session.state = SessionState::Connected;
        session.updated_at = now;
        Ok(session.clone())
    }

    pub fn disconnect_session(
        &mut self,
        peer: Uuid,
        session_id: Uuid,
        now: u64,
    ) -> Result<SessionRecord, ControlError> {
        let session = {
            let session = self.session_mut(peer, session_id)?;
            session.state = SessionState::Disconnected;
            session.updated_at = now;
            session.clone()
        };
        // Routing is live-only: an inactive session carries no pending
        // signaling into whatever flow reconnects it later.
        self.signals.remove(&session.session_id);
        Ok(session)
    }

    /// Queues one opaque signaling message for the session's other peer.
    /// Membership, liveness, and size are validated before anything is
    /// stored; the queue is bounded and refuses overflow rather than
    /// reordering SDP material.
    pub fn queue_signal(
        &mut self,
        peer: Uuid,
        session_id: Uuid,
        signal: &str,
        now: u64,
    ) -> Result<(), ControlError> {
        self.validate_signal(peer, session_id, signal)?;
        let queue = self.signals.entry(session_id).or_default();
        if queue.len() >= MAX_SESSION_SIGNALS {
            return Err(ControlError::SignalQueueFull);
        }
        queue.push(QueuedSignal {
            from_peer: peer,
            signal: signal.to_owned(),
            enqueued_at: now,
        });
        Ok(())
    }

    /// Drains the signaling messages the session's other peer left for
    /// `peer`. The caller's own queued signals are theirs and stay untouched.
    pub fn drain_signals(
        &mut self,
        peer: Uuid,
        session_id: Uuid,
    ) -> Result<Vec<QueuedSignal>, ControlError> {
        let session = self
            .sessions
            .values()
            .find(|session| session.session_id == session_id)
            .ok_or(ControlError::UnknownSession)?;
        if session.first_peer != peer && session.second_peer != peer {
            return Err(ControlError::Unauthorized);
        }
        let Some(queue) = self.signals.get_mut(&session.session_id) else {
            return Ok(Vec::new());
        };
        let inbound: Vec<QueuedSignal> = queue
            .iter()
            .filter(|entry| entry.from_peer != peer)
            .cloned()
            .collect();
        queue.retain(|entry| entry.from_peer == peer);
        Ok(inbound)
    }

    /// The registered devices this device is paired with. Call targets are a
    /// control-plane fact; the core never guesses peer identities.
    pub fn paired_peers(&self, observer: Uuid) -> Result<Vec<IdentityRecord>, ControlError> {
        self.require_identity(observer)?;
        Ok(self
            .relationships
            .iter()
            .filter(|(first, second)| *first == observer || *second == observer)
            .map(|(first, second)| if *first == observer { *second } else { *first })
            .filter_map(|peer| self.identities.get(&peer).cloned())
            .collect())
    }

    pub fn validate_signal(
        &self,
        peer: Uuid,
        session_id: Uuid,
        signal: &str,
    ) -> Result<(), ControlError> {
        if signal.len() > MAX_SIGNAL_BYTES {
            return Err(ControlError::OversizedSignal);
        }
        let session = self
            .sessions
            .values()
            .find(|session| session.session_id == session_id)
            .ok_or(ControlError::UnknownSession)?;
        if session.first_peer != peer && session.second_peer != peer {
            return Err(ControlError::Unauthorized);
        }
        if session.state == SessionState::Disconnected {
            return Err(ControlError::InactivePairing);
        }
        Ok(())
    }

    pub fn expire(&mut self, now: u64) {
        for request in self.pairings.values_mut() {
            if matches!(
                request.snapshot.state,
                PairingState::PendingCode | PairingState::WaitingApproval
            ) && request.snapshot.expires_at <= now
            {
                request.snapshot.state = PairingState::Expired;
            }
        }
    }

    fn require_identity(&self, device_id: Uuid) -> Result<(), ControlError> {
        self.identities
            .contains_key(&device_id)
            .then_some(())
            .ok_or(ControlError::UnknownIdentity)
    }

    fn session_mut(
        &mut self,
        peer: Uuid,
        session_id: Uuid,
    ) -> Result<&mut SessionRecord, ControlError> {
        let session = self
            .sessions
            .values_mut()
            .find(|session| session.session_id == session_id)
            .ok_or(ControlError::UnknownSession)?;
        if session.first_peer != peer && session.second_peer != peer {
            return Err(ControlError::Unauthorized);
        }
        Ok(session)
    }
}

fn is_pairing_code(code: &str) -> bool {
    code.len() == PAIRING_CODE_LENGTH && code.bytes().all(|byte| byte.is_ascii_digit())
}

fn peer_key(first: Uuid, second: Uuid) -> (Uuid, Uuid) {
    if first < second {
        (first, second)
    } else {
        (second, first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identities() -> (ControlPlane, Uuid, Uuid) {
        let mut control = ControlPlane::default();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        control
            .register_identity(first, "harbor:one".into(), "public-one".into(), 10)
            .unwrap();
        control
            .register_identity(second, "harbor:two".into(), "public-two".into(), 10)
            .unwrap();
        (control, first, second)
    }

    fn paired() -> (ControlPlane, Uuid, Uuid) {
        let (mut control, first, second) = identities();
        let request = control.create_pairing(second, "123456".into(), 20).unwrap();
        control.submit_pairing(first, "123456", 21).unwrap();
        control
            .accept_pairing(second, request.pairing_id, 22)
            .unwrap();
        (control, first, second)
    }

    #[test]
    fn pairing_requires_explicit_acceptance_before_authorizing_peers() {
        let (mut control, first, second) = identities();
        let request = control.create_pairing(second, "123456".into(), 20).unwrap();
        control.submit_pairing(first, "123456", 21).unwrap();
        assert_eq!(
            control.connect_session(first, second, 22),
            Err(ControlError::PeersNotPaired)
        );

        let accepted = control
            .accept_pairing(second, request.pairing_id, 23)
            .unwrap();
        assert_eq!(accepted.state, PairingState::Accepted);
        assert_eq!(
            control.connect_session(first, second, 24).unwrap().state,
            SessionState::Connected
        );
    }

    #[test]
    fn pairing_code_expires_without_creating_a_relationship() {
        let (mut control, first, second) = identities();
        control.create_pairing(second, "123456".into(), 10).unwrap();
        assert_eq!(
            control.submit_pairing(first, "123456", 10 + PAIRING_TTL_SECONDS),
            Err(ControlError::UnknownPairing)
        );
        assert_eq!(
            control.connect_session(first, second, 11 + PAIRING_TTL_SECONDS),
            Err(ControlError::PeersNotPaired)
        );
    }

    #[test]
    fn only_target_can_accept_or_decline_incoming_request() {
        let (mut control, first, second) = identities();
        let request = control.create_pairing(second, "123456".into(), 10).unwrap();
        control.submit_pairing(first, "123456", 11).unwrap();
        assert_eq!(
            control.accept_pairing(first, request.pairing_id, 12),
            Err(ControlError::Unauthorized)
        );
        assert_eq!(
            control
                .decline_pairing(second, request.pairing_id, 12)
                .unwrap()
                .state,
            PairingState::Declined
        );
    }

    #[test]
    fn presence_is_visible_only_to_paired_peers_and_expires_to_offline() {
        let (mut control, first, second) = paired();
        control
            .publish_presence(second, Presence::Online, 30)
            .unwrap();
        assert_eq!(control.presence_of(first, second, 31), Ok(Presence::Online));
        assert_eq!(
            control.presence_of(first, second, 30 + PRESENCE_LEASE_SECONDS),
            Ok(Presence::Offline)
        );
    }

    #[test]
    fn session_signal_is_limited_and_rejected_after_disconnect() {
        let (mut control, first, second) = paired();
        let session = control.connect_session(first, second, 30).unwrap();
        assert_eq!(
            control.validate_signal(first, session.session_id, "candidate"),
            Ok(())
        );
        assert_eq!(
            control.validate_signal(
                second,
                session.session_id,
                &"x".repeat(MAX_SIGNAL_BYTES + 1)
            ),
            Err(ControlError::OversizedSignal)
        );
        control
            .disconnect_session(first, session.session_id, 31)
            .unwrap();
        assert_eq!(
            control.validate_signal(second, session.session_id, "candidate"),
            Err(ControlError::InactivePairing)
        );
    }

    #[test]
    fn signals_route_only_to_the_session_peer_and_drop_on_disconnect() {
        let (mut control, first, second) = paired();
        let stranger = Uuid::new_v4();
        control
            .register_identity(stranger, "harbor:stranger".into(), "public-x".into(), 25)
            .unwrap();
        let session = control.connect_session(first, second, 30).unwrap();

        // Only the session's members may queue, and only into a live session.
        assert_eq!(
            control.queue_signal(first, session.session_id, "offer", 30),
            Ok(())
        );
        assert_eq!(
            control.queue_signal(stranger, session.session_id, "intruder", 30),
            Err(ControlError::Unauthorized)
        );

        // Draining returns only what the other peer left, and clears it.
        let inbound = control.drain_signals(second, session.session_id).unwrap();
        assert_eq!(inbound.len(), 1);
        assert_eq!(inbound[0].from_peer, first);
        assert_eq!(inbound[0].signal, "offer");
        assert!(
            control
                .drain_signals(second, session.session_id)
                .unwrap()
                .is_empty()
        );

        // Disconnect tears the relay queue down with the session.
        control
            .queue_signal(first, session.session_id, "late", 31)
            .unwrap();
        control
            .disconnect_session(first, session.session_id, 32)
            .unwrap();
        assert!(
            control
                .drain_signals(second, session.session_id)
                .unwrap()
                .is_empty()
        );
    }

    #[test]
    fn signal_queues_refuse_overflow_instead_of_reordering() {
        let (mut control, first, second) = paired();
        let session = control.connect_session(first, second, 30).unwrap();
        for index in 0..MAX_SESSION_SIGNALS {
            control
                .queue_signal(first, session.session_id, &format!("s{index}"), 30)
                .unwrap();
        }
        assert_eq!(
            control.queue_signal(first, session.session_id, "overflow", 30),
            Err(ControlError::SignalQueueFull)
        );
        // The recipient still drains the retained prefix, in order.
        let inbound = control.drain_signals(second, session.session_id).unwrap();
        assert_eq!(
            inbound.first().map(|entry| entry.signal.as_str()),
            Some("s0")
        );
        assert_eq!(inbound.len(), MAX_SESSION_SIGNALS);
    }

    #[test]
    fn paired_peers_lists_only_paired_registered_devices() {
        let (mut control, first, second) = paired();
        let stranger = Uuid::new_v4();
        control
            .register_identity(stranger, "harbor:stranger".into(), "public-x".into(), 25)
            .unwrap();

        let peers = control.paired_peers(first).unwrap();
        assert_eq!(peers.len(), 1);
        assert_eq!(peers[0].device_id, second);

        assert!(control.paired_peers(stranger).unwrap().is_empty());
        // Unknown devices are a resolution error, never an empty list.
        assert_eq!(
            control.paired_peers(Uuid::new_v4()),
            Err(ControlError::UnknownIdentity)
        );
    }

    #[test]
    fn snapshot_round_trip_preserves_identities_and_relationships() {
        let (control, first, second) = paired();
        let snapshot = control.snapshot();
        assert!(snapshot.identities.len() == 2 && snapshot.relationships.len() == 1);

        let mut restored = ControlPlane::restore(snapshot);
        assert_eq!(restored.identity(first), control.identity(first));
        assert_eq!(restored.identity(second), control.identity(second));
        assert_eq!(
            restored.connect_session(first, second, 60).unwrap().state,
            SessionState::Connected
        );
    }

    #[test]
    fn restore_never_resurrects_transient_pairings_or_dangling_relationships() {
        let (mut control, first, second) = paired();
        let stranger = Uuid::new_v4();
        control.create_pairing(first, "111111".into(), 50).unwrap();

        let mut snapshot = control.snapshot();
        snapshot.relationships.push((second, stranger));
        let mut restored = ControlPlane::restore(snapshot);
        restored
            .register_identity(
                stranger,
                "harbor:stranger".into(),
                "public-stranger".into(),
                60,
            )
            .unwrap();
        assert_eq!(
            restored.connect_session(second, stranger, 61),
            Err(ControlError::PeersNotPaired)
        );
        assert_eq!(
            restored.connect_session(first, second, 62).unwrap().state,
            SessionState::Connected
        );
        assert_eq!(
            restored.submit_pairing(second, "111111", 63),
            Err(ControlError::UnknownPairing)
        );
    }
}
