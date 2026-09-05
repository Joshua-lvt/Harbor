//! Device endpoints of Harbor identities: person first, devices second.
//!
//! A Harbor identity is a person (`harbor_id`). Devices (`Desktop`,
//! `Mobile`) are endpoints of that identity — never separate people,
//! never separate chats, never separate relationships.
//!
//! This module owns the policy the whole product shares:
//!
//! * [`DeviceType`] and [`compatibility`]: `Mobile ↔ Mobile` is refused
//!   everywhere (UI, pairing, session, direct channel, call signaling).
//!   A session between two identities needs a participating Desktop.
//! * [`identity_presence`]: identity presence is the OR of its authorized
//!   devices — one device alive means the identity is ONLINE. A device
//!   going quiet never reports the identity OFFLINE while another is up.
//! * [`MediaEndpoint`] / [`TakeoverDecision`]: one active media endpoint
//!   per identity; the second own device joins only through explicit
//!   takeover, and the first leaves media first.
//! * [`MobileStatus`]: the only phone aggregate that may cross IPC or
//!   P2P. Location and current-app are present only under their share
//!   toggle; there is no history, no notification content, no keystroke
//!   or screen material — [`MobileStatus::validate`] enforces that.
//! * [`MobileStatus::from_platform`]: unavailable platform APIs stay
//!   `None` and validate cleanly. State is never invented.
//! * [`should_auto_join`]: standalone persistent-call policy (default
//!   ON; OFF means explicit user connect).
//! * [`initial_mic_muted`]: a Mobile endpoint always joins muted.
//! * [`reconnect_delay`]: bounded backoff, never an aggressive loop.
//! * [`apply_shared_transcript`]: companion chat convergence — the same
//!   conversation on both own devices, idempotent by message ID.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::direct::{ChatSession, Delivery, Direction};

/// Schema version of the serialized [`MobileStatus`].
pub const MOBILE_STATUS_SCHEMA_VERSION: u16 = 1;
/// A Mobile endpoint always joins a call muted; unmute is an explicit tap.
pub const MOBILE_MIC_INITIAL_MUTED: bool = true;
/// Standalone persistent call ships ON: a valid session stays established.
pub const PERSISTENT_CALL_DEFAULT: bool = true;
/// Current-app labels are display names, not identifiers; cap them short.
pub const CURRENT_APP_MAX_CHARS: usize = 64;
/// Reconnect backoff floor/ceiling in seconds: patient, never spinning.
pub const RECONNECT_MIN_DELAY_SECS: u64 = 5;
pub const RECONNECT_MAX_DELAY_SECS: u64 = 300;

/// One endpoint of a Harbor identity. Per-install, local-only, persisted
/// in settings (`deviceType`); it never travels to the server as a
/// server-parsed field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeviceType {
    #[default]
    Desktop,
    Mobile,
}

impl DeviceType {
    /// Parses the persisted `deviceType` setting. Unknown values are `None`:
    /// the caller reports them, never defaults them silently.
    pub fn parse(value: &str) -> Option<DeviceType> {
        match value {
            "desktop" => Some(DeviceType::Desktop),
            "mobile" => Some(DeviceType::Mobile),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            DeviceType::Desktop => "desktop",
            DeviceType::Mobile => "mobile",
        }
    }
}
/// Why a session between two endpoints cannot exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompatibilityError {
    /// Both endpoints are Mobile: Harbor Mobile requires a desktop peer.
    MobileToMobile,
}

impl CompatibilityError {
    pub fn code(self) -> &'static str {
        match self {
            CompatibilityError::MobileToMobile => "mobile_to_mobile",
        }
    }

    pub fn ui_key(self) -> &'static str {
        match self {
            CompatibilityError::MobileToMobile => "error.device.mobileToMobile",
        }
    }
}

/// Whether two endpoints may hold a session. Everything except
/// `Mobile ↔ Mobile` is allowed; the refusal is symmetric.
pub fn compatibility(local: DeviceType, remote: DeviceType) -> Result<(), CompatibilityError> {
    match (local, remote) {
        (DeviceType::Mobile, DeviceType::Mobile) => Err(CompatibilityError::MobileToMobile),
        _ => Ok(()),
    }
}

