//! Minimal RFC 5766 TURN server for the media fallback path: Allocate with
//! long-term credentials, Refresh, CreatePermission, Send/Data indications,
//! ChannelBind + ChannelData. One UDP socket shared with STUN-lite (same
//! port); the relayed address IS that socket, so server-to-client Data
//! leaves from the endpoint clients already talk to (NAT-safe by
//! construction: the same 5-tuple family the control exchange uses).
//!
//! TURN-unaware peers send raw UDP at us; per-allocation learned peer
//! addresses route it back as Data indications (or ChannelData when bound).
//! Cross-allocation misdelivery (two pairs behind one public IP) fails
//! closed at the client (STUN ufrag / DTLS checks), never silently.
//!
//! Transport-independent core like the relay table: no sockets, no clock
//! inside (callers pass `now` in epoch seconds), no panics on any input.
//! Long-term keys are MD5(username:realm:password) exactly as RFC 5389 and
//! Pion derive them; our minted passwords are ASCII hex (SASLprep-stable).

use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::sync::Arc;

use crc32fast::Hasher as Crc32;
use hmac::{Hmac, Mac};
use sha1::Sha1;
use uuid::Uuid;

use harbor_protocol::stun::STUN_MAGIC_COOKIE;

pub const TURN_REALM: &str = "harbor";
pub const TURN_DEFAULT_LIFETIME_SECS: u64 = 600;
pub const TURN_MAX_LIFETIME_SECS: u64 = 600;
pub const TURN_MIN_LIFETIME_SECS: u64 = 60;
pub const TURN_PERMISSION_LIFETIME_SECS: u64 = 300;
pub const TURN_CHANNEL_LIFETIME_SECS: u64 = 600;
pub const TURN_NONCE_TTL_SECS: u64 = 3600;
pub const TURN_CRED_TTL_SECS: u64 = 3600;
pub const MAX_TURN_ALLOCATIONS: usize = 16;
pub const MAX_PERMISSIONS_PER_ALLOCATION: usize = 8;
pub const MAX_CHANNELS_PER_ALLOCATION: usize = 8;
pub const MAX_NONCES: usize = 128;
pub const TURN_FIRST_CHANNEL: u16 = 0x4000;
pub const TURN_LAST_CHANNEL: u16 = 0x7FFE;
pub const TURN_FINGERPRINT_XOR: u32 = 0x5354554e;
/// Per-allocation relay budget per one-second window, both directions. The
/// host VM is small; voice/video fits comfortably, sustained floods are
/// dropped silently (media tolerates loss better than latency).
pub const TURN_RELAY_BUDGET_BYTES_PER_SEC: u64 = 512 * 1024;

const CLASS_REQUEST: u16 = 0b00;
const CLASS_INDICATION: u16 = 0b01;
const CLASS_SUCCESS: u16 = 0b10;
const CLASS_ERROR: u16 = 0b11;

pub const METHOD_BINDING: u16 = 0x001;
pub const METHOD_ALLOCATE: u16 = 0x003;
pub const METHOD_REFRESH: u16 = 0x004;
pub const METHOD_SEND: u16 = 0x006;
pub const METHOD_DATA: u16 = 0x007;
pub const METHOD_CREATE_PERMISSION: u16 = 0x008;
pub const METHOD_CHANNEL_BIND: u16 = 0x009;

pub const ATTR_USERNAME: u16 = 0x0006;
pub const ATTR_MESSAGE_INTEGRITY: u16 = 0x0008;
pub const ATTR_ERROR_CODE: u16 = 0x0009;
pub const ATTR_UNKNOWN_ATTRIBUTES: u16 = 0x000A;
pub const ATTR_REALM: u16 = 0x0014;
pub const ATTR_NONCE: u16 = 0x0015;
pub const ATTR_XOR_MAPPED_ADDRESS: u16 = 0x0020;
pub const ATTR_XOR_RELAYED_ADDRESS: u16 = 0x0016;
pub const ATTR_XOR_PEER_ADDRESS: u16 = 0x0012;
pub const ATTR_DATA: u16 = 0x0013;
pub const ATTR_CHANNEL_NUMBER: u16 = 0x000C;
pub const ATTR_LIFETIME: u16 = 0x000D;
pub const ATTR_REQUESTED_TRANSPORT: u16 = 0x0019;
pub const ATTR_EVEN_PORT: u16 = 0x0018;
pub const ATTR_FINGERPRINT: u16 = 0x8028;
pub const ATTR_SOFTWARE: u16 = 0x8022;
pub const ATTR_PADDING: u16 = 0x0026;
pub const ATTR_DONT_FRAGMENT: u16 = 0x001A;
pub const ATTR_RESERVATION_TOKEN: u16 = 0x0022;

/// STUN message type from a 12-bit method and 2-bit class, RFC 5389 §6:
/// bits hold M3-0, C0, M6-4, C1, M11-7 (top two bits stay zero).
pub fn msg_type(method: u16, class: u16) -> u16 {
    let method = method & 0x0FFF;
    let class = class & 0x03;
    (method & 0x000F)
        | (((method >> 4) & 0x0007) << 5)
        | (((method >> 7) & 0x001F) << 9)
        | ((class & 0x01) << 4)
        | (((class >> 1) & 0x01) << 8)
}

fn method_of(msg_type: u16) -> u16 {
    (msg_type & 0x000F) | (((msg_type >> 5) & 0x0007) << 4) | (((msg_type >> 9) & 0x001F) << 7)
}

fn class_of(msg_type: u16) -> u16 {
    (((msg_type >> 8) & 0x0001) << 1) | ((msg_type >> 4) & 0x0001)
}

/// One parsed attribute: type plus raw value (padding already skipped).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Attr {
    attr_type: u16,
    value: Vec<u8>,
}

fn parse_attrs(bytes: &[u8]) -> Option<Vec<Attr>> {
    let mut attrs = Vec::new();
    let mut offset = 0_usize;
    while offset < bytes.len() {
        if offset + 4 > bytes.len() {
            return None;
        }
        let attr_type = u16::from_be_bytes([bytes[offset], bytes[offset + 1]]);
        let length = u16::from_be_bytes([bytes[offset + 2], bytes[offset + 3]]) as usize;
        let padded = length.div_ceil(4) * 4;
        if offset + 4 + length > bytes.len() {
            return None;
        }
        attrs.push(Attr {
            attr_type,
            value: bytes[offset + 4..offset + 4 + length].to_vec(),
        });
        offset += 4 + padded;
    }
    Some(attrs)
}

fn find_attr(attrs: &[Attr], attr_type: u16) -> Vec<&[u8]> {
    attrs
        .iter()
        .filter(|attr| attr.attr_type == attr_type)
        .map(|attr| attr.value.as_slice())
        .collect()
}

fn fingerprint_of(message: &[u8]) -> u32 {
    let mut hasher = Crc32::new();
    hasher.update(message);
    hasher.finalize() ^ TURN_FINGERPRINT_XOR
}

/// Long-term credential key: MD5(username ":" realm ":" password), exactly
/// as RFC 5389 §10.2.2 and Pion derive it. Our minted passwords are ASCII
/// hex, so no SASLprep edge cases apply.
pub fn long_term_key(username: &str, realm: &str, password: &str) -> [u8; 16] {
    let mut input = Vec::with_capacity(username.len() + realm.len() + password.len() + 2);
    input.extend_from_slice(username.as_bytes());
    input.push(b':');
    input.extend_from_slice(realm.as_bytes());
    input.push(b':');
    input.extend_from_slice(password.as_bytes());
    md5::compute(&input).0
}

fn hmac_sha1(key: &[u8], message: &[u8]) -> [u8; 20] {
    let mut mac = Hmac::<Sha1>::new_from_slice(key).expect("hmac takes any key size");
    mac.update(message);
    mac.finalize().into_bytes().into()
}

/// Constant-time compare: online MI guessing is network-bound anyway, but
/// there is no reason to leak even that.
fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right.iter())
        .fold(0_u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// Builds a full STUN message: header + attrs, optional MESSAGE-INTEGRITY
/// (keyed) and FINGERPRINT. MI covers the header (length adjusted to the MI
/// end, fingerprint excluded) through the preceding attribute — the MI
/// header itself is NOT covered, per RFC 5389 §14.5. Fingerprint covers
/// everything before it, with the final length.
fn build_message(
    msg_type: u16,
    transaction: &[u8; 12],
    attrs: &[(u16, Vec<u8>)],
    integrity_key: Option<&[u8]>,
    fingerprint: bool,
) -> Vec<u8> {
    let mut body: Vec<u8> = Vec::new();
    for (attr_type, value) in attrs {
        body.extend_from_slice(&attr_type.to_be_bytes());
        body.extend_from_slice(&(value.len() as u16).to_be_bytes());
        body.extend_from_slice(value);
        body.extend(std::iter::repeat_n(
            0_u8,
            value.len().next_multiple_of(4) - value.len(),
        ));
    }
    let mut message = Vec::with_capacity(20 + body.len() + 32);
    message.extend_from_slice(&msg_type.to_be_bytes());
    let length_position = message.len();
    message.extend_from_slice(&0_u16.to_be_bytes());
    message.extend_from_slice(&STUN_MAGIC_COOKIE.to_be_bytes());
    message.extend_from_slice(transaction);
    message.extend_from_slice(&body);
    if integrity_key.is_none() && !fingerprint {
        let total = body.len() as u16;
        message[length_position..length_position + 2].copy_from_slice(&total.to_be_bytes());
    }
    if integrity_key.is_some() {
        let adjusted = (message.len() - 20 + 4 + 20) as u16;
        message[length_position..length_position + 2].copy_from_slice(&adjusted.to_be_bytes());
    }
    if let Some(key) = integrity_key {
        let tag = hmac_sha1(key, &message);
        message.extend_from_slice(&ATTR_MESSAGE_INTEGRITY.to_be_bytes());
        message.extend_from_slice(&20_u16.to_be_bytes());
        message.extend_from_slice(&tag);
    }
    if fingerprint {
        let total = (message.len() - 20 + 4 + 4) as u16;
        message[length_position..length_position + 2].copy_from_slice(&total.to_be_bytes());
        let crc = fingerprint_of(&message);
        message.extend_from_slice(&ATTR_FINGERPRINT.to_be_bytes());
        message.extend_from_slice(&4_u16.to_be_bytes());
        message.extend_from_slice(&crc.to_be_bytes());
    }
    message
}

/// Verifies MESSAGE-INTEGRITY against `key`: recomputes HMAC-SHA1 over the
/// header (length adjusted to the MI end) through the MI header. MI must be
/// last or second-to-last (fingerprint only after it); anything else fails
/// closed. `raw` must be the exact datagram (callers enforce length first).
fn verify_mi(raw: &[u8], attrs: &[Attr], key: &[u8]) -> bool {
    let Some(position) = attrs
        .iter()
        .position(|a| a.attr_type == ATTR_MESSAGE_INTEGRITY)
    else {
        return false;
    };
    if attrs[position].value.len() != 20 {
        return false;
    }
    if attrs[position + 1..]
        .iter()
        .any(|a| a.attr_type != ATTR_FINGERPRINT)
    {
        return false;
    }
    // Offset of the MI attribute: header plus preceding attrs with padding.
    // The HMAC input ends where the MI header starts (header length still
    // adjusted to the MI end, per RFC 5389 §14.5).
    let mut offset = 20_usize;
    for attr in &attrs[..position] {
        offset += 4 + attr.value.len().next_multiple_of(4);
    }
    let mi_end = offset + 4 + 20;
    if mi_end > raw.len() {
        return false;
    }
    let adjusted = (mi_end - 20) as u16;
    let mut input = Vec::with_capacity(offset);
    input.extend_from_slice(&raw[..2]);
    input.extend_from_slice(&adjusted.to_be_bytes());
    input.extend_from_slice(&raw[4..offset]);
    constant_time_eq(&hmac_sha1(key, &input), &raw[mi_end - 20..mi_end])
}

fn error_response(
    method: u16,
    transaction: &[u8; 12],
    code: u16,
    reason: &str,
    extra: &[(u16, Vec<u8>)],
    integrity_key: Option<&[u8]>,
) -> Vec<u8> {
    let class = (code / 100) as u8;
    let number = (code % 100) as u8;
    let mut value = vec![0x00, 0x00, class, number];
    value.extend_from_slice(reason.as_bytes());
    let mut attrs = vec![(ATTR_ERROR_CODE, value)];
    attrs.extend(extra.iter().cloned());
    build_message(
        msg_type(method, CLASS_ERROR),
        transaction,
        &attrs,
        integrity_key,
        true,
    )
}

fn realm_attr() -> (u16, Vec<u8>) {
    (ATTR_REALM, TURN_REALM.as_bytes().to_vec())
}

fn nonce_attr(nonce: &str) -> (u16, Vec<u8>) {
    (ATTR_NONCE, nonce.as_bytes().to_vec())
}

#[derive(Debug, Clone)]
struct Channel {
    peer: SocketAddr,
    expires_at: u64,
}

#[derive(Debug)]
struct Allocation {
    id: Uuid,
    client: Uuid,
    client_addr: SocketAddr,
    permissions: HashMap<IpAddr, u64>,
    channels: HashMap<u16, Channel>,
    channels_by_peer: HashMap<SocketAddr, u16>,
    expires_at: u64,
    last_active: u64,
    /// Dedicated relay socket in external mode (None in compat mode: the
    /// shared front socket is the relayed address). Expiry frees the port.
    socket: Option<Arc<UdpSocket>>,
    /// Address advertised in XOR-RELAYED-ADDRESS. External mode: public IP
    /// + the dedicated socket's local port; compat: the front socket.
    relayed: Option<SocketAddr>,
    /// Byte budget window (both directions share it).
    budget_second: u64,
    budget_bytes: u64,
}

/// External-mode configuration: the public address clients must learn and
/// the bounded UDP port range for per-allocation relay sockets. Set only on
/// hosts behind a 1:1 NAT (OCI); unset preserves shared-socket K11 behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnRelayConfig {
    pub external_ip: IpAddr,
    pub port_low: u16,
    pub port_high: u16,
}

