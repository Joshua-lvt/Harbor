//! Durable, versioned storage for the minimal server-side control state.
//!
//! Only identity records and pairing relationships are persisted; pending
//! pairings, presence leases, and sessions stay transient. Writes are atomic
//! (temporary file plus rename) and, on Unix, require a private directory and
//! a mode-0600 state file.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use harbor_control::ControlSnapshot;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

const STATE_FILE: &str = "control-state-v1.json";
const STATE_SCHEMA_VERSION: u16 = 1;

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("state storage I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("state storage contains invalid data")]
    InvalidDocument,
    #[error("state storage permissions are too broad")]
    InsecurePermissions,
}

#[derive(Debug, Serialize, Deserialize)]
struct StoredState {
    schema_version: u16,
    snapshot: ControlSnapshot,
}

#[derive(Debug, Clone)]
pub struct StateStore {
    path: PathBuf,
}

impl StateStore {
    /// Prepares `directory` for durable control state.
    pub fn open(directory: &Path) -> Result<Self, StoreError> {
        fs::create_dir_all(directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self {
            path: directory.join(STATE_FILE),
        })
    }

    /// Loads the stored snapshot, returning `None` when no state exists yet.
    pub fn load(&self) -> Result<Option<ControlSnapshot>, StoreError> {
        let bytes = match fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        require_private_file(&self.path)?;
        let stored: StoredState =
            serde_json::from_slice(&bytes).map_err(|_| StoreError::InvalidDocument)?;
        if stored.schema_version != STATE_SCHEMA_VERSION {
            return Err(StoreError::InvalidDocument);
        }
        Ok(Some(stored.snapshot))
    }

    /// Atomically replaces the stored snapshot.
    pub fn store(&self, snapshot: &ControlSnapshot) -> Result<(), StoreError> {
        let stored = StoredState {
            schema_version: STATE_SCHEMA_VERSION,
            snapshot: snapshot.clone(),
        };
        let bytes = serde_json::to_vec(&stored).map_err(|_| StoreError::InvalidDocument)?;
        let temporary = self.path.with_extension(format!("tmp-{}", Uuid::new_v4()));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        io::Write::write_all(&mut file, &bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, &self.path)?;
        require_private_file(&self.path)?;
        Ok(())
    }
}

fn require_private_file(path: &Path) -> Result<(), StoreError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(path)?.permissions().mode() & 0o077 != 0 {
            return Err(StoreError::InsecurePermissions);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use harbor_control::IdentityRecord;
    use uuid::Uuid;

    use super::*;

    fn temporary_directory() -> PathBuf {
        std::env::temp_dir().join(format!("harbor-server-state-test-{}", Uuid::new_v4()))
    }

    fn snapshot_with_relationship() -> ControlSnapshot {
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        ControlSnapshot {
            identities: vec![
                IdentityRecord {
                    device_id: first,
                    harbor_id: "harbor:one".into(),
                    public_key: "public-one".into(),
                    registered_at: 10,
                },
                IdentityRecord {
                    device_id: second,
                    harbor_id: "harbor:two".into(),
                    public_key: "public-two".into(),
                    registered_at: 10,
                },
            ],
            relationships: vec![(first, second)],
        }
    }

    #[test]
    fn snapshots_round_trip_through_the_store() {
        let directory = temporary_directory();
        let store = StateStore::open(&directory).unwrap();
        assert!(store.load().unwrap().is_none());

        let snapshot = snapshot_with_relationship();
        store.store(&snapshot).unwrap();
        assert_eq!(store.load().unwrap(), Some(snapshot));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn rejects_unknown_schema_versions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = temporary_directory();
        let store = StateStore::open(&directory).unwrap();
        fs::write(
            store.path.clone(),
            br#"{"schema_version": 99, "snapshot": {"identities": [], "relationships": []}}"#,
        )
        .unwrap();
        fs::set_permissions(&store.path, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(matches!(store.load(), Err(StoreError::InvalidDocument)));
        fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_existing_state_file_with_broad_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = temporary_directory();
        let store = StateStore::open(&directory).unwrap();
        store.store(&snapshot_with_relationship()).unwrap();
        fs::set_permissions(&store.path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(store.load(), Err(StoreError::InsecurePermissions)));
        fs::remove_dir_all(directory).unwrap();
    }
}