/// Whether this install must refuse media with the known peers: we are a
/// Mobile and every peer whose type is already known is Mobile too. Unknown
/// peers never block — the hello in flight decides, and a hostile answer
/// ends the call instead of the UI inventing a refusal.
pub fn session_blocked(own: DeviceType, peers: &[Option<DeviceType>]) -> bool {
    if own != DeviceType::Mobile {
        return false;
    }
    let mut known = 0;
    for peer in peers {
        let Some(peer) = peer else {
            // An unknown peer may still be a desktop: never block on a
            // maybe. The hello in flight decides.
            return false;
        };
        known += 1;
        if *peer != DeviceType::Mobile {
            return false;
        }
    }
    known > 0
}
/// How this install relates to its own identity's other devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MobileMode {
    /// This identity also has an authorized Desktop: the phone extends it.
    Companion,
    /// No Desktop on this identity: the phone is the identity's voice.
    Standalone,
}

/// One known device of an identity, own or peer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceRecord {
    pub device_id: Uuid,
    pub device_type: DeviceType,
    pub authorized: bool,
    pub last_seen: u64,
}

impl DeviceRecord {
    pub fn new(device_id: Uuid, device_type: DeviceType, authorized: bool, now: u64) -> Self {
        Self {
            device_id,
            device_type,
            authorized,
            last_seen: now,
        }
    }
}

/// Companion when the identity owns an authorized Desktop, standalone
/// otherwise. Unauthorized entries never count.
pub fn mobile_mode(own_devices: &[DeviceRecord]) -> MobileMode {
    let has_desktop = own_devices
        .iter()
        .any(|device| device.authorized && device.device_type == DeviceType::Desktop);
    if has_desktop {
        MobileMode::Companion
    } else {
        MobileMode::Standalone
    }
}

/// This install's mode from its own record plus the authorized linked
/// devices (the companion registry). Companion needs both endpoint kinds
/// present: a lone phone is standalone, a lone desktop is just a desktop.
pub fn own_mode(own: &DeviceRecord, linked: &[DeviceRecord]) -> MobileMode {
    let mut has_desktop = own.device_type == DeviceType::Desktop;
    let mut has_mobile = own.device_type == DeviceType::Mobile;
    for device in linked.iter().filter(|device| device.authorized) {
        match device.device_type {
            DeviceType::Desktop => has_desktop = true,
            DeviceType::Mobile => has_mobile = true,
        }
    }
    if has_desktop && has_mobile {
        MobileMode::Companion
    } else {
        MobileMode::Standalone
    }
}

/// Identity presence from its authorized devices' committed states:
/// any ONLINE wins, then any AWAY, else OFFLINE. Ordered from most to
/// least present so a quiet device can never drag a live identity down.
pub fn identity_presence(
    states: &[crate::presence::PresenceState],
) -> crate::presence::PresenceState {
    use crate::presence::PresenceState;
    if states.contains(&PresenceState::Online) {
        PresenceState::Online
    } else if states.contains(&PresenceState::Away) {
        PresenceState::Away
    } else {
        PresenceState::Offline
    }
}

/// The single device of an identity currently holding call media.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaEndpoint {
    pub device_id: Uuid,
    pub device_type: DeviceType,
}

/// Verdict of a takeover request from another own device.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TakeoverDecision {
    /// Granted: the current endpoint must leave media before the
    /// requester joins. Never two mics of one identity at once.
    Grant { drop_device: Uuid, join_device: Uuid },
    /// The requester already holds media; nothing to do.
    AlreadyActive,
}

impl MediaEndpoint {
    /// Explicit takeover: granted to any *other* device; the handoff
    /// order (drop first, join second) is the caller's duty.
    pub fn request_takeover(&self, requester: Uuid) -> TakeoverDecision {
        if requester == self.device_id {
            TakeoverDecision::AlreadyActive
        } else {
            TakeoverDecision::Grant {
                drop_device: self.device_id,
                join_device: requester,
            }
        }
    }
}

/// Whether the endpoint joins the call without further taps: a valid
/// session auto-joins while persistent call is ON; with it OFF only an
/// explicit user request joins. Never auto-answer against policy.
pub fn should_auto_join(persistent_call: bool, session_valid: bool, user_requested: bool) -> bool {
    session_valid && (persistent_call || user_requested)
}

