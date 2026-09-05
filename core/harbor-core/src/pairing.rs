//! Local pairing session state over the control-plane server.
//!
//! The core owns the pairing vocabulary from the product plan: a host
//! registers a short code and waits for approval decisions, a peer enters the
//! code and watches for the host's decision. Every server interaction is an
//! exchange of signed envelopes over the pinned TLS client; private keys stay
//! inside the local identity and never cross this module's surface.

use std::io;
use std::path::Path;

use serde::Serialize;
use thiserror::Error;
use uuid::Uuid;

use crate::LocalIdentity;
use crate::server::{ServerClient, ServerClientError, ServerPin, rfc3339_now};
use crate::storage;

const SERVER_PIN_FILE: &str = "server-pin-v1.json";
const SERVER_PIN_SCHEMA_VERSION: u16 = 1;

/// The durable pin for the control-plane server: where to connect and which
/// certificate fingerprint to trust. Both values are public material.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
pub struct StoredServerPin {
    pub schema_version: u16,
    pub address: String,
    pub fingerprint_hex: String,
}

/// Reads the persisted server pin, if one is configured and valid.
pub fn load_server_pin(directory: &Path) -> Option<ServerPin> {
    let bytes = std::fs::read(directory.join(SERVER_PIN_FILE)).ok()?;
    let stored: StoredServerPin = serde_json::from_slice(&bytes).ok()?;
    if stored.schema_version != SERVER_PIN_SCHEMA_VERSION {
        return None;
    }
    ServerPin::parse(stored.address, &stored.fingerprint_hex).ok()
}