impl TurnRelayConfig {
    pub fn new(external_ip: IpAddr, port_low: u16, port_high: u16) -> Option<Self> {
        if port_low == 0 || port_low > port_high {
            return None;
        }
        Some(Self {
            external_ip,
            port_low,
            port_high,
        })
    }

    /// Binds one wildcard socket of the external IP's family on the first
    /// free port in the configured range.
    fn bind_socket(&self) -> Option<(Arc<UdpSocket>, SocketAddr)> {
        let wildcard: SocketAddr = if self.external_ip.is_ipv4() {
            (IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0).into()
        } else {
            (IpAddr::V6(Ipv6Addr::UNSPECIFIED), 0).into()
        };
        for port in self.port_low..=self.port_high {
            if let Ok(socket) = UdpSocket::bind(SocketAddr::new(wildcard.ip(), port)) {
                let advertised = SocketAddr::new(self.external_ip, port);
                return Some((Arc::new(socket), advertised));
            }
        }
        None
    }
}

/// Peer-scope policy for relay destinations. In external mode the VM sits
/// on a public service network: loopback/link-local/multicast/RFC1918/ULA
/// peers are never legitimate (a client in error would relay into the
/// host's own network); CGNAT's 100.64/10 IS legitimate. Compat mode (no
/// external IP) keeps every address legal so LAN and loopback tests work.
fn peer_allowed(ip: &IpAddr, external: bool) -> bool {
    if !external {
        return true;
    }
    match ip {
        IpAddr::V4(v4) => {
            let [a, b, _, _] = v4.octets();
            !(a == 0
                || a == 127
                || (a == 169 && b == 254)
                || a >= 224
                || a == 10
                || (a == 172 && (16..=31).contains(&b))
                || (a == 192 && b == 168))
        }
        IpAddr::V6(v6) => {
            // IPv4-mapped IPv6 (::ffff:a.b.c.d) must face the v4 policy:
            // otherwise a private range could re-enter through v6 encoding
            // on a dual-stack host.
            if let Some(mapped) = v6.to_ipv4_mapped() {
                return peer_allowed(&IpAddr::V4(mapped), true);
            }
            let segments = v6.segments();
            !(v6.is_loopback()
                || v6.is_multicast()
                || v6.is_unspecified()
                || (segments[0] & 0xfe00) == 0xfc00
                || (segments[0] & 0xffc0) == 0xfe80)
        }
    }
}

/// One-second sliding budget per allocation, both directions; refusal is a
/// silent drop (UDP relays never back-pressure a firehose).
fn budget_admit(alloc: &mut Allocation, now: u64, bytes: u64) -> bool {
    if alloc.budget_second != now {
        alloc.budget_second = now;
        alloc.budget_bytes = 0;
    }
    if alloc.budget_bytes.saturating_add(bytes) > TURN_RELAY_BUDGET_BYTES_PER_SEC {
        return false;
    }
    alloc.budget_bytes += bytes;
    true
}

#[derive(Debug, Clone)]
struct Credential {
    password: String,
    expires_at: u64,
}

/// What one inbound datagram asks the serve loop to do.
#[derive(Debug)]
pub enum TurnOutcome {
    /// A STUN reply for the sender.
    Reply(Vec<u8>),
    /// Raw peer bytes to forward elsewhere (relayed traffic). `via` carries
    /// the allocation's own socket in external mode (the peer sees the
    /// relayed 5-tuple); None sends from the shared front socket (compat).
    Forward {
        to: SocketAddr,
        bytes: Vec<u8>,
        via: Option<Arc<UdpSocket>>,
    },
    /// Indication consumed, garbage, or misaddressed: nothing to send.
    Quiet,
}

/// TURN relay state: credentials, nonces, and allocations. All methods take
/// the caller-verified facts as parameters where relevant; authentication
/// itself happens here against the credential store.
#[derive(Debug, Default)]
pub struct TurnState {
    credentials: HashMap<String, Credential>,
    nonces: VecDeque<(String, u64)>,
    allocations: HashMap<Uuid, Allocation>,
    by_client: HashMap<Uuid, Uuid>,
    /// External (public-NAT) relay configuration: dedicated per-allocation
    /// sockets. `None` selects compat mode (shared front socket; the K11
    /// direct-address deployment and every unit test).
    relay: Option<TurnRelayConfig>,
    /// Allocations that just gained a dedicated socket; the socket-owning
    /// loop drains these to spawn their peer-facing reader threads.
    socket_ready: Vec<(Uuid, SocketAddr)>,
}

