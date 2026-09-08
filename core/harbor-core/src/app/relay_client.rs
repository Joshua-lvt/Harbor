//! Client side of relay fallback: session policy, poll worker, and the
//! end-to-end crypto glue. Delivery into the transcript reuses the exact
//! bearer path (`absorb_link_frame`); the relay only changes how sealed
//! frames travel, never what they mean.
//!
//! Split of duties:
//!
//! * [`RelaySessions`] (pump thread): which peer has a session, handshake
//!   state, sealing. Pure policy plus [`super::relay_crypto`]; no sockets.
//! * [`RelayPollWorker`] (own thread): one persistent event connection
//!   parked in `relay.poll`, emitting what the server delivers. It never
//!   decides anything; shutdown detaches it like [`super::MobileLink`]
//!   (worst linger: one parked hold, then it sees the flag and exits).
//! * `relay.open` / `accept` / `close` / `data` exchanges ride the control
//!   connection on the pump thread (immediate request/response).
//!
//! Trigger policy (see `relay_wanted`): chat queued, bearer down, worker
//! path unavailable. Acceptance is automatic for paired peers with idle
//! expiry server-side; the explicit `relay.accept` message (10 s window)
//! still flows, so the server never relays on a bare open.

use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use uuid::Uuid;

use super::mobile_link::{fingerprint_hex, parse_fingerprint_hex};
use super::now_seconds;
use super::relay_crypto::{PendingCrypto, RelayCrypto};
use crate::device::reconnect_delay;
use crate::{ServerClient, ServerPin, load_or_create, rfc3339_now};

/// Idle grace before an unneeded poll worker stops. Sessions drive the
/// worker: it runs while any session lives, plus this long after the last
/// one drains so flapping chats do not churn TLS connections.
const WORKER_IDLE_GRACE: Duration = Duration::from_secs(60);

/// Inbox for frames that arrive before the handshake completes: the peer
/// seals as soon as *it* is ready, which races our own completion. Bounded;
/// beyond the cap the newest arrivals drop (the oldest prefix is what
/// unblocks the stream, and live senders retransmit what matters).
pub const INBOX_CAP: usize = 16;

/// Relay send budget per second, shared across sessions: the K11+ uplink is
/// the constraint, not CPU. Chunk sends that miss the budget wait for the
/// next window (board `pending` keeps them); the worker path paces itself
/// the same way, one chunk per transfer per tick.
pub const RELAY_THROTTLE_BYTES_PER_SEC: u64 = 256 * 1024;

/// One sealed-then-routed companion frame, as the poll worker reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireFrame {
    pub relay_id: Uuid,
    pub from: Uuid,
    pub seq: u64,
    pub bytes: String,
}

/// An open awaiting our acceptance, as the poll worker reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireOpen {
    pub relay_id: Uuid,
    pub from: Uuid,
    pub purpose: String,
}

/// What the poll loop delivers. Timeouts (empty answers) are swallowed in
/// the worker: no information, no event.
#[derive(Debug)]
pub enum RelayEvent {
    Polled {
        frames: Vec<WireFrame>,
        closed: Vec<Uuid>,
        opens: Vec<WireOpen>,
    },
}

/// Handshake state of one peer session. Keys are ephemeral per session;
/// reopening after a server-side expiry mints fresh ones.
pub(crate) enum SessionCrypto {
    Pending(PendingCrypto),
    Ready(RelayCrypto),
}

pub(crate) struct RelaySession {
    pub relay_id: Uuid,
    pub peer: Uuid,
    pub crypto: SessionCrypto,
    /// Our ephemeral public half, kept after completion: the handshake
    /// advance (re)sends it until the peer acknowledges keys.
    pub eph_pub: [u8; 32],
    pub opened_by_me: bool,
    pub accepted_sent: bool,
    pub keys_sent: bool,
    /// Frames that arrived before completion (peer seals when *it* is
    /// ready). Drained in order on completion; see `drain_inbox`.
    pub inbox: VecDeque<(u64, String)>,
    /// Transfer ids already announced on this session. Announce-once per
    /// session keeps the bounded server queue for real traffic.
    pub offered: std::collections::HashSet<String>,
}

