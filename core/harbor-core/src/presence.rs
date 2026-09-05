//! Local presence from multiple weak signals, one deliberate verdict.
//!
//! Presence is decided on this machine from a private
//! [`UserActivitySnapshot`] fed by the Qt side's native detector. The snapshot
//! is internal forever: only the committed aggregate (ONLINE/AWAY/OFFLINE)
//! is published, and only the partner-facing lease vocabulary
//! (ONLINE/IDLE/OFFLINE) ever crosses the wire.
//!
//! The rules follow the product contract:
//!
//! * recent input is strong ONLINE evidence; five idle minutes is only a
//!   candidate, never a verdict;
//! * actively *advancing* playback is positive but not absolute evidence — a
//!   paused or stalled player is worth nothing, and a stopped one ages out;
//! * a locked screen is the strongest AWAY evidence and can never be
//!   outvoted by media;
//! * every downgrade is confirmed before it commits (hysteresis), while the
//!   ride back to ONLINE is fast once real evidence reappears;
//! * a partner's absence becomes OFFLINE only after a tolerated window of
//!   *healthy* expired answers — transport failures say nothing about them.
//!
//! The heuristic is deliberately simple. Its goal is natural behavior: a
//! person watching a film stays ONLINE, a person who left is eventually
//! AWAY, and nobody flaps.

use serde::{Deserialize, Serialize};

/// The single idle boundary: below it, idle age is fresh-input ONLINE
/// evidence; at or beyond it, the machine becomes an AWAY *candidate*.
/// One constant, two sides — overlapping comparisons here once made the
/// boundary itself flip-flop.
pub const AWAY_CANDIDATE_IDLE_SECS: u64 = 300;
/// An AWAY candidate must stay continuously plausible this long to commit.
pub const AWAY_CONFIRM_SECS: u64 = 45;
/// A lock commits AWAY after this brief guard (a lock flash is not a verdict).
pub const LOCK_CONFIRM_SECS: u64 = 10;
/// Minimum dwell between committed transitions, in both directions.
pub const MIN_STATE_DWELL_SECS: u64 = 20;
/// The presence lease is 45 s on the control plane; refresh well ahead of it.
pub const PUBLISH_REFRESH_SECS: u64 = 15;
/// Healthy "no lease" answers must persist this long before a partner reads
/// OFFLINE. Covers their short reconnects without outliving the lease much.
pub const PARTNER_OFFLINE_CONFIRM_SECS: u64 = 75;

/// The three-state product vocabulary. The control-plane lease speaks
/// ONLINE/IDLE/OFFLINE; AWAY maps onto IDLE at the publish/poll edge only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum PresenceState {
    Online,
    Away,
    Offline,
}

impl PresenceState {
    /// The control-plane lease value this state publishes as.
    pub fn lease_value(self) -> &'static str {
        match self {
            PresenceState::Online => "ONLINE",
            PresenceState::Away => "IDLE",
            PresenceState::Offline => "OFFLINE",
        }
    }
}

/// What the platform detector last saw the media layer doing. The detector
/// reports `Playing` only while playback is *advancing*; a player claiming
/// Playing with a frozen position arrives as `Paused` — "playing" is never
/// taken on faith.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MediaSignal {
    Playing,
    Paused,
    Stopped,
}

/// Private multi-signal observation. Every field is optional: an unavailable
/// platform API stays `None` and contributes nothing — state is never
/// invented. This type never leaves the machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UserActivitySnapshot {
    #[serde(default)]
    pub input_idle_seconds: Option<u64>,
    #[serde(default)]
    pub screen_locked: Option<bool>,
    #[serde(default)]
    pub session_active: Option<bool>,
    #[serde(default)]
    pub media: Option<MediaSignal>,
}

impl UserActivitySnapshot {
    /// True when the snapshot carries no usable signal at all.
    fn is_empty(&self) -> bool {
        self.input_idle_seconds.is_none()
            && self.screen_locked.is_none()
            && self.session_active.is_none()
            && self.media.is_none()
    }
}

/// The locally committed presence, guarded by hysteresis.
///
/// Downgrades are candidates first and commit only after they stay
/// continuously plausible; upgrades (back to ONLINE) commit on real evidence
/// the moment the minimum dwell allows. The initial state is OFFLINE: until
/// evidence says otherwise, nobody can claim this machine is attended.
#[derive(Debug)]
pub struct LocalPresence {
    state: PresenceState,
    /// The state this machine committed away from, kept so events can say
    /// what changed. `None` only before the first commit.
    previous: Option<PresenceState>,
    candidate: Option<(PresenceState, u64)>,
    last_transition: Option<u64>,
    revision: u64,
}

