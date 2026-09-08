import QtQuick
import QtTest
import Harbor 2.0

// Contract tests for the network-diagnostics bridge. The facade is stubbed
// with the exact property surface the real C++ facade exposes, so these
// tests pin the forwarding of the run request, the mirror of a measured
// result into AppState, the absence semantics (no result stays null — never
// a fabricated number), and the teardown on core loss, independently of the
// supervised Rust core.
TestCase {
    id: root

    name: "HarborNetworkBridge"

    QtObject {
        id: stubFacade

        property bool coreReady: true
        property bool networkDiagnosticsRunning: false
        property var networkDiagnostics: null
        property var calls: []

        // networkDiagnosticsChanged and coreReadyChanged come from the
        // properties themselves. The real facade notifies both properties
        // through that one shared signal; the stub's auto signal lives on
        // the result property, so a running flip re-asserts the result to
        // fire it the same way.

        function runNetworkDiagnostics() {
            var next = stubFacade.calls.slice()
            next.push("runNetworkDiagnostics")
            stubFacade.calls = next
            _setRunning(true)
        }

        function _setRunning(running) {
            stubFacade.networkDiagnosticsRunning = running
            var current = stubFacade.networkDiagnostics
            stubFacade.networkDiagnostics = current === null
                    ? { inFlight: true } : Object.assign({}, current)
        }

        function _publish(diagnostics) {
            stubFacade.networkDiagnostics = diagnostics
            stubFacade.networkDiagnosticsRunning = false
        }
    }

    HarborNetworkBridge {
        id: bridge
    }

    property var pristineDiagnostics: null
    property bool pristineRunning: false

    function init() {
        pristineDiagnostics = AppState.networkDiagnostics
        pristineRunning = AppState.networkDiagnosticsRunning
        bridge.facade = null
        stubFacade.calls = []
        stubFacade.coreReady = true
        stubFacade.networkDiagnosticsRunning = false
        stubFacade.networkDiagnostics = null
        AppState.networkDiagnostics = null
        AppState.networkDiagnosticsRunning = false
    }

    function cleanup() {
        bridge.facade = null
        AppState.networkDiagnostics = pristineDiagnostics
        AppState.networkDiagnosticsRunning = pristineRunning
    }

    function test_inertWithoutFacade() {
        verify(bridge.facade === null)
        verify(!bridge.live)
        // No facade: the run request is a no-op and never invents state.
        bridge.run()
        compare(stubFacade.calls, [])
        compare(AppState.networkDiagnostics, null)
        compare(AppState.networkDiagnosticsRunning, false)
    }

    function test_runForwardsOnlyWhenLive() {
        bridge.run()
        compare(stubFacade.calls, [])

        bridge.facade = stubFacade
        verify(bridge.live)
        bridge.run()
        compare(stubFacade.calls, ["runNetworkDiagnostics"])
        compare(AppState.networkDiagnosticsRunning, true)
    }

    function test_measuredResultMirrorsIntoAppState() {
        bridge.facade = stubFacade
        compare(AppState.networkDiagnostics, null)

        stubFacade._publish({
            serverConfigured: true,
            serverReachable: true,
            handshakeMs: 18.0,
            rttMs: 43.0,
            directActive: false
        })
        tryCompare(AppState, "networkDiagnosticsRunning", false)
        verify(AppState.networkDiagnostics !== null)
        compare(AppState.networkDiagnostics.serverConfigured, true)
        compare(AppState.networkDiagnostics.serverReachable, true)
        compare(AppState.networkDiagnostics.handshakeMs, 18.0)
        compare(AppState.networkDiagnostics.rttMs, 43.0)
        compare(AppState.networkDiagnostics.directActive, false)

        // A re-run replaces the whole snapshot, it never merges with the
        // previous one: facts absent from the new result stay absent.
        stubFacade.networkDiagnostics = {
            serverConfigured: false,
            serverReachable: false,
            directActive: false
        }
        verify(AppState.networkDiagnostics.handshakeMs === undefined)
        verify(AppState.networkDiagnostics.rttMs === undefined)
    }

    function test_unreachableServerMirrorsHonestly() {
        bridge.facade = stubFacade
        stubFacade._publish({
            serverConfigured: true,
            serverReachable: false,
            directActive: false
        })
        verify(AppState.networkDiagnostics !== null)
        compare(AppState.networkDiagnostics.serverConfigured, true)
        compare(AppState.networkDiagnostics.serverReachable, false)
        verify(AppState.networkDiagnostics.handshakeMs === undefined)
        verify(AppState.networkDiagnostics.rttMs === undefined)
    }

    function test_coreFaultClearsMirroredDiagnostics() {
        bridge.facade = stubFacade
        stubFacade._setRunning(true)
        verify(AppState.networkDiagnosticsRunning)

        // A dead core takes its measurements with it: no stale result or
        // stuck running flag may survive the outage.
        stubFacade.coreReady = false
        tryCompare(AppState, "networkDiagnostics", null)
        compare(AppState.networkDiagnosticsRunning, false)

        // Recovery mirrors fresh facts when they arrive.
        stubFacade.coreReady = true
        stubFacade._publish({
            serverConfigured: true,
            serverReachable: true,
            rttMs: 12.0,
            directActive: false
        })
        compare(AppState.networkDiagnostics.rttMs, 12.0)
    }
}