/// A Mobile endpoint joins muted, always. Desktop keeps its own mute state.
pub fn initial_mic_muted(device_type: DeviceType) -> bool {
    match device_type {
        DeviceType::Mobile => MOBILE_MIC_INITIAL_MUTED,
        DeviceType::Desktop => false,
    }
}

/// Bounded reconnect backoff: 5 s doubling per attempt, capped at 300 s.
/// Attempt counting restarts on a successful connect; the floor keeps a
/// dead network from spinning the radio.
pub fn reconnect_delay(attempt: u32) -> u64 {
    let shift = attempt.min(6);
    RECONNECT_MIN_DELAY_SECS
        .saturating_mul(1_u64 << shift)
        .min(RECONNECT_MAX_DELAY_SECS)
}

/// Phone usage as observed locally. Coarse on purpose: ACTIVE / IDLE /
/// OFFLINE plus when last seen — never keys, content, or detail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum PhoneActivity {
    Active,
    Idle,
    Offline,
}

/// One location fix: current position only, never a route.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocationFix {
    pub latitude: f64,
    pub longitude: f64,
    pub accuracy_meters: f32,
    pub updated_at: u64,
}

impl LocationFix {
    pub fn valid(&self) -> bool {
        self.latitude.is_finite()
            && self.longitude.is_finite()
            && (-90.0..=90.0).contains(&self.latitude)
            && (-180.0..=180.0).contains(&self.longitude)
            && self.accuracy_meters.is_finite()
            && self.accuracy_meters >= 0.0
    }
}

/// The whole phone state that may leave the device, and nothing else.
/// Every optional fact needs both its share toggle (user intent) and a
/// real platform observation (grant + signal); otherwise it stays `None`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MobileStatus {
    pub schema_version: u16,
    pub device_type: DeviceType,
    pub battery_percent: Option<u8>,
    pub charging: bool,
    pub phone_activity: PhoneActivity,
    pub last_active_at: Option<u64>,
    /// Display label of the foreground app, only under activity sharing.
    pub current_app: Option<String>,
    pub location_sharing_enabled: bool,
    pub location: Option<LocationFix>,
    pub notification_sharing_enabled: bool,
}

impl MobileStatus {
    /// Honest empty: no signals, nothing shared. Always valid.
    pub fn unavailable() -> Self {
        Self {
            schema_version: MOBILE_STATUS_SCHEMA_VERSION,
            device_type: DeviceType::Mobile,
            battery_percent: None,
            charging: false,
            phone_activity: PhoneActivity::Offline,
            last_active_at: None,
            current_app: None,
            location_sharing_enabled: false,
            location: None,
            notification_sharing_enabled: false,
        }
    }

    /// Structural + consent validation. Fails closed: a fix without its
    /// toggle, an app label without activity, or an out-of-range battery
    /// is refused rather than transmitted.
    pub fn validate(&self) -> Result<(), MobileStatusError> {
        if self.schema_version != MOBILE_STATUS_SCHEMA_VERSION {
            return Err(MobileStatusError::UnknownSchema);
        }
        if let Some(percent) = self.battery_percent {
            if percent > 100 {
                return Err(MobileStatusError::BatteryOutOfRange);
            }
        }
        if self.location_sharing_enabled {
            // Enabled without a fix yet is pending (cold GPS, indoors),
            // not a lie: only a present-but-invalid fix is refused.
            match &self.location {
                None => {}
                Some(fix) if fix.valid() => {}
                _ => return Err(MobileStatusError::LocationInvalid),
            }
        } else if self.location.is_some() {
            return Err(MobileStatusError::LocationWithoutConsent);
        }
        if let Some(app) = &self.current_app {
            let trimmed = app.trim();
            if trimmed.is_empty()
                || trimmed.chars().count() > CURRENT_APP_MAX_CHARS
                || trimmed.chars().any(char::is_control)
            {
                return Err(MobileStatusError::CurrentAppInvalid);
            }
            if self.phone_activity != PhoneActivity::Active {
                return Err(MobileStatusError::AppWithoutActivity);
            }
        }
        Ok(())
    }

