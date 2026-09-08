//! TLS transport for the Harbor control-plane server.
//!
//! The listener speaks the same length-prefixed framing as the local IPC but
//! accepts only signed `AuthenticatedEnvelope` requests, and answers with
//! correlated response envelopes whose integrity comes from TLS. The frame
//! cap and the control-plane allowlist make a media/data-plane detour through
//! this listener impossible by construction.

use std::fs;
use std::io;
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

use harbor_protocol::{AuthenticatedEnvelope, Envelope, FrameError, FrameStream};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};

use crate::relay::{POLL_MAX_BYTES, RelayDelivery, RelayTable};
use crate::{RelayAction, ServerCore};

/// Tighter than the local 1 MiB cap: the largest legitimate payload is a
/// session signal at [`harbor_control::MAX_SIGNAL_BYTES`] (64 KiB), which may
/// roughly double under JSON escaping; 256 KiB leaves ample slack while
/// keeping bulk transfers physically impossible through the control plane.
pub const MAX_NETWORK_FRAME_BYTES: usize = 256 * 1024;

/// Relay and control traffic share the listener: the budget covers a control
/// plus a long-poll connection per device with headroom. Per-IP caps would
/// punish CGNAT-shared addresses, so connection storms are contained by the
/// total plus a shorter idle fuse for never-authenticated squatters below.
const MAX_CONNECTIONS: usize = 16;

/// Idle connections (including an unfinished TLS handshake) are cut here.
const CONNECTION_IDLE_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a parked `relay.poll` waits for traffic before answering empty.
/// Under the 30 s connection idle kill, with margin for one round trip.
const POLL_HOLD_SECS: u64 = 25;

/// Delivery granularity of the connection loop. Every iteration also serves
/// a parked poll, so relayed traffic sees at most this much extra delay;
/// idle iterations cost one timer check and (only when parked) one table
/// lock. Voice tolerates ~20 ms of extra jitter, not 100 ms.
const POLL_TICK: Duration = Duration::from_millis(20);

/// Idle fuse for connections that never authenticated (handshake squatters).
/// Authenticated connections keep the 30 s `CONNECTION_IDLE_TIMEOUT`.
const UNAUTH_IDLE_TIMEOUT: Duration = Duration::from_secs(10);
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);

const CERT_FILE: &str = "cert.pem";
const KEY_FILE: &str = "key.pem";

/// Where and how the listener binds and sources its TLS identity.
pub struct ListenerConfig {
    pub bind: SocketAddr,
    /// Directory for the generated self-signed identity (cert.pem/key.pem).
    pub tls_dir: PathBuf,
    /// Operator-provided certificate/key; when both are set no identity is
    /// generated and `tls_dir` is untouched.
    pub cert_pem: Option<PathBuf>,
    pub key_pem: Option<PathBuf>,
}

/// A running TLS listener owning its accept thread.
pub struct Listener {
    local_addr: SocketAddr,
    fingerprint: [u8; 32],
    shutdown: Arc<AtomicBool>,
}

impl Listener {
    /// Binds and starts serving. `core` is shared between connection threads;
    /// durable flushes happen inside `ServerCore::handle` per mutating
    /// request, so no coordination beyond the mutex is needed.
    pub fn spawn(
        config: ListenerConfig,
        core: Arc<Mutex<ServerCore>>,
    ) -> Result<Self, TransportError> {
        let identity = TlsIdentity::load_or_create(&config)?;
        let fingerprint = identity.fingerprint();
        let server_config = Arc::new(
            rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(vec![identity.certificate], identity.private_key)?,
        );

        let tcp = TcpListener::bind(config.bind)?;
        let local_addr = tcp.local_addr()?;
        let shutdown = Arc::new(AtomicBool::new(false));
        std::thread::Builder::new()
            .name("harbor-server-listener".into())
            .spawn({
                let shutdown = Arc::clone(&shutdown);
                move || accept_loop(tcp, server_config, core, shutdown)
            })?;

        Ok(Listener {
            local_addr,
            fingerprint,
            shutdown,
        })
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// SHA-256 over the served certificate DER. This is what clients pin:
    /// the certificate is self-signed, so the fingerprint — distributed
    /// out-of-band — is the server's identity anchor. Public material, safe
    /// to log.
    pub fn certificate_fingerprint(&self) -> [u8; 32] {
        self.fingerprint
    }

    /// Stops accepting and wakes the blocked accept call. In-flight
    /// connection threads finish or hit their idle timeout; the process
    /// exiting drops them regardless.
    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        // A wildcard bind ([::] or 0.0.0.0) is not itself dialable, so wake
        // the acceptor through loopback in the bound family instead.
        let wake = match self.local_addr {
            SocketAddr::V4(bound) if bound.ip().is_unspecified() => SocketAddr::new(
                std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST),
                bound.port(),
            ),
            SocketAddr::V6(bound) if bound.ip().is_unspecified() => SocketAddr::new(
                std::net::IpAddr::V6(std::net::Ipv6Addr::LOCALHOST),
                bound.port(),
            ),
            bound => bound,
        };
        let _ = TcpStream::connect(wake);
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("tls configuration failed: {0}")]
    Tls(#[from] rustls::Error),
    #[error("tls identity: {0}")]
    Identity(String),
}

