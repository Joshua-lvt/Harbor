//! Gate-9 T1 canary: authenticated TURN against the REAL Oracle staging
//! server over the network — Allocate with long-term credentials, mutual
//! permissions, and bidirectional media between two provisioned devices,
//! asserting the relayed address is the public IP (never the guest-private
//! or wildcard address) with a port from the relay range.
//!
//! The two devices run on this same machine but meet only through the
//! server: A sends to B's relayed address, the server forwards from A's
//! dedicated socket, and B's dedicated socket wraps it for B's client (and
//! back). That hairpin is exactly the path `relay_receipt`'s own-source
//! bypass exists for.
//!
//! Runs only with operator configuration (nothing compiled in):
//!   HARBOR_T1_HOST         Oracle IPv4 under test (e.g. from HARBOR_ORACLE_ADDRESS)
//!   HARBOR_T1_FINGERPRINT  64-hex staging pin (NOT the K11 production pin)
//!   HARBOR_T1_EXPECT_IP    public IP the relay must advertise
//!   HARBOR_T1_PORT         control/UDP port (default 9091)
//!   HARBOR_T1_PORT_RANGE   relay range under test (default 49160-49175)
//! Without HOST/FINGERPRINT/EXPECT_IP the test passes as skipped.

use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use ed25519_dalek::SigningKey;
use harbor_protocol::{AuthenticatedEnvelope, Envelope, FrameStream};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const REALM: &str = "harbor";
const UDP_TIMEOUT: Duration = Duration::from_secs(5);
const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const TCP_IO_TIMEOUT: Duration = Duration::from_secs(30);

// --- STUN wire ---------------------------------------------------------------

const CLASS_REQUEST: u16 = 0b00;
const CLASS_INDICATION: u16 = 0b01;
const CLASS_SUCCESS: u16 = 0b10;
const CLASS_ERROR: u16 = 0b11;
const METHOD_ALLOCATE: u16 = 0x003;
const METHOD_SEND: u16 = 0x006;
const METHOD_DATA: u16 = 0x007;
const METHOD_CREATE_PERMISSION: u16 = 0x008;
const ATTR_USERNAME: u16 = 0x0006;
const ATTR_MESSAGE_INTEGRITY: u16 = 0x0008;
const ATTR_ERROR_CODE: u16 = 0x0009;
const ATTR_REALM: u16 = 0x0014;
const ATTR_NONCE: u16 = 0x0015;
const ATTR_XOR_RELAYED_ADDRESS: u16 = 0x0016;
const ATTR_XOR_PEER_ADDRESS: u16 = 0x0012;
const ATTR_DATA: u16 = 0x0013;
const ATTR_REQUESTED_TRANSPORT: u16 = 0x0019;
const ATTR_FINGERPRINT: u16 = 0x8028;
const MAGIC: u32 = 0x2112A442;
const FINGERPRINT_XOR: u32 = 0x5354554E;

fn msg_type(method: u16, class: u16) -> u16 {
    let method = method & 0x0FFF;
    let class = class & 0x03;
    (method & 0x000F)
        | (((method >> 4) & 0x0007) << 5)
        | (((method >> 7) & 0x001F) << 9)
        | ((class & 0x01) << 4)
        | (((class >> 1) & 0x01) << 8)
}

fn long_term_key(username: &str, realm: &str, password: &str) -> [u8; 16] {
    let mut input = Vec::new();
    input.extend_from_slice(username.as_bytes());
    input.push(b':');
    input.extend_from_slice(realm.as_bytes());
    input.push(b':');
    input.extend_from_slice(password.as_bytes());
    md5::compute(&input).0
}

fn hmac_sha1(key: &[u8], message: &[u8]) -> [u8; 20] {
    use hmac::Mac as _;
    let mut mac = hmac::Hmac::<sha1::Sha1>::new_from_slice(key).expect("hmac takes any key size");
    mac.update(message);
    mac.finalize().into_bytes().into()
}

fn fingerprint_of(message: &[u8]) -> u32 {
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(message);
    hasher.finalize() ^ FINGERPRINT_XOR
}

fn pad_attr(body: &mut Vec<u8>, attr_type: u16, value: &[u8]) {
    body.extend_from_slice(&attr_type.to_be_bytes());
    body.extend_from_slice(&(value.len() as u16).to_be_bytes());
    body.extend_from_slice(value);
    body.extend(std::iter::repeat_n(
        0_u8,
        value.len().next_multiple_of(4) - value.len(),
    ));
}

