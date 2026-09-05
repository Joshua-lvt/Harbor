//! TLS client for the Harbor control-plane server.
//!
//! The server presents a self-signed certificate; trust comes exclusively
//! from the SHA-256 fingerprint of that certificate, pinned out-of-band (the
//! server's startup line or the pairing material shared by the other device).
//! No CA validation, no hostname logic: a served certificate whose fingerprint
//! differs from the pin fails before any request is sent.

use std::sync::Arc;

use harbor_protocol::{Envelope, FrameError, FrameStream};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::LocalIdentity;

/// How to reach and trust the server: a socket address and the hex-encoded
/// SHA-256 of its certificate DER (64 hex characters).
///
/// The address accepts `host:port`, bracketed IPv6 (`[2001:db8::1]:9091`),
/// and DNS names. Resolution happens at connect time and every resolved
/// address is tried in order, so one name can carry both AAAA and A records
/// and the client naturally prefers whatever the network actually routes.
/// The pin is certificate-derived and therefore transport-agnostic: the same
/// fingerprint secures IPv4, IPv6, and hostname endpoints alike.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServerPin {
    pub address: String,
    pub fingerprint_hex: String,
}

impl ServerPin {
    pub fn parse(
        address: impl Into<String>,
        fingerprint_hex: &str,
    ) -> Result<Self, ServerClientError> {
        let address = address.into();
        validate_address(&address)?;
        decode_fingerprint(fingerprint_hex)?;
        Ok(Self {
            address,
            fingerprint_hex: fingerprint_hex.trim().to_ascii_lowercase(),
        })
    }

    fn fingerprint(&self) -> [u8; 32] {
        decode_fingerprint(&self.fingerprint_hex).expect("validated at construction")
    }
}

fn validate_address(address: &str) -> Result<(), ServerClientError> {
    if address.is_empty()
        || address.len() > 255
        || address.trim() != address
        || address
            .chars()
            .any(|character| character.is_control() || character.is_whitespace())
    {
        return Err(ServerClientError::InvalidAddress);
    }

    let (host, port) = if let Some(rest) = address.strip_prefix('[') {
        let (host, suffix) = rest
            .split_once(']')
            .ok_or(ServerClientError::InvalidAddress)?;
        let port = suffix
            .strip_prefix(':')
            .ok_or(ServerClientError::InvalidAddress)?;
        (host, port)
    } else {
        let (host, port) = address
            .rsplit_once(':')
            .ok_or(ServerClientError::InvalidAddress)?;
        if host.contains(':') {
            return Err(ServerClientError::InvalidAddress);
        }
        (host, port)
    };

    if host.is_empty()
        || port.is_empty()
        || port.parse::<u16>().ok().filter(|port| *port != 0).is_none()
    {
        return Err(ServerClientError::InvalidAddress);
    }

    if host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(());
    }

    // Hostnames are accepted without resolving them here. Resolution and
    // connectivity belong to the first real TLS request, not configuration.
    if host.len() > 253
        || host.split('.').any(|label| {
            label.is_empty()
                || label.len() > 63
                || !label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
                || label.starts_with('-')
                || label.ends_with('-')
        })
    {
        return Err(ServerClientError::InvalidAddress);
    }
    Ok(())
}

/// Dials the pinned address with a bounded timeout per candidate. DNS names
/// resolve to every address family the network offers and each is tried in
/// order; the first success wins and the last failure is reported.
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

fn dial(pin: &ServerPin) -> Result<std::net::TcpStream, ServerClientError> {
    use std::net::ToSocketAddrs as _;
    let mut last_error = std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "server address did not resolve",
    );
    for address in pin
        .address
        .as_str()
        .to_socket_addrs()
        .map_err(ServerClientError::Connect)?
    {
        match std::net::TcpStream::connect_timeout(&address, CONNECT_TIMEOUT) {
            Ok(socket) => return Ok(socket),
            Err(error) => last_error = error,
        }
    }
    // Debug logging only: which endpoint failed is an operator fact, and it
    // never leaves this process (failures surface as retryable ui_keys).
    eprintln!(
        "harbor-core: control-plane dial failed for {}: {last_error}",
        pin.address
    );
    Err(ServerClientError::Connect(last_error))
}

