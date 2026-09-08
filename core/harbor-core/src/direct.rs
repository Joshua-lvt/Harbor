//! Direct-session policy: chat and file transfer between paired peers.
//!
//! Everything in this module is policy and bookkeeping. Bytes move only
//! across the worker's direct DataChannels; the control-plane server never
//! sees a message body, a file byte, or a chunk. The worker transports
//! frames; this module decides what may be sent, where received files land,
//! and whether what arrived is what was promised.
//!
//! Limits are the Fase 0 capability matrix, enforced here before anything
//! enters a queue, a staging file, or the wire:
//!
//! - chat body: 4096 bytes after control-character stripping;
//! - outbound queue: 32 messages while no direct path is live;
//! - transcript: the most recent 200 messages, in memory only;
//! - file size: the shared protocol ceiling (1 TiB), admitted further
//!   against free space on the staging/destination volume plus a 64 MiB
//!   margin; chunk: 16 KiB; concurrent transfers: 2;
//! - an unanswered offer expires after 10 minutes.

use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// A chat body may carry at most this many bytes once sanitized.
pub const CHAT_MAX_BODY_BYTES: usize = 4096;
/// Messages held while no direct path is live; beyond this the oldest is
/// refused at compose time rather than silently dropped later.
pub const CHAT_QUEUE_CAPACITY: usize = 32;
/// The in-memory transcript never grows without bound.
pub const CHAT_TRANSCRIPT_CAPACITY: usize = 200;
/// A single transfer may move at most this many bytes. This is the shared
/// protocol ceiling — the media worker (channels.go `fileMaxBytes`) carries
/// the same number, so both sides refuse the same offers. Real capacity on
/// top of this comes from free space: bytes are admitted only when the
/// target volume can actually hold them.
pub const TRANSFER_MAX_BYTES: u64 = 1 << 40;
/// Space that must remain free on the target volume beyond the file itself,
/// so a transfer never fills a disk to the last byte.
pub const TRANSFER_FREE_MARGIN_BYTES: u64 = 64 << 20;
/// The streaming chunk size this side speaks.
pub const TRANSFER_CHUNK_BYTES: usize = 16 << 10;
/// At most this many transfers may be live (offered or active) at once.
pub const TRANSFER_CONCURRENT_LIMIT: usize = 2;
/// An offer the peer neither accepts nor rejects dies after this long.
pub const TRANSFER_OFFER_TTL_SECS: u64 = 600;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirectError {
    pub code: &'static str,
    pub ui_key: &'static str,
}

impl DirectError {
    fn new(code: &'static str, ui_key: &'static str) -> Self {
        Self { code, ui_key }
    }
}

/// Strips control characters a text surface must never render (everything in
/// C0, DEL, and the bidi/zero-width overrides that can make a safe string
/// lie about its direction), then enforces the size limit. Newlines and tabs
/// survive; HTML or script material is not interpreted anywhere downstream —
/// QML renders this as plain text.
pub fn sanitize_message_body(raw: &str) -> Result<String, DirectError> {
    let sanitized: String = raw
        .chars()
        .filter(|c| {
            let cp = *c as u32;
            if *c == '\n' || *c == '\t' {
                return true;
            }
            if cp < 0x20 || cp == 0x7f {
                return false;
            }
            !(0x200b..=0x200f).contains(&cp)
                && !(0x202a..=0x202e).contains(&cp)
                && !(0x2066..=0x2069).contains(&cp)
        })
        .collect();
    if sanitized.trim().is_empty() {
        return Err(DirectError::new("chat_empty", "error.chat.empty"));
    }
    if sanitized.len() > CHAT_MAX_BODY_BYTES {
        return Err(DirectError::new("chat_too_large", "error.chat.tooLarge"));
    }
    Ok(sanitized)
}

/// Names a received file so it can only ever land as a plain file name inside
/// the destination directory: no separators, no traversal, no reserved
/// device names, no bidi overrides, no trailing dot/space tricks, no silent
/// length truncation that could turn one name into another.
pub fn sanitize_filename(raw: &str) -> Result<String, DirectError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(DirectError::new(
            "name_invalid",
            "error.transfer.nameInvalid",
        ));
    }
    for c in trimmed.chars() {
        let cp = c as u32;
        let forbidden = matches!(c, '/' | '\\') || (cp < 0x20) || c == '\u{7f}';
        let deceptive = (0x200b..=0x200f).contains(&cp)
            || (0x202a..=0x202e).contains(&cp)
            || (0x2066..=0x2069).contains(&cp);
        if forbidden || deceptive {
            return Err(DirectError::new(
                "name_invalid",
                "error.transfer.nameInvalid",
            ));
        }
    }
    if trimmed == "." || trimmed == ".." {
        return Err(DirectError::new(
            "name_invalid",
            "error.transfer.nameInvalid",
        ));
    }
    // Windows reserves a small set of device names in any case and with or
    // without an extension; they are refused, never mapped.
    let stem = trimmed.split('.').next().unwrap_or(trimmed);
    let reserved = matches!(
        stem.to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    );
    if reserved {
        return Err(DirectError::new(
            "name_invalid",
            "error.transfer.nameInvalid",
        ));
    }
    // A name that only becomes safe by being cut is not that name.
    let mut capped: String = trimmed.chars().take(255).collect();
    capped = capped.trim_end_matches(['.', ' ']).to_string();
    if capped.is_empty() {
        return Err(DirectError::new(
            "name_invalid",
            "error.transfer.nameInvalid",
        ));
    }
    Ok(capped)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    /// Composed, not yet handed to a live direct path.
    WaitingForConnection,
    /// The worker accepted the frame onto the direct channel.
    Sent,
    /// The peer's worker acknowledged the frame.
    Delivered,
    /// The direct path refused or died before delivery.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Direction {
    Outgoing,
    Incoming,
}