/// Sessions keyed by peer device (one live session per peer; the server
/// supersedes same-pair dupes anyway), plus open-attempt cooldowns so a
/// failing server is probed politely, never hammered.
#[derive(Default)]
pub(crate) struct RelaySessions {
    // BTreeMap: first-ready selection stays deterministic across the check
    // and the send (same order for every caller in one pump).
    sessions: BTreeMap<Uuid, RelaySession>,
    last_open_attempt: BTreeMap<Uuid, Instant>,
    throttle_window: Option<Instant>,
    throttle_bytes: u64,
    /// Test seam: bypasses the uplink budget (loopback rigs have no uplink).
    /// Production always leaves this `None`.
    throttle_override: Option<u64>,
}

impl RelaySessions {
    pub fn session_for_peer(&self, peer: &Uuid) -> Option<&RelaySession> {
        self.sessions.get(peer)
    }

    pub fn session_for_peer_mut(&mut self, peer: &Uuid) -> Option<&mut RelaySession> {
        self.sessions.get_mut(peer)
    }

    pub fn session_for_id(&self, relay_id: &Uuid) -> Option<&RelaySession> {
        self.sessions.values().find(|s| &s.relay_id == relay_id)
    }

    pub fn session_for_id_mut(&mut self, relay_id: &Uuid) -> Option<&mut RelaySession> {
        self.sessions.values_mut().find(|s| &s.relay_id == relay_id)
    }

    pub fn insert_if_absent(&mut self, session: RelaySession) -> bool {
        if self.sessions.contains_key(&session.peer)
            || self.session_for_id(&session.relay_id).is_some()
        {
            return false;
        }
        self.sessions.insert(session.peer, session);
        true
    }

    pub fn remove_by_id(&mut self, relay_id: &Uuid) -> Option<RelaySession> {
        let peer = self.session_for_id(relay_id)?.peer;
        self.sessions.remove(&peer)
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// Drains the pre-completion inbox in arrival order for the caller to
    /// open through the now-ready crypto (strict sequence still enforced
    /// per frame; a gap stops nothing here — unopenable frames simply fail
    /// at open like any other).
    pub fn drain_inbox(&mut self, relay_id: &Uuid) -> Vec<(u64, String)> {
        match self.session_for_id_mut(relay_id) {
            Some(session) => std::mem::take(&mut session.inbox).into_iter().collect(),
            None => Vec::new(),
        }
    }

    pub fn ready_peers(&self) -> Vec<Uuid> {
        self.sessions
            .values()
            .filter(|s| matches!(s.crypto, SessionCrypto::Ready(_)))
            .map(|s| s.peer)
            .collect()
    }

    pub fn sessions_snapshot(&self) -> Vec<(Uuid, Uuid)> {
        self.sessions
            .values()
            .map(|s| (s.peer, s.relay_id))
            .collect()
    }

    /// Completes the handshake for `relay_id` with the peer's public half.
    /// Already-ready sessions and unknown ids report false (the caller
    /// stages acceptor sessions for paired peers, drops anything else).
    pub fn complete_with(&mut self, relay_id: &Uuid, from: &Uuid, public: [u8; 32]) -> bool {
        let Some(session) = self.session_for_id_mut(relay_id) else {
            return false;
        };
        if &session.peer != from || !matches!(session.crypto, SessionCrypto::Pending(_)) {
            return false;
        }
        let ready = match &session.crypto {
            SessionCrypto::Pending(pending) => {
                pending.complete(public, *relay_id, session.opened_by_me)
            }
            SessionCrypto::Ready(_) => return true,
        };
        match ready {
            Ok(crypto) => {
                session.crypto = SessionCrypto::Ready(crypto);
                true
            }
            Err(_) => false,
        }
    }

    pub fn open_cooldown_ok(&self, peer: &Uuid, now: Instant) -> bool {
        self.last_open_attempt
            .get(peer)
            .is_none_or(|at| now.duration_since(*at) >= Duration::from_secs(10))
    }

    pub fn note_open_attempt(&mut self, peer: Uuid, now: Instant) {
        self.last_open_attempt.insert(peer, now);
    }

    /// Token bucket for relay sends: at most the budget per one-second
    /// window, shared across sessions. Refusals simply defer to a later
    /// pump (board `pending` retains the chunk); no data is ever dropped
    /// for pacing.
    pub fn throttle_try_consume(&mut self, bytes: usize) -> bool {
        let budget = self
            .throttle_override
            .unwrap_or(RELAY_THROTTLE_BYTES_PER_SEC);
        let now = Instant::now();
        match self.throttle_window {
            Some(start) if now.duration_since(start) < Duration::from_secs(1) => {}
            _ => {
                self.throttle_window = Some(now);
                self.throttle_bytes = 0;
            }
        }
        if self.throttle_bytes.saturating_add(bytes as u64) > budget {
            return false;
        }
        self.throttle_bytes += bytes as u64;
        true
    }

    /// Test seam for loopback rigs (see `throttle_override`).
    #[cfg(test)]
    pub fn set_throttle_override(&mut self, budget: Option<u64>) {
        self.throttle_override = budget;
    }
}

/// Whether relay fallback is wanted right now: chat waiting, bearer down,
/// worker path unavailable. Pure policy, unit-tested; the pump supplies the
/// three facts.
pub(crate) fn relay_wanted(chat_queued: usize, bearer_live: bool, worker_covers: bool) -> bool {
    chat_queued > 0 && !bearer_live && !worker_covers
}

/// The poll worker: one persistent event connection parked in `relay.poll`,
/// emitting deliveries. Thin by design — policy lives on the pump; this loop
/// only connects, polls, parses, and backs off.
pub(crate) struct RelayPollWorker {
    events_tx: Option<mpsc::Sender<RelayEvent>>,
    events_rx: Option<mpsc::Receiver<RelayEvent>>,
    worker: Option<std::thread::JoinHandle<()>>,
    shutdown_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    idle_since: Option<Instant>,
}

impl RelayPollWorker {
    pub fn new() -> Self {
        let (events_tx, events_rx) = mpsc::channel();
        Self {
            events_tx: Some(events_tx),
            events_rx: Some(events_rx),
            worker: None,
            shutdown_flag: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            idle_since: None,
        }
    }

