//! STUN-lite rendezvous wire format, shared by the server socket and the
//! core's reflexive lookup. Small JSON datagrams over UDP — not RFC 5389:
//!
//! ```json
//! {"type": "stun_lite_request", "nonce": "<opaque>"}
//! {"type": "stun_lite_response", "nonce": "<echo>", "observed_address": "<ip>", "observed_port": 1234}
//! ```
//!
//! The nonce is caller-opaque and only ever echoed; the server never trusts
//! any address the client claims — `observed_*` always comes from the
//! datagram's actual source. Anything malformed is dropped silently (no error
//! reply: this socket is not an oracle).

use std::net::IpAddr;

use serde_json::{Value, json};

pub const MAX_DATAGRAM_BYTES: usize = 512;
pub const REQUEST_TYPE: &str = "stun_lite_request";
pub const RESPONSE_TYPE: &str = "stun_lite_response";

/// RFC 5389 binding framing, spoken alongside the JSON lite dialect on the
/// same socket so stock WebRTC (Pion) can do server-reflexive discovery
/// without any new address to distribute.
pub const STUN_MAGIC_COOKIE: u32 = 0x2112A442;
pub const STUN_BINDING_REQUEST: u16 = 0x0001;
const STUN_BINDING_RESPONSE: u16 = 0x0101;
const STUN_ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;

/// True for what looks like an RFC 5389 binding request: 20+ bytes, top two
/// bits zero, type 0x0001, magic cookie present. Disjoint by construction
/// from RTP/RTCP (top bits 10, RFC 5389 §6) and from our JSON dialect
/// (`{` is 0x7B, top bits 01).
pub fn is_binding_request(bytes: &[u8]) -> bool {
    bytes.len() >= 20
        && bytes[0] & 0xC0 == 0
        && u16::from_be_bytes([bytes[0], bytes[1]]) == STUN_BINDING_REQUEST
        && u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) == STUN_MAGIC_COOKIE
}

/// Builds a binding success response for `source`, echoing the request's
/// 12-byte transaction id with an XOR-MAPPED-ADDRESS. No MESSAGE-INTEGRITY
/// (no shared secret in lite rendezvous) and no FINGERPRINT: plain-STUN
/// clients accept unauthenticated responses for reflexive discovery.
pub fn binding_response(request: &[u8], source: std::net::SocketAddr) -> Option<Vec<u8>> {
    if !is_binding_request(request) {
        return None;
    }
    let transaction: &[u8; 12] = request[8..20].try_into().ok()?;
    let value = xor_addr_value(source, transaction);
    let mut response = Vec::with_capacity(20 + 4 + value.len());
    response.extend_from_slice(&STUN_BINDING_RESPONSE.to_be_bytes());
    response.extend_from_slice(&(4 + value.len() as u16).to_be_bytes());
    response.extend_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());
    response.extend_from_slice(transaction);
    response.extend_from_slice(&STUN_ATTR_XOR_MAPPED_ADDRESS.to_be_bytes());
    response.extend_from_slice(&(value.len() as u16).to_be_bytes());
    response.extend_from_slice(&value);
    Some(response)
}

/// XOR-encoded address value (family + port + address, no TLV header) for
/// MAPPED-style attributes, per RFC 5389 §15.2. Shared by binding responses
/// and the TURN server (relayed/peer/mapped addresses all encode alike).
pub fn xor_addr_value(addr: std::net::SocketAddr, transaction: &[u8; 12]) -> Vec<u8> {
    let magic = STUN_MAGIC_COOKIE.to_be_bytes();
    let mut value = vec![
        0x00,
        match addr.ip() {
            std::net::IpAddr::V4(_) => 0x01,
            std::net::IpAddr::V6(_) => 0x02,
        },
    ];
    value.extend_from_slice(&(addr.port() ^ 0x2112).to_be_bytes());
    match addr.ip() {
        std::net::IpAddr::V4(ip) => {
            for (index, byte) in ip.octets().iter().enumerate() {
                value.push(byte ^ magic[index]);
            }
        }
        std::net::IpAddr::V6(ip) => {
            let mut mask = [0_u8; 16];
            mask[..4].copy_from_slice(&magic);
            mask[4..].copy_from_slice(transaction);
            for (index, byte) in ip.octets().iter().enumerate() {
                value.push(byte ^ mask[index]);
            }
        }
    }
    value
}

