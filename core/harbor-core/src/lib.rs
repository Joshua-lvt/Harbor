//! Local identity persistence for the Harbor domain process.
//!
//! The private signing seed is never returned from this crate. On Unix systems
//! the fallback store requires a private directory and a mode-0600 file until a
//! platform keyring adapter is available.

use std::{fs, io, path::Path};

use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use ed25519_dalek::SigningKey;
use harbor_control::IdentityRecord;
use harbor_protocol::{AuthenticatedEnvelope, AuthenticationError, Envelope};
use serde::{Deserialize, Serialize};
use storage::StorageError;
use thiserror::Error;
use uuid::Uuid;

pub mod activity;
pub mod app;
pub mod appicon;
pub mod device;
pub mod direct;
pub mod monitor;
pub mod presence;
pub mod profile;
mod pairing;
mod server;
mod settings;
mod storage;

pub use pairing::{
    PairingError, PairingPhase, PairingRole, PairingSession, load_server_pin, register_identity,
    store_server_pin,
};pub use server::{ServerClient, ServerClientError, ServerPin, rfc3339_now};

pub use settings::{Settings, SettingsError, StoredSettings};
pub use storage::default_state_dir;

const IDENTITY_FILE: &str = "identity-v1.json";
const IDENTITY_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Error)]
pub enum IdentityError {
    #[error("identity storage I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("identity storage contains invalid data")]
    InvalidDocument,
    #[error("identity storage permissions are too broad")]
    InsecurePermissions,
    #[error("system clock is before the Unix epoch")]
    InvalidClock,
}

impl From<StorageError> for IdentityError {
    fn from(error: StorageError) -> Self {
        match error {
            StorageError::Io(io) => IdentityError::Io(io),
            StorageError::InsecurePermissions => IdentityError::InsecurePermissions,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LocalIdentity {
    record: IdentityRecord,
    signing_key: SigningKey,
}

impl LocalIdentity {
    pub fn record(&self) -> &IdentityRecord {
        &self.record
    }

    pub fn sign_envelope(
        &self,
        envelope: Envelope,
    ) -> Result<AuthenticatedEnvelope, AuthenticationError> {
        AuthenticatedEnvelope::sign(self.record.device_id, envelope, &self.signing_key)
    }

    /// Signs raw challenge bytes for the direct-link handshake. The
    /// signature proves possession of this install's identity key without
    /// ever exposing the seed; verification needs only the public record.
    pub fn sign_bytes(&self, message: &[u8]) -> [u8; 64] {
        use ed25519_dalek::Signer as _;
        self.signing_key.sign(message).to_bytes()
    }

    /// The private seed, for the in-process link worker only. Same-process
    /// memory, never serialized, never logged, never transmitted.
    pub(crate) fn seed_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredIdentity {
    schema_version: u16,
    device_id: Uuid,
    harbor_id: String,
    private_seed: String,
    created_at: u64,
}

pub fn load_or_create(directory: &Path, now: u64) -> Result<LocalIdentity, IdentityError> {
    storage::prepare_private_directory(directory)?;
    let path = directory.join(IDENTITY_FILE);
    if path.exists() {
        storage::require_private_file(&path)?;
        return decode_identity(&fs::read(&path)?);
    }

    let identity = new_identity(now);
    write_identity(&path, &identity)?;
    Ok(identity)
}

fn new_identity(now: u64) -> LocalIdentity {
    let mut seed = [0_u8; 32];
    for chunk in seed.chunks_exact_mut(16) {
        chunk.copy_from_slice(Uuid::new_v4().as_bytes());
    }
    let signing_key = SigningKey::from_bytes(&seed);
    let device_id = Uuid::new_v4();
    let harbor_id = format!("harbor-{}", &device_id.simple().to_string()[..8]);
    let record = IdentityRecord {
        device_id,
        harbor_id,
        public_key: STANDARD_NO_PAD.encode(signing_key.verifying_key().as_bytes()),
        registered_at: now,
    };
    LocalIdentity {
        record,
        signing_key,
    }
}

fn decode_identity(bytes: &[u8]) -> Result<LocalIdentity, IdentityError> {
    let stored: StoredIdentity =
        serde_json::from_slice(bytes).map_err(|_| IdentityError::InvalidDocument)?;
    if stored.schema_version != IDENTITY_SCHEMA_VERSION || stored.harbor_id.trim().is_empty() {
        return Err(IdentityError::InvalidDocument);
    }
    let seed = storage::decode_fixed_field(&stored.private_seed, 32)
        .ok_or(IdentityError::InvalidDocument)?;
    let seed: [u8; 32] = seed
        .try_into()
        .expect("decode_fixed_field returned exactly 32 bytes");
    let signing_key = SigningKey::from_bytes(&seed);
    Ok(LocalIdentity {
        record: IdentityRecord {
            device_id: stored.device_id,
            harbor_id: stored.harbor_id,
            public_key: STANDARD_NO_PAD.encode(signing_key.verifying_key().as_bytes()),
            registered_at: stored.created_at,
        },
        signing_key,
    })
}

fn write_identity(path: &Path, identity: &LocalIdentity) -> Result<(), IdentityError> {
    let stored = StoredIdentity {
        schema_version: IDENTITY_SCHEMA_VERSION,
        device_id: identity.record.device_id,
        harbor_id: identity.record.harbor_id.clone(),
        private_seed: STANDARD_NO_PAD.encode(identity.signing_key.to_bytes()),
        created_at: identity.record.registered_at,
    };
    let bytes = serde_json::to_vec(&stored).map_err(|_| IdentityError::InvalidDocument)?;
    storage::write_private_atomic(path, &bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_directory() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("harbor-identity-test-{}", Uuid::new_v4()))
    }

    #[test]
    fn identity_is_stable_across_reloads_without_exposing_private_state() {
        let directory = temporary_directory();
        let created = load_or_create(&directory, 123).unwrap();
        let reloaded = load_or_create(&directory, 456).unwrap();
        assert_eq!(created.record(), reloaded.record());
        assert_eq!(
            created.signing_key.to_bytes(),
            reloaded.signing_key.to_bytes()
        );
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_existing_identity_file_with_broad_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = temporary_directory();
        let _identity = load_or_create(&directory, 123).unwrap();
        let path = directory.join(IDENTITY_FILE);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            load_or_create(&directory, 124),
            Err(IdentityError::InsecurePermissions)
        ));
        fs::remove_dir_all(directory).unwrap();
    }
}
