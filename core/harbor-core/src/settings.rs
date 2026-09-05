//! Durable, versioned user settings for the Harbor domain process.
//!
//! Settings use the same private storage discipline as the local identity.
//! Unknown fields in the stored document survive a save (forward
//! compatibility), while unknown keys in an update patch are rejected so the
//! QML-facing contract stays explicit.

use std::{env, io, path::Path, path::PathBuf};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;

use crate::storage::{self, StorageError};
use crate::device::DeviceRecord;

const SETTINGS_FILE: &str = "settings-v1.json";
const SETTINGS_SCHEMA_VERSION: u16 = 1;
const DISPLAY_NAME_MAX_CHARS: usize = 80;
const STATUS_MESSAGE_MAX_CHARS: usize = 140;
const AVATAR_MAX_BYTES: usize = 4 * 1024 * 1024;
const AVATAR_MIME_PREFIXES: &[&str] = &[
    "data:image/png;base64,",
    "data:image/jpeg;base64,",
    "data:image/jpg;base64,",
    "data:image/webp;base64,",
    "data:image/gif;base64,",
];

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("settings storage I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error(transparent)]
    Storage(#[from] StorageError),
    #[error("settings storage contains invalid data")]
    InvalidDocument,
    #[error("settings storage directory cannot be resolved")]
    UnresolvableHome,
    #[error("update patch is not a JSON object")]
    InvalidPatch,
    #[error("update patch contains unknown setting keys")]
    UnknownKey,
}

/// The durable settings document. Field defaults mirror the fixtures the UI
/// shipped with, so a fresh install behaves like the prototype until the user
/// changes something.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct StoredSettings {
    schema_version: u16,
    pub locale: String,
    /// Optional human-facing profile metadata. The public subset
    /// (display name, status, avatar) syncs peer-to-peer with the paired
    /// device through the profile protocol; it is never sent to the server.
    pub display_name: String,
    pub avatar: String,
    pub avatar_type: String,
    /// Free-form status line shown beside the display name. Local durable
    /// state, like the rest of the profile block.
    pub status_message: String,
    /// Per-device monotonic profile revision, published with the public
    /// profile so the peer applies newer state and ignores replays. Bumped
    /// inside [`Settings::update`] on any profile-key change.
    pub profile_revision: u64,
    pub higher_contrast: bool,
    pub background: bool,
    pub reduced_motion: bool,
    pub start_with_system: bool,
    pub minimize_to_tray: bool,
    pub close_to_tray: bool,
    pub auto_connect: bool,
    pub notifications_enabled: bool,
    pub game_notifications: bool,
    pub app_notifications: bool,
    pub connection_notifications: bool,
    pub notification_sound: bool,
    /// Message previews on notifications: ON shows the message text, OFF
    /// shows only a generic "New message". The choice never leaves the
    /// machine — it gates the UI layer that renders the notification.
    #[serde(default = "default_true")]
    pub message_previews: bool,
    /// Partner-presence announcement toggles. Switching one off silences
    /// the notification; presence itself keeps flowing normally underneath.
    #[serde(default = "default_true")]
    pub notify_partner_online: bool,
    #[serde(default = "default_true")]
    pub notify_partner_away: bool,
    #[serde(default = "default_true")]
    pub notify_partner_offline: bool,
    pub presence_visibility: bool,
    pub activity_sharing: bool,
    pub game_visibility: bool,
    pub device_visibility: bool,
    pub voice_activation: bool,
    pub debug_mode: bool,
    pub accent_intensity: f64,
    pub microphone_volume: f64,
    pub output_volume: f64,
    pub input_device: String,
    pub output_device: String,
    pub push_to_talk_key: String,
    /// When true, an unmuted microphone only transmits while the push-to-talk
    /// key is held; false is an open microphone.
    pub push_to_talk_enabled: bool,
    /// Directory completed file transfers land in; empty means the
    /// platform default (the user's Downloads directory).
    pub transfer_directory: String,
    pub appearance_mode: String,
    /// Accent preset key (`ocean`, `cyan`, `aqua`, `blue`, `teal`,
    /// `violet`, `pink`) or a custom `#RRGGBB` color.
    #[serde(default = "default_accent_color")]
    pub accent_color: String,
    /// Glass/transparency effect strength, 0..=1. Multiplies surface alphas.
    #[serde(default = "default_unit_intensity")]
    pub glass_intensity: f64,
    /// Animation strength, 0..=1. Scales motion durations and particles.
    #[serde(default = "default_unit_intensity")]
    pub animation_intensity: f64,
    /// Background particles on/off (master atmosphere switch stays separate).
    #[serde(default = "default_true")]
    pub particles_enabled: bool,
    /// Ocean background variation: `lagoon`, `abyss`, or `sunrise`.
    #[serde(default = "default_ocean_variant")]
    pub ocean_variant: String,
    /// Component rounding: `soft` or `medium`.
    #[serde(default = "default_corner_radius")]
    pub corner_radius: String,
    /// Interface density: `comfortable` or `compact`.
    #[serde(default = "default_density")]
    pub density: String,
    /// Desktop companion widget visible.
    #[serde(default = "default_true")]
    pub widget_enabled: bool,
    /// Widget corner: `topLeft`, `topRight`, `bottomLeft`, or `bottomRight`.
    #[serde(default = "default_widget_position")]
    pub widget_position: String,
    /// Widget shows the partner's current activity.
    #[serde(default = "default_true")]
    pub widget_show_activity: bool,
    /// Widget shows avatars (partner, and both sides in call presence).
    #[serde(default = "default_true")]
    pub widget_show_avatar: bool,
    /// Widget shows the joined-call symbol while in a call.
    #[serde(default = "default_true")]
    pub widget_show_call_presence: bool,
    /// This install's endpoint kind: `desktop` or `mobile`. Local-only;
    /// the server never parses device claims.
    #[serde(default = "default_device_type")]
    pub device_type: String,
    /// Standalone persistent call: ON keeps a valid session established;
    /// OFF requires an explicit user connect.
    #[serde(default = "default_true")]
    pub persistent_call: bool,
    /// Share-intent toggles for phone state. Intents persist here; grants
    /// are platform facts. Sensitive shares ship OFF.
    #[serde(default)]
    pub share_location: bool,
    #[serde(default)]
    pub share_phone_activity: bool,
    #[serde(default)]
    pub share_phone_notifications: bool,
    /// Companion registry: other devices of this identity the user
    /// authorized (the future link ceremony writes here; the mode,
    /// identity presence, and takeover arbitration read here). Empty
    /// means no linked device is known — never an invented companion.
    #[serde(default)]
    pub linked_devices: Vec<DeviceRecord>,
}

