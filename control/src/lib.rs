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
    #[error("the Harbor ID must look like harbor-xxxxxxxx (8 hex characters)")]
    InvalidHarborId,
    #[error("the Harbor ID is already registered to another device")]
    HarborIdAlreadyRegistered,
    #[error("a device's Harbor ID cannot be changed")]
    HarborIdImmutable,
    #[error("a pairing code is already active")]
    DuplicatePairingCode,
    #[error("a device cannot pair with itself")]
    SelfPairing,
    #[error("the devices are already paired")]
    AlreadyPaired,
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

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RestoreError {
    #[error("persisted identity {device_id} has invalid fields")]
    InvalidIdentity { device_id: Uuid },
    #[error("persisted device ID {device_id} appears more than once")]
    DuplicateDeviceId { device_id: Uuid },
    #[error(
        "persisted Harbor ID {harbor_id:?} is bound to both {first_device} and {second_device}"
    )]
    DuplicateHarborId {
        harbor_id: String,
        first_device: Uuid,
        second_device: Uuid,
    },
    #[error("persisted relationship references an unknown identity")]
    DanglingRelationship { first: Uuid, second: Uuid },
    #[error("persisted relationship cannot connect device {device_id} to itself")]
    SelfRelationship { device_id: Uuid },
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
        if !is_harbor_id(&harbor_id) {
            return Err(ControlError::InvalidHarborId);
        }
        if public_key.trim().is_empty() || public_key.len() > 4096 {
            return Err(ControlError::InvalidIdentity);
        }
        if let Some(existing) = self.identities.get(&device_id) {
            if !existing.harbor_id.eq_ignore_ascii_case(&harbor_id) {
                return Err(ControlError::HarborIdImmutable);
            }
        }
        if self.identities.values().any(|identity| {
            identity.device_id != device_id && identity.harbor_id.eq_ignore_ascii_case(&harbor_id)
        }) {
            return Err(ControlError::HarborIdAlreadyRegistered);
        }
        let stored_harbor_id = self
            .identities
            .get(&device_id)
            .map(|identity| identity.harbor_id.clone())
            .unwrap_or_else(|| harbor_id.clone());
        let registered_at = self
            .identities
            .get(&device_id)
            .map(|identity| identity.registered_at)
            .unwrap_or(now);

        let identity = IdentityRecord {
            device_id,
            harbor_id: stored_harbor_id,
            public_key,
            registered_at,
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

    /// Harbor ID of a registered device, if known. Used to display who is
    /// asking in pairing invitations without exposing device UUIDs as names.
    pub fn harbor_id_of(&self, device_id: Uuid) -> Option<&str> {
        self.identities
            .get(&device_id)
            .map(|identity| identity.harbor_id.as_str())
    }

    /// Resolves a Harbor ID to its device. Exact full-string match only, with
    /// ASCII-insensitive hex casing on input. Stored casing is preserved.
    pub fn device_by_harbor_id(&self, harbor_id: &str) -> Option<Uuid> {
        if !is_harbor_id(harbor_id) {
            return None;
        }
        self.identities
            .values()
            .find(|identity| identity.harbor_id.eq_ignore_ascii_case(harbor_id))
            .map(|identity| identity.device_id)
    }

    /// Exports the durable state for persistence. Pending pairings, presence
    /// leases, and sessions are dropped by design.
    pub fn snapshot(&self) -> ControlSnapshot {
        ControlSnapshot {
            identities: self.identities.values().cloned().collect(),
            relationships: self.relationships.iter().copied().collect(),
        }
    }

    /// Rebuilds a control plane from durable state. Historical noncanonical
    /// Harbor IDs are retained for compatibility; new registrations still
    /// require the canonical format. Corrupt or colliding state is surfaced.
    pub fn restore(snapshot: ControlSnapshot) -> Result<Self, RestoreError> {
        let mut control = Self::default();
        for identity in snapshot.identities {
            if identity.harbor_id.is_empty()
                || identity.harbor_id.len() > 64
                || identity.public_key.trim().is_empty()
                || identity.public_key.len() > 4096
            {
                return Err(RestoreError::InvalidIdentity {
                    device_id: identity.device_id,
                });
            }
            if control.identities.contains_key(&identity.device_id) {
                return Err(RestoreError::DuplicateDeviceId {
                    device_id: identity.device_id,
                });
            }
            if let Some(existing) = control
                .identities
                .values()
                .find(|existing| existing.harbor_id.eq_ignore_ascii_case(&identity.harbor_id))
            {
                return Err(RestoreError::DuplicateHarborId {
                    harbor_id: identity.harbor_id,
                    first_device: existing.device_id,
                    second_device: identity.device_id,
                });
            }
            let device_id = identity.device_id;
            control.presence.insert(
                device_id,
                PresenceLease {
                    state: Presence::Offline,
                    expires_at: identity.registered_at,
                },
            );
            control.identities.insert(device_id, identity);
        }
        for (first, second) in snapshot.relationships {
            if first == second {
                return Err(RestoreError::SelfRelationship { device_id: first });
            }
            if !control.identities.contains_key(&first) || !control.identities.contains_key(&second)
            {
                return Err(RestoreError::DanglingRelationship { first, second });
            }
            control.relationships.insert(peer_key(first, second));
        }
        Ok(control)
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

    /// Harbor-ID pairing invitation: `requester` names the peer by its full
    /// Harbor ID instead of typing a six-digit code. The invitation lands
    /// directly in `WaitingApproval` — knowing the Harbor ID IS the
    /// introduction, and the target still consents explicitly via
    /// accept/decline. Retrying an identical live invitation returns the
    /// existing request (idempotent, no duplicates).
    ///
    /// The legacy code flow (`create_pairing`/`submit_pairing`) is untouched:
    /// invitations skip `PendingCode` at birth, so a code submit can never
    /// match them (`submit_pairing` only sees `PendingCode`).
    pub fn invite_pairing(
        &mut self,
        requester: Uuid,
        peer_harbor_id: &str,
        now: u64,
    ) -> Result<PairingSnapshot, ControlError> {
        self.expire(now);
        self.require_identity(requester)?;
        if !is_harbor_id(peer_harbor_id) {
            return Err(ControlError::InvalidHarborId);
        }
        let target = self
            .device_by_harbor_id(peer_harbor_id)
            .ok_or(ControlError::UnknownPairing)?;
        if target == requester {
            return Err(ControlError::SelfPairing);
        }
        if self.relationships.contains(&peer_key(requester, target)) {
            return Err(ControlError::AlreadyPaired);
        }
        if let Some(request) = self.pairings.values().find(|request| {
            request.snapshot.requester == Some(requester)
                && request.snapshot.target == target
                && request.snapshot.state == PairingState::WaitingApproval
        }) {
            return Ok(request.snapshot.clone());
        }

        let pairing_id = Uuid::new_v4();
        let snapshot = PairingSnapshot {
            pairing_id,
            requester: Some(requester),
            target,
            state: PairingState::WaitingApproval,
            expires_at: now + PAIRING_TTL_SECONDS,
        };
        self.pairings.insert(
            pairing_id,
            PairingRequest {
                snapshot: snapshot.clone(),
                code: String::new(),
            },
        );
        Ok(snapshot)
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

/// Canonical Harbor ID check, shared by the server gate and every client:
/// exactly `harbor-` plus 8 hexadecimal characters (15 chars total).
/// Full-string match only — no trimming, no prefix stripping, no numeric
/// conversion. Uppercase hex is accepted on input; identities are minted
/// lowercase and stored verbatim.
pub fn is_harbor_id(value: &str) -> bool {
    const PREFIX: &str = "harbor-";
    const TOTAL_LEN: usize = 15;
    value.len() == TOTAL_LEN
        && value.starts_with(PREFIX)
        && value
            .bytes()
            .skip(PREFIX.len())
            .all(|byte| byte.is_ascii_hexdigit())
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
            .register_identity(first, "harbor-00000001".into(), "public-one".into(), 10)
            .unwrap();
        control
            .register_identity(second, "harbor-00000002".into(), "public-two".into(), 10)
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
            .register_identity(stranger, "harbor-00000003".into(), "public-x".into(), 25)
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
            .register_identity(stranger, "harbor-00000003".into(), "public-x".into(), 25)
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

        let mut restored = ControlPlane::restore(snapshot).unwrap();
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
        control.create_pairing(first, "111111".into(), 50).unwrap();

        let restored = ControlPlane::restore(control.snapshot()).unwrap();
        let mut restored = restored;
        assert_eq!(
            restored.connect_session(first, second, 62).unwrap().state,
            SessionState::Connected
        );
        assert_eq!(
            restored.submit_pairing(second, "111111", 63),
            Err(ControlError::UnknownPairing)
        );
        let stranger = Uuid::new_v4();
        let mut malformed = control.snapshot();
        malformed.relationships.push((second, stranger));
        assert!(matches!(
            ControlPlane::restore(malformed),
            Err(RestoreError::DanglingRelationship { .. })
        ));
    }

    #[test]
    fn harbor_id_validation_accepts_only_the_canonical_shape() {
        for valid in [
            "harbor-d31846b8",
            "harbor-12345678",
            "harbor-abcdef12",
            "harbor-ABCDEF12",
        ] {
            assert!(is_harbor_id(valid), "{valid} must be valid");
        }
        for invalid in [
            "123456",
            "d31846b8",
            "harbor-d31846b",
            "harbor-d31846b89",
            "harbor-d31846g8",
            "harbor_12345678",
            "harbor 12345678",
            "",
            "harbor-",
            "harbor-1234567 ",
            " harbor-12345678",
            "HARBOR-12345678",
            "harbor--1234567",
        ] {
            assert!(!is_harbor_id(invalid), "{invalid:?} must be invalid");
        }
    }

    #[test]
    fn registration_binds_unique_immutable_harbor_ids() {
        let mut control = ControlPlane::default();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let stored = control
            .register_identity(first, "harbor-ABCDEF12".into(), "public-one".into(), 10)
            .unwrap();
        assert_eq!(stored.harbor_id, "harbor-ABCDEF12");

        // Hex casing is input-normalized for lookup, but the minted spelling
        // remains stable on idempotent re-registration.
        let repeated = control
            .register_identity(first, "harbor-abcdef12".into(), "public-one".into(), 20)
            .unwrap();
        assert_eq!(repeated.harbor_id, "harbor-ABCDEF12");
        assert_eq!(repeated.registered_at, 10);
        assert_eq!(
            control.register_identity(second, "harbor-abcdef12".into(), "public-two".into(), 20),
            Err(ControlError::HarborIdAlreadyRegistered)
        );
        assert_eq!(
            control.register_identity(first, "harbor-12345678".into(), "public-one".into(), 30),
            Err(ControlError::HarborIdImmutable)
        );
        assert_eq!(
            control.register_identity(second, "not-a-harbor-id".into(), "public-two".into(), 20),
            Err(ControlError::InvalidHarborId)
        );
    }

    #[test]
    fn restore_preserves_legacy_ids_and_relationships() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let mut restored = ControlPlane::restore(ControlSnapshot {
            identities: vec![
                IdentityRecord {
                    device_id: first,
                    harbor_id: "harbor-test".into(),
                    public_key: "public-one".into(),
                    registered_at: 10,
                },
                IdentityRecord {
                    device_id: second,
                    harbor_id: "harbor-00000002".into(),
                    public_key: "public-two".into(),
                    registered_at: 10,
                },
            ],
            relationships: vec![(first, second)],
        })
        .unwrap();
        assert_eq!(restored.harbor_id_of(first), Some("harbor-test"));
        assert_eq!(restored.paired_peers(first).unwrap()[0].device_id, second);
        assert_eq!(
            restored
                .create_pairing(second, "123456".into(), 20)
                .unwrap()
                .target,
            second
        );
        assert_eq!(
            restored.submit_pairing(first, "123456", 21).unwrap().target,
            second
        );
    }

    #[test]
    fn restore_surfaces_collisions_and_malformed_state() {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let duplicate = ControlPlane::restore(ControlSnapshot {
            identities: vec![
                IdentityRecord {
                    device_id: first,
                    harbor_id: "harbor-ABCDEF12".into(),
                    public_key: "one".into(),
                    registered_at: 1,
                },
                IdentityRecord {
                    device_id: second,
                    harbor_id: "harbor-abcdef12".into(),
                    public_key: "two".into(),
                    registered_at: 2,
                },
            ],
            relationships: vec![],
        });
        assert!(matches!(
            duplicate,
            Err(RestoreError::DuplicateHarborId { .. })
        ));

        let malformed = ControlPlane::restore(ControlSnapshot {
            identities: vec![IdentityRecord {
                device_id: first,
                harbor_id: String::new(),
                public_key: "one".into(),
                registered_at: 1,
            }],
            relationships: vec![],
        });
        assert!(matches!(
            malformed,
            Err(RestoreError::InvalidIdentity { .. })
        ));

        let dangling = ControlPlane::restore(ControlSnapshot {
            identities: vec![IdentityRecord {
                device_id: first,
                harbor_id: "harbor-test".into(),
                public_key: "one".into(),
                registered_at: 1,
            }],
            relationships: vec![(first, second)],
        });
        assert!(matches!(
            dangling,
            Err(RestoreError::DanglingRelationship { .. })
        ));
    }

    #[test]
    fn restore_rejects_duplicate_device_ids_before_overwriting() {
        let device_id = Uuid::new_v4();
        let restored = ControlPlane::restore(ControlSnapshot {
            identities: vec![
                IdentityRecord {
                    device_id,
                    harbor_id: "harbor-aaaaaaaa".into(),
                    public_key: "public-one".into(),
                    registered_at: 1,
                },
                IdentityRecord {
                    device_id,
                    harbor_id: "harbor-bbbbbbbb".into(),
                    public_key: "public-two".into(),
                    registered_at: 2,
                },
            ],
            relationships: vec![],
        });
        assert!(matches!(
            restored,
            Err(RestoreError::DuplicateDeviceId { device_id: id }) if id == device_id
        ));
    }

    #[test]
    fn restore_rejects_self_relationships() {
        let device_id = Uuid::new_v4();
        let restored = ControlPlane::restore(ControlSnapshot {
            identities: vec![IdentityRecord {
                device_id,
                harbor_id: "harbor-aaaaaaaa".into(),
                public_key: "public-one".into(),
                registered_at: 1,
            }],
            relationships: vec![(device_id, device_id)],
        });
        assert!(matches!(
            restored,
            Err(RestoreError::SelfRelationship { device_id: id }) if id == device_id
        ));
    }

    fn harbor_identities() -> (ControlPlane, Uuid, String, Uuid, String) {
        let mut control = ControlPlane::default();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let first_harbor = control
            .register_identity(first, "harbor-aaaaaaaa".into(), "public-one".into(), 10)
            .unwrap()
            .harbor_id;
        let second_harbor = control
            .register_identity(second, "harbor-bbbbbbbb".into(), "public-two".into(), 10)
            .unwrap()
            .harbor_id;
        (control, first, first_harbor, second, second_harbor)
    }

    #[test]
    fn invite_pairing_resolves_full_harbor_ids_and_needs_acceptance() {
        let (mut control, first, _, second, second_harbor) = harbor_identities();
        let invite = control.invite_pairing(first, &second_harbor, 20).unwrap();
        assert_eq!(invite.state, PairingState::WaitingApproval);
        assert_eq!(invite.requester, Some(first));
        assert_eq!(invite.target, second);
        // The target sees who asks, by Harbor ID.
        let incoming = control.incoming_pairings(second, 21).unwrap();
        assert_eq!(incoming.len(), 1);
        assert_eq!(incoming[0].pairing_id, invite.pairing_id);
        assert_eq!(control.harbor_id_of(first), Some("harbor-aaaaaaaa"));
        // Nothing is authorized before explicit acceptance.
        assert_eq!(
            control.connect_session(first, second, 22),
            Err(ControlError::PeersNotPaired)
        );
        control
            .accept_pairing(second, invite.pairing_id, 23)
            .unwrap();
        assert_eq!(
            control.connect_session(first, second, 24).unwrap().state,
            SessionState::Connected
        );
        // Lookup is exact-only, while hex casing is intentionally ignored.
        assert_eq!(control.device_by_harbor_id("harbor-aaaaaaaa"), Some(first));
        assert_eq!(control.device_by_harbor_id("aaaaaaaa"), None);
        assert_eq!(control.device_by_harbor_id("harbor-aaaa"), None);
        assert_eq!(control.device_by_harbor_id("harbor-AAAAAAAA"), Some(first));
    }

    #[test]
    fn invite_pairing_rejects_malformed_unknown_self_and_paired() {
        let (mut control, first, _, second, second_harbor) = harbor_identities();
        assert_eq!(
            control.invite_pairing(first, "123456", 20),
            Err(ControlError::InvalidHarborId)
        );
        assert_eq!(
            control.invite_pairing(first, "harbor-cccccccc", 20),
            Err(ControlError::UnknownPairing)
        );
        assert_eq!(
            control.invite_pairing(first, "harbor-aaaaaaaa", 20),
            Err(ControlError::SelfPairing)
        );
        // Retry while live returns the same invitation (idempotent).
        let invite = control.invite_pairing(first, &second_harbor, 21).unwrap();
        let retry = control.invite_pairing(first, &second_harbor, 22).unwrap();
        assert_eq!(retry.pairing_id, invite.pairing_id);
        // After acceptance, inviting again names the existing relationship.
        control
            .accept_pairing(second, invite.pairing_id, 23)
            .unwrap();
        assert_eq!(
            control.invite_pairing(first, &second_harbor, 24),
            Err(ControlError::AlreadyPaired)
        );
        // Code submits cannot touch invitations (different state at birth).
        assert_eq!(
            control.submit_pairing(second, "", 25),
            Err(ControlError::UnknownPairing)
        );
    }
}