#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub id: String,
    pub body: String,
    pub direction: Direction,
    pub delivery: Delivery,
    pub timestamp: u64,
}

/// Chat bookkeeping for one session: an outbound queue that survives a dead
/// path, and an ordered in-memory transcript. Nothing here is persisted; the
/// process owns it for exactly as long as the user keeps it running.
#[derive(Default)]
pub struct ChatSession {
    transcript: VecDeque<ChatMessage>,
    /// Outbound messages awaiting a live path, in compose order.
    queue: VecDeque<ChatMessage>,
    queue_capacity: usize,
}

impl ChatSession {
    pub fn new() -> Self {
        Self {
            transcript: VecDeque::new(),
            queue: VecDeque::new(),
            queue_capacity: CHAT_QUEUE_CAPACITY,
        }
    }

    /// Validates and queues one outbound message. A full queue is an honest
    /// refusal at compose time, not a later silent loss.
    pub fn compose(&mut self, id: String, raw: &str, now: u64) -> Result<ChatMessage, DirectError> {
        let body = sanitize_message_body(raw)?;
        if self.queue.len() >= self.queue_capacity {
            return Err(DirectError::new("chat_queue_full", "error.chat.queueFull"));
        }
        let message = ChatMessage {
            id,
            body,
            direction: Direction::Outgoing,
            delivery: Delivery::WaitingForConnection,
            timestamp: now,
        };
        self.record_transcript(message.clone());
        self.queue.push_back(message.clone());
        Ok(message)
    }

    /// Pops queued messages for transmission once a path is live.
    pub fn drain_queue(&mut self) -> Vec<ChatMessage> {
        self.queue.drain(..).collect()
    }

    /// Requeues messages whose transmission failed so they may retry.
    pub fn requeue(&mut self, messages: Vec<ChatMessage>) {
        for message in messages.into_iter().rev() {
            self.queue.push_front(message);
        }
    }

    /// Records one inbound message exactly once. The worker may be polled
    /// again after a response is lost, so IDs make delivery idempotent.
    pub fn record_inbound(&mut self, id: String, body: String, now: u64) -> bool {
        if self.transcript.iter().any(|message| message.id == id) {
            return false;
        }
        let sanitized = sanitize_message_body(&body).unwrap_or_else(|_| {
            // A body the policy refuses is still a fact: it arrived. The
            // transcript keeps a redacted marker instead of dropping the
            // message silently.
            format!("[redacted {id}]")
        });
        self.record_transcript(ChatMessage {
            id,
            body: sanitized,
            direction: Direction::Incoming,
            delivery: Delivery::Delivered,
            timestamp: now,
        });
        true
    }

    /// Records one message synced from another device of the same identity.
    /// Idempotent by id, like [`ChatSession::record_inbound`]: the companion
    /// phone and the desktop converge on the same conversation instead of
    /// forking one transcript per device.
    pub fn record_synced(
        &mut self,
        id: String,
        body: String,
        direction: Direction,
        delivery: Delivery,
        now: u64,
    ) -> bool {
        if self.transcript.iter().any(|message| message.id == id) {
            return false;
        }
        let sanitized = sanitize_message_body(&body).unwrap_or_else(|_| format!("[redacted {id}]"));
        self.record_transcript(ChatMessage {
            id,
            body: sanitized,
            direction,
            delivery,
            timestamp: now,
        });
        true
    }

    /// Marks a specific transcript entry's delivery fact.
    pub fn mark_delivery(&mut self, id: &str, delivery: Delivery) {
        for message in &mut self.transcript {
            if message.id == id && message.direction == Direction::Outgoing {
                message.delivery = delivery;
            }
        }
    }

    fn record_transcript(&mut self, message: ChatMessage) {
        while self.transcript.len() >= CHAT_TRANSCRIPT_CAPACITY {
            self.transcript.pop_front();
        }
        self.transcript.push_back(message);
    }

    pub fn snapshot(&self) -> Vec<ChatMessage> {
        self.transcript.iter().cloned().collect()
    }

    pub fn queue_len(&self) -> usize {
        self.queue.len()
    }

