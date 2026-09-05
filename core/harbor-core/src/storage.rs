//! Shared private-file storage discipline for the Harbor domain process.
//!
//! State files live in a mode-0700 directory and are written atomically
//! (temporary file plus rename) with mode 0600 on Unix. Existing files that
//! carry broad permissions are rejected instead of silently rewritten.
//! On Windows the same calls succeed without extra hardening: a user's
//! profile directories already carry private ACLs by default, so there is
//! no broad-permission state to reject.

use std::{
    env,
    fs,
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum StorageError {
    #[error("storage I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("storage permissions are too broad")]
    InsecurePermissions,
}

pub fn prepare_private_directory(directory: &Path) -> Result<(), StorageError> {
    fs::create_dir_all(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub fn require_private_file(path: &Path) -> Result<(), StorageError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(path)?.permissions().mode() & 0o077 != 0 {
            return Err(StorageError::InsecurePermissions);
        }
    }
    Ok(())
}

/// Atomically replaces `path` with `bytes`, creating the file privately.
pub fn write_private_atomic(path: &Path, bytes: &[u8]) -> Result<(), StorageError> {
    let temporary = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(&temporary, path)?;
    require_private_file(path)?;
    Ok(())
}

/// Decodes an unpadded-Base64 field of exactly `length` bytes.
pub fn decode_fixed_field(value: &str, length: usize) -> Option<Vec<u8>> {
    let decoded = STANDARD_NO_PAD.decode(value).ok()?;
    (decoded.len() == length).then_some(decoded)
}

/// Resolves the Harbor state directory on every supported platform.
///
/// Precedence: `HARBOR_STATE_DIR`, then the platform home —
/// `%LOCALAPPDATA%\Harbor` (falling back to `%APPDATA%` and
/// `%USERPROFILE%\AppData\Local`) on Windows, `$XDG_STATE_HOME/harbor`
/// (falling back to `$HOME/.local/state/harbor`) elsewhere — then the
/// current directory as a last resort. Returns `None` only when no home
/// of any kind could be determined.
pub fn default_state_dir() -> Option<PathBuf> {
    if let Some(dir) = env::var_os("HARBOR_STATE_DIR").map(PathBuf::from) {
        return Some(dir);
    }
    #[cfg(windows)]
    {
        if let Some(local) = env::var_os("LOCALAPPDATA").map(PathBuf::from) {
            return Some(local.join("Harbor"));
        }
        if let Some(roaming) = env::var_os("APPDATA").map(PathBuf::from) {
            return Some(roaming.join("Harbor"));
        }
        if let Some(profile) = env::var_os("USERPROFILE").map(PathBuf::from) {
            return Some(profile.join("AppData").join("Local").join("Harbor"));
        }
    }
    #[cfg(not(windows))]
    {
        if let Some(state) = env::var_os("XDG_STATE_HOME").map(PathBuf::from) {
            return Some(state.join("harbor"));
        }
    }
    // `HOME` exists on Unix and under most Windows shells (Git Bash,
    // MSYS2); accept its conventional subpath on either platform.
    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        #[cfg(windows)]
        {
            return Some(home.join("AppData").join("Local").join("Harbor"));
        }
        #[cfg(not(windows))]
        {
            return Some(home.join(".local/state").join("harbor"));
        }
    }
    None
}
