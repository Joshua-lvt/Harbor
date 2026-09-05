//! Versioned, bounded control-plane messages shared by Harbor processes.
//!
//! The protocol deliberately has no message variants for media frames, chat
//! bodies, DataChannel payloads, or file chunks. Those data planes must remain
//! direct peer-to-peer concerns when WebRTC is introduced.

use std::collections::BTreeSet;
use std::io::{Read, Write};

use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;

pub const VERSION: u16 = 1;
pub const MAX_FRAME_BYTES: usize = 1024 * 1024;

const NETWORK_CONTROL_MESSAGE_TYPES: &[&str] = &[
    "core.hello",
    "core.ready",
    "core.shutdown",
    "identity.get",
    "identity.update",
    "settings.get",
    "settings.update",
    "server.config",
    "server.configure",
    "pairing.create",
    "pairing.submit",
    "pairing.cancel",
    "pairing.incoming",
    "pairing.accept",
    "pairing.decline",
    "pairing.status",
    "pairing.state",
    "pairing.enter_code",
    "pairing.reset",
    "presence.publish",
    "presence.changed",
    // Paired read of a peer's presence lease; the aggregate answer is the
    // only presence fact that ever travels on this transport.
    "presence.status",
    "session.connect",
    "session.disconnect",
    "session.signal",
    "session.signal_poll",
    "contacts.list",
];

