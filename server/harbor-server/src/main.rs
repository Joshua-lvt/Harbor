//! Harbor Server control-plane entry point.
//!
//! Binds one TLS listener speaking the framed, signed control protocol. The
//! listener is control-plane only: media, chat, file, and DataChannel traffic
//! has no message type on the allowlist and cannot traverse this process. No
//! media or data-plane listener belongs here.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use harbor_server::{Listener, ListenerConfig, ServerCore};

fn state_directory() -> PathBuf {
    if let Ok(dir) = std::env::var("HARBOR_SERVER_STATE_DIR") {
        return PathBuf::from(dir);
    }
    let base = std::env::var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .ok()
        .filter(|path| path.is_absolute())
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|home| PathBuf::from(home).join(".local/state"))
        })
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("harbor-server")
}

fn main() {
    let bind = std::env::var("HARBOR_SERVER_BIND").unwrap_or_else(|_| "127.0.0.1:9091".to_owned());
    let bind: std::net::SocketAddr = match bind.parse() {
        Ok(addr) => addr,
        Err(error) => {
            eprintln!("harbor-server: invalid HARBOR_SERVER_BIND {bind:?}: {error}");
            std::process::exit(2);
        }
    };

    let state_dir = state_directory();
    let core = match ServerCore::open(&state_dir.join("state")) {
        Ok(core) => core,
        Err(error) => {
            eprintln!(
                "harbor-server: cannot open state at {}: {error}",
                state_dir.display()
            );
            std::process::exit(2);
        }
    };

    let config = ListenerConfig {
        bind,
        tls_dir: state_dir.join("tls"),
        cert_pem: std::env::var("HARBOR_SERVER_CERT").ok().map(PathBuf::from),
        key_pem: std::env::var("HARBOR_SERVER_KEY").ok().map(PathBuf::from),
    };

    let listener = match Listener::spawn(config, Arc::new(Mutex::new(core))) {
        Ok(listener) => listener,
        Err(error) => {
            eprintln!("harbor-server: cannot start listener on {bind}: {error}");
            std::process::exit(2);
        }
    };

    // The fingerprint is public pinning material, not a secret; it is what a
    // Harbor core must compare against when it first connects.
    let fingerprint = listener
        .certificate_fingerprint()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    eprintln!(
        "harbor-server: control-plane listener on {} (protocol v{}, certificate sha256:{fingerprint})",
        listener.local_addr(),
        harbor_protocol::VERSION,
    );

    // The listener owns its accept thread; main only parks so a SIGTERM/SIGINT
    // (default handlers) end the whole process tree, dropping in-flight
    // connections without orphaned listeners.
    loop {
        std::thread::park();
    }
}
