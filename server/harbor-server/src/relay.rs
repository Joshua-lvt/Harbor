//! Relay fallback state: opaque store-and-forward between paired devices
//! over their existing outbound connections.
//!
//! Transport-independent like [`harbor_control::ControlPlane`]: no sockets,
//! no clock inside (callers pass `now`), no panics on any input. The server
//! routes by session id, sizes frames, and drops — it never parses,
//! interprets, or retains payloads beyond the bounded per-session queue.
//! End-to-end encryption lives on the clients; these bytes are already
//! ciphertext here.
//!
//! Delivery is pull-based: recipients drain due frames on their own
//! connection loop, so the table never owns sockets, channels, or threads.
//! A device with nothing parked simply accumulates (bounded) until its next
//! poll; overflow refuses instead of growing memory.

use std::collections::{HashMap, VecDeque};

use harbor_control::ControlPlane;
use serde::Serialize;
use uuid::Uuid;

/// Live relay sessions per listener. A handful of paired devices is the
/// entire expected population; the bound keeps one chatty pair from starving
/// the phone hosting the server.
pub const MAX_RELAY_SESSIONS: usize = 4;
/// Per-session pending frames per direction aggregate. Overflow refuses new
/// frames (`QueueFull`) instead of growing memory: chat requeues on the
/// client, voice drops (RTP-tolerant), files pause production.
pub const MAX_PENDING_PER_SESSION: usize = 8;
/// Largest `relay.data` payload in JSON string chars. Fits 32 KiB of
/// ciphertext as base64 with margin; anything larger is not a relay frame.
pub const MAX_RELAY_DATA_CHARS: usize = 48 * 1024;
/// The acceptor has this long to answer an open before it expires.
pub const RELAY_ACCEPT_WINDOW_SECS: u64 = 10;
/// Silence in either direction past this closes the session.
pub const RELAY_IDLE_TIMEOUT_SECS: u64 = 60;
/// Absolute session lifetime. Rekeying opens a fresh session; nothing lives
/// forever on stale keys.
pub const RELAY_SESSION_TTL_SECS: u64 = 4 * 3600;
/// Bytes forwarded per session, all directions summed. Past this the server
/// refuses with `SessionExhausted`: the client opens a fresh session and
/// resumes at file offsets (transfer state is transport-free), so a large
/// file crosses sessions instead of growing one without bound.
pub const MAX_SESSION_BYTES: u64 = 256 << 20;
/// Cap for one poll answer, under the 256 KiB network frame with JSON
/// overhead to spare.
pub const POLL_MAX_BYTES: usize = 200 * 1024;
/// Closed-session notices retained per offline device. Old relay ids are
/// informational only; bounding them prevents an unsendable poll response.
pub const MAX_PENDING_CLOSED_NOTICES: usize = 16;
/// Recent closed sessions retained only to make duplicate close requests
/// idempotent. Older tombstones are safe to forget once this bound is hit.
pub const MAX_CLOSED_SESSION_TOMBSTONES: usize = 32;
/// Rate ceiling per session, per one-second window. The normative ~2 Mbps
/// flow budget stays client-enforced (throttle); these are the server's
/// safety net against a paired-but-runaway client, so they sit well above
/// legitimate traffic (256 KiB/s throttle + per-chunk margin) and only trip
/// on sustained abuse.
pub const RELAY_SESSION_MAX_FRAMES_PER_SEC: u64 = 512;
pub const RELAY_SESSION_MAX_BYTES_PER_SEC: u64 = 1024 * 1024;
/// Circuit breaker, whole table, per one-second window. Tripping refuses
/// pushes AND new opens (both map to `relay_busy`, which clients already
/// retry with backoff) while the control plane keeps answering untouched —
/// the relay degrades before the control plane by construction.
pub const RELAY_GLOBAL_MAX_FRAMES_PER_SEC: u64 = 2048;
pub const RELAY_GLOBAL_MAX_BYTES_PER_SEC: u64 = 4 * 1024 * 1024;

/// One fixed one-second window. Second-granularity `now` is the clock every
/// caller already passes; a fixed window is enough because the caps target
/// sustained abuse, not microbursts (the 8-deep pull queue already bounds
/// those).
#[derive(Debug, Clone, Copy, Default)]
struct RateWindow {
    second: u64,
    frames: u64,
    bytes: u64,
}

impl RateWindow {
    /// Records one frame if it fits both caps; refuses (without recording)
    /// when either would overflow. Refusal is pre-enqueue by construction,
    /// so `relay_busy` unambiguously tells the client to retire the session.
    fn admit(
        &mut self,
        now: u64,
        frames_cap: u64,
        bytes_cap: u64,
        frame_bytes: u64,
    ) -> Result<(), RateVerdict> {
        if self.second != now {
            self.second = now;
            self.frames = 0;
            self.bytes = 0;
        }
        if self.frames >= frames_cap {
            return Err(RateVerdict::Frames);
        }
        if self.bytes.saturating_add(frame_bytes) > bytes_cap {
            return Err(RateVerdict::Bytes);
        }
        self.frames += 1;
        self.bytes = self.bytes.saturating_add(frame_bytes);
        Ok(())
    }
}

/// Which cap tripped — only for the metadata log line, never the payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RateVerdict {
    Frames,
    Bytes,
}

/// What the relay session carries. Routing and limits do not depend on it;
/// delivery granularity does (media sessions get a tighter poll tick).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum RelayPurpose {
    Chat,
    File,
    Media,
}

