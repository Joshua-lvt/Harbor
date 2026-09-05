//! Public-profile synchronization between paired peers.
//!
//! The local profile (display name, status message, avatar) is durable
//! local state owned by [`crate::settings`]. This module defines the
//! shareable subset — [`PublicProfile`] — plus the revision, validation,
//! chunking, and caching policy that carries it peer-to-peer over the
//! direct control channel. It never touches identity material: no private
//! keys, seeds, device secrets, or filesystem paths cross here. The avatar
//! travels as a `data:` URL (rebuilt by the peer from bytes, never from a
//! path), and a GIF keeps its animation because the bytes are preserved.
//!
//! Transport rules (enforced with the worker, which stays dumb):
//! - one `profile` action on the existing control channel, one inbox;
//! - every frame is small JSON (`PROFILE_FRAME_MAX_BYTES`), versioned;
//! - small avatars ride inline in the hello; larger ones are peer-paced
//!   chunk frames reassembled and hash-verified before publication;
//! - revisions are per-device monotonic: a hello applies only when its
//!   revision is newer than the stored one, so replays and reorders are
//!   ignored and reconnects converge on the newest state only.

use std::path::Path;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::storage;

/// Version of the peer-to-peer profile frame schema.
pub const PROFILE_PROTOCOL_VERSION: u64 = 1;
/// Largest single profile frame the control channel carries. Mirrors the
/// worker's inbound bound; the core never emits past it.
pub const PROFILE_FRAME_MAX_BYTES: usize = 8 * 1024;
/// Avatars at or below this data-URL size ride inline in the hello frame.
/// Larger ones use the peer-paced chunk flow.
pub const PROFILE_INLINE_AVATAR_MAX_BYTES: usize = 4096;
/// Raw avatar bytes per chunk frame (base64 inflates ~4/3 on the wire).
pub const PROFILE_AVATAR_CHUNK_RAW_BYTES: usize = 3072;
/// Chunk frames emitted per sync tick: paced, never a burst.
pub const PROFILE_AVATAR_CHUNKS_PER_TICK: usize = 8;
/// Upper bound on chunk count per avatar (well past the 4 MiB avatar cap).
pub const PROFILE_AVATAR_MAX_CHUNKS: usize = 2048;
/// Seconds between hello refreshes on a live call: idempotent by revision,
/// so a lost frame cannot strand a peer on stale state.
pub const PROFILE_HELLO_REFRESH_SECONDS: u64 = 60;
/// Durable partner snapshot: the last validated peer profile, so a restart
/// does not blank the partner until the peer sends something newer.
pub const PARTNER_PROFILE_FILE: &str = "partner-profile-v1.json";
const PARTNER_PROFILE_SCHEMA_VERSION: u16 = 1;

const DISPLAY_NAME_MAX_CHARS: usize = 80;
const STATUS_MESSAGE_MAX_CHARS: usize = 140;
/// Decoded avatar budget shared with the settings validator.
const AVATAR_MAX_BYTES: usize = 4 * 1024 * 1024;
const AVATAR_MIME_PREFIXES: &[&str] = &[
    "data:image/png;base64,",
    "data:image/jpeg;base64,",
    "data:image/jpg;base64,",
    "data:image/webp;base64,",
    "data:image/gif;base64,",
];

/// The shareable profile: public fields only. There is deliberately no
/// identity, key, path, or metadata field to leak.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PublicProfile {
    /// Schema version of this frame.
    pub v: u64,
    /// Per-device monotonic revision. Bumped on every local profile edit.
    pub revision: u64,
    pub display_name: String,
    pub status_message: String,
    /// `none` | `image` | `gif`. Empty avatar implies `none`.
    pub avatar_type: String,
    /// Lowercase hex SHA-256 over the avatar data-URL bytes. Empty when the
    /// avatar is empty, so a hash can never describe bytes it did not see.
    pub avatar_hash: String,
    /// Data URL (`data:image/...;base64,...`) or empty. Inline in a hello
    /// only when small; chunked otherwise; never a filesystem path.
    pub avatar: String,
}

/// The persisted view of the peer's last validated profile.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PartnerSnapshot {
    pub schema_version: u16,
    pub revision: u64,
    pub display_name: String,
    pub status_message: String,
    pub avatar_type: String,
    pub avatar_hash: String,
    pub avatar: String,
}

impl Default for PartnerSnapshot {
    fn default() -> Self {
        Self {
            schema_version: PARTNER_PROFILE_SCHEMA_VERSION,
            revision: 0,
            display_name: String::new(),
            status_message: String::new(),
            avatar_type: "none".into(),
            avatar_hash: String::new(),
            avatar: String::new(),
        }
    }
}