    /// Builds the shareable status from raw platform observations plus the
    /// user's share intents. A toggle that is ON but has no observation
    /// yields `None` — never an invention.
    pub fn from_platform(signals: &PlatformSignals, intents: &ShareIntents, now: u64) -> Self {
        let _ = now;
        let mut status = Self::unavailable();
        status.battery_percent = signals.battery_percent.filter(|_| intents.battery);
        status.charging = signals.charging && status.battery_percent.is_some();
        if intents.phone_activity {
            status.phone_activity = signals.phone_activity;
            status.last_active_at = signals.last_active_at;
            status.current_app = signals.current_app.clone().filter(|app| {
                let trimmed = app.trim();
                !trimmed.is_empty()
                    && trimmed.chars().count() <= CURRENT_APP_MAX_CHARS
                    && !trimmed.chars().any(char::is_control)
            });
            if status.phone_activity != PhoneActivity::Active {
                status.current_app = None;
            }
        }
        status.location_sharing_enabled = intents.location && signals.location_permission;
        status.location = signals
            .location
            .filter(|fix| status.location_sharing_enabled && fix.valid());
        status.notification_sharing_enabled =
            intents.phone_notifications && signals.notification_access;
        status
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MobileStatusError {
    UnknownSchema,
    BatteryOutOfRange,
    LocationWithoutConsent,
    LocationInvalid,
    CurrentAppInvalid,
    AppWithoutActivity,
}

impl MobileStatusError {
    pub fn code(self) -> &'static str {
        match self {
            MobileStatusError::UnknownSchema => "unknown_schema",
            MobileStatusError::BatteryOutOfRange => "battery_out_of_range",
            MobileStatusError::LocationWithoutConsent => "location_without_consent",
            MobileStatusError::LocationInvalid => "location_invalid",
            MobileStatusError::CurrentAppInvalid => "current_app_invalid",
            MobileStatusError::AppWithoutActivity => "app_without_activity",
        }
    }
}

/// Raw platform observations. Each is `None`/false when the API is
/// missing, denied, or silent — the adapter reports, the core decides.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlatformSignals {
    pub battery_percent: Option<u8>,
    pub charging: bool,
    pub phone_activity: PhoneActivity,
    pub last_active_at: Option<u64>,
    pub current_app: Option<String>,
    pub location_permission: bool,
    pub location: Option<LocationFix>,
    pub notification_access: bool,
}

impl Default for PhoneActivity {
    fn default() -> Self {
        PhoneActivity::Offline
    }
}

/// User share intents (persisted toggles). Intent without grant still
/// shares nothing — see [`MobileStatus::from_platform`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShareIntents {
    pub battery: bool,
    pub phone_activity: bool,
    pub location: bool,
    pub phone_notifications: bool,
}

impl Default for ShareIntents {
    fn default() -> Self {
        // Sensitive shares ship OFF; battery is coarse device state.
        Self {
            battery: true,
            phone_activity: false,
            location: false,
            phone_notifications: false,
        }
    }
}

/// One chat record synced from another device of the same identity.
/// Same IDs as the peer conversation: convergence is dedup, not merge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedChatRecord {
    pub id: String,
    pub body: String,
    pub outgoing: bool,
    pub delivered: bool,
    pub timestamp: u64,
}