impl RelayPurpose {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "chat" => Some(RelayPurpose::Chat),
            "file" => Some(RelayPurpose::File),
            "media" => Some(RelayPurpose::Media),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelayState {
    Opening,
    Open,
    Closed,
}

#[derive(Debug, Clone)]
struct RelaySession {
    id: Uuid,
    opener: Uuid,
    peer: Uuid,
    purpose: RelayPurpose,
    state: RelayState,
    created_at: u64,
    last_active: u64,
    /// Next accepted sequence per direction: index 0 is opener→peer,
    /// 1 is peer→opener. Strictly increasing: a replayed or reordered frame
    /// is refused instead of forwarded.
    expected_seq: [u64; 2],
    bytes_forwarded: u64,
    /// Per-session rate window (Phase 4.1).
    rate: RateWindow,
}

impl RelaySession {
    fn members(&self) -> [Uuid; 2] {
        [self.opener, self.peer]
    }

    fn is_member(&self, device: Uuid) -> bool {
        device == self.opener || device == self.peer
    }

    fn direction(&self, from: Uuid) -> Option<usize> {
        if from == self.opener {
            Some(0)
        } else if from == self.peer {
            Some(1)
        } else {
            None
        }
    }

    fn is_live(&self) -> bool {
        matches!(self.state, RelayState::Opening | RelayState::Open)
    }
}

/// One queued opaque frame: ciphertext plus the routing facts around it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelayFrame {
    pub from: Uuid,
    pub seq: u64,
    pub bytes: String,
    pub enqueued_at: u64,
}

/// An unaccepted open waiting on the peer, delivered once via its poll.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RelayOpenNotice {
    pub relay_id: Uuid,
    pub from: Uuid,
    pub purpose: RelayPurpose,
}

/// What one poll answer carries: due frames, sessions that closed since the
/// last answer, and opens awaiting acceptance.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RelayDelivery {
    pub frames: Vec<DeliveredFrame>,
    pub closed: Vec<Uuid>,
    pub opens: Vec<RelayOpenNotice>,
}

/// One frame routed to its recipient, tagged with its session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveredFrame {
    pub relay_id: Uuid,
    pub from: Uuid,
    pub seq: u64,
    pub bytes: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelayFault {
    UnknownSession,
    NotMember,
    ClosedSession,
    OutOfOrder,
    QueueFull,
    TooManySessions,
    SelfRelay,
    OversizedFrame,
    AcceptExpired,
    InvalidPayload,
    SessionExhausted,
    /// Per-session rate ceiling tripped (Phase 4.1). Pre-enqueue refusal.
    RateLimited,
    /// Global circuit breaker tripped (Phase 4.1): pushes AND new opens
    /// refuse while the control plane keeps serving.
    Overloaded,
}

impl RelayFault {
    /// Protocol surface: stable code, UI key, and whether the client should
    /// retry the same request. `ClosedSession` is not retryable because the
    /// fix is a new session (`relay.open`), not the same frame again; only
    /// `QueueFull`/`TooManySessions` ask for a later retry.
    pub fn code(&self) -> &'static str {
        match self {
            // Unknown means the server holds no such session (restart drops
            // them by design): the client opens a fresh one instead of
            // retrying the dead frame. Closed is the same cure for an
            // expired, superseded, hung-up, or byte-exhausted session.
            // OutOfOrder gets its own code (not a generic shape error): the
            // client drops and reopens, since only a fresh counter pair
            // resyncs after a maybe-delivered gap.
            RelayFault::UnknownSession => "relay_unknown",
            RelayFault::ClosedSession => "relay_closed",
            RelayFault::OutOfOrder => "relay_order",
            RelayFault::SessionExhausted => "relay_exhausted",
            RelayFault::OversizedFrame
            | RelayFault::AcceptExpired
            | RelayFault::InvalidPayload
            | RelayFault::SelfRelay => "invalid_request",
            RelayFault::NotMember => "unauthorized",
            // Rate caps and the breaker answer `relay_busy` like a full
            // queue: the client provably rolled nothing forward (refusal is
            // pre-enqueue) and retries the same frame with its own backoff.
            RelayFault::QueueFull
            | RelayFault::TooManySessions
            | RelayFault::RateLimited
            | RelayFault::Overloaded => "relay_busy",
        }
    }

    pub fn ui_key(&self) -> &'static str {
        match self {
            RelayFault::NotMember => "error.server.unauthorized",
            RelayFault::QueueFull
            | RelayFault::TooManySessions
            | RelayFault::RateLimited
            | RelayFault::Overloaded
            | RelayFault::SessionExhausted => "error.relay.unavailable",
            _ => "error.protocol.invalidRequest",
        }
    }

    pub fn retryable(&self) -> bool {
        matches!(
            self,
            RelayFault::QueueFull
                | RelayFault::TooManySessions
                | RelayFault::RateLimited
                | RelayFault::Overloaded
        )
    }
}

/// Relay sessions, bounded queues, and per-device notices. All methods take
/// the caller-verified facts as parameters; authentication and pairing live
/// with the caller (transport), exactly like `ControlPlane` stays ignorant
/// of sockets.
#[derive(Debug, Default)]
pub struct RelayTable {
    sessions: HashMap<Uuid, RelaySession>,
    pending: HashMap<Uuid, VecDeque<RelayFrame>>,
    opens: HashMap<Uuid, Vec<RelayOpenNotice>>,
    closed_notify: HashMap<Uuid, Vec<Uuid>>,
    /// Whole-table breaker window (Phase 4.1). One log line per tripped
    /// second, metadata only.
    global_rate: RateWindow,
    breaker_logged_second: u64,
}