fn default_accent_color() -> String {
    "ocean".into()
}

fn default_unit_intensity() -> f64 {
    1.0
}

fn default_true() -> bool {
    true
}

fn default_ocean_variant() -> String {
    "lagoon".into()
}

fn default_corner_radius() -> String {
    "soft".into()
}

fn default_density() -> String {
    "comfortable".into()
}

fn default_widget_position() -> String {
    "bottomRight".into()
}

fn default_device_type() -> String {
    "desktop".into()
}

fn valid_accent_color(value: &str) -> bool {
    matches!(
        value,
        "ocean" | "cyan" | "aqua" | "blue" | "teal" | "violet" | "pink"
    ) || {
        let bytes = value.as_bytes();
        bytes.len() == 7
            && bytes[0] == b'#'
            && bytes[1..].iter().all(|byte| byte.is_ascii_hexdigit())
    }
}

fn valid_avatar(value: &str) -> bool {
    if value.is_empty() {
        return true;
    }
    let Some(prefix) = AVATAR_MIME_PREFIXES
        .iter()
        .find(|prefix| value.starts_with(**prefix))
    else {
        return false;
    };
    let encoded = &value[prefix.len()..];
    !encoded.is_empty()
        && !encoded.bytes().any(|byte| byte.is_ascii_whitespace())
        && STANDARD
            .decode(encoded)
            .map(|bytes| !bytes.is_empty() && bytes.len() <= AVATAR_MAX_BYTES)
            .unwrap_or(false)
}

fn valid_profile(values: &StoredSettings) -> bool {
    values.display_name.chars().count() <= DISPLAY_NAME_MAX_CHARS
        && values.status_message.chars().count() <= STATUS_MESSAGE_MAX_CHARS
        && values.avatar.len() <= AVATAR_MAX_BYTES
        && valid_avatar(&values.avatar)
        && matches!(values.avatar_type.as_str(), "image" | "gif")
        && valid_accent_color(&values.accent_color)
        && (0.0..=1.0).contains(&values.glass_intensity)
        && (0.0..=1.0).contains(&values.animation_intensity)
        && matches!(
            values.ocean_variant.as_str(),
            "lagoon" | "abyss" | "sunrise" | "rose" | "ember" | "onyx" | "forest" | "dusk"
        )
        && matches!(values.corner_radius.as_str(), "soft" | "medium")
        && matches!(values.density.as_str(), "comfortable" | "compact")
        && matches!(
            values.widget_position.as_str(),
            "topLeft" | "topRight" | "bottomLeft" | "bottomRight"
        )
        && matches!(values.device_type.as_str(), "desktop" | "mobile")
}