/// Client-faithful authed request: attrs + long-term MI + fingerprint.
fn authed_request(
    method: u16,
    txid: [u8; 12],
    attrs: Vec<(u16, Vec<u8>)>,
    username: &str,
    password: &str,
) -> Vec<u8> {
    let mut body = Vec::new();
    for (attr_type, value) in &attrs {
        pad_attr(&mut body, *attr_type, value);
    }
    let key = long_term_key(username, REALM, password);
    let total = body.len() + 4 + 20 + 4 + 4;
    let mut message = Vec::with_capacity(20 + total);
    message.extend_from_slice(&msg_type(method, CLASS_REQUEST).to_be_bytes());
    message.extend_from_slice(&(total as u16).to_be_bytes());
    message.extend_from_slice(&MAGIC.to_be_bytes());
    message.extend_from_slice(&txid);
    message.extend_from_slice(&body);
    let mut hmac_input = Vec::with_capacity(20 + body.len());
    hmac_input.extend_from_slice(&msg_type(method, CLASS_REQUEST).to_be_bytes());
    hmac_input.extend_from_slice(&((body.len() + 4 + 20) as u16).to_be_bytes());
    hmac_input.extend_from_slice(&MAGIC.to_be_bytes());
    hmac_input.extend_from_slice(&txid);
    hmac_input.extend_from_slice(&body);
    message.extend_from_slice(&ATTR_MESSAGE_INTEGRITY.to_be_bytes());
    message.extend_from_slice(&20_u16.to_be_bytes());
    message.extend_from_slice(&hmac_sha1(&key, &hmac_input));
    let fingerprint = fingerprint_of(&message);
    message.extend_from_slice(&ATTR_FINGERPRINT.to_be_bytes());
    message.extend_from_slice(&4_u16.to_be_bytes());
    message.extend_from_slice(&fingerprint.to_be_bytes());
    message
}

fn parse_attrs(bytes: &[u8]) -> Vec<(u16, Vec<u8>)> {
    let mut attrs = Vec::new();
    let mut offset = 0_usize;
    while offset + 4 <= bytes.len() {
        let attr_type = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
        let length = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]) as usize;
        let padded = length.next_multiple_of(4);
        if offset + 4 + length > bytes.len() {
            break;
        }
        attrs.push((attr_type, bytes[offset + 4..offset + 4 + length].to_vec()));
        offset += 4 + padded;
    }
    attrs
}

fn find_attr(attrs: &[(u16, Vec<u8>)], attr_type: u16) -> Option<&[u8]> {
    attrs
        .iter()
        .find(|(kind, _)| *kind == attr_type)
        .map(|(_, value)| value.as_slice())
}

fn error_code(attrs: &[(u16, Vec<u8>)]) -> Option<u16> {
    let raw = find_attr(attrs, ATTR_ERROR_CODE)?;
    if raw.len() < 4 {
        return None;
    }
    Some(raw[2] as u16 * 100 + raw[3] as u16)
}

fn parse_reply(reply: &[u8]) -> Reply {
    assert!(reply.len() >= 20, "reply too short: {}", reply.len());
    let msg_type = u16::from_be_bytes([reply[0], reply[1]]);
    let length = u16::from_be_bytes([reply[2], reply[3]]) as usize;
    assert_eq!(reply.len(), 20 + length, "reply length mismatch");
    let txid: [u8; 12] = reply[8..20].try_into().unwrap();
    (msg_type, txid, parse_attrs(&reply[20..]))
}

type Attrs = Vec<(u16, Vec<u8>)>;
type Reply = (u16, [u8; 12], Attrs);

fn txid(n: u8) -> [u8; 12] {
    [
        n,
        n + 1,
        n + 2,
        n + 3,
        n + 4,
        n + 5,
        n + 6,
        n + 7,
        n + 8,
        n + 9,
        n + 10,
        n + 11,
    ]
}

// --- pinned TLS control plane -------------------------------------------------

#[derive(Debug)]
struct PinVerifier {
    fingerprint: [u8; 32],
}