impl Default for LocalPresence {
    fn default() -> Self {
        Self {
            state: PresenceState::Offline,
            previous: None,
            candidate: None,
            last_transition: None,
            revision: 0,
        }
    }
}

impl LocalPresence {
    pub fn state(&self) -> PresenceState {
        self.state
    }

    /// The state before the most recent commit, when there was one.
    pub fn previous_state(&self) -> Option<PresenceState> {
        self.previous
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Absorbs one snapshot. Returns true when the committed state changed.
    pub fn observe(&mut self, snapshot: &UserActivitySnapshot, now: u64) -> bool {
        if snapshot.is_empty() {
            return false;
        }
        let desired = desired_state(snapshot);
        let Some(desired) = desired else {
            return false;
        };
        if desired == self.state {
            self.candidate = None;
            return false;
        }

        // A candidate that switched identity restarts its confirmation clock:
        // confirmation is continuity, not accumulated interest. A newborn
        // candidate starts its clock at this very observation.
        let since = match self.candidate {
            Some((pending, since)) if pending == desired => since,
            _ => {
                self.candidate = Some((desired, now));
                now
            }
        };
        // The minimum dwell protects against flapping in both directions,
        // but a false AWAY is the forbidden error: strong ONLINE evidence
        // (fresh input, advancing media) commits without waiting it out.
        let confirmed = now.saturating_sub(since) >= confirmation_for(desired, snapshot)
            && (desired == PresenceState::Online || self.dwell_elapsed(now));
        if confirmed {
            self.commit(desired, now);
            return true;
        }
        false
    }

    /// The core is going away: publish an honest OFFLINE regardless of the
    /// evidence that was last seen. Returns true when this was a change.
    pub fn mark_offline(&mut self, now: u64) -> bool {
        if self.state == PresenceState::Offline {
            return false;
        }
        self.commit(PresenceState::Offline, now);
        true
    }

    fn dwell_elapsed(&self, now: u64) -> bool {
        match self.last_transition {
            Some(at) => now.saturating_sub(at) >= MIN_STATE_DWELL_SECS,
            None => true,
        }
    }

    fn commit(&mut self, state: PresenceState, now: u64) {
        self.previous = Some(std::mem::replace(&mut self.state, state));
        self.candidate = None;
        self.last_transition = Some(now);
        self.revision = self.revision.saturating_add(1);
    }
}

/// What the evidence says, or `None` when it says nothing usable.
fn desired_state(snapshot: &UserActivitySnapshot) -> Option<PresenceState> {
    let locked = snapshot.screen_locked == Some(true);
    let session_gone = snapshot.session_active == Some(false);
    let media_playing = snapshot.media == Some(MediaSignal::Playing);
    let fresh_input = matches!(snapshot.input_idle_seconds, Some(idle) if idle < AWAY_CANDIDATE_IDLE_SECS);

    // A locked (or vanished) session is the strongest evidence there is:
    // nothing at this machine is being consumed, whatever the player claims.
    if locked || session_gone {
        return Some(PresenceState::Away);
    }
    if fresh_input || media_playing {
        return Some(PresenceState::Online);
    }
    // Paused or stopped media and idle input: absence of evidence. The idle
    // age drives the (slow) AWAY candidate once it crosses the threshold.
    match snapshot.input_idle_seconds {
        Some(idle) if idle >= AWAY_CANDIDATE_IDLE_SECS => Some(PresenceState::Away),
        Some(_) => Some(PresenceState::Online),
        None => {
            // Without an idle measurement the session itself is the only
            // positive signal: an active session is weak ONLINE evidence.
            // Conservatism lives here — a machine we cannot prove idle stays
            // visibly present rather than silently vanishing.
            if media_playing || snapshot.session_active == Some(true) {
                Some(PresenceState::Online)
            } else {
                None
            }
        }
    }
}

/// How long a desired state must persist before it commits. The clock runs
/// on the evidence, not just the destination: a lock (or a vanished
/// session) is authoritative enough to confirm quickly, while idle- and
/// media-shaped doubts must survive the full window.
fn confirmation_for(state: PresenceState, snapshot: &UserActivitySnapshot) -> u64 {
    match state {
        PresenceState::Online => 0,
        PresenceState::Away | PresenceState::Offline => {
            let authoritative = snapshot.screen_locked == Some(true)
                || snapshot.session_active == Some(false);
            if authoritative {
                LOCK_CONFIRM_SECS
            } else {
                AWAY_CONFIRM_SECS
            }
        }
    }
}

/// The partner's presence, as observed through healthy control-plane answers.
///
/// The first observation is a silent baseline: history that predates the
/// watch is never announced. Arrivals commit immediately; absence needs a
/// tolerated streak before it reads OFFLINE, and a dead transport says
/// nothing at all.
#[derive(Debug, Default)]
pub struct PartnerPresence {
    state: Option<PresenceState>,
    /// The state this machine committed away from, for event payloads.
    previous: Option<PresenceState>,
    offline_since: Option<u64>,
    revision: u64,
}

impl PartnerPresence {
    pub fn state(&self) -> Option<PresenceState> {
        self.state
    }

