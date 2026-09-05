//! Real local activity: process observation, classification, and redaction.
//!
//! The engine consumes raw process observations (the `LocalActivityRecord`
//! material: pid, executable path, command line) and keeps them private. The
//! only records it emits are sanitized `ShareableActivityRecord`s: a stable
//! id, a category, a non-localized localization key with parameters, and a
//! UTC timestamp. No pid, path, command line, or window title ever reaches a
//! serialized record, and the `game_titles` policy re-redacts titles at
//! serialization time so a privacy flip applies to the whole timeline at
//! once.
//!
//! `RemoteActivityRecord` is the schema every future peer-delivered activity
//! record must pass (`validate_remote_record`); delivery itself waits for the
//! session phase and never involves the control-plane server.

use std::collections::HashMap;

use serde_json::{Value, json};
use uuid::Uuid;

use crate::appicon::{APP_ID_MAX, normalize_app_id, resolve_icon_key, sanitize_icon_key};

/// Kept timeline entries (shareable records). Enough for a day of moments;
/// the stats card carries the weekly aggregates, not the timeline.
pub const TIMELINE_CAP: usize = 50;
/// Sanitized labels never exceed this many characters.
pub const LABEL_MAX: usize = 40;
/// A process restarting inside this window after its close is a relaunch of
/// the same session from the user's point of view: it still accumulates
/// monitored time, but it does not flood the timeline.
pub const REOPEN_DEBOUNCE_SECONDS: u64 = 30;
/// Rolling window the weekly stats cover.
pub const STATS_WINDOW_DAYS: u64 = 7;
/// Remote records may claim a timestamp this far into the future at most
/// (clock skew tolerance for the peer-delivery phase).
pub const REMOTE_FUTURE_TOLERANCE_SECONDS: u64 = 300;

/// Process basenames classified as games. Everything else with an executable
/// is an application; the monitor lifecycle itself is `system`. Matching is
/// on the sanitized, lowercased basename.
const KNOWN_GAMES: &[&str] = &[
    "minecraft",
    "minetest",
    "openttd",
    "freeciv",
    "wesnoth",
    "0ad",
    "supermariowar",
    "stardewvalley",
    "terraria",
    "valheim",
    "factorio",
    "rimworld",
    "dwarffortress",
    "hollowknight",
    "celeste",
    "hades",
    "deadcells",
    "portal2",
    "witcher3",
    "witcher3.exe",
    "cyberpunk2077",
    "eldenring",
    "cs2",
    "csgo",
    "dota2",
    "teamfortress2",
    "left4dead2",
    "gta5",
    "rdr2",
    "skyrim",
    "fallout4",
    "eldenring.exe",
];

/// Infrastructure and session helpers never represent a human moment. Keep
/// this denylist local to classification so their paths and command lines
/// remain private and they never reach the shareable timeline.
const TECHNICAL_PROCESSES: &[&str] = &[
    "bash",
    "sh",
    "dash",
    "zsh",
    "fish",
    "waitpid",
    "systemctl",
    "systemd",
    "systemd-logind",
    "systemd-journal",
    "dbus",
    "dbus-broker",
    "dbus-daemon",
    "at-spi-bus-launcher",
    "at-spi2-registryd",
    "gnome-keyring-daemon",
    "gnome-keyring",
    "gpg-agent",
    "ssh-agent",
    "start-hyprland",
    "hyprland",
    "sway",
    "xwayland",
    "xdg-desktop-portal",
    "xdg-desktop-portal-hyprland",
    "xdg-desktop-portal-gtk",
    "xdg-permission-store",
    "pipewire",
    "pipewire-pulse",
    "wireplumber",
    "pulseaudio",
    "harbor",
    "harbor-core",
    "harbor-media",
];

/// A raw process observation: the local-only material. Never serialized.
#[derive(Debug, Clone)]
pub struct RawObservation {
    pub pid: u32,
    pub exe_path: Option<String>,
    pub command_line: Option<String>,
}

/// The local-only record the engine retains for a tracked process. Never
/// serialized; exists so the privacy boundary is a type, not a discipline.
/// Nothing reads it yet — later local diagnostics may — but holding the raw
/// material here is what keeps it out of every shareable record.
#[allow(dead_code)]
#[derive(Debug, Clone)]
struct LocalActivityRecord {
    pid: u32,
    exe_path: Option<String>,
    command_line: Option<String>,
}

