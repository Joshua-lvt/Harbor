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

use harbor_control::is_harbor_id;

use crate::LocalIdentity;
use crate::server::{ServerClient, ServerClientError, ServerPin, rfc3339_now};
use crate::storage;

const SERVER_PIN_FILE: &str = "server-pin-v1.json";
const SERVER_PIN_SCHEMA_VERSION: u16 = 1;

/// Exact K11 production endpoints that may be migrated to Oracle. Only an
/// exact match migrates: custom endpoints, already-migrated hostnames, and
/// canary/test addresses are never overwritten (E3). The IPv6 literal is the
/// distributed default (desktop `HarborFacade.cpp`, mobile
/// `HarborMobileHost.qml`); the two LAN literals are obsolete UI text that
/// some installs may still carry.
pub const LEGACY_K11_ENDPOINTS: &[&str] = &[
    "[2804:d59:8777:ad00:3a30:f9ff:fe3e:de81]:9091",
    "192.168.1.6:9091",
    "192.168.1.7:9091",
];

/// Production Oracle endpoint: operator configuration, deliberately NOT a
/// compiled literal. The public address is deployment data, not source: it
/// arrives via `HARBOR_ORACLE_ADDRESS` (`HARBOR_SERVER_DEFAULT_ADDRESS` is
/// accepted as an alias — the same variable the desktop default honors).
/// Returns `None` when unset; callers treat that as "no Oracle endpoint
/// configured" instead of guessing.
pub fn oracle_endpoint() -> Option<String> {
    for key in ["HARBOR_ORACLE_ADDRESS", "HARBOR_SERVER_DEFAULT_ADDRESS"] {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim().to_owned();
            if !trimmed.is_empty() {
                return Some(trimmed);
            }
        }
    }
    None
}

/// Every endpoint `migrate_server_pin` may move: the legacy K11 literals
/// plus the configured Oracle endpoint. The Oracle address is a source
/// (never a compiled default) so both hops — K11 → Oracle IP, then Oracle
/// IP → a later address if the endpoint ever changes — migrate with the same
/// idempotent mechanism. Anything else is a custom endpoint and is preserved
/// untouched (E3).
pub fn is_migration_source(address: &str) -> bool {
    LEGACY_K11_ENDPOINTS.contains(&address)
        || oracle_endpoint().is_some_and(|oracle| oracle == address)
}

/// Outcome of one `migrate_server_pin` call. `Migrated` performed an atomic
/// write; every other variant wrote nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinMigration {
    /// `from` exactly matched a legacy endpoint; pin now points at `to`.
    Migrated { from: String, to: String },
    /// Already at `to`: second runs are no-ops (E4 idempotence).
    AlreadyMigrated { address: String },
    /// No pin stored: fresh installs configure directly (E1), never migrate.
    NotConfigured,
    /// Custom or unknown endpoint: preserved untouched (E3).
    PreservedCustom { address: String },
}

#[derive(Debug, thiserror::Error)]
pub enum PinMigrationError {
    #[error("destination address is invalid")]
    InvalidAddress,
    #[error("no server pin is configured")]
    NotConfigured,
}

