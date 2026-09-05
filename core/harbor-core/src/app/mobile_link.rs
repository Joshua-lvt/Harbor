//! Direct TCP bearer between Harbor endpoints — the mobile path.
//!
//! Desktop-to-desktop calls ride the Pion worker's DataChannels. A phone
//! has no worker (Android cannot spawn the core's child processes), so a
//! mobile endpoint talks to its desktop peer over this bearer instead:
//! one TLS-over-TCP connection, dialed outward by the mobile, carrying the
//! same chat, profile, and phone-state frames the DataChannel would carry.
//!
//! Trust, in order:
//!
//! 1. The desktop listens only while a validated `device_hello` says a
//!    mobile peer exists, and advertises `{addrs, port, fingerprint}` to
//!    that peer alone inside a signed control-plane signal (opaque relay).
//! 2. The dialer pins the served certificate to the invited fingerprint —
//!    the same pinning the control plane uses, same SHA-256 of the DER.
//! 3. Both sides exchange hellos; the listener additionally challenges the
//!    dialer to sign a fresh nonce with its Ed25519 identity key, verified
//!    against the paired record's public key. A hello that does not name
//!    the authenticated peer, or a failed challenge, closes the connection.
//!
//! Framing is 4-byte big-endian length plus JSON `{kind, payload}`, capped
//! at 1 MiB like the local IPC. Content policy stays in the core: chat
//! bodies pass the same sanitizer, profile frames pass the same ingest,
//! phone state passes the same consent validation. This module moves bytes
//! and proves identities; it never decides what anything means.
//!
//! All sockets live on one worker thread. The core talks to it through two
//! bounded channels and never touches a socket — threads plus std::net
//! only, so the same code runs on desktop and Android.

use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::device::DeviceType;

pub const LINK_MAX_FRAME_BYTES: usize = 1024 * 1024;
/// Per-address dial budget: the invite lists several addrs, each fails fast.
pub const LINK_DIAL_TIMEOUT: Duration = Duration::from_secs(3);
/// One handshake (TLS + hello + challenge) must finish inside this.
pub const LINK_HANDSHAKE_DEADLINE: Duration = Duration::from_secs(10);
/// Frame kinds. `challenge`/`challenge_response` are handshake-internal.
pub const KIND_HELLO: &str = "hello";
pub const KIND_CHAT: &str = "chat";
pub const KIND_CHAT_ACK: &str = "chat_ack";
pub const KIND_PROFILE: &str = "profile";
pub const KIND_MOBILE: &str = "mobile_status";
pub const KIND_PHONE_NOTIFICATION: &str = "phone_notification";
pub const KIND_CHALLENGE: &str = "challenge";
pub const KIND_CHALLENGE_RESPONSE: &str = "challenge_response";
pub const KIND_BYE: &str = "bye";
/// Bearer-level keepalive. The worker answers pings and consumes pongs
/// itself; the core never sees them.
pub const KIND_PING: &str = "ping";
pub const KIND_PONG: &str = "pong";

/// One validated link frame.
#[derive(Debug, Clone, PartialEq)]
pub struct LinkFrame {
    pub kind: String,
    pub payload: Value,
}

impl LinkFrame {
    pub fn new(kind: &str, payload: Value) -> Option<Self> {
        if kind.is_empty() || kind.len() > 64 || !payload.is_object() {
            return None;
        }
        Some(Self {
            kind: kind.to_owned(),
            payload,
        })
    }

    fn encode(&self) -> Vec<u8> {
        let mut bytes = serde_json::to_vec(&json!({"kind": self.kind, "payload": self.payload}))
            .unwrap_or_default();
        bytes.truncate(LINK_MAX_FRAME_BYTES);
        let mut frame = (bytes.len() as u32).to_be_bytes().to_vec();
        frame.extend_from_slice(&bytes);
        frame
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        let value: Value = serde_json::from_slice(bytes).ok()?;
        let kind = value.get("kind")?.as_str()?;
        let payload = value.get("payload")?.clone();
        LinkFrame::new(kind, payload)
    }
}

/// A paired peer this bearer may talk to: addressing plus the public key
/// the challenge verifies against.
#[derive(Debug, Clone)]
pub struct LinkPeer {
    pub device_id: Uuid,
    pub harbor_id: String,
    pub public_key: String,
}

/// What the worker thread reports. `Frame` always carries the
/// TLS-and-challenge-authenticated sender.
#[derive(Debug)]
pub enum LinkEvent {
    Listening { port: u16, fingerprint_hex: String },
    ListenerFailed,
    Connected { peer: Uuid, harbor: String, device: DeviceType },
    Frame { peer: Uuid, frame: LinkFrame },
    Disconnected,
}

#[derive(Debug)]
enum LinkCommand {
    Listen { context: LinkContext },
    Dial { context: LinkContext, invite: LinkInvite },
    Send(LinkFrame),
    Shutdown,
}

/// Everything the worker needs, snapshotted when the core commands it:
/// our identity, our kind, and the paired peers we accept.
#[derive(Debug, Clone)]
pub struct LinkContext {
    pub device_id: Uuid,
    pub harbor_id: String,
    pub signing_seed: [u8; 32],
    pub device_type: DeviceType,
    pub peers: Vec<LinkPeer>,
}

/// A validated dial invitation from the listening peer.
#[derive(Debug, Clone)]
pub struct LinkInvite {
    pub addrs: Vec<String>,
    pub port: u16,
    pub fingerprint: [u8; 32],
}