fn accept_loop(
    tcp: TcpListener,
    server_config: Arc<rustls::ServerConfig>,
    core: Arc<Mutex<ServerCore>>,
    shutdown: Arc<AtomicBool>,
) {
    let active = Arc::new(AtomicUsize::new(0));
    // One relay table for the listener lifetime, shared by all connection
    // threads. Delivery is pull-based (recipients drain on their own loop),
    // so the table never owns sockets or threads — just sessions, bounded
    // queues, and notices.
    let relay = Arc::new(Mutex::new(RelayTable::new()));
    loop {
        let (stream, peer) = match tcp.accept() {
            Ok(accepted) => accepted,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                eprintln!("harbor-server: accept failed: {error}");
                break;
            }
        };
        if shutdown.load(Ordering::SeqCst) {
            break;
        }
        if active.load(Ordering::SeqCst) >= MAX_CONNECTIONS {
            eprintln!("harbor-server: connection limit reached, refusing {peer}");
            continue; // the unhandled stream closes on drop
        }
        active.fetch_add(1, Ordering::SeqCst);
        let connection_core = Arc::clone(&core);
        let connection_config = Arc::clone(&server_config);
        let slot_active = Arc::clone(&active);
        let connection_relay = Arc::clone(&relay);
        let spawned = std::thread::Builder::new()
            .name("harbor-server-connection".into())
            .spawn(move || {
                let _slot = ConnectionSlot(slot_active);
                serve_connection(
                    connection_config,
                    stream,
                    peer,
                    connection_core,
                    connection_relay,
                );
            });
        if spawned.is_err() {
            active.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

/// Returns the reservation to the accept loop's budget when the thread ends.
struct ConnectionSlot(Arc<AtomicUsize>);

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Serves one connection: control requests answer immediately, verified
/// `relay.poll` requests park for up to `POLL_HOLD_SECS` while the loop keeps
/// reading. Delivery is pull-based — each iteration drains whatever the
/// relay table holds for this connection's device — so no cross-thread
/// channels or socket sharing exist; at most one parked poll is kept (a
/// second poll answers the first empty). Partial frames survive across the
/// short-tick reads inside `FrameStream`'s pending buffer.
fn serve_connection(
    server_config: Arc<rustls::ServerConfig>,
    stream: TcpStream,
    peer: SocketAddr,
    core: Arc<Mutex<ServerCore>>,
    relay: Arc<Mutex<RelayTable>>,
) {
    let _ = stream.set_read_timeout(Some(POLL_TICK));
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let _ = stream.set_nodelay(true);

    let Ok(tls_connection) = rustls::ServerConnection::new(server_config) else {
        return;
    };
    let mut tls = rustls::StreamOwned::new(tls_connection, stream);
    let mut frames = FrameStream::with_limit(&mut tls, MAX_NETWORK_FRAME_BYTES);

    let mut parked: Option<(Envelope, Instant)> = None;
    // Last verified signer on this connection. Set no later than parking, so
    // delivery and idle decisions never guess; squatters that never verify
    // keep the short fuse even while chatting.
    let mut peer_device: Option<Uuid> = None;
    let mut last_frame = Instant::now();
    loop {
        // 1. Parked poll: deliver anything due, or time it out.
        if let Some((request, deadline)) = parked.take() {
            match poll_answer(&relay, peer_device, &request) {
                Some(answer) => {
                    if frames.write_frame(&answer.bytes).is_err() {
                        let mut table = relay.lock().unwrap_or_else(|poison| poison.into_inner());
                        table.restore_for(answer.device, answer.delivery);
                        break;
                    }
                }
                None if Instant::now() >= deadline => {
                    match poll_reply(&request, &RelayDelivery::default(), true) {
                        Some(reply) => {
                            if frames.write_frame(&reply).is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
                None => parked = Some((request, deadline)),
            }
        }
        match frames.read_frame() {
            Ok(Some(bytes)) => {
                last_frame = Instant::now();
                match handle_frame(&core, &relay, &bytes, peer, &mut peer_device) {
                    FrameOutcome::Reply(reply) => {
                        if frames.write_frame(&reply).is_err() {
                            break;
                        }
                    }
                    FrameOutcome::Park(request) => {
                        if let Some((old, _)) = parked.take() {
                            match poll_reply(&old, &RelayDelivery::default(), false) {
                                Some(reply) => {
                                    if frames.write_frame(&reply).is_err() {
                                        break;
                                    }
                                }
                                None => break,
                            }
                        }
                        parked = Some((
                            *request,
                            Instant::now() + Duration::from_secs(POLL_HOLD_SECS),
                        ));
                    }
                    FrameOutcome::Close => break,
                }
            }
            Ok(None) => break,
            Err(FrameError::Io(error))
                if error.kind() == io::ErrorKind::WouldBlock
                    || error.kind() == io::ErrorKind::TimedOut =>
            {
                let limit = if peer_device.is_some() {
                    CONNECTION_IDLE_TIMEOUT
                } else {
                    UNAUTH_IDLE_TIMEOUT
                };
                if last_frame.elapsed() > limit {
                    break;
                }
            }
            Err(FrameError::OversizedFrame) => {
                eprintln!("harbor-server: closed a connection that announced an oversized frame");
                break;
            }
            Err(_) => break,
        }
    }

    // A TLS close_notify distinguishes "we chose to stop" from a truncated
    // stream: the peer's next read reports clean EOF instead of an error.
    tls.conn.send_close_notify();
    let _ = std::io::Write::flush(&mut tls);
}

/// What one inbound frame asks the connection loop to do. The parked poll
/// boxes its request envelope: polls are rare (one per connection) and the
/// envelope dwarfs the other variants.
enum FrameOutcome {
    Reply(Vec<u8>),
    Park(Box<Envelope>),
    Close,
}

fn handle_frame(
    core: &Mutex<ServerCore>,
    relay: &Arc<Mutex<RelayTable>>,
    bytes: &[u8],
    peer: SocketAddr,
    peer_device: &mut Option<Uuid>,
) -> FrameOutcome {
    let authenticated: AuthenticatedEnvelope = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(_) => {
            eprintln!("harbor-server: closed a connection that sent an unparseable frame");
            return FrameOutcome::Close;
        }
    };
    if authenticated.envelope.message_type.starts_with("relay.") {
        return serve_relay_frame(core, relay, authenticated, peer_device);
    }
    match respond(core, authenticated, peer, peer_device) {
        Some(reply) => FrameOutcome::Reply(reply),
        None => FrameOutcome::Close,
    }
}

/// Routes one `relay.*` frame: control operations answer immediately,
/// verified polls park for the loop to answer, anything unshaped closes.
/// Verification happens before parking, so strangers cannot park polls and
/// eat connection budget.
fn serve_relay_frame(
    core: &Mutex<ServerCore>,
    relay: &Arc<Mutex<RelayTable>>,
    authenticated: AuthenticatedEnvelope,
    peer_device: &mut Option<Uuid>,
) -> FrameOutcome {
    let signer = authenticated.signer_id;
    let now = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(elapsed) => elapsed.as_secs(),
        Err(_) => return FrameOutcome::Close,
    };
    let outcome = match core.lock() {
        Ok(server) => server.relay_request(relay, authenticated, now),
        Err(_) => return FrameOutcome::Close,
    };
    match outcome {
        Ok(RelayAction::Reply(response)) => {
            if response.validate_network().is_err() {
                eprintln!("harbor-server: generated an invalid relay response envelope");
                return FrameOutcome::Close;
            }
            if response.error.is_none() {
                *peer_device = Some(signer);
            }
            match serde_json::to_vec(&response) {
                Ok(reply) => FrameOutcome::Reply(reply),
                Err(_) => FrameOutcome::Close,
            }
        }
        Ok(RelayAction::Park(request)) => {
            *peer_device = Some(signer);
            FrameOutcome::Park(Box::new(request))
        }
        Err(_) => FrameOutcome::Close,
    }
}

/// Answers a parked poll when anything is due for `device`: queued frames,
/// closed sessions, or opens awaiting acceptance. `None` keeps waiting.
/// Poisoned table locks recover (plain data) rather than hanging polls.
struct PollAnswer {
    bytes: Vec<u8>,
    device: Uuid,
    delivery: RelayDelivery,
}

fn poll_answer(
    relay: &Arc<Mutex<RelayTable>>,
    device: Option<Uuid>,
    request: &Envelope,
) -> Option<PollAnswer> {
    let device = device?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    let mut table = relay.lock().unwrap_or_else(|poison| poison.into_inner());
    table.expire(now);
    let delivery = table.drain_for(device, now, POLL_MAX_BYTES);
    if delivery.frames.is_empty() && delivery.closed.is_empty() && delivery.opens.is_empty() {
        return None;
    }
    match poll_reply(request, &delivery, false) {
        Some(bytes) => Some(PollAnswer {
            bytes,
            device,
            delivery,
        }),
        None => {
            table.restore_for(device, delivery);
            None
        }
    }
}

/// Builds one poll answer envelope: drained delivery plus a timeout flag.
/// `None` only when the request cannot be correlated at all (protocol
/// violation: parked polls always carry an id, checked before parking).
fn poll_reply(request: &Envelope, delivery: &RelayDelivery, timeout: bool) -> Option<Vec<u8>> {
    let timestamp = request.timestamp.clone()?;
    let response = Envelope::response_to(
        request,
        "relay.poll",
        serde_json::json!({
            "frames": delivery.frames.iter().map(|frame| serde_json::json!({
                "relay_id": frame.relay_id,
                "from": frame.from,
                "seq": frame.seq,
                "bytes": frame.bytes,
            })).collect::<Vec<_>>(),
            "closed": delivery.closed,
            "opens": delivery.opens,
            "timeout": timeout,
        }),
        timestamp,
    )
    .ok()?;
    if response.validate_network().is_err() {
        return None;
    }
    serde_json::to_vec(&response).ok()
}

/// One request, one correlated response. `None` means no frame goes back and
/// the connection is closed — either handling failed, or the reply failed
/// validation. `peer` is the source endpoint observed at accept time; it
/// travels with the request so rendezvous facts (never trust decisions) can
/// record it. An error-free reply marks the connection authenticated.
fn respond(
    core: &Mutex<ServerCore>,
    authenticated: AuthenticatedEnvelope,
    peer: SocketAddr,
    peer_device: &mut Option<Uuid>,
) -> Option<Vec<u8>> {
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    let signer = authenticated.signer_id;
    let outcome = match core.lock() {
        Ok(mut server) => server.handle_observed(authenticated, now, Some(peer)),
        Err(_) => return None,
    };
    match outcome {
        Ok(response) => {
            // Validate here and return bare JSON: the framing prefix is
            // `write_frame`'s job, and `encode_frame` would double-wrap it.
            if response.validate_network().is_err() {
                eprintln!("harbor-server: generated an invalid response envelope");
                return None;
            }
            if response.error.is_none() {
                *peer_device = Some(signer);
            }
            serde_json::to_vec(&response).ok()
        }
        Err(error) => {
            eprintln!("harbor-server: request handling failed: {error}");
            None
        }
    }
}

/// The server's TLS material: certificate chain plus private key.
struct TlsIdentity {
    certificate: CertificateDer<'static>,
    private_key: PrivateKeyDer<'static>,
}

impl TlsIdentity {
    /// Operator files win; otherwise a persistent self-signed identity is
    /// kept in `tls_dir`. A half-written pair (crash between the two writes)
    /// is regenerated instead of trusted.
    fn load_or_create(config: &ListenerConfig) -> Result<Self, TransportError> {
        if let (Some(cert), Some(key)) = (&config.cert_pem, &config.key_pem) {
            return Self::from_pem_files(cert, key);
        }

        let cert_path = config.tls_dir.join(CERT_FILE);
        let key_path = config.tls_dir.join(KEY_FILE);
        if cert_path.exists() && key_path.exists() {
            if let Ok(identity) = Self::from_pem_files(&cert_path, &key_path) {
                return Ok(identity);
            }
        }
        Self::generate(&cert_path, &key_path)
    }

    fn from_pem_files(cert_path: &Path, key_path: &Path) -> Result<Self, TransportError> {
        let certificates: Vec<_> = rustls_pemfile::certs(&mut io::BufReader::new(
            fs::File::open(cert_path).map_err(|error| {
                TransportError::Identity(format!("cannot read {}: {error}", cert_path.display()))
            })?,
        ))
        .collect::<Result<_, _>>()?;
        let private_key = rustls_pemfile::private_key(&mut io::BufReader::new(
            fs::File::open(key_path).map_err(|error| {
                TransportError::Identity(format!("cannot read {}: {error}", key_path.display()))
            })?,
        ))?
        .ok_or_else(|| {
            TransportError::Identity(format!("no private key found in {}", key_path.display()))
        })?;

        let certificate = certificates.into_iter().next().ok_or_else(|| {
            TransportError::Identity(format!("no certificate found in {}", cert_path.display()))
        })?;
        Ok(Self {
            certificate,
            private_key,
        })
    }

    fn generate(cert_path: &Path, key_path: &Path) -> Result<Self, TransportError> {
        let key_pair = rcgen::KeyPair::generate()
            .map_err(|error| TransportError::Identity(error.to_string()))?;
        let mut params = rcgen::CertificateParams::new(vec!["harbor-server".to_owned()])
            .map_err(|error| TransportError::Identity(error.to_string()))?;
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "harbor-server");
        let certificate = params
            .self_signed(&key_pair)
            .map_err(|error| TransportError::Identity(error.to_string()))?;

        if let Some(parent) = cert_path.parent() {
            fs::create_dir_all(parent)?;
        }
        // PEM, so the restart path can re-read it with rustls_pemfile.
        write_private_atomic(key_path, key_pair.serialize_pem().as_bytes())?;
        // The cert is public; it is written last so a crash never leaves a
        // cert that does not match the persisted key.
        fs::write(cert_path, certificate.pem())?;

        Ok(Self {
            certificate: certificate.der().to_owned(),
            private_key: PrivateKeyDer::Pkcs8(key_pair.serialize_der().into()),
        })
    }

    fn fingerprint(&self) -> [u8; 32] {
        Sha256::digest(self.certificate.as_ref()).into()
    }
}

/// Temp-file + rename write with owner-only permissions.
fn write_private_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension("tmp");
    {
        use std::io::Write as _;
        let mut file = fs::File::create(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temporary, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&temporary, path)
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;

    use ed25519_dalek::SigningKey;
    use harbor_protocol::Envelope;
    use serde_json::json;
    use uuid::Uuid;

    use super::*;

    fn base64_of(key: &SigningKey) -> String {
        use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
        STANDARD_NO_PAD.encode(key.verifying_key().as_bytes())
    }

    fn rfc3339_now() -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        time::OffsetDateTime::from_unix_timestamp(now)
            .unwrap()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap()
    }

    fn open_core() -> (Arc<Mutex<ServerCore>>, PathBuf) {
        let directory =
            std::env::temp_dir().join(format!("harbor-server-transport-{}", Uuid::new_v4()));
        (
            Arc::new(Mutex::new(ServerCore::open(&directory).unwrap())),
            directory,
        )
    }

    fn spawned_listener(
        bind: SocketAddr,
    ) -> (
        Listener,
        Arc<Mutex<ServerCore>>,
        PathBuf,
        ed25519_dalek::SigningKey,
    ) {
        let (core, directory) = open_core();
        let key = SigningKey::from_bytes(&[21; 32]);
        let listener = Listener::spawn(
            ListenerConfig {
                bind,
                tls_dir: directory.join("tls"),
                cert_pem: None,
                key_pem: None,
            },
            Arc::clone(&core),
        )
        .unwrap();
        (listener, core, directory, key)
    }

    /// A test client that pins the server by certificate fingerprint — the
    /// same trust model a production Harbor core will use against the K11+.
    #[derive(Debug)]
    struct PinnedFingerprint([u8; 32]);

    impl rustls::client::danger::ServerCertVerifier for PinnedFingerprint {
        fn verify_server_cert(
            &self,
            end_entity: &CertificateDer<'_>,
            _intermediates: &[CertificateDer<'_>],
            _server_name: &rustls::pki_types::ServerName<'_>,
            _ocsp_response: &[u8],
            _now: rustls::pki_types::UnixTime,
        ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            let served: [u8; 32] = Sha256::digest(end_entity.as_ref()).into();
            if served == self.0 {
                Ok(rustls::client::danger::ServerCertVerified::assertion())
            } else {
                Err(rustls::Error::General(
                    "server certificate fingerprint does not match the pinned value".into(),
                ))
            }
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

    struct TestClient {
        frames: FrameStream<rustls::StreamOwned<rustls::ClientConnection, TcpStream>>,
    }

    impl TestClient {
        fn connect(listener: &Listener) -> Self {
            Self::connect_to(listener.local_addr(), listener.certificate_fingerprint())
        }

        fn connect_to(address: SocketAddr, fingerprint: [u8; 32]) -> Self {
            let client_config = rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(PinnedFingerprint(fingerprint)))
                .with_no_client_auth();
            let server_name =
                rustls::pki_types::ServerName::try_from("harbor-server".to_owned()).unwrap();
            let connection =
                rustls::ClientConnection::new(Arc::new(client_config), server_name).unwrap();
            let socket = TcpStream::connect(address).unwrap();
            Self {
                frames: FrameStream::with_limit(
                    rustls::StreamOwned::new(connection, socket),
                    MAX_NETWORK_FRAME_BYTES,
                ),
            }
        }

        fn request(&mut self, authenticated: &AuthenticatedEnvelope) -> Envelope {
            let bytes = serde_json::to_vec(authenticated).unwrap();
            self.frames.write_frame(&bytes).unwrap();
            let reply = self.frames.read_frame().unwrap().expect("a response frame");
            serde_json::from_slice(&reply).unwrap()
        }
    }

    fn signed_identity_update(key: &SigningKey, device_id: Uuid) -> AuthenticatedEnvelope {
        AuthenticatedEnvelope::sign(
            device_id,
            Envelope::request(
                "identity.update",
                json!({
                    "device_id": device_id,
                    "harbor_id": format!("harbor-{:08x}", device_id.as_u128() as u32),
                    "public_key": base64_of(key),
                }),
                rfc3339_now(),
            ),
            key,
        )
        .unwrap()
    }

    fn signed_request(
        key: &SigningKey,
        signer: Uuid,
        message_type: &str,
        payload: serde_json::Value,
    ) -> AuthenticatedEnvelope {
        AuthenticatedEnvelope::sign(
            signer,
            Envelope::request(message_type, payload, rfc3339_now()),
            key,
        )
        .unwrap()
    }

    #[test]
    fn signed_requests_round_trip_over_tls_and_reject_wrong_pins() {
        let (listener, _core, directory, key) = spawned_listener("127.0.0.1:0".parse().unwrap());
        let device_id = Uuid::new_v4();

        let mut client = TestClient::connect(&listener);
        let registered = client.request(&signed_identity_update(&key, device_id));
        assert!(registered.error.is_none());

        let pairing = client.request(&signed_request(
            &key,
            device_id,
            "pairing.create",
            json!({"code": "135790"}),
        ));
        assert!(pairing.error.is_none());
        drop(client);

        // A client pinning a different fingerprint must never complete a
        // handshake against this server.
        let impostor_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedFingerprint([7; 32])))
            .with_no_client_auth();
        let impostor_name =
            rustls::pki_types::ServerName::try_from("harbor-server".to_owned()).unwrap();
        let impostor = rustls::ClientConnection::new(Arc::new(impostor_config), impostor_name);
        let socket = TcpStream::connect(listener.local_addr()).unwrap();
        let mut impostor_tls = rustls::StreamOwned::new(impostor.unwrap(), socket);
        let mut buffer = [0_u8; 64];
        assert!(impostor_tls.read(&mut buffer).is_err());

        listener.shutdown();
        std::fs::remove_dir_all(directory).unwrap();
    }

    /// A wildcard IPv6 bind serves native IPv6 loopback (and, on dual-stack
    /// Linux, IPv4-mapped loopback too). The wildcard itself is never
    /// dialed; clients use a concrete loopback in the same family.
    #[test]
    fn wildcard6_listeners_serve_ipv6_loopback() {
        let (core, directory) = open_core();
        let key = SigningKey::from_bytes(&[21; 32]);
        let listener = match Listener::spawn(
            ListenerConfig {
                bind: "[::]:0".parse().unwrap(),
                tls_dir: directory.join("tls"),
                cert_pem: None,
                key_pem: None,
            },
            Arc::clone(&core),
        ) {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("skipping IPv6 test: cannot bind [::] ({error})");
                return;
            }
        };
        let port = listener.local_addr().port();
        let v6: SocketAddr = format!("[::1]:{port}").parse().unwrap();
        let mut client = TestClient::connect_to(v6, listener.certificate_fingerprint());
        let device_id = Uuid::new_v4();
        let registered = client.request(&signed_identity_update(&key, device_id));
        assert!(registered.error.is_none(), "{registered:?}");

        // Shutdown on a wildcard bind must wake the acceptor instead of
        // hanging the suite; it returns once the wake is attempted.
        listener.shutdown();
        std::fs::remove_dir_all(directory).unwrap();
    }

    /// Rendezvous foundation: the server reports the source endpoint it
    /// observed for this very connection — loopback here, a global address
    /// in production — without trusting any client claim about it.
    #[test]
    fn endpoint_self_reports_the_observed_loopback_source() {
        let (listener, _core, directory, key) = spawned_listener("127.0.0.1:0".parse().unwrap());
        let device_id = Uuid::new_v4();

        let mut client = TestClient::connect(&listener);
        client.request(&signed_identity_update(&key, device_id));
        let probed = client.request(&signed_request(&key, device_id, "endpoint.self", json!({})));
        assert!(probed.error.is_none(), "{probed:?}");
        assert_eq!(probed.payload["address"], json!("127.0.0.1"));
        assert_eq!(probed.payload["transport"], json!("tcp"));
        assert!(
            probed.payload["port"]
                .as_u64()
                .is_some_and(|port| port != 0),
            "{probed:?}"
        );

        listener.shutdown();
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn paired_devices(
        client_a: &mut TestClient,
        key_a: &SigningKey,
        id_a: Uuid,
        client_b: &mut TestClient,
        key_b: &SigningKey,
        id_b: Uuid,
    ) {
        client_a.request(&signed_identity_update(key_a, id_a));
        client_b.request(&signed_identity_update(key_b, id_b));
        client_a.request(&signed_request(
            key_a,
            id_a,
            "pairing.create",
            json!({"code": "135790"}),
        ));
        client_b.request(&signed_request(
            key_b,
            id_b,
            "pairing.submit",
            json!({"code": "135790"}),
        ));
        let incoming =
            client_a.request(&signed_request(key_a, id_a, "pairing.incoming", json!({})));
        let pairing_id = incoming.payload["requests"][0]["pairing_id"]
            .as_str()
            .unwrap()
            .to_owned();
        let accepted = client_a.request(&signed_request(
            key_a,
            id_a,
            "pairing.accept",
            json!({"pairing_id": pairing_id}),
        ));
        assert!(accepted.error.is_none(), "{accepted:?}");
    }

    /// Full relay lifecycle between two paired devices over real TLS:
    /// open → parked poll delivers the notice → accept → data → poll
    /// delivers frames → close → poll delivers the notice.
    #[test]
    fn relay_open_accept_data_poll_close_flows() {
        let (listener, _core, directory, _key) = spawned_listener("127.0.0.1:0".parse().unwrap());
        let key_a = SigningKey::from_bytes(&[41; 32]);
        let key_b = SigningKey::from_bytes(&[42; 32]);
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let mut ctrl_a = TestClient::connect(&listener);
        let mut ctrl_b = TestClient::connect(&listener);
        paired_devices(&mut ctrl_a, &key_a, id_a, &mut ctrl_b, &key_b, id_b);

        // B parks a poll on its own connection. Registration is per device,
        // not per connection, so no second identity dance is needed.
        let mut poll_b = TestClient::connect(&listener);
        let poll_request = signed_request(&key_b, id_b, "relay.poll", json!({}));
        let parked = std::thread::spawn(move || poll_b.request(&poll_request));

        // A opens; the parked poll must answer with the notice promptly
        // (20 ms loop tick — far under the 25 s hold).
        let opened = ctrl_a.request(&signed_request(
            &key_a,
            id_a,
            "relay.open",
            json!({"peer": id_b, "purpose": "chat"}),
        ));
        assert!(opened.error.is_none(), "{opened:?}");
        let relay_id = opened.payload["relay_id"].as_str().unwrap().to_owned();
        let answer = parked.join().expect("poll thread answers");
        assert!(answer.error.is_none(), "{answer:?}");
        let opens = answer.payload["opens"].as_array().unwrap();
        assert_eq!(opens.len(), 1);
        assert_eq!(opens[0]["relay_id"], json!(relay_id));
        assert_eq!(opens[0]["from"], json!(id_a.to_string()));
        assert_eq!(opens[0]["purpose"], json!("chat"));

        // Accept, push two frames, poll again for immediate delivery.
        let accepted = ctrl_b.request(&signed_request(
            &key_b,
            id_b,
            "relay.accept",
            json!({"relay_id": relay_id}),
        ));
        assert!(accepted.error.is_none(), "{accepted:?}");
        for (seq, body) in [(0_u64, "frame-zero"), (1, "frame-one")] {
            let pushed = ctrl_a.request(&signed_request(
                &key_a,
                id_a,
                "relay.data",
                json!({"relay_id": relay_id, "seq": seq, "bytes": body}),
            ));
            assert!(pushed.error.is_none(), "{pushed:?}");
        }
        let mut poll_b2 = TestClient::connect(&listener);
        let delivered = poll_b2.request(&signed_request(&key_b, id_b, "relay.poll", json!({})));
        let frames = delivered.payload["frames"].as_array().unwrap();
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0]["seq"], json!(0));
        assert_eq!(frames[0]["bytes"], json!("frame-zero"));
        assert_eq!(frames[0]["from"], json!(id_a.to_string()));
        assert_eq!(frames[1]["seq"], json!(1));

        // Close; the next poll learns it with no frames attached.
        let closed = ctrl_a.request(&signed_request(
            &key_a,
            id_a,
            "relay.close",
            json!({"relay_id": relay_id}),
        ));
        assert!(closed.error.is_none(), "{closed:?}");
        let mut poll_b3 = TestClient::connect(&listener);
        let noticed = poll_b3.request(&signed_request(&key_b, id_b, "relay.poll", json!({})));
        assert!(noticed.error.is_none(), "{noticed:?}");
        assert!(noticed.payload["frames"].as_array().unwrap().is_empty());
        assert!(
            noticed.payload["closed"]
                .as_array()
                .unwrap()
                .iter()
                .any(|id| id == &json!(relay_id))
        );

        listener.shutdown();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn relay_refuses_strangers_replays_and_oversize() {
        let (listener, _core, directory, _key) = spawned_listener("127.0.0.1:0".parse().unwrap());
        let key_a = SigningKey::from_bytes(&[43; 32]);
        let key_b = SigningKey::from_bytes(&[44; 32]);
        let key_c = SigningKey::from_bytes(&[45; 32]);
        let id_a = Uuid::new_v4();
        let id_b = Uuid::new_v4();
        let id_c = Uuid::new_v4();
        let mut ctrl_a = TestClient::connect(&listener);
        let mut ctrl_b = TestClient::connect(&listener);
        let mut ctrl_c = TestClient::connect(&listener);
        paired_devices(&mut ctrl_a, &key_a, id_a, &mut ctrl_b, &key_b, id_b);
        ctrl_c.request(&signed_identity_update(&key_c, id_c));

        // Registered but not paired with A: open refuses.
        let refused = ctrl_c.request(&signed_request(
            &key_c,
            id_c,
            "relay.open",
            json!({"peer": id_a, "purpose": "chat"}),
        ));
        assert_eq!(refused.error.unwrap().code, "unauthorized");

        // Unknown session, and the client-originated error type.
        let unknown = ctrl_a.request(&signed_request(
            &key_a,
            id_a,
            "relay.data",
            json!({"relay_id": Uuid::new_v4(), "seq": 0, "bytes": "x"}),
        ));
        assert_eq!(unknown.error.unwrap().code, "relay_unknown");
        let error_type = ctrl_a.request(&signed_request(
            &key_a,
            id_a,
            "relay.error",
            json!({"relay_id": Uuid::new_v4()}),
        ));
        assert_eq!(error_type.error.unwrap().code, "invalid_request");

        let opened = ctrl_a.request(&signed_request(
            &key_a,
            id_a,
            "relay.open",
            json!({"peer": id_b, "purpose": "chat"}),
        ));
        let relay_id = opened.payload["relay_id"].as_str().unwrap().to_owned();

        // Pushing before accept, and accepting as the opener, both refuse.
        // A not-yet-open session reports closed: the client opens fresh.
        let early = ctrl_a.request(&signed_request(
            &key_a,
            id_a,
            "relay.data",
            json!({"relay_id": relay_id, "seq": 0, "bytes": "early"}),
        ));
        assert_eq!(early.error.unwrap().code, "relay_closed");
        let self_accept = ctrl_a.request(&signed_request(
            &key_a,
            id_a,
            "relay.accept",
            json!({"relay_id": relay_id}),
        ));
        assert_eq!(self_accept.error.unwrap().code, "unauthorized");

        ctrl_b.request(&signed_request(
            &key_b,
            id_b,
            "relay.accept",
            json!({"relay_id": relay_id}),
        ));

        // Gap, replay, and oversize all refuse without forwarding. The gap
        // carries its own code so clients resync (drop + reopen) instead of
        // retrying a dead counter.
        let gap = ctrl_a.request(&signed_request(
            &key_a,
            id_a,
            "relay.data",
            json!({"relay_id": relay_id, "seq": 1, "bytes": "jump"}),
        ));
        assert_eq!(gap.error.unwrap().code, "relay_order");
        ctrl_a.request(&signed_request(
            &key_a,
            id_a,
            "relay.data",
            json!({"relay_id": relay_id, "seq": 0, "bytes": "zero"}),
        ));
        let replay = ctrl_a.request(&signed_request(
            &key_a,
            id_a,
            "relay.data",
            json!({"relay_id": relay_id, "seq": 0, "bytes": "zero-again"}),
        ));
        assert_eq!(replay.error.unwrap().code, "relay_order");
        let big = ctrl_a.request(&signed_request(
            &key_a,
            id_a,
            "relay.data",
            json!({"relay_id": relay_id, "seq": 1, "bytes": "x".repeat(48 * 1024 + 1)}),
        ));
        assert_eq!(big.error.unwrap().code, "invalid_request");

        // A full queue asks for a later retry instead of growing memory:
        // seq 0 already sits queued, so seven more fill all eight slots.
        for seq in 1..8_u64 {
            let pushed = ctrl_a.request(&signed_request(
                &key_a,
                id_a,
                "relay.data",
                json!({"relay_id": relay_id, "seq": seq, "bytes": "f"}),
            ));
            assert!(pushed.error.is_none(), "{pushed:?}");
        }
        let full = ctrl_a.request(&signed_request(
            &key_a,
            id_a,
            "relay.data",
            json!({"relay_id": relay_id, "seq": 8, "bytes": "overflow"}),
        ));
        let busy = full.error.unwrap();
        assert_eq!(busy.code, "relay_busy");
        assert!(busy.retryable);

        // An unauthenticated poll is refused immediately — strangers never
        // park polls and eat connection budget.
        let mut stranger = TestClient::connect(&listener);
        let denied = stranger.request(&signed_request(
            &SigningKey::from_bytes(&[46; 32]),
            Uuid::new_v4(),
            "relay.poll",
            json!({}),
        ));
        assert_eq!(denied.error.unwrap().code, "unauthorized");

        listener.shutdown();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn stale_requests_receive_stale_timestamp_errors() {
        let (listener, _core, directory, key) = spawned_listener("127.0.0.1:0".parse().unwrap());
        let device_id = Uuid::new_v4();

        let past = AuthenticatedEnvelope::sign(
            device_id,
            Envelope::request(
                "identity.update",
                json!({
                    "device_id": device_id,
                    "harbor_id": "harbor-aabbccdd",
                    "public_key": base64_of(&key),
                }),
                "2020-01-01T00:00:00Z",
            ),
            &key,
        )
        .unwrap();
        let mut client = TestClient::connect(&listener);
        let response = client.request(&past);
        assert_eq!(response.error.unwrap().code, "stale_timestamp");

        listener.shutdown();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn oversized_signals_and_frames_are_refused_without_a_response() {
        let (listener, _core, directory, key) = spawned_listener("127.0.0.1:0".parse().unwrap());
        let device_id = Uuid::new_v4();

        let mut client = TestClient::connect(&listener);
        client.request(&signed_identity_update(&key, device_id));

        // A signal over the control-plane limit is refused by dispatch. The
        // frame itself is legal on the wire (64 KiB + overhead fits the
        // 256 KiB network cap), so the refusal must come from control, not
        // from transport.
        let oversized_signal = signed_request(
            &key,
            device_id,
            "session.signal",
            json!({
                "session_id": Uuid::new_v4(),
                "signal": "x".repeat(harbor_control::MAX_SIGNAL_BYTES + 1),
            }),
        );
        let response = client.request(&oversized_signal);
        assert_eq!(response.error.unwrap().code, "invalid_request");

        // A frame announcing more than the network cap is cut before any
        // parse: the server closes the connection without a response.
        let impostor_config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(PinnedFingerprint(
                listener.certificate_fingerprint(),
            )))
            .with_no_client_auth();
        let name = rustls::pki_types::ServerName::try_from("harbor-server".to_owned()).unwrap();
        let connection = rustls::ClientConnection::new(Arc::new(impostor_config), name).unwrap();
        let socket = TcpStream::connect(listener.local_addr()).unwrap();
        let mut tls = rustls::StreamOwned::new(connection, socket);
        use std::io::Write as _;
        tls.write_all(&((MAX_NETWORK_FRAME_BYTES + 1) as u32).to_be_bytes())
            .unwrap();
        // The reply is a clean close with no response bytes: the announced
        // frame was never parsed.
        let mut buffer = [0_u8; 16];
        assert!(matches!(tls.read(&mut buffer), Ok(0)));

        // The listener keeps serving legitimate clients after both refusals.
        let mut healthy = TestClient::connect(&listener);
        assert!(
            healthy
                .request(&signed_request(
                    &key,
                    device_id,
                    "pairing.create",
                    json!({"code": "246801"}),
                ))
                .error
                .is_none()
        );

        listener.shutdown();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn garbage_bytes_never_take_the_listener_down() {
        let (listener, core, directory, key) = spawned_listener("127.0.0.1:0".parse().unwrap());
        let device_id = Uuid::new_v4();

        // Raw junk without TLS: the server's read of handshake bytes fails
        // and the connection is dropped; the listener must keep serving.
        let mut junk = TcpStream::connect(listener.local_addr()).unwrap();
        std::io::Write::write_all(&mut junk, b"not-a-tls-handshake-at-all").unwrap();

        // Structured plaintext over TLS that is not an authenticated request:
        // the connection is closed, the listener stays healthy.
        let mut client = TestClient::connect(&listener);
        let bytes = serde_json::to_vec(&json!({"signer_id": device_id})).unwrap();
        client.frames.write_frame(&bytes).unwrap();
        assert!(client.frames.read_frame().unwrap().is_none());

        // The listener still serves a valid client afterwards.
        let mut healthy = TestClient::connect(&listener);
        let registered = healthy.request(&signed_identity_update(&key, device_id));
        assert!(registered.error.is_none());
        assert!(core.lock().unwrap().identity(device_id).is_some());

        listener.shutdown();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn the_generated_identity_is_stable_across_restarts() {
        let (core, directory) = open_core();
        let tls_dir = directory.join("tls");
        let config = |bind: SocketAddr| ListenerConfig {
            bind,
            tls_dir: tls_dir.clone(),
            cert_pem: None,
            key_pem: None,
        };

        let first =
            Listener::spawn(config("127.0.0.1:0".parse().unwrap()), Arc::clone(&core)).unwrap();
        let fingerprint = first.certificate_fingerprint();
        first.shutdown();

        let second = Listener::spawn(config("127.0.0.1:0".parse().unwrap()), core).unwrap();
        assert_eq!(second.certificate_fingerprint(), fingerprint);
        second.shutdown();

        // The key never leaves the directory in world-readable form.
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(tls_dir.join(KEY_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o077, 0);

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn operator_provided_pem_files_are_served_directly() {
        let (core, directory) = open_core();
        let provided_dir = directory.join("provided");
        fs::create_dir_all(&provided_dir).unwrap();

        let key_pair = rcgen::KeyPair::generate().unwrap();
        let mut params = rcgen::CertificateParams::new(vec!["harbor-server".to_owned()]).unwrap();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "operator");
        let certificate = params.self_signed(&key_pair).unwrap();
        let cert_path = provided_dir.join("operator-cert.pem");
        let key_path = provided_dir.join("operator-key.pem");
        fs::write(&cert_path, certificate.pem()).unwrap();
        fs::write(&key_path, key_pair.serialize_pem()).unwrap();

        let listener = Listener::spawn(
            ListenerConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                tls_dir: directory.join("tls"),
                cert_pem: Some(cert_path),
                key_pem: Some(key_path),
            },
            core,
        )
        .unwrap();
        let expected: [u8; 32] = Sha256::digest(certificate.der().as_ref()).into();
        assert_eq!(listener.certificate_fingerprint(), expected);
        assert!(!directory.join("tls").join(CERT_FILE).exists());

        listener.shutdown();
        std::fs::remove_dir_all(directory).unwrap();
    }
}