impl RelayTable {
    pub fn new() -> Self {
        Self::default()
    }

    fn live_count(&self) -> usize {
        self.sessions.values().filter(|s| s.is_live()).count()
    }

    fn note_closed(&mut self, device: Uuid, id: Uuid) {
        let notices = self.closed_notify.entry(device).or_default();
        if notices.contains(&id) {
            return;
        }
        if notices.len() >= MAX_PENDING_CLOSED_NOTICES {
            notices.remove(0);
        }
        notices.push(id);
    }

    fn close_locked(&mut self, id: Uuid) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        if session.state == RelayState::Closed {
            return;
        }
        session.state = RelayState::Closed;
        let members = session.members();
        let peer = session.peer;
        self.pending.remove(&id);
        for member in members {
            self.note_closed(member, id);
        }
        if let Some(list) = self.opens.get_mut(&peer) {
            list.retain(|notice| notice.relay_id != id);
        }
        while self
            .sessions
            .values()
            .filter(|session| session.state == RelayState::Closed)
            .count()
            > MAX_CLOSED_SESSION_TOMBSTONES
        {
            let oldest = self
                .sessions
                .values()
                .filter(|session| session.state == RelayState::Closed)
                .min_by_key(|session| (session.created_at, session.id))
                .map(|session| session.id);
            if let Some(oldest) = oldest {
                self.sessions.remove(&oldest);
            }
        }
    }

    /// Opens a relay session from `opener` to `peer`. The caller verified
    /// authentication and pairing; this enforces relay-local rules. Only the
    /// newest session per pair stays live: opening supersedes an older one
    /// (rekeying, reconnect races), so duplicates can never accumulate.
    pub fn open(
        &mut self,
        opener: Uuid,
        peer: Uuid,
        purpose: RelayPurpose,
        now: u64,
    ) -> Result<Uuid, RelayFault> {
        if opener == peer {
            return Err(RelayFault::SelfRelay);
        }
        // Breaker first: while the table is over its global budget, new
        // sessions add nothing but load. The control plane never sees this
        // gate — relay degrades before control by construction.
        if self
            .global_rate
            .admit(
                now,
                RELAY_GLOBAL_MAX_FRAMES_PER_SEC,
                RELAY_GLOBAL_MAX_BYTES_PER_SEC,
                0,
            )
            .is_err()
        {
            self.note_breaker(now, None);
            return Err(RelayFault::Overloaded);
        }
        if self.live_count() >= MAX_RELAY_SESSIONS {
            return Err(RelayFault::TooManySessions);
        }
        let stale: Vec<Uuid> = self
            .sessions
            .values()
            .filter(|s| s.is_live() && pair_key(s.opener, s.peer) == pair_key(opener, peer))
            .map(|s| s.id)
            .collect();
        for id in stale {
            self.close_locked(id);
        }
        let id = Uuid::new_v4();
        self.sessions.insert(
            id,
            RelaySession {
                id,
                opener,
                peer,
                purpose,
                state: RelayState::Opening,
                created_at: now,
                last_active: now,
                expected_seq: [0, 0],
                bytes_forwarded: 0,
                rate: RateWindow::default(),
            },
        );
        self.opens.entry(peer).or_default().push(RelayOpenNotice {
            relay_id: id,
            from: opener,
            purpose,
        });
        Ok(id)
    }

    /// Accepts an open. Only the peer (never the opener) accepts, inside the
    /// accept window; accepting twice is idempotent for retried polls.
    pub fn accept(&mut self, id: Uuid, by: Uuid, now: u64) -> Result<(), RelayFault> {
        let session = self.sessions.get(&id).ok_or(RelayFault::UnknownSession)?;
        if !session.is_member(by) {
            return Err(RelayFault::NotMember);
        }
        if by != session.peer {
            return Err(RelayFault::NotMember);
        }
        match session.state {
            RelayState::Open => return Ok(()),
            RelayState::Closed => return Err(RelayFault::ClosedSession),
            RelayState::Opening => {}
        }
        if now > session.created_at.saturating_add(RELAY_ACCEPT_WINDOW_SECS) {
            self.close_locked(id);
            return Err(RelayFault::AcceptExpired);
        }
        let session = self.sessions.get_mut(&id).expect("session checked");
        session.state = RelayState::Open;
        session.last_active = now;
        if let Some(list) = self.opens.get_mut(&by) {
            list.retain(|notice| notice.relay_id != id);
        }
        Ok(())
    }

    /// Queues one opaque frame. Membership, liveness, size, and strict
    /// sequence are validated first; a full queue refuses instead of growing.
    pub fn push(
        &mut self,
        id: Uuid,
        from: Uuid,
        seq: u64,
        bytes: String,
        now: u64,
    ) -> Result<(), RelayFault> {
        let session = self.sessions.get(&id).ok_or(RelayFault::UnknownSession)?;
        let Some(direction) = session.direction(from) else {
            return Err(RelayFault::NotMember);
        };
        if session.state != RelayState::Open {
            return Err(RelayFault::ClosedSession);
        }
        if bytes.len() > MAX_RELAY_DATA_CHARS {
            return Err(RelayFault::OversizedFrame);
        }
        if session.bytes_forwarded.saturating_add(bytes.len() as u64) > MAX_SESSION_BYTES {
            return Err(RelayFault::SessionExhausted);
        }
        if seq != session.expected_seq[direction] {
            return Err(RelayFault::OutOfOrder);
        }
        // The queue bound is also a pre-enqueue refusal. Check it before
        // charging either rate window; repeated retries of a full queue
        // must not manufacture rate-limit traffic that never forwarded.
        if self
            .pending
            .get(&id)
            .is_some_and(|queue| queue.len() >= MAX_PENDING_PER_SESSION)
        {
            return Err(RelayFault::QueueFull);
        }
        // Phase 4.1 load shedding: the global breaker first (a refused frame
        // must charge nothing), then the per-session ceiling — rolling the
        // global charge back if the session refuses, so a refused frame
        // never double-charges anywhere. Both refuse BEFORE the enqueue, so
        // the client can retire the session without ambiguous delivery, and
        // neither advances the sequence counter on refusal.
        if self
            .global_rate
            .admit(
                now,
                RELAY_GLOBAL_MAX_FRAMES_PER_SEC,
                RELAY_GLOBAL_MAX_BYTES_PER_SEC,
                bytes.len() as u64,
            )
            .is_err()
        {
            self.note_breaker(now, Some((id, bytes.len() as u64)));
            return Err(RelayFault::Overloaded);
        }
        {
            let session = self.sessions.get_mut(&id).expect("session checked");
            if session
                .rate
                .admit(
                    now,
                    RELAY_SESSION_MAX_FRAMES_PER_SEC,
                    RELAY_SESSION_MAX_BYTES_PER_SEC,
                    bytes.len() as u64,
                )
                .is_err()
            {
                // Undo the global charge taken above: `admit` succeeded in
                // this very second, so the rollback is exact.
                self.global_rate.frames -= 1;
                self.global_rate.bytes = self.global_rate.bytes.saturating_sub(bytes.len() as u64);
                return Err(RelayFault::RateLimited);
            }
        }
        let queue = self.pending.entry(id).or_default();
        queue.push_back(RelayFrame {
            from,
            seq,
            bytes: bytes.clone(),
            enqueued_at: now,
        });
        let session = self.sessions.get_mut(&id).expect("session checked");
        session.expected_seq[direction] = seq.saturating_add(1);
        session.last_active = now;
        session.bytes_forwarded = session.bytes_forwarded.saturating_add(bytes.len() as u64);
        Ok(())
    }

    /// One metadata-only log line per tripped second: counts, timestamp,
    /// and (for pushes) the relay id and refused frame size. Never the
    /// payload — the server cannot log what it cannot read.
    fn note_breaker(&mut self, now: u64, refused: Option<(Uuid, u64)>) {
        if self.breaker_logged_second == now {
            return;
        }
        self.breaker_logged_second = now;
        match refused {
            Some((relay_id, frame_bytes)) => eprintln!(
                "harbor-server: relay breaker tripped (second={now} frames={} bytes={} refused relay_id={relay_id} frame_bytes={frame_bytes})",
                self.global_rate.frames, self.global_rate.bytes
            ),
            None => eprintln!(
                "harbor-server: relay breaker tripped (second={now} frames={} bytes={} refused open)",
                self.global_rate.frames, self.global_rate.bytes
            ),
        }
    }

    /// Closes a session from either member. Idempotent: closing twice (both
    /// sides hanging up) is fine. Pending frames die with the session and
    /// both members are notified on their next poll.
    pub fn close(&mut self, id: Uuid, by: Uuid, _now: u64) -> Result<(), RelayFault> {
        let session = self.sessions.get(&id).ok_or(RelayFault::UnknownSession)?;
        if !session.is_member(by) {
            return Err(RelayFault::NotMember);
        }
        self.close_locked(id);
        Ok(())
    }

    /// Collects everything due for `device`: others' frames (in queue order,
    /// within `max_bytes`), sessions closed since the last answer, and opens
    /// awaiting acceptance (delivered once). Draining counts as activity.
    pub fn drain_for(&mut self, device: Uuid, now: u64, max_bytes: usize) -> RelayDelivery {
        let mut delivery = RelayDelivery::default();
        let mut used = 0_usize;
        let mut drained_sessions = Vec::new();
        let ids: Vec<Uuid> = self.sessions.keys().copied().collect();
        for id in ids {
            let take = {
                let (is_member, is_open) = match self.sessions.get(&id) {
                    Some(session) => (session.is_member(device), session.state == RelayState::Open),
                    None => (false, false),
                };
                is_member && is_open
            };
            if !take {
                continue;
            }
            let mut exhausted = false;
            while let Some(position) = self
                .pending
                .get(&id)
                .and_then(|queue| queue.iter().position(|frame| frame.from != device))
            {
                let Some(frame) = self.pending.get(&id).and_then(|q| q.get(position)) else {
                    break;
                };
                let wire_bytes = serde_json::to_vec(&serde_json::json!({
                    "relay_id": id,
                    "from": frame.from,
                    "seq": frame.seq,
                    "bytes": frame.bytes,
                }))
                .map(|bytes| bytes.len())
                .unwrap_or(max_bytes.saturating_add(1));
                if used.saturating_add(wire_bytes) > max_bytes {
                    exhausted = true;
                    break;
                }
                let frame = self
                    .pending
                    .get_mut(&id)
                    .and_then(|q| q.remove(position))
                    .expect("frame just seen");
                used += wire_bytes;
                delivery.frames.push(DeliveredFrame {
                    relay_id: id,
                    from: frame.from,
                    seq: frame.seq,
                    bytes: frame.bytes,
                });
            }
            if !exhausted || !delivery.frames.is_empty() {
                drained_sessions.push(id);
            }
        }
        for id in drained_sessions {
            if let Some(session) = self.sessions.get_mut(&id) {
                session.last_active = now;
            }
        }
        if let Some(closed) = self.closed_notify.remove(&device) {
            delivery.closed = closed;
        }
        if let Some(opens) = self.opens.remove(&device) {
            delivery.opens = opens;
        }
        delivery
    }

    /// Restores a delivery whose TLS response did not leave the server.
    /// Frames return ahead of newer traffic while preserving their order;
    /// notices remain pending for the next poll as well.
    pub fn restore_for(&mut self, device: Uuid, delivery: RelayDelivery) {
        for frame in delivery.frames.into_iter().rev() {
            if self
                .sessions
                .get(&frame.relay_id)
                .is_some_and(|session| session.state == RelayState::Open)
            {
                self.pending
                    .entry(frame.relay_id)
                    .or_default()
                    .push_front(RelayFrame {
                        from: frame.from,
                        seq: frame.seq,
                        bytes: frame.bytes,
                        enqueued_at: 0,
                    });
            }
        }
        if !delivery.closed.is_empty() {
            for id in delivery.closed {
                self.note_closed(device, id);
            }
        }
        if !delivery.opens.is_empty() {
            let pending = self.opens.entry(device).or_default();
            pending.splice(0..0, delivery.opens);
        }
    }

    /// Sweeps time-based transitions: unaccepted opens, idle sessions, and
    /// over-TTL sessions close with notice to both members. Cheap (a handful
    /// of sessions); callers run it on relay traffic and poll ticks.
    pub fn expire(&mut self, now: u64) {
        let due: Vec<Uuid> = self
            .sessions
            .values()
            .filter(|s| {
                s.is_live()
                    && (matches!(s.state, RelayState::Opening)
                        && now > s.created_at.saturating_add(RELAY_ACCEPT_WINDOW_SECS)
                        || now > s.last_active.saturating_add(RELAY_IDLE_TIMEOUT_SECS)
                        || now > s.created_at.saturating_add(RELAY_SESSION_TTL_SECS))
            })
            .map(|s| s.id)
            .collect();
        for id in due {
            self.close_locked(id);
        }
    }

    /// Live sessions involving `device` (for delivery-tick granularity).
    /// Closed sessions never tick faster.
    pub fn has_open_media(&self, device: Uuid) -> bool {
        self.sessions.values().any(|s| {
            s.is_member(device) && s.state == RelayState::Open && s.purpose == RelayPurpose::Media
        })
    }

    /// Pairs two devices for the control-plane pairing check the caller runs.
    /// Kept here so transport asks one question (`paired_peers`) in one place.
    pub fn check_paired(control: &ControlPlane, first: Uuid, second: Uuid) -> bool {
        control
            .paired_peers(first)
            .map(|peers| peers.iter().any(|peer| peer.device_id == second))
            .unwrap_or(false)
    }
}