impl LinkInvite {
    /// Parses an invite signal payload. Bounds are tight: a handful of
    /// literal addresses, a real port, 64 hex chars. Anything else is not
    /// an invite.
    pub fn parse(payload: &Value) -> Option<Self> {
        let addrs = payload.get("addrs")?.as_array()?;
        if addrs.is_empty() || addrs.len() > 8 {
            return None;
        }
        let mut parsed = Vec::new();
        for addr in addrs {
            let addr = addr.as_str()?;
            if addr.is_empty() || addr.len() > 253 {
                return None;
            }
            parsed.push(addr.to_owned());
        }
        let port = payload.get("port")?.as_u64()?;
        if port == 0 || port > 65535 {
            return None;
        }
        let fingerprint = parse_fingerprint_hex(payload.get("fingerprint")?.as_str()?)?;
        Some(Self {
            addrs: parsed,
            port: port as u16,
            fingerprint,
        })
    }
}

pub fn parse_fingerprint_hex(raw: &str) -> Option<[u8; 32]> {
    let raw = raw.trim();
    if raw.len() != 64 {
        return None;
    }
    let mut out = [0_u8; 32];
    for (index, chunk) in raw.as_bytes().chunks(2).enumerate() {
        let high = (chunk[0] as char).to_digit(16)?;
        let low = (chunk[1] as char).to_digit(16)?;
        out[index] = ((high << 4) | low) as u8;
    }
    Some(out)
}