/// Migrates an exact legacy K11 endpoint to `to_address`, preserving the
/// pinned fingerprint.
///
/// - Fingerprint is taken from the stored pin, never from the caller: a
///   migration cannot swap trust (cutover keeps `b9846…`).
/// - Only `LEGACY_K11_ENDPOINTS` exact matches migrate; everything else is
///   returned as `AlreadyMigrated`/`PreservedCustom` with no write.
/// - The write reuses `store_server_pin` (temp-file + rename, 0600), so a
///   crash leaves the old or the new valid JSON, never a partial file (E4).
/// - Callers must reconnect control/STUN/TURN/relay after `Migrated` (E5):
///   this function only swaps durable state, never live connections, and
///   never talks to two authorities.
pub fn migrate_server_pin(
    directory: &Path,
    to_address: &str,
) -> Result<PinMigration, PinMigrationError> {
    let Some(current) = load_server_pin(directory) else {
        return Ok(PinMigration::NotConfigured);
    };
    // Validate the destination against the stored fingerprint: same trust,
    // new route. Rejects malformed hostnames/IPs before any comparison.
    let dest = ServerPin::parse(to_address, &current.fingerprint_hex)
        .map_err(|_| PinMigrationError::InvalidAddress)?;
    if current.address == dest.address {
        return Ok(PinMigration::AlreadyMigrated {
            address: current.address,
        });
    }
    if !is_migration_source(&current.address) {
        return Ok(PinMigration::PreservedCustom {
            address: current.address,
        });
    }
    let from = current.address.clone();
    store_server_pin(directory, &dest).map_err(|_| PinMigrationError::InvalidAddress)?;
    Ok(PinMigration::Migrated {
        from,
        to: dest.address,
    })
}