    /// The state before the most recent commit, when there was one.
    pub fn previous_state(&self) -> Option<PresenceState> {
        self.previous
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Absorbs one healthy poll answer. Returns true when the observed state
    /// changed (never on the very first observation).
    pub fn observe(&mut self, reported: PresenceState, now: u64) -> bool {
        let Some(previous) = self.state else {
            // Baseline: remember, do not announce.
            self.state = Some(reported);
            self.offline_since = None;
            return false;
        };
        if reported == PresenceState::Offline {
            let since = *self.offline_since.get_or_insert(now);
            if previous == PresenceState::Offline {
                return false;
            }
            if now.saturating_sub(since) >= PARTNER_OFFLINE_CONFIRM_SECS {
                return self.commit(PresenceState::Offline, now);
            }
            return false;
        }
        self.offline_since = None;
        if reported == previous {
            return false;
        }
        self.commit(reported, now)
    }

    fn commit(&mut self, state: PresenceState, _now: u64) -> bool {
        self.previous = Some(self.state.replace(state).expect("commit only follows a baseline"));
        self.offline_since = None;
        self.revision = self.revision.saturating_add(1);
        true
    }
}

/// Owns both machines and the publication cadence for the local state.
#[derive(Debug, Default)]
pub struct PresenceTracker {
    pub local: LocalPresence,
    pub partner: PartnerPresence,
    last_publish: Option<u64>,
    unpublished_change: bool,
    last_emitted_local: Option<PresenceState>,
    last_emitted_partner: Option<PresenceState>,
}

impl PresenceTracker {
    /// Whether a lease refresh is due: right after any committed change, or
    /// periodically while connected so the 45 s lease never silently expires.
    pub fn publish_due(&self, now: u64) -> bool {
        if self.unpublished_change {
            return true;
        }
        match self.last_publish {
            Some(at) => now.saturating_sub(at) >= PUBLISH_REFRESH_SECS,
            None => true,
        }
    }

    /// The sense path's entry point: absorbs a snapshot and, when the
    /// committed state changed, marks the lease publish as owed right away.
    pub fn observe_local(&mut self, snapshot: &UserActivitySnapshot, now: u64) -> bool {
        let changed = self.local.observe(snapshot, now);
        self.unpublished_change = self.unpublished_change || changed;
        changed
    }

    /// Records that the current local state was just published.
    pub fn mark_published(&mut self, now: u64) {
        self.last_publish = Some(now);
        self.unpublished_change = false;
    }

    /// The state that should be published right now.
    pub fn publishable_state(&self) -> PresenceState {
        self.local.state()
    }

    /// True while a fresh local transition still awaits its first publish.
    pub fn take_pending_publish(&mut self) -> bool {
        std::mem::replace(&mut self.unpublished_change, false)
    }

    /// The local state as last described to the UI, if an event went out.
    pub fn last_emitted_local(&self) -> Option<PresenceState> {
        self.last_emitted_local
    }

    /// The partner state as last described to the UI, if an event went out.
    pub fn last_emitted_partner(&self) -> Option<PresenceState> {
        self.last_emitted_partner
    }