impl Default for StoredSettings {
    fn default() -> Self {
        Self {
            schema_version: SETTINGS_SCHEMA_VERSION,
            locale: "en".into(),
            display_name: String::new(),
            status_message: String::new(),
            profile_revision: 1,
            avatar: String::new(),
            avatar_type: "image".into(),
            higher_contrast: false,
            background: true,
            reduced_motion: false,
            start_with_system: true,
            minimize_to_tray: true,
            close_to_tray: true,
            auto_connect: true,
            notifications_enabled: true,
            game_notifications: true,
            app_notifications: true,
            connection_notifications: true,
            notification_sound: true,
            message_previews: true,
            notify_partner_online: true,
            notify_partner_away: true,
            notify_partner_offline: true,
            presence_visibility: true,
            activity_sharing: true,
            game_visibility: true,
            device_visibility: true,
            voice_activation: true,
            debug_mode: false,
            accent_intensity: 0.75,
            microphone_volume: 0.72,
            output_volume: 0.64,
            input_device: "default-microphone".into(),
            output_device: "harbor-headphones".into(),
            push_to_talk_key: "Space".into(),
            push_to_talk_enabled: true,
            transfer_directory: String::new(),
            appearance_mode: "dark".into(),
            accent_color: default_accent_color(),
            glass_intensity: default_unit_intensity(),
            animation_intensity: default_unit_intensity(),
            particles_enabled: default_true(),
            ocean_variant: default_ocean_variant(),
            corner_radius: default_corner_radius(),
            density: default_density(),
            widget_enabled: default_true(),
            widget_position: default_widget_position(),
            widget_show_activity: default_true(),
            widget_show_avatar: default_true(),
            widget_show_call_presence: default_true(),
            device_type: default_device_type(),
            persistent_call: default_true(),
            share_location: false,
            share_phone_activity: false,
            share_phone_notifications: false,
            linked_devices: Vec::new(),
        }
    }
}

/// Loaded settings bound to the directory that owns their persistence.
#[derive(Debug, Clone)]
pub struct Settings {
    values: StoredSettings,
    path: PathBuf,
}

impl Settings {
    pub fn load_default() -> Result<Self, SettingsError> {
        let state_home = storage::default_state_dir().ok_or(SettingsError::UnresolvableHome)?;
        Self::load_or_create(&state_home)
    }

    pub fn load_or_create(directory: &Path) -> Result<Self, SettingsError> {
        storage::prepare_private_directory(directory)?;
        let path = directory.join(SETTINGS_FILE);
        if path.exists() {
            storage::require_private_file(&path)?;
            let bytes = std::fs::read(&path)?;
            let values: StoredSettings =
                serde_json::from_slice(&bytes).map_err(|_| SettingsError::InvalidDocument)?;
            if values.schema_version != SETTINGS_SCHEMA_VERSION || !valid_profile(&values) {
                return Err(SettingsError::InvalidDocument);
            }
            return Ok(Self { values, path });
        }

        let settings = Self {
            values: StoredSettings::default(),
            path,
        };
        settings.persist()?;
        Ok(settings)
    }

    pub fn values(&self) -> &StoredSettings {
        &self.values
    }

    /// Serializes the current settings, including the schema version.
    pub fn to_json(&self) -> Value {
        json!(self.values)
    }

    /// Applies a shallow patch of known keys and persists the result.
    pub fn update(&mut self, patch: &Value) -> Result<(), SettingsError> {
        let Some(patch) = patch.as_object() else {
            return Err(SettingsError::InvalidPatch);
        };
        let mut merged = self.to_json();
        let Some(document) = merged.as_object_mut() else {
            return Err(SettingsError::InvalidDocument);
        };
        if patch.keys().any(|key| !document.contains_key(key)) {
            return Err(SettingsError::UnknownKey);
        }
        for (key, value) in patch {
            document.insert(key.clone(), value.clone());
        }
        let mut values: StoredSettings =
            serde_json::from_value(merged).map_err(|_| SettingsError::InvalidPatch)?;
        if !valid_profile(&values) {
            return Err(SettingsError::InvalidPatch);
        }
        // Any profile-key change publishes a new revision so the paired peer
        // applies it and ignores replays. The bump persists atomically with
        // the patch itself.
        const PROFILE_KEYS: &[&str] = &["displayName", "statusMessage", "avatar", "avatarType"];
        if patch.keys().any(|key| PROFILE_KEYS.contains(&key.as_str())) {
            values.profile_revision = values.profile_revision.saturating_add(1).max(1);
        }

        let previous = std::mem::replace(&mut self.values, values);
        if let Err(error) = self.persist() {
            self.values = previous;
            return Err(error);
        }
        Ok(())
    }

