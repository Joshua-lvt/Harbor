import QtQuick
import QtTest
import Harbor 2.0

// Contract tests for the pairing bridge. The facade is stubbed with the exact
// property and invokable surface the real C++ facade exposes, so these tests
// pin the phase→mode mapping and the action wiring independently of the
// supervised Rust process. Pairing is Harbor-ID based throughout.
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
        property string pairingPeerHarborId: ""
        property string pairingErrorKey: ""
        property string identityHarborId: "harbor-aaaaaaaa"
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

        function pairInvite(harborId) {
            stubFacade._call("pairing.invite:" + harborId)
            stubFacade.pairingPeerHarborId = harborId
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
            stubFacade.pairingPeerHarborId = ""
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

        function onPairingCompleted(partnerHarborId) {
            root.completedCount++
            root.completedName = partnerHarborId
        }

        function onIncomingPairingDeclined(partnerHarborId) {
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
        stubFacade.pairingPeerHarborId = ""
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
        compare(bridge.pairingMode, "home")
        // No facade: actions are no-ops, never a simulated flow.
        compare(bridge.connectWithHarborId("harbor-d31846b8"), false)
        compare(bridge.completePairing("harbor-d31846b8"), "")
        compare(bridge.pairingMode, "home")
        compare(stubFacade.calls, [])
    }

    function test_harborIdValidationGatesTheRequest() {
        // The canonical shape, mirrored from the core and server gates.
        verify(bridge.isValidHarborId("harbor-d31846b8"))
        verify(bridge.isValidHarborId("harbor-12345678"))
        verify(bridge.isValidHarborId("harbor-abcdef12"))
        verify(bridge.isValidHarborId("harbor-ABCDEF12"))
        verify(!bridge.isValidHarborId("123456"))
        verify(!bridge.isValidHarborId("d31846b8"))
        verify(!bridge.isValidHarborId("harbor-d31846b"))
        verify(!bridge.isValidHarborId("harbor-d31846b89"))
        verify(!bridge.isValidHarborId("harbor-d31846g8"))
        verify(!bridge.isValidHarborId("harbor_12345678"))
        verify(!bridge.isValidHarborId("harbor 12345678"))
        verify(!bridge.isValidHarborId(""))
    }

    function test_invalidIdsNeverReachTheCore() {
        bridge.facade = stubFacade

        compare(bridge.connectWithHarborId("123456"), false)
        compare(bridge.harborIdFieldState, "invalid")
        verify(stubFacade.calls.indexOf("pairing.invite:123456") < 0)

        compare(bridge.connectWithHarborId(" harbor-d31846b8"), false)
        compare(bridge.connectWithHarborId("harbor-d31846b8 "), false)
        compare(bridge.harborIdFieldState, "invalid")
        verify(stubFacade.calls.every(call => call.indexOf("pairing.invite:") !== 0))

        // Empty input is a distinct state, also quiet.
        compare(bridge.connectWithHarborId("   "), false)
        compare(stubFacade.calls, [])
    }

    function test_validInviteSendsTheFullHarborId() {
        bridge.facade = stubFacade

        compare(bridge.connectWithHarborId("harbor-d31846b8"), true)
        tryCompare(bridge, "pairingMode", "connecting")
        compare(bridge.enteredHarborId, "harbor-d31846b8")
        compare(bridge.harborIdFieldState, "valid")
        verify(stubFacade.calls.indexOf("pairing.invite:harbor-d31846b8") >= 0)

        // A second connect while one is in flight is refused: no duplicates.
        compare(bridge.connectWithHarborId("harbor-12345678"), false)
        verify(stubFacade.calls.indexOf("pairing.invite:harbor-12345678") < 0)
    }

    function test_incomingShowsHarborIdAndCompletes() {
        bridge.facade = stubFacade

        // The server attaches the requester's Harbor ID; the bridge prefers
        // it over the legacy bare-UUID name.
        stubFacade.pairingIncoming = ({ harborId: "harbor-bbbbbbbb", pairingId: "abc" })
        stubFacade._advance("INCOMING_REQUEST", "host")
        tryCompare(bridge, "pairingMode", "incoming")
        compare(bridge.incomingRequest.harborId, "harbor-bbbbbbbb")

        stubFacade.pairAccept()
        tryCompare(bridge, "pairingMode", "success")
        tryCompare(root, "completedCount", 1)
        compare(root.completedName, "harbor-bbbbbbbb")
    }

    function test_attachingToExistingIncomingStateSynchronizesImmediately() {
        stubFacade.pairingIncoming = ({ harborId: "harbor-bbbbbbbb", pairingId: "abc" })
        stubFacade.pairingPhase = "INCOMING_REQUEST"
        stubFacade.pairingRole = "host"

        bridge.facade = stubFacade

        tryCompare(bridge, "pairingMode", "incoming")
        compare(bridge.incomingRequest.harborId, "harbor-bbbbbbbb")
        compare(bridge.incomingRequest.pairingId, "abc")
    }

    function test_openingForIncomingRequestDoesNotResetIt() {
        stubFacade.pairingIncoming = ({ harborId: "harbor-bbbbbbbb", pairingId: "abc" })
        stubFacade.pairingPhase = "INCOMING_REQUEST"
        stubFacade.pairingRole = "host"
        bridge.facade = stubFacade

        AppState.pairingVisible = true

        tryCompare(bridge, "pairingMode", "incoming")
        verify(stubFacade.calls.indexOf("pairing.reset") < 0)
        compare(stubFacade.pairingPhase, "INCOMING_REQUEST")
    }

    function test_peerSeesDeclineAsError() {
        bridge.facade = stubFacade
        bridge.connectWithHarborId("harbor-d31846b8")
        tryCompare(bridge, "pairingMode", "connecting")

        stubFacade._advance("DECLINED", "peer")
        tryCompare(bridge, "pairingMode", "error")
        compare(bridge.pairingErrorKey, "pairing.error.declined")
        compare(root.declinedCount, 0)
    }

    function test_transientPollOutageKeepsThePage() {
        bridge.facade = stubFacade
        bridge.connectWithHarborId("harbor-d31846b8")
        tryCompare(bridge, "pairingMode", "connecting")

        // A retryable server outage during polling must not kick the peer
        // out of the connecting page; the next poll retries.
        stubFacade.pairingErrorKey = "error.server.unavailable"
        stubFacade._advance("ERROR", "peer")
        compare(bridge.pairingMode, "connecting")

        // Any other error is terminal and lands on the error page.
        stubFacade.pairingErrorKey = "error.server.unauthorized"
        stubFacade._advance("ERROR", "peer")
        tryCompare(bridge, "pairingMode", "error")
        compare(bridge.pairingErrorKey, "error.server.unauthorized")
    }

    function test_reopeningTheOverlayStartsClean() {
        bridge.facade = stubFacade
        bridge.connectWithHarborId("harbor-d31846b8")
        tryCompare(bridge, "pairingMode", "connecting")

        AppState.pairingVisible = true
        tryCompare(bridge, "pairingMode", "home")
        compare(bridge.enteredHarborId, "")
        verify(stubFacade.calls.indexOf("pairing.state") >= 0)
        verify(stubFacade.calls.indexOf("pairing.reset") >= 0)
    }

    function test_copyGoesThroughTheFacade() {
        bridge.facade = stubFacade
        bridge.mockCopy("harbor-d31846b8", "harborId")
        verify(stubFacade.calls.indexOf("clipboard:harbor-d31846b8") >= 0)
        verify(bridge.mockCopyFeedbackVisible)
        compare(bridge.mockCopyTarget, "harborId")
    }

    function test_copyWorksWhileReconnecting() {
        // Regression: the Harbor-ID copy showed "copied" while the core was
        // reconnecting but wrote nothing to the clipboard.
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
        bridge.connectWithHarborId("harbor-d31846b8")
        tryCompare(bridge, "pairingMode", "connecting")

        AppState.pairingVisible = true
        bridge.closePairing()
        compare(AppState.pairingVisible, false)
        compare(bridge.pairingMode, "home")
        verify(stubFacade.calls.indexOf("pairing.reset") >= 0)
    }

    function test_closingIncomingRequestDeclinesIt() {
        bridge.facade = stubFacade
        AppState.pairingVisible = true
        stubFacade.pairingIncoming = ({ harborId: "harbor-bbbbbbbb", pairingId: "abc" })
        stubFacade._advance("INCOMING_REQUEST", "host")
        tryCompare(bridge, "pairingMode", "incoming")

        bridge.closePairing()

        compare(AppState.pairingVisible, false)
        verify(stubFacade.calls.indexOf("pairing.decline") >= 0)
        verify(stubFacade.calls.indexOf("pairing.reset") >= 0)
    }

    function test_shellClosingIncomingRequestDeclinesIt() {
        bridge.facade = stubFacade
        AppState.pairingVisible = true
        stubFacade.pairingIncoming = ({ harborId: "harbor-bbbbbbbb", pairingId: "abc" })
        stubFacade._advance("INCOMING_REQUEST", "host")
        tryCompare(bridge, "pairingMode", "incoming")
        stubFacade.calls = []

        AppState.pairingVisible = false

        verify(stubFacade.calls.indexOf("pairing.decline") >= 0)
        verify(stubFacade.calls.indexOf("pairing.reset") >= 0)
    }
}