/// True when the stored pin points at a migratable source (legacy K11 or
/// Oracle canary): the UI migration banner gate.
pub fn server_pin_needs_migration(directory: &Path) -> bool {
    load_server_pin(directory).is_some_and(|pin| is_migration_source(&pin.address))
}

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
    /// Harbor ID this device invited (Harbor-ID flow), echoed for display.
    /// Never a secret: it is the peer's public identifier, typed by the user.
    peer_harbor_id: Option<String>,
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
            "peer_harbor_id": self.peer_harbor_id,
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
        Ok(self.apply_incoming(payload))
    }

    fn apply_incoming(&mut self, payload: serde_json::Value) -> bool {
        let requests = payload
            .get("requests")
            .and_then(serde_json::Value::as_array);
        let Some(first) = requests.and_then(|list| list.first()) else {
            self.pending_request = None;
            return false;
        };
        if matches!(
            self.phase,
            PairingPhase::Idle | PairingPhase::WaitingApproval | PairingPhase::Error
        ) {
            self.role = Some(PairingRole::Host);
            self.phase = PairingPhase::IncomingRequest;
            self.error_key = None;
        }
        if let Some(id) = first.get("pairing_id").and_then(serde_json::Value::as_str) {
            self.pairing_id = Uuid::parse_str(id).ok().or(self.pairing_id);
        }
        self.pending_request = Some(first.clone());
        true
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

    /// Peer: invites the device behind `harbor_id` and waits for approval.
    /// The full Harbor ID is validated locally first (nothing leaves this
    /// machine when it is malformed) and travels verbatim to the server,
    /// which resolves it to the peer device. No numeric conversion anywhere.
    pub fn peer_connect(
        &mut self,
        pin: &ServerPin,
        identity: &LocalIdentity,
        harbor_id: &str,
    ) -> Result<(), PairingError> {
        if !is_harbor_id(harbor_id) {
            return Err(PairingError::InvalidHarborId);
        }
        let harbor_id = harbor_id.to_owned();
        self.begin(PairingRole::Peer, PairingPhase::Requesting);
        let payload = exchange(
            pin,
            identity,
            "pairing.invite",
            serde_json::json!({ "peer": harbor_id }),
        )?;
        self.pairing_id = parse_pairing_id(&payload);
        self.peer_harbor_id = Some(harbor_id);
        Ok(())
    }

    /// Peer: enters the host's code and requests approval.
    ///
    /// Legacy six-digit flow: kept for older clients. New UI uses
    /// [`PairingSession::peer_connect`] with a Harbor ID instead.
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
    #[error("the Harbor ID must look like harbor-xxxxxxxx (8 hex characters)")]
    InvalidHarborId,
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

    #[test]
    fn harbor_id_invite_moves_an_idle_recipient_to_incoming() {
        let pairing_id = Uuid::new_v4();
        let mut session = PairingSession::default();
        assert!(session.apply_incoming(serde_json::json!({
            "requests": [{
                "pairing_id": pairing_id,
                "requester_harbor_id": "harbor-aabbccdd"
            }]
        })));

        let snapshot = session.snapshot();
        assert_eq!(snapshot["phase"], "INCOMING_REQUEST");
        assert_eq!(snapshot["role"], "host");
        assert_eq!(snapshot["pairing_id"], pairing_id.to_string());
        assert_eq!(
            session.pending_request().unwrap()["requester_harbor_id"],
            "harbor-aabbccdd"
        );
    }

    #[test]
    fn harbor_id_invite_recovers_after_polling_error() {
        let mut session = PairingSession::default();
        session.mark_error("error.server.unavailable".into());

        assert!(session.apply_incoming(serde_json::json!({
            "requests": [{
                "pairing_id": Uuid::new_v4(),
                "requester_harbor_id": "harbor-aabbccdd"
            }]
        })));

        let snapshot = session.snapshot();
        assert_eq!(snapshot["phase"], "INCOMING_REQUEST");
        assert_eq!(snapshot["role"], "host");
        assert!(snapshot["error_key"].is_null());
    }

    fn test_pin(address: &str) -> ServerPin {
        ServerPin::parse(address, &"b".repeat(64)).unwrap()
    }

    #[test]
    fn migration_moves_only_exact_legacy_endpoints() {
        // TEST-NET-1 stands in for the operator endpoint: the real address
        // arrives via HARBOR_ORACLE_ADDRESS, never compiled in.
        const TEST_ORACLE: &str = "192.0.2.99:9091";
        let dir = std::env::temp_dir().join(format!("harbor-migrate-test-{}", Uuid::new_v4()));
        // NotConfigured: fresh installs never migrate (E1).
        assert_eq!(
            migrate_server_pin(&dir, TEST_ORACLE).unwrap(),
            PinMigration::NotConfigured
        );
        // Legacy IPv6 migrates, preserving the fingerprint.
        store_server_pin(&dir, &test_pin(LEGACY_K11_ENDPOINTS[0])).unwrap();
        assert!(server_pin_needs_migration(&dir));
        let outcome = migrate_server_pin(&dir, TEST_ORACLE).unwrap();
        assert_eq!(
            outcome,
            PinMigration::Migrated {
                from: LEGACY_K11_ENDPOINTS[0].to_owned(),
                to: TEST_ORACLE.to_owned(),
            }
        );
        let pin = load_server_pin(&dir).unwrap();
        assert_eq!(pin.address, TEST_ORACLE);
        assert_eq!(pin.fingerprint_hex, "b".repeat(64));
        // Without env configured the TEST address is custom, not a source.
        assert!(!server_pin_needs_migration(&dir));
        // Second run is a no-op (E4 idempotence).
        assert_eq!(
            migrate_server_pin(&dir, TEST_ORACLE).unwrap(),
            PinMigration::AlreadyMigrated {
                address: TEST_ORACLE.to_owned(),
            }
        );
        // Legacy LAN literals migrate too.
        for legacy in &LEGACY_K11_ENDPOINTS[1..] {
            store_server_pin(&dir, &test_pin(legacy)).unwrap();
            assert!(matches!(
                migrate_server_pin(&dir, TEST_ORACLE).unwrap(),
                PinMigration::Migrated { .. }
            ));
        }
        // Custom endpoints are preserved (E3).
        store_server_pin(&dir, &test_pin("custom.example.com:9091")).unwrap();
        assert!(!server_pin_needs_migration(&dir));
        assert_eq!(
            migrate_server_pin(&dir, TEST_ORACLE).unwrap(),
            PinMigration::PreservedCustom {
                address: "custom.example.com:9091".to_owned(),
            }
        );
        assert_eq!(
            load_server_pin(&dir).unwrap().address,
            "custom.example.com:9091"
        );
        // Invalid destinations never write.
        store_server_pin(&dir, &test_pin(LEGACY_K11_ENDPOINTS[0])).unwrap();
        assert!(matches!(
            migrate_server_pin(&dir, "not an address!!"),
            Err(PinMigrationError::InvalidAddress)
        ));
        assert_eq!(
            load_server_pin(&dir).unwrap().address,
            LEGACY_K11_ENDPOINTS[0]
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn migration_env_oracle_is_a_source_for_the_second_hop() {
        const TEST_ORACLE: &str = "192.0.2.99:9091";
        struct EnvRestore {
            oracle: Option<String>,
            fallback: Option<String>,
        }
        impl Drop for EnvRestore {
            fn drop(&mut self) {
                match &self.oracle {
                    Some(value) => unsafe { std::env::set_var("HARBOR_ORACLE_ADDRESS", value) },
                    None => unsafe { std::env::remove_var("HARBOR_ORACLE_ADDRESS") },
                }
                match &self.fallback {
                    Some(value) => unsafe {
                        std::env::set_var("HARBOR_SERVER_DEFAULT_ADDRESS", value)
                    },
                    None => unsafe { std::env::remove_var("HARBOR_SERVER_DEFAULT_ADDRESS") },
                }
            }
        }
        let _restore = EnvRestore {
            oracle: std::env::var("HARBOR_ORACLE_ADDRESS").ok(),
            fallback: std::env::var("HARBOR_SERVER_DEFAULT_ADDRESS").ok(),
        };
        unsafe {
            std::env::remove_var("HARBOR_ORACLE_ADDRESS");
        }
        unsafe {
            std::env::remove_var("HARBOR_SERVER_DEFAULT_ADDRESS");
        }
        assert_eq!(oracle_endpoint(), None);
        assert!(!is_migration_source(TEST_ORACLE));

        unsafe {
            std::env::set_var("HARBOR_ORACLE_ADDRESS", TEST_ORACLE);
        }
        assert_eq!(oracle_endpoint().as_deref(), Some(TEST_ORACLE));
        assert!(is_migration_source(TEST_ORACLE));
        // Alias honored, primary wins.
        unsafe {
            std::env::set_var("HARBOR_SERVER_DEFAULT_ADDRESS", "198.51.100.9:9091");
        }
        assert_eq!(oracle_endpoint().as_deref(), Some(TEST_ORACLE));
        unsafe {
            std::env::remove_var("HARBOR_ORACLE_ADDRESS");
        }
        assert_eq!(oracle_endpoint().as_deref(), Some("198.51.100.9:9091"));

        // Second hop through the env address preserves the pin.
        unsafe {
            std::env::set_var("HARBOR_ORACLE_ADDRESS", TEST_ORACLE);
        }
        let dir = std::env::temp_dir().join(format!("harbor-migrate-env-{}", Uuid::new_v4()));
        store_server_pin(&dir, &test_pin(TEST_ORACLE)).unwrap();
        assert!(server_pin_needs_migration(&dir));
        let outcome = migrate_server_pin(&dir, "harbor.example.com:9091").unwrap();
        assert_eq!(
            outcome,
            PinMigration::Migrated {
                from: TEST_ORACLE.to_owned(),
                to: "harbor.example.com:9091".to_owned(),
            }
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn migration_to_hostname_preserves_pin() {
        let dir = std::env::temp_dir().join(format!("harbor-migrate-host-{}", Uuid::new_v4()));
        store_server_pin(&dir, &test_pin(LEGACY_K11_ENDPOINTS[0])).unwrap();
        let outcome = migrate_server_pin(&dir, "harbor.example.com:9091").unwrap();
        assert_eq!(
            outcome,
            PinMigration::Migrated {
                from: LEGACY_K11_ENDPOINTS[0].to_owned(),
                to: "harbor.example.com:9091".to_owned(),
            }
        );
        let pin = load_server_pin(&dir).unwrap();
        assert_eq!(pin.address, "harbor.example.com:9091");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