pub fn fingerprint_hex(fingerprint: &[u8; 32]) -> String {
    fingerprint.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// The address this machine most likely answers on for the invited peer:
/// the source IP toward the control-plane server, plus loopback (which an
/// adb-reverse tunnel turns into the real path on emulators).
pub fn local_dial_addrs(server_addr: &str) -> Vec<String> {
    let mut addrs = Vec::new();
    if let Ok(socket) = UdpSocket::bind("0.0.0.0:0") {
        if socket.connect(server_addr).is_ok() {
            if let Ok(local) = socket.local_addr() {
                let ip = local.ip().to_string();
                if !ip.starts_with("127.") && ip != "::1" {
                    addrs.push(ip);
                }
            }
        }
    }
    addrs.push("127.0.0.1".to_owned());
    addrs
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(raw: &str) -> Option<Vec<u8>> {
    if raw.len() % 2 != 0 || raw.is_empty() || raw.len() > 256 {
        return None;
    }
    let mut out = Vec::with_capacity(raw.len() / 2);
    for chunk in raw.as_bytes().chunks(2) {
        let high = (chunk[0] as char).to_digit(16)?;
        let low = (chunk[1] as char).to_digit(16)?;
        out.push(((high << 4) | low) as u8);
    }
    Some(out)
}

/// Core-side handle. Owns the worker thread and the live-peer facts the
/// snapshot reads; sockets never leave the worker.
pub(crate) struct MobileLink {
    cmd_tx: Option<mpsc::Sender<LinkCommand>>,
    events_rx: Option<mpsc::Receiver<LinkEvent>>,
    worker: Option<std::thread::JoinHandle<()>>,
    shutdown_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    live_peer: Option<Uuid>,
    live_harbor: Option<String>,
    live_device: Option<DeviceType>,
    listening_port: Option<u16>,
    listening_fingerprint: Option<String>,
}

impl MobileLink {
    pub(crate) fn new() -> Self {
        Self {
            cmd_tx: None,
            events_rx: None,
            worker: None,
            shutdown_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            live_peer: None,
            live_harbor: None,
            live_device: None,
            listening_port: None,
            listening_fingerprint: None,
        }
    }

    pub(crate) fn live_peer(&self) -> Option<Uuid> {
        self.live_peer
    }

    pub(crate) fn live_device(&self) -> Option<DeviceType> {
        self.live_device
    }

    pub(crate) fn listening(&self) -> Option<(u16, &str)> {
        match (self.listening_port, self.listening_fingerprint.as_deref()) {
            (Some(port), Some(fingerprint)) => Some((port, fingerprint)),
            _ => None,
        }
    }

    fn ensure_worker(&mut self) {
        if self.cmd_tx.is_some() {
            return;
        }
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, events_rx) = mpsc::channel();
        let shutdown = std::sync::Arc::clone(&self.shutdown_flag);
        let worker = std::thread::Builder::new()
            .name("harbor-link".into())
            .spawn(move || run_worker(cmd_rx, event_tx, shutdown))
            .ok();
        self.cmd_tx = Some(cmd_tx);
        self.events_rx = Some(events_rx);
        self.worker = worker;
    }

    pub(crate) fn listen(&mut self, context: LinkContext) {
        self.ensure_worker();
        if let Some(tx) = self.cmd_tx.as_ref() {
            let _ = tx.send(LinkCommand::Listen { context });
        }
    }

    pub(crate) fn dial(&mut self, context: LinkContext, invite: LinkInvite) {
        self.ensure_worker();
        if let Some(tx) = self.cmd_tx.as_ref() {
            let _ = tx.send(LinkCommand::Dial { context, invite });
        }
    }

    /// Queues one frame for the live connection. Silently dropped without
    /// one — the core only sends while `live_peer` is set.
    pub(crate) fn send(&mut self, frame: LinkFrame) {
        if self.live_peer.is_none() {
            return;
        }
        if let Some(tx) = self.cmd_tx.as_ref() {
            let _ = tx.send(LinkCommand::Send(frame));
        }
    }

    /// Drains worker events and folds connection facts into this handle.
    /// Returns the frame events for the core to apply.
    pub(crate) fn drain(&mut self) -> Vec<(Uuid, LinkFrame)> {
        let mut frames = Vec::new();
        let events: Vec<LinkEvent> = self
            .events_rx
            .as_ref()
            .map(|rx| rx.try_iter().collect())
            .unwrap_or_default();
        for event in events {
            match event {
                LinkEvent::Listening { port, fingerprint_hex } => {
                    self.listening_port = Some(port);
                    self.listening_fingerprint = Some(fingerprint_hex);
                }
                LinkEvent::ListenerFailed => {
                    self.listening_port = None;
                    self.listening_fingerprint = None;
                }
                LinkEvent::Connected { peer, harbor, device } => {
                    self.live_peer = Some(peer);
                    self.live_harbor = Some(harbor);
                    self.live_device = Some(device);
                }
                LinkEvent::Frame { peer, frame } => {
                    if Some(peer) == self.live_peer {
                        frames.push((peer, frame));
                    }
                }
                LinkEvent::Disconnected => {
                    self.live_peer = None;
                    self.live_harbor = None;
                    self.live_device = None;
                }
            }
        }
        frames
    }

    pub(crate) fn shutdown(&mut self) {
        self.shutdown_flag
            .store(true, std::sync::atomic::Ordering::SeqCst);
        if let Some(tx) = self.cmd_tx.take() {
            let _ = tx.send(LinkCommand::Shutdown);
        }
        // A worker parked in a blocking accept only wakes for a connection:
        // give it a dummy one. The flag above makes it exit instead of
        // serving the wakeup.
        if let Some(port) = self.listening_port {
            let _ = TcpStream::connect_timeout(
                &SocketAddr::from(([127, 0, 0, 1], port)),
                Duration::from_millis(500),
            );
        }
        self.events_rx = None;
        self.live_peer = None;
        self.listening_port = None;
        self.listening_fingerprint = None;
    }
}

impl Drop for MobileLink {
    fn drop(&mut self) {
        self.shutdown();
    }
}

type TlsStream = rustls::StreamOwned<rustls::ServerConnection, TcpStream>;
type TlsClientStream = rustls::StreamOwned<rustls::ClientConnection, TcpStream>;

enum WorkerConn {
    Tls(TlsStream),
    Plain(TlsClientStream),
}

impl WorkerConn {
    fn read_frame(&mut self, timeout: Duration) -> Result<Option<LinkFrame>, ConnEnd> {
        set_timeout(self, Some(timeout))?;
        let mut prefix = [0_u8; 4];
        if let Err(error) = read_exact(self, &mut prefix) {
            return match error {
                ConnEnd::Timeout => Ok(None),
                fatal => Err(fatal),
            };
        }
        let length = u32::from_be_bytes(prefix) as usize;
        if length == 0 || length > LINK_MAX_FRAME_BYTES {
            return Err(ConnEnd::Protocol);
        }
        let mut body = vec![0_u8; length];
        if read_exact(self, &mut body).is_err() {
            return Err(ConnEnd::Closed);
        }
        LinkFrame::decode(&body).map(Some).ok_or(ConnEnd::Protocol)
    }

    fn write_frame(&mut self, frame: &LinkFrame) -> Result<(), ConnEnd> {
        set_timeout(self, Some(Duration::from_secs(5)))?;
        write_all(self, &frame.encode()).map_err(|_| ConnEnd::Closed)
    }
}

#[derive(Debug)]
enum ConnEnd {
    Closed,
    Protocol,
    Timeout,
}

fn set_timeout(conn: &mut WorkerConn, timeout: Option<Duration>) -> Result<(), ConnEnd> {
    let socket = match conn {
        WorkerConn::Tls(stream) => stream.get_mut(),
        WorkerConn::Plain(stream) => stream.get_mut(),
    };
    socket
        .set_read_timeout(timeout)
        .map_err(|_| ConnEnd::Closed)?;
    Ok(())
}

fn read_exact(conn: &mut WorkerConn, mut buf: &mut [u8]) -> Result<(), ConnEnd> {
    use std::io::Read as _;
    while !buf.is_empty() {
        let stream: &mut dyn std::io::Read = match conn {
            WorkerConn::Tls(stream) => stream,
            WorkerConn::Plain(stream) => stream,
        };
        match stream.read(buf) {
            Ok(0) => return Err(ConnEnd::Closed),
            Ok(count) => {
                buf = &mut buf[count..];
            }
            Err(error)
                if error.kind() == io::ErrorKind::TimedOut
                    || error.kind() == io::ErrorKind::WouldBlock =>
            {
                return Err(ConnEnd::Timeout)
            }
            Err(_) => return Err(ConnEnd::Closed),
        }
    }
    Ok(())
}

fn write_all(conn: &mut WorkerConn, mut buf: &[u8]) -> Result<(), ()> {
    use std::io::Write as _;
    while !buf.is_empty() {
        let stream: &mut dyn std::io::Write = match conn {
            WorkerConn::Tls(stream) => stream,
            WorkerConn::Plain(stream) => stream,
        };
        match stream.write(buf) {
            Ok(0) => return Err(()),
            Ok(count) => {
                buf = &buf[count..];
                let _ = stream.flush();
            }
            Err(_) => return Err(()),
        }
    }
    Ok(())
}

fn generate_identity() -> Option<(Vec<u8>, Vec<u8>)> {
    let key_pair = rcgen::KeyPair::generate().ok()?;
    let mut params = rcgen::CertificateParams::new(vec!["harbor-link".to_owned()]).ok()?;
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "harbor-link");
    let certificate = params.self_signed(&key_pair).ok()?;
    Some((
        certificate.der().to_vec(),
        key_pair.serialize_der(),
    ))
}