fn pair_key(first: Uuid, second: Uuid) -> (Uuid, Uuid) {
    if first < second {
        (first, second)
    } else {
        (second, first)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pair() -> (Uuid, Uuid) {
        (Uuid::new_v4(), Uuid::new_v4())
    }

    fn opened(purpose: RelayPurpose) -> (RelayTable, Uuid, Uuid, Uuid) {
        let mut table = RelayTable::new();
        let (first, second) = pair();
        let id = table.open(first, second, purpose, 1000).unwrap();
        (table, id, first, second)
    }

    #[test]
    fn open_accept_push_drain_flows() {
        let (mut table, id, first, second) = opened(RelayPurpose::Chat);
        assert!(table.accept(id, first, 1001).is_err());
        table.accept(id, second, 1001).unwrap();
        table.accept(id, second, 1002).unwrap();

        table
            .push(id, first, 0, "ciphertext-a".to_owned(), 1003)
            .unwrap();
        table
            .push(id, second, 0, "ciphertext-b".to_owned(), 1004)
            .unwrap();

        let delivery = table.drain_for(second, 1005, POLL_MAX_BYTES);
        assert_eq!(delivery.frames.len(), 1);
        assert_eq!(delivery.frames[0].relay_id, id);
        assert_eq!(delivery.frames[0].from, first);
        assert_eq!(delivery.frames[0].seq, 0);
        assert_eq!(delivery.frames[0].bytes, "ciphertext-a");
        assert!(delivery.closed.is_empty());

        // The sender never receives its own frames; the peer's frame waits.
        let own = table.drain_for(first, 1006, POLL_MAX_BYTES);
        assert_eq!(own.frames.len(), 1);
        assert_eq!(own.frames[0].from, second);

        // Drained queues stay empty.
        assert!(
            table
                .drain_for(second, 1007, POLL_MAX_BYTES)
                .frames
                .is_empty()
        );
    }

    #[test]
    fn drain_skips_own_frames_without_blocking_the_other_direction() {
        let (mut table, id, first, second) = opened(RelayPurpose::Chat);
        table.accept(id, second, 1001).unwrap();
        table
            .push(id, second, 0, "toward-first".into(), 1002)
            .unwrap();
        table
            .push(id, first, 0, "toward-second".into(), 1003)
            .unwrap();

        let second_delivery = table.drain_for(second, 1004, POLL_MAX_BYTES);
        assert_eq!(second_delivery.frames.len(), 1);
        assert_eq!(second_delivery.frames[0].bytes, "toward-second");
        let first_delivery = table.drain_for(first, 1005, POLL_MAX_BYTES);
        assert_eq!(first_delivery.frames.len(), 1);
        assert_eq!(first_delivery.frames[0].bytes, "toward-first");
    }

    #[test]
    fn failed_poll_delivery_can_be_restored_in_order() {
        let (mut table, id, first, second) = opened(RelayPurpose::Chat);
        table.accept(id, second, 1001).unwrap();
        table.push(id, first, 0, "first".into(), 1002).unwrap();
        table.push(id, first, 1, "second".into(), 1003).unwrap();

        let delivery = table.drain_for(second, 1004, POLL_MAX_BYTES);
        assert_eq!(delivery.frames.len(), 2);
        table.restore_for(second, delivery);
        let retried = table.drain_for(second, 1005, POLL_MAX_BYTES);
        assert_eq!(
            retried
                .frames
                .iter()
                .map(|frame| frame.seq)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    #[test]
    fn closed_notices_stay_bounded_for_an_offline_peer() {
        let mut table = RelayTable::new();
        let (first, second) = pair();
        for now in 0..(MAX_PENDING_CLOSED_NOTICES as u64 + 10) {
            let id = table.open(first, second, RelayPurpose::Chat, now).unwrap();
            table.close(id, first, now).unwrap();
        }
        assert_eq!(
            table.closed_notify.get(&second).map(Vec::len),
            Some(MAX_PENDING_CLOSED_NOTICES)
        );
        assert!(table.sessions.len() <= MAX_CLOSED_SESSION_TOMBSTONES);
        let failed_delivery = table.drain_for(second, 100, POLL_MAX_BYTES);
        for now in 200..210 {
            let id = table.open(first, second, RelayPurpose::Chat, now).unwrap();
            table.close(id, first, now).unwrap();
        }
        table.restore_for(second, failed_delivery);
        assert_eq!(
            table.closed_notify.get(&second).map(Vec::len),
            Some(MAX_PENDING_CLOSED_NOTICES)
        );
    }

    #[test]
    fn open_rejects_self_and_caps_sessions() {
        let mut table = RelayTable::new();
        let (first, _) = pair();
        assert_eq!(
            table.open(first, first, RelayPurpose::Chat, 1000),
            Err(RelayFault::SelfRelay)
        );

        let mut ids = Vec::new();
        for _ in 0..MAX_RELAY_SESSIONS {
            let (a, b) = pair();
            ids.push(table.open(a, b, RelayPurpose::Chat, 1000).unwrap());
        }
        let (x, y) = pair();
        assert_eq!(
            table.open(x, y, RelayPurpose::Chat, 1000),
            Err(RelayFault::TooManySessions)
        );
        table
            .close(ids[0], table.sessions[&ids[0]].opener, 1001)
            .unwrap();
        table.open(x, y, RelayPurpose::Chat, 1001).unwrap();
    }

    #[test]
    fn reopening_supersedes_the_older_session_for_a_pair() {
        let (mut table, first_id, first, second) = opened(RelayPurpose::Chat);
        table.accept(first_id, second, 1001).unwrap();
        let second_id = table.open(first, second, RelayPurpose::File, 1002).unwrap();
        assert_ne!(first_id, second_id);

        // The older session is closed: pushes refuse, and the closer learns.
        assert_eq!(
            table.push(first_id, first, 0, "late".to_owned(), 1003),
            Err(RelayFault::ClosedSession)
        );
        let notice = table.drain_for(second, 1004, POLL_MAX_BYTES);
        assert!(notice.closed.contains(&first_id));
    }

    #[test]
    fn accept_window_and_membership_are_enforced() {
        let (mut table, id, first, second) = opened(RelayPurpose::Chat);
        let stranger = Uuid::new_v4();
        assert_eq!(table.accept(id, stranger, 1001), Err(RelayFault::NotMember));
        assert_eq!(
            table.accept(Uuid::new_v4(), second, 1001),
            Err(RelayFault::UnknownSession)
        );
        assert_eq!(
            table.accept(id, second, 1000 + RELAY_ACCEPT_WINDOW_SECS + 1),
            Err(RelayFault::AcceptExpired)
        );
        // Expiry closed the session with notice.
        let notice = table.drain_for(first, 1012, POLL_MAX_BYTES);
        assert!(notice.closed.contains(&id));
    }

    #[test]
    fn push_validates_membership_liveness_size_and_strict_order() {
        let (mut table, id, first, second) = opened(RelayPurpose::Chat);
        let stranger = Uuid::new_v4();
        // Not open yet: pushes refuse even from members.
        assert_eq!(
            table.push(id, first, 0, "early".to_owned(), 1001),
            Err(RelayFault::ClosedSession)
        );
        table.accept(id, second, 1001).unwrap();

        assert_eq!(
            table.push(id, stranger, 0, "intruder".to_owned(), 1002),
            Err(RelayFault::NotMember)
        );
        assert_eq!(
            table.push(id, first, 0, "x".repeat(MAX_RELAY_DATA_CHARS + 1), 1002),
            Err(RelayFault::OversizedFrame)
        );
        // Gap first: strict order refuses the jump.
        assert_eq!(
            table.push(id, first, 1, "jump".to_owned(), 1002),
            Err(RelayFault::OutOfOrder)
        );
        table.push(id, first, 0, "zero".to_owned(), 1002).unwrap();
        // Replay: the same sequence never forwards twice.
        assert_eq!(
            table.push(id, first, 0, "zero-again".to_owned(), 1003),
            Err(RelayFault::OutOfOrder)
        );
        table.push(id, first, 1, "one".to_owned(), 1003).unwrap();

        // Directions track independently.
        table
            .push(id, second, 0, "other-zero".to_owned(), 1004)
            .unwrap();
        assert_eq!(
            table.push(id, second, 0, "other-zero-again".to_owned(), 1005),
            Err(RelayFault::OutOfOrder)
        );
    }

    #[test]
    fn full_queues_refuse_instead_of_growing() {
        let (mut table, id, first, second) = opened(RelayPurpose::Chat);
        table.accept(id, second, 1001).unwrap();
        for seq in 0..MAX_PENDING_PER_SESSION as u64 {
            table
                .push(id, first, seq, format!("f{seq}"), 1100 + seq)
                .unwrap();
        }
        let global_before = table.global_rate;
        let session_before = table.sessions[&id].rate;
        // Repeated full-queue retries charge neither rate window: the frame
        // was never enqueued or forwarded.
        for _ in 0..3 {
            assert_eq!(
                table.push(
                    id,
                    first,
                    MAX_PENDING_PER_SESSION as u64,
                    "full".to_owned(),
                    2000
                ),
                Err(RelayFault::QueueFull)
            );
        }
        assert_eq!(table.global_rate.frames, global_before.frames);
        assert_eq!(table.global_rate.bytes, global_before.bytes);
        assert_eq!(table.sessions[&id].rate.frames, session_before.frames);
        assert_eq!(table.sessions[&id].rate.bytes, session_before.bytes);
    }

    #[test]
    fn close_is_idempotent_and_notifies_both_members() {
        let (mut table, id, first, second) = opened(RelayPurpose::Chat);
        table.accept(id, second, 1001).unwrap();
        table
            .push(id, first, 0, "in-flight".to_owned(), 1002)
            .unwrap();
        table.close(id, second, 1003).unwrap();
        table.close(id, first, 1004).unwrap();

        // Pending dies with the session; both sides learn on next drain.
        for device in [first, second] {
            let notice = table.drain_for(device, 1005, POLL_MAX_BYTES);
            assert!(notice.frames.is_empty());
            assert!(notice.closed.contains(&id));
        }
        // Second drain: notices are one-shot.
        assert!(
            table
                .drain_for(first, 1006, POLL_MAX_BYTES)
                .closed
                .is_empty()
        );
        assert_eq!(
            table.close(Uuid::new_v4(), first, 1007),
            Err(RelayFault::UnknownSession)
        );
    }

    #[test]
    fn expire_sweeps_idle_ttl_and_unaccepted_sessions() {
        let (mut table, id, first, _second) = opened(RelayPurpose::Chat);
        // Never accepted: gone after the window.
        table.expire(1000 + RELAY_ACCEPT_WINDOW_SECS + 1);
        assert!(
            table
                .drain_for(first, 1012, POLL_MAX_BYTES)
                .closed
                .contains(&id)
        );

        let (mut table, id, first, second) = opened(RelayPurpose::Media);
        table.accept(id, second, 1001).unwrap();
        // Active traffic keeps it alive past the idle mark from creation.
        table
            .push(
                id,
                first,
                0,
                "keep".to_owned(),
                1000 + RELAY_IDLE_TIMEOUT_SECS - 1,
            )
            .unwrap();
        table.expire(1000 + RELAY_IDLE_TIMEOUT_SECS + 1);
        let alive = table.drain_for(second, 1100, POLL_MAX_BYTES);
        assert_eq!(alive.frames.len(), 1);
        assert!(alive.closed.is_empty());
        // True silence kills it.
        table.expire(2000 + RELAY_IDLE_TIMEOUT_SECS + 1);
        assert!(
            table
                .drain_for(first, 2100, POLL_MAX_BYTES)
                .closed
                .contains(&id)
        );
    }

    #[test]
    fn sessions_exhaust_after_256_mib_forwarded() {
        let (mut table, id, first, second) = opened(RelayPurpose::File);
        table.accept(id, second, 1001).unwrap();
        table.sessions.get_mut(&id).unwrap().bytes_forwarded = MAX_SESSION_BYTES - 10;
        table
            .push(id, first, 0, "0123456789".to_owned(), 1002)
            .unwrap();
        assert_eq!(
            table.push(id, first, 1, "one-more".to_owned(), 1003),
            Err(RelayFault::SessionExhausted)
        );
        let fault = RelayFault::SessionExhausted;
        assert_eq!(fault.code(), "relay_exhausted");
        assert!(!fault.retryable());
    }

    #[test]
    fn drain_respects_the_byte_budget_in_queue_order() {
        let (mut table, id, first, second) = opened(RelayPurpose::Chat);
        table.accept(id, second, 1001).unwrap();
        table.push(id, first, 0, "a".repeat(100), 1002).unwrap();
        table.push(id, first, 1, "b".repeat(100), 1003).unwrap();
        // Budget applies to the serialized frame, including routing fields.
        let partial = table.drain_for(second, 1004, 300);
        assert_eq!(partial.frames.len(), 1);
        assert_eq!(partial.frames[0].seq, 0);
        let rest = table.drain_for(second, 1005, POLL_MAX_BYTES);
        assert_eq!(rest.frames.len(), 1);
        assert_eq!(rest.frames[0].seq, 1);
    }

    #[test]
    fn rate_ceiling_refuses_pre_enqueue_and_recovers_next_second() {
        let (mut table, id, first, second) = opened(RelayPurpose::File);
        table.accept(id, second, 1001).unwrap();
        // Frame cap: push (and drain, so the 8-deep queue never refuses)
        // until the per-session window trips; the refused seq then retries
        // cleanly one second later.
        let mut seq = 0_u64;
        let mut refused = None;
        for _ in 0..RELAY_SESSION_MAX_FRAMES_PER_SEC + 1 {
            match table.push(id, first, seq, "x".to_owned(), 2000) {
                Ok(()) => {
                    seq += 1;
                    let _ = table.drain_for(second, 2000, POLL_MAX_BYTES);
                }
                Err(RelayFault::RateLimited) => {
                    refused = Some(seq);
                    break;
                }
                Err(other) => panic!("unexpected fault: {other:?}"),
            }
        }
        let refused_seq = refused.expect("cap must trip inside the window");
        table
            .push(id, first, refused_seq, "x".to_owned(), 2001)
            .unwrap();
        // Byte cap: 40 KiB chunks until the 1 MiB window overflows.
        let (mut table, id, first, second) = opened(RelayPurpose::File);
        table.accept(id, second, 1001).unwrap();
        let chunk = "x".repeat(40 * 1024);
        let mut seq = 0_u64;
        while (seq + 1) * (40 * 1024) <= RELAY_SESSION_MAX_BYTES_PER_SEC {
            table.push(id, first, seq, chunk.clone(), 2000).unwrap();
            let _ = table.drain_for(second, 2000, POLL_MAX_BYTES);
            seq += 1;
        }
        assert_eq!(
            table.push(id, first, seq, chunk, 2000),
            Err(RelayFault::RateLimited)
        );
    }

    #[test]
    fn global_breaker_refuses_pushes_and_opens_but_recovers() {
        let mut table = RelayTable::new();
        let mut sessions = Vec::new();
        for _ in 0..MAX_RELAY_SESSIONS {
            let (a, b) = pair();
            let id = table.open(a, b, RelayPurpose::File, 2000).unwrap();
            table.accept(id, b, 2000).unwrap();
            sessions.push((id, a, b));
        }
        // Round-robin tiny pushes (drained each time so only rate windows
        // can refuse) until the global frame cap trips. The four opens
        // already counted, so the breaker fires while sessions still have
        // headroom — the global ceiling is what gives.
        let mut seqs = vec![0_u64; sessions.len()];
        let mut tripped = false;
        for round in 0..(RELAY_GLOBAL_MAX_FRAMES_PER_SEC + MAX_RELAY_SESSIONS as u64 + 8) {
            let idx = round as usize % sessions.len();
            let (id, a, b) = sessions[idx];
            match table.push(id, a, seqs[idx], "x".to_owned(), 2000) {
                Ok(()) => {
                    seqs[idx] += 1;
                    let _ = table.drain_for(b, 2000, POLL_MAX_BYTES);
                }
                Err(RelayFault::RateLimited) => continue,
                Err(RelayFault::Overloaded) => {
                    tripped = true;
                    break;
                }
                Err(other) => panic!("unexpected fault: {other:?}"),
            }
        }
        assert!(tripped, "breaker must trip inside the window");
        // Over the budget: pushes AND opens refuse; the fault surfaces as
        // `relay_busy`, retryable — the client's existing backoff applies.
        let (id, a, _b) = sessions[0];
        assert_eq!(
            table.push(id, a, seqs[0], "one-more".to_owned(), 2000),
            Err(RelayFault::Overloaded)
        );
        let (x, y) = pair();
        assert_eq!(
            table.open(x, y, RelayPurpose::Chat, 2000),
            Err(RelayFault::Overloaded)
        );
        let fault = RelayFault::Overloaded;
        assert_eq!(fault.code(), "relay_busy");
        assert!(fault.retryable());
        // Next second the window resets: opens and pushes flow again (a
        // session slot is freed first — the 4-session quota still applies).
        table.close(id, a, 2000).unwrap();
        table.open(x, y, RelayPurpose::Chat, 2001).unwrap();
        let (live_id, live_a, live_b) = sessions[1];
        table
            .push(live_id, live_a, seqs[1], "one-more".to_owned(), 2001)
            .unwrap();
        let _ = table.drain_for(live_b, 2001, POLL_MAX_BYTES);
    }

    #[test]
    fn opens_are_announced_once_and_media_ticks_faster() {
        let (mut table, id, first, second) = opened(RelayPurpose::Media);
        let notice = table.drain_for(second, 1001, POLL_MAX_BYTES);
        assert_eq!(notice.opens.len(), 1);
        assert_eq!(notice.opens[0].relay_id, id);
        assert_eq!(notice.opens[0].from, first);
        assert_eq!(notice.opens[0].purpose, RelayPurpose::Media);
        assert!(
            table
                .drain_for(second, 1002, POLL_MAX_BYTES)
                .opens
                .is_empty()
        );
        assert!(!table.has_open_media(first));
        table.accept(id, second, 1003).unwrap();
        assert!(table.has_open_media(first));
        assert!(table.has_open_media(second));
        assert!(!table.has_open_media(Uuid::new_v4()));
    }
}
