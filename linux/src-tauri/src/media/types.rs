//! Versioned, size-limited IPC types shared by the native media bridge and the
//! Linux WebView. These types intentionally contain no access token or TURN
//! credential; signaling carries only SDP/ICE data.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MEDIA_PROTOCOL_VERSION: u8 = 1;
pub const MAX_MEDIA_SIGNAL_BYTES: usize = 256 * 1024;
pub const MAX_IDENTIFIER_BYTES: usize = 256;
pub const MAX_FRAME_BYTES: usize = 256 * 1024;

pub const MEDIA_STATE_EVENT: &str = "media_state";
#[allow(dead_code)]
pub const MEDIA_SIGNAL_EVENT: &str = "media_signal";
pub const MEDIA_AUDIO_EVENT: &str = "media_audio";
pub const MEDIA_VIDEO_EVENT: &str = "media_video";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaSignalEnvelope {
    pub v: u8,
    pub room_id: String,
    pub sender_id: String,
    pub seq: u64,
    pub kind: String,
    pub data: Value,
    pub sent_at: i64,
}

impl MediaSignalEnvelope {
    pub fn validate(&self) -> Result<(), String> {
        if self.v != MEDIA_PROTOCOL_VERSION {
            return Err("unsupported media signaling version".to_string());
        }
        if !valid_identifier(&self.room_id) || !valid_identifier(&self.sender_id) {
            return Err("invalid media signaling identifier".to_string());
        }
        if !matches!(self.kind.as_str(), "offer" | "answer" | "ice") {
            return Err("unsupported media signaling kind".to_string());
        }
        let encoded = serde_json::to_vec(self).map_err(|_| "invalid media signal".to_string())?;
        if encoded.len() > MAX_MEDIA_SIGNAL_BYTES {
            return Err("media signaling payload is too large".to_string());
        }
        Ok(())
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_IDENTIFIER_BYTES && !value.chars().any(char::is_control)
}

#[derive(Debug, Clone, Serialize)]
pub struct MediaCapabilities {
    pub protocol_version: u8,
    pub backend: String,
    pub audio_capture: bool,
    pub screen_capture: bool,
    pub screen_backend: String,
    pub native_webrtc: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MediaStartRequest {
    pub room_id: String,
    pub device_id: String,
    pub partner_id: String,
    #[serde(default)]
    pub screen: bool,
}

impl MediaStartRequest {
    pub fn validate(&self) -> Result<(), String> {
        for (name, value) in [
            ("room_id", self.room_id.as_str()),
            ("device_id", self.device_id.as_str()),
            ("partner_id", self.partner_id.as_str()),
        ] {
            if !valid_identifier(value) {
                return Err(format!("invalid media {name}"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct MediaStateEvent {
    pub state: String,
    pub backend: String,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MediaFrameEvent {
    pub timestamp_ns: u64,
    pub data_base64: String,
    pub dropped_before_emit: u64,
}

#[cfg(test)]
mod tests {
    use super::{MediaSignalEnvelope, MediaStartRequest, MAX_MEDIA_SIGNAL_BYTES};
    use serde_json::json;

    fn signal() -> MediaSignalEnvelope {
        MediaSignalEnvelope {
            v: 1,
            room_id: "pair:a:b".to_string(),
            sender_id: "a".to_string(),
            seq: 1,
            kind: "ice".to_string(),
            data: json!({"candidate": "candidate:1"}),
            sent_at: 1,
        }
    }

    #[test]
    fn validates_supported_signal() {
        assert!(signal().validate().is_ok());
    }

    #[test]
    fn rejects_unknown_kind_and_oversized_payload() {
        let mut invalid = signal();
        invalid.kind = "bogus".to_string();
        assert!(invalid.validate().is_err());

        let mut oversized = signal();
        oversized.data = json!("x".repeat(MAX_MEDIA_SIGNAL_BYTES));
        assert!(oversized.validate().is_err());
    }

    #[test]
    fn validates_start_identifiers() {
        let request = MediaStartRequest {
            room_id: "pair:a:b".to_string(),
            device_id: "a".to_string(),
            partner_id: "b".to_string(),
            screen: false,
        };
        assert!(request.validate().is_ok());
    }
}