impl PartnerSnapshot {
    /// Loads the durable snapshot best-effort: anything unreadable,
    /// mis-versioned, or invalid means "no profile yet", never a failure.
    pub fn load(directory: &Path) -> Self {
        let path = directory.join(PARTNER_PROFILE_FILE);
        let bytes = std::fs::read(&path).unwrap_or_default();
        if bytes.is_empty() {
            return Self::default();
        }
        if crate::storage::require_private_file(&path).is_err() {
            return Self::default();
        }
        let snapshot: Self = serde_json::from_slice(&bytes).unwrap_or_default();
        if snapshot.schema_version != PARTNER_PROFILE_SCHEMA_VERSION {
            return Self::default();
        }
        // Stored bytes were validated on receipt, but the file is only a
        // cache: re-validate before trusting it after a restart. A hash
        // without bytes (fetch interrupted by the restart) is kept: the
        // next hello re-requests the bytes while fields stay valid.
        if snapshot.revision == 0 {
            return Self::default();
        }
        if !valid_label(&snapshot.display_name, DISPLAY_NAME_MAX_CHARS)
            || !valid_label(&snapshot.status_message, STATUS_MESSAGE_MAX_CHARS)
            || !matches!(snapshot.avatar_type.as_str(), "none" | "image" | "gif")
        {
            return Self::default();
        }
        if snapshot.avatar.is_empty() {
            // No bytes: either an explicit removal, or a hash whose fetch
            // the restart interrupted (re-requested on the next hello).
            let clean = (snapshot.avatar_type == "none" && snapshot.avatar_hash.is_empty())
                || (snapshot.avatar_type != "none"
                    && snapshot.avatar_hash.len() == 64
                    && snapshot.avatar_hash.bytes().all(|b| b.is_ascii_hexdigit()));
            if !clean {
                return Self::default();
            }
        } else if !valid_avatar_parts(
            &snapshot.avatar_type,
            &snapshot.avatar,
            &snapshot.avatar_hash,
        ) {
            return Self::default();
        }
        snapshot
    }

    pub fn persist(&self, directory: &Path) {
        let bytes = serde_json::to_vec(self).unwrap_or_default();
        if bytes.is_empty() {
            return;
        }
        let _ = storage::write_private_atomic(&directory.join(PARTNER_PROFILE_FILE), &bytes);
    }

    /// QML-facing view: public fields only, camelCase like the rest of the
    /// local IPC. Revision and hash stay below this boundary.
    pub fn json(&self) -> Value {
        json!({
            "displayName": self.display_name,
            "statusMessage": self.status_message,
            "avatarType": self.avatar_type,
            "avatar": self.avatar,
        })
    }
}

/// Lowercase hex SHA-256 over the avatar data-URL bytes. Empty avatar maps
/// to an empty hash so absence is never confused with content.
pub fn avatar_data_hash(avatar: &str) -> String {
    if avatar.is_empty() {
        return String::new();
    }
    let digest = Sha256::digest(avatar.as_bytes());
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        hex.push_str(&format!("{byte:02x}"));
    }
    hex
}

/// Builds the local public profile from durable settings values. Callers
/// pass the already-loaded values so this stays a pure function.
pub fn local_profile(
    revision: u64,
    display_name: &str,
    status_message: &str,
    avatar: &str,
    avatar_type: &str,
) -> PublicProfile {
    let avatar_type = if avatar.is_empty() {
        "none"
    } else if avatar_type == "gif" {
        "gif"
    } else {
        "image"
    };
    PublicProfile {
        v: PROFILE_PROTOCOL_VERSION,
        revision: revision.max(1),
        display_name: display_name.to_owned(),
        status_message: status_message.to_owned(),
        avatar_type: avatar_type.into(),
        avatar_hash: avatar_data_hash(avatar),
        avatar: avatar.to_owned(),
    }
}

/// Serializes one profile frame for the wire. The `t` discriminator keeps
/// the worker to a single action while the core multiplexes frame kinds.
pub fn hello_frame(profile: &PublicProfile, inline_avatar: bool) -> String {
    let avatar = if inline_avatar {
        profile.avatar.clone()
    } else {
        String::new()
    };
    json!({
        "t": "hello",
        "v": profile.v,
        "revision": profile.revision,
        "display_name": profile.display_name,
        "status_message": profile.status_message,
        "avatar_type": profile.avatar_type,
        "avatar_hash": profile.avatar_hash,
        "avatar": avatar,
    })
    .to_string()
}

pub fn avatar_offer_frame(revision: u64, hash: &str, size: usize, chunks: usize) -> String {
    json!({
        "t": "avatar_offer",
        "v": PROFILE_PROTOCOL_VERSION,
        "revision": revision,
        "hash": hash,
        "size": size,
        "chunks": chunks,
    })
    .to_string()
}

