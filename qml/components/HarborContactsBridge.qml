pragma ComponentBehavior: Bound
import QtQml

// Mirrors the facade's durable people-and-machines facts into AppState: the
// paired-peer snapshot from the control plane, and the real devices page it
// implies (this machine, named by the operating system, plus the paired
// peer's real Harbor id). The control plane owns the relationship; this
// bridge only relays the safe {deviceId, harborId} pairs. Inert without a
// facade, so the deterministic mock provider stays the single QML truth in
// tests.
// qmllint disable missing-property
QtObject {
    id: provider

    property QtObject facade: null
    readonly property bool live: facade !== null && facade.coreReady
    property bool hadLive: false

    function _sync() {
        var current = facade
        if (current === null || !current.coreReady) {
            if (hadLive) {
                hadLive = false
                AppState.setPairedPeers([])
                AppState.deviceName = ""
                // Core down: no device truth to show. The empty state with
                // its pairing action is the honest page.
                AppState.setDevices([])
                AppState.setConnection("disconnected")
            }
            return
        }
        hadLive = true
        // The core runs, so this Harbor is online. Peer presence stays a
        // separate fact — partnerState only moves when real presence says so.
        AppState.setConnection("connected")
        // Older test facades expose the pairedPeers snapshot without the
        // readiness flag; the real facade marks it authoritative before its
        // pairedPeersChanged signal is emitted.
        if (typeof current.pairedPeersResolved === "undefined"
                || current.pairedPeersResolved)
            AppState.setPairedPeers(current.pairedPeers)
        AppState.deviceName = String(current.deviceName || "")
        _composeDevices()
    }

    // The live devices page: this machine plus at most the one paired peer.
    // Names are real facts (host name, durable Harbor id) — never the
    // fixture persona. Presence has no live feed yet, so the peer's
    // connection state mirrors the session's last knowledge instead of a
    // fabricated "online".
    function _composeDevices() {
        var entries = [{
            id: "self",
            name: AppState.deviceName.length > 0 ? AppState.deviceName : "Harbor",
            typeKey: "devices.type.thisDevice",
            iconName: "monitor",
            statusKey: "devices.status.connectedNow",
            statusParams: {},
            connected: true,
            primary: true,
            manageable: false
        }]
        if (AppState.pairedPeers.length > 0) {
            var peer = AppState.pairedPeers[0]
            entries.push({
                id: String(peer.deviceId),
                name: String(peer.harborId),
                typeKey: "devices.type.computer",
                iconName: "laptop",
                statusKey: AppState.partnerState === "online"
                           ? "devices.status.connectedNow"
                           : "devices.status.offline",
                statusParams: {},
                connected: AppState.partnerState === "online",
                primary: false,
                manageable: false
            })
        }
        AppState.setDevices(entries)
    }

    Component.onCompleted: _sync()
    onFacadeChanged: _sync()

    readonly property list<QtObject> wiring: [
        Connections {
            target: provider.facade
            function onPairedPeersChanged() { provider._sync() }
            function onPairedPeersResolvedChanged() { provider._sync() }
            function onCoreReadyChanged() { provider._sync() }
        },
        Connections {
            target: AppState
            function onPartnerStateChanged() {
                // Same stale-binding hazard as _sync: inside this cascade
                // `live` may still hold the pre-teardown value, so read the
                // facade itself.
                var current = facade
                if (current !== null && current.coreReady)
                    provider._composeDevices()
            }
        }
    ]
}