#[derive(Debug)]
struct PinVerifier {
    fingerprint: [u8; 32],
    rejected: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl rustls::client::danger::ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let served: [u8; 32] = Sha256::digest(end_entity.as_ref()).into();
        if served == self.fingerprint {
            return Ok(rustls::client::danger::ServerCertVerified::assertion());
        }
        self.rejected
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Err(rustls::Error::InvalidCertificate(
            rustls::CertificateError::UnknownIssuer,
        ))
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            certificate,
            signature,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        certificate: &CertificateDer<'_>,
        signature: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            certificate,
            signature,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn server_config(cert_der: Vec<u8>, key_der: Vec<u8>) -> Option<std::sync::Arc<rustls::ServerConfig>> {
    let certificate = CertificateDer::from(cert_der);
    let key = PrivateKeyDer::Pkcs8(key_der.into());
    rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate], key)
        .ok()
        .map(std::sync::Arc::new)
}

fn client_config(fingerprint: [u8; 32]) -> std::sync::Arc<rustls::ClientConfig> {
    let verifier = PinVerifier {
        fingerprint,
        rejected: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    std::sync::Arc::new(
        rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(std::sync::Arc::new(verifier))
            .with_no_client_auth(),
    )
}

fn link_server_name() -> ServerName<'static> {
    ServerName::try_from("harbor-link".to_owned()).expect("constant server name")
}

/// The dialer's hello: who we are. The listener answers with its own.
pub(crate) fn hello_frame(context: &LinkContext) -> LinkFrame {
    LinkFrame::new(
        KIND_HELLO,
        json!({
            "device_id": context.device_id.to_string(),
            "harbor_id": context.harbor_id,
            "device_type": context.device_type.as_str(),
        }),
    )
    .expect("constant hello shape")
}

fn find_peer<'a>(context: &'a LinkContext, device_id: &Uuid) -> Option<&'a LinkPeer> {
    context.peers.iter().find(|peer| &peer.device_id == device_id)
}

/// Validates a received hello against the authenticated connection: the
/// claimed device must be the peer this connection proved, with the paired
/// harbor record. Returns the claimed kind.
fn accept_hello(context: &LinkContext, peer: &Uuid, frame: &LinkFrame) -> Option<DeviceType> {
    if frame.kind != KIND_HELLO {
        return None;
    }
    let claimed_id = frame
        .payload
        .get("device_id")
        .and_then(Value::as_str)
        .and_then(|id| Uuid::parse_str(id).ok())?;
    if &claimed_id != peer {
        return None;
    }
    let known = find_peer(context, peer)?;
    let claimed_harbor = frame.payload.get("harbor_id")?.as_str()?;
    if claimed_harbor != known.harbor_id {
        return None;
    }
    frame
        .payload
        .get("device_type")
        .and_then(Value::as_str)
        .and_then(DeviceType::parse)
}

fn verify_signature(public_key: &str, message: &[u8], signature_hex: &str) -> bool {
    let key_bytes = STANDARD_NO_PAD.decode(public_key).unwrap_or_default();
    let key_bytes: Option<[u8; 32]> = key_bytes.try_into().ok();
    let signature_bytes = unhex(signature_hex).unwrap_or_default();
    let (Some(key_bytes), Ok(signature)) = (
        key_bytes,
        Signature::from_slice(&signature_bytes),
    ) else {
        return false;
    };
    let Ok(key) = VerifyingKey::from_bytes(&key_bytes) else {
        return false;
    };
    key.verify(message, &signature).is_ok()
}

fn fresh_nonce(context: &LinkContext, peer: &Uuid) -> Vec<u8> {
    use std::time::{SystemTime, UNIX_EPOCH};
    static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_nanos())
        .unwrap_or(0);
    let count = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut hasher = Sha256::new();
    hasher.update(nanos.to_be_bytes());
    hasher.update(std::process::id().to_be_bytes());
    hasher.update(count.to_be_bytes());
    hasher.update(peer.as_bytes());
    hasher.update(context.device_id.as_bytes());
    hasher.finalize().to_vec()
}

fn sign_nonce(context: &LinkContext, nonce: &[u8]) -> [u8; 64] {
    use ed25519_dalek::{Signer as _, SigningKey};
    SigningKey::from_bytes(&context.signing_seed).sign(nonce).to_bytes()
}