pub fn avatar_request_frame(revision: u64, hash: &str) -> String {
    json!({
        "t": "avatar_request",
        "v": PROFILE_PROTOCOL_VERSION,
        "revision": revision,
        "hash": hash,
    })
    .to_string()
}

pub fn avatar_chunk_frame(revision: u64, hash: &str, seq: usize, data_b64: &str) -> String {
    json!({
        "t": "avatar_chunk",
        "v": PROFILE_PROTOCOL_VERSION,
        "revision": revision,
        "hash": hash,
        "seq": seq,
        "data": data_b64,
    })
    .to_string()
}

pub fn avatar_cancel_frame(revision: u64, hash: &str) -> String {
    json!({
        "t": "avatar_cancel",
        "v": PROFILE_PROTOCOL_VERSION,
        "revision": revision,
        "hash": hash,
    })
    .to_string()
}

/// Splits avatar bytes into base64 chunk payloads for the wire.
pub fn fragment_avatar(avatar: &str) -> Vec<String> {
    avatar
        .as_bytes()
        .chunks(PROFILE_AVATAR_CHUNK_RAW_BYTES)
        .map(|chunk| STANDARD.encode(chunk))
        .collect()
}

/// A validated inbound hello: newer-than-stored state the caller may apply.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidHello {
    pub revision: u64,
    pub display_name: String,
    pub status_message: String,
    pub avatar_type: String,
    pub avatar_hash: String,
    /// Present only when the sender inlined a small avatar.
    pub avatar: String,
}

/// Validates one inbound hello frame. Anything malformed, oversized,
/// over-revisioned-backwards, or carrying non-public shapes is dropped.
/// Callers additionally compare `revision` against the stored snapshot.
pub fn validate_hello(raw: &str) -> Option<ValidHello> {
    if raw.len() > PROFILE_FRAME_MAX_BYTES {
        return None;
    }
    let frame: Value = serde_json::from_str(raw).ok()?;
    if frame.get("t").and_then(Value::as_str) != Some("hello") {
        return None;
    }
    if frame.get("v").and_then(Value::as_u64) != Some(PROFILE_PROTOCOL_VERSION) {
        return None;
    }
    let revision = frame.get("revision").and_then(Value::as_u64)?;
    if revision == 0 {
        return None;
    }
    let display_name = frame
        .get("display_name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let status_message = frame
        .get("status_message")
        .and_then(Value::as_str)
        .unwrap_or("");
    if !valid_label(display_name, DISPLAY_NAME_MAX_CHARS)
        || !valid_label(status_message, STATUS_MESSAGE_MAX_CHARS)
    {
        return None;
    }
    let avatar_type = frame.get("avatar_type").and_then(Value::as_str)?;
    if !matches!(avatar_type, "none" | "image" | "gif") {
        return None;
    }
    let avatar_hash = frame.get("avatar_hash").and_then(Value::as_str).unwrap_or("");
    let avatar = frame.get("avatar").and_then(Value::as_str).unwrap_or("");
    // A hello may reference a chunked avatar by hash alone; its bytes are
    // verified on arrival before anything publishes. Inline bytes verify now.
    if avatar.is_empty() {
        if avatar_type == "none" {
            if !avatar_hash.is_empty() {
                return None;
            }
        } else if avatar_hash.len() != 64
            || !avatar_hash.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return None;
        }
    } else if !valid_avatar_parts(avatar_type, avatar, avatar_hash) {
        return None;
    }
    Some(ValidHello {
        revision,
        display_name: display_name.to_owned(),
        status_message: status_message.to_owned(),
        avatar_type: avatar_type.into(),
        avatar_hash: avatar_hash.into(),
        avatar: avatar.into(),
    })
}

/// Human labels: bounded, no control characters, no invisible bidi games.
/// Empty is allowed and means "unknown", never a fabrication.
fn valid_label(value: &str, max_chars: usize) -> bool {
    if value.chars().count() > max_chars {
        return false;
    }
    !value.chars().any(|c| {
        c < '\u{20}' || c == '\u{7f}' || ('\u{200b}'..='\u{200f}').contains(&c) || ('\u{202a}'..='\u{202e}').contains(&c) || ('\u{2066}'..='\u{2069}').contains(&c)
    })
}