/// Decodes an XOR-encoded address value against `transaction`. Returns the
/// socket address or `None` for malformed values.
pub fn xor_addr_decode(value: &[u8], transaction: &[u8; 12]) -> Option<std::net::SocketAddr> {
    if value.len() < 4 || value[0] != 0x00 {
        return None;
    }
    let magic = STUN_MAGIC_COOKIE.to_be_bytes();
    let port = u16::from_be_bytes([value[2], value[3]]) ^ 0x2112;
    match value[1] {
        0x01 => {
            if value.len() != 8 {
                return None;
            }
            let mut octets = [0_u8; 4];
            for (index, byte) in value[4..8].iter().enumerate() {
                octets[index] = byte ^ magic[index];
            }
            Some(std::net::SocketAddr::from((
                std::net::Ipv4Addr::from(octets),
                port,
            )))
        }
        0x02 => {
            if value.len() != 20 {
                return None;
            }
            let mut mask = [0_u8; 16];
            mask[..4].copy_from_slice(&magic);
            mask[4..].copy_from_slice(transaction);
            let mut octets = [0_u8; 16];
            for (index, byte) in value[4..20].iter().enumerate() {
                octets[index] = byte ^ mask[index];
            }
            Some(std::net::SocketAddr::from((
                std::net::Ipv6Addr::from(octets),
                port,
            )))
        }
        _ => None,
    }
}

/// Nonces are short caller-chosen correlation strings: 1–64 chars of
/// `[A-Za-z0-9_-]`. Anything else is not a request.
pub fn valid_nonce(raw: &str) -> bool {
    if raw.is_empty() || raw.len() > 64 {
        return false;
    }
    raw.bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

/// Builds a request datagram, or `None` for an unacceptable nonce.
pub fn request_bytes(nonce: &str) -> Option<Vec<u8>> {
    if !valid_nonce(nonce) {
        return None;
    }
    let bytes = serde_json::to_vec(&json!({"type": REQUEST_TYPE, "nonce": nonce})).ok()?;
    if bytes.is_empty() || bytes.len() > MAX_DATAGRAM_BYTES {
        return None;
    }
    Some(bytes)
}

/// Extracts the nonce of a well-formed request datagram, or `None` for
/// anything the socket must silently drop (oversize, non-JSON, wrong shape).
pub fn parse_request(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() || bytes.len() > MAX_DATAGRAM_BYTES {
        return None;
    }
    let value: Value = serde_json::from_slice(bytes).ok()?;
    if value.get("type").and_then(Value::as_str) != Some(REQUEST_TYPE) {
        return None;
    }
    let nonce = value.get("nonce")?.as_str()?;
    if !valid_nonce(nonce) {
        return None;
    }
    Some(nonce.to_owned())
}

/// Builds the response echoing `nonce` for the observed source address.
pub fn response_bytes(nonce: &str, observed: std::net::SocketAddr) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "type": RESPONSE_TYPE,
        "nonce": nonce,
        "observed_address": observed.ip().to_string(),
        "observed_port": observed.port(),
    }))
    .unwrap_or_default()
}

/// A validated response echo: the nonce the caller sent plus the address the
/// server actually saw.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Echo {
    pub nonce: String,
    pub address: IpAddr,
    pub port: u16,
}