/// One authenticated connection: frames flow until bye, timeout, or any
/// protocol violation. Returns when the connection is over.
fn serve_connection(
    conn: &mut WorkerConn,
    context: &LinkContext,
    authenticated: &AuthenticatedPeer,
    events: &mpsc::Sender<LinkEvent>,
    commands: &mpsc::Receiver<LinkCommand>,
    shutdown: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    deadline: Instant,
) {
    let _ = context;
    if shutdown_requested(shutdown) {
        return;
    }
    let mut last_rx = Instant::now();
    let mut ping_outstanding = false;
    loop {
        if Instant::now() > deadline {
            return;
        }
        // Outbound first: shutdown and queued frames must not wait on a
        // silent peer.
        let mut closed = false;
        for command in commands.try_iter() {
            match command {
                LinkCommand::Send(frame) => {
                    if conn.write_frame(&frame).is_err() {
                        return;
                    }
                }
                LinkCommand::Shutdown => {
                    let bye = LinkFrame::new(KIND_BYE, json!({})).expect("constant bye");
                    let _ = conn.write_frame(&bye);
                    closed = true;
                    break;
                }
                // A second Listen/Dial while connected is a no-op: one link.
                LinkCommand::Listen { .. } | LinkCommand::Dial { .. } => {}
            }
        }
        if closed {
            return;
        }
        match conn.read_frame(Duration::from_millis(200)) {
            Ok(None) => {}
            Ok(Some(frame)) => {
                last_rx = Instant::now();
                ping_outstanding = false;
                if frame.kind == KIND_BYE {
                    return;
                }
                if frame.kind == KIND_PING {
                    let pong = LinkFrame::new(KIND_PONG, json!({})).expect("constant pong");
                    if conn.write_frame(&pong).is_err() {
                        return;
                    }
                    continue;
                }
                if frame.kind == KIND_PONG {
                    continue;
                }
                let _ = events.send(LinkEvent::Frame {
                    peer: authenticated.device_id,
                    frame,
                });
            }
            Err(ConnEnd::Timeout) => {}
            Err(_) => return,
        }
        // Bearer keepalive: a ping after 30 s of silence, death after 75 s
        // with no reply. Content liveness stays the core's business.
        let quiet = last_rx.elapsed();
        if quiet > Duration::from_secs(75) {
            return;
        }
        if quiet > Duration::from_secs(30) && !ping_outstanding {
            let ping = LinkFrame::new(KIND_PING, json!({})).expect("constant ping");
            if conn.write_frame(&ping).is_err() {
                return;
            }
            ping_outstanding = true;
        }
    }
}

#[derive(Debug, Clone)]
struct AuthenticatedPeer {
    device_id: Uuid,
    harbor: String,
    device: DeviceType,
}

macro_rules! set_timeout_or_return {
    ($conn:expr) => {
        if set_timeout($conn, Some(LINK_HANDSHAKE_DEADLINE)).is_err() {
            return;
        }
    };
}

/// Listener side: TLS-accept, hello, challenge, then serve.
fn accept_one(
    socket: TcpStream,
    config: &std::sync::Arc<rustls::ServerConfig>,
    context: &LinkContext,
    events: &mpsc::Sender<LinkEvent>,
    commands: &mpsc::Receiver<LinkCommand>,
    shutdown: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let _ = socket.set_read_timeout(Some(Duration::from_secs(5)));
    let server_conn = match rustls::ServerConnection::new(std::sync::Arc::clone(config)) {
        Ok(conn) => conn,
        Err(_) => return,
    };
    if shutdown_requested(shutdown) {
        return;
    }
    let mut conn = WorkerConn::Tls(rustls::StreamOwned::new(server_conn, socket));
    // Handshake inside the deadline: drive it through one bounded read.
    set_timeout_or_return!(&mut conn);
    // Hello first.
    let Some(frame) = read_handshake_frame(&mut conn) else {
        return;
    };
    // The hello names the dialer, but nothing is trusted yet: the challenge
    // below proves the key before any event names a peer.
    let claimed = frame
        .payload
        .get("device_id")
        .and_then(Value::as_str)
        .and_then(|id| Uuid::parse_str(id).ok());
    let Some(claimed) = claimed else { return };
    let Some(known) = find_peer(context, &claimed) else {
        return;
    };
    if shutdown_requested(shutdown) {
        return;
    }
    let nonce = fresh_nonce(context, &claimed);
    let challenge = LinkFrame::new(KIND_CHALLENGE, json!({"nonce": hex(&nonce)}))
        .expect("constant challenge");
    if conn.write_frame(&challenge).is_err() {
        return;
    }
    let Some(answer) = read_handshake_frame(&mut conn) else {
        return;
    };
    if answer.kind != KIND_CHALLENGE_RESPONSE {
        return;
    }
    let signature = answer
        .payload
        .get("signature")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !verify_signature(&known.public_key, &nonce, signature) {
        return;
    }
    // Proven: now the hello's harbor and kind bind to this connection.
    let Some(device) = accept_hello(context, &claimed, &frame) else {
        return;
    };
    let authenticated = AuthenticatedPeer {
        device_id: claimed,
        harbor: known.harbor_id.clone(),
        device,
    };
    // Answer with our own hello so the dialer learns our kind too.
    if conn.write_frame(&hello_frame(context)).is_err() {
        return;
    }
    let _ = events.send(LinkEvent::Connected {
        peer: authenticated.device_id,
        harbor: authenticated.harbor.clone(),
        device: authenticated.device,
    });
    serve_connection(
        &mut conn,
        context,
        &authenticated,
        events,
        commands,
        shutdown,
        Instant::now() + Duration::from_secs(3600),
    );
    let _ = events.send(LinkEvent::Disconnected);
}

fn read_handshake_frame(conn: &mut WorkerConn) -> Option<LinkFrame> {
    match conn.read_frame(LINK_HANDSHAKE_DEADLINE) {
        Ok(Some(frame)) => Some(frame),
        _ => None,
    }
}