/// Avatar triple consistency: type/none-ness, hash presence, data-URL shape,
/// base64 health, decoded budget, and hash agreement. Paths can never pass:
/// only `data:image/...;base64,` prefixes are accepted.
fn valid_avatar_parts(avatar_type: &str, avatar: &str, avatar_hash: &str) -> bool {
    if avatar.is_empty() {
        return avatar_type == "none" && avatar_hash.is_empty();
    }
    if !matches!(avatar_type, "image" | "gif") || avatar_hash.is_empty() {
        return false;
    }
    if avatar_hash.len() != 64 || !avatar_hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return false;
    }
    let Some(prefix) = AVATAR_MIME_PREFIXES
        .iter()
        .find(|prefix| avatar.starts_with(**prefix))
    else {
        return false;
    };
    // A GIF-declared avatar must actually be GIF bytes, and vice versa.
    let is_gif = *prefix == "data:image/gif;base64,";
    if (avatar_type == "gif") != is_gif {
        return false;
    }
    let encoded = &avatar[prefix.len()..];
    if encoded.is_empty() || encoded.bytes().any(|b| b.is_ascii_whitespace()) {
        return false;
    }
    let Ok(decoded) = STANDARD.decode(encoded) else {
        return false;
    };
    if decoded.is_empty() || decoded.len() > AVATAR_MAX_BYTES {
        return false;
    }
    avatar_hash.eq_ignore_ascii_case(&avatar_data_hash(avatar))
}

/// Validated inbound avatar offer.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidOffer {
    pub revision: u64,
    pub hash: String,
    pub size: usize,
    pub chunks: usize,
}

pub fn validate_offer(raw: &str) -> Option<ValidOffer> {
    if raw.len() > PROFILE_FRAME_MAX_BYTES {
        return None;
    }
    let frame: Value = serde_json::from_str(raw).ok()?;
    if frame.get("t").and_then(Value::as_str) != Some("avatar_offer")
        || frame.get("v").and_then(Value::as_u64) != Some(PROFILE_PROTOCOL_VERSION)
    {
        return None;
    }
    let revision = frame.get("revision").and_then(Value::as_u64)?;
    let hash = frame.get("hash").and_then(Value::as_str)?;
    let size = frame.get("size").and_then(Value::as_u64)? as usize;
    let chunks = frame.get("chunks").and_then(Value::as_u64)? as usize;
    if revision == 0
        || hash.len() != 64
        || !hash.bytes().all(|b| b.is_ascii_hexdigit())
        || size == 0
        || size > AVATAR_MAX_BYTES + 1024
        || chunks == 0
        || chunks > PROFILE_AVATAR_MAX_CHUNKS
    {
        return None;
    }
    Some(ValidOffer {
        revision,
        hash: hash.to_owned(),
        size,
        chunks,
    })
}

/// Validated inbound avatar chunk.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidChunk {
    pub revision: u64,
    pub hash: String,
    pub seq: usize,
    pub data: String,
}

pub fn validate_chunk(raw: &str) -> Option<ValidChunk> {
    if raw.len() > PROFILE_FRAME_MAX_BYTES {
        return None;
    }
    let frame: Value = serde_json::from_str(raw).ok()?;
    if frame.get("t").and_then(Value::as_str) != Some("avatar_chunk")
        || frame.get("v").and_then(Value::as_u64) != Some(PROFILE_PROTOCOL_VERSION)
    {
        return None;
    }
    let revision = frame.get("revision").and_then(Value::as_u64)?;
    let hash = frame.get("hash").and_then(Value::as_str)?;
    let seq = frame.get("seq").and_then(Value::as_u64)? as usize;
    let data = frame.get("data").and_then(Value::as_str)?;
    if revision == 0
        || hash.len() != 64
        || !hash.bytes().all(|b| b.is_ascii_hexdigit())
        || data.is_empty()
        || data.len() > PROFILE_FRAME_MAX_BYTES
    {
        return None;
    }
    // One chunk carries bounded raw bytes; anything else is a peer bug.
    let Ok(decoded) = STANDARD.decode(data) else {
        return None;
    };
    if decoded.is_empty() || decoded.len() > PROFILE_AVATAR_CHUNK_RAW_BYTES {
        return None;
    }
    Some(ValidChunk {
        revision,
        hash: hash.to_owned(),
        seq,
        data: data.to_owned(),
    })
}

/// Validated avatar request / cancel reference.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidRef {
    pub revision: u64,
    pub hash: String,
}

pub fn validate_ref(raw: &str, kind: &str) -> Option<ValidRef> {
    if raw.len() > PROFILE_FRAME_MAX_BYTES {
        return None;
    }
    let frame: Value = serde_json::from_str(raw).ok()?;
    if frame.get("t").and_then(Value::as_str) != Some(kind)
        || frame.get("v").and_then(Value::as_u64) != Some(PROFILE_PROTOCOL_VERSION)
    {
        return None;
    }
    let revision = frame.get("revision").and_then(Value::as_u64)?;
    let hash = frame.get("hash").and_then(Value::as_str)?;
    if revision == 0 || hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(ValidRef {
        revision,
        hash: hash.to_owned(),
    })
}