impl TurnState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enables external mode: allocations get dedicated sockets from the
    /// configured port range and advertise `config.external_ip`.
    pub fn set_external_relay(&mut self, config: TurnRelayConfig) {
        self.relay = Some(config);
    }

    pub fn external_mode(&self) -> bool {
        self.relay.is_some()
    }

    /// Allocations waiting for their peer-reader thread: `(id, client_addr)`.
    pub fn drain_socket_ready(&mut self) -> Vec<(Uuid, SocketAddr)> {
        std::mem::take(&mut self.socket_ready)
    }

    /// The dedicated relay socket of one allocation (external mode only).
    pub fn allocation_socket(&self, id: &Uuid) -> Option<Arc<UdpSocket>> {
        self.allocations.get(id).and_then(|a| a.socket.clone())
    }

    /// Current client address of one allocation (reader threads re-resolve
    /// per datagram so NAT rebinding is honored without extra plumbing).
    pub fn allocation_client(&self, id: &Uuid) -> Option<SocketAddr> {
        self.allocations.get(id).map(|a| a.client_addr)
    }

    /// Advertised relayed address of one allocation (external mode: public
    /// IP + dedicated port; compat: None, front socket is the address).
    pub fn allocation_relayed(&self, id: &Uuid) -> Option<SocketAddr> {
        self.allocations.get(id).and_then(|a| a.relayed)
    }

    /// Whether an allocation still exists and is unexpired; relay reader
    /// threads poll this to exit after expiry or refresh-0 deletion.
    pub fn allocation_active(&mut self, id: &Uuid, now: u64) -> bool {
        self.expire(now);
        self.allocations
            .get(id)
            .is_some_and(|alloc| alloc.expires_at > now)
    }

    /// One raw peer datagram that arrived on an allocation's dedicated
    /// socket (external mode): enforce destination policy and budget, then
    /// wrap it for the client — ChannelData when a channel is bound to that
    /// source, else a Data indication when a permission exists, else drop.
    /// Peer STUN (ICE Binding) is relayed like any other payload: on a
    /// dedicated socket nothing else can answer on the peer's behalf.
    ///
    /// Sources from our own relay sockets (server-forwarded hairpin: client
    /// A behind this same relay talking to client B behind it) skip the
    /// destination policy — the guest address they carry is ours by
    /// construction — but still need B's permission/channel and budget like
    /// any other arrival.
    pub fn relay_receipt(
        &mut self,
        alloc_id: &Uuid,
        source: SocketAddr,
        bytes: &[u8],
        now: u64,
    ) -> Option<Vec<u8>> {
        self.relay?;
        self.expire(now);
        if !self.is_own_relay_source(source) && !peer_allowed(&source.ip(), true) {
            return None;
        }
        let transaction: [u8; 12] = {
            let full = Uuid::new_v4().into_bytes();
            full[..12].try_into().unwrap_or([0_u8; 12])
        };
        let alloc = self.allocations.get_mut(alloc_id)?;
        if alloc.expires_at <= now {
            return None;
        }
        if !budget_admit(alloc, now, bytes.len() as u64) {
            return None;
        }
        alloc.last_active = now;
        if let Some(number) = alloc.channels_by_peer.get(&source) {
            let mut out = Vec::with_capacity(4 + bytes.len());
            out.extend_from_slice(&number.to_be_bytes());
            out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
            out.extend_from_slice(bytes);
            return Some(out);
        }
        if let Some(expires) = alloc.permissions.get(&source.ip()) {
            if *expires > now {
                return Some(build_message(
                    msg_type(METHOD_DATA, CLASS_INDICATION),
                    &transaction,
                    &[
                        (
                            ATTR_XOR_PEER_ADDRESS,
                            harbor_protocol::stun::xor_addr_value(source, &transaction),
                        ),
                        (ATTR_DATA, bytes.to_vec()),
                    ],
                    None,
                    false,
                ));
            }
        }
        None
    }

    /// Whether `source` is one of our own live relay sockets (same host,
    /// port currently owned by an allocation). Matched by port against the
    /// advertised relayed addresses — no syscalls, no concrete-IP guessing
    /// behind the 1:1 NAT. A spoofed in-range port still needs the
    /// receiving allocation's permission/channel, exactly like the baseline
    /// IP-spoofing exposure inherent to UDP + IP-scoped permissions.
    fn is_own_relay_source(&self, source: SocketAddr) -> bool {
        self.allocations
            .values()
            .any(|alloc| alloc.relayed.is_some_and(|r| r.port() == source.port()))
    }

    /// Mints (or rotates) a credential for an authenticated device. The
    /// username IS the device id string, binding every allocation to it;
    /// the password is 128 fresh bits, hexed.
    pub fn mint_credential(&mut self, device: Uuid, now: u64) -> (String, String, u64) {
        let username = device.to_string();
        let password = Uuid::new_v4().simple().to_string();
        let expires_at = now.saturating_add(TURN_CRED_TTL_SECS);
        self.credentials.insert(
            username.clone(),
            Credential {
                password: password.clone(),
                expires_at,
            },
        );
        (username, password, expires_at)
    }

    /// Test seam: proves the control-plane endpoint and the UDP loop share
    /// one credential store (same `TurnState`, no second provisioning step).
    #[cfg(test)]
    pub fn credential_password_for_tests(&self, username: &str) -> Option<String> {
        self.credentials
            .get(username)
            .map(|cred| cred.password.clone())
    }

    fn fresh_nonce(&mut self, now: u64) -> String {
        while self.nonces.len() >= MAX_NONCES {
            self.nonces.pop_front();
        }
        let nonce = Uuid::new_v4().simple().to_string();
        self.nonces
            .push_back((nonce.clone(), now.saturating_add(TURN_NONCE_TTL_SECS)));
        nonce
    }

    fn use_nonce(&mut self, nonce: &str, now: u64) -> bool {
        self.nonces
            .iter()
            .any(|(known, expires)| known == nonce && *expires > now)
    }

    /// Sweeps time-based state: expired credentials, nonces, allocations,
    /// permissions, and channels. Cheap (a handful of entries); callers run
    /// it on every TURN message.
    pub fn expire(&mut self, now: u64) {
        self.credentials.retain(|_, cred| cred.expires_at > now);
        self.nonces.retain(|(_, expires)| *expires > now);
        let dead: Vec<Uuid> = self
            .allocations
            .values()
            .filter(|alloc| alloc.expires_at <= now)
            .map(|alloc| alloc.id)
            .collect();
        for id in dead {
            self.remove_allocation(&id);
        }
        for alloc in self.allocations.values_mut() {
            alloc.permissions.retain(|_, expires| *expires > now);
            alloc.channels.retain(|_, channel| channel.expires_at > now);
            alloc
                .channels_by_peer
                .retain(|_, number| alloc.channels.contains_key(number));
        }
    }

    fn remove_allocation(&mut self, id: &Uuid) {
        if let Some(alloc) = self.allocations.remove(id) {
            if self.by_client.get(&alloc.client) == Some(id) {
                self.by_client.remove(&alloc.client);
            }
        }
    }

    /// Routes one inbound datagram: STUN requests get replies, indications
    /// and channel data get forwarded or dropped, garbage stays silent.
    /// `relay_addr` is our own socket address (relayed address + context).
    pub fn handle_datagram(
        &mut self,
        bytes: &[u8],
        source: SocketAddr,
        relay_addr: SocketAddr,
        now: u64,
    ) -> TurnOutcome {
        self.expire(now);
        if bytes.len() < 4 {
            return TurnOutcome::Quiet;
        }
        // Compat mode only: known peers route BEFORE the STUN/ChannelData
        // demux: peer media is opaque bytes, not STUN — e.g. `b"voice.."`
        // (0x76…) would misparse as ChannelData. Genuine STUN from a peer
        // address still falls through to normal dispatch (a host may be peer
        // and client at once). External mode never relays on the front door:
        // peers talk to the allocation's dedicated socket (`relay_receipt`).
        if self.relay.is_none() {
            if let Some(outcome) = self.relay_from_peer(bytes, source) {
                return outcome;
            }
        }
        match bytes[0] & 0xC0 {
            0x00 => self.handle_stun(bytes, source, relay_addr, now),
            0x40 => self.handle_channel_data(bytes, source, now),
            _ => TurnOutcome::Quiet,
        }
    }

    /// Relays one raw peer datagram toward its allocation client: ChannelData
    /// when the peer holds a channel binding, a Data indication when it holds
    /// a permission, `None` when the source is no peer of ours (or speaks
    /// genuine STUN, which the normal dispatch must see).
    fn relay_from_peer(&self, bytes: &[u8], source: SocketAddr) -> Option<TurnOutcome> {
        if bytes[0] & 0xC0 == 0x00
            && bytes.len() >= 20
            && magic_of(bytes) == STUN_MAGIC_COOKIE
            && bytes.len() == 20 + u16::from_be_bytes([bytes[2], bytes[3]]) as usize
        {
            return None;
        }
        for alloc in self.allocations.values() {
            if let Some(number) = alloc.channels_by_peer.get(&source) {
                let mut out = Vec::with_capacity(4 + bytes.len());
                out.extend_from_slice(&number.to_be_bytes());
                out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
                out.extend_from_slice(bytes);
                return Some(TurnOutcome::Forward {
                    to: alloc.client_addr,
                    bytes: out,
                    via: None,
                });
            }
            if alloc.permissions.contains_key(&source.ip()) {
                let full = Uuid::new_v4().into_bytes();
                let txid: [u8; 12] = full[..12].try_into().unwrap_or([0_u8; 12]);
                let indication = build_message(
                    msg_type(METHOD_DATA, CLASS_INDICATION),
                    &txid,
                    &[
                        (
                            ATTR_XOR_PEER_ADDRESS,
                            harbor_protocol::stun::xor_addr_value(source, &txid),
                        ),
                        (ATTR_DATA, bytes.to_vec()),
                    ],
                    None,
                    false,
                );
                return Some(TurnOutcome::Forward {
                    to: alloc.client_addr,
                    bytes: indication,
                    via: None,
                });
            }
        }
        None
    }

    fn handle_stun(
        &mut self,
        bytes: &[u8],
        source: SocketAddr,
        relay_addr: SocketAddr,
        now: u64,
    ) -> TurnOutcome {
        if bytes.len() < 20 || magic_of(bytes) != STUN_MAGIC_COOKIE {
            return TurnOutcome::Quiet;
        }
        let msg_type = u16::from_be_bytes([bytes[0], bytes[1]]);
        let announced = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
        if bytes.len() != 20 + announced {
            return TurnOutcome::Quiet;
        }
        let transaction: &[u8; 12] = match bytes[8..20].try_into() {
            Ok(txid) => txid,
            Err(_) => return TurnOutcome::Quiet,
        };
        let Some(attrs) = parse_attrs(&bytes[20..]) else {
            return TurnOutcome::Quiet;
        };
        if let Some(fp) = find_attr(&attrs, ATTR_FINGERPRINT).first() {
            if fp.len() != 4 {
                return TurnOutcome::Quiet;
            }
            let at = bytes.len() - 8;
            let computed = fingerprint_of(&bytes[..at]);
            let transmitted = u32::from_be_bytes([fp[0], fp[1], fp[2], fp[3]]);
            if computed != transmitted {
                return TurnOutcome::Quiet;
            }
        }
        match (method_of(msg_type), class_of(msg_type)) {
            (METHOD_ALLOCATE, CLASS_REQUEST) => TurnOutcome::Reply(self.serve_allocate(
                bytes,
                &attrs,
                transaction,
                source,
                relay_addr,
                now,
            )),
            (METHOD_REFRESH, CLASS_REQUEST) => {
                TurnOutcome::Reply(self.serve_refresh(bytes, &attrs, transaction, source, now))
            }
            (METHOD_CREATE_PERMISSION, CLASS_REQUEST) => TurnOutcome::Reply(self.serve_permission(
                bytes,
                &attrs,
                transaction,
                source,
                relay_addr,
                now,
            )),
            (METHOD_CHANNEL_BIND, CLASS_REQUEST) => TurnOutcome::Reply(self.serve_channel_bind(
                bytes,
                &attrs,
                transaction,
                source,
                relay_addr,
                now,
            )),
            (METHOD_SEND, CLASS_INDICATION) => {
                match self.serve_send(&attrs, transaction, source, now) {
                    Some((peer, data, via)) => TurnOutcome::Forward {
                        to: peer,
                        bytes: data,
                        via,
                    },
                    None => TurnOutcome::Quiet,
                }
            }
            // Binding requests belong to the STUN-lite loop (checked first
            // there); Data indications never legitimately arrive here (they
            // travel server-to-client only). Named explicitly so the method
            // table documents the full dispatch.
            (METHOD_BINDING, _) | (METHOD_DATA, _) => TurnOutcome::Quiet,
            _ => TurnOutcome::Quiet,
        }
    }

    /// Shared request gate: unknown comprehension-required attrs, then
    /// long-term auth. Returns the verified device plus the MI key on
    /// success, or a complete error reply otherwise.
    fn gate(
        &mut self,
        method: u16,
        raw: &[u8],
        attrs: &[Attr],
        transaction: &[u8; 12],
        now: u64,
    ) -> Result<(Uuid, Vec<u8>), Vec<u8>> {
        let mut unknown = Vec::new();
        for attr in attrs {
            if attr.attr_type < 0x8000
                && !matches!(
                    attr.attr_type,
                    ATTR_USERNAME
                        | ATTR_MESSAGE_INTEGRITY
                        | ATTR_REALM
                        | ATTR_NONCE
                        | ATTR_REQUESTED_TRANSPORT
                        | ATTR_LIFETIME
                        | ATTR_XOR_PEER_ADDRESS
                        | ATTR_DATA
                        | ATTR_CHANNEL_NUMBER
                        | ATTR_EVEN_PORT
                        | ATTR_RESERVATION_TOKEN
                        | ATTR_DONT_FRAGMENT
                        | ATTR_FINGERPRINT
                        | ATTR_SOFTWARE
                        | ATTR_PADDING
                )
            {
                unknown.extend_from_slice(&attr.attr_type.to_be_bytes());
            }
        }
        if !unknown.is_empty() {
            return Err(error_response(
                method,
                transaction,
                420,
                "Unknown Attributes",
                &[(ATTR_UNKNOWN_ATTRIBUTES, unknown)],
                None,
            ));
        }
        if !find_attr(attrs, ATTR_RESERVATION_TOKEN).is_empty() {
            return Err(error_response(
                method,
                transaction,
                400,
                "Bad Request",
                &[],
                None,
            ));
        }
        let username = find_attr(attrs, ATTR_USERNAME)
            .first()
            .and_then(|raw| std::str::from_utf8(raw).ok())
            .map(str::to_owned);
        let realm = find_attr(attrs, ATTR_REALM)
            .first()
            .and_then(|raw| std::str::from_utf8(raw).ok());
        let nonce = find_attr(attrs, ATTR_NONCE)
            .first()
            .and_then(|raw| std::str::from_utf8(raw).ok());
        let (Some(username), Some(realm), Some(nonce)) = (username, realm, nonce) else {
            return Err(self.challenge(method, transaction, now));
        };
        if realm != TURN_REALM {
            return Err(self.challenge(method, transaction, now));
        }
        // Unknown or expired credential answers 401 BEFORE the nonce is even
        // examined: without a key the request is unverifiable, so staleness
        // is meaningless (and must not leak which nonces are live).
        let Some(cred) = self.credentials.get(&username) else {
            return Err(self.challenge(method, transaction, now));
        };
        if cred.expires_at <= now {
            return Err(self.challenge(method, transaction, now));
        }
        let key = long_term_key(&username, TURN_REALM, &cred.password);
        // MI mismatch here means wrong password or a forgery that survived
        // the fingerprint (blind tampering already died above). We re-issue
        // the 401 challenge instead of staying silent so a client with a
        // rotated password recovers on the next Allocate; deliberate abuse
        // still pays full price (fresh nonce, full checks, quotas).
        if !verify_mi(raw, attrs, &key) {
            return Err(self.challenge(method, transaction, now));
        }
        // Only a request whose integrity VERIFIED can be judged stale: 438.
        if !self.use_nonce(nonce, now) {
            return Err(self.stale(method, transaction, now));
        }
        let Ok(device) = username.parse::<Uuid>() else {
            return Err(self.challenge(method, transaction, now));
        };
        Ok((device, key.to_vec()))
    }

    fn challenge(&mut self, method: u16, transaction: &[u8; 12], now: u64) -> Vec<u8> {
        let nonce = self.fresh_nonce(now);
        error_response(
            method,
            transaction,
            401,
            "Unauthorized",
            &[realm_attr(), nonce_attr(&nonce)],
            None,
        )
    }

    fn stale(&mut self, method: u16, transaction: &[u8; 12], now: u64) -> Vec<u8> {
        let nonce = self.fresh_nonce(now);
        error_response(
            method,
            transaction,
            438,
            "Stale Nonce",
            &[realm_attr(), nonce_attr(&nonce)],
            None,
        )
    }

    fn serve_allocate(
        &mut self,
        raw: &[u8],
        attrs: &[Attr],
        transaction: &[u8; 12],
        source: SocketAddr,
        relay_addr: SocketAddr,
        now: u64,
    ) -> Vec<u8> {
        let authed = match self.gate(METHOD_ALLOCATE, raw, attrs, transaction, now) {
            Ok(verified) => verified,
            Err(reply) => return reply,
        };
        let (device, key) = authed;
        // Requested transport must be UDP; EvenPort reservations unsupported.
        match find_attr(attrs, ATTR_REQUESTED_TRANSPORT).first() {
            Some(transport) if transport.len() == 4 && transport[0] == 17 => {}
            _ => {
                return error_response(
                    METHOD_ALLOCATE,
                    transaction,
                    442,
                    "Unsupported Transport Protocol",
                    &[],
                    Some(&key),
                );
            }
        }
        if let Some(even) = find_attr(attrs, ATTR_EVEN_PORT).first() {
            if even.first().is_some_and(|flags| flags & 0x80 != 0) {
                return error_response(
                    METHOD_ALLOCATE,
                    transaction,
                    400,
                    "Bad Request",
                    &[],
                    Some(&key),
                );
            }
        }
        let lifetime = match find_attr(attrs, ATTR_LIFETIME).first() {
            None => TURN_DEFAULT_LIFETIME_SECS,
            Some(raw) if raw.len() == 4 => {
                let asked = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as u64;
                if asked == 0 {
                    return error_response(
                        METHOD_ALLOCATE,
                        transaction,
                        400,
                        "Bad Request",
                        &[],
                        Some(&key),
                    );
                }
                asked.clamp(TURN_MIN_LIFETIME_SECS, TURN_MAX_LIFETIME_SECS)
            }
            Some(_) => {
                return error_response(
                    METHOD_ALLOCATE,
                    transaction,
                    400,
                    "Bad Request",
                    &[],
                    Some(&key),
                );
            }
        };
        if self.allocations.len() >= MAX_TURN_ALLOCATIONS && !self.by_client.contains_key(&device) {
            return error_response(
                METHOD_ALLOCATE,
                transaction,
                486,
                "Allocation Quota Reached",
                &[],
                Some(&key),
            );
        }
        if let Some(old) = self.by_client.get(&device).copied() {
            self.remove_allocation(&old);
        }
        let id = Uuid::new_v4();
        // External mode (Oracle behind 1:1 NAT): each allocation owns a
        // dedicated UDP socket from the configured range; the client learns
        // the public IP + that socket's port. Compat mode (None): the shared
        // front socket stays the relayed address (K11 + every unit test).
        if let Some(cfg) = self.relay {
            let Some((socket, advertised)) = cfg.bind_socket() else {
                return error_response(
                    METHOD_ALLOCATE,
                    transaction,
                    486,
                    "Allocation Quota Reached",
                    &[],
                    Some(&key),
                );
            };
            self.allocations.insert(
                id,
                Allocation {
                    id,
                    client: device,
                    client_addr: source,
                    permissions: HashMap::new(),
                    channels: HashMap::new(),
                    channels_by_peer: HashMap::new(),
                    expires_at: now.saturating_add(lifetime),
                    last_active: now,
                    socket: Some(socket),
                    relayed: Some(advertised),
                    budget_second: now,
                    budget_bytes: 0,
                },
            );
            self.by_client.insert(device, id);
            self.socket_ready.push((id, source));
            return build_message(
                msg_type(METHOD_ALLOCATE, CLASS_SUCCESS),
                transaction,
                &[
                    (
                        ATTR_XOR_RELAYED_ADDRESS,
                        harbor_protocol::stun::xor_addr_value(advertised, transaction),
                    ),
                    (ATTR_LIFETIME, (lifetime as u32).to_be_bytes().to_vec()),
                    (
                        ATTR_XOR_MAPPED_ADDRESS,
                        harbor_protocol::stun::xor_addr_value(source, transaction),
                    ),
                ],
                Some(&key),
                true,
            );
        }
        self.allocations.insert(
            id,
            Allocation {
                id,
                client: device,
                client_addr: source,
                permissions: HashMap::new(),
                channels: HashMap::new(),
                channels_by_peer: HashMap::new(),
                expires_at: now.saturating_add(lifetime),
                last_active: now,
                socket: None,
                relayed: None,
                budget_second: now,
                budget_bytes: 0,
            },
        );
        self.by_client.insert(device, id);
        build_message(
            msg_type(METHOD_ALLOCATE, CLASS_SUCCESS),
            transaction,
            &[
                (
                    ATTR_XOR_RELAYED_ADDRESS,
                    harbor_protocol::stun::xor_addr_value(relay_addr, transaction),
                ),
                (ATTR_LIFETIME, (lifetime as u32).to_be_bytes().to_vec()),
                (
                    ATTR_XOR_MAPPED_ADDRESS,
                    harbor_protocol::stun::xor_addr_value(source, transaction),
                ),
            ],
            Some(&key),
            true,
        )
    }

    fn serve_refresh(
        &mut self,
        raw: &[u8],
        attrs: &[Attr],
        transaction: &[u8; 12],
        source: SocketAddr,
        now: u64,
    ) -> Vec<u8> {
        let authed = match self.gate(METHOD_REFRESH, raw, attrs, transaction, now) {
            Ok(verified) => verified,
            Err(reply) => return reply,
        };
        let (device, key) = authed;
        let Some(id) = self.by_client.get(&device).copied() else {
            return error_response(
                METHOD_REFRESH,
                transaction,
                437,
                "Allocation Mismatch",
                &[],
                Some(&key),
            );
        };
        let lifetime = match find_attr(attrs, ATTR_LIFETIME).first() {
            None => TURN_DEFAULT_LIFETIME_SECS,
            Some(raw) if raw.len() == 4 => {
                u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as u64
            }
            Some(_) => {
                return error_response(
                    METHOD_REFRESH,
                    transaction,
                    400,
                    "Bad Request",
                    &[],
                    Some(&key),
                );
            }
        };
        if lifetime == 0 {
            self.remove_allocation(&id);
            return build_message(
                msg_type(METHOD_REFRESH, CLASS_SUCCESS),
                transaction,
                &[(ATTR_LIFETIME, 0_u32.to_be_bytes().to_vec())],
                Some(&key),
                true,
            );
        }
        let granted = lifetime.clamp(TURN_MIN_LIFETIME_SECS, TURN_MAX_LIFETIME_SECS);
        match self.allocations.get_mut(&id) {
            Some(alloc) => {
                alloc.expires_at = now.saturating_add(granted);
                alloc.last_active = now;
                alloc.client_addr = source;
            }
            None => {
                return error_response(
                    METHOD_REFRESH,
                    transaction,
                    437,
                    "Allocation Mismatch",
                    &[],
                    Some(&key),
                );
            }
        }
        build_message(
            msg_type(METHOD_REFRESH, CLASS_SUCCESS),
            transaction,
            &[(ATTR_LIFETIME, (granted as u32).to_be_bytes().to_vec())],
            Some(&key),
            true,
        )
    }

    fn serve_permission(
        &mut self,
        raw: &[u8],
        attrs: &[Attr],
        transaction: &[u8; 12],
        source: SocketAddr,
        relay_addr: SocketAddr,
        now: u64,
    ) -> Vec<u8> {
        let authed = match self.gate(METHOD_CREATE_PERMISSION, raw, attrs, transaction, now) {
            Ok(verified) => verified,
            Err(reply) => return reply,
        };
        let (device, key) = authed;
        let peers: Vec<SocketAddr> = find_attr(attrs, ATTR_XOR_PEER_ADDRESS)
            .iter()
            .filter_map(|raw| harbor_protocol::stun::xor_addr_decode(raw, transaction))
            .collect();
        if peers.is_empty() {
            return error_response(
                METHOD_CREATE_PERMISSION,
                transaction,
                400,
                "Bad Request",
                &[],
                Some(&key),
            );
        }
        let Some(id) = self.by_client.get(&device).copied() else {
            return error_response(
                METHOD_CREATE_PERMISSION,
                transaction,
                437,
                "Allocation Mismatch",
                &[],
                Some(&key),
            );
        };
        let relay_family_v6 = relay_addr.is_ipv6();
        let external = self.relay.is_some();
        // External mode advertises the public IP family, not the guest's
        // private bind (10.0.0.7): an IPv6 peer against an IPv4 Oracle is a
        // family mismatch, not a relayable destination.
        let expected_v6 = self
            .relay
            .map(|cfg| cfg.external_ip.is_ipv6())
            .unwrap_or(relay_family_v6);
        {
            let Some(alloc) = self.allocations.get_mut(&id) else {
                return error_response(
                    METHOD_CREATE_PERMISSION,
                    transaction,
                    437,
                    "Allocation Mismatch",
                    &[],
                    Some(&key),
                );
            };
            for peer in &peers {
                if peer.is_ipv6() != expected_v6 {
                    return error_response(
                        METHOD_CREATE_PERMISSION,
                        transaction,
                        443,
                        "Peer Address Family Mismatch",
                        &[],
                        Some(&key),
                    );
                }
                if external && !peer_allowed(&peer.ip(), true) {
                    return error_response(
                        METHOD_CREATE_PERMISSION,
                        transaction,
                        403,
                        "Forbidden",
                        &[],
                        Some(&key),
                    );
                }
            }
            if alloc.permissions.len() + peers.len() > MAX_PERMISSIONS_PER_ALLOCATION {
                return error_response(
                    METHOD_CREATE_PERMISSION,
                    transaction,
                    486,
                    "Allocation Quota Reached",
                    &[],
                    Some(&key),
                );
            }
            for peer in peers {
                alloc
                    .permissions
                    .insert(peer.ip(), now.saturating_add(TURN_PERMISSION_LIFETIME_SECS));
                alloc.last_active = now;
            }
            alloc.client_addr = source;
        }
        build_message(
            msg_type(METHOD_CREATE_PERMISSION, CLASS_SUCCESS),
            transaction,
            &[],
            Some(&key),
            true,
        )
    }

    fn serve_channel_bind(
        &mut self,
        raw: &[u8],
        attrs: &[Attr],
        transaction: &[u8; 12],
        source: SocketAddr,
        relay_addr: SocketAddr,
        now: u64,
    ) -> Vec<u8> {
        let authed = match self.gate(METHOD_CHANNEL_BIND, raw, attrs, transaction, now) {
            Ok(verified) => verified,
            Err(reply) => return reply,
        };
        let (device, key) = authed;
        let number = match find_attr(attrs, ATTR_CHANNEL_NUMBER).first() {
            Some(raw) if raw.len() == 4 => u16::from_be_bytes([raw[0], raw[1]]),
            _ => {
                return error_response(
                    METHOD_CHANNEL_BIND,
                    transaction,
                    400,
                    "Bad Request",
                    &[],
                    Some(&key),
                );
            }
        };
        if !(TURN_FIRST_CHANNEL..=TURN_LAST_CHANNEL).contains(&number) {
            return error_response(
                METHOD_CHANNEL_BIND,
                transaction,
                400,
                "Bad Request",
                &[],
                Some(&key),
            );
        }
        let peer = match find_attr(attrs, ATTR_XOR_PEER_ADDRESS)
            .first()
            .and_then(|raw| harbor_protocol::stun::xor_addr_decode(raw, transaction))
        {
            Some(peer) => peer,
            None => {
                return error_response(
                    METHOD_CHANNEL_BIND,
                    transaction,
                    400,
                    "Bad Request",
                    &[],
                    Some(&key),
                );
            }
        };
        if peer.is_ipv6()
            != self
                .relay
                .map(|cfg| cfg.external_ip.is_ipv6())
                .unwrap_or(relay_addr.is_ipv6())
        {
            return error_response(
                METHOD_CHANNEL_BIND,
                transaction,
                443,
                "Peer Address Family Mismatch",
                &[],
                Some(&key),
            );
        }
        if self.relay.is_some() && !peer_allowed(&peer.ip(), true) {
            return error_response(
                METHOD_CHANNEL_BIND,
                transaction,
                403,
                "Forbidden",
                &[],
                Some(&key),
            );
        }
        let Some(id) = self.by_client.get(&device).copied() else {
            return error_response(
                METHOD_CHANNEL_BIND,
                transaction,
                437,
                "Allocation Mismatch",
                &[],
                Some(&key),
            );
        };
        {
            let Some(alloc) = self.allocations.get_mut(&id) else {
                return error_response(
                    METHOD_CHANNEL_BIND,
                    transaction,
                    437,
                    "Allocation Mismatch",
                    &[],
                    Some(&key),
                );
            };
            if alloc.channels.len() >= MAX_CHANNELS_PER_ALLOCATION
                && !alloc.channels.contains_key(&number)
            {
                return error_response(
                    METHOD_CHANNEL_BIND,
                    transaction,
                    486,
                    "Allocation Quota Reached",
                    &[],
                    Some(&key),
                );
            }
            alloc.channels.insert(
                number,
                Channel {
                    peer,
                    expires_at: now.saturating_add(TURN_CHANNEL_LIFETIME_SECS),
                },
            );
            alloc.channels_by_peer.insert(peer, number);
            alloc
                .permissions
                .insert(peer.ip(), now.saturating_add(TURN_PERMISSION_LIFETIME_SECS));
            alloc.last_active = now;
            alloc.client_addr = source;
        }
        build_message(
            msg_type(METHOD_CHANNEL_BIND, CLASS_SUCCESS),
            transaction,
            &[],
            Some(&key),
            true,
        )
    }

    /// Send indications carry no reply: forward DATA to the permitted peer or
    /// drop. The sender must be a known allocation client (exact source
    /// match); the peer must hold an unexpired permission. External mode
    /// additionally enforces destination policy + per-second budget and
    /// returns the allocation's own socket as the send source (the peer sees
    /// the relayed 5-tuple); compat mode sends from the shared front socket.
    fn serve_send(
        &mut self,
        attrs: &[Attr],
        transaction: &[u8; 12],
        source: SocketAddr,
        now: u64,
    ) -> Option<(SocketAddr, Vec<u8>, Option<Arc<UdpSocket>>)> {
        let external = self.relay.is_some();
        let id = self
            .allocations
            .values()
            .find(|alloc| alloc.client_addr == source)
            .map(|alloc| alloc.id)?;
        let (peer, data, via) = {
            let alloc = self.allocations.get(&id)?;
            let peer = find_attr(attrs, ATTR_XOR_PEER_ADDRESS)
                .first()
                .and_then(|raw| harbor_protocol::stun::xor_addr_decode(raw, transaction))?;
            let data = find_attr(attrs, ATTR_DATA).first()?.to_vec();
            match alloc.permissions.get(&peer.ip()) {
                Some(expires) if *expires > now => {}
                _ => return None,
            }
            if external && !peer_allowed(&peer.ip(), true) {
                return None;
            }
            (peer, data, alloc.socket.clone())
        };
        if let Some(alloc) = self.allocations.get_mut(&id) {
            if external && !budget_admit(alloc, now, data.len() as u64) {
                return None;
            }
            alloc.last_active = now;
        }
        Some((peer, data, via))
    }

    fn handle_channel_data(&mut self, bytes: &[u8], source: SocketAddr, now: u64) -> TurnOutcome {
        if bytes.len() < 4 {
            return TurnOutcome::Quiet;
        }
        let number = u16::from_be_bytes([bytes[0], bytes[1]]);
        if !(TURN_FIRST_CHANNEL..=TURN_LAST_CHANNEL).contains(&number) {
            return TurnOutcome::Quiet;
        }
        let length = u16::from_be_bytes([bytes[2], bytes[3]]) as usize;
        if bytes.len() != 4 + length.div_ceil(4) * 4 {
            return TurnOutcome::Quiet;
        }
        let external = self.relay.is_some();
        let id = match self
            .allocations
            .values()
            .find(|alloc| alloc.client_addr == source)
            .map(|alloc| alloc.id)
        {
            Some(id) => id,
            None => return TurnOutcome::Quiet,
        };
        let (peer, via) = match self.allocations.get(&id).and_then(|alloc| {
            alloc
                .channels
                .get(&number)
                .filter(|channel| channel.expires_at > now)
                .map(|channel| (channel.peer, alloc.socket.clone()))
        }) {
            Some(pair) => pair,
            None => return TurnOutcome::Quiet,
        };
        if external && !peer_allowed(&peer.ip(), true) {
            return TurnOutcome::Quiet;
        }
        let payload_len = length as u64;
        if let Some(alloc) = self.allocations.get_mut(&id) {
            if external && !budget_admit(alloc, now, payload_len) {
                return TurnOutcome::Quiet;
            }
            alloc.last_active = now;
        }
        TurnOutcome::Forward {
            to: peer,
            bytes: bytes[4..4 + length].to_vec(),
            via,
        }
    }
}