/// A sanitized, shareable activity record: the only shape that ever leaves
/// the engine.
#[derive(Debug, Clone)]
pub struct ActivityEntry {
    pub id: String,
    pub category: &'static str,
    /// The sanitized display label behind the title params. Label-less
    /// system entries are empty and never leave the device.
    pub label: String,
    /// Stable, cross-platform app identity (`firefox`, `code`, `vlc`).
    /// Derived from the executable basename; empty only for system entries.
    pub app_id: String,
    /// Theme-safe icon key (freedesktop theme name on Linux, exe basename
    /// on Windows). Never an absolute path. Empty when unknown — the UI
    /// then falls back to the category icon.
    pub icon_key: String,
    pub title_key: &'static str,
    pub title_params: Value,
    pub description_key: &'static str,
    pub description_params: Value,
    pub occurred_at: u64,
}

impl ActivityEntry {
    fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "category": self.category,
            "kind": "opened",
            "label": self.label,
            "app_id": self.app_id,
            "icon": self.icon_key,
            "title_key": self.title_key,
            "title_params": self.title_params,
            "description_key": self.description_key,
            "description_params": self.description_params,
            "occurred_at": self.occurred_at,
        })
    }
}

/// A remote activity record as a peer would deliver it once the session
/// phase exists. Schema-validated, never trusted raw.
#[derive(Debug, Clone, PartialEq)]
pub struct RemoteActivityRecord {
    pub id: String,
    pub sender: String,
    pub category: String,
    pub kind: String,
    pub label: String,
    /// Peer-resolved app identity/icon keys. Optional for backward
    /// compatibility: older peers send records without them.
    pub app_id: String,
    pub icon_key: String,
    pub occurred_at: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum ActivityError {
    #[error("activity record is missing a required field")]
    MissingField,
    #[error("activity record has an invalid field: {0}")]
    InvalidField(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MonitorState {
    /// No monitor ran yet (the process just started).
    #[default]
    Idle,
    Running,
    /// The platform has no monitor implementation.
    Unsupported,
    /// The monitor exists but observation failed.
    Unavailable,
}

impl MonitorState {
    pub fn as_str(self) -> &'static str {
        match self {
            MonitorState::Idle => "idle",
            MonitorState::Running => "running",
            MonitorState::Unsupported => "unsupported",
            MonitorState::Unavailable => "unavailable",
        }
    }
}

#[derive(Debug)]
struct OpenProcess {
    #[allow(dead_code)] // the privacy boundary: held locally, never read out
    local: LocalActivityRecord,
    label: String,
    app_id: String,
    icon_key: String,
    category: &'static str,
    opened_at: u64,
    /// Whether the open was announced in the timeline (a debounced relaunch
    /// is tracked for duration but not announced).
    announced: bool,
}

#[derive(Debug, Clone, Copy, Default)]
struct DailyStat {
    game_opens: u64,
    app_opens: u64,
    monitored_seconds: u64,
}

/// day bucket (days since epoch) -> stat
type Stats = HashMap<u64, DailyStat>;

/// The activity state machine. Owned by the core; shared with the monitor
/// thread through a mutex.
#[derive(Debug, Default)]
pub struct ActivityEngine {
    monitor: MonitorState,
    known: HashMap<u32, OpenProcess>,
    last_closed: HashMap<String, u64>,
    timeline: Vec<ActivityEntry>,
    stats: Stats,
}

impl ActivityEngine {
    /// The monitor observed the platform successfully for the first time.
    /// Returns whether the timeline changed.
    pub fn mark_monitor_started(&mut self, now: u64) -> bool {
        if self.monitor == MonitorState::Running {
            return false;
        }
        self.monitor = MonitorState::Running;
        self.timeline.push(ActivityEntry {
            id: Uuid::new_v4().to_string(),
            category: "system",
            label: String::new(),
            app_id: String::new(),
            icon_key: String::new(),
            title_key: "activity.event.monitorStarted",
            title_params: json!({}),
            description_key: "",
            description_params: json!({}),
            occurred_at: now,
        });
        self.truncate_timeline();
        true
    }

    /// The platform has no monitor or observation failed. `unavailable`
    /// (transient failure) reports once; a later success recovers to
    /// `running` and monitoring resumes.
    pub fn mark_monitor_unavailable(&mut self, now: u64, unsupported: bool) -> bool {
        let next = if unsupported {
            MonitorState::Unsupported
        } else {
            MonitorState::Unavailable
        };
        if self.monitor == next {
            return false;
        }
        self.monitor = next;
        self.timeline.push(ActivityEntry {
            id: Uuid::new_v4().to_string(),
            category: "system",
            label: String::new(),
            app_id: String::new(),
            icon_key: String::new(),
            title_key: "activity.event.monitorUnavailable",
            title_params: json!({}),
            description_key: "",
            description_params: json!({}),
            occurred_at: now,
        });
        self.truncate_timeline();
        true
    }

    /// Ingests one scan of raw observations and diffs it against the known
    /// process set. Returns whether anything user-visible changed.
    pub fn ingest(&mut self, now: u64, observations: Vec<RawObservation>) -> bool {
        let mut changed = false;

        for observation in &observations {
            if self.known.contains_key(&observation.pid) {
                continue;
            }
            let Some(label) = observation
                .exe_path
                .as_deref()
                .and_then(basename)
                .and_then(sanitize_label)
            else {
                // Kernel threads, zombies, and unnamed processes are not
                // user-visible activity; skip them entirely.
                continue;
            };
            let lowered = label.to_lowercase();
            if TECHNICAL_PROCESSES.contains(&lowered.as_str()) {
                continue;
            }
            // Stable app identity for icons. Falls back to the sanitized
            // label when the path yields no app id (e.g. tasklist image
            // names on Windows arrive without a directory).
            let app_id = observation
                .exe_path
                .as_deref()
                .and_then(normalize_app_id)
                .unwrap_or_else(|| lowered.clone());
            let category = if KNOWN_GAMES.contains(&lowered.as_str())
                || KNOWN_GAMES.contains(&app_id.as_str())
            {
                "game"
            } else {
                "app"
            };
            // Dedup: another process with the same label or app id is
            // already open (helper processes, multi-window apps, or the
            // same app from two install paths) — track it, announce
            // nothing.
            let duplicate = self
                .known
                .values()
                .any(|process| process.label == label || (!app_id.is_empty() && process.app_id == app_id));
            let debounced = self
                .last_closed
                .get(&label)
                .is_some_and(|closed_at| now.saturating_sub(*closed_at) <= REOPEN_DEBOUNCE_SECONDS);
            let announced = !duplicate && !debounced;

            // Resolve the icon key only for announced opens: the Linux
            // lookup scans .desktop files and must not run for every
            // helper process on every tick.
            let icon_key = if announced {
                observation
                    .exe_path
                    .as_deref()
                    .and_then(resolve_icon_key)
                    .and_then(|key| sanitize_icon_key(&key))
                    .unwrap_or_default()
            } else {
                String::new()
            };

            if announced {
                self.timeline.push(ActivityEntry {
                    id: Uuid::new_v4().to_string(),
                    category,
                    label: label.clone(),
                    app_id: app_id.clone(),
                    icon_key: icon_key.clone(),
                    title_key: if category == "game" {
                        "activity.event.gameOpened"
                    } else {
                        "activity.event.appOpened"
                    },
                    title_params: json!({ if category == "game" { "game" } else { "app" }: label}),
                    description_key: if category == "game" {
                        "activity.event.gameLaunched"
                    } else {
                        "activity.event.applicationOpened"
                    },
                    description_params: json!({}),
                    occurred_at: now,
                });
                let day = now / 86_400;
                let stat = self.stats.entry(day).or_default();
                if category == "game" {
                    stat.game_opens += 1;
                } else {
                    stat.app_opens += 1;
                }
                changed = true;
            }

            self.known.insert(
                observation.pid,
                OpenProcess {
                    local: LocalActivityRecord {
                        pid: observation.pid,
                        exe_path: observation.exe_path.clone(),
                        command_line: observation.command_line.clone(),
                    },
                    label,
                    app_id,
                    icon_key,
                    category,
                    opened_at: now,
                    announced,
                },
            );
        }

        // Processes that vanished since the last scan closed.
        let observed_pids: Vec<u32> = observations.iter().map(|o| o.pid).collect();
        let closed: Vec<u32> = self
            .known
            .keys()
            .filter(|pid| !observed_pids.contains(pid))
            .copied()
            .collect();
        for pid in closed {
            if let Some(process) = self.known.remove(&pid) {
                let duration = now.saturating_sub(process.opened_at);
                let day = process.opened_at / 86_400;
                self.stats.entry(day).or_default().monitored_seconds += duration;
                self.last_closed.insert(process.label.clone(), now);
                // `announced == false` closes change nothing user-visible.
                let _ = process.category;
                changed |= process.announced;
            }
        }

        if changed {
            self.truncate_timeline();
        }
        self.prune_stats(now);
        changed
    }

    /// Serializes the timeline with the game-titles policy applied. This is
    /// the only place redaction happens, so a policy flip re-redacts the
    /// whole timeline without touching stored facts.
    pub fn timeline_json(&self, game_titles: bool) -> Vec<Value> {
        self.timeline
            .iter()
            .map(|entry| {
                let mut record = entry.to_json();
                if entry.category == "game" && !game_titles {
                    record["title_key"] = json!("activity.event.gameOpenedGeneric");
                    record["title_params"] = json!({});
                }
                record
            })
            .collect()
    }

    /// The peer-facing view of the timeline: one validated-schema record per
    /// announced open. Every record passes `validate_remote_record` by
    /// construction, and each field already passed `sanitize_label` at
    /// ingest. System entries carry no label and never leave the device; a
    /// hidden game title omits the whole record rather than inventing a
    /// placeholder name.
    pub fn shareable_json(&self, game_titles: bool, sender: &str) -> Vec<Value> {
        self.timeline
            .iter()
            .filter(|entry| !entry.label.is_empty())
            .filter(|entry| entry.category != "game" || game_titles)
            .map(|entry| {
                json!({
                    "id": entry.id,
                    "sender": sender,
                    "category": entry.category,
                    "kind": "opened",
                    "label": entry.label,
                    "app_id": entry.app_id,
                    "icon": entry.icon_key,
                    "occurred_at": entry.occurred_at,
                })
            })
            .collect()
    }

    /// Weekly aggregates over the rolling window: game/app open counts and
    /// whole hours of monitored time. Monitored time only accumulates while
    /// the app runs — it never invents history.
    pub fn stats_json(&self, now: u64) -> Value {
        let cutoff_day = (now / 86_400).saturating_sub(STATS_WINDOW_DAYS - 1);
        let mut games = 0;
        let mut apps = 0;
        let mut seconds = 0;
        for (day, stat) in &self.stats {
            if *day >= cutoff_day {
                games += stat.game_opens;
                apps += stat.app_opens;
                seconds += stat.monitored_seconds;
            }
        }
        // Sessions still open count their elapsed time toward today.
        for process in self.known.values() {
            if process.opened_at / 86_400 >= cutoff_day {
                seconds += now.saturating_sub(process.opened_at);
            }
        }
        json!({
            "games": games,
            "apps": apps,
            "hours": seconds / 3_600,
        })
    }

    pub fn monitor_state(&self) -> MonitorState {
        self.monitor
    }

    fn truncate_timeline(&mut self) {
        if self.timeline.len() > TIMELINE_CAP {
            let excess = self.timeline.len() - TIMELINE_CAP;
            self.timeline.drain(0..excess);
        }
    }

    fn prune_stats(&mut self, now: u64) {
        let cutoff_day = (now / 86_400).saturating_sub(STATS_WINDOW_DAYS - 1);
        self.stats.retain(|day, _| *day >= cutoff_day);
    }
}

/// Validates a peer-delivered activity record against the shareable schema.
/// Delivery is a later phase; validation is contract-tested now so the
/// future transport cannot relax the rules silently.
pub fn validate_remote_record(
    value: &Value,
    now: u64,
) -> Result<RemoteActivityRecord, ActivityError> {
    let object = value.as_object().ok_or(ActivityError::MissingField)?;
    let field = |key: &'static str| object.get(key).ok_or(ActivityError::MissingField);

    let id = field("id")?
        .as_str()
        .ok_or(ActivityError::InvalidField("id"))?;
    if id.is_empty() || id.len() > 64 {
        return Err(ActivityError::InvalidField("id"));
    }
    let sender = field("sender")?
        .as_str()
        .ok_or(ActivityError::InvalidField("sender"))?;
    if sender.is_empty() || sender.len() > 64 {
        return Err(ActivityError::InvalidField("sender"));
    }
    let category = field("category")?
        .as_str()
        .ok_or(ActivityError::InvalidField("category"))?;
    if !matches!(category, "game" | "app" | "system") {
        return Err(ActivityError::InvalidField("category"));
    }
    let kind = field("kind")?
        .as_str()
        .ok_or(ActivityError::InvalidField("kind"))?;
    if !matches!(kind, "opened" | "closed") {
        return Err(ActivityError::InvalidField("kind"));
    }
    let label = field("label")?
        .as_str()
        .ok_or(ActivityError::InvalidField("label"))?;
    if label.is_empty()
        || label.len() > LABEL_MAX
        || label
            .chars()
            .any(|c| c.is_control() || c == '/' || c == '\\' || c == ':' || c == '*' || c == '?')
    {
        return Err(ActivityError::InvalidField("label"));
    }
    let occurred_at = field("occurred_at")?
        .as_u64()
        .ok_or(ActivityError::InvalidField("occurred_at"))?;
    if occurred_at > now.saturating_add(REMOTE_FUTURE_TOLERANCE_SECONDS) {
        return Err(ActivityError::InvalidField("occurred_at"));
    }
    // App identity and icon keys are optional for backward compatibility
    // with peers that predate per-app icons. When present they must be
    // theme-safe; otherwise they degrade to empty (category fallback).
    let app_id = match object.get("app_id") {
        None => String::new(),
        Some(value) => {
            let text = value.as_str().ok_or(ActivityError::InvalidField("app_id"))?;
            if text.is_empty() {
                String::new()
            } else if text.len() > APP_ID_MAX
                || !text
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'))
            {
                return Err(ActivityError::InvalidField("app_id"));
            } else {
                text.to_string()
            }
        }
    };
    let icon_key = match object.get("icon") {
        None => String::new(),
        Some(value) => {
            let text = value.as_str().ok_or(ActivityError::InvalidField("icon"))?;
            if text.is_empty() {
                String::new()
            } else {
                match sanitize_icon_key(text) {
                    Some(key) => key,
                    None => return Err(ActivityError::InvalidField("icon")),
                }
            }
        }
    };

    Ok(RemoteActivityRecord {
        id: id.to_string(),
        sender: sender.to_string(),
        category: category.to_string(),
        kind: kind.to_string(),
        label: label.to_string(),
        app_id,
        icon_key,
        occurred_at,
    })
}

/// The last path component of an executable path, or the whole string when
/// it contains no separator. Handles both `/` (Linux) and `\` (Windows)
/// plus the `\\?\` extended-path prefix.
fn basename(path: &str) -> Option<&str> {
    let mut trimmed = path.trim();
    if trimmed.is_empty() {
        return None;
    }
    for prefix in ["\\\\?\\", "\\\\.\\"] {
        if let Some(rest) = trimmed.strip_prefix(prefix) {
            trimmed = rest;
            break;
        }
    }
    let trimmed = trimmed.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() {
        return None;
    }
    let after_slash = trimmed.rsplit('/').next().unwrap_or(trimmed);
    after_slash
        .rsplit('\\')
        .next()
        .filter(|name| !name.is_empty())
}

/// Turns a raw executable basename into a displayable label: strips a
/// trailing executable suffix (case-insensitive), replaces control and
/// separator characters with spaces, collapses whitespace, caps length.
/// Returns `None` when nothing presentable remains.
pub fn sanitize_label(raw: &str) -> Option<String> {
    let cut: String = raw
        .chars()
        .map(|c| {
            if c.is_control() {
                ' '
            } else if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '+' | '@') {
                c
            } else {
                ' '
            }
        })
        .collect();
    let mut label = cut.split_whitespace().collect::<Vec<_>>().join(" ");
    // Case-insensitive so `Code.EXE` (Windows) sanitizes like `code.exe`.
    let lowered = label.to_lowercase();
    for suffix in [".exe", ".bin"] {
        if lowered.ends_with(suffix) {
            label = label[..label.len() - suffix.len()].to_string();
            break;
        }
    }
    if label.chars().count() > LABEL_MAX {
        label = label.chars().take(LABEL_MAX).collect();
    }
    let label = label.trim().to_string();
    if label.is_empty() || label.chars().all(|c| c == '.') {
        None
    } else {
        Some(label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(pid: u32, exe: &str) -> RawObservation {
        RawObservation {
            pid,
            exe_path: Some(exe.to_string()),
            command_line: Some(format!("{exe} --flag secret-value")),
        }
    }

    #[test]
    fn sanitize_label_strips_paths_args_and_controls() {
        assert_eq!(sanitize_label("minecraft"), Some("minecraft".to_string()));
        assert_eq!(sanitize_label("game.exe"), Some("game".to_string()));
        assert_eq!(sanitize_label("my game"), Some("my game".to_string()));
        assert_eq!(
            sanitize_label("weird\tname$$"),
            Some("weird name".to_string())
        );
        assert_eq!(sanitize_label("a\nb"), Some("a b".to_string()));
        assert_eq!(sanitize_label(""), None);
        let long = "x".repeat(100);
        assert_eq!(sanitize_label(&long), Some("x".repeat(LABEL_MAX)));
    }

    #[test]
    fn open_and_close_round_trip_keeps_local_material_private() {
        let mut engine = ActivityEngine::default();
        assert!(engine.mark_monitor_started(1_000));
        assert!(engine.ingest(
            1_000,
            vec![
                observation(7, "/usr/bin/vlc"),
                observation(9, "/opt/minecraft-launcher/minecraft")
            ]
        ));

        // The shareable timeline carries keys and labels only.
        let timeline = engine.timeline_json(true);
        assert_eq!(timeline.len(), 3); // monitor started + two opens
        let serialized = serde_json::to_string(&timeline).unwrap();
        assert!(!serialized.contains("usr"), "path leaked: {serialized}");
        assert!(!serialized.contains("opt"), "path leaked: {serialized}");
        assert!(!serialized.contains("flag"), "cmdline leaked: {serialized}");
        assert!(
            !serialized.contains("secret"),
            "cmdline leaked: {serialized}"
        );

        // Closing both processes accumulates monitored time.
        assert!(engine.ingest(1_000 + 3_600, vec![]));
        let stats = engine.stats_json(1_000 + 3_600);
        assert_eq!(stats["apps"], 1);
        assert_eq!(stats["games"], 1);
        assert_eq!(stats["hours"], 2);
    }

    #[test]
    fn technical_processes_never_become_moments() {
        let mut engine = ActivityEngine::default();
        engine.mark_monitor_started(0);
        assert!(!engine.ingest(
            0,
            vec![
                observation(1, "/usr/bin/gnome-keyring-daemon"),
                observation(2, "/usr/bin/dbus-broker"),
                observation(3, "/usr/bin/systemctl"),
                observation(4, "/bin/bash"),
                observation(5, "/usr/bin/start-hyprland"),
                observation(6, "/usr/bin/harbor-media"),
            ]
        ));
        assert_eq!(engine.timeline_json(true).len(), 1);
        assert_eq!(engine.stats_json(0)["apps"], 0);
    }

    #[test]
    fn game_title_policy_redacts_at_serialization_time() {
        let mut engine = ActivityEngine::default();
        engine.mark_monitor_started(0);
        engine.ingest(0, vec![observation(1, "/opt/minecraft")]);

        let visible = engine.timeline_json(true);
        let game = visible
            .iter()
            .find(|entry| entry["category"] == "game")
            .unwrap();
        assert_eq!(game["title_key"], "activity.event.gameOpened");
        assert_eq!(game["title_params"]["game"], "minecraft");

        let redacted = engine.timeline_json(false);
        let game = redacted
            .iter()
            .find(|entry| entry["category"] == "game")
            .unwrap();
        assert_eq!(game["title_key"], "activity.event.gameOpenedGeneric");
        assert!(game["title_params"].as_object().unwrap().is_empty());
    }

    #[test]
    fn relaunch_within_the_debounce_window_floods_nothing() {
        let mut engine = ActivityEngine::default();
        engine.mark_monitor_started(0);
        engine.ingest(0, vec![observation(1, "/usr/bin/game")]);
        let entries_after_open = engine.timeline_json(true).len();

        // Close and immediately relaunch: tracked, but not announced.
        assert!(engine.ingest(1, vec![]));
        assert!(!engine.ingest(2, vec![observation(2, "/usr/bin/game")]));
        assert_eq!(engine.timeline_json(true).len(), entries_after_open);

        // The relaunch still accumulates monitored time when it closes.
        engine.ingest(3_600 + 60, vec![]);
        assert!(engine.stats_json(3_600 + 60)["hours"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn duplicate_labels_announce_once() {
        let mut engine = ActivityEngine::default();
        engine.mark_monitor_started(0);
        assert!(engine.ingest(
            0,
            vec![
                observation(1, "/usr/bin/app"),
                observation(2, "/usr/bin/app")
            ]
        ));
        // The third process with the same label is tracked but announced
        // nothing; the ingest result reflects the announced closes' stats.
        assert!(engine.ingest(1, vec![observation(3, "/usr/bin/app")]));
        let app_entries = engine
            .timeline_json(true)
            .iter()
            .filter(|entry| entry["title_key"] == "activity.event.appOpened")
            .count();
        assert_eq!(app_entries, 1);

        // A duplicate closing without having been announced changes nothing.
        assert!(!engine.ingest(2, vec![]));
    }

    #[test]
    fn unavailable_monitor_reports_once_and_recovers() {
        let mut engine = ActivityEngine::default();
        assert!(engine.mark_monitor_unavailable(0, true));
        assert_eq!(engine.monitor_state(), MonitorState::Unsupported);
        assert!(!engine.mark_monitor_unavailable(1, true));
        assert!(engine.mark_monitor_started(2));
        assert_eq!(engine.monitor_state(), MonitorState::Running);
    }

    #[test]
    fn timeline_is_capped_and_stats_prune_to_the_week() {
        let mut engine = ActivityEngine::default();
        engine.mark_monitor_started(0);
        // Distinct labels so the debounce window never suppresses an open.
        for pid in 0..(TIMELINE_CAP as u32 + 20) {
            let exe = format!("/usr/bin/app-{pid}");
            engine.ingest(pid as u64 + 1, vec![observation(pid + 100, &exe)]);
            engine.ingest(pid as u64 + 2, vec![]);
        }
        assert_eq!(engine.timeline_json(true).len(), TIMELINE_CAP);

        // Stats older than the window disappear.
        let now = (STATS_WINDOW_DAYS + 2) * 86_400;
        assert!(engine.ingest(now, vec![observation(999, "/usr/bin/old")]));
        let stats = engine.stats_json(now);
        assert_eq!(stats["apps"], 1);
    }

    #[test]
    fn remote_records_validate_against_the_schema() {
        let now = 5_000;
        let good = json!({
            "id": "abc-123",
            "sender": "device-uuid",
            "category": "game",
            "kind": "opened",
            "label": "minecraft",
            "occurred_at": now - 10
        });
        let record = validate_remote_record(&good, now).unwrap();
        assert_eq!(record.label, "minecraft");
        assert_eq!(record.category, "game");

        // Path separators and control characters are never valid labels.
        let mut traversal = good.clone();
        traversal["label"] = json!("../../etc/passwd");
        assert!(matches!(
            validate_remote_record(&traversal, now),
            Err(ActivityError::InvalidField("label"))
        ));

        let mut future = good.clone();
        future["occurred_at"] = json!(now + REMOTE_FUTURE_TOLERANCE_SECONDS + 1);
        assert!(matches!(
            validate_remote_record(&future, now),
            Err(ActivityError::InvalidField("occurred_at"))
        ));

        let mut category = good.clone();
        category["category"] = json!("media");
        assert!(matches!(
            validate_remote_record(&category, now),
            Err(ActivityError::InvalidField("category"))
        ));

        let mut kind = good;
        kind["kind"] = json!("streaming");
        assert!(matches!(
            validate_remote_record(&kind, now),
            Err(ActivityError::InvalidField("kind"))
        ));
    }
}