/// In-progress inbound avatar reassembly. Published only whole and verified.
#[derive(Debug, Default)]
pub struct InboundAvatar {
    pub hash: String,
    pub size: usize,
    pub chunks: Vec<Option<String>>,
}

impl InboundAvatar {
    pub fn start(offer: &ValidOffer) -> Self {
        Self {
            hash: offer.hash.clone(),
            size: offer.size,
            chunks: vec![None; offer.chunks],
        }
    }

    /// Stores one chunk; returns the assembled avatar bytes once complete.
    pub fn store(&mut self, chunk: &ValidChunk) -> Option<String> {
        if chunk.hash != self.hash || chunk.seq >= self.chunks.len() {
            return None;
        }
        if self.chunks[chunk.seq].is_none() {
            self.chunks[chunk.seq] = Some(chunk.data.clone());
        }
        if self.chunks.iter().any(Option::is_none) {
            return None;
        }
        let mut raw = Vec::with_capacity(self.size);
        for slot in &self.chunks {
            raw.extend_from_slice(&STANDARD.decode(slot.as_ref().expect("complete")).unwrap_or_default());
        }
        if raw.len() != self.size {
            return None;
        }
        String::from_utf8(raw).ok()
    }
}

/// Tracks one outbound chunked avatar stream toward the peer.
#[derive(Debug, Default)]
pub struct OutboundAvatar {
    pub hash: String,
    pub revision: u64,
    pub fragments: Vec<String>,
    pub cursor: usize,
}

impl OutboundAvatar {
    pub fn start(revision: u64, avatar: &str) -> Self {
        Self {
            hash: avatar_data_hash(avatar),
            revision,
            fragments: fragment_avatar(avatar),
            cursor: 0,
        }
    }

    pub fn remaining(&self) -> usize {
        self.fragments.len().saturating_sub(self.cursor)
    }
}

/// Live synchronization state for one core process. The partner snapshot is
/// durable; everything else is per-call transient and resets on disconnect.
#[derive(Debug, Default)]
pub struct ProfileSync {
    pub partner: PartnerSnapshot,
    pub last_sent_revision: u64,
    pub last_hello_at: u64,
    pub requested_hash: Option<String>,
    pub inbound: Option<InboundAvatar>,
    pub outbound: Option<OutboundAvatar>,
}

/// Outcome of ingesting one inbound frame: whether partner state changed,
/// plus response frames the transport should emit this tick.
#[derive(Debug, Default)]
pub struct IngestOutcome {
    pub applied: bool,
    pub send: Vec<String>,
}

impl ProfileSync {
    pub fn load(directory: &Path) -> Self {
        Self {
            partner: PartnerSnapshot::load(directory),
            ..Self::default()
        }
    }

    pub fn persist_partner(&self, directory: &Path) {
        self.partner.persist(directory);
    }

    /// A call ending drops every in-flight exchange and forces a fresh
    /// hello on the next call, so reconnects converge instead of resuming
    /// stale cursors. The durable snapshot is untouched.
    pub fn note_disconnected(&mut self) {
        self.last_sent_revision = 0;
        self.last_hello_at = 0;
        self.requested_hash = None;
        self.inbound = None;
        self.outbound = None;
    }

    /// The hello to emit this tick, if the peer has not seen this revision
    /// or a refresh is due. Idempotent by revision on receipt.
    pub fn pending_hello(&self, local: &PublicProfile, now: u64) -> Option<String> {
        if local.revision != self.last_sent_revision
            || now.saturating_sub(self.last_hello_at) >= PROFILE_HELLO_REFRESH_SECONDS
        {
            let inline = local.avatar.len() <= PROFILE_INLINE_AVATAR_MAX_BYTES;
            let frame = hello_frame(local, inline);
            if frame.len() <= PROFILE_FRAME_MAX_BYTES {
                return Some(frame);
            }
        }
        None
    }

    pub fn mark_hello_sent(&mut self, revision: u64, now: u64) {
        self.last_sent_revision = revision;
        self.last_hello_at = now;
    }

