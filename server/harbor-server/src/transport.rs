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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use harbor_protocol::{AuthenticatedEnvelope, FrameError, FrameStream};
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use sha2::{Digest, Sha256};

use crate::ServerCore;

/// Tighter than the local 1 MiB cap: the largest legitimate payload is a
/// session signal at [`harbor_control::MAX_SIGNAL_BYTES`] (64 KiB), which may
/// roughly double under JSON escaping; 256 KiB leaves ample slack while
/// keeping bulk transfers physically impossible through the control plane.
pub const MAX_NETWORK_FRAME_BYTES: usize = 256 * 1024;

/// A handful of paired devices is the entire expected population; a small
/// bound keeps a connection storm from exhausting the phone hosting the
/// server.
const MAX_CONNECTIONS: usize = 8;

/// Idle connections (including an unfinished TLS handshake) are cut here.
const CONNECTION_IDLE_TIMEOUT: Duration = Duration::from_secs(30);
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
        let spawned = std::thread::Builder::new()
            .name("harbor-server-connection".into())
            .spawn(move || {
                let _slot = ConnectionSlot(slot_active);
                serve_connection(connection_config, stream, connection_core);
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

fn serve_connection(
    server_config: Arc<rustls::ServerConfig>,
    stream: TcpStream,
    core: Arc<Mutex<ServerCore>>,
) {
    let _ = stream.set_read_timeout(Some(CONNECTION_IDLE_TIMEOUT));
    let _ = stream.set_write_timeout(Some(WRITE_TIMEOUT));
    let _ = stream.set_nodelay(true);

    let Ok(tls_connection) = rustls::ServerConnection::new(server_config) else {
        return;
    };
    let mut tls = rustls::StreamOwned::new(tls_connection, stream);
    let mut frames = FrameStream::with_limit(&mut tls, MAX_NETWORK_FRAME_BYTES);

    loop {
        match frames.read_frame() {
            Ok(Some(request)) => match respond(&core, &request) {
                Some(reply) => {
                    if frames.write_frame(&reply).is_err() {
                        break;
                    }
                }
                None => {
                    eprintln!("harbor-server: closed a connection that sent an unparseable frame");
                    break;
                }
            },
            Ok(None) => break,
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

/// One request, one correlated response. `None` means no frame goes back and
/// the connection is closed — either the bytes are not an authenticated
/// request, or handling failed for a reason worth a distinct log line.
fn respond(core: &Mutex<ServerCore>, bytes: &[u8]) -> Option<Vec<u8>> {
    let authenticated: AuthenticatedEnvelope = match serde_json::from_slice(bytes) {
        Ok(value) => value,
        Err(_) => {
            eprintln!("harbor-server: closed a connection that sent an unparseable frame");
            return None;
        }
    };
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    let outcome = match core.lock() {
        Ok(mut server) => server.handle(authenticated, now),
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
                    "harbor_id": "harbor-transport-test",
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
                    "harbor_id": "harbor-transport-test",
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