/// Validates a response datagram against the sent `nonce`. Address must parse
/// as a literal IP and the port must be nonzero; anything else is `None`.
pub fn parse_response(bytes: &[u8], nonce: &str) -> Option<Echo> {
    if bytes.is_empty() || bytes.len() > MAX_DATAGRAM_BYTES {
        return None;
    }
    let value: Value = serde_json::from_slice(bytes).ok()?;
    if value.get("type").and_then(Value::as_str) != Some(RESPONSE_TYPE) {
        return None;
    }
    if value.get("nonce").and_then(Value::as_str) != Some(nonce) {
        return None;
    }
    let address = value
        .get("observed_address")
        .and_then(Value::as_str)
        .and_then(|text| text.parse::<IpAddr>().ok())?;
    let port = value.get("observed_port").and_then(Value::as_u64)?;
    if port == 0 || port > 65535 {
        return None;
    }
    Some(Echo {
        nonce: nonce.to_owned(),
        address,
        port: port as u16,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_round_trips_through_parse() {
        let bytes = request_bytes("abc-123_XYZ").unwrap();
        assert_eq!(parse_request(&bytes).as_deref(), Some("abc-123_XYZ"));
    }

    #[test]
    fn bad_nonces_never_become_requests() {
        for nonce in ["", "has space", "semi;colon", "quote\"", "unicode-é"] {
            assert!(request_bytes(nonce).is_none(), "{nonce:?}");
        }
        assert!(request_bytes(&"n".repeat(65)).is_none());
        assert!(request_bytes(&"n".repeat(64)).is_some());
    }

    #[test]
    fn response_echoes_nonce_and_observed_source() {
        let addr: std::net::SocketAddr = "192.0.2.10:40001".parse().unwrap();
        let bytes = response_bytes("n-1", addr);
        assert_eq!(
            parse_response(&bytes, "n-1"),
            Some(Echo {
                nonce: "n-1".to_owned(),
                address: "192.0.2.10".parse().unwrap(),
                port: 40001,
            })
        );
    }

    #[test]
    fn binding_responses_echo_txid_with_xor_mapped_address() {
        // Crafted binding request, transaction id 0x01..0x0c.
        let mut request = vec![0x00, 0x01, 0x00, 0x00, 0x21, 0x12, 0xA4, 0x42];
        request.extend(1_u8..=12);
        assert!(is_binding_request(&request));

        // IPv4: port and address XOR the magic cookie.
        let source: std::net::SocketAddr = "192.0.2.10:40001".parse().unwrap();
        let reply = binding_response(&request, source).unwrap();
        assert!(matches!(&reply[..2], [0x01, 0x01]));
        assert!(matches!(&reply[4..8], [0x21, 0x12, 0xA4, 0x42]));
        assert_eq!(&reply[8..20], &request[8..20]);
        assert!(matches!(&reply[20..22], [0x00, 0x20]));
        assert!(matches!(&reply[22..24], [0x00, 0x08]));
        assert!(matches!(&reply[24..26], [0x00, 0x01]));
        let port = u16::from_be_bytes([reply[26], reply[27]]) ^ 0x2112;
        assert_eq!(port, 40001);
        let addr = [
            reply[28] ^ 0x21,
            reply[29] ^ 0x12,
            reply[30] ^ 0xA4,
            reply[31] ^ 0x42,
        ];
        assert_eq!(addr, [192, 0, 2, 10]);

        // IPv6: address XORs magic ++ transaction id.
        let source6: std::net::SocketAddr = "[2001:db8::1]:443".parse().unwrap();
        let reply6 = binding_response(&request, source6).unwrap();
        assert!(matches!(&reply6[22..24], [0x00, 0x14]));
        assert_eq!(&reply6[24..26], [0x00, 0x02]);
        let mut mask = [0_u8; 16];
        mask[..4].copy_from_slice(&[0x21, 0x12, 0xA4, 0x42]);
        mask[4..].copy_from_slice(&request[8..20]);
        let decoded: Vec<u8> = reply6[28..44]
            .iter()
            .zip(mask.iter())
            .map(|(byte, mask)| byte ^ mask)
            .collect();
        assert_eq!(
            decoded,
            [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]
        );
    }

    #[test]
    fn xor_addresses_round_trip_both_families() {
        let txid = [7_u8, 6, 5, 4, 3, 2, 1, 0, 9, 8, 7, 6];
        for addr in [
            "192.0.2.10:40001".parse().unwrap(),
            "127.0.0.1:9".parse().unwrap(),
            "[2001:db8::1]:443".parse().unwrap(),
            "[::1]:0".parse().unwrap(),
        ] {
            let value = xor_addr_value(addr, &txid);
            assert_eq!(xor_addr_decode(&value, &txid), Some(addr));
        }
        assert!(xor_addr_decode(&[], &txid).is_none());
        assert!(xor_addr_decode(&[0x00, 0x03, 0, 0], &txid).is_none());
        assert!(xor_addr_decode(&[0x00, 0x01, 0, 0, 1, 2], &txid).is_none());
    }

    #[test]
    fn binding_detection_rejects_short_wrong_type_and_bad_magic() {
        assert!(!is_binding_request(&[]));
        assert!(!is_binding_request(&[0x00, 0x01]));
        // RTP/RTCP-shaped (top bits 10).
        assert!(!is_binding_request(&[
            0x80, 0x01, 0, 0, 0x21, 0x12, 0xA4, 0x42, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12
        ]));
        // Wrong type with good magic.
        let mut wrong = vec![0x00, 0x02, 0x00, 0x00, 0x21, 0x12, 0xA4, 0x42];
        wrong.extend(1_u8..=12);
        assert!(!is_binding_request(&wrong));
        assert!(binding_response(&wrong, "127.0.0.1:9".parse().unwrap()).is_none());
        // JSON lite dialect never matches.
        assert!(!is_binding_request(br#"{"type": "stun_lite_request"}"#));
    }

    #[test]
    fn responses_reject_mismatched_nonce_bad_address_and_garbage() {
        let addr: std::net::SocketAddr = "[2001:db8::1]:443".parse().unwrap();
        let bytes = response_bytes("n-1", addr);
        assert!(parse_response(&bytes, "other").is_none());

        let mut wrong: Value = serde_json::from_slice(&bytes).unwrap();
        wrong["observed_address"] = json!("not-an-ip");
        assert!(parse_response(&serde_json::to_vec(&wrong).unwrap(), "n-1").is_none());

        wrong = serde_json::from_slice(&bytes).unwrap();
        wrong["observed_port"] = json!(0);
        assert!(parse_response(&serde_json::to_vec(&wrong).unwrap(), "n-1").is_none());

        for garbage in [
            &b"{}".to_vec()[..],
            b"\x00\x01",
            b"plain text",
            &vec![b'x'; 513][..],
        ] {
            assert!(parse_response(garbage, "n-1").is_none());
            assert!(parse_request(garbage).is_none());
        }
    }
}
