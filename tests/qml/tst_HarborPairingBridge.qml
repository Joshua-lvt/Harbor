import QtQuick
import QtTest
import Harbor 2.0

// Contract tests for the pairing bridge. The facade is stubbed with the exact
// property and invokable surface the real C++ facade exposes, so these tests
// pin the phase→mode mapping and the action wiring independently of the
// supervised Rust process.
TestCase {
    id: root

    name: "HarborPairingBridge"

    property string pristinePartnerName
    property string pristinePartnerInitials
    property string pristinePartnerStatusKey
    property string pristinePartnerStatus

    QtObject {
        id: stubFacade

        property bool coreReady: true
        property string pairingPhase: "IDLE"
        property string pairingRole: ""
        property string pairingCode: ""
        property string pairingErrorKey: ""
        property var pairingIncoming: ({})
        property var calls: []

        signal pairingChanged

        function _call(entry) {
            var next = stubFacade.calls.slice()
            next.push(entry)
            stubFacade.calls = next
        }

        function _advance(phase, role) {
            stubFacade.pairingPhase = phase
            stubFacade.pairingRole = role
            stubFacade.pairingChanged()
        }

        function pairHostCreate() {
            stubFacade._call("pairing.create")
            stubFacade.pairingCode = "483920"
            stubFacade._advance("WAITING_APPROVAL", "host")
        }

        function pairEnterCode() {
            stubFacade._call("pairing.enter_code")
            stubFacade._advance("ENTERING_CODE", "peer")
        }

        function pairSubmit(code) {
            stubFacade._call("pairing.submit:" + code)
            stubFacade._advance("REQUESTING", "peer")
        }

        function pairPollIncoming() {
            stubFacade._call("pairing.incoming")
        }

        function pairPollStatus() {
            stubFacade._call("pairing.status")
        }

        function pairAccept() {
            stubFacade._call("pairing.accept")
            stubFacade._advance("ACCEPTED", "host")
        }

        function pairDecline() {
            stubFacade._call("pairing.decline")
            stubFacade._advance("DECLINED", "host")
        }

        function pairCancel() {
            stubFacade._call("pairing.cancel")
        }

        function pairReset() {
            stubFacade._call("pairing.reset")
            stubFacade.pairingCode = ""
            stubFacade.pairingErrorKey = ""
            stubFacade.pairingIncoming = ({})
            stubFacade._advance("IDLE", "")
        }

        function refreshPairingState() {
            stubFacade._call("pairing.state")
        }

        function copyToClipboard(text) {
            stubFacade._call("clipboard:" + text)
        }
    }

    HarborPairingBridge {
        id: bridge
    }

    // Signal spies for the two provider signals the view re-emits.
    property int completedCount: 0
    property string completedName: ""
    property int declinedCount: 0

    Connections {
        target: bridge

        function onPairingCompleted(partnerName) {
            root.completedCount++
            root.completedName = partnerName
        }

        function onIncomingPairingDeclined(partnerName) {
            root.declinedCount++
        }
    }

    function init() {
        pristinePartnerName = "Taylor"
        pristinePartnerInitials = "TA"
        pristinePartnerStatusKey = AppState.partnerStatusKey
        pristinePartnerStatus = AppState.partnerStatus
        bridge.facade = null
        stubFacade.calls = []
        stubFacade.coreReady = true
        stubFacade.pairingPhase = "IDLE"
        stubFacade.pairingRole = ""
        stubFacade.pairingCode = ""
        stubFacade.pairingErrorKey = ""
        stubFacade.pairingIncoming = ({})
        bridge._resetLocal()
        root.completedCount = 0
        root.completedName = ""
        root.declinedCount = 0
        AppState.pairingVisible = false
        AppState.connectionState = "connected"
    }

    function cleanup() {
        bridge.facade = null
        AppState.pairingVisible = false
        AppState.updatePartnerProfile({
            name: pristinePartnerName,
            initials: pristinePartnerInitials,
            status: pristinePartnerStatus
        })
        AppState.partnerStatusKey = pristinePartnerStatusKey
    }

    function test_inertWithoutFacade() {
        verify(bridge.facade === null)
        verify(!bridge.live)
        compare(bridge.pairingMode, "choice")
        // No facade: actions are no-ops, never a simulated flow.
        compare(bridge.setPairingMode("qr"), "")
        compare(bridge.completePairing("Taylor"), "")
        compare(bridge.submitPairingCode("483920"), false)
        compare(bridge.pairingMode, "choice")
        compare(stubFacade.calls, [])
    }

    function test_hostFlowMapsPhasesToModes() {
        bridge.facade = stubFacade
        verify(bridge.live)

        bridge.setPairingMode("qr")
        tryCompare(bridge, "pairingMode", "qr")
        compare(bridge.pairingCode, "483920")
        compare(bridge.pairingCodeSeconds, bridge.codeTtlSeconds)
        verify(stubFacade.calls.indexOf("pairing.create") >= 0)

        // A peer submitted the code: the poll surfaces it on the facade.
        stubFacade.pairingIncoming = ({ name: "peer-harbor-id", pairingId: "abc" })
        stubFacade._advance("INCOMING_REQUEST", "host")
        tryCompare(bridge, "pairingMode", "incoming")
        compare(bridge.incomingRequest.name, "peer-harbor-id")
        compare(bridge.incomingRequest.code, "483920")

        stubFacade.pairAccept()
        tryCompare(bridge, "pairingMode", "success")
        tryCompare(root, "completedCount", 1)
        compare(root.completedName, "peer-harbor-id")
    }

    function test_peerFlowSubmitValidationAndWaiting() {
        bridge.facade = stubFacade

        // Non-digit input normalizes down to digits; short codes fail locally
        // without ever calling the core.
        compare(bridge.submitPairingCode("48a 3"), false)
        tryCompare(bridge, "pairingMode", "error")
        compare(bridge.pairingErrorKey, "pairing.error.tooShort")
        verify(stubFacade.calls.indexOf("pairing.submit:483") < 0)

        compare(bridge.submitPairingCode("004 839"), true)
        tryCompare(bridge, "pairingMode", "waiting")
        compare(bridge.enteredPairingCode, "004839")
        verify(stubFacade.calls.indexOf("pairing.submit:004839") >= 0)
    }

    function test_peerSeesDeclineAsError() {
        bridge.facade = stubFacade
        bridge.submitPairingCode("483920")
        tryCompare(bridge, "pairingMode", "waiting")

        stubFacade._advance("DECLINED", "peer")
        tryCompare(bridge, "pairingMode", "error")
        compare(bridge.pairingErrorKey, "pairing.error.declined")
        compare(root.declinedCount, 0)
    }

    function test_hostDeclineClosesOverlayLikeTheMock() {
        bridge.facade = stubFacade
        bridge.setPairingMode("qr")
        stubFacade.pairingIncoming = ({ name: "peer-harbor-id", pairingId: "abc" })
        stubFacade._advance("INCOMING_REQUEST", "host")
        tryCompare(bridge, "pairingMode", "incoming")

        AppState.pairingVisible = true
        stubFacade.pairDecline()
        tryCompare(AppState, "pairingVisible", false)
        tryCompare(bridge, "pairingMode", "choice")
        compare(root.declinedCount, 1)
    }

    function test_transientPollOutageKeepsThePage() {
        bridge.facade = stubFacade
        bridge.setPairingMode("qr")
        tryCompare(bridge, "pairingMode", "qr")

        // A retryable server outage during polling must not kick the host
        // out of the code page; the next poll retries.
        stubFacade.pairingErrorKey = "error.server.unavailable"
        stubFacade._advance("ERROR", "host")
        compare(bridge.pairingMode, "qr")

        // Any other error is terminal and lands on the error page.
        stubFacade.pairingErrorKey = "error.server.unauthorized"
        stubFacade._advance("ERROR", "host")
        tryCompare(bridge, "pairingMode", "error")
        compare(bridge.pairingErrorKey, "error.server.unauthorized")
    }

    function test_reopeningTheOverlayStartsClean() {
        bridge.facade = stubFacade
        bridge.setPairingMode("qr")
        tryCompare(bridge, "pairingMode", "qr")

        AppState.pairingVisible = true
        tryCompare(bridge, "pairingMode", "choice")
        compare(bridge.pairingCodeSeconds, 0)
        verify(stubFacade.calls.indexOf("pairing.state") >= 0)
        verify(stubFacade.calls.indexOf("pairing.reset") >= 0)
    }

    function test_copyGoesThroughTheFacade() {
        bridge.facade = stubFacade
        bridge.mockCopy("483920", "pairingCode")
        verify(stubFacade.calls.indexOf("clipboard:483920") >= 0)
        verify(bridge.mockCopyFeedbackVisible)
        compare(bridge.mockCopyTarget, "pairingCode")
    }

    function test_copyWorksWhileReconnecting() {
        // Regression: the Harbor-ID copy showed "copied" while the core was
        // reconnecting but wrote nothing to the clipboard, leaving a stale
        // six-digit pairing code behind on paste.
        bridge.facade = stubFacade
        stubFacade.coreReady = false
        verify(!bridge.live)
        stubFacade.calls = []
        bridge.mockCopy("harbor-d31846b8", "harborId")
        verify(stubFacade.calls.indexOf("clipboard:harbor-d31846b8") >= 0)
        verify(bridge.mockCopyFeedbackVisible)
        compare(bridge.mockCopyTarget, "harborId")
        stubFacade.coreReady = true
    }

    function test_closePairingResetsAndHides() {
        bridge.facade = stubFacade
        bridge.setPairingMode("qr")
        tryCompare(bridge, "pairingMode", "qr")

        AppState.pairingVisible = true
        bridge.closePairing()
        compare(AppState.pairingVisible, false)
        compare(bridge.pairingMode, "choice")
        verify(stubFacade.calls.indexOf("pairing.reset") >= 0)
    }
}