impl rustls::client::danger::ServerCertVerifier for PinVerifier {
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
            Err(rustls::Error::General("pin mismatch".into()))
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

type TlsFrames = FrameStream<rustls::StreamOwned<rustls::ClientConnection, std::net::TcpStream>>;

fn tls_connect(host: &str, port: u16, fingerprint_hex: &str) -> TlsFrames {
    let mut fingerprint = [0_u8; 32];
    assert_eq!(fingerprint_hex.len(), 64, "pin must be 64 hex chars");
    for (index, chunk) in fingerprint_hex.as_bytes().chunks_exact(2).enumerate() {
        let high = (chunk[0] as char).to_digit(16).expect("pin hex") as u8;
        let low = (chunk[1] as char).to_digit(16).expect("pin hex") as u8;
        fingerprint[index] = (high << 4) | low;
    }
    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinVerifier { fingerprint }))
        .with_no_client_auth();
    let server_name = ServerName::try_from("harbor-server".to_owned()).unwrap();
    let mut connection = rustls::ClientConnection::new(Arc::new(config), server_name).unwrap();
    let address: SocketAddr = format!("{host}:{port}")
        .to_socket_addrs()
        .expect("T1 host resolves")
        .next()
        .expect("T1 host has an address");
    let mut socket = std::net::TcpStream::connect_timeout(&address, TCP_CONNECT_TIMEOUT)
        .expect("T1 TCP connects");
    socket.set_nodelay(true).unwrap();
    socket.set_read_timeout(Some(TCP_IO_TIMEOUT)).unwrap();
    socket.set_write_timeout(Some(TCP_CONNECT_TIMEOUT)).unwrap();
    connection
        .complete_io(&mut socket)
        .expect("T1 TLS handshake");
    FrameStream::new(rustls::StreamOwned::new(connection, socket))
}

fn now_rfc3339() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap()
}

fn rpc(
    frames: &mut TlsFrames,
    key: &SigningKey,
    device_id: Uuid,
    message_type: &str,
    payload: serde_json::Value,
) -> Envelope {
    let request = Envelope::request(message_type, payload, now_rfc3339());
    let request_id = request.request_id.clone();
    let signed = AuthenticatedEnvelope::sign(device_id, request, key).unwrap();
    let bytes = serde_json::to_vec(&signed).unwrap();
    frames.write_frame(&bytes).unwrap();
    let reply = frames.read_frame().unwrap().expect("T1 reply arrives");
    let response: Envelope = serde_json::from_slice(&reply).unwrap();
    assert_eq!(response.reply_to, request_id, "T1 reply correlates");
    assert!(
        response.error.is_none(),
        "T1 {message_type} refused: {:?}",
        response.error
    );
    response
}

// --- one provisioned device ----------------------------------------------------

struct Device {
    username: String,
    password: String,
    sock: UdpSocket,
    relayed: SocketAddr,
}