/// Dialer side: connect, pin the certificate, hello, answer the challenge.
fn dial_one(
    invite: &LinkInvite,
    context: &LinkContext,
    events: &mpsc::Sender<LinkEvent>,
    commands: &mpsc::Receiver<LinkCommand>,
    shutdown: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    for addr in &invite.addrs {
        if shutdown_requested(shutdown) {
            return;
        }
        let socket_addr = socket_addrs(addr, invite.port);
        let socket = match TcpStream::connect_timeout(&socket_addr, LINK_DIAL_TIMEOUT) {
            Ok(socket) => socket,
            Err(_) => continue,
        };
        let _ = socket.set_nodelay(true);
        let client_conn = match rustls::ClientConnection::new(client_config(invite.fingerprint), link_server_name()) {
            Ok(conn) => conn,
            Err(_) => continue,
        };
        let mut conn = WorkerConn::Plain(rustls::StreamOwned::new(client_conn, socket));
        // The handshake must complete inside the deadline; a pinned-cert
        // refusal fails here, before any Harbor bytes move.
        if set_timeout(&mut conn, Some(LINK_HANDSHAKE_DEADLINE)).is_err() {
            continue;
        }
        // Touch the connection once so the handshake (and the pin check)
        // actually runs before the hello leaves.
        if probe_handshake(&mut conn).is_err() {
            continue;
        }
        if conn.write_frame(&hello_frame(context)).is_err() {
            continue;
        }
        // The listener challenges, then answers with its own hello.
        let mut authenticated: Option<AuthenticatedPeer> = None;
        let deadline = Instant::now() + LINK_HANDSHAKE_DEADLINE;
        let peer = loop {
            if Instant::now() > deadline {
                break None;
            }
            let Some(frame) = read_handshake_frame(&mut conn) else {
                break None;
            };
            if frame.kind == KIND_CHALLENGE {
                let Some(nonce_hex) = frame.payload.get("nonce").and_then(Value::as_str) else {
                    break None;
                };
                let Some(nonce) = unhex(nonce_hex) else {
                    break None;
                };
                let signature = hex(&sign_nonce(context, &nonce));
                let answer = LinkFrame::new(
                    KIND_CHALLENGE_RESPONSE,
                    json!({"signature": signature}),
                )
                .expect("constant answer");
                if conn.write_frame(&answer).is_err() {
                    break None;
                }
            } else if frame.kind == KIND_HELLO {
                // The certificate pin already bound this connection to the
                // invited peer; the hello names which device answered.
                let claimed = frame
                    .payload
                    .get("device_id")
                    .and_then(Value::as_str)
                    .and_then(|id| Uuid::parse_str(id).ok());
                let (Some(claimed), Some(device)) = (
                    claimed,
                    frame
                        .payload
                        .get("device_type")
                        .and_then(Value::as_str)
                        .and_then(DeviceType::parse),
                ) else {
                    break None;
                };
                let Some(known) = find_peer(context, &claimed) else {
                    break None;
                };
                let harbor = frame
                    .payload
                    .get("harbor_id")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if harbor != known.harbor_id {
                    break None;
                }
                authenticated = Some(AuthenticatedPeer {
                    device_id: claimed,
                    harbor: known.harbor_id.clone(),
                    device,
                });
                break authenticated.clone();
            } else {
                break None;
            }
        };
        let Some(authenticated) = peer else {
            continue;
        };
        let _ = events.send(LinkEvent::Connected {
            peer: authenticated.device_id,
            harbor: authenticated.harbor.clone(),
            device: authenticated.device,
        });
        serve_connection(
            &mut conn,
            context,
            &authenticated,
            events,
            commands,
            shutdown,
            Instant::now() + Duration::from_secs(3600),
        );
        let _ = events.send(LinkEvent::Disconnected);
        return;
    }
}

fn probe_handshake(conn: &mut WorkerConn) -> Result<(), ConnEnd> {
    // Drive the TLS handshake to completion before any Harbor bytes move,
    // so a pinned-certificate refusal fails here with its cause intact.
    match conn {
        WorkerConn::Plain(stream) => stream.conn.complete_io(&mut stream.sock),
        WorkerConn::Tls(stream) => stream.conn.complete_io(&mut stream.sock),
    }
    .map(|_| ())
    .map_err(|_| ConnEnd::Closed)
}

fn socket_addrs(addr: &str, port: u16) -> SocketAddr {
    if addr.contains(':') {
        format!("[{addr}]:{port}")
            .parse()
            .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], port)))
    } else {
        format!("{addr}:{port}")
            .parse()
            .unwrap_or_else(|_| SocketAddr::from(([127, 0, 0, 1], port)))
    }
}

fn shutdown_requested(shutdown: &std::sync::Arc<std::sync::atomic::AtomicBool>) -> bool {
    shutdown.load(std::sync::atomic::Ordering::SeqCst)
}

