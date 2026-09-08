pragma ComponentBehavior: Bound
import QtQml

// Production network-diagnostics provider between the supervised Rust core
// and the NetworkView surface. The core opens a real pinned connection to
// the configured control plane and signs one probe exchange; this bridge
// only mirrors its measured, sanitized result into AppState. An absent
// result stays absent — the views render — rather than any number.
//
// The facade is a C++ context property, so this glue file is deliberately
// dynamically typed; qmllint cannot know its members.
// qmllint disable missing-property
QtObject {
    id: provider

    property QtObject facade: null
    readonly property bool live: facade !== null && facade.coreReady
    // Tracks mirrored state so a later core fault clears it instead of
    // leaving a stale measurement on screen.
    property bool hadLive: false

    function run() {
        if (!live)
            return
        facade.runNetworkDiagnostics()
    }

    function _sync() {
        var current = facade
        if (current === null || !current.coreReady) {
            if (hadLive) {
                hadLive = false
                AppState.networkDiagnostics = null
                AppState.networkDiagnosticsRunning = false
            }
            return
        }
        hadLive = true
        AppState.networkDiagnosticsRunning = Boolean(current.networkDiagnosticsRunning)
        var result = current.networkDiagnostics
        AppState.networkDiagnostics =
                result && typeof result === "object"
                ? Object.assign({}, result) : null
    }

    Component.onCompleted: _sync()
    onFacadeChanged: _sync()

    readonly property list<QtObject> wiring: [
        Connections {
            target: provider.facade

            function onNetworkDiagnosticsChanged() {
                provider._sync()
            }

            function onCoreReadyChanged() {
                provider._sync()
            }
        }
    ]
}
