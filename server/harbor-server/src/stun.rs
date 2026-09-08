//! STUN-lite rendezvous socket: shows a device its reflexive (server-side
//! observed) UDP endpoint so direct dials can advertise a globally meaningful
//! address.
//!
//! One UDP socket on the SAME `SocketAddr` as the TCP control listener (same
//! port, different protocol — no extra address to distribute, no extra hole
//! in the ops story). Since Phase 3.2 the same socket also serves TURN: the
//! relayed address IS this socket, so Allocate/Refresh/Permission/Send/Data
//! and ChannelData share the port with the STUN dialects. Demux order is
//! load-bearing: RFC binding, then the JSON lite dialect, then TURN-shaped
//! datagrams (STUN methods or ChannelData top bits); anything else is
//! silently dropped. No per-request state beyond the shared [`TurnState`].

use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use harbor_protocol::stun;

use crate::turn::{TurnOutcome, TurnState};

const RECV_TIMEOUT: Duration = Duration::from_millis(250);
/// TURN/media datagrams exceed the 512-byte JSON lite dialect: voice/video
/// plus framing needs headroom. JSON parsing still enforces its own 512-byte
/// ceiling internally, so a larger socket buffer only widens TURN.
const TURN_RECV_BYTES: usize = 2048;

#[derive(Debug, thiserror::Error)]
pub enum StunError {
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// A running STUN-lite socket owning its serve thread.
pub struct StunServer {
    local_addr: SocketAddr,
    shutdown: Arc<AtomicBool>,
}

impl StunServer {
    /// Binds `bind` for UDP and starts answering. `turn` is the
    /// [`ServerCore`](crate::ServerCore)-shared TURN state: the control plane
    /// mints credentials into it, this loop authenticates against it.
    /// Lifetime mirrors [`crate::Listener`]: dropping the process ends
    /// everything; in-flight datagrams need no draining.
    pub fn spawn(bind: SocketAddr, turn: Arc<Mutex<TurnState>>) -> Result<Self, StunError> {
        let socket = UdpSocket::bind(bind)?;
        let local_addr = socket.local_addr()?;
        let _ = socket.set_read_timeout(Some(RECV_TIMEOUT));
        let shutdown = Arc::new(AtomicBool::new(false));
        std::thread::Builder::new()
            .name("harbor-server-stun".into())
            .spawn({
                let shutdown = Arc::clone(&shutdown);
                move || serve_loop(socket, shutdown, turn)
            })
            .map_err(StunError::Io)?;
        Ok(Self {
            local_addr,
            shutdown,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Stops answering; the serve thread exits on its next read timeout.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
    }
}

fn serve_loop(socket: UdpSocket, shutdown: Arc<AtomicBool>, turn: Arc<Mutex<TurnState>>) {
    // The relayed address: TURN Allocate answers XOR_RELAYED_ADDRESS with
    // this socket, so relayed media arrives here and demuxes by peer.
    // `local_addr` just succeeded in `spawn`; the loopback fallback only
    // covers a socket that lost its binding mid-flight (answer degrades,
    // the loop never panics).
    let relay_addr = socket
        .local_addr()
        .unwrap_or(SocketAddr::from(([127, 0, 0, 1], 0)));
    let mut buffer = [0_u8; TURN_RECV_BYTES];
    loop {
        match socket.recv_from(&mut buffer) {
            Ok((count, source)) => {
                let datagram = &buffer[..count];
                // RFC 5389 binding requests first (disjoint from the JSON
                // dialect by top bits, but explicit order documents intent).
                if stun::is_binding_request(datagram) {
                    if let Some(reply) = stun::binding_response(datagram, source) {
                        let _ = socket.send_to(&reply, source);
                    }
                    continue;
                }
                // JSON lite dialect second: its `{` (0x7B) sits in the
                // ChannelData top-bits range, so it must win before TURN.
                if let Some(nonce) = stun::parse_request(datagram) {
                    let reply = stun::response_bytes(&nonce, source);
                    let _ = socket.send_to(&reply, source);
                    continue;
                }
                // TURN last: STUN-shaped methods and ChannelData only.
                // Anything else (including truncated datagrams) stays silent.
                if is_turn_shaped(datagram) {
                    let now = SystemTime::now()
                        .duration_since(UNIX_EPOCH)
                        .map(|elapsed| elapsed.as_secs())
                        .unwrap_or(0);
                    let ready = {
                        let mut state = turn.lock().unwrap_or_else(|poison| poison.into_inner());
                        let outcome = state.handle_datagram(datagram, source, relay_addr, now);
                        match outcome {
                            TurnOutcome::Reply(bytes) => {
                                let _ = socket.send_to(&bytes, source);
                            }
                            TurnOutcome::Forward { to, bytes, via } => {
                                if let Some(relay_socket) = via {
                                    let _ = relay_socket.send_to(&bytes, to);
                                } else {
                                    let _ = socket.send_to(&bytes, to);
                                }
                            }
                            TurnOutcome::Quiet => {}
                        }
                        // Spawn peer-reader threads for allocations that just
                        // gained a dedicated socket (external mode): each owns
                        // its relay socket and forwards peer datagrams to the
                        // client via the front socket + relay_receipt.
                        let mut ready = Vec::new();
                        for (alloc_id, _) in state.drain_socket_ready() {
                            if let Some(relay_socket) = state.allocation_socket(&alloc_id) {
                                ready.push((alloc_id, relay_socket));
                            }
                        }
                        ready
                    };
                    for (alloc_id, relay_socket) in ready {
                        let turn_clone = Arc::clone(&turn);
                        let Ok(front_clone) = socket.try_clone() else {
                            continue;
                        };
                        std::thread::Builder::new()
                            .name(format!("harbor-turn-relay-{alloc_id}"))
                            .spawn(move || {
                                relay_reader_loop(alloc_id, relay_socket, front_clone, turn_clone)
                            })
                            .ok();
                    }
                }
            }
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::TimedOut =>
            {
                if shutdown.load(Ordering::SeqCst) {
                    break;
                }
            }
            Err(_) => {
                if shutdown.load(Ordering::SeqCst) {
                    break;
                }
            }
        }
    }
}

/// TURN-shaped datagrams only: STUN methods (top bits `00`, the binding
/// request already handled above) or ChannelData (top bits `01`, length
/// present). The JSON lite dialect is checked before this, so its `{`
/// never arrives here; anything else stays silent without taking the lock.
fn is_turn_shaped(datagram: &[u8]) -> bool {
    if datagram.is_empty() {
        return false;
    }
    match datagram[0] & 0xC0 {
        0x00 => datagram.len() >= 20,
        0x40 => datagram.len() >= 4,
        _ => false,
    }
}

/// Peer-facing reader for one external-mode allocation: everything arriving
/// on the dedicated relay socket belongs to that allocation by construction
/// (no 5-tuple ambiguity), so ICE Binding from peers is relayed like any
/// other payload via [`TurnState::relay_receipt`]. Replies leave from the
/// front socket toward the allocation's current client address (rebinding
/// honored per datagram). Exits when the allocation expires or is deleted.
fn relay_reader_loop(
    alloc_id: uuid::Uuid,
    relay_socket: std::sync::Arc<UdpSocket>,
    front_socket: UdpSocket,
    turn: Arc<Mutex<TurnState>>,
) {
    let _ = relay_socket.set_read_timeout(Some(RECV_TIMEOUT));
    let mut buffer = [0_u8; TURN_RECV_BYTES];
    loop {
        match relay_socket.recv_from(&mut buffer) {
            Ok((count, source)) => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_secs())
                    .unwrap_or(0);
                let (reply, client) = {
                    let mut state = turn.lock().unwrap_or_else(|poison| poison.into_inner());
                    if !state.allocation_active(&alloc_id, now) {
                        break;
                    }
                    let reply = state.relay_receipt(&alloc_id, source, &buffer[..count], now);
                    let client = state.allocation_client(&alloc_id);
                    (reply, client)
                };
                if let (Some(bytes), Some(client)) = (reply, client) {
                    let _ = front_socket.send_to(&bytes, client);
                }
            }
            Err(error)
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::TimedOut =>
            {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_secs())
                    .unwrap_or(0);
                let active = {
                    let mut state = turn.lock().unwrap_or_else(|poison| poison.into_inner());
                    state.allocation_active(&alloc_id, now)
                };
                if !active {
                    break;
                }
            }
            Err(_) => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|elapsed| elapsed.as_secs())
                    .unwrap_or(0);
                let active = {
                    let mut state = turn.lock().unwrap_or_else(|poison| poison.into_inner());
                    state.allocation_active(&alloc_id, now)
                };
                if !active {
                    break;
                }
                // Hard errors (not timeouts) would spin at 100% CPU until
                // expiry; back off a little before retrying.
                std::thread::sleep(Duration::from_millis(20));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    fn spawned(bind: &str) -> Option<StunServer> {
        match StunServer::spawn(
            bind.parse().unwrap(),
            Arc::new(Mutex::new(TurnState::new())),
        ) {
            Ok(server) => Some(server),
            Err(error) => {
                eprintln!("skipping STUN test: cannot bind {bind} ({error})");
                None
            }
        }
    }

