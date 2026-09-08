pragma ComponentBehavior: Bound
import QtQml

// Production activity provider between the supervised Rust core and the
// ActivityView. The core owns the real engine (monitor, timeline, policies);
// this bridge mirrors its snapshots into AppState so the view keeps reading
// the same observable store the deterministic test provider writes.
//
// The core's snapshot carries only record material — {id, category, titleKey,
// titleParams, descriptionKey, descriptionParams, occurredAt}. Raw local
// material (pid, executable path, command line) never crosses the core
// boundary. The display name is a UI concern, so the bridge injects it here;
// times are local "HH:MM" strings resolved at render data time, keeping the
// fact/presentation split: the core emits facts, the UI presents them.
//
// The facade is a C++ context property, so this glue file is deliberately
// dynamically typed; qmllint cannot know its members. QtObject has no default
// property, so connections live in an explicit list.
// qmllint disable missing-property
QtObject {
    id: provider

    property QtObject facade: null
    readonly property bool live: facade !== null && facade.coreReady
    // Tracks a real snapshot having been mirrored so a later core fault clears
    // it instead of leaving stale local activity visible.
    property bool hadLiveFacade: false

    // Week totals come straight from the core's rolling-week window; the
    // view keeps its own fixtures for the deterministic test provider.
    readonly property var weekStats: {
        if (facade === null || !facade.coreReady)
            return []
        var stats = facade.activityStats
        return [
            { value: _formatCount(stats.games), labelKey: "common.labels.games" },
            { value: _formatCount(stats.apps), labelKey: "common.labels.applications" },
            { value: _formatHours(stats.hours), labelKey: "common.labels.hours" }
        ]
    }

    function refresh() {
        if (live)
            facade.refreshActivity()
    }

    // ---- Mapping ----------------------------------------------------------

    function _applySnapshot() {
        if (facade === null || !facade.coreReady)
            return
        hadLiveFacade = true
        var timeline = facade.activityTimeline
        var activities = []
        for (var i = 0; i < timeline.length; i++)
            activities.push(_convertEntry(timeline[i]))
        // A fresh array every time: mutating entries in place would not
        // refresh bindings that observe the collection by replacement.
        AppState.activities = activities

        var remote = []
        var remoteTimeline = facade.remoteActivity || []
        for (var remoteIndex = 0; remoteIndex < remoteTimeline.length; remoteIndex++)
            remote.push(_convertRemoteEntry(remoteTimeline[remoteIndex]))
        AppState.setRemoteActivities(remote)
    }

    function _clearSnapshot() {
        if (!hadLiveFacade)
            return
        AppState.activities = []
        AppState.setRemoteActivities([])
        hadLiveFacade = false
    }

    function _convertRemoteEntry(entry) {
        entry = entry || {}
        return {
            id: String(entry.id || ""),
            sender: String(entry.sender || AppState.partnerName),
            category: String(entry.category || "system"),
            kind: String(entry.kind || "opened"),
            // `label` has already passed the core's strict remote schema;
            // it is plain text, never a path, command, or markup.
            // `appId`/`iconKey` are theme-safe keys for real program icons.
            label: String(entry.label || ""),
            appId: String(entry.appId || entry.app_id || ""),
            iconKey: String(entry.iconKey || entry.icon || ""),
            time: _formatTime(entry.occurredAt)
        }
    }

    function _convertEntry(entry) {
        entry = entry || {}
        var titleParams = _copyParams(entry.titleParams)
        // Local records belong to this device and are never presented as the
        // partner's moments. Keep their subject neutral at this boundary.
        titleParams.name = AppState.selfName
        return {
            id: String(entry.id || ""),
            category: String(entry.category || "system"),
            // Stable app identity + theme-safe icon key for real program
            // icons. Empty means "unknown": the view falls back to the
            // category glyph. Never a path, pid, or command line.
            appId: String(entry.appId || entry.app_id || ""),
            iconKey: String(entry.iconKey || entry.icon || ""),
            titleKey: String(entry.titleKey || ""),
            titleParams: titleParams,
            descriptionKey: String(entry.descriptionKey || ""),
            descriptionParams: _copyParams(entry.descriptionParams),
            time: _formatTime(entry.occurredAt)
        }
    }

    function _copyParams(params) {
        var copy = {}
        for (var key in params || {})
            copy[key] = params[key]
        return copy
    }

    function _formatCount(value) {
        var count = Math.floor(Number(value))
        return String(isFinite(count) && count > 0 ? count : 0)
    }

    function _formatHours(value) {
        var hours = Number(value)
        if (!isFinite(hours) || hours < 0)
            hours = 0
        var rounded = Math.round(hours * 10) / 10
        if (rounded === Math.floor(rounded))
            return String(Math.floor(rounded))
        return rounded.toFixed(1)
    }

    function _formatTime(secondsSinceEpoch) {
        var date = new Date((Number(secondsSinceEpoch) || 0) * 1000)
        var hours = date.getHours()
        var minutes = date.getMinutes()
        return (hours < 10 ? "0" : "") + hours + ":" + (minutes < 10 ? "0" : "") + minutes
    }

    // A facade appearing (the view binds it once the core is ready) may come
    // with snapshots already published; creation itself covers the rest.
    Component.onCompleted: _applySnapshot()
    onFacadeChanged: {
        if (facade === null)
            _clearSnapshot()
        else
            _applySnapshot()
    }

    readonly property list<QtObject> wiring: [
        Connections {
            target: provider.facade

            function onActivityChanged() {
                provider._applySnapshot()
            }

            // A facade appearing or the core becoming ready is followed by
            // the facade's own fetch; activityChanged carries the snapshot,
            // but a ready flip may also arrive with state already present.
            function onCoreReadyChanged() {
                if (provider.facade !== null && provider.facade.coreReady)
                    provider._applySnapshot()
                else
                    provider._clearSnapshot()
            }
        }
    ]
}