    /// IDs that have been handed to the direct channel but still need the
    /// peer-worker acknowledgement before becoming DELIVERED.
    pub fn sent_ids(&self) -> Vec<String> {
        self.transcript
            .iter()
            .filter(|message| {
                message.direction == Direction::Outgoing && message.delivery == Delivery::Sent
            })
            .map(|message| message.id.clone())
            .collect()
    }

    /// A terminal call failure does not pretend that a sent-but-unacknowledged
    /// frame arrived. Those entries surface as FAILED in the transcript.
    pub fn fail_unacknowledged(&mut self) {
        for message in &mut self.transcript {
            if message.direction == Direction::Outgoing && message.delivery == Delivery::Sent {
                message.delivery = Delivery::Failed;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferPhase {
    Offered,
    Active,
    Completed,
    Canceled,
    Failed,
}

/// One streamed source chunk: `(sequence, offset, bytes, is_final)`.
pub type OutgoingChunk = (usize, u64, Vec<u8>, bool);

/// The sanitized, UI-safe view of one transfer. Paths never cross to QML.
#[derive(Debug, Clone)]
pub struct TransferRecord {
    pub id: String,
    pub direction: Direction,
    pub phase: TransferPhase,
    pub name: String,
    pub size: u64,
    pub received_bytes: u64,
    pub peer_received: bool,
    pub expired: bool,
}

/// Runtime state for one live transfer: the streaming hash and file handle
/// live exactly as long as the transfer does.
struct ActiveTransfer {
    record: TransferRecord,
    sum: String,
    source: Option<PathBuf>,
    file: Option<File>,
    hasher: Sha256,
    next_seq: usize,
    pending: Option<OutgoingChunk>,
    outgoing_finished: bool,
    expires_at: u64,
}

#[derive(Default)]
pub struct TransferBoard {
    active: Vec<ActiveTransfer>,
    /// Overridable free-space probe; production uses the real statvfs call,
    /// tests substitute a deterministic answer.
    space_probe: Option<fn(&Path) -> Option<u64>>,
}

impl TransferBoard {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    fn with_space_probe(mut self, probe: fn(&Path) -> Option<u64>) -> Self {
        self.space_probe = Some(probe);
        self
    }

    fn find_mut(&mut self, id: &str) -> Option<&mut ActiveTransfer> {
        self.active.iter_mut().find(|t| t.record.id == id)
    }

    fn live_count(&self) -> usize {
        self.active.iter().filter(|t| !t.record.expired).count()
    }

    /// Registers an outgoing transfer: validates the source, sizes it, hashes
    /// it in one streaming pass (the offer must carry the real digest), and
    /// keeps only the metadata.
    pub fn begin_outgoing(
        &mut self,
        id: String,
        source: &Path,
        now: u64,
    ) -> Result<TransferRecord, DirectError> {
        if self.live_count() >= TRANSFER_CONCURRENT_LIMIT {
            return Err(DirectError::new("transfer_busy", "error.transfer.tooMany"));
        }
        let raw_name = source
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| DirectError::new("name_invalid", "error.transfer.nameInvalid"))?;
        let name = sanitize_filename(raw_name)?;
        let metadata = fs::metadata(source)
            .map_err(|_| DirectError::new("transfer_unreadable", "error.transfer.unreadable"))?;
        if !metadata.is_file() {
            return Err(DirectError::new(
                "transfer_unreadable",
                "error.transfer.unreadable",
            ));
        }
        let size = metadata.len();
        if size == 0 || size > TRANSFER_MAX_BYTES {
            return Err(DirectError::new(
                "transfer_too_large",
                "error.transfer.tooLarge",
            ));
        }
        let mut file = File::open(source)
            .map_err(|_| DirectError::new("transfer_unreadable", "error.transfer.unreadable"))?;
        let mut hasher = Sha256::new();
        std::io::copy(&mut file, &mut hasher)
            .map_err(|_| DirectError::new("transfer_unreadable", "error.transfer.unreadable"))?;
        let digest = hex_lower(&hasher.finalize());
        self.active.push(ActiveTransfer {
            record: TransferRecord {
                id,
                direction: Direction::Outgoing,
                phase: TransferPhase::Offered,
                name,
                size,
                received_bytes: 0,
                peer_received: false,
                expired: false,
            },
            sum: digest,
            source: Some(source.to_path_buf()),
            file: None,
            hasher: Sha256::new(),
            next_seq: 0,
            pending: None,
            outgoing_finished: false,
            expires_at: now + TRANSFER_OFFER_TTL_SECS,
        });
        Ok(self.active.last().expect("just pushed").record.clone())
    }

    /// Returns the metadata required for a private worker offer. The digest is
    /// never serialized across the UI boundary; it travels only over the
    /// worker's direct control channel.
    pub fn outgoing_offer(&self, id: &str) -> Option<(String, u64, String)> {
        self.active
            .iter()
            .find(|transfer| {
                transfer.record.id == id
                    && transfer.record.direction == Direction::Outgoing
                    && transfer.record.phase == TransferPhase::Offered
            })
            .map(|transfer| {
                (
                    transfer.record.name.clone(),
                    transfer.record.size,
                    transfer.sum.clone(),
                )
            })
    }

    /// Registers an inbound offer after validating what the peer claimed.
    pub fn offer_inbound(
        &mut self,
        id: String,
        raw_name: &str,
        size: u64,
        sum: String,
        now: u64,
    ) -> Result<TransferRecord, DirectError> {
        if self.live_count() >= TRANSFER_CONCURRENT_LIMIT {
            return Err(DirectError::new("transfer_busy", "error.transfer.tooMany"));
        }
        let name = sanitize_filename(raw_name)?;
        if size == 0 || size > TRANSFER_MAX_BYTES {
            return Err(DirectError::new(
                "transfer_too_large",
                "error.transfer.tooLarge",
            ));
        }
        if sum.len() != 64 || !sum.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(DirectError::new("sum_invalid", "error.transfer.sumInvalid"));
        }
        self.active.push(ActiveTransfer {
            record: TransferRecord {
                id,
                direction: Direction::Incoming,
                phase: TransferPhase::Offered,
                name,
                size,
                received_bytes: 0,
                peer_received: false,
                expired: false,
            },
            sum: sum.to_ascii_lowercase(),
            source: None,
            file: None,
            hasher: Sha256::new(),
            next_seq: 0,
            pending: None,
            outgoing_finished: false,
            expires_at: now + TRANSFER_OFFER_TTL_SECS,
        });
        Ok(self.active.last().expect("just pushed").record.clone())
    }

    pub fn accept_inbound(&mut self, id: &str, staging_dir: &Path) -> Result<(), DirectError> {
        let probe = self.space_probe;
        let transfer = self
            .find_mut(id)
            .ok_or_else(|| DirectError::new("transfer_unknown", "error.transfer.unknown"))?;
        if transfer.record.direction != Direction::Incoming
            || transfer.record.phase != TransferPhase::Offered
        {
            return Err(DirectError::new("transfer_state", "error.transfer.state"));
        }
        fs::create_dir_all(staging_dir).map_err(|_| {
            DirectError::new("staging_unavailable", "error.transfer.stagingUnavailable")
        })?;
        // The staged bytes land on this volume; refuse the accept when it
        // cannot hold them plus the safety margin. The offer stays Offered.
        let required = transfer
            .record
            .size
            .saturating_add(TRANSFER_FREE_MARGIN_BYTES);
        if let Some(free) = match probe {
            Some(probe) => probe(staging_dir),
            None => available_bytes(staging_dir),
        } {
            if free < required {
                return Err(DirectError::new("no_space", "error.transfer.noSpace"));
            }
        }
        let path = staging_dir.join(format!("{}.part", transfer.record.id));
        let file = File::create(&path).map_err(|_| {
            DirectError::new("staging_unavailable", "error.transfer.stagingUnavailable")
        })?;
        transfer.record.phase = TransferPhase::Active;
        transfer.file = Some(file);
        Ok(())
    }

    /// Streams the next outgoing chunk, or `None` when the file is done.
    /// Reads incrementally; the file is never held in memory whole. The
    /// digest the offer carried was computed at registration, so this pass
    /// only reads.
    pub fn next_outgoing_chunk(&mut self, id: &str) -> Result<Option<OutgoingChunk>, DirectError> {
        let transfer = self
            .find_mut(id)
            .ok_or_else(|| DirectError::new("transfer_unknown", "error.transfer.unknown"))?;
        if transfer.record.phase != TransferPhase::Active {
            return Err(DirectError::new("transfer_state", "error.transfer.state"));
        }
        if transfer.outgoing_finished {
            return Ok(None);
        }
        if transfer.file.is_none() {
            let source = transfer
                .source
                .clone()
                .ok_or_else(|| DirectError::new("transfer_state", "error.transfer.state"))?;
            transfer.file = Some(File::open(&source).map_err(|_| {
                DirectError::new("transfer_unreadable", "error.transfer.unreadable")
            })?);
        }
        if let Some((seq, offset, chunk, done)) = &transfer.pending {
            return Ok(Some((*seq, *offset, chunk.clone(), *done)));
        }
        let mut chunk = vec![0_u8; TRANSFER_CHUNK_BYTES];
        let file = transfer.file.as_mut().expect("just opened");
        let offset = file.stream_position().unwrap_or(0);
        let mut filled = 0;
        while filled < chunk.len() {
            match file.read(&mut chunk[filled..]) {
                Ok(0) => break,
                Ok(count) => filled += count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => {
                    return Err(DirectError::new(
                        "transfer_unreadable",
                        "error.transfer.unreadable",
                    ));
                }
            }
        }
        chunk.truncate(filled);
        if filled == 0 {
            return Err(DirectError::new("transfer_short", "error.transfer.state"));
        }
        let seq = transfer.next_seq;
        let done = file
            .stream_position()
            .map(|pos| pos >= transfer.record.size)
            .unwrap_or(false);
        transfer.pending = Some((seq, offset, chunk.clone(), done));
        Ok(Some((seq, offset, chunk, done)))
    }

    /// Commits a chunk only after the worker accepted it. If SCTP backpressure
    /// pauses the worker, the same pending bytes are returned on the next poll.
    pub fn confirm_outgoing_chunk(&mut self, id: &str, seq: usize) -> bool {
        let Some(transfer) = self.find_mut(id) else {
            return false;
        };
        let Some((pending_seq, _, _, done)) = transfer.pending.as_ref() else {
            return false;
        };
        if *pending_seq != seq {
            return false;
        }
        transfer.next_seq += 1;
        if *done {
            transfer.file = None;
            transfer.outgoing_finished = true;
        }
        transfer.pending = None;
        true
    }

    /// Writes one inbound chunk in strict order, hashing as it lands.
    pub fn write_inbound_chunk(
        &mut self,
        id: &str,
        seq: usize,
        offset: u64,
        data: &[u8],
        final_chunk: bool,
    ) -> Result<(), DirectError> {
        let transfer = self
            .find_mut(id)
            .ok_or_else(|| DirectError::new("transfer_unknown", "error.transfer.unknown"))?;
        if transfer.record.phase != TransferPhase::Active {
            return Err(DirectError::new("transfer_state", "error.transfer.state"));
        }
        let expected_offset = transfer.record.received_bytes;
        let Some(file) = transfer.file.as_mut() else {
            return Err(DirectError::new("transfer_state", "error.transfer.state"));
        };
        // Strict ordering: chunks land exactly in sequence and at the running
        // offset, so a peer bug surfaces here instead of corrupting a file.
        if seq != transfer.next_seq || offset != expected_offset {
            return Err(DirectError::new("chunk_order", "error.transfer.chunkOrder"));
        }
        if expected_offset + data.len() as u64 > transfer.record.size {
            return Err(DirectError::new("chunk_order", "error.transfer.chunkOrder"));
        }
        file.write_all(data).map_err(|_| {
            DirectError::new("staging_unavailable", "error.transfer.stagingUnavailable")
        })?;
        transfer.hasher.update(data);
        transfer.record.received_bytes += data.len() as u64;
        transfer.next_seq += 1;
        let _ = final_chunk;
        Ok(())
    }

    /// Verifies size and SHA-256, then moves the staged file into the
    /// destination under a non-colliding name. Only a fully validated file
    /// ever leaves staging; a mismatch destroys it and fails the transfer.
    pub fn finalize_inbound(
        &mut self,
        id: &str,
        staging_dir: &Path,
        destination_dir: &Path,
    ) -> Result<PathBuf, DirectError> {
        let probe = self.space_probe;
        let transfer = self
            .find_mut(id)
            .ok_or_else(|| DirectError::new("transfer_unknown", "error.transfer.unknown"))?;
        if transfer.record.direction != Direction::Incoming {
            return Err(DirectError::new("transfer_state", "error.transfer.state"));
        }
        let staged = staging_dir.join(format!("{}.part", transfer.record.id));
        let digest = hex_lower(&transfer.hasher.clone().finalize());
        let verified = digest == transfer.sum
            && transfer.record.received_bytes == transfer.record.size
            && staged.exists();
        if !verified {
            transfer.record.phase = TransferPhase::Failed;
            let _ = fs::remove_file(&staged);
            if transfer.record.received_bytes != transfer.record.size {
                return Err(DirectError::new(
                    "size_mismatch",
                    "error.transfer.sizeMismatch",
                ));
            }
            return Err(DirectError::new(
                "sum_mismatch",
                "error.transfer.sumMismatch",
            ));
        }

        let name = transfer.record.name.clone();
        // Checked before anything moves: on refusal the staged file stays
        // intact (the transfer remains Active) so freeing space or choosing
        // another destination can still land it — an explicit cancel cleans up.
        let required = transfer
            .record
            .size
            .saturating_add(TRANSFER_FREE_MARGIN_BYTES);
        if let Some(free) = match probe {
            Some(probe) => probe(destination_dir),
            None => available_bytes(destination_dir),
        } {
            if free < required {
                return Err(DirectError::new("no_space", "error.transfer.noSpace"));
            }
        }
        fs::create_dir_all(destination_dir).map_err(|_| {
            DirectError::new(
                "destination_unavailable",
                "error.transfer.destinationUnavailable",
            )
        })?;
        let final_path = noncollating_destination(destination_dir, &name);
        // Same filesystem: the move is one atomic rename. Staging and the
        // destination may live on different mounts; then the file is copied
        // to a temporary name on the destination's filesystem first, and the
        // final step is still one atomic rename onto the never-existing name
        // picked above.
        if fs::rename(&staged, &final_path).is_err() {
            let temporary = destination_dir.join(format!(".harbor-partial-{}", transfer.record.id));
            fs::copy(&staged, &temporary)
                .and_then(|_| fs::rename(&temporary, &final_path))
                .map_err(|_| {
                    DirectError::new(
                        "destination_unavailable",
                        "error.transfer.destinationUnavailable",
                    )
                })?;
            let _ = fs::remove_file(&staged);
        }
        transfer.record.phase = TransferPhase::Completed;
        Ok(final_path)
    }

    /// Marks a transfer canceled from either side and drops its runtime.
    pub fn cancel(&mut self, id: &str, staging_dir: &Path) -> bool {
        let staged = staging_dir.join(format!("{id}.part"));
        let _ = fs::remove_file(&staged);
        if let Some(transfer) = self.find_mut(id) {
            if matches!(
                transfer.record.phase,
                TransferPhase::Offered | TransferPhase::Active
            ) {
                transfer.record.phase = TransferPhase::Canceled;
            }
            return true;
        }
        false
    }

    /// Expires stale offers; a transfer nobody answered is canceled, not
    /// silently forgotten.
    pub fn expire(&mut self, now: u64) -> Vec<String> {
        let mut expired = Vec::new();
        for transfer in &mut self.active {
            if transfer.record.phase == TransferPhase::Offered && now > transfer.expires_at {
                transfer.record.phase = TransferPhase::Canceled;
                transfer.record.expired = true;
                expired.push(transfer.record.id.clone());
            }
        }
        expired
    }

    pub fn observe_accepted(&mut self, id: &str) -> bool {
        if let Some(transfer) = self.find_mut(id) {
            if transfer.record.direction == Direction::Outgoing
                && transfer.record.phase == TransferPhase::Offered
            {
                transfer.record.phase = TransferPhase::Active;
                return true;
            }
        }
        false
    }

    pub fn observe_canceled(&mut self, id: &str) -> bool {
        if let Some(transfer) = self.find_mut(id) {
            if matches!(
                transfer.record.phase,
                TransferPhase::Offered | TransferPhase::Active
            ) {
                transfer.record.phase = TransferPhase::Canceled;
                return true;
            }
        }
        false
    }

    /// The peer verified the digest: the delivery fact is now mutual.
    pub fn observe_peer_received(&mut self, id: &str) -> bool {
        if let Some(transfer) = self.find_mut(id) {
            if transfer.record.direction == Direction::Outgoing {
                transfer.record.peer_received = true;
                return true;
            }
        }
        false
    }

    /// Finished transfers stay visible as session facts; the board only
    /// compacts them away once they would crowd out live ones.
    pub fn compact(&mut self) {
        const BOARD_CAPACITY: usize = 20;
        while self.active.len() > BOARD_CAPACITY {
            let oldest_finished = self.active.iter().position(|t| {
                matches!(
                    t.record.phase,
                    TransferPhase::Completed | TransferPhase::Canceled | TransferPhase::Failed
                )
            });
            match oldest_finished {
                Some(index) => {
                    self.active.remove(index);
                }
                None => break,
            }
        }
    }

    pub fn records(&self) -> Vec<TransferRecord> {
        self.active.iter().map(|t| t.record.clone()).collect()
    }
}

/// Picks a destination path that never overwrites: `name`, then
/// `name (1).ext`, `name (2).ext`, …
fn noncollating_destination(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let stem;
    let extension;
    match name.rsplit_once('.') {
        Some((before, after)) if !before.is_empty() => {
            stem = before.to_string();
            extension = after.to_string();
        }
        _ => {
            stem = name.to_string();
            extension = String::new();
        }
    }
    for index in 1..10_000_u32 {
        let candidate = match extension.as_str() {
            "" => dir.join(format!("{stem} ({index})")),
            ext => dir.join(format!("{stem} ({index}).{ext}")),
        };
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{name}.refused-duplicate"))
}

fn hex_lower(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Free bytes on the volume holding `path`, or `None` when the platform
/// cannot say. Policy treats "unknown" as admit-with-ceiling — a missing
/// answer is never turned into a fabricated number.
pub(crate) fn available_bytes(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c_path = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        let rc = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
        if rc != 0 {
            return None;
        }
        Some(stat.f_bavail as u64 * stat.f_bsize as u64)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        None
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io::Write;

    use super::*;
    use uuid::Uuid;

    fn temporary_directory(label: &str) -> PathBuf {
        let directory =
            std::env::temp_dir().join(format!("harbor-direct-{label}-{}", Uuid::new_v4()));
        fs::create_dir_all(&directory).unwrap();
        directory
    }

    #[test]
    fn chat_strips_controls_and_refuses_empty_or_oversized_bodies() {
        assert_eq!(
            sanitize_message_body("hello\u{202e}\0world\n").unwrap(),
            "helloworld\n"
        );
        assert_eq!(
            sanitize_message_body("\t\n").unwrap_err().code,
            "chat_empty"
        );
        assert_eq!(
            sanitize_message_body(&"x".repeat(CHAT_MAX_BODY_BYTES + 1))
                .unwrap_err()
                .code,
            "chat_too_large"
        );
    }

    #[test]
    fn chat_queue_refuses_overflow_and_keeps_transcript_bounded() {
        let mut chat = ChatSession::new();
        for index in 0..CHAT_QUEUE_CAPACITY {
            chat.compose(format!("m-{index}"), "ready", index as u64)
                .unwrap();
        }
        assert_eq!(chat.queue_len(), CHAT_QUEUE_CAPACITY);
        assert_eq!(
            chat.compose("full".into(), "not silently lost", 100)
                .unwrap_err()
                .code,
            "chat_queue_full"
        );
        let sent = chat.drain_queue();
        assert_eq!(sent.len(), CHAT_QUEUE_CAPACITY);
        assert_eq!(chat.queue_len(), 0);

        for index in 0..(CHAT_TRANSCRIPT_CAPACITY + 10) {
            chat.record_inbound(
                format!("in-{index}"),
                "kept as plain text".into(),
                index as u64,
            );
        }
        let transcript = chat.snapshot();
        assert_eq!(transcript.len(), CHAT_TRANSCRIPT_CAPACITY);
        assert_eq!(transcript[0].id, "in-10");
    }

    #[test]
    fn filenames_cannot_traverse_or_become_device_names() {
        for bad in [
            "../secret.txt",
            "folder/file.txt",
            "folder\\file.txt",
            ".",
            "..",
            "CON",
            "nul.txt",
            "display\u{202e}gnp.exe",
            "name\0.txt",
        ] {
            assert!(sanitize_filename(bad).is_err(), "{bad:?} must be rejected");
        }
        assert_eq!(
            sanitize_filename(" safely named.png ").unwrap(),
            "safely named.png"
        );
    }

    #[test]
    fn inbound_transfer_verifies_hash_then_renames_without_overwrite() {
        let root = temporary_directory("receive");
        let staging = root.join("staging");
        let destination = root.join("downloads");
        fs::create_dir_all(&destination).unwrap();
        // A preexisting file proves finalization must choose a fresh name.
        fs::write(destination.join("notes.txt"), b"older file").unwrap();

        let bytes = b"direct transfer bytes";
        let sum = hex_lower(&Sha256::digest(bytes));
        let mut board = TransferBoard::new();
        board
            .offer_inbound("t-1".into(), "notes.txt", bytes.len() as u64, sum, 1)
            .unwrap();
        board.accept_inbound("t-1", &staging).unwrap();
        board
            .write_inbound_chunk("t-1", 0, 0, &bytes[..6], false)
            .unwrap();
        board
            .write_inbound_chunk("t-1", 1, 6, &bytes[6..], true)
            .unwrap();
        let saved = board
            .finalize_inbound("t-1", &staging, &destination)
            .unwrap();
        assert_eq!(
            saved.file_name().and_then(|name| name.to_str()),
            Some("notes (1).txt")
        );
        assert_eq!(fs::read(&saved).unwrap(), bytes);
        assert_eq!(
            fs::read(destination.join("notes.txt")).unwrap(),
            b"older file"
        );
        assert_eq!(board.records()[0].phase, TransferPhase::Completed);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mismatch_deletes_staging_and_never_produces_a_destination_file() {
        let root = temporary_directory("mismatch");
        let staging = root.join("staging");
        let destination = root.join("downloads");
        let mut board = TransferBoard::new();
        board
            .offer_inbound("t-2".into(), "safe.txt", 4, "0".repeat(64), 1)
            .unwrap();
        board.accept_inbound("t-2", &staging).unwrap();
        board
            .write_inbound_chunk("t-2", 0, 0, b"real", true)
            .unwrap();
        assert_eq!(
            board
                .finalize_inbound("t-2", &staging, &destination)
                .unwrap_err()
                .code,
            "sum_mismatch"
        );
        assert!(!staging.join("t-2.part").exists());
        assert!(!destination.join("safe.txt").exists());
        assert_eq!(board.records()[0].phase, TransferPhase::Failed);

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn outgoing_transfer_hashes_then_streams_bounded_chunks() {
        let root = temporary_directory("send");
        let source = root.join("report.bin");
        let bytes = vec![0x5a; TRANSFER_CHUNK_BYTES + 7];
        fs::File::create(&source)
            .unwrap()
            .write_all(&bytes)
            .unwrap();

        let mut board = TransferBoard::new();
        board.begin_outgoing("t-3".into(), &source, 1).unwrap();
        assert!(board.observe_accepted("t-3"));
        let (first, first_offset, first_bytes, first_final) =
            board.next_outgoing_chunk("t-3").unwrap().unwrap();
        assert_eq!(first, 0);
        assert_eq!(first_offset, 0);
        assert_eq!(first_bytes.len(), TRANSFER_CHUNK_BYTES);
        assert!(!first_final);
        assert!(board.confirm_outgoing_chunk("t-3", first));
        let (second, second_offset, second_bytes, second_final) =
            board.next_outgoing_chunk("t-3").unwrap().unwrap();
        assert_eq!(second, 1);
        assert_eq!(second_offset, TRANSFER_CHUNK_BYTES as u64);
        assert_eq!(second_bytes.len(), 7);
        assert!(second_final);
        assert!(board.confirm_outgoing_chunk("t-3", second));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn offers_expire_and_concurrency_has_a_hard_cap() {
        let root = temporary_directory("limits");
        let source = root.join("one.txt");
        fs::write(&source, b"one").unwrap();
        let mut board = TransferBoard::new();
        for index in 0..TRANSFER_CONCURRENT_LIMIT {
            board
                .begin_outgoing(format!("t-{index}"), &source, 1)
                .unwrap();
        }
        assert_eq!(
            board
                .begin_outgoing("overflow".into(), &source, 1)
                .unwrap_err()
                .code,
            "transfer_busy"
        );
        let expired = board.expire(1 + TRANSFER_OFFER_TTL_SECS + 1);
        assert_eq!(expired.len(), TRANSFER_CONCURRENT_LIMIT);
        assert!(board.records().iter().all(|record| record.expired));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn offers_beyond_the_shared_ceiling_are_refused() {
        let mut board = TransferBoard::new();
        let sum = "a".repeat(64);
        for size in [TRANSFER_MAX_BYTES + 1, u64::MAX] {
            assert_eq!(
                board
                    .offer_inbound("t-big".into(), "huge.iso", size, sum.clone(), 1)
                    .unwrap_err()
                    .code,
                "transfer_too_large",
                "{size} must be refused"
            );
        }
        // Nothing leaked into the board on refusal.
        assert!(board.records().is_empty());
    }

    #[test]
    fn sparse_source_beyond_the_old_limit_registers_and_streams() {
        let root = temporary_directory("sparse");
        let source = root.join("sparse.bin");
        let size = (256 + 4) << 20; // above the historical 256 MiB ceiling
        let file = fs::File::create(&source).unwrap();
        file.set_len(size).unwrap();
        drop(file);

        let mut board = TransferBoard::new();
        let record = board.begin_outgoing("t-sparse".into(), &source, 1).unwrap();
        assert_eq!(record.size, size);
        assert!(board.observe_accepted("t-sparse"));
        // The first chunks stream from a file that never fit in memory.
        let mut served = 0;
        let mut offset = 0;
        for _ in 0..3 {
            let (seq, chunk_offset, bytes, done) =
                board.next_outgoing_chunk("t-sparse").unwrap().unwrap();
            assert_eq!(seq, served);
            assert_eq!(chunk_offset, offset);
            assert_eq!(bytes.len(), TRANSFER_CHUNK_BYTES);
            assert!(!done);
            assert!(board.confirm_outgoing_chunk("t-sparse", seq));
            served += 1;
            offset += bytes.len() as u64;
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accept_refuses_when_staging_cannot_hold_the_file() {
        let root = temporary_directory("nostage");
        let staging = root.join("staging");
        let mut board = TransferBoard::new().with_space_probe(|_| Some(1024));
        board
            .offer_inbound("t-ns".into(), "big.bin", 4096, "a".repeat(64), 1)
            .unwrap();
        assert_eq!(
            board.accept_inbound("t-ns", &staging).unwrap_err().code,
            "no_space"
        );
        // The offer survives the refusal, so freeing space can still let it
        // through.
        assert_eq!(board.records()[0].phase, TransferPhase::Offered);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn accept_admits_when_space_cannot_be_measured() {
        let root = temporary_directory("unknown-space");
        let staging = root.join("staging");
        let mut board = TransferBoard::new().with_space_probe(|_| None);
        board
            .offer_inbound("t-uk".into(), "big.bin", 4096, "a".repeat(64), 1)
            .unwrap();
        board.accept_inbound("t-uk", &staging).unwrap();
        assert_eq!(board.records()[0].phase, TransferPhase::Active);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn finalize_keeps_staging_when_destination_cannot_hold_the_file() {
        let root = temporary_directory("nodest");
        let staging = root.join("staging");
        let destination = root.join("downloads");
        // The probe answers per-volume: staging is roomy, the destination
        // volume is full.
        let mut board = TransferBoard::new().with_space_probe(|path| {
            if path.ends_with("downloads") {
                Some(0)
            } else {
                Some(u64::MAX)
            }
        });
        let bytes = b"land later";
        board
            .offer_inbound(
                "t-nd".into(),
                "later.txt",
                bytes.len() as u64,
                hex_lower(&Sha256::digest(bytes)),
                1,
            )
            .unwrap();
        board.accept_inbound("t-nd", &staging).unwrap();
        board
            .write_inbound_chunk("t-nd", 0, 0, bytes, true)
            .unwrap();
        assert_eq!(
            board
                .finalize_inbound("t-nd", &staging, &destination)
                .unwrap_err()
                .code,
            "no_space"
        );
        // Refusal happened before anything moved: the staged bytes and the
        // Active phase both survive for a retry on a roomier volume.
        assert!(staging.join("t-nd.part").exists());
        assert!(!destination.exists() || destination.read_dir().unwrap().next().is_none());
        assert_eq!(board.records()[0].phase, TransferPhase::Active);
        let _ = fs::remove_dir_all(root);
    }
}
