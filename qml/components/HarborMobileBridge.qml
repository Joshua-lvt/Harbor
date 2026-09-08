pragma ComponentBehavior: Bound
import QtQml

// Production phone provider between the supervised Rust core and the
// MobileView. The core owns the sharing truth (the peer's MobileStatus and
// the display-only notification events); this bridge mirrors them into the
// transient AppState slots so the view keeps reading the same observable
// store the deterministic test fixtures write.
//
// Nothing here is durable: a side that stops sharing returns its snapshot
// to null, notices are a bounded display-only FIFO, and losing the facade
// clears both instead of leaving a stale battery or location on screen.
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
    // it instead of leaving stale phone facts visible.
    property bool hadLiveFacade: false

    function refresh() {
        if (live)
            facade.refreshMobile()
    }

    // ---- Mapping ----------------------------------------------------------

    function _applySnapshot() {
        if (facade === null || !facade.coreReady)
            return
        hadLiveFacade = true
        var state = facade.mobileState || {}
        AppState.setPeerPhone(_convertStatus(state.peer))
    }

    function _convertStatus(status) {
        if (!status || typeof status !== "object")
            return null
        var battery = Math.floor(Number(status.batteryPercent))
        var location = status.location && typeof status.location === "object"
            ? status.location : null
        return {
            batteryPercent: (isFinite(battery) && battery >= 0 && battery <= 100) ? battery : null,
            charging: status.charging === true,
            phoneActivity: _normalizeActivity(status.phoneActivity),
            lastActiveAt: Number(status.lastActiveAt) || 0,
            currentApp: String(status.currentApp || ""),
            locationSharingEnabled: status.locationSharingEnabled === true,
            location: _convertFix(location),
            notificationSharingEnabled: status.notificationSharingEnabled === true,
            deviceType: String(status.deviceType || "")
        }
    }

    function _normalizeActivity(value) {
        var activity = String(value || "OFFLINE").toUpperCase()
        return activity === "ACTIVE" || activity === "IDLE" ? activity : "OFFLINE"
    }

    function _convertFix(fix) {
        if (!fix)
            return null
        var latitude = Number(fix.latitude)
        var longitude = Number(fix.longitude)
        var accuracy = Number(fix.accuracyMeters)
        if (!isFinite(latitude) || !isFinite(longitude) || !isFinite(accuracy)
                || accuracy < 0 || Math.abs(latitude) > 90 || Math.abs(longitude) > 180)
            return null
        return {
            latitude: latitude,
            longitude: longitude,
            accuracyMeters: accuracy,
            updatedAt: Number(fix.updatedAt) || 0
        }
    }

    function _pushNotice(payload) {
        var notice = payload || {}
        var stamp = Number(notice.timestamp) || 0
        // Android posts milliseconds; the core accepts any nonzero stamp.
        if (stamp > 100000000000)
            stamp = Math.floor(stamp / 1000)
        AppState.pushPhoneNotice({
            appLabel: String(notice.appLabel || ""),
            title: String(notice.title || ""),
            text: String(notice.text || ""),
            at: stamp
        })
    }

    function _clearSnapshot() {
        if (!hadLiveFacade)
            return
        AppState.setPeerPhone(null)
        AppState.clearPhoneNotices()
        hadLiveFacade = false
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

            function onMobileChanged() {
                provider._applySnapshot()
            }

            function onPhoneNotification(payload) {
                if (provider.live)
                    provider._pushNotice(payload)
            }

            // A facade appearing or the core becoming ready is followed by
            // the facade's own fetch; mobileChanged carries the snapshot,
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