    /// Runs the worker while sessions live. Safe to call every pump:
    /// starting is idempotent. A stopped worker must exit before replacement,
    /// so restarting cannot leave two destructive pollers alive.
    pub fn ensure_worker(&mut self, pin: &ServerPin, state_dir: &Path) {
        self.idle_since = None;
        self.ensure(pin.clone(), state_dir.to_path_buf());
    }

    /// Parks the worker after [`WORKER_IDLE_GRACE`] without sessions. Call
    /// every pump while session-less; a no-op once stopped.
    pub fn idle_stop(&mut self) {
        if self.worker.is_none() {
            return;
        }
        let now = Instant::now();
        match self.idle_since {
            Some(since) if now.duration_since(since) >= WORKER_IDLE_GRACE => self.stop(),
            None => self.idle_since = Some(now),
            _ => {}
        }
    }

    fn ensure(&mut self, pin: ServerPin, state_dir: PathBuf) {
        if let Some(worker) = self.worker.as_ref() {
            if !worker.is_finished() {
                return;
            }
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
        let Some(events_tx) = self.events_tx.clone() else {
            return;
        };
        self.shutdown_flag
            .store(false, std::sync::atomic::Ordering::SeqCst);
        let shutdown = std::sync::Arc::clone(&self.shutdown_flag);
        self.worker = Some(std::thread::spawn(move || {
            run_poll_loop(pin, state_dir, events_tx, shutdown)
        }));
    }

    pub fn drain(&mut self) -> Vec<RelayEvent> {
        let mut events = Vec::new();
        if let Some(rx) = self.events_rx.as_ref() {
            events.extend(rx.try_iter());
        }
        events
    }

    pub fn stop(&mut self) {
        self.shutdown_flag
            .store(true, std::sync::atomic::Ordering::SeqCst);
        self.idle_since = None;
    }
}

impl Drop for RelayPollWorker {
    fn drop(&mut self) {
        self.stop();
    }
}

fn shutdown_requested(shutdown: &std::sync::Arc<std::sync::atomic::AtomicBool>) -> bool {
    shutdown.load(std::sync::atomic::Ordering::SeqCst)
}

/// Sleeps `wait` in slices so a stop request lands within 100 ms instead of
/// after the whole backoff. Returns true when asked to stop mid-sleep.
fn sleep_slices(wait: Duration, shutdown: &std::sync::Arc<std::sync::atomic::AtomicBool>) -> bool {
    let deadline = Instant::now() + wait;
    loop {
        if shutdown_requested(shutdown) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn run_poll_loop(
    pin: ServerPin,
    state_dir: PathBuf,
    events: mpsc::Sender<RelayEvent>,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let mut attempt = 0_u32;
    loop {
        if shutdown_requested(&shutdown) {
            return;
        }
        let identity = match load_or_create(&state_dir, now_seconds()) {
            Ok(identity) => identity,
            Err(_) => {
                if sleep_slices(backoff_delay(attempt), &shutdown) {
                    return;
                }
                attempt = attempt.saturating_add(1);
                continue;
            }
        };
        let mut client = match ServerClient::connect(&pin) {
            Ok(client) => client,
            Err(_) => {
                if sleep_slices(backoff_delay(attempt), &shutdown) {
                    return;
                }
                attempt = attempt.saturating_add(1);
                continue;
            }
        };
        attempt = 0;
        loop {
            if shutdown_requested(&shutdown) {
                return;
            }
            let request =
                harbor_protocol::Envelope::request("relay.poll", json!({}), rfc3339_now());
            match client.exchange(request, &identity) {
                Ok(response) => {
                    // An error answer (unauthorized and friends) is not a
                    // parked poll: drop the connection and back off instead
                    // of hot-spinning refusals.
                    if response.error.is_some() {
                        break;
                    }
                    if let Some(event) = parse_poll_event(&response.payload) {
                        let _ = events.send(event);
                    }
                }
                Err(_) => break,
            }
        }
        if sleep_slices(backoff_delay(attempt), &shutdown) {
            return;
        }
        attempt = attempt.saturating_add(1);
    }
}

fn backoff_delay(attempt: u32) -> Duration {
    Duration::from_secs(reconnect_delay(attempt))
}

/// Parses one poll answer into an event. Malformed entries are skipped, not
/// fatal: the next poll re-delivers anything real. All-empty answers (hold
/// timeouts) produce no event at all.
fn parse_poll_event(payload: &Value) -> Option<RelayEvent> {
    let mut frames = Vec::new();
    if let Some(list) = payload.get("frames").and_then(Value::as_array) {
        for entry in list {
            let frame = (|| {
                Some(WireFrame {
                    relay_id: entry.get("relay_id")?.as_str()?.parse().ok()?,
                    from: entry.get("from")?.as_str()?.parse().ok()?,
                    seq: entry.get("seq")?.as_u64()?,
                    bytes: entry.get("bytes")?.as_str()?.to_owned(),
                })
            })();
            if let Some(frame) = frame {
                frames.push(frame);
            }
        }
    }
    let mut closed = Vec::new();
    if let Some(list) = payload.get("closed").and_then(Value::as_array) {
        for entry in list {
            if let Some(id) = entry.as_str().and_then(|text| text.parse().ok()) {
                closed.push(id);
            }
        }
    }
    let mut opens = Vec::new();
    if let Some(list) = payload.get("opens").and_then(Value::as_array) {
        for entry in list {
            let notice = (|| {
                Some(WireOpen {
                    relay_id: entry.get("relay_id")?.as_str()?.parse().ok()?,
                    from: entry.get("from")?.as_str()?.parse().ok()?,
                    purpose: entry.get("purpose")?.as_str()?.to_owned(),
                })
            })();
            if let Some(notice) = notice {
                opens.push(notice);
            }
        }
    }
    if frames.is_empty() && closed.is_empty() && opens.is_empty() {
        return None;
    }
    Some(RelayEvent::Polled {
        frames,
        closed,
        opens,
    })
}

/// Our ephemeral public half, hexed like certificate fingerprints, for
/// `relay.open` payloads and `relay_keys` signals.
pub(crate) fn eph_pub_hex(public: &[u8; 32]) -> String {
    fingerprint_hex(public)
}

pub(crate) fn parse_eph_pub(raw: &str) -> Option<[u8; 32]> {
    parse_fingerprint_hex(raw)
}

/// Seals one bearer-protocol frame (`{kind, payload}` JSON) for `session`.
/// Returns the base64 ciphertext ready for `relay.data`, or `None` when the
/// session cannot send (not ready, oversize, counter spent).
pub(crate) fn seal_frame(
    session: &mut RelaySession,
    kind: &str,
    payload: &Value,
) -> Option<(u64, String)> {
    let crypto = match &mut session.crypto {
        SessionCrypto::Ready(crypto) => crypto,
        SessionCrypto::Pending(_) => return None,
    };
    let plaintext = serde_json::to_vec(&json!({"kind": kind, "payload": payload})).ok()?;
    let sealed = crypto.seal(&plaintext).ok()?;
    use base64::Engine as _;
    Some((
        sealed.seq,
        base64::engine::general_purpose::STANDARD_NO_PAD.encode(&sealed.ciphertext),
    ))
}

/// Opens sealed relay bytes back into a bearer frame. Strict sequence,
/// authentication, and shape are all enforced; anything else is `None`.
pub(crate) fn open_frame(
    session: &mut RelaySession,
    seq: u64,
    bytes: &str,
) -> Option<(String, Value)> {
    let crypto = match &mut session.crypto {
        SessionCrypto::Ready(crypto) => crypto,
        SessionCrypto::Pending(_) => return None,
    };
    use base64::Engine as _;
    let ciphertext = base64::engine::general_purpose::STANDARD_NO_PAD
        .decode(bytes)
        .ok()?;
    let plaintext = crypto.open(seq, &ciphertext).ok()?;
    let value: Value = serde_json::from_slice(&plaintext).ok()?;
    let kind = value.get("kind")?.as_str()?.to_owned();
    let payload = value.get("payload")?.clone();
    if kind.is_empty() || kind.len() > 64 || !payload.is_object() {
        return None;
    }
    Some((kind, payload))
}

#[cfg(test)]
mod tests {
    use super::super::relay_crypto::PendingCrypto;
    use super::*;

    fn pending_session(peer: Uuid) -> RelaySession {
        let pending = PendingCrypto::generate();
        let eph_pub = pending.public_bytes();
        RelaySession {
            relay_id: Uuid::new_v4(),
            peer,
            crypto: SessionCrypto::Pending(pending),
            eph_pub,
            opened_by_me: true,
            accepted_sent: false,
            keys_sent: false,
            inbox: Default::default(),
            offered: Default::default(),
        }
    }

    #[test]
    fn sessions_track_peers_and_readiness() {
        let mut sessions = RelaySessions::default();
        let peer = Uuid::new_v4();
        assert!(sessions.session_for_peer(&peer).is_none());
        assert!(sessions.ready_peers().is_empty());
        assert!(sessions.insert_if_absent(pending_session(peer)));
        assert!(sessions.session_for_peer(&peer).is_some());
        assert!(sessions.ready_peers().is_empty());
        assert!(sessions.session_for_id(&Uuid::new_v4()).is_none());
        let id = sessions.session_for_peer(&peer).unwrap().relay_id;
        assert!(sessions.session_for_id(&id).is_some());
        assert!(sessions.remove_by_id(&id).is_some());
        assert!(sessions.is_empty());
    }

    #[test]
    fn stale_sessions_cannot_replace_or_remove_the_current_peer_session() {
        let mut sessions = RelaySessions::default();
        let peer = Uuid::new_v4();
        let current = pending_session(peer);
        let current_id = current.relay_id;
        assert!(sessions.insert_if_absent(current));

        let stale = pending_session(peer);
        let stale_id = stale.relay_id;
        assert!(!sessions.insert_if_absent(stale));
        assert_eq!(
            sessions.session_for_peer(&peer).unwrap().relay_id,
            current_id
        );
        assert!(sessions.remove_by_id(&stale_id).is_none());
        assert_eq!(
            sessions.session_for_peer(&peer).unwrap().relay_id,
            current_id
        );
    }

    #[test]
    fn open_cooldown_paces_failing_servers() {
        let mut sessions = RelaySessions::default();
        let peer = Uuid::new_v4();
        let now = Instant::now();
        assert!(sessions.open_cooldown_ok(&peer, now));
        sessions.note_open_attempt(peer, now);
        assert!(!sessions.open_cooldown_ok(&peer, now));
        assert!(sessions.open_cooldown_ok(&peer, now + Duration::from_secs(11)));
    }

    #[test]
    fn pending_sessions_seal_nothing_until_keys_complete() {
        let peer = Uuid::new_v4();
        let mut session = pending_session(peer);
        assert!(seal_frame(&mut session, "chat", &json!({"id": "a"})).is_none());
        assert!(open_frame(&mut session, 0, "eA").is_none());
    }

    #[test]
    fn sealed_frames_round_trip_through_a_completed_pair() {
        let relay_id = Uuid::new_v4();
        let opener_id = Uuid::new_v4();
        let peer_id = Uuid::new_v4();
        let opener_pending = PendingCrypto::generate();
        let acceptor_pending = PendingCrypto::generate();
        let opener_pub = opener_pending.public_bytes();
        let acceptor_pub = acceptor_pending.public_bytes();

        let mut opener = pending_session(peer_id);
        opener.relay_id = relay_id;
        opener.crypto = SessionCrypto::Ready(
            opener_pending
                .complete(acceptor_pub, relay_id, true)
                .unwrap(),
        );
        let mut acceptor = RelaySession {
            relay_id,
            peer: opener_id,
            crypto: SessionCrypto::Ready(
                acceptor_pending
                    .complete(opener_pub, relay_id, false)
                    .unwrap(),
            ),
            eph_pub: acceptor_pub,
            opened_by_me: false,
            accepted_sent: true,
            keys_sent: true,
            inbox: Default::default(),
            offered: Default::default(),
        };

        let (seq, bytes) = seal_frame(&mut opener, "chat", &json!({"id": "m1"})).unwrap();
        assert_eq!(seq, 0);
        let (kind, payload) = open_frame(&mut acceptor, seq, &bytes).unwrap();
        assert_eq!(kind, "chat");
        assert_eq!(payload["id"], json!("m1"));
        assert!(open_frame(&mut acceptor, seq, &bytes).is_none());
    }

    #[test]
    fn poll_parsing_skips_malformed_entries_and_empty_answers() {
        assert!(
            parse_poll_event(&json!({"frames": [], "closed": [], "opens": [], "timeout": true}))
                .is_none()
        );
        let event = parse_poll_event(&json!({
            "frames": [
                {"relay_id": Uuid::new_v4(), "from": Uuid::new_v4(), "seq": 0, "bytes": "eA"},
                {"relay_id": "junk", "from": 7, "seq": "x"},
            ],
            "closed": [Uuid::new_v4(), 42],
            "opens": [
                {"relay_id": Uuid::new_v4(), "from": Uuid::new_v4(), "purpose": "chat"},
                {"relay_id": "junk"},
            ],
            "timeout": false,
        }))
        .expect("one good entry of each kind suffices");
        match event {
            RelayEvent::Polled {
                frames,
                closed,
                opens,
            } => {
                assert_eq!(frames.len(), 1);
                assert_eq!(closed.len(), 1);
                assert_eq!(opens.len(), 1);
            }
        }
    }

    #[test]
    fn relay_is_wanted_only_for_queued_chat_without_a_path() {
        assert!(relay_wanted(3, false, false));
        assert!(!relay_wanted(0, false, false));
        assert!(!relay_wanted(3, true, false));
        assert!(!relay_wanted(3, false, true));
    }

    #[test]
    fn throttle_budgets_and_overrides() {
        let mut sessions = RelaySessions::default();
        for _ in 0..16 {
            assert!(sessions.throttle_try_consume(16 * 1024));
        }
        assert!(!sessions.throttle_try_consume(16 * 1024));
        sessions.set_throttle_override(Some(u64::MAX));
        assert!(sessions.throttle_try_consume(16 * 1024));
    }
}