    /// Routes one inbound frame. Only a hello newer than the stored snapshot
    /// changes partner state; replays, reorders and duplicates are ignored.
    /// Avatar bytes publish only whole and hash-verified.
    pub fn ingest(&mut self, raw: &str, local: &PublicProfile) -> IngestOutcome {
        let mut outcome = IngestOutcome::default();
        let kind = serde_json::from_str::<Value>(raw)
            .ok()
            .and_then(|frame| frame.get("t").and_then(Value::as_str).map(str::to_owned));
        match kind.as_deref() {
            Some("hello") => {
                let Some(valid) = validate_hello(raw) else {
                    return outcome;
                };
                if valid.revision > self.partner.revision {
                    self.partner.revision = valid.revision;
                    self.partner.display_name = valid.display_name.clone();
                    self.partner.status_message = valid.status_message.clone();
                    self.partner.avatar_type = valid.avatar_type.clone();
                    if valid.avatar_hash == avatar_data_hash(&self.partner.avatar)
                        && !valid.avatar_hash.is_empty()
                    {
                        // Same bytes we already hold: no reprocess, no refetch.
                    } else if valid.avatar.is_empty() {
                        // Removal (or a chunked avatar): clear until bytes
                        // arrive. A removal carries no hash and clears any
                        // in-flight fetch with it.
                        self.partner.avatar.clear();
                        self.partner.avatar_hash.clear();
                        if valid.avatar_hash.is_empty() {
                            self.requested_hash = None;
                            self.inbound = None;
                        }
                    } else {
                        // Inline avatar: hash agreement already verified.
                        self.partner.avatar = valid.avatar.clone();
                        self.partner.avatar_hash = valid.avatar_hash.clone();
                    }
                    outcome.applied = true;
                }
                // Missing bytes are (re)requested even on a duplicate hello:
                // a refresh resumes a fetch lost to a restart or a dropped
                // request, while fields stay pinned to the newest revision.
                if !valid.avatar_hash.is_empty()
                    && valid.avatar_hash != avatar_data_hash(&self.partner.avatar)
                    && self.requested_hash.as_deref() != Some(valid.avatar_hash.as_str())
                {
                    self.requested_hash = Some(valid.avatar_hash.clone());
                    self.inbound = None;
                    outcome
                        .send
                        .push(avatar_request_frame(local.revision, &valid.avatar_hash));
                }
            }
            Some("avatar_offer") => {
                let Some(offer) = validate_offer(raw) else {
                    return outcome;
                };
                if self.requested_hash.as_deref() == Some(offer.hash.as_str())
                    && offer.revision == self.partner.revision
                {
                    self.inbound = Some(InboundAvatar::start(&offer));
                }
            }
            Some("avatar_chunk") => {
                let Some(chunk) = validate_chunk(raw) else {
                    return outcome;
                };
                let complete = match self.inbound.as_mut() {
                    Some(inbound) if chunk.hash == inbound.hash => inbound.store(&chunk),
                    _ => None,
                };
                if let Some(assembled) = complete {
                    if valid_avatar_parts(&self.partner.avatar_type, &assembled, &chunk.hash) {
                        self.partner.avatar = assembled;
                        self.partner.avatar_hash = chunk.hash.clone();
                        outcome.applied = true;
                    }
                    self.inbound = None;
                    self.requested_hash = None;
                }
            }
            Some("avatar_request") => {
                let Some(request) = validate_ref(raw, "avatar_request") else {
                    return outcome;
                };
                if !local.avatar.is_empty() && request.hash == local.avatar_hash {
                    let stream = OutboundAvatar::start(local.revision, &local.avatar);
                    let total = stream.fragments.len();
                    outcome.send.push(avatar_offer_frame(
                        local.revision,
                        &stream.hash,
                        local.avatar.len(),
                        total,
                    ));
                    self.outbound = Some(stream);
                } else {
                    outcome
                        .send
                        .push(avatar_cancel_frame(local.revision, &request.hash));
                }
            }
            Some("avatar_cancel") => {
                let Some(cancel) = validate_ref(raw, "avatar_cancel") else {
                    return outcome;
                };
                if self.inbound.as_ref().is_some_and(|i| i.hash == cancel.hash) {
                    self.inbound = None;
                }
                if self.requested_hash.as_deref() == Some(cancel.hash.as_str()) {
                    self.requested_hash = None;
                }
                if self.outbound.as_ref().is_some_and(|o| o.hash == cancel.hash) {
                    self.outbound = None;
                }
            }
            _ => {}
        }
        outcome
    }

    /// A stale outbound stream (local avatar moved on) declines itself so
    /// the peer stops waiting on superseded bytes.
    pub fn stale_outbound_cancel(&mut self, local: &PublicProfile) -> Option<String> {
        let stale = match self.outbound.as_ref() {
            Some(stream) => stream.hash != local.avatar_hash || local.avatar.is_empty(),
            None => false,
        };
        if !stale {
            return None;
        }
        let hash = self.outbound.as_ref().expect("stale").hash.clone();
        self.outbound = None;
        Some(avatar_cancel_frame(local.revision, &hash))
    }

    /// Next chunk frame without advancing: the caller advances only after
    /// the worker accepts the send, so a failed tick retries in place.
    pub fn peek_chunk(&self, local: &PublicProfile) -> Option<String> {
        let stream = self.outbound.as_ref()?;
        if stream.hash != local.avatar_hash || local.avatar.is_empty() {
            return None;
        }
        let data = stream.fragments.get(stream.cursor)?;
        Some(avatar_chunk_frame(
            stream.revision,
            &stream.hash,
            stream.cursor,
            data,
        ))
    }