fn magic_of(bytes: &[u8]) -> u32 {
    u32::from_be_bytes([bytes[4], bytes[5], bytes[6], bytes[7]])
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// Builds a client-faithful request: attrs + long-term MI + fingerprint,
    /// computed here with the hmac crate directly (not via `build_message`),
    /// so the tests pin the wire format from the client side.
    fn client_request(
        method: u16,
        txid: [u8; 12],
        attrs: Vec<(u16, Vec<u8>)>,
        username: &str,
        password: &str,
    ) -> Vec<u8> {
        let mut body = Vec::new();
        for (attr_type, value) in &attrs {
            body.extend_from_slice(&attr_type.to_be_bytes());
            body.extend_from_slice(&(value.len() as u16).to_be_bytes());
            body.extend_from_slice(value);
            body.extend(std::iter::repeat_n(
                0_u8,
                value.len().next_multiple_of(4) - value.len(),
            ));
        }
        let key = long_term_key(username, TURN_REALM, password);
        let total = body.len() + 4 + 20 + 4 + 4;
        let mut message = Vec::with_capacity(20 + total);
        message.extend_from_slice(&msg_type(method, CLASS_REQUEST).to_be_bytes());
        message.extend_from_slice(&(total as u16).to_be_bytes());
        message.extend_from_slice(&0x2112A442_u32.to_be_bytes());
        message.extend_from_slice(&txid);
        message.extend_from_slice(&body);
        // MI HMAC input per RFC 5389 §14.5: header (length adjusted to the
        // MI end, fingerprint excluded) through the preceding attribute —
        // NOT including the MI header. This intentionally mirrors production
        // logic line for line; the RFC 5769 vector test pins it externally.
        let mut hmac_input = Vec::with_capacity(20 + body.len());
        hmac_input.extend_from_slice(&msg_type(method, CLASS_REQUEST).to_be_bytes());
        hmac_input.extend_from_slice(&((body.len() + 4 + 20) as u16).to_be_bytes());
        hmac_input.extend_from_slice(&0x2112A442_u32.to_be_bytes());
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

    fn parse_reply(reply: &[u8]) -> (u16, [u8; 12], Vec<Attr>) {
        assert!(reply.len() >= 20);
        let msg_type = u16::from_be_bytes([reply[0], reply[1]]);
        let length = u16::from_be_bytes([reply[2], reply[3]]) as usize;
        assert_eq!(reply.len(), 20 + length);
        let txid: [u8; 12] = reply[8..20].try_into().unwrap();
        (msg_type, txid, parse_attrs(&reply[20..]).unwrap())
    }

    fn error_code(attrs: &[Attr]) -> Option<u16> {
        let found = find_attr(attrs, ATTR_ERROR_CODE);
        let raw = found.first()?;
        if raw.len() < 4 {
            return None;
        }
        Some(raw[2] as u16 * 100 + raw[3] as u16)
    }

    #[test]
    fn crc_sanity_check_value() {
        // CRC32("123456789") = 0xCBF43926; our fingerprint XORs 0x5354554E.
        assert_eq!(fingerprint_of(b"123456789"), 0xCBF43926 ^ 0x5354554E);
    }

    #[test]
    fn message_types_match_rfc_values() {
        assert_eq!(msg_type(0x001, 0b00), 0x0001);
        assert_eq!(msg_type(0x001, 0b10), 0x0101);
        assert_eq!(msg_type(0x003, 0b00), 0x0003);
        assert_eq!(msg_type(0x003, 0b11), 0x0113);
        assert_eq!(msg_type(0x004, 0b00), 0x0004);
        assert_eq!(msg_type(0x008, 0b00), 0x0008);
        assert_eq!(msg_type(0x009, 0b00), 0x0009);
        assert_eq!(msg_type(0x006, 0b01), 0x0016);
        assert_eq!(msg_type(0x007, 0b01), 0x0017);
        assert_eq!(method_of(0x0113), 0x003);
        assert_eq!(class_of(0x0113), 0b11);
    }

    #[test]
    fn built_messages_verify_against_themselves() {
        let key = long_term_key("device-1", TURN_REALM, "secret-password");
        let txid = txid(7);
        let built = build_message(
            msg_type(METHOD_ALLOCATE, CLASS_SUCCESS),
            &txid,
            &[
                (
                    ATTR_XOR_RELAYED_ADDRESS,
                    vec![0x00, 0x01, 0x21, 0x12, 0xA4, 0x42, 0x7F, 0x00, 0x00, 0x01],
                ),
                (ATTR_LIFETIME, 600_u32.to_be_bytes().to_vec()),
            ],
            Some(&key),
            true,
        );
        let attrs = parse_attrs(&built[20..]).unwrap();
        assert!(verify_mi(&built, &attrs, &key));
        // Fingerprint over everything before it validates too.
        let at = built.len() - 8;
        let mut check = built[..at].to_vec();
        assert_eq!(
            fingerprint_of(&check),
            u32::from_be_bytes([built[at + 4], built[at + 5], built[at + 6], built[at + 7]])
        );
        let _ = &mut check;
    }

    #[test]
    fn rfc5769_long_term_vector_verifies() {
        // Exact bytes from RFC 5769 section 2.2 (also in pion/stun
        // rfc5769_test.go): username, realm example.org, password TheMatrIX.
        #[rustfmt::skip]
        let request: Vec<u8> = vec![
            0x00, 0x01, 0x00, 0x60, 0x21, 0x12, 0xa4, 0x42, 0x78, 0xad, 0x34, 0x33, 0xc6,
            0xad, 0x72, 0xc0, 0x29, 0xda, 0x41, 0x2e, 0x00, 0x06, 0x00, 0x12, 0xe3, 0x83,
            0x9e, 0xe3, 0x83, 0x88, 0xe3, 0x83, 0xaa, 0xe3, 0x83, 0x83, 0xe3, 0x82, 0xaf,
            0xe3, 0x82, 0xb9, 0x00, 0x00, 0x00, 0x15, 0x00, 0x1c, 0x66, 0x2f, 0x2f, 0x34,
            0x39, 0x39, 0x6b, 0x39, 0x35, 0x34, 0x64, 0x36, 0x4f, 0x4c, 0x33, 0x34, 0x6f,
            0x4c, 0x39, 0x46, 0x53, 0x54, 0x76, 0x79, 0x36, 0x34, 0x73, 0x41, 0x00, 0x14,
            0x00, 0x0b, 0x65, 0x78, 0x61, 0x6d, 0x70, 0x6c, 0x65, 0x2e, 0x6f, 0x72, 0x67,
            0x00, 0x00, 0x08, 0x00, 0x14, 0xf6, 0x70, 0x24, 0x65, 0x6d, 0xd6, 0x4a, 0x3e,
            0x02, 0xb8, 0xe0, 0x71, 0x2e, 0x85, 0xc9, 0xa2, 0x8c, 0xa8, 0x96, 0x66,
        ];
        let attrs = parse_attrs(&request[20..]).unwrap();
        let key = long_term_key("マトリックス", "example.org", "TheMatrIX");
        assert!(verify_mi(&request, &attrs, &key));
        let mut tampered = request.clone();
        tampered[50] ^= 0x01;
        let tampered_attrs = parse_attrs(&tampered[20..]).unwrap();
        assert!(!verify_mi(&tampered, &tampered_attrs, &key));
        let wrong = long_term_key("マトリックス", "example.org", "wrong");
        assert!(!verify_mi(&request, &attrs, &wrong));
    }

    fn provisioned() -> (TurnState, String, String, SocketAddr, SocketAddr) {
        let mut turn = TurnState::new();
        let device = Uuid::new_v4();
        let (username, password, _) = turn.mint_credential(device, 1000);
        let client: SocketAddr = "192.0.2.10:40001".parse().unwrap();
        let relay: SocketAddr = "192.0.2.99:9091".parse().unwrap();
        (turn, username, password, client, relay)
    }

    fn authed_allocate(
        turn: &mut TurnState,
        username: &str,
        password: &str,
        client: SocketAddr,
        relay: SocketAddr,
        now: u64,
    ) {
        let anon = vec![
            0x00, 0x03, 0x00, 0x08, 0x21, 0x12, 0xA4, 0x42, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12,
            0x00, 0x19, 0x00, 0x04, 0x11, 0x00, 0x00, 0x00,
        ];
        let reply = match turn.handle_datagram(&anon, client, relay, now) {
            TurnOutcome::Reply(bytes) => bytes,
            TurnOutcome::Quiet | TurnOutcome::Forward { .. } => {
                panic!("anonymous allocate must answer")
            }
        };
        let (got_type, _, attrs) = parse_reply(&reply);
        assert_eq!(got_type, msg_type(METHOD_ALLOCATE, CLASS_ERROR));
        assert_eq!(error_code(&attrs), Some(401));
        let nonce = find_attr(&attrs, ATTR_NONCE).first().unwrap().to_vec();
        let realm = find_attr(&attrs, ATTR_REALM).first().unwrap().to_vec();
        assert_eq!(realm, b"harbor");

        let tx = txid(40);
        let request = client_request(
            METHOD_ALLOCATE,
            tx,
            vec![
                (ATTR_REQUESTED_TRANSPORT, vec![17, 0, 0, 0]),
                (ATTR_USERNAME, username.as_bytes().to_vec()),
                (ATTR_REALM, realm),
                (ATTR_NONCE, nonce),
            ],
            username,
            password,
        );
        let reply = match turn.handle_datagram(&request, client, relay, now) {
            TurnOutcome::Reply(bytes) => bytes,
            TurnOutcome::Quiet | TurnOutcome::Forward { .. } => {
                panic!("authed allocate must answer")
            }
        };
        let (got_type, echo, attrs) = parse_reply(&reply);
        assert_eq!(got_type, msg_type(METHOD_ALLOCATE, CLASS_SUCCESS));
        assert_eq!(echo, tx);
        let found = find_attr(&attrs, ATTR_XOR_RELAYED_ADDRESS);
        let relayed = found.first().unwrap();
        let decoded = harbor_protocol::stun::xor_addr_decode(relayed, &tx);
        if turn.external_mode() {
            // External mode advertises the public IP + dedicated port, never
            // the front socket passed as `relay`.
            let addr = decoded.expect("relayed decodes");
            let cfg = turn.relay.expect("external config");
            assert_eq!(addr.ip(), cfg.external_ip);
            assert!((cfg.port_low..=cfg.port_high).contains(&addr.port()));
        } else {
            assert_eq!(decoded, Some(relay));
        }
    }

    #[test]
    fn allocate_refresh_permission_send_data_lifecycle() {
        let (mut turn, username, password, client, relay) = provisioned();
        authed_allocate(&mut turn, &username, &password, client, relay, 1000);

        let anon_refresh = {
            let mut message = vec![0x00, 0x04, 0x00, 0x00, 0x21, 0x12, 0xA4, 0x42];
            message.extend_from_slice(&txid(62));
            message
        };
        let reply = match turn.handle_datagram(&anon_refresh, client, relay, 1001) {
            TurnOutcome::Reply(bytes) => bytes,
            TurnOutcome::Quiet | TurnOutcome::Forward { .. } => panic!("refresh challenge"),
        };
        let (_, _, attrs) = parse_reply(&reply);
        let nonce = find_attr(&attrs, ATTR_NONCE).first().unwrap().to_vec();
        let extend = client_request(
            METHOD_REFRESH,
            txid(63),
            vec![
                (ATTR_USERNAME, username.as_bytes().to_vec()),
                (ATTR_REALM, b"harbor".to_vec()),
                (ATTR_NONCE, nonce.clone()),
                (ATTR_LIFETIME, 300_u32.to_be_bytes().to_vec()),
            ],
            &username,
            &password,
        );
        let reply = match turn.handle_datagram(&extend, client, relay, 1002) {
            TurnOutcome::Reply(bytes) => bytes,
            TurnOutcome::Quiet | TurnOutcome::Forward { .. } => panic!("refresh extend"),
        };
        let (_, _, attrs) = parse_reply(&reply);
        let found = find_attr(&attrs, ATTR_LIFETIME);
        let lifetime = found.first().unwrap();
        assert_eq!(
            u32::from_be_bytes([lifetime[0], lifetime[1], lifetime[2], lifetime[3]]),
            300
        );

        let peer: SocketAddr = "198.51.100.7:5000".parse().unwrap();
        let perm = client_request(
            METHOD_CREATE_PERMISSION,
            txid(70),
            vec![
                (ATTR_USERNAME, username.as_bytes().to_vec()),
                (ATTR_REALM, b"harbor".to_vec()),
                (ATTR_NONCE, nonce.clone()),
                (
                    ATTR_XOR_PEER_ADDRESS,
                    harbor_protocol::stun::xor_addr_value(peer, &txid(70)),
                ),
            ],
            &username,
            &password,
        );
        let reply = match turn.handle_datagram(&perm, client, relay, 1003) {
            TurnOutcome::Reply(bytes) => bytes,
            TurnOutcome::Quiet | TurnOutcome::Forward { .. } => panic!("permission"),
        };
        let (got_type, _, _) = parse_reply(&reply);
        assert_eq!(got_type, msg_type(METHOD_CREATE_PERMISSION, CLASS_SUCCESS));

        let mut send = vec![0x00, 0x16, 0x00, 0x00, 0x21, 0x12, 0xA4, 0x42];
        send.extend_from_slice(&txid(71));
        let mut send_attrs = Vec::new();
        send_attrs.extend_from_slice(&ATTR_XOR_PEER_ADDRESS.to_be_bytes());
        let peer_value = harbor_protocol::stun::xor_addr_value(peer, &txid(71));
        send_attrs.extend_from_slice(&(peer_value.len() as u16).to_be_bytes());
        send_attrs.extend_from_slice(&peer_value);
        send_attrs.extend_from_slice(&ATTR_DATA.to_be_bytes());
        send_attrs.extend_from_slice(&5_u16.to_be_bytes());
        send_attrs.extend_from_slice(b"hello");
        send_attrs.extend([0_u8; 3]);
        send[2..4].copy_from_slice(&(send_attrs.len() as u16).to_be_bytes());
        send.extend_from_slice(&send_attrs);
        match turn.handle_datagram(&send, client, relay, 1004) {
            TurnOutcome::Forward { to, bytes, via } => {
                assert_eq!(to, peer);
                assert_eq!(bytes, b"hello");
                assert!(via.is_none());
            }
            TurnOutcome::Reply(_) | TurnOutcome::Quiet => panic!("send must forward"),
        }

        match turn.handle_datagram(b"voice-bytes", peer, relay, 1005) {
            TurnOutcome::Forward { to, bytes, via } => {
                assert_eq!(to, client);
                assert!(via.is_none());
                assert!(bytes.len() >= 20);
                let got_type = u16::from_be_bytes([bytes[0], bytes[1]]);
                let txid: [u8; 12] = bytes[8..20].try_into().unwrap();
                assert_eq!(got_type, msg_type(METHOD_DATA, CLASS_INDICATION));
                let data_attrs = parse_attrs(&bytes[20..]).unwrap();
                let found_peer = find_attr(&data_attrs, ATTR_XOR_PEER_ADDRESS);
                let from = found_peer.first().unwrap();
                assert_eq!(
                    harbor_protocol::stun::xor_addr_decode(from, &txid),
                    Some(peer)
                );
                let found_data = find_attr(&data_attrs, ATTR_DATA);
                let data = found_data.first().unwrap();
                assert_eq!(data, b"voice-bytes");
            }
            TurnOutcome::Reply(_) | TurnOutcome::Quiet => panic!("peer data must relay"),
        }

        let delete = client_request(
            METHOD_REFRESH,
            txid(80),
            vec![
                (ATTR_USERNAME, username.as_bytes().to_vec()),
                (ATTR_REALM, b"harbor".to_vec()),
                (ATTR_NONCE, nonce),
                (ATTR_LIFETIME, 0_u32.to_be_bytes().to_vec()),
            ],
            &username,
            &password,
        );
        let reply = match turn.handle_datagram(&delete, client, relay, 1006) {
            TurnOutcome::Reply(bytes) => bytes,
            TurnOutcome::Quiet | TurnOutcome::Forward { .. } => panic!("delete"),
        };
        let (got_type, _, _) = parse_reply(&reply);
        assert_eq!(got_type, msg_type(METHOD_REFRESH, CLASS_SUCCESS));
    }

    #[test]
    fn auth_failures_challenge_or_silence() {
        let (mut turn, username, password, client, relay) = provisioned();
        let tx = txid(90);
        let bad_user = client_request(
            METHOD_ALLOCATE,
            tx,
            vec![
                (ATTR_REQUESTED_TRANSPORT, vec![17, 0, 0, 0]),
                (ATTR_USERNAME, b"ghost".to_vec()),
                (ATTR_REALM, b"harbor".to_vec()),
                (ATTR_NONCE, b"made-up-nonce".to_vec()),
            ],
            "ghost",
            "wrong",
        );
        let reply = match turn.handle_datagram(&bad_user, client, relay, 1000) {
            TurnOutcome::Reply(bytes) => bytes,
            TurnOutcome::Quiet | TurnOutcome::Forward { .. } => panic!("must challenge"),
        };
        let (_, _, attrs) = parse_reply(&reply);
        assert_eq!(error_code(&attrs), Some(401));

        let tx = txid(91);
        let mut tampered = client_request(
            METHOD_ALLOCATE,
            tx,
            vec![
                (ATTR_REQUESTED_TRANSPORT, vec![17, 0, 0, 0]),
                (ATTR_USERNAME, username.as_bytes().to_vec()),
                (ATTR_REALM, b"harbor".to_vec()),
                (ATTR_NONCE, b"made-up-nonce".to_vec()),
            ],
            &username,
            &password,
        );
        let last = tampered.len() - 1;
        tampered[last] ^= 0x01;
        match turn.handle_datagram(&tampered, client, relay, 1001) {
            TurnOutcome::Quiet => {}
            TurnOutcome::Reply(_) | TurnOutcome::Forward { .. } => {
                panic!("tampered datagram must stay silent")
            }
        }

        let (mut turn, username, password, client, relay) = provisioned();
        let tx = txid(92);
        let stale = client_request(
            METHOD_ALLOCATE,
            tx,
            vec![
                (ATTR_REQUESTED_TRANSPORT, vec![17, 0, 0, 0]),
                (ATTR_USERNAME, username.as_bytes().to_vec()),
                (ATTR_REALM, b"harbor".to_vec()),
                (ATTR_NONCE, b"expired-nonce".to_vec()),
            ],
            &username,
            &password,
        );
        let reply = match turn.handle_datagram(&stale, client, relay, 1002) {
            TurnOutcome::Reply(bytes) => bytes,
            TurnOutcome::Quiet | TurnOutcome::Forward { .. } => panic!("must answer 438"),
        };
        let (_, _, attrs) = parse_reply(&reply);
        assert_eq!(error_code(&attrs), Some(438));
        assert!(!find_attr(&attrs, ATTR_NONCE).is_empty());
    }

    #[test]
    fn channel_bind_round_trips_as_channel_data() {
        let (mut turn, username, password, client, relay) = provisioned();
        authed_allocate(&mut turn, &username, &password, client, relay, 1000);
        let peer: SocketAddr = "198.51.100.9:6000".parse().unwrap();
        let nonce = turn
            .nonces
            .front()
            .map(|(nonce, _)| nonce.as_bytes().to_vec())
            .unwrap();

        let bind = client_request(
            METHOD_CHANNEL_BIND,
            txid(100),
            vec![
                (ATTR_USERNAME, username.as_bytes().to_vec()),
                (ATTR_REALM, b"harbor".to_vec()),
                (ATTR_NONCE, nonce),
                (ATTR_CHANNEL_NUMBER, vec![0x40, 0x01, 0x00, 0x00]),
                (
                    ATTR_XOR_PEER_ADDRESS,
                    harbor_protocol::stun::xor_addr_value(peer, &txid(100)),
                ),
            ],
            &username,
            &password,
        );
        let reply = match turn.handle_datagram(&bind, client, relay, 1001) {
            TurnOutcome::Reply(bytes) => bytes,
            TurnOutcome::Quiet | TurnOutcome::Forward { .. } => panic!("channel bind"),
        };
        let (got_type, _, _) = parse_reply(&reply);
        assert_eq!(got_type, msg_type(METHOD_CHANNEL_BIND, CLASS_SUCCESS));

        let mut channel_data = vec![0x40, 0x01, 0x00, 0x05];
        channel_data.extend_from_slice(b"audio");
        channel_data.extend([0_u8; 3]);
        match turn.handle_datagram(&channel_data, client, relay, 1002) {
            TurnOutcome::Forward { to, bytes, via } => {
                assert_eq!(to, peer);
                assert_eq!(bytes, b"audio");
                assert!(via.is_none());
            }
            TurnOutcome::Reply(_) | TurnOutcome::Quiet => panic!("channel data must forward"),
        }

        match turn.handle_datagram(b"video", peer, relay, 1003) {
            TurnOutcome::Forward { to, bytes, via } => {
                assert_eq!(to, client);
                assert!(via.is_none());
                assert_eq!(u16::from_be_bytes([bytes[0], bytes[1]]), 0x4001);
                assert_eq!(&bytes[4..9], b"video");
            }
            TurnOutcome::Reply(_) | TurnOutcome::Quiet => panic!("peer data must relay bound"),
        }
    }

    #[test]
    fn expiry_and_caps_behave() {
        let (mut turn, username, password, client, relay) = provisioned();
        authed_allocate(&mut turn, &username, &password, client, relay, 1000);
        turn.expire(1000 + TURN_MAX_LIFETIME_SECS + 1);
        assert!(turn.allocations.is_empty());
        let tx = txid(110);
        // 0x7777: unknown AND comprehension-required (< 0x8000), so the
        // server must answer 420. (0x9999 would be comprehension-optional
        // and silently ignored per RFC 5389 §7.3 — a previous revision of
        // this test used it by mistake.)
        let mut odd = vec![0x00, 0x03, 0x00, 0x0c, 0x21, 0x12, 0xA4, 0x42];
        odd.extend_from_slice(&tx);
        odd.extend_from_slice(&[0x00, 0x19, 0x00, 0x04, 0x11, 0x00, 0x00, 0x00]);
        odd.extend_from_slice(&[0x77, 0x77, 0x00, 0x00]);
        match turn.handle_datagram(&odd, client, relay, 2000) {
            TurnOutcome::Reply(bytes) => {
                let (_, _, attrs) = parse_reply(&bytes);
                assert_eq!(error_code(&attrs), Some(420));
                let found = find_attr(&attrs, ATTR_UNKNOWN_ATTRIBUTES);
                let unknown = found.first().unwrap();
                assert_eq!(unknown, &[0x77, 0x77]);
            }
            TurnOutcome::Quiet | TurnOutcome::Forward { .. } => panic!("420 expected"),
        }
    }

    fn provisioned_external() -> (TurnState, String, String, SocketAddr, SocketAddr) {
        let mut turn = TurnState::new();
        // TEST-NET-1 as the "public" IP: routable-shaped for peer_allowed,
        // never a real bind (bind_socket uses wildcard + range port).
        let external: std::net::IpAddr = "192.0.2.99".parse().unwrap();
        let cfg = TurnRelayConfig::new(external, 49160, 49175).expect("valid test range");
        turn.set_external_relay(cfg);
        let device = Uuid::new_v4();
        let (username, password, _) = turn.mint_credential(device, 1000);
        let client: SocketAddr = "192.0.2.10:40001".parse().unwrap();
        let front: SocketAddr = "10.0.0.7:9091".parse().unwrap();
        (turn, username, password, client, front)
    }

    #[test]
    fn external_allocate_advertises_public_ip_with_dedicated_socket() {
        let (mut turn, username, password, client, front) = provisioned_external();
        assert!(turn.external_mode());
        authed_allocate(&mut turn, &username, &password, client, front, 1000);
        // authed_allocate asserts the reply equals `front` in compat mode;
        // in external mode the relayed address must instead be the public IP
        // with a port from the configured range. Re-read it directly:
        let alloc_id = turn
            .by_client
            .values()
            .next()
            .copied()
            .expect("allocation exists");
        let relayed = turn
            .allocation_relayed(&alloc_id)
            .expect("external relayed addr");
        assert_eq!(relayed.ip().to_string(), "192.0.2.99");
        assert!((49160..=49175).contains(&relayed.port()));
        assert!(turn.allocation_socket(&alloc_id).is_some());
        assert_eq!(turn.drain_socket_ready().len(), 1);
    }

    #[test]
    fn external_permission_rejects_nonpublic_peers_with_403() {
        let (mut turn, username, password, client, front) = provisioned_external();
        authed_allocate(&mut turn, &username, &password, client, front, 1000);
        let nonce = turn
            .nonces
            .front()
            .map(|(nonce, _)| nonce.as_bytes().to_vec())
            .unwrap();
        for (peer_str, want_code) in [
            ("127.0.0.1:5000", Some(403)),
            ("10.0.0.9:5000", Some(403)),
            ("192.168.1.7:5000", Some(403)),
            ("224.0.0.1:5000", Some(403)),
            ("8.8.8.8:5000", None),
            ("198.51.100.7:5000", None),
        ] {
            let peer: SocketAddr = peer_str.parse().unwrap();
            let req = client_request(
                METHOD_CREATE_PERMISSION,
                txid(120),
                vec![
                    (ATTR_USERNAME, username.as_bytes().to_vec()),
                    (ATTR_REALM, b"harbor".to_vec()),
                    (ATTR_NONCE, nonce.clone()),
                    (
                        ATTR_XOR_PEER_ADDRESS,
                        harbor_protocol::stun::xor_addr_value(peer, &txid(120)),
                    ),
                ],
                &username,
                &password,
            );
            let reply = match turn.handle_datagram(&req, client, front, 1001) {
                TurnOutcome::Reply(bytes) => bytes,
                TurnOutcome::Quiet | TurnOutcome::Forward { .. } => {
                    panic!("permission must answer")
                }
            };
            let (got_type, _, attrs) = parse_reply(&reply);
            match want_code {
                Some(code) => {
                    assert_eq!(got_type, msg_type(METHOD_CREATE_PERMISSION, CLASS_ERROR));
                    assert_eq!(error_code(&attrs), Some(code), "peer {peer_str}");
                }
                None => {
                    assert_eq!(got_type, msg_type(METHOD_CREATE_PERMISSION, CLASS_SUCCESS));
                }
            }
        }
    }

    #[test]
    fn external_relay_receipt_relays_public_and_drops_private() {
        let (mut turn, username, password, client, front) = provisioned_external();
        // Allocate bypasses the relayed-address assert (checked above); do a
        // manual allocate here so socket_ready accounting stays simple.
        authed_allocate(&mut turn, &username, &password, client, front, 1000);
        let alloc_id = turn.by_client.values().next().copied().unwrap();
        let nonce = turn
            .nonces
            .front()
            .map(|(nonce, _)| nonce.as_bytes().to_vec())
            .unwrap();
        let public_peer: SocketAddr = "198.51.100.7:5000".parse().unwrap();
        let perm = client_request(
            METHOD_CREATE_PERMISSION,
            txid(130),
            vec![
                (ATTR_USERNAME, username.as_bytes().to_vec()),
                (ATTR_REALM, b"harbor".to_vec()),
                (ATTR_NONCE, nonce),
                (
                    ATTR_XOR_PEER_ADDRESS,
                    harbor_protocol::stun::xor_addr_value(public_peer, &txid(130)),
                ),
            ],
            &username,
            &password,
        );
        let reply = match turn.handle_datagram(&perm, client, front, 1001) {
            TurnOutcome::Reply(bytes) => bytes,
            TurnOutcome::Quiet | TurnOutcome::Forward { .. } => panic!("permission"),
        };
        let (got_type, _, _) = parse_reply(&reply);
        assert_eq!(got_type, msg_type(METHOD_CREATE_PERMISSION, CLASS_SUCCESS));

        // Public peer payload arrives on the dedicated socket: wrapped as a
        // Data indication for the client.
        let wrapped = turn
            .relay_receipt(&alloc_id, public_peer, b"voice-bytes", 1002)
            .expect("public peer relays");
        let got_type = u16::from_be_bytes([wrapped[0], wrapped[1]]);
        assert_eq!(got_type, msg_type(METHOD_DATA, CLASS_INDICATION));

        // Private/loopback sources never relay in external mode, even with
        // bytes that look like media.
        for blocked in ["127.0.0.1:9000", "10.0.0.9:9000", "192.168.1.7:9000"] {
            let source: SocketAddr = blocked.parse().unwrap();
            assert!(
                turn.relay_receipt(&alloc_id, source, b"voice-bytes", 1003)
                    .is_none(),
                "blocked {blocked}"
            );
        }
        // Compat front-door path stays shut in external mode: peers must use
        // the dedicated socket, never the shared front door.
        match turn.handle_datagram(b"voice-bytes", public_peer, front, 1004) {
            TurnOutcome::Quiet => {}
            TurnOutcome::Reply(_) | TurnOutcome::Forward { .. } => {
                panic!("front door must not relay in external mode")
            }
        }
    }

    #[test]
    fn external_channel_bind_relays_as_channel_data_via_dedicated_socket() {
        let (mut turn, username, password, client, front) = provisioned_external();
        authed_allocate(&mut turn, &username, &password, client, front, 1000);
        let alloc_id = turn.by_client.values().next().copied().unwrap();
        let nonce = turn
            .nonces
            .front()
            .map(|(nonce, _)| nonce.as_bytes().to_vec())
            .unwrap();
        let peer: SocketAddr = "198.51.100.9:6000".parse().unwrap();
        let bind = client_request(
            METHOD_CHANNEL_BIND,
            txid(150),
            vec![
                (ATTR_USERNAME, username.as_bytes().to_vec()),
                (ATTR_REALM, b"harbor".to_vec()),
                (ATTR_NONCE, nonce.clone()),
                (ATTR_CHANNEL_NUMBER, vec![0x40, 0x01, 0x00, 0x00]),
                (
                    ATTR_XOR_PEER_ADDRESS,
                    harbor_protocol::stun::xor_addr_value(peer, &txid(150)),
                ),
            ],
            &username,
            &password,
        );
        let reply = match turn.handle_datagram(&bind, client, front, 1001) {
            TurnOutcome::Reply(bytes) => bytes,
            TurnOutcome::Quiet | TurnOutcome::Forward { .. } => panic!("channel bind"),
        };
        let (got_type, _, _) = parse_reply(&reply);
        assert_eq!(got_type, msg_type(METHOD_CHANNEL_BIND, CLASS_SUCCESS));

        // Client -> peer ChannelData leaves from the allocation's own socket.
        let mut channel_data = vec![0x40, 0x01, 0x00, 0x05];
        channel_data.extend_from_slice(b"audio");
        channel_data.extend([0_u8; 3]);
        match turn.handle_datagram(&channel_data, client, front, 1002) {
            TurnOutcome::Forward { to, bytes, via } => {
                assert_eq!(to, peer);
                assert_eq!(bytes, b"audio");
                assert!(via.is_some());
            }
            TurnOutcome::Reply(_) | TurnOutcome::Quiet => panic!("channel data must forward"),
        }

        // Peer -> client on the dedicated socket arrives ChannelData-framed.
        let framed = turn
            .relay_receipt(&alloc_id, peer, b"video", 1003)
            .expect("bound peer relays");
        assert_eq!(u16::from_be_bytes([framed[0], framed[1]]), 0x4001);
        assert_eq!(&framed[4..9], b"video");

        // A mapped-private peer cannot hold a channel. Against an IPv4
        // relay it dies on family mismatch (443) before destination policy;
        // the mapped-v4 policy below is what guards an IPv6 relay.
        let mapped: SocketAddr = "[::ffff:10.0.0.9]:6000".parse().unwrap();
        let bad_bind = client_request(
            METHOD_CHANNEL_BIND,
            txid(151),
            vec![
                (ATTR_USERNAME, username.as_bytes().to_vec()),
                (ATTR_REALM, b"harbor".to_vec()),
                (ATTR_NONCE, nonce),
                (ATTR_CHANNEL_NUMBER, vec![0x40, 0x02, 0x00, 0x00]),
                (
                    ATTR_XOR_PEER_ADDRESS,
                    harbor_protocol::stun::xor_addr_value(mapped, &txid(151)),
                ),
            ],
            &username,
            &password,
        );
        let reply = match turn.handle_datagram(&bad_bind, client, front, 1004) {
            TurnOutcome::Reply(bytes) => bytes,
            TurnOutcome::Quiet | TurnOutcome::Forward { .. } => panic!("channel bind must answer"),
        };
        let (got_type, _, attrs) = parse_reply(&reply);
        assert_eq!(got_type, msg_type(METHOD_CHANNEL_BIND, CLASS_ERROR));
        assert_eq!(error_code(&attrs), Some(443));
    }

    #[test]
    fn own_relay_sources_match_live_ports_only() {
        let (mut turn, username, password, client, front) = provisioned_external();
        authed_allocate(&mut turn, &username, &password, client, front, 1000);
        let alloc_id = turn.by_client.values().next().copied().unwrap();
        let relayed = turn
            .allocation_relayed(&alloc_id)
            .expect("external relayed");
        // Same port as a live allocation reads as ours whatever the IP —
        // including the guest-private source a hairpinned forward carries.
        for ip in ["10.0.0.7", "127.0.0.1", "192.0.2.99", "8.8.8.8"] {
            let source: SocketAddr = format!("{ip}:{}", relayed.port()).parse().unwrap();
            assert!(turn.is_own_relay_source(source), "own port {source}");
        }
        // Any other port is not ours, even on loopback.
        let other: SocketAddr = "127.0.0.1:9".parse().unwrap();
        assert!(!turn.is_own_relay_source(other));
        // After expiry the port stops reading as ours.
        turn.expire(1000 + TURN_MAX_LIFETIME_SECS + 1);
        let gone: SocketAddr = format!("10.0.0.7:{}", relayed.port()).parse().unwrap();
        assert!(!turn.is_own_relay_source(gone));
    }

    #[test]
    fn peer_policy_blocks_private_mapped_and_keeps_cgnat() {
        let blocked = [
            "127.0.0.1",
            "10.0.0.9",
            "172.16.0.1",
            "172.31.255.255",
            "192.168.1.7",
            "169.254.10.20",
            "224.0.0.1",
            "::1",
            "fe80::1",
            "fc00::1",
            "::ffff:10.0.0.9",
            "::ffff:192.168.1.7",
            "::ffff:127.0.0.1",
        ];
        for addr in blocked {
            let ip: IpAddr = addr.parse().unwrap();
            assert!(!peer_allowed(&ip, true), "external must block {addr}");
            assert!(peer_allowed(&ip, false), "compat keeps {addr} legal");
        }
        for addr in [
            "8.8.8.8",
            "100.64.0.1",
            "192.0.2.1",
            "2001:db8::1",
            "::ffff:8.8.8.8",
        ] {
            let ip: IpAddr = addr.parse().unwrap();
            assert!(peer_allowed(&ip, true), "external must allow {addr}");
        }
    }

    #[test]
    fn external_send_budget_drops_floods_silently() {
        let (mut turn, username, password, client, front) = provisioned_external();
        authed_allocate(&mut turn, &username, &password, client, front, 1000);
        let nonce = turn
            .nonces
            .front()
            .map(|(nonce, _)| nonce.as_bytes().to_vec())
            .unwrap();
        let peer: SocketAddr = "198.51.100.7:5000".parse().unwrap();
        let perm = client_request(
            METHOD_CREATE_PERMISSION,
            txid(140),
            vec![
                (ATTR_USERNAME, username.as_bytes().to_vec()),
                (ATTR_REALM, b"harbor".to_vec()),
                (ATTR_NONCE, nonce),
                (
                    ATTR_XOR_PEER_ADDRESS,
                    harbor_protocol::stun::xor_addr_value(peer, &txid(140)),
                ),
            ],
            &username,
            &password,
        );
        let reply = match turn.handle_datagram(&perm, client, front, 1001) {
            TurnOutcome::Reply(bytes) => bytes,
            TurnOutcome::Quiet | TurnOutcome::Forward { .. } => panic!("permission"),
        };
        let (got_type, _, _) = parse_reply(&reply);
        assert_eq!(got_type, msg_type(METHOD_CREATE_PERMISSION, CLASS_SUCCESS));

        // One small Send succeeds and carries the allocation socket.
        let mut send = vec![0x00, 0x16, 0x00, 0x00, 0x21, 0x12, 0xA4, 0x42];
        send.extend_from_slice(&txid(141));
        let mut attrs = Vec::new();
        let peer_value = harbor_protocol::stun::xor_addr_value(peer, &txid(141));
        attrs.extend_from_slice(&ATTR_XOR_PEER_ADDRESS.to_be_bytes());
        attrs.extend_from_slice(&(peer_value.len() as u16).to_be_bytes());
        attrs.extend_from_slice(&peer_value);
        attrs.extend_from_slice(&ATTR_DATA.to_be_bytes());
        attrs.extend_from_slice(&5_u16.to_be_bytes());
        attrs.extend_from_slice(b"hello");
        attrs.extend([0_u8; 3]);
        send[2..4].copy_from_slice(&(attrs.len() as u16).to_be_bytes());
        send.extend_from_slice(&attrs);
        match turn.handle_datagram(&send, client, front, 1002) {
            TurnOutcome::Forward { to, bytes, via } => {
                assert_eq!(to, peer);
                assert_eq!(bytes, b"hello");
                assert!(via.is_some());
            }
            TurnOutcome::Reply(_) | TurnOutcome::Quiet => panic!("send must forward"),
        }

        // Flood the same second beyond 512 KiB: relay_receipt must start
        // dropping (silent, no reply) once the budget is spent.
        let big = vec![0xabu8; 60_000];
        let mut admitted = 0_usize;
        let alloc_id = turn.by_client.values().next().copied().unwrap();
        for _ in 0..12 {
            if turn.relay_receipt(&alloc_id, peer, &big, 1002).is_some() {
                admitted += 1;
            }
        }
        assert!(
            admitted < 12,
            "budget must drop floods (admitted {admitted}/12)"
        );
        // Next second the window resets: traffic flows again.
        assert!(
            turn.relay_receipt(&alloc_id, peer, b"again", 1003)
                .is_some(),
            "budget must reset each second"
        );
    }
}
