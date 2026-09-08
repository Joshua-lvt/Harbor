//! Harbor Server control-plane entry point.
//!
//! Binds one TLS listener speaking the framed, signed control protocol, plus
//! a STUN-lite UDP socket on the same address (same port, different protocol)
//! so devices can learn their reflexive endpoint for direct dials. The
//! listener is control-plane only: media, chat, file, and DataChannel traffic
//! has no message type on the allowlist and cannot traverse this process. No
//! media or data-plane listener belongs here.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use harbor_server::{Listener, ListenerConfig, ServerCore, StunServer, TurnRelayConfig};

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

/// External TURN relay configuration from the environment:
///
/// - `HARBOR_TURN_EXTERNAL_IP` (aliases `HARBOR_TURN_PUBLIC_IP`,
///   `HARBOR_TURN_PUBLIC_ADDR`): public IPv4/IPv6 clients must dial.
/// - `HARBOR_TURN_RELAY_PORT_RANGE` (alias `HARBOR_TURN_PORT_RANGE`):
///   `"low-high"` UDP range for per-allocation sockets,
///   default `"49160-49175"` (16 = `MAX_TURN_ALLOCATIONS`).
///
/// Returns `None` when no external IP is set (compat mode: shared front
/// socket, current K11 behavior). Invalid values exit non-zero: silently
/// starting without the relay range on a public host would re-announce a
/// private address.
fn turn_relay_config_from_env() -> Option<TurnRelayConfig> {
    let external = std::env::var("HARBOR_TURN_EXTERNAL_IP")
        .or_else(|_| std::env::var("HARBOR_TURN_PUBLIC_IP"))
        .or_else(|_| std::env::var("HARBOR_TURN_PUBLIC_ADDR"))
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())?;
    let external_ip: std::net::IpAddr = match external.parse() {
        Ok(addr) => addr,
        Err(error) => {
            eprintln!("harbor-server: invalid HARBOR_TURN_EXTERNAL_IP {external:?}: {error}");
            std::process::exit(2);
        }
    };
    let range_raw = std::env::var("HARBOR_TURN_RELAY_PORT_RANGE")
        .or_else(|_| std::env::var("HARBOR_TURN_PORT_RANGE"))
        .unwrap_or_else(|_| "49160-49175".to_owned());
    let (low, high) = match range_raw.split_once('-') {
        Some((low, high)) => match (low.trim().parse::<u16>(), high.trim().parse::<u16>()) {
            (Ok(low), Ok(high)) => (low, high),
            _ => {
                eprintln!(
                    "harbor-server: invalid HARBOR_TURN_RELAY_PORT_RANGE {range_raw:?}: want \"low-high\""
                );
                std::process::exit(2);
            }
        },
        None => {
            eprintln!(
                "harbor-server: invalid HARBOR_TURN_RELAY_PORT_RANGE {range_raw:?}: want \"low-high\""
            );
            std::process::exit(2);
        }
    };
    match TurnRelayConfig::new(external_ip, low, high) {
        Some(config) => Some(config),
        None => {
            eprintln!(
                "harbor-server: invalid HARBOR_TURN_RELAY_PORT_RANGE {range_raw:?}: low must be nonzero and <= high"
            );
            std::process::exit(2);
        }
    }
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
    // TURN state outlives the move of `core` into the listener: the UDP
    // loop and the `turn.credentials` endpoint share this one store.
    let turn = core.turn();
    // External relay mode (Oracle behind 1:1 NAT): when a public IP is
    // configured, each TURN allocation owns a dedicated UDP socket from the
    // relay range and XOR-RELAYED-ADDRESS advertises public IP + that port.
    // Unset preserves the shared-socket K11 behavior (compat mode).
    if let Some(relay) = turn_relay_config_from_env() {
        eprintln!(
            "harbor-server: turn external relay {} ports {}-{}",
            relay.external_ip, relay.port_low, relay.port_high
        );
        turn.lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .set_external_relay(relay);
    }

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
    // STUN-lite shares the listener's address over UDP: no extra address to
    // distribute, no extra firewall story beyond opening the same port for
    // UDP. Clients derive it as "server host, same port, UDP". The same
    // socket serves TURN (relayed address = this socket).
    match StunServer::spawn(listener.local_addr(), turn) {
        Ok(stun) => eprintln!(
            "harbor-server: stun-lite rendezvous on {} (udp)",
            stun.local_addr()
        ),
        Err(error) => {
            eprintln!("harbor-server: cannot start stun-lite on {bind}: {error}");
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