    pub fn advance_chunk(&mut self) {
        if let Some(stream) = self.outbound.as_mut() {
            stream.cursor = stream.cursor.saturating_add(1);
            if stream.cursor >= stream.fragments.len() {
                self.outbound = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIF_AVATAR: &str = "data:image/gif;base64,R0lGODlhAQABAIAAAP///wAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw==";
    const PNG_AVATAR: &str = "data:image/png;base64,AA==";

    #[test]
    fn empty_avatar_hashes_to_empty_and_types_to_none() {
        assert_eq!(avatar_data_hash(""), "");
        let profile = local_profile(1, "Ari", "", "", "image");
        assert_eq!(profile.avatar_type, "none");
        assert_eq!(profile.avatar_hash, "");
        assert_eq!(profile.revision, 1);
    }

    #[test]
    fn revision_zero_clamps_to_one() {
        let profile = local_profile(0, "", "", "", "image");
        assert_eq!(profile.revision, 1);
    }

    #[test]
    fn hello_round_trip_validates() {
        let profile = local_profile(7, "Ari", "Building", PNG_AVATAR, "image");
        let frame = hello_frame(&profile, true);
        assert!(frame.len() <= PROFILE_FRAME_MAX_BYTES);
        let valid = validate_hello(&frame).expect("own hello must validate");
        assert_eq!(valid.revision, 7);
        assert_eq!(valid.display_name, "Ari");
        assert_eq!(valid.avatar, PNG_AVATAR);
        assert_eq!(valid.avatar_hash, avatar_data_hash(PNG_AVATAR));
    }

    #[test]
    fn gif_avatar_preserved_through_validation() {
        let profile = local_profile(3, "Ari", "", GIF_AVATAR, "gif");
        let frame = hello_frame(&profile, true);
        let valid = validate_hello(&frame).expect("gif hello must validate");
        assert_eq!(valid.avatar_type, "gif");
        assert_eq!(valid.avatar, GIF_AVATAR);
    }

    #[test]
    fn malformed_hellos_are_rejected() {
        // Wrong version.
        assert!(validate_hello(r#"{"t":"hello","v":2,"revision":1,"avatar_type":"none","avatar_hash":"","avatar":""}"#).is_none());
        // Zero revision.
        assert!(validate_hello(r#"{"t":"hello","v":1,"revision":0,"avatar_type":"none","avatar_hash":"","avatar":""}"#).is_none());
        // Filesystem paths can never pass as avatars.
        let path = r#"{"t":"hello","v":1,"revision":1,"avatar_type":"image","avatar_hash":"aa","avatar":"/home/joshy/Pictures/a.png"}"#;
        assert!(validate_hello(path).is_none());
        // Declared image carrying GIF bytes (and vice versa) is rejected.
        let profile = local_profile(1, "", "", GIF_AVATAR, "image");
        assert!(validate_hello(&hello_frame(&profile, true)).is_none());
        // Hash mismatch is rejected.
        let forged = r#"{"t":"hello","v":1,"revision":1,"avatar_type":"image","avatar_hash":"0000000000000000000000000000000000000000000000000000000000000000","avatar":"data:image/png;base64,AA=="}"#;
        assert!(validate_hello(forged).is_none());
        // Control characters in labels are rejected.
        let control = "{\"t\":\"hello\",\"v\":1,\"revision\":1,\"display_name\":\"a\u{0007}b\",\"avatar_type\":\"none\",\"avatar_hash\":\"\",\"avatar\":\"\"}";
        assert!(validate_hello(control).is_none());
        // Oversized display names are rejected.
        let long = format!("{{\"t\":\"hello\",\"v\":1,\"revision\":1,\"display_name\":\"{}\",\"avatar_type\":\"none\",\"avatar_hash\":\"\",\"avatar\":\"\"}}", "x".repeat(81));
        assert!(validate_hello(&long).is_none());
        // Unknown frame kinds and garbage are rejected.
        assert!(validate_hello(r#"{"t":"halo","v":1,"revision":1}"#).is_none());
        assert!(validate_hello("not json").is_none());
        assert!(validate_hello("").is_none());
    }

    #[test]
    fn avatar_fragments_reassemble_byte_identical() {
        let avatar = format!("data:image/png;base64,{}", "QUJD".repeat(3000));
        let fragments = fragment_avatar(&avatar);
        assert!(fragments.len() > 1);
        let mut inbound = InboundAvatar {
            hash: avatar_data_hash(&avatar),
            size: avatar.len(),
            chunks: vec![None; fragments.len()],
        };
        for (seq, data) in fragments.iter().enumerate() {
            let chunk = ValidChunk {
                revision: 2,
                hash: inbound.hash.clone(),
                seq,
                data: data.clone(),
            };
            // Duplicates are harmless.
            let _ = inbound.store(&chunk);
            let done = inbound.store(&chunk);
            if seq + 1 == fragments.len() {
                assert_eq!(done.as_deref(), Some(avatar.as_str()));
            } else {
                assert!(done.is_none());
            }
        }
    }

    #[test]
    fn reassembly_rejects_size_mismatch() {
        let fragments = fragment_avatar(PNG_AVATAR);
        let mut inbound = InboundAvatar {
            hash: avatar_data_hash(PNG_AVATAR),
            size: PNG_AVATAR.len() + 1,
            chunks: vec![None; fragments.len()],
        };
        let chunk = ValidChunk {
            revision: 1,
            hash: inbound.hash.clone(),
            seq: 0,
            data: fragments[0].clone(),
        };
        assert!(inbound.store(&chunk).is_none());
    }

    #[test]
    fn hash_only_hello_validates_for_chunked_avatars() {
        let avatar = format!("data:image/png;base64,{}", "QUJD".repeat(2000));
        let profile = local_profile(2, "Ari", "", &avatar, "image");
        assert!(avatar.len() > PROFILE_INLINE_AVATAR_MAX_BYTES);
        let frame = hello_frame(&profile, false);
        let valid = validate_hello(&frame).expect("hash-only hello must validate");
        assert_eq!(valid.avatar, "");
        assert_eq!(valid.avatar_hash, avatar_data_hash(&avatar));
    }

    #[test]
    fn duplicate_hello_resumes_a_missing_avatar_fetch() {
        let avatar = format!("data:image/png;base64,{}", "QUJD".repeat(2000));
        let peer = local_profile(2, "Taylor", "", &avatar, "image");
        let hello = hello_frame(&peer, false);
        let local = local_profile(1, "Me", "", "", "image");
        let mut sync = ProfileSync::default();

        // First hello applies fields and asks for the bytes.
        let first = sync.ingest(&hello, &local);
        assert!(first.applied);
        assert_eq!(sync.partner.display_name, "Taylor");
        assert!(sync.partner.avatar.is_empty());
        assert_eq!(first.send.len(), 1);

        // The answer is lost; a duplicate hello (refresh) asks again
        // without touching fields — no replay, no rewind.
        sync.requested_hash = None;
        let second = sync.ingest(&hello, &local);
        assert!(!second.applied);
        assert_eq!(second.send.len(), 1);
        assert_eq!(sync.partner.display_name, "Taylor");
    }

    #[test]
    fn bogus_offers_and_chunks_are_rejected() {
        assert!(validate_offer(r#"{"t":"avatar_offer","v":1,"revision":1,"hash":"zz","size":10,"chunks":1}"#).is_none());
        assert!(validate_offer(r#"{"t":"avatar_offer","v":1,"revision":1,"hash":"0000000000000000000000000000000000000000000000000000000000000000","size":0,"chunks":1}"#).is_none());
        assert!(validate_chunk(r#"{"t":"avatar_chunk","v":1,"revision":1,"hash":"0000000000000000000000000000000000000000000000000000000000000000","seq":0,"data":"!!!"}"#).is_none());
        assert!(validate_ref(r#"{"t":"avatar_request","v":1,"revision":1,"hash":"short"}"#, "avatar_request").is_none());
        assert!(validate_ref(r#"{"t":"avatar_request","v":1,"revision":1,"hash":"0000000000000000000000000000000000000000000000000000000000000000"}"#, "avatar_cancel").is_none());
    }

    #[test]
    fn partner_snapshot_persists_and_reloads() {
        let directory = std::env::temp_dir().join(format!("harbor-profile-cache-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let snapshot = PartnerSnapshot {
            schema_version: PARTNER_PROFILE_SCHEMA_VERSION,
            revision: 4,
            display_name: "Taylor".into(),
            status_message: "Away".into(),
            avatar_type: "gif".into(),
            avatar_hash: avatar_data_hash(GIF_AVATAR),
            avatar: GIF_AVATAR.into(),
        };
        snapshot.persist(&directory);
        assert_eq!(PartnerSnapshot::load(&directory), snapshot);
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn corrupt_snapshot_loads_empty() {
        let directory = std::env::temp_dir().join(format!("harbor-profile-cache-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join(PARTNER_PROFILE_FILE), b"garbage").unwrap();
        assert_eq!(PartnerSnapshot::load(&directory), PartnerSnapshot::default());
        let _ = std::fs::remove_dir_all(&directory);
    }
}