    /// Records that an event just described both current states.
    pub fn mark_emitted(&mut self) {
        self.last_emitted_local = Some(self.local.state());
        self.last_emitted_partner = self.partner.state();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(idle: Option<u64>, locked: Option<bool>, media: Option<MediaSignal>) -> UserActivitySnapshot {
        UserActivitySnapshot {
            input_idle_seconds: idle,
            screen_locked: locked,
            session_active: Some(true),
            media,
        }
    }

    fn online_from(machine: &mut LocalPresence, now: &mut u64) {
        // First evidence commits OFFLINE -> ONLINE immediately.
        assert!(machine.observe(&snapshot(Some(0), None, None), *now));
        assert_eq!(machine.state(), PresenceState::Online);
        *now += 1;
    }

    #[test]
    fn first_real_evidence_moves_offline_to_online_immediately() {
        let mut machine = LocalPresence::default();
        let mut now = 1_000_u64;
        online_from(&mut machine, &mut now);
        assert_eq!(machine.revision(), 1);
    }

    #[test]
    fn an_empty_snapshot_invents_nothing() {
        let mut machine = LocalPresence::default();
        assert!(!machine.observe(&UserActivitySnapshot::default(), 1_000));
        assert_eq!(machine.state(), PresenceState::Offline);
    }

    #[test]
    fn an_active_session_without_idle_measurement_keeps_presence_honest() {
        let mut machine = LocalPresence::default();
        // Only logind answered: the session exists, so this machine is not
        // offline — but nothing here can ever prove AWAY either.
        let mut logins_only = UserActivitySnapshot::default();
        logins_only.session_active = Some(true);
        assert!(machine.observe(&logins_only, 1_000));
        assert_eq!(machine.state(), PresenceState::Online);
        let mut now = 1_001_u64 + 60 * 60;
        assert!(!machine.observe(&logins_only, now));
        assert_eq!(machine.state(), PresenceState::Online);
        // A vanished session still ends it — confirmed, not instant.
        now += 1;
        let mut gone = logins_only;
        gone.session_active = Some(false);
        assert!(!machine.observe(&gone, now));
        now += AWAY_CONFIRM_SECS;
        assert!(machine.observe(&gone, now));
        assert_eq!(machine.state(), PresenceState::Away);
    }

    #[test]
    fn five_idle_minutes_are_a_candidate_that_confirms_late() {
        let mut machine = LocalPresence::default();
        let mut now = 1_000_u64;
        online_from(&mut machine, &mut now);
        // Idle reaches the threshold: candidate, not a verdict.
        now += AWAY_CANDIDATE_IDLE_SECS;
        assert!(!machine.observe(&snapshot(Some(AWAY_CANDIDATE_IDLE_SECS), None, None), now));
        assert_eq!(machine.state(), PresenceState::Online);
        // The candidate survives unconfirmed and then commits.
        now += AWAY_CONFIRM_SECS;
        assert!(machine.observe(&snapshot(Some(AWAY_CANDIDATE_IDLE_SECS + AWAY_CONFIRM_SECS), None, None), now));
        assert_eq!(machine.state(), PresenceState::Away);
    }

    #[test]
    fn a_candidate_dies_when_fresh_input_returns_before_committing() {
        let mut machine = LocalPresence::default();
        let mut now = 1_000_u64;
        online_from(&mut machine, &mut now);
        now += AWAY_CANDIDATE_IDLE_SECS + 5;
        assert!(!machine.observe(&snapshot(Some(AWAY_CANDIDATE_IDLE_SECS + 5), None, None), now));
        // One mouse move during the confirmation window: no AWAY ever happens.
        now += AWAY_CONFIRM_SECS - 10;
        assert!(!machine.observe(&snapshot(Some(2), None, None), now));
        now += 60;
        assert!(!machine.observe(&snapshot(Some(62), None, None), now));
        assert_eq!(machine.state(), PresenceState::Online);
    }

    #[test]
    fn returning_evidence_commits_online_quickly() {
        let mut machine = LocalPresence::default();
        let mut now = 1_000_u64;
        online_from(&mut machine, &mut now);
        let away_age = AWAY_CANDIDATE_IDLE_SECS + AWAY_CONFIRM_SECS + MIN_STATE_DWELL_SECS;
        now += away_age;
        assert!(!machine.observe(&snapshot(Some(away_age), None, None), now));
        now += AWAY_CONFIRM_SECS;
        assert!(machine.observe(&snapshot(Some(away_age + AWAY_CONFIRM_SECS), None, None), now));
        assert_eq!(machine.state(), PresenceState::Away);
        // The user is back: one input event restores ONLINE on the spot —
        // the dwell never manufactures a false AWAY.
        now += 5;
        assert!(machine.observe(&snapshot(Some(0), None, None), now));
        assert_eq!(machine.state(), PresenceState::Online);
    }

    #[test]
    fn a_film_keeps_the_viewer_online_through_a_long_idle() {
        let mut machine = LocalPresence::default();
        let mut now = 1_000_u64;
        online_from(&mut machine, &mut now);
        // Forty minutes without input while a film plays.
        let forty_minutes = 40 * 60;
        now += forty_minutes;
        assert!(!machine.observe(&snapshot(Some(forty_minutes), None, Some(MediaSignal::Playing)), now));
        assert_eq!(machine.state(), PresenceState::Online);
    }

    #[test]
    fn a_paused_film_is_worth_nothing_and_idle_takes_over() {
        let mut machine = LocalPresence::default();
        let mut now = 1_000_u64;
        online_from(&mut machine, &mut now);
        now += AWAY_CANDIDATE_IDLE_SECS + 10;
        assert!(!machine.observe(&snapshot(Some(AWAY_CANDIDATE_IDLE_SECS + 10), None, Some(MediaSignal::Paused)), now));
        now += AWAY_CONFIRM_SECS;
        assert!(machine.observe(&snapshot(Some(AWAY_CANDIDATE_IDLE_SECS + 10 + AWAY_CONFIRM_SECS), None, Some(MediaSignal::Paused)), now));
        assert_eq!(machine.state(), PresenceState::Away);
    }

    #[test]
    fn a_stopped_film_lets_the_idle_candidate_proceed() {
        let mut machine = LocalPresence::default();
        let mut now = 1_000_u64;
        online_from(&mut machine, &mut now);
        now += AWAY_CANDIDATE_IDLE_SECS;
        assert!(!machine.observe(&snapshot(Some(AWAY_CANDIDATE_IDLE_SECS), None, Some(MediaSignal::Stopped)), now));
        now += AWAY_CONFIRM_SECS;
        assert!(machine.observe(&snapshot(Some(AWAY_CANDIDATE_IDLE_SECS + AWAY_CONFIRM_SECS), None, Some(MediaSignal::Stopped)), now));
        assert_eq!(machine.state(), PresenceState::Away);
    }

    #[test]
    fn a_lock_commits_away_quickly_and_outvotes_media() {
        let mut machine = LocalPresence::default();
        let mut now = 1_000_u64;
        online_from(&mut machine, &mut now);
        // Locked with a film running is still AWAY — after a short guard
        // that a lock flash cannot survive.
        now += MIN_STATE_DWELL_SECS + 5;
        assert!(!machine.observe(&snapshot(Some(5), Some(true), Some(MediaSignal::Playing)), now));
        now += LOCK_CONFIRM_SECS;
        assert!(machine.observe(&snapshot(Some(5 + LOCK_CONFIRM_SECS), Some(true), Some(MediaSignal::Playing)), now));
        assert_eq!(machine.state(), PresenceState::Away);
    }

    #[test]
    fn an_unlock_with_input_restores_online() {
        let mut machine = LocalPresence::default();
        let mut now = 1_000_u64;
        online_from(&mut machine, &mut now);
        now += MIN_STATE_DWELL_SECS + LOCK_CONFIRM_SECS;
        // The lock itself confirms after its guard.
        assert!(!machine.observe(&snapshot(Some(60), Some(true), None), now));
        now += LOCK_CONFIRM_SECS;
        assert!(machine.observe(&snapshot(Some(60 + LOCK_CONFIRM_SECS), Some(true), None), now));
        assert_eq!(machine.state(), PresenceState::Away);
        // Unlock plus a fresh input event returns ONLINE instantly.
        now += 3;
        assert!(machine.observe(&snapshot(Some(0), Some(false), None), now));
        assert_eq!(machine.state(), PresenceState::Online);
    }

    #[test]
    fn transitions_never_flap_inside_the_minimum_dwell() {
        let mut machine = LocalPresence::default();
        let mut now = 1_000_u64;
        online_from(&mut machine, &mut now);
        // A lock lands inside the dwell window: it may become a candidate,
        // but it cannot commit until the dwell has elapsed.
        now += 2;
        assert!(!machine.observe(&snapshot(Some(2), Some(true), None), now));
        now += LOCK_CONFIRM_SECS; // past the lock guard, still inside the dwell
        assert!(!machine.observe(&snapshot(Some(2 + LOCK_CONFIRM_SECS), Some(true), None), now));
        assert_eq!(machine.state(), PresenceState::Online);
        now += MIN_STATE_DWELL_SECS;
        assert!(machine.observe(&snapshot(Some(2 + LOCK_CONFIRM_SECS + MIN_STATE_DWELL_SECS), Some(true), None), now));
        assert_eq!(machine.state(), PresenceState::Away);
    }

    #[test]
    fn shutdown_commits_an_honest_offline() {
        let mut machine = LocalPresence::default();
        let mut now = 1_000_u64;
        online_from(&mut machine, &mut now);
        now += 5;
        assert!(machine.mark_offline(now));
        assert_eq!(machine.state(), PresenceState::Offline);
        // Repeating the marker changes nothing.
        now += 5;
        assert!(!machine.mark_offline(now));
        assert_eq!(machine.revision(), 2);
    }

    #[test]
    fn the_partner_baseline_is_silent() {
        let mut partner = PartnerPresence::default();
        assert!(!partner.observe(PresenceState::Online, 1_000));
        assert_eq!(partner.state(), Some(PresenceState::Online));
    }

    #[test]
    fn partner_arrivals_commit_immediately() {
        let mut partner = PartnerPresence::default();
        assert!(!partner.observe(PresenceState::Offline, 1_000));
        assert!(partner.observe(PresenceState::Online, 1_100));
        assert!(partner.observe(PresenceState::Away, 1_200));
        assert_eq!(partner.state(), Some(PresenceState::Away));
    }

    #[test]
    fn partner_offline_needs_a_streak_of_healthy_answers() {
        let mut partner = PartnerPresence::default();
        assert!(!partner.observe(PresenceState::Online, 1_000));
        // A single expired answer changes nothing.
        assert!(!partner.observe(PresenceState::Offline, 1_010));
        assert_eq!(partner.state(), Some(PresenceState::Online));
        // Short reconnect gaps do not either.
        assert!(!partner.observe(PresenceState::Online, 1_030));
        // A full tolerated streak does.
        assert!(!partner.observe(PresenceState::Offline, 1_040));
        assert!(!partner.observe(PresenceState::Offline, 1_040 + PARTNER_OFFLINE_CONFIRM_SECS - 1));
        assert!(partner.observe(PresenceState::Offline, 1_040 + PARTNER_OFFLINE_CONFIRM_SECS));
        assert_eq!(partner.state(), Some(PresenceState::Offline));
    }

    #[test]
    fn partner_state_repeats_never_renotify() {
        let mut partner = PartnerPresence::default();
        assert!(!partner.observe(PresenceState::Online, 1_000));
        assert!(!partner.observe(PresenceState::Online, 2_000));
        assert!(!partner.observe(PresenceState::Online, 3_000));
        assert_eq!(partner.revision(), 0);
    }

    #[test]
    fn emission_memory_distinguishes_changed_sides() {
        let mut tracker = PresenceTracker::default();
        assert!(tracker.local.observe(&snapshot(Some(0), None, None), 1_000));
        assert_eq!(tracker.last_emitted_local(), None);
        tracker.mark_emitted();
        assert_eq!(tracker.last_emitted_local(), Some(PresenceState::Online));
        assert_eq!(tracker.last_emitted_partner(), None);
    }

    #[test]
    fn the_tracker_publishes_on_change_and_refreshes_periodically() {
        let mut tracker = PresenceTracker::default();
        let mut now = 1_000_u64;
        assert!(tracker.publish_due(now));
        tracker.mark_published(now);
        // Quiet: nothing due until the refresh window elapses.
        now += PUBLISH_REFRESH_SECS - 1;
        assert!(!tracker.publish_due(now));
        now += 1;
        assert!(tracker.publish_due(now));
        tracker.mark_published(now);
        // A fresh transition is due immediately.
        now += 3;
        assert!(tracker.observe_local(&snapshot(Some(0), None, None), now));
        assert_eq!(tracker.publishable_state(), PresenceState::Online);
        assert!(tracker.publish_due(now));
        tracker.mark_published(now);
        assert!(!tracker.publish_due(now));
    }

    #[test]
    fn away_maps_to_the_lease_vocabulary() {
        assert_eq!(PresenceState::Online.lease_value(), "ONLINE");
        assert_eq!(PresenceState::Away.lease_value(), "IDLE");
        assert_eq!(PresenceState::Offline.lease_value(), "OFFLINE");
    }

    #[test]
    fn the_snapshot_round_trips_through_camel_case_json() {
        let raw = serde_json::json!({
            "inputIdleSeconds": 42,
            "screenLocked": false,
            "sessionActive": true,
            "media": "playing"
        });
        let parsed: UserActivitySnapshot = serde_json::from_value(raw).expect("valid snapshot");
        assert_eq!(parsed.input_idle_seconds, Some(42));
        assert_eq!(parsed.screen_locked, Some(false));
        assert_eq!(parsed.session_active, Some(true));
        assert_eq!(parsed.media, Some(MediaSignal::Playing));
    }
}