impl Device {
    /// Registers one fresh identity, mints its TURN credentials, and
    /// allocates over UDP: challenge first (proves the state machine),
    /// then the authenticated Allocate whose relayed address is returned.
    fn provision(frames: &mut TlsFrames, seed: u8, server: SocketAddr) -> (Self, Vec<u8>) {
        let key = SigningKey::from_bytes(&[seed; 32]);
        let device_id = Uuid::new_v4();
        let public_key = STANDARD_NO_PAD.encode(key.verifying_key().as_bytes());
        rpc(
            frames,
            &key,
            device_id,
            "identity.update",
            serde_json::json!({
                "device_id": device_id,
                "harbor_id": format!("t1-{seed}"),
                "public_key": public_key,
            }),
        );
        let creds = rpc(
            frames,
            &key,
            device_id,
            "turn.credentials",
            serde_json::json!({}),
        );
        let username = creds.payload["username"].as_str().unwrap().to_owned();
        let password = creds.payload["password"].as_str().unwrap().to_owned();
        assert_eq!(creds.payload["realm"].as_str().unwrap(), REALM);

        let sock = UdpSocket::bind("0.0.0.0:0").unwrap();
        sock.set_read_timeout(Some(UDP_TIMEOUT)).unwrap();
        sock.set_write_timeout(Some(UDP_TIMEOUT)).unwrap();

        // Bare Allocate must challenge (proves the state machine answers).
        let anon = vec![
            0x00, 0x03, 0x00, 0x08, 0x21, 0x12, 0xA4, 0x42, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
            0x00, 0x19, 0x00, 0x04, 0x11, 0x00, 0x00, 0x00,
        ];
        sock.send_to(&anon, server).unwrap();
        let mut buffer = [0_u8; 2048];
        let (count, _) = sock.recv_from(&mut buffer).expect("T1 401 arrives");
        let (got_type, _, attrs) = parse_reply(&buffer[..count]);
        assert_eq!(got_type, msg_type(METHOD_ALLOCATE, CLASS_ERROR));
        assert_eq!(error_code(&attrs), Some(401));
        let nonce = find_attr(&attrs, ATTR_NONCE).unwrap().to_vec();
        let realm = find_attr(&attrs, ATTR_REALM).unwrap().to_vec();
        assert!(!nonce.is_empty() && !realm.is_empty());

        // Authed Allocate.
        let tx = txid(seed);
        let request = authed_request(
            METHOD_ALLOCATE,
            tx,
            vec![
                (ATTR_REQUESTED_TRANSPORT, vec![17, 0, 0, 0]),
                (ATTR_USERNAME, username.as_bytes().to_vec()),
                (ATTR_REALM, realm),
                (ATTR_NONCE, nonce.clone()),
            ],
            &username,
            &password,
        );
        sock.send_to(&request, server).unwrap();
        let (count, _) = sock.recv_from(&mut buffer).expect("T1 Allocate answers");
        let (got_type, echo, attrs) = parse_reply(&buffer[..count]);
        assert_eq!(got_type, msg_type(METHOD_ALLOCATE, CLASS_SUCCESS));
        assert_eq!(echo, tx);
        let relayed_raw = find_attr(&attrs, ATTR_XOR_RELAYED_ADDRESS).expect("relayed present");
        let relayed =
            harbor_protocol::stun::xor_addr_decode(relayed_raw, &tx).expect("relayed decodes");
        (
            Self {
                username,
                password,
                sock,
                relayed,
            },
            nonce,
        )
    }

    fn permission(&self, server: SocketAddr, peer: SocketAddr, nonce: &[u8], n: u8) {
        let tx = txid(n);
        let request = authed_request(
            METHOD_CREATE_PERMISSION,
            tx,
            vec![
                (ATTR_USERNAME, self.username.as_bytes().to_vec()),
                (ATTR_REALM, REALM.as_bytes().to_vec()),
                (ATTR_NONCE, nonce.to_vec()),
                (
                    ATTR_XOR_PEER_ADDRESS,
                    harbor_protocol::stun::xor_addr_value(peer, &tx),
                ),
            ],
            &self.username,
            &self.password,
        );
        self.sock.send_to(&request, server).unwrap();
        let mut buffer = [0_u8; 2048];
        let (count, _) = self
            .sock
            .recv_from(&mut buffer)
            .expect("T1 permission answers");
        let (got_type, _, attrs) = parse_reply(&buffer[..count]);
        assert_eq!(
            got_type,
            msg_type(METHOD_CREATE_PERMISSION, CLASS_SUCCESS),
            "permission for {peer} must succeed (error {:?})",
            error_code(&attrs)
        );
    }

    fn send_to(&self, server: SocketAddr, peer: SocketAddr, data: &[u8], n: u8) {
        let tx = txid(n);
        let mut message = msg_type(METHOD_SEND, CLASS_INDICATION)
            .to_be_bytes()
            .to_vec();
        message.extend_from_slice(&0_u16.to_be_bytes());
        message.extend_from_slice(&MAGIC.to_be_bytes());
        message.extend_from_slice(&tx);
        let mut attrs = Vec::new();
        let peer_value = harbor_protocol::stun::xor_addr_value(peer, &tx);
        attrs.extend_from_slice(&ATTR_XOR_PEER_ADDRESS.to_be_bytes());
        attrs.extend_from_slice(&(peer_value.len() as u16).to_be_bytes());
        attrs.extend_from_slice(&peer_value);
        attrs.extend_from_slice(&ATTR_DATA.to_be_bytes());
        attrs.extend_from_slice(&(data.len() as u16).to_be_bytes());
        attrs.extend_from_slice(data);
        attrs.extend(std::iter::repeat_n(
            0_u8,
            data.len().next_multiple_of(4) - data.len(),
        ));
        message[2..4].copy_from_slice(&(attrs.len() as u16).to_be_bytes());
        message.extend_from_slice(&attrs);
        self.sock.send_to(&message, server).unwrap();
    }