fn decode_fingerprint(fingerprint_hex: &str) -> Result<[u8; 32], ServerClientError> {
    let trimmed = fingerprint_hex.trim();
    if trimmed.len() != 64 || !trimmed.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ServerClientError::InvalidFingerprint);
    }
    let mut fingerprint = [0_u8; 32];
    for (index, chunk) in trimmed.as_bytes().chunks_exact(2).enumerate() {
        let high = (chunk[0] as char).to_digit(16).expect("validated hex") as u8;
        let low = (chunk[1] as char).to_digit(16).expect("validated hex") as u8;
        fingerprint[index] = (high << 4) | low;
    }
    Ok(fingerprint)
}

#[derive(Debug, Error)]
pub enum ServerClientError {
    #[error("server address is invalid")]
    InvalidAddress,
    #[error("server certificate fingerprint is invalid")]
    InvalidFingerprint,
    #[error("connect failed: {0}")]
    Connect(#[from] std::io::Error),
    #[error("tls handshake failed: {0}")]
    Tls(#[from] rustls::Error),
    #[error("server certificate does not match the pinned fingerprint")]
    UntrustedCertificate,
    #[error("protocol framing failed: {0}")]
    Frame(#[from] FrameError),
    #[error("server response could not be parsed as an envelope")]
    InvalidResponse,
    #[error("the response does not correlate with the request")]
    UnrelatedResponse,
}

/// One pinned connection to the server. Requests are signed with the local
/// identity; responses are correlated envelopes. The client is deliberately
/// stateless beyond the TLS session — pairing, presence, and session logic
/// stay in the domain layers.
pub struct ServerClient {
    frames: FrameStream<rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream>>,
}

impl ServerClient {
    /// Connects and completes the TLS handshake, refusing any certificate
    /// that does not match the pin.
    ///
    /// Every resolved address is dialed with a bounded timeout instead of
    /// the OS default (which can stall for minutes on a blackholed route),
    /// so an unreachable endpoint fails in seconds and the caller can move
    /// on instead of hanging the UI.
    pub fn connect(pin: &ServerPin) -> Result<Self, ServerClientError> {
        let rejected = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let verifier = PinnedFingerprintVerifier {
            fingerprint: pin.fingerprint(),
            rejected: Arc::clone(&rejected),
        };
        let config = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(verifier))
            .with_no_client_auth();
        // The verifier ignores the name, but rustls still requires a valid
        // ServerName; the server's certificate CN doubles as the SNI value.
        let server_name = ServerName::try_from("harbor-server".to_owned())
            .map_err(|_| ServerClientError::InvalidAddress)?;
        let mut connection = rustls::ClientConnection::new(Arc::new(config), server_name)?;
        let mut socket = dial(pin)?;
        socket.set_nodelay(true)?;

        // Drive the handshake now, so an untrusted certificate fails at
        // connect time rather than silently deferring to the first exchange.
        // The verifier flags the rejection so a handshake error caused by the
        // pin is reported precisely, without string-matching alert text.
        let handshake = connection.complete_io(&mut socket);
        if rejected.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(ServerClientError::UntrustedCertificate);
        }
        handshake?;

        Ok(Self {
            frames: FrameStream::new(rustls::StreamOwned::new(connection, socket)),
        })
    }

    /// Signs and sends one request envelope, then waits for the correlated
    /// response. Server-side errors arrive as the response envelope's `error`,
    /// not as a transport failure.
    pub fn exchange(
        &mut self,
        request: Envelope,
        identity: &LocalIdentity,
    ) -> Result<Envelope, ServerClientError> {
        let signed = identity
            .sign_envelope(request)
            .map_err(|_| ServerClientError::InvalidResponse)?;
        let request_id = signed.envelope.request_id.clone();
        let bytes = serde_json::to_vec(&signed).map_err(|_| ServerClientError::InvalidResponse)?;
        self.frames.write_frame(&bytes)?;

        let reply = self
            .frames
            .read_frame()?
            .ok_or(ServerClientError::InvalidResponse)?;
        let response: Envelope =
            serde_json::from_slice(&reply).map_err(|_| ServerClientError::InvalidResponse)?;
        if response.reply_to.as_deref() != request_id.as_deref() {
            return Err(ServerClientError::UnrelatedResponse);
        }
        Ok(response)
    }
}

/// The production trust decision: the served certificate must hash to the
/// pinned fingerprint. Signature schemes delegate to the ring provider so the
/// TLS handshake itself stays fully verified.
#[derive(Debug)]
struct PinnedFingerprintVerifier {
    fingerprint: [u8; 32],
    rejected: Arc<std::sync::atomic::AtomicBool>,
}

impl rustls::client::danger::ServerCertVerifier for PinnedFingerprintVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let served: [u8; 32] = Sha256::digest(end_entity.as_ref()).into();
        if served == self.fingerprint {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            self.rejected
                .store(true, std::sync::atomic::Ordering::SeqCst);
            Err(rustls::Error::General(
                "server certificate does not match the pinned fingerprint".into(),
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

/// Current wall clock as an RFC 3339 string for outgoing request envelopes.
pub fn rfc3339_now() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_secs() as i64;
    time::OffsetDateTime::from_unix_timestamp(now)
        .expect("unix seconds are a representable OffsetDateTime")
        .format(&time::format_description::well_known::Rfc3339)
        .expect("RFC 3339 formatting of a valid timestamp cannot fail")
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};

    use harbor_protocol::Envelope;
    use harbor_server::{Listener, ListenerConfig};
    use serde_json::json;
    use uuid::Uuid;

    use super::*;

    fn spawned_server() -> (
        Listener,
        Arc<Mutex<harbor_server::ServerCore>>,
        String,
        PathBuf,
    ) {
        let directory =
            std::env::temp_dir().join(format!("harbor-core-server-test-{}", Uuid::new_v4()));
        let core = Arc::new(Mutex::new(
            harbor_server::ServerCore::open(&directory.join("state")).unwrap(),
        ));
        let listener = Listener::spawn(
            ListenerConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                tls_dir: directory.join("tls"),
                cert_pem: None,
                key_pem: None,
            },
            Arc::clone(&core),
        )
        .unwrap();
        let fingerprint = listener
            .certificate_fingerprint()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        (listener, core, fingerprint, directory)
    }

    fn pinned(address: String, fingerprint: &str) -> ServerPin {
        ServerPin::parse(address, fingerprint).unwrap()
    }

    fn identity() -> LocalIdentity {
        let directory = std::env::temp_dir().join(format!("harbor-core-client-{}", Uuid::new_v4()));
        crate::load_or_create(&directory, 1).unwrap()
    }

    fn identity_update(device_id: Uuid, harbor_id: &str, public_key: &str) -> Envelope {
        Envelope::request(
            "identity.update",
            json!({
                "device_id": device_id,
                "harbor_id": harbor_id,
                "public_key": public_key,
            }),
            rfc3339_now(),
        )
    }

    #[test]
    fn pinned_clients_exchange_signed_requests_with_the_real_server() {
        let (listener, core, fingerprint, directory) = spawned_server();
        let local = identity();
        let device_id = local.record().device_id;
        let public_key = local.record().public_key.clone();

        let mut client =
            ServerClient::connect(&pinned(listener.local_addr().to_string(), &fingerprint))
                .unwrap();
        let registered = client
            .exchange(
                identity_update(device_id, "harbor-core-client", &public_key),
                &local,
            )
            .unwrap();
        assert!(registered.error.is_none());
        assert!(registered.reply_to.is_some());
        assert!(core.lock().unwrap().identity(device_id).is_some());

        // A subsequent pairing request over the same connection is signed and
        // accepted by the server.
        let pairing = client
            .exchange(
                Envelope::request("pairing.create", json!({ "code": "112233" }), rfc3339_now()),
                &local,
            )
            .unwrap();
        assert!(pairing.error.is_none());

        listener.shutdown();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn a_wrong_fingerprint_never_completes_the_handshake() {
        let (listener, _core, _fingerprint, directory) = spawned_server();
        let wrong = pinned(listener.local_addr().to_string(), &"0".repeat(64));

        // The pin is enforced during connect, before any request is sent.
        assert!(matches!(
            ServerClient::connect(&wrong),
            Err(ServerClientError::UntrustedCertificate)
        ));

        listener.shutdown();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn malformed_pins_are_rejected_before_any_network_activity() {
        assert!(matches!(
            ServerPin::parse("127.0.0.1:1", "not-hex"),
            Err(ServerClientError::InvalidFingerprint)
        ));
        assert!(matches!(
            ServerPin::parse("127.0.0.1:1", &"a".repeat(63)),
            Err(ServerClientError::InvalidFingerprint)
        ));
    }

    #[test]
    fn malformed_addresses_are_rejected_before_persistence() {
        for address in [
            "",
            "127.0.0.1",
            "127.0.0.1:0",
            "127.0.0.1:65536",
            "[::1]:",
            "::1:443",
            "-harbor.example:443",
            "harbor..example:443",
            "harbor.example:443/extra",
        ] {
            assert!(
                matches!(
                    ServerPin::parse(address, &"a".repeat(64)),
                    Err(ServerClientError::InvalidAddress)
                ),
                "accepted invalid address {address:?}"
            );
        }

        assert!(ServerPin::parse("[::1]:443", &"a".repeat(64)).is_ok());
        assert!(ServerPin::parse("harbor.example:443", &"a".repeat(64)).is_ok());
    }

    /// The pinned client dials bracketed IPv6 endpoints with the same
    /// handshake, pinning, and signed exchange as IPv4: the pin is
    /// transport-agnostic.
    #[test]
    fn pinned_clients_dial_ipv6_loopback() {
        let directory =
            std::env::temp_dir().join(format!("harbor-core-server-v6-{}", Uuid::new_v4()));
        let core = Arc::new(Mutex::new(
            harbor_server::ServerCore::open(&directory.join("state")).unwrap(),
        ));
        let listener = match Listener::spawn(
            ListenerConfig {
                bind: "[::1]:0".parse().unwrap(),
                tls_dir: directory.join("tls"),
                cert_pem: None,
                key_pem: None,
            },
            Arc::clone(&core),
        ) {
            Ok(listener) => listener,
            Err(error) => {
                eprintln!("skipping IPv6 test: cannot bind [::1] ({error})");
                return;
            }
        };
        let fingerprint = listener
            .certificate_fingerprint()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let address = format!("[::1]:{}", listener.local_addr().port());
        let local = identity();
        let device_id = local.record().device_id;
        let public_key = local.record().public_key.clone();
        let mut client = ServerClient::connect(&pinned(address, &fingerprint)).unwrap();
        let registered = client
            .exchange(
                identity_update(device_id, "harbor-core-client-v6", &public_key),
                &local,
            )
            .unwrap();
        assert!(registered.error.is_none(), "{registered:?}");

        listener.shutdown();
        std::fs::remove_dir_all(directory).unwrap();
    }

    /// A blackholed route fails in seconds, not OS-timeout minutes: dialing
    /// uses a bounded timeout per resolved address.
    #[test]
    fn unreachable_endpoints_fail_fast() {
        use std::time::Instant;
        // TEST-NET-1 is unroutable by definition; nothing answers there.
        let pin = pinned("192.0.2.1:9091".to_owned(), &"a".repeat(64));
        let started = Instant::now();
        let error = match ServerClient::connect(&pin) {
            Ok(_) => panic!("unroutable address must not connect"),
            Err(error) => error,
        };
        assert!(
            matches!(error, ServerClientError::Connect(_)),
            "unexpected error: {error:?}"
        );
        assert!(
            started.elapsed() < std::time::Duration::from_secs(60),
            "dial must stay bounded, took {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn server_side_rejections_arrive_as_error_envelopes() {
        let (listener, _core, fingerprint, directory) = spawned_server();
        let local = identity();

        let mut client =
            ServerClient::connect(&pinned(listener.local_addr().to_string(), &fingerprint))
                .unwrap();
        // A pairing request from an unregistered device is refused by the
        // server as a protocol error, not a transport failure.
        let refused = client
            .exchange(
                Envelope::request("pairing.create", json!({ "code": "445566" }), rfc3339_now()),
                &local,
            )
            .unwrap();
        assert_eq!(refused.error.unwrap().code, "unauthorized");

        listener.shutdown();
        std::fs::remove_dir_all(directory).unwrap();
    }
}