    fn recv_response(socket: &UdpSocket) -> Option<Vec<u8>> {
        let mut buffer = [0_u8; stun::MAX_DATAGRAM_BYTES];
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            match socket.recv(&mut buffer) {
                Ok(count) => return Some(buffer[..count].to_vec()),
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        || error.kind() == io::ErrorKind::TimedOut =>
                {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(_) => return None,
            }
        }
        None
    }

    #[test]
    fn well_formed_requests_get_nonce_and_source_echoed() {
        let Some(server) = spawned("127.0.0.1:0") else {
            return;
        };
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        client
            .send_to(
                &stun::request_bytes("probe-1").unwrap(),
                server.local_addr(),
            )
            .unwrap();

        let reply = recv_response(&client).expect("a STUN-lite echo");
        let echo = stun::parse_response(&reply, "probe-1").expect("valid echo");
        assert_eq!(echo.nonce, "probe-1");
        assert_eq!(echo.address.to_string(), "127.0.0.1");
        assert_eq!(echo.port, client.local_addr().unwrap().port());

        server.shutdown();
    }

    #[test]
    fn rfc_binding_requests_get_xor_mapped_answers() {
        let Some(server) = spawned("127.0.0.1:0") else {
            return;
        };
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let mut request = vec![0x00, 0x01, 0x00, 0x00, 0x21, 0x12, 0xA4, 0x42];
        request.extend([9_u8, 8, 7, 6, 5, 4, 3, 2, 1, 0, 9, 8]);
        client.send_to(&request, server.local_addr()).unwrap();

        let reply = recv_response(&client).expect("a binding response");
        assert!(reply.len() >= 32);
        assert_eq!(&reply[..2], &[0x01, 0x01]);
        assert_eq!(&reply[8..20], &request[8..20]);
        let port = u16::from_be_bytes([reply[26], reply[27]]) ^ 0x2112;
        assert_eq!(port, client.local_addr().unwrap().port());
        assert_eq!(&reply[24..26], &[0x00, 0x01]);

        server.shutdown();
    }