    fn recv_data(&self, expect: &[u8], expect_peer_port: u16) {
        let mut buffer = [0_u8; 2048];
        let (count, _) = self
            .sock
            .recv_from(&mut buffer)
            .expect("T1 peer media arrives");
        let (got_type, txid, attrs) = parse_reply(&buffer[..count]);
        assert_eq!(got_type, msg_type(METHOD_DATA, CLASS_INDICATION));
        let from_raw = find_attr(&attrs, ATTR_XOR_PEER_ADDRESS).expect("peer present");
        let from = harbor_protocol::stun::xor_addr_decode(from_raw, &txid).expect("peer decodes");
        // The hairpinned forward arrives from OUR OWN relay socket (guest
        // address, sender's relay port) — the own-source path that
        // destination policy would otherwise drop.
        assert_eq!(
            from.port(),
            expect_peer_port,
            "indication carries the sender relay port"
        );
        let data = find_attr(&attrs, ATTR_DATA).expect("data present");
        assert_eq!(data, expect, "media bytes survive the relay");
    }
}

fn public_ip_or_panic(value: &str) -> std::net::IpAddr {
    let ip: std::net::IpAddr = value.parse().expect("EXPECT_IP parses");
    assert!(
        !ip.is_loopback() && !ip.is_multicast() && !ip.is_unspecified(),
        "EXPECT_IP must be public, got {ip}"
    );
    if let std::net::IpAddr::V4(v4) = ip {
        let [a, b, _, _] = v4.octets();
        assert!(
            !(a == 10 || (a == 172 && (16..=31).contains(&b)) || (a == 192 && b == 168)),
            "EXPECT_IP must be public, got {ip}"
        );
    }
    ip
}

#[test]
fn t1_turn_relay_is_bidirectional_with_public_advertisement() {
    let (Some(host), Some(fingerprint), Some(expect_ip)) = (
        std::env::var("HARBOR_T1_HOST").ok(),
        std::env::var("HARBOR_T1_FINGERPRINT").ok(),
        std::env::var("HARBOR_T1_EXPECT_IP").ok(),
    ) else {
        println!("skipping live T1: set HARBOR_T1_HOST/FINGERPRINT/EXPECT_IP");
        return;
    };
    let port: u16 = std::env::var("HARBOR_T1_PORT")
        .ok()
        .and_then(|raw| raw.parse().ok())
        .unwrap_or(9091);
    let (port_low, port_high): (u16, u16) = std::env::var("HARBOR_T1_PORT_RANGE")
        .ok()
        .and_then(|raw| {
            raw.split_once('-')
                .and_then(|(low, high)| Some((low.trim().parse().ok()?, high.trim().parse().ok()?)))
        })
        .unwrap_or((49160, 49175));
    let expect_ip = public_ip_or_panic(&expect_ip);
    let server: SocketAddr = format!("{host}:{port}")
        .to_socket_addrs()
        .expect("T1 server resolves")
        .next()
        .expect("T1 server has an address");

    // One TLS control connection provisions both devices; every exchange
    // is stateless, like the production client.
    let mut frames = tls_connect(&host, port, &fingerprint);
    let (device_a, nonce_a) = Device::provision(&mut frames, 21, server);
    let (device_b, nonce_b) = Device::provision(&mut frames, 22, server);

    // The advertisement must be the public IP (never guest-private or
    // wildcard) with a port from the relay range — the T1 invariant.
    for device in [&device_a, &device_b] {
        assert_eq!(
            device.relayed.ip(),
            expect_ip,
            "relayed address is the public IP"
        );
        assert!(
            (port_low..=port_high).contains(&device.relayed.port()),
            "relayed port {} in {port_low}-{port_high}",
            device.relayed.port()
        );
    }
    assert_ne!(
        device_a.relayed.port(),
        device_b.relayed.port(),
        "allocations own distinct relay sockets"
    );

    // Mutual permissions, then media both ways through the relay.
    device_a.permission(server, device_b.relayed, &nonce_a, 31);
    device_b.permission(server, device_a.relayed, &nonce_b, 32);
    device_a.send_to(server, device_b.relayed, b"hello-t1-a", 33);
    device_b.recv_data(b"hello-t1-a", device_a.relayed.port());
    device_b.send_to(server, device_a.relayed, b"hello-t1-b", 34);
    device_a.recv_data(b"hello-t1-b", device_b.relayed.port());

    println!(
        "T1 green: {} <-> {} via {host}:{port}",
        device_a.relayed, device_b.relayed
    );
}