/// Applies own-device records to this install's transcript. Returns how
/// many were new; replays and duplicates change nothing.
pub fn apply_shared_transcript(session: &mut ChatSession, records: &[SharedChatRecord]) -> usize {
    let mut added = 0;
    for record in records {
        let direction = if record.outgoing {
            Direction::Outgoing
        } else {
            Direction::Incoming
        };
        let delivery = if record.outgoing {
            if record.delivered {
                Delivery::Delivered
            } else {
                Delivery::WaitingForConnection
            }
        } else {
            Delivery::Delivered
        };
        if session.record_synced(
            record.id.clone(),
            record.body.clone(),
            direction,
            delivery,
            record.timestamp,
        ) {
            added += 1;
        }
    }
    added
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::presence::PresenceState;

    fn fix() -> LocationFix {
        LocationFix {
            latitude: -23.5505,
            longitude: -46.6333,
            accuracy_meters: 12.0,
            updated_at: 1_700_000_000,
        }
    }

    fn sharing_intents() -> ShareIntents {
        ShareIntents {
            battery: true,
            phone_activity: true,
            location: true,
            phone_notifications: true,
        }
    }

    fn full_signals() -> PlatformSignals {
        PlatformSignals {
            battery_percent: Some(73),
            charging: true,
            phone_activity: PhoneActivity::Active,
            last_active_at: Some(1_700_000_100),
            current_app: Some("YouTube".into()),
            location_permission: true,
            location: Some(fix()),
            notification_access: true,
        }
    }

    #[test]
    fn device_types_round_trip_through_lowercase_json() {
        assert_eq!(
            serde_json::to_value(DeviceType::Desktop).unwrap(),
            serde_json::json!("desktop")
        );
        assert_eq!(
            serde_json::to_value(DeviceType::Mobile).unwrap(),
            serde_json::json!("mobile")
        );
        assert_eq!(
            serde_json::from_value::<DeviceType>(serde_json::json!("mobile")).unwrap(),
            DeviceType::Mobile
        );
    }

    #[test]
    fn desktop_pairs_with_desktop() {
        assert_eq!(
            compatibility(DeviceType::Desktop, DeviceType::Desktop),
            Ok(())
        );
    }

    #[test]
    fn mobile_pairs_with_desktop_in_both_directions() {
        assert_eq!(
            compatibility(DeviceType::Mobile, DeviceType::Desktop),
            Ok(())
        );
        assert_eq!(
            compatibility(DeviceType::Desktop, DeviceType::Mobile),
            Ok(())
        );
    }

    #[test]
    fn mobile_to_mobile_is_refused_symmetrically() {
        assert_eq!(
            compatibility(DeviceType::Mobile, DeviceType::Mobile),
            Err(CompatibilityError::MobileToMobile)
        );
        let error = CompatibilityError::MobileToMobile;
        assert_eq!(error.code(), "mobile_to_mobile");
        assert_eq!(error.ui_key(), "error.device.mobileToMobile");
    }

    #[test]
    fn companion_mode_needs_an_authorized_desktop() {
        let now = 100;
        let desktop = DeviceRecord::new(Uuid::new_v4(), DeviceType::Desktop, true, now);
        let mobile = DeviceRecord::new(Uuid::new_v4(), DeviceType::Mobile, true, now);
        assert_eq!(mobile_mode(&[desktop, mobile.clone()]), MobileMode::Companion);
        // An unauthorized desktop does not make a companion.
        let ghost = DeviceRecord::new(Uuid::new_v4(), DeviceType::Desktop, false, now);
        assert_eq!(mobile_mode(&[ghost, mobile.clone()]), MobileMode::Standalone);
        assert_eq!(mobile_mode(&[mobile]), MobileMode::Standalone);
        assert_eq!(mobile_mode(&[]), MobileMode::Standalone);
    }

    #[test]
    fn session_blocked_only_refuses_known_mobile_to_mobile() {
        use DeviceType::{Desktop, Mobile};
        assert!(!session_blocked(Mobile, &[]));
        assert!(!session_blocked(Mobile, &[None, None]));
        assert!(!session_blocked(Mobile, &[Some(Mobile), None]));
        assert!(session_blocked(Mobile, &[Some(Mobile)]));
        assert!(session_blocked(Mobile, &[Some(Mobile), Some(Mobile)]));
        assert!(!session_blocked(Mobile, &[Some(Mobile), Some(Desktop)]));
        assert!(!session_blocked(Desktop, &[Some(Mobile)]));
        assert!(!session_blocked(Desktop, &[]));
    }

    #[test]
    fn own_mode_needs_both_endpoint_kinds() {
        let now = 100;
        let desktop = DeviceRecord::new(Uuid::new_v4(), DeviceType::Desktop, true, now);
        let mobile = DeviceRecord::new(Uuid::new_v4(), DeviceType::Mobile, true, now);
        let ghost = DeviceRecord::new(Uuid::new_v4(), DeviceType::Desktop, false, now);
        // Phone plus linked desktop: companion, from either side.
        assert_eq!(own_mode(&mobile, &[desktop.clone()]), MobileMode::Companion);
        assert_eq!(own_mode(&desktop, &[mobile.clone()]), MobileMode::Companion);
        // Unauthorized links never count; lone endpoints are standalone.
        assert_eq!(own_mode(&mobile, &[ghost]), MobileMode::Standalone);
        assert_eq!(own_mode(&mobile, &[]), MobileMode::Standalone);
        assert_eq!(own_mode(&desktop, &[]), MobileMode::Standalone);
    }

    #[test]
    fn device_type_parses_the_persisted_setting_only() {
        assert_eq!(DeviceType::parse("desktop"), Some(DeviceType::Desktop));
        assert_eq!(DeviceType::parse("mobile"), Some(DeviceType::Mobile));
        assert_eq!(DeviceType::parse("tablet"), None);
        assert_eq!(DeviceType::parse(""), None);
        assert_eq!(DeviceType::Mobile.as_str(), "mobile");
    }

    #[test]
    fn identity_presence_is_the_or_of_its_devices() {
        use PresenceState::{Away, Offline, Online};
        assert_eq!(identity_presence(&[Online, Online]), Online);
        assert_eq!(identity_presence(&[Offline, Online]), Online);
        assert_eq!(identity_presence(&[Online, Offline]), Online);
        assert_eq!(identity_presence(&[Offline, Offline]), Offline);
        assert_eq!(identity_presence(&[Away, Offline]), Away);
        assert_eq!(identity_presence(&[Away, Away]), Away);
        // A quiet device never drags a live identity offline.
        assert_eq!(identity_presence(&[Offline, Away, Online]), Online);
        assert_eq!(identity_presence(&[]), Offline);
    }

    #[test]
    fn shared_transcript_converges_without_duplicates() {
        let mut session = ChatSession::new();
        let records = vec![
            SharedChatRecord {
                id: "abc".into(),
                body: "Oi".into(),
                outgoing: true,
                delivered: true,
                timestamp: 10,
            },
            SharedChatRecord {
                id: "def".into(),
                body: "Oi amor".into(),
                outgoing: false,
                delivered: true,
                timestamp: 20,
            },
        ];
        assert_eq!(apply_shared_transcript(&mut session, &records), 2);
        // Replays add nothing.
        assert_eq!(apply_shared_transcript(&mut session, &records), 0);
        assert_eq!(session.snapshot().len(), 2);
    }

    #[test]
    fn presence_updates_follow_identity_or() {
        use PresenceState::{Offline, Online};
        // Desktop drops; mobile still up: identity stays ONLINE.
        assert_eq!(identity_presence(&[Offline, Online]), Online);
        // Both drop: identity reads OFFLINE.
        assert_eq!(identity_presence(&[Offline, Offline]), Offline);
    }

    #[test]
    fn battery_state_accepts_full_range_only() {
        let mut status = MobileStatus::unavailable();
        status.battery_percent = Some(73);
        status.charging = true;
        assert_eq!(status.validate(), Ok(()));
        status.battery_percent = Some(101);
        assert_eq!(
            status.validate(),
            Err(MobileStatusError::BatteryOutOfRange)
        );
    }

    #[test]
    fn location_sharing_on_requires_a_valid_fix() {
        let mut status = MobileStatus::unavailable();
        status.location_sharing_enabled = true;
        status.location = Some(fix());
        assert_eq!(status.validate(), Ok(()));
        // Enabled with no fix yet is pending, not invalid.
        status.location = None;
        assert_eq!(status.validate(), Ok(()));
        let mut bad = fix();
        bad.latitude = 91.0;
        status.location = Some(bad);
        assert_eq!(status.validate(), Err(MobileStatusError::LocationInvalid));
    }

    #[test]
    fn location_sharing_off_forbids_any_fix() {
        let mut status = MobileStatus::unavailable();
        status.location = Some(fix());
        assert_eq!(
            status.validate(),
            Err(MobileStatusError::LocationWithoutConsent)
        );
        status.location = None;
        assert_eq!(status.validate(), Ok(()));
    }

    #[test]
    fn phone_activity_on_carries_app_and_last_seen() {
        let status = MobileStatus::from_platform(&full_signals(), &sharing_intents(), 1_700_000_200);
        assert_eq!(status.phone_activity, PhoneActivity::Active);
        assert_eq!(status.current_app.as_deref(), Some("YouTube"));
        assert_eq!(status.validate(), Ok(()));
    }

    #[test]
    fn phone_activity_off_shares_nothing() {
        let intents = ShareIntents::default();
        let status = MobileStatus::from_platform(&full_signals(), &intents, 1_700_000_200);
        assert_eq!(status.phone_activity, PhoneActivity::Offline);
        assert_eq!(status.current_app, None);
        assert_eq!(status.location, None);
        assert!(!status.notification_sharing_enabled);
        assert_eq!(status.validate(), Ok(()));
        // Battery is coarse device state and still flows.
        assert_eq!(status.battery_percent, Some(73));
    }

    #[test]
    fn notification_sharing_toggles_cleanly() {
        let on = MobileStatus::from_platform(&full_signals(), &sharing_intents(), 1);
        assert!(on.notification_sharing_enabled);
        let mut intents = sharing_intents();
        intents.phone_notifications = false;
        let off = MobileStatus::from_platform(&full_signals(), &intents, 1);
        assert!(!off.notification_sharing_enabled);
        assert_eq!(on.validate(), Ok(()));
        assert_eq!(off.validate(), Ok(()));
    }

    #[test]
    fn invented_app_labels_are_refused() {
        let mut status = MobileStatus::unavailable();
        status.phone_activity = PhoneActivity::Idle;
        status.current_app = Some("YouTube".into());
        assert_eq!(status.validate(), Err(MobileStatusError::AppWithoutActivity));
        status.phone_activity = PhoneActivity::Active;
        status.current_app = Some("  ".into());
        assert_eq!(status.validate(), Err(MobileStatusError::CurrentAppInvalid));
    }

    #[test]
    fn takeover_moves_media_pc_to_mobile() {
        let desktop = Uuid::new_v4();
        let mobile = Uuid::new_v4();
        let endpoint = MediaEndpoint {
            device_id: desktop,
            device_type: DeviceType::Desktop,
        };
        assert_eq!(
            endpoint.request_takeover(mobile),
            TakeoverDecision::Grant {
                drop_device: desktop,
                join_device: mobile,
            }
        );
    }

    #[test]
    fn takeover_moves_media_mobile_to_pc() {
        let desktop = Uuid::new_v4();
        let mobile = Uuid::new_v4();
        let endpoint = MediaEndpoint {
            device_id: mobile,
            device_type: DeviceType::Mobile,
        };
        assert_eq!(
            endpoint.request_takeover(desktop),
            TakeoverDecision::Grant {
                drop_device: mobile,
                join_device: desktop,
            }
        );
        assert_eq!(
            endpoint.request_takeover(mobile),
            TakeoverDecision::AlreadyActive
        );
    }

    #[test]
    fn standalone_persistent_call_joins_while_off_needs_a_tap() {
        assert!(PERSISTENT_CALL_DEFAULT);
        assert!(should_auto_join(true, true, false));
        assert!(!should_auto_join(true, false, false));
        assert!(!should_auto_join(false, true, false));
        assert!(should_auto_join(false, true, true));
    }

    #[test]
    fn microphone_starts_muted_on_mobile_only() {
        assert!(initial_mic_muted(DeviceType::Mobile));
        assert!(!initial_mic_muted(DeviceType::Desktop));
    }

    #[test]
    fn reconnect_backoff_is_patient_and_bounded() {
        assert_eq!(reconnect_delay(0), 5);
        assert_eq!(reconnect_delay(1), 10);
        assert_eq!(reconnect_delay(2), 20);
        assert_eq!(reconnect_delay(5), 160);
        assert_eq!(reconnect_delay(6), 300);
        assert_eq!(reconnect_delay(100), RECONNECT_MAX_DELAY_SECS);
    }

    #[test]
    fn permissions_unavailable_yields_honest_empty_status() {
        let intents = sharing_intents();
        let status =
            MobileStatus::from_platform(&PlatformSignals::default(), &intents, 1_700_000_200);
        // Intents are ON but nothing was observed: nothing is claimed.
        assert_eq!(status.battery_percent, None);
        assert_eq!(status.current_app, None);
        assert_eq!(status.location, None);
        assert!(!status.location_sharing_enabled);
        assert!(!status.notification_sharing_enabled);
        assert_eq!(status.validate(), Ok(()));
    }

    #[test]
    fn android_apis_unavailable_never_invents_state() {
        // Toggle ON + silent API: fields stay None, never defaults.
        let signals = PlatformSignals {
            battery_percent: None,
            charging: true,
            ..PlatformSignals::default()
        };
        let status = MobileStatus::from_platform(&signals, &sharing_intents(), 1);
        assert_eq!(status.battery_percent, None);
        assert!(!status.charging);
        assert_eq!(status.validate(), Ok(()));
    }
}