fn run_worker(
    commands: mpsc::Receiver<LinkCommand>,
    events: mpsc::Sender<LinkEvent>,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let mut listener: Option<(TcpListener, std::sync::Arc<rustls::ServerConfig>)> = None;
    let mut context: Option<LinkContext> = None;
    loop {
        if shutdown_requested(&shutdown) {
            return;
        }
        // The accept below blocks, deliberately: a parked listener costs no
        // CPU, and shutdown arrives as a flag plus a dummy loopback
        // connection that this same accept returns. The dummy carries no
        // handshake, so the flag check after accept exits before serving it.
        if let Some((tcp, config)) = listener.as_ref() {
            match tcp.accept() {
                Ok((socket, _)) => {
                    if shutdown_requested(&shutdown) {
                        return;
                    }
                    if let Some(context) = context.clone() {
                        accept_one(socket, config, &context, &events, &commands, &shutdown);
                    }
                }
                Err(_) => {
                    let _ = events.send(LinkEvent::ListenerFailed);
                    listener = None;
                }
            }
            continue;
        }
        match commands.recv_timeout(Duration::from_millis(100)) {
            Ok(LinkCommand::Shutdown) => return,
            Ok(LinkCommand::Listen { context: next }) => {
                bind_listener(&mut listener, &mut context, next, &events);
            }
            Ok(LinkCommand::Dial { context: next, invite }) => {
                context = Some(next.clone());
                dial_one(&invite, &next, &events, &commands, &shutdown);
            }
            // Sends only make sense on a live connection, which owns its
            // serve loop; a stray send here means the core raced a
            // disconnect and is safely ignored.
            Ok(LinkCommand::Send(_)) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    }
}

fn bind_listener(
    listener: &mut Option<(TcpListener, std::sync::Arc<rustls::ServerConfig>)>,
    context: &mut Option<LinkContext>,
    next: LinkContext,
    events: &mpsc::Sender<LinkEvent>,
) {
    *context = Some(next);
    if listener.is_some() {
        return;
    }
    // Blocking accept with a loopback wakeup (see run_worker): the flag,
    // not a poll, ends the wait. Nonblocking polling proved unreliable on
    // some stacks, so blocking plus an explicit wakeup is the design.
    let bound = TcpListener::bind("[::]:0").and_then(|tcp| {
        tcp.set_nonblocking(false)?;
        Ok(tcp)
    });
    let tcp = match bound {
        Ok(tcp) => tcp,
        Err(_) => {
            let _ = events.send(LinkEvent::ListenerFailed);
            return;
        }
    };
    let port = tcp.local_addr().map(|addr| addr.port()).unwrap_or(0);
    let (Some((cert_der, key_der))) = generate_identity() else {
        let _ = events.send(LinkEvent::ListenerFailed);
        return;
    };
    let fingerprint: [u8; 32] = Sha256::digest(&cert_der).into();
    match server_config(cert_der, key_der) {
        Some(config) => {
            *listener = Some((tcp, config));
            let _ = events.send(LinkEvent::Listening {
                port,
                fingerprint_hex: fingerprint_hex(&fingerprint),
            });
        }
        None => {
            let _ = events.send(LinkEvent::ListenerFailed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_context() -> LinkContext {
        LinkContext {
            device_id: Uuid::new_v4(),
            harbor_id: "harbor-test".into(),
            signing_seed: [7_u8; 32],
            device_type: DeviceType::Desktop,
            peers: vec![LinkPeer {
                device_id: Uuid::new_v4(),
                harbor_id: "harbor-peer".into(),
                public_key: STANDARD_NO_PAD.encode(
                    ed25519_dalek::SigningKey::from_bytes(&[9_u8; 32])
                        .verifying_key()
                        .as_bytes(),
                ),
            }],
        }
    }

    #[test]
    fn frames_round_trip_inside_the_cap() {
        let frame = LinkFrame::new(KIND_CHAT, json!({"id": "a", "body": "oi"})).unwrap();
        let peer = Uuid::new_v4();
        let _ = peer;
        let encoded = frame.encode();
        let length = u32::from_be_bytes(encoded[..4].try_into().unwrap()) as usize;
        let decoded = LinkFrame::decode(&encoded[4..4 + length]).unwrap();
        assert_eq!(decoded, frame);
        assert!(LinkFrame::new("", json!({})).is_none());
        assert!(LinkFrame::new(KIND_CHAT, json!([])).is_none());
    }

    #[test]
    fn invites_parse_tightly_or_not_at_all() {
        let good = json!({
            "addrs": ["10.0.0.2", "127.0.0.1"],
            "port": 41234,
            "fingerprint": "ab".repeat(32),
        });
        let invite = LinkInvite::parse(&good).unwrap();
        assert_eq!(invite.addrs.len(), 2);
        assert_eq!(invite.port, 41234);
        assert!(LinkInvite::parse(&json!({"addrs": [], "port": 1, "fingerprint": "ab".repeat(32)})).is_none());
        assert!(LinkInvite::parse(&json!({"addrs": ["x"], "port": 0, "fingerprint": "ab".repeat(32)})).is_none());
        assert!(LinkInvite::parse(&json!({"addrs": ["x"], "port": 1, "fingerprint": "zz"})).is_none());
        let mut many = vec![];
        for _ in 0..9 {
            many.push(json!("10.0.0.1"));
        }
        assert!(LinkInvite::parse(&json!({"addrs": many, "port": 1, "fingerprint": "ab".repeat(32)})).is_none());
    }

    #[test]
    fn hello_claims_must_match_the_authenticated_peer() {
        let context = test_context();
        let peer = context.peers[0].device_id;
        // A hello naming exactly this authenticated peer validates.
        let tailored = LinkFrame::new(
            KIND_HELLO,
            json!({
                "device_id": peer.to_string(),
                "harbor_id": "harbor-peer",
                "device_type": "mobile",
            }),
        )
        .unwrap();
        assert_eq!(
            accept_hello(&context, &peer, &tailored),
            Some(DeviceType::Mobile)
        );
        // Wrong harbor or unknown kind never validates.
        let hostile = LinkFrame::new(
            KIND_HELLO,
            json!({
                "device_id": peer.to_string(),
                "harbor_id": "harbor-evil",
                "device_type": "mobile",
            }),
        )
        .unwrap();
        assert!(accept_hello(&context, &peer, &hostile).is_none());
    }

    #[test]
    fn challenge_signatures_verify_against_the_paired_key() {
        let context = test_context();
        let peer = context.peers[0].device_id;
        let nonce = fresh_nonce(&context, &peer);
        // Signed with another seed: refused.
        assert!(!verify_signature(&context.peers[0].public_key, &nonce, &hex(&[0_u8; 64])));
        // Signed with the paired seed: accepted. (Seed 9 signs here to
        // match the test peer's public key.)
        let proper = {
            use ed25519_dalek::Signer as _;
            ed25519_dalek::SigningKey::from_bytes(&[9_u8; 32]).sign(&nonce).to_bytes()
        };
        assert!(verify_signature(&context.peers[0].public_key, &nonce, &hex(&proper)));
    }

    #[test]
    fn listener_and_dialer_complete_a_pinned_challenged_link() {
        let desktop = test_context();
        // The phone's context pairs with the desktop and vice versa.
        let desktop_signing = ed25519_dalek::SigningKey::from_bytes(&[11_u8; 32]);
        let phone_signing = ed25519_dalek::SigningKey::from_bytes(&[13_u8; 32]);
        let desktop_id = Uuid::new_v4();
        let phone_id = Uuid::new_v4();
        let desktop_context = LinkContext {
            device_id: desktop_id,
            harbor_id: "harbor-desk".into(),
            signing_seed: [11_u8; 32],
            device_type: DeviceType::Desktop,
            peers: vec![LinkPeer {
                device_id: phone_id,
                harbor_id: "harbor-phone".into(),
                public_key: STANDARD_NO_PAD.encode(phone_signing.verifying_key().as_bytes()),
            }],
        };
        let phone_context = LinkContext {
            device_id: phone_id,
            harbor_id: "harbor-phone".into(),
            signing_seed: [13_u8; 32],
            device_type: DeviceType::Mobile,
            peers: vec![LinkPeer {
                device_id: desktop_id,
                harbor_id: "harbor-desk".into(),
                public_key: STANDARD_NO_PAD.encode(desktop_signing.verifying_key().as_bytes()),
            }],
        };
        let _ = desktop;

        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (event_tx, event_rx) = mpsc::channel();
        // Listener in this thread would block the test; run the worker with
        // a Listen command and read back its advertisement.
        let shutdown = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let worker_shutdown = std::sync::Arc::clone(&shutdown);
        let worker = std::thread::spawn(move || run_worker(cmd_rx, event_tx, worker_shutdown));
        cmd_tx
            .send(LinkCommand::Listen {
                context: desktop_context.clone(),
            })
            .unwrap();
        let (port, fingerprint_hex) = loop {
            match event_rx.recv_timeout(Duration::from_secs(5)).unwrap() {
                LinkEvent::Listening { port, fingerprint_hex } => break (port, fingerprint_hex),
                other => panic!("expected Listening, got {other:?}"),
            }
        };
        let fingerprint = parse_fingerprint_hex(&fingerprint_hex).unwrap();
        // Dial on a second channel pair... instead drive dial_one directly:
        // it needs its own event/command channels.
        let (dial_cmd_tx, dial_cmd_rx) = mpsc::channel();
        let (dial_event_tx, dial_event_rx) = mpsc::channel();
        let invite = LinkInvite {
            addrs: vec!["127.0.0.1".into()],
            port,
            fingerprint,
        };
        let dial_context = phone_context.clone();
        let dial_shutdown = std::sync::Arc::clone(&shutdown);
        let dialer = std::thread::spawn(move || {
            dial_one(&invite, &dial_context, &dial_event_tx, &dial_cmd_rx, &dial_shutdown);
        });
        // Both sides must report the authenticated peer.
        let mut desktop_saw = None;
        let mut phone_saw = None;
        let deadline = Instant::now() + Duration::from_secs(30);
        while (desktop_saw.is_none() || phone_saw.is_none()) && Instant::now() < deadline {
            for event in event_rx.try_iter() {
                if let LinkEvent::Connected { peer, device, .. } = event {
                    desktop_saw = Some((peer, device));
                }
            }
            for event in dial_event_rx.try_iter() {
                if let LinkEvent::Connected { peer, device, .. } = event {
                    phone_saw = Some((peer, device));
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        shutdown.store(true, std::sync::atomic::Ordering::SeqCst);
        let _ = dial_cmd_tx.send(LinkCommand::Shutdown);
        let _ = cmd_tx.send(LinkCommand::Shutdown);
        // A loopback wakeup releases the worker's blocking accept so the
        // flag above ends it promptly; the dial side is straight-line code.
        let _ = std::net::TcpStream::connect_timeout(
            &SocketAddr::from(([127, 0, 0, 1], port)),
            Duration::from_secs(2),
        );
        dialer.join().ok();
        worker.join().ok();
        let (peer, device) = desktop_saw.expect("desktop authenticates the phone");
        assert_eq!(peer, phone_id);
        assert_eq!(device, DeviceType::Mobile);
        let (peer, device) = phone_saw.expect("phone authenticates the desktop");
        assert_eq!(peer, desktop_id);
        assert_eq!(device, DeviceType::Desktop);
    }
}