// The UI↔core channel is private, but still uses an explicit allowlist. These
// types never become valid signed messages on the Harbor Server transport.
const LOCAL_MESSAGE_TYPES: &[&str] = &[
    "core.hello",
    "core.ready",
    "core.shutdown",
    "identity.get",
    "identity.update",
    "settings.get",
    "settings.update",
    "server.config",
    "server.configure",
    "pairing.create",
    "pairing.submit",
    "pairing.cancel",
    "pairing.incoming",
    "pairing.accept",
    "pairing.decline",
    "pairing.status",
    "pairing.state",
    "pairing.enter_code",
    "pairing.reset",
    // Safe paired-peer metadata (device id and public Harbor id) is exposed
    // locally so first-run gating does not invent a client-only paired flag.
    "contacts.list",
    "activity.state",
    "activity.updated",
    // Local presence plumbing mirrors the activity boundary: the Qt side's
    // native detector pushes private UserActivitySnapshot facts with
    // presence.sense, the UI reads the committed aggregate with presence.state,
    // and transitions surface as the unsolicited presence.updated event. None
    // of these are valid on the network transport — only the aggregated
    // ONLINE/AWAY/OFFLINE lease state ever leaves the machine.
    "presence.sense",
    "presence.state",
    "presence.updated",
    "presence.publish",
    "presence.changed",
    "session.connect",
    "session.disconnect",
    "session.signal",
    "call.start",
    "call.accept",
    "call.decline",
    "call.end",
    "call.mute",
    "call.push_to_talk",
    "call.state_changed",
    "call.share_screen_start",
    "call.share_screen_stop",
    "call.share_state_changed",
    // Audio enumeration/control and live voice facts are call-local facts;
    // they never travel to the control-plane server.
    "audio.devices",
    "audio.config",
    "audio.loopback_start",
    "audio.loopback_poll",
    "audio.loopback_stop",
    "voice.level",
    // Direct DataChannel controls are private UI↔core messages only. They
    // are intentionally absent from NETWORK_CONTROL_MESSAGE_TYPES.
    "chat.send",
    "direct.state",
    "direct.updated",
    // Public-profile sync rides the paired direct channel only: the local
    // snapshot for the UI and the peer-delivered update event. Both are
    // intentionally absent from NETWORK_CONTROL_MESSAGE_TYPES.
    "profile.state",
    "profile.updated",
    "transfer.offer_local",
    "transfer.accept",
    "transfer.reject",
    "transfer.cancel",
    // Device endpoints of an identity (Harbor Mobile): the per-install
    // device type plus the companion registry snapshot. Local only — the
    // server never parses device claims.
    "device.state",
    "device.updated",
    "device.configure",
    // Aggregated phone state (battery, activity, location, notification
    // sharing intents). Local IPC and P2P only, never the server.
    "mobile.state",
    "mobile.updated",
    "mobile.update",
    // Explicit media-endpoint handoff between two own devices. Local
    // policy; the resulting call moves ride the existing call messages.
    "call.takeover",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolError {
    pub code: String,
    pub ui_key: String,
    pub retryable: bool,
    pub detail: String,
}

impl ProtocolError {
    pub fn invalid_request(detail: impl Into<String>) -> Self {
        Self {
            code: "invalid_request".into(),
            ui_key: "error.protocol.invalidRequest".into(),
            retryable: false,
            detail: detail.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Envelope {
    #[serde(rename = "v")]
    pub version: u16,
    #[serde(rename = "type")]
    pub message_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    pub payload: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

impl Envelope {
    pub fn request(
        message_type: impl Into<String>,
        payload: Value,
        timestamp: impl Into<String>,
    ) -> Self {
        Self {
            version: VERSION,
            message_type: message_type.into(),
            request_id: Some(Uuid::new_v4().to_string()),
            event_id: None,
            timestamp: Some(timestamp.into()),
            reply_to: None,
            payload,
            error: None,
        }
    }

    pub fn event(
        message_type: impl Into<String>,
        payload: Value,
        timestamp: impl Into<String>,
    ) -> Self {
        Self {
            version: VERSION,
            message_type: message_type.into(),
            request_id: None,
            event_id: Some(Uuid::new_v4().to_string()),
            timestamp: Some(timestamp.into()),
            reply_to: None,
            payload,
            error: None,
        }
    }

    pub fn response_to(
        request: &Envelope,
        message_type: impl Into<String>,
        payload: Value,
        timestamp: impl Into<String>,
    ) -> Result<Self, ValidationError> {
        let request_id = request
            .request_id
            .clone()
            .ok_or(ValidationError::MissingRequestId)?;
        Ok(Self {
            version: VERSION,
            message_type: message_type.into(),
            request_id: Some(Uuid::new_v4().to_string()),
            event_id: None,
            timestamp: Some(timestamp.into()),
            reply_to: Some(request_id),
            payload,
            error: None,
        })
    }

    /// Validates an envelope permitted on the private UI↔core channel.
    pub fn validate(&self) -> Result<(), ValidationError> {
        self.validate_for(LOCAL_MESSAGE_TYPES)
    }

    /// Validates an envelope permitted on the signed Harbor Server transport.
    /// Local activity and call state are deliberately excluded.
    pub fn validate_network(&self) -> Result<(), ValidationError> {
        self.validate_for(NETWORK_CONTROL_MESSAGE_TYPES)
    }

    fn validate_for(&self, allowed_types: &[&str]) -> Result<(), ValidationError> {
        if self.version != VERSION {
            return Err(ValidationError::UnsupportedVersion(self.version));
        }
        if !allowed_types.contains(&self.message_type.as_str()) {
            return Err(ValidationError::ForbiddenMessageType(
                self.message_type.clone(),
            ));
        }
        if !self.payload.is_object() {
            return Err(ValidationError::PayloadMustBeObject);
        }
        validate_id("request_id", self.request_id.as_deref())?;
        validate_id("event_id", self.event_id.as_deref())?;
        validate_id("reply_to", self.reply_to.as_deref())?;

        if self.request_id.is_some() && self.event_id.is_some() {
            return Err(ValidationError::AmbiguousCorrelation);
        }
        if self.request_id.is_none() && self.event_id.is_none() {
            return Err(ValidationError::MissingCorrelation);
        }
        if self.timestamp.as_deref().is_none_or(str::is_empty) {
            return Err(ValidationError::MissingTimestamp);
        }
        if let Some(error) = &self.error {
            if error.code.is_empty() || error.ui_key.is_empty() {
                return Err(ValidationError::InvalidError);
            }
        }
        Ok(())
    }

    pub fn capabilities(&self) -> BTreeSet<&str> {
        self.payload
            .get("capabilities")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AuthenticatedEnvelope {
    pub signer_id: Uuid,
    pub envelope: Envelope,
    pub signature: String,
}

#[derive(Serialize)]
struct SigningPayload<'a> {
    signer_id: Uuid,
    envelope: &'a Envelope,
}

impl AuthenticatedEnvelope {
    pub fn sign(
        signer_id: Uuid,
        envelope: Envelope,
        signing_key: &SigningKey,
    ) -> Result<Self, AuthenticationError> {
        envelope
            .validate_network()
            .map_err(AuthenticationError::InvalidEnvelope)?;
        let payload = signing_payload(signer_id, &envelope)?;
        let signature = signing_key.sign(&payload);
        Ok(Self {
            signer_id,
            envelope,
            signature: STANDARD_NO_PAD.encode(signature.to_bytes()),
        })
    }

    pub fn verify(&self, public_key: &str) -> Result<(), AuthenticationError> {
        self.envelope
            .validate_network()
            .map_err(AuthenticationError::InvalidEnvelope)?;
        let key_bytes = STANDARD_NO_PAD
            .decode(public_key)
            .map_err(|_| AuthenticationError::InvalidPublicKey)?;
        let key_bytes: [u8; 32] = key_bytes
            .try_into()
            .map_err(|_| AuthenticationError::InvalidPublicKey)?;
        let verifying_key = VerifyingKey::from_bytes(&key_bytes)
            .map_err(|_| AuthenticationError::InvalidPublicKey)?;
        let signature_bytes = STANDARD_NO_PAD
            .decode(&self.signature)
            .map_err(|_| AuthenticationError::InvalidSignature)?;
        let signature = Signature::from_slice(&signature_bytes)
            .map_err(|_| AuthenticationError::InvalidSignature)?;
        verifying_key
            .verify(
                &signing_payload(self.signer_id, &self.envelope)?,
                &signature,
            )
            .map_err(|_| AuthenticationError::InvalidSignature)
    }
}

fn signing_payload(signer_id: Uuid, envelope: &Envelope) -> Result<Vec<u8>, AuthenticationError> {
    serde_json::to_vec(&SigningPayload {
        signer_id,
        envelope,
    })
    .map_err(|error| AuthenticationError::Serialization(error.to_string()))
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuthenticationError {
    #[error(transparent)]
    InvalidEnvelope(ValidationError),
    #[error("identity public key is invalid")]
    InvalidPublicKey,
    #[error("envelope signature is invalid")]
    InvalidSignature,
    #[error("could not serialize signed envelope: {0}")]
    Serialization(String),
}

fn validate_id(field: &'static str, value: Option<&str>) -> Result<(), ValidationError> {
    if let Some(value) = value {
        Uuid::parse_str(value).map_err(|_| ValidationError::InvalidId {
            field,
            value: value.to_owned(),
        })?;
    }
    Ok(())
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ValidationError {
    #[error("unsupported protocol version {0}")]
    UnsupportedVersion(u16),
    #[error("message type is not in the control-plane allowlist: {0}")]
    ForbiddenMessageType(String),
    #[error("payload must be a JSON object")]
    PayloadMustBeObject,
    #[error("{field} must be a UUID: {value}")]
    InvalidId { field: &'static str, value: String },
    #[error("an envelope must have exactly one request_id or event_id")]
    AmbiguousCorrelation,
    #[error("an envelope must have a request_id or event_id")]
    MissingCorrelation,
    #[error("timestamp is required")]
    MissingTimestamp,
    #[error("structured errors require a code and ui_key")]
    InvalidError,
    #[error("a response can only be made to a request")]
    MissingRequestId,
}

#[derive(Debug, Error)]
pub enum FrameError {
    #[error("frame payload is empty")]
    EmptyFrame,
    #[error("frame payload exceeds the configured frame limit")]
    OversizedFrame,
    #[error("stream ended with a truncated frame")]
    TruncatedFrame,
    #[error("invalid UTF-8 in protocol frame")]
    InvalidUtf8,
    #[error("invalid JSON protocol frame: {0}")]
    InvalidJson(String),
    #[error(transparent)]
    InvalidEnvelope(#[from] ValidationError),
    #[error("frame stream I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

/// Error comparisons ignore I/O details so tests can keep asserting variants.
impl PartialEq for FrameError {
    fn eq(&self, other: &Self) -> bool {
        use FrameError::*;
        matches!(
            (self, other),
            (EmptyFrame, EmptyFrame)
                | (OversizedFrame, OversizedFrame)
                | (TruncatedFrame, TruncatedFrame)
                | (InvalidUtf8, InvalidUtf8)
                | (InvalidJson(_), InvalidJson(_))
                | (InvalidEnvelope(_), InvalidEnvelope(_))
                | (Io(_), Io(_))
        ) && match (self, other) {
            (InvalidJson(left), InvalidJson(right)) => left == right,
            (InvalidEnvelope(left), InvalidEnvelope(right)) => left == right,
            _ => true,
        }
    }
}

impl Eq for FrameError {}

/// The announced payload length of a frame, validated before any allocation.
fn frame_length(prefix: &[u8; 4], limit: usize) -> Result<usize, FrameError> {
    let length = u32::from_be_bytes(*prefix) as usize;
    if length == 0 {
        return Err(FrameError::EmptyFrame);
    }
    if length > limit {
        return Err(FrameError::OversizedFrame);
    }
    Ok(length)
}

pub fn encode_frame(envelope: &Envelope) -> Result<Vec<u8>, FrameError> {
    envelope.validate()?;
    let json =
        serde_json::to_vec(envelope).map_err(|error| FrameError::InvalidJson(error.to_string()))?;
    if json.is_empty() {
        return Err(FrameError::EmptyFrame);
    }
    if json.len() > MAX_FRAME_BYTES {
        return Err(FrameError::OversizedFrame);
    }

    let length = u32::try_from(json.len()).map_err(|_| FrameError::OversizedFrame)?;
    let mut frame = Vec::with_capacity(4 + json.len());
    frame.extend_from_slice(&length.to_be_bytes());
    frame.extend_from_slice(&json);
    Ok(frame)
}

#[derive(Debug, Default)]
pub struct FrameDecoder {
    pending: Vec<u8>,
}

impl FrameDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Envelope>, FrameError> {
        self.pending.extend_from_slice(bytes);
        let mut messages = Vec::new();

        loop {
            if self.pending.len() < 4 {
                break;
            }
            let length = frame_length(
                &self.pending[..4].try_into().expect("four-byte prefix"),
                MAX_FRAME_BYTES,
            )?;
            let total = 4 + length;
            if self.pending.len() < total {
                break;
            }

            let payload = self.pending[4..total].to_vec();
            self.pending.drain(..total);
            let text = std::str::from_utf8(&payload).map_err(|_| FrameError::InvalidUtf8)?;
            let envelope: Envelope = serde_json::from_str(text)
                .map_err(|error| FrameError::InvalidJson(error.to_string()))?;
            envelope.validate()?;
            messages.push(envelope);
        }

        Ok(messages)
    }

    pub fn finish(self) -> Result<(), FrameError> {
        if self.pending.is_empty() {
            Ok(())
        } else {
            Err(FrameError::TruncatedFrame)
        }
    }
}

/// Streaming frame reader/writer over any byte transport.
///
/// Unlike `FrameDecoder` (chunk-driven, used by the supervised local core),
/// `FrameStream` owns a Read+Write stream so one owner can interleave framed
/// reads and writes — the shape a TLS session has. A configurable limit lets
/// network-facing callers impose a tighter cap than the local 1 MiB maximum.
pub struct FrameStream<R: Read> {
    inner: R,
    pending: Vec<u8>,
    limit: usize,
}

impl<R: Read> FrameStream<R> {
    pub fn new(inner: R) -> Self {
        Self::with_limit(inner, MAX_FRAME_BYTES)
    }

    pub fn with_limit(inner: R, limit: usize) -> Self {
        Self {
            inner,
            pending: Vec::new(),
            limit,
        }
    }

    /// Reads the next complete frame payload, or `None` on a clean EOF at a
    /// frame boundary. EOF with a partial frame buffered is a protocol error.
    pub fn read_frame(&mut self) -> Result<Option<Vec<u8>>, FrameError> {
        let mut chunk = [0_u8; 8192];
        loop {
            if let Some(payload) = self.pop_frame()? {
                return Ok(Some(payload));
            }
            let count = self.inner.read(&mut chunk)?;
            if count == 0 {
                return if self.pending.is_empty() {
                    Ok(None)
                } else {
                    Err(FrameError::TruncatedFrame)
                };
            }
            self.pending.extend_from_slice(&chunk[..count]);
        }
    }

    fn pop_frame(&mut self) -> Result<Option<Vec<u8>>, FrameError> {
        if self.pending.len() < 4 {
            return Ok(None);
        }
        let length = frame_length(
            &self.pending[..4].try_into().expect("four-byte prefix"),
            self.limit,
        )?;
        let total = 4 + length;
        if self.pending.len() < total {
            return Ok(None);
        }

        let payload = self.pending[4..total].to_vec();
        self.pending.drain(..total);
        Ok(Some(payload))
    }
}

impl<R: Read + Write> FrameStream<R> {
    /// Writes one length-prefixed frame and flushes the stream.
    pub fn write_frame(&mut self, payload: &[u8]) -> Result<(), FrameError> {
        if payload.is_empty() {
            return Err(FrameError::EmptyFrame);
        }
        if payload.len() > self.limit {
            return Err(FrameError::OversizedFrame);
        }
        let length = u32::try_from(payload.len()).map_err(|_| FrameError::OversizedFrame)?;
        self.inner.write_all(&length.to_be_bytes())?;
        self.inner.write_all(payload)?;
        self.inner.flush()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use serde_json::json;

    use super::*;

    fn hello() -> Envelope {
        Envelope::request(
            "core.hello",
            json!({"client": "harbor-ui", "protocol_min": 1, "protocol_max": 1, "capabilities": ["pairing", "presence"]}),
            "2026-08-31T20:00:00Z",
        )
    }

    #[test]
    fn valid_request_round_trips_through_fragmented_frames() {
        let message = hello();
        let frame = encode_frame(&message).unwrap();
        let split = frame.len() / 2;
        let mut decoder = FrameDecoder::default();
        assert!(decoder.push(&frame[..split]).unwrap().is_empty());
        let received = decoder.push(&frame[split..]).unwrap();
        assert_eq!(received, vec![message]);
        decoder.finish().unwrap();
    }

    #[test]
    fn rejects_data_plane_message_types() {
        let mut message = hello();
        message.message_type = "transfer.chunk".into();
        assert_eq!(
            message.validate(),
            Err(ValidationError::ForbiddenMessageType(
                "transfer.chunk".into()
            ))
        );
    }

    #[test]
    fn local_call_messages_cannot_be_signed_for_network_transport() {
        let signing_key = SigningKey::from_bytes(&[9; 32]);
        for message_type in [
            "call.start",
            "call.end",
            "call.mute",
            "call.state_changed",
            "call.share_screen_start",
            "call.share_screen_stop",
            "call.share_state_changed",
        ] {
            let call = Envelope::request(
                message_type,
                json!({"call_id": "call-1"}),
                "2026-08-31T20:00:00Z",
            );
            assert_eq!(call.validate(), Ok(()));
            assert_eq!(
                call.validate_network(),
                Err(ValidationError::ForbiddenMessageType(message_type.into()))
            );
            assert!(matches!(
                AuthenticatedEnvelope::sign(Uuid::new_v4(), call, &signing_key),
                Err(AuthenticationError::InvalidEnvelope(ValidationError::ForbiddenMessageType(kind))) if kind == message_type
            ));
        }
    }

    #[test]
    fn rejects_oversized_length_before_allocating_payload() {
        let mut decoder = FrameDecoder::default();
        let frame = (MAX_FRAME_BYTES as u32 + 1).to_be_bytes();
        assert_eq!(decoder.push(&frame), Err(FrameError::OversizedFrame));
    }

    #[test]
    fn response_requires_request_correlation() {
        let event = Envelope::event("core.ready", json!({}), "2026-08-31T20:00:00Z");
        assert_eq!(
            Envelope::response_to(&event, "core.ready", json!({}), "2026-08-31T20:00:01Z"),
            Err(ValidationError::MissingRequestId)
        );
    }

    #[test]
    fn capabilities_are_collected_without_promoting_untyped_values() {
        let message = hello();
        assert_eq!(
            message.capabilities(),
            BTreeSet::from(["pairing", "presence"])
        );
    }

    #[test]
    fn signed_envelopes_bind_the_signer_and_payload() {
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let public_key = STANDARD_NO_PAD.encode(signing_key.verifying_key().as_bytes());
        let mut signed =
            AuthenticatedEnvelope::sign(Uuid::new_v4(), hello(), &signing_key).unwrap();
        assert_eq!(signed.verify(&public_key), Ok(()));

        signed.envelope.payload["client"] = json!("tampered");
        assert_eq!(
            signed.verify(&public_key),
            Err(AuthenticationError::InvalidSignature)
        );
    }

    #[test]
    fn frame_stream_round_trips_fragmented_writes_and_reads() {
        let payload = serde_json::to_vec(&hello()).unwrap();
        let mut wire = Cursor::new(Vec::new());
        {
            let mut writer = FrameStream::new(&mut wire);
            writer.write_frame(&payload).unwrap();
            writer.write_frame(b"second").unwrap();
        }
        wire.set_position(0);

        let mut reader = FrameStream::new(&mut wire);
        assert_eq!(reader.read_frame().unwrap().unwrap(), payload);
        assert_eq!(reader.read_frame().unwrap().unwrap(), b"second");
        assert_eq!(reader.read_frame().unwrap(), None);
    }

    #[test]
    fn frame_stream_enforces_its_configured_limit() {
        let mut wire = Cursor::new(Vec::new());
        {
            let mut writer = FrameStream::with_limit(&mut wire, 16);
            assert_eq!(writer.write_frame(b"fits"), Ok(()));
            assert_eq!(
                writer.write_frame(&[0_u8; 17]),
                Err(FrameError::OversizedFrame)
            );
        }

        // A peer announcing 0x09090909 payload bytes must be rejected before
        // any allocation, whatever the local limit is.
        let mut reader = FrameStream::with_limit(Cursor::new([9_u8; 4]), 16);
        assert_eq!(reader.read_frame(), Err(FrameError::OversizedFrame));
    }

    #[test]
    fn frame_stream_rejects_eof_mid_frame() {
        let mut wire = Cursor::new(Vec::new());
        {
            let mut writer = FrameStream::new(&mut wire);
            writer.write_frame(b"complete").unwrap();
        }
        let mut bytes = wire.into_inner();
        bytes.truncate(6); // full header plus a partial payload

        let mut reader = FrameStream::new(Cursor::new(bytes));
        assert_eq!(reader.read_frame(), Err(FrameError::TruncatedFrame));
    }
}