/// Persists the server pin atomically with private file permissions.
pub fn store_server_pin(directory: &Path, pin: &ServerPin) -> io::Result<()> {
    let storage_io_error = |error: crate::storage::StorageError| match error {
        crate::storage::StorageError::Io(io) => io,
        other => io::Error::new(io::ErrorKind::PermissionDenied, other.to_string()),
    };
    storage::prepare_private_directory(directory).map_err(storage_io_error)?;
    let stored = StoredServerPin {
        schema_version: SERVER_PIN_SCHEMA_VERSION,
        address: pin.address.clone(),
        fingerprint_hex: pin.fingerprint_hex.clone(),
    };
    let bytes = serde_json::to_vec(&stored).map_err(|_| {
        io::Error::new(io::ErrorKind::InvalidData, "server pin is not serializable")
    })?;
    storage::write_private_atomic(&directory.join(SERVER_PIN_FILE), &bytes)
        .map_err(storage_io_error)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PairingPhase {
    #[default]
    Idle,
    EnteringCode,
    Requesting,
    WaitingApproval,
    IncomingRequest,
    Accepted,
    Declined,
    Success,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PairingRole {
    Host,
    Peer,
}

/// The local view of one pairing attempt. One session at a time: closing the
/// pairing surface resets it.
#[derive(Debug, Default)]
pub struct PairingSession {
    phase: PairingPhase,
    role: Option<PairingRole>,
    code: Option<String>,
    pairing_id: Option<Uuid>,
    error_key: Option<String>,
    /// The most recent incoming request as the server serialized it, so the
    /// UI can show who is asking. Never part of the session snapshot.
    pending_request: Option<serde_json::Value>,
}

impl PairingSession {
    pub fn phase(&self) -> PairingPhase {
        self.phase
    }

    /// The latest incoming pairing request, if a poll surfaced one.
    pub fn pending_request(&self) -> Option<&serde_json::Value> {
        self.pending_request.as_ref()
    }

    /// Snapshot for the IPC surface; no private material appears here.
    pub fn snapshot(&self) -> serde_json::Value {
        serde_json::json!({
            "phase": self.phase,
            "role": self.role,
            "code": self.code,
            "pairing_id": self.pairing_id,
            "error_key": self.error_key,
        })
    }

    /// Peer flow start: the user opened the code entry.
    pub fn enter_code(&mut self) {
        self.begin(PairingRole::Peer, PairingPhase::EnteringCode);
    }

    /// Leaves the local phase unchanged (a host begins by generating a code,
    /// a peer by entering one) but records the role for later actions.
    pub fn begin_host(&mut self) {
        self.begin(PairingRole::Host, PairingPhase::Idle);
    }

    fn begin(&mut self, role: PairingRole, phase: PairingPhase) {
        *self = Self {
            phase,
            role: Some(role),
            ..Self::default()
        };
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// Marks the session failed after a refused request; `error_key` is the
    /// localized key the server attached to the refusal.
    pub fn mark_error(&mut self, error_key: String) {
        self.phase = PairingPhase::Error;
        self.error_key = Some(error_key);
    }

    /// Host: generates a six-digit code, registers it with the server, and
    /// waits for the peer to submit it.
    pub fn host_create(
        &mut self,
        pin: &ServerPin,
        identity: &LocalIdentity,
    ) -> Result<String, PairingError> {
        self.begin(PairingRole::Host, PairingPhase::WaitingApproval);
        let code = generate_pairing_code();
        let payload = exchange(
            pin,
            identity,
            "pairing.create",
            serde_json::json!({ "code": code }),
        )?;
        self.code = Some(code.clone());
        self.pairing_id = parse_pairing_id(&payload);
        Ok(code)
    }

    /// Host: polls for a peer's submitted request.
    pub fn host_poll(
        &mut self,
        pin: &ServerPin,
        identity: &LocalIdentity,
    ) -> Result<bool, PairingError> {
        let payload = exchange(pin, identity, "pairing.incoming", serde_json::json!({}))?;
        let requests = payload
            .get("requests")
            .and_then(serde_json::Value::as_array);
        let Some(first) = requests.and_then(|list| list.first()) else {
            self.pending_request = None;
            return Ok(false);
        };
        if self.phase == PairingPhase::WaitingApproval {
            self.phase = PairingPhase::IncomingRequest;
        }
        if let Some(id) = first.get("pairing_id").and_then(serde_json::Value::as_str) {
            self.pairing_id = Uuid::parse_str(id).ok().or(self.pairing_id);
        }
        self.pending_request = Some(first.clone());
        Ok(true)
    }

    /// Host: approves the pending request; the relationship now exists.
    pub fn host_accept(
        &mut self,
        pin: &ServerPin,
        identity: &LocalIdentity,
    ) -> Result<(), PairingError> {
        self.decide(pin, identity, "pairing.accept", PairingPhase::Accepted)
    }

    /// Host: refuses the pending request.
    pub fn host_decline(
        &mut self,
        pin: &ServerPin,
        identity: &LocalIdentity,
    ) -> Result<(), PairingError> {
        self.decide(pin, identity, "pairing.decline", PairingPhase::Declined)
    }

    fn decide(
        &mut self,
        pin: &ServerPin,
        identity: &LocalIdentity,
        message_type: &str,
        decided: PairingPhase,
    ) -> Result<(), PairingError> {
        let pairing_id = self.pairing_id.ok_or(PairingError::NoActiveRequest)?;
        let payload = exchange(
            pin,
            identity,
            message_type,
            serde_json::json!({ "pairing_id": pairing_id }),
        )?;
        self.phase = decided;
        // A decline on the host side also ends the code display.
        if decided == PairingPhase::Declined {
            self.code = None;
        }
        let _ = payload;
        Ok(())
    }

    /// Peer: enters the host's code and requests approval.
    pub fn peer_submit(
        &mut self,
        pin: &ServerPin,
        identity: &LocalIdentity,
        code: &str,
    ) -> Result<(), PairingError> {
        self.phase = PairingPhase::Requesting;
        let payload = exchange(
            pin,
            identity,
            "pairing.submit",
            serde_json::json!({ "code": code }),
        )?;
        self.pairing_id = parse_pairing_id(&payload);
        Ok(())
    }

    /// Peer: polls the host's decision. Stays in REQUESTING while the host
    /// has not decided.
    pub fn peer_poll(
        &mut self,
        pin: &ServerPin,
        identity: &LocalIdentity,
    ) -> Result<PairingPhase, PairingError> {
        let pairing_id = self.pairing_id.ok_or(PairingError::NoActiveRequest)?;
        let payload = exchange(
            pin,
            identity,
            "pairing.status",
            serde_json::json!({ "pairing_id": pairing_id }),
        )?;
        match payload.get("state").and_then(serde_json::Value::as_str) {
            Some("ACCEPTED") => self.phase = PairingPhase::Accepted,
            Some("DECLINED") => self.phase = PairingPhase::Declined,
            Some("EXPIRED") | Some("CANCELLED") => {
                self.phase = PairingPhase::Error;
                self.error_key = Some("error.pairing.unavailable".to_owned());
            }
            _ => self.phase = PairingPhase::Requesting,
        }
        Ok(self.phase)
    }

    /// Peer: withdraws the pending request.
    pub fn peer_cancel(
        &mut self,
        pin: &ServerPin,
        identity: &LocalIdentity,
    ) -> Result<(), PairingError> {
        if let Some(pairing_id) = self.pairing_id {
            exchange(
                pin,
                identity,
                "pairing.cancel",
                serde_json::json!({ "pairing_id": pairing_id }),
            )?;
        }
        self.reset();
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum PairingError {
    #[error("no pairing request is active")]
    NoActiveRequest,
    #[error("the six-digit pairing code is required")]
    MissingCode,
    #[error("control-server connection failed: {0}")]
    Connect(#[from] ServerClientError),
    #[error("pairing request was refused: {0}")]
    Refused(String),
}

/// Registers this device's identity with the server (proof-of-key: the
/// signed envelope carries the public key it signs with). Idempotent.
pub fn register_identity(pin: &ServerPin, identity: &LocalIdentity) -> Result<(), PairingError> {
    let record = identity.record();
    exchange(
        pin,
        identity,
        "identity.update",
        serde_json::json!({
            "device_id": record.device_id,
            "harbor_id": record.harbor_id,
            "public_key": record.public_key,
        }),
    )
    .map(|_| ())
}

/// One pinned connection per operation: the foundation keeps network I/O
/// simple and restartable; connection reuse is a later optimization.
fn exchange(
    pin: &ServerPin,
    identity: &LocalIdentity,
    message_type: &str,
    payload: serde_json::Value,
) -> Result<serde_json::Value, PairingError> {
    let mut client = ServerClient::connect(pin)?;
    let response = client.exchange(
        harbor_protocol::Envelope::request(message_type, payload, rfc3339_now()),
        identity,
    )?;
    if let Some(error) = response.error {
        return Err(PairingError::Refused(error.ui_key));
    }
    Ok(response.payload)
}

fn parse_pairing_id(payload: &serde_json::Value) -> Option<Uuid> {
    payload
        .get("pairing_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok())
}

/// Six ASCII digits, derived from random UUID bytes.
fn generate_pairing_code() -> String {
    let bytes = Uuid::new_v4().into_bytes();
    let value = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) % 1_000_000;
    format!("{value:06}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_is_six_ascii_digits() {
        for _ in 0..64 {
            let code = generate_pairing_code();
            assert_eq!(code.len(), 6);
            assert!(code.bytes().all(|byte| byte.is_ascii_digit()));
        }
    }

    #[test]
    fn phases_serialize_in_the_plan_vocabulary() {
        assert_eq!(
            serde_json::to_value(PairingPhase::EnteringCode).unwrap(),
            "ENTERING_CODE"
        );
        assert_eq!(
            serde_json::to_value(PairingPhase::WaitingApproval).unwrap(),
            "WAITING_APPROVAL"
        );
        assert_eq!(
            serde_json::to_value(PairingPhase::IncomingRequest).unwrap(),
            "INCOMING_REQUEST"
        );
        assert_eq!(serde_json::to_value(PairingRole::Peer).unwrap(), "peer");
    }

    #[test]
    fn the_session_snapshot_never_carries_private_material() {
        let mut session = PairingSession::default();
        session.enter_code();
        let snapshot = session.snapshot();
        assert_eq!(snapshot["phase"], "ENTERING_CODE");
        assert_eq!(snapshot["role"], "peer");
        assert!(snapshot.get("seed").is_none());
        assert!(snapshot.get("private_key").is_none());
        session.reset();
        assert_eq!(session.snapshot()["phase"], "IDLE");
    }
}