    fn persist(&self) -> Result<(), SettingsError> {
        let bytes =
            serde_json::to_vec_pretty(&self.values).map_err(|_| SettingsError::InvalidDocument)?;
        storage::write_private_atomic(&self.path, &bytes)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use uuid::Uuid;

    use super::*;

    fn temporary_directory() -> PathBuf {
        env::temp_dir().join(format!("harbor-settings-test-{}", Uuid::new_v4()))
    }

    #[test]
    fn fresh_settings_match_the_shipped_fixture_defaults() {
        let directory = temporary_directory();
        let settings = Settings::load_or_create(&directory).unwrap();
        assert_eq!(settings.values().locale, "en");
        assert!(settings.values().display_name.is_empty());
        assert!(settings.values().status_message.is_empty());
        assert_eq!(settings.values().profile_revision, 1);
        assert!(settings.values().avatar.is_empty());
        assert_eq!(settings.values().avatar_type, "image");
        assert_eq!(settings.values().appearance_mode, "dark");
        assert_eq!(settings.values().accent_color, "ocean");
        assert!((settings.values().glass_intensity - 1.0).abs() < f64::EPSILON);
        assert!((settings.values().animation_intensity - 1.0).abs() < f64::EPSILON);
        assert!(settings.values().particles_enabled);
        assert_eq!(settings.values().ocean_variant, "lagoon");
        assert_eq!(settings.values().corner_radius, "soft");
        assert_eq!(settings.values().density, "comfortable");
        assert!(settings.values().widget_enabled);
        assert_eq!(settings.values().widget_position, "bottomRight");
        assert!(settings.values().widget_show_activity);
        assert!(settings.values().widget_show_avatar);
        assert!(settings.values().widget_show_call_presence);
        // Mobile intents: this install is a desktop endpoint, the
        // persistent call ships ON, sensitive phone shares ship OFF.
        assert_eq!(settings.values().device_type, "desktop");
        assert!(settings.values().persistent_call);
        assert!(!settings.values().share_location);
        assert!(!settings.values().share_phone_activity);
        assert!(!settings.values().share_phone_notifications);
        assert!((settings.values().microphone_volume - 0.72).abs() < f64::EPSILON);
        assert!(directory.join(SETTINGS_FILE).exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn updates_merge_known_keys_and_persist_atomically() {
        let directory = temporary_directory();
        let mut settings = Settings::load_or_create(&directory).unwrap();
        settings
            .update(&json!({
                "appearanceMode": "light",
                "microphoneVolume": 0.4,
                "displayName": "Ari",
                "statusMessage": "Building something new",
                "avatar": "data:image/png;base64,AA==",
                "avatarType": "image"
            }))
            .unwrap();
        assert_eq!(settings.values().display_name, "Ari");
        assert_eq!(settings.values().status_message, "Building something new");
        assert_eq!(settings.values().profile_revision, 2);

        // Non-profile keys leave the revision alone; each profile edit bumps.
        settings.update(&json!({"appearanceMode": "light"})).unwrap();
        assert_eq!(settings.values().profile_revision, 2);
        settings.update(&json!({"statusMessage": "Away"})).unwrap();
        assert_eq!(settings.values().profile_revision, 3);
        assert_eq!(settings.values().avatar, "data:image/png;base64,AA==");
        assert_eq!(settings.values().avatar_type, "image");
        assert_eq!(settings.values().appearance_mode, "light");
        assert!((settings.values().microphone_volume - 0.4).abs() < f64::EPSILON);

        // Appearance personalization round-trips and persists.
        settings
            .update(&json!({
                "accentColor": "#3AA9DC",
                "glassIntensity": 0.6,
                "animationIntensity": 0.5,
                "particlesEnabled": false,
                "oceanVariant": "abyss",
                "cornerRadius": "medium",
                "density": "compact",
            }))
            .unwrap();
        assert_eq!(settings.values().accent_color, "#3AA9DC");
        assert!((settings.values().glass_intensity - 0.6).abs() < f64::EPSILON);
        assert!((settings.values().animation_intensity - 0.5).abs() < f64::EPSILON);
        assert!(!settings.values().particles_enabled);
        assert_eq!(settings.values().ocean_variant, "abyss");
        assert_eq!(settings.values().corner_radius, "medium");
        assert_eq!(settings.values().density, "compact");

        // Widget visibility round-trips and persists.
        settings
            .update(&json!({
                "widgetEnabled": false,
                "widgetPosition": "topLeft",
                "widgetShowActivity": false,
                "widgetShowAvatar": false,
                "widgetShowCallPresence": false,
            }))
            .unwrap();
        assert!(!settings.values().widget_enabled);
        assert_eq!(settings.values().widget_position, "topLeft");
        assert!(!settings.values().widget_show_activity);
        assert!(!settings.values().widget_show_avatar);
        assert!(!settings.values().widget_show_call_presence);

        // Every shipped ocean variant persists.
        for variant in ["lagoon", "rose", "ember", "onyx", "forest", "dusk", "sunrise"] {
            settings.update(&json!({"oceanVariant": variant})).unwrap();
            assert_eq!(settings.values().ocean_variant, variant);
        }

        // Mobile intents round-trip and persist; device type is constrained.
        settings
            .update(&json!({
                "deviceType": "mobile",
                "persistentCall": false,
                "shareLocation": true,
                "sharePhoneActivity": true,
                "sharePhoneNotifications": true,
            }))
            .unwrap();
        assert_eq!(settings.values().device_type, "mobile");
        assert!(!settings.values().persistent_call);
        assert!(settings.values().share_location);
        assert!(settings.values().share_phone_activity);
        assert!(settings.values().share_phone_notifications);

        let reloaded = Settings::load_or_create(&directory).unwrap();
        assert_eq!(reloaded.values(), settings.values());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn unknown_or_misfit_patch_keys_are_rejected_without_writes() {
        let directory = temporary_directory();
        let mut settings = Settings::load_or_create(&directory).unwrap();
        assert!(matches!(
            settings.update(&json!({"notASetting": true})),
            Err(SettingsError::UnknownKey)
        ));
        assert!(matches!(
            settings.update(&json!({"appearanceMode": 42})),
            Err(SettingsError::InvalidPatch)
        ));
        assert!(matches!(
            settings.update(&json!([1, 2])),
            Err(SettingsError::InvalidPatch)
        ));
        assert!(matches!(
            settings.update(&json!({"avatarType": "video"})),
            Err(SettingsError::InvalidPatch)
        ));
        for avatar in [
            "file:///tmp/avatar.png",
            "https://example.com/avatar.png",
            "data:image/png;base64,not-base64",
            "data:image/png;base64,",
        ] {
            assert!(matches!(
                settings.update(&json!({"avatar": avatar})),
                Err(SettingsError::InvalidPatch)
            ));
        }
        assert!(matches!(
            settings.update(&json!({"displayName": "x".repeat(DISPLAY_NAME_MAX_CHARS + 1)})),
            Err(SettingsError::InvalidPatch)
        ));
        assert!(matches!(
            settings.update(&json!({"statusMessage": "x".repeat(STATUS_MESSAGE_MAX_CHARS + 1)})),
            Err(SettingsError::InvalidPatch)
        ));
        // Appearance impersonation is rejected: unknown presets, malformed
        // hex, out-of-range strengths, and unknown enum values.
        for patch in [
            json!({"accentColor": "infrared"}),
            json!({"accentColor": "#12345"}),
            json!({"accentColor": "3AA9DC"}),
            json!({"glassIntensity": 1.5}),
            json!({"glassIntensity": -0.1}),
            json!({"animationIntensity": 2.0}),
            json!({"oceanVariant": "swamp"}),
            json!({"cornerRadius": "round"}),
            json!({"density": "tight"}),
            json!({"widgetPosition": "center"}),
            json!({"deviceType": "tablet"}),
        ] {
            assert!(
                matches!(settings.update(&patch), Err(SettingsError::InvalidPatch)),
                "accepted invalid patch {patch}"
            );
        }
        assert_eq!(settings.values().appearance_mode, "dark");
        assert!(settings.values().display_name.is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rejects_existing_settings_file_with_broad_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let directory = temporary_directory();
        let _settings = Settings::load_or_create(&directory).unwrap();
        let path = directory.join(SETTINGS_FILE);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(matches!(
            Settings::load_or_create(&directory),
            Err(SettingsError::Storage(StorageError::InsecurePermissions))
        ));
        std::fs::remove_dir_all(directory).unwrap();
    }
}