    #[test]
    fn turn_allocate_without_credentials_gets_a_401_challenge() {
        let Some(server) = spawned("127.0.0.1:0") else {
            return;
        };
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        // Bare Allocate request (method 0x0003, UDP transport, no auth):
        // must demux to TURN and challenge, proving the shared state and
        // the relay address are wired through the real socket.
        let mut allocate = vec![0x00, 0x03, 0x00, 0x08, 0x21, 0x12, 0xA4, 0x42];
        allocate.extend([7_u8, 6, 5, 4, 3, 2, 1, 0, 9, 8, 7, 6]);
        allocate.extend([0x00, 0x19, 0x00, 0x04, 0x11, 0x00, 0x00, 0x00]);
        client.send_to(&allocate, server.local_addr()).unwrap();

        let reply = recv_response(&client).expect("a 401 challenge");
        assert_eq!(&reply[..2], &[0x01, 0x13]);
        let code = find_error_code(&reply).expect("an ERROR-CODE attribute");
        assert_eq!(code, 401);

        server.shutdown();
    }

    /// Minimal ERROR-CODE scan: the full attribute parser lives in TURN
    /// unit tests; here only the class/number of the first ERROR-CODE
    /// matters (proves challenge, not success).
    fn find_error_code(reply: &[u8]) -> Option<u16> {
        let announced = u16::from_be_bytes([reply[2], reply[3]]) as usize;
        let mut offset = 20_usize;
        while offset + 4 <= reply.len() && offset < 20 + announced {
            let attr_type = u16::from_be_bytes([reply[offset], reply[offset + 1]]);
            let attr_len = u16::from_be_bytes([reply[offset + 2], reply[offset + 3]]) as usize;
            if attr_type == 0x0009 && attr_len >= 4 && offset + 4 + attr_len <= reply.len() {
                let class = reply[offset + 6];
                let number = reply[offset + 7];
                return Some(u16::from(class) * 100 + u16::from(number));
            }
            offset += 4 + attr_len.next_multiple_of(4);
        }
        None
    }

    #[test]
    fn garbage_oversize_and_wrong_shape_are_silently_dropped() {
        let Some(server) = spawned("127.0.0.1:0") else {
            return;
        };
        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        for datagram in [
            b"not json".to_vec(),
            b"{}".to_vec(),
            serde_json::to_vec(&serde_json::json!({"type": "stun_lite_request"})).unwrap(),
            vec![b'x'; stun::MAX_DATAGRAM_BYTES + 1],
        ] {
            client.send_to(&datagram, server.local_addr()).unwrap();
        }
        // One shared silence window for all four: any reply at all fails.
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut buffer = [0_u8; stun::MAX_DATAGRAM_BYTES];
        while Instant::now() < deadline {
            match client.recv(&mut buffer) {
                Ok(count) => panic!("datagram must get no reply, got {:?}", &buffer[..count]),
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock
                        || error.kind() == io::ErrorKind::TimedOut =>
                {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(error) => panic!("rendezvous socket recv failed: {error}"),
            }
        }

        server.shutdown();
    }
}
