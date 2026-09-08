import QtQuick
import QtTest
import Harbor 2.0

// Contract tests for the direct bridge. The facade is stubbed with the exact
// property surface the real C++ facade exposes, so these tests pin the
// metadata-only snapshot mirroring (chat messages and transfers), core-fault
// teardown, the no-op behavior without a facade, and the localized surfacing
// of chat.*/transfer.* refusals — independently of the supervised Rust core.
TestCase {
    id: root

    name: "HarborDirectBridge"

    QtObject {
        id: stubFacade

        property bool coreReady: true
        // Sanitized snapshots only: no paths, digests, or chunk bytes.
        property var chatMessages: []
        property var transfers: []
        property var calls: []
        // Partner public profile only: no identity, keys, or revisions.
        property var partnerProfile: ({})

        // Mirror of the typed C++ settings surface the bridge reads/writes.
        property QtObject settings: QtObject {
            property string transferDirectory: ""
        }

        signal directChanged
        signal profileChanged
        // coreReadyChanged comes from the coreReady property itself.
        signal requestFailed(string requestType, string uiKey)

        function refreshDirectState() {
            var next = stubFacade.calls.slice()
            next.push("refreshDirectState")
            stubFacade.calls = next
        }

        function refreshProfileState() {
            var next = stubFacade.calls.slice()
            next.push("refreshProfileState")
            stubFacade.calls = next
        }

        function sendChatMessage(body) {
            var next = stubFacade.calls.slice()
            next.push("sendChatMessage:" + body)
            stubFacade.calls = next
        }

        function offerLocalFile(sourcePath) {
            var next = stubFacade.calls.slice()
            next.push("offerLocalFile:" + sourcePath)
            stubFacade.calls = next
        }

        function acceptTransfer(transferId) {
            var next = stubFacade.calls.slice()
            next.push("acceptTransfer:" + transferId)
            stubFacade.calls = next
        }

        function rejectTransfer(transferId) {
            var next = stubFacade.calls.slice()
            next.push("rejectTransfer:" + transferId)
            stubFacade.calls = next
        }

        function cancelTransfer(transferId) {
            var next = stubFacade.calls.slice()
            next.push("cancelTransfer:" + transferId)
            stubFacade.calls = next
        }

        function _publish(messages, transfers) {
            stubFacade.chatMessages = messages
            stubFacade.transfers = transfers
            stubFacade.directChanged()
        }

        function _publishProfile(partner) {
            stubFacade.partnerProfile = partner
            stubFacade.profileChanged()
        }
    }

    HarborDirectBridge {
        id: bridge
    }

    property var lastToast: null

    Connections {
        target: AppState

        function onLocalizedToastRequested(category, titleKey, titleParams,
                                           descriptionKey, descriptionParams) {
            root.lastToast = { category: category, titleKey: titleKey }
        }
    }

    property var pristineMessages
    property var pristineTransfers

    function init() {
        pristineMessages = AppState.chatMessages
        pristineTransfers = AppState.transfers
        bridge.facade = null
        stubFacade.calls = []
        stubFacade.coreReady = true
        stubFacade.chatMessages = []
        stubFacade.transfers = []
        stubFacade.partnerProfile = ({})
        stubFacade.settings.transferDirectory = ""
        lastToast = null
        AppState.setDirectState([], [])
    }

    function cleanup() {
        bridge.facade = null
        AppState.setDirectState(pristineMessages, pristineTransfers)
        AppState.updatePartnerProfile({
            name: "Taylor", status: "", avatar: "", avatarType: "image"
        })
    }

    function test_inertWithoutFacade() {
        verify(bridge.facade === null)
        verify(!bridge.live)
        // No facade: actions are no-ops and never invent state.
        bridge.refresh()
        bridge.sendMessage("hello")
        bridge.offerFile("/tmp/a.pdf")
        bridge.acceptTransfer("t-1")
        bridge.rejectTransfer("t-1")
        bridge.cancelTransfer("t-1")
        bridge.setTransferDirectory("/tmp/downloads")
        compare(stubFacade.calls, [])
        compare(stubFacade.settings.transferDirectory, "")
        compare(bridge.transferDirectory, "")
        compare(AppState.chatMessages.length, 0)
        compare(AppState.transfers.length, 0)
    }

    function test_snapshotsMirrorIntoAppState() {
        bridge.facade = stubFacade
        verify(bridge.live)

        var messages = [
            { id: "m-1", body: "hello peer", direction: "OUTGOING", delivery: "DELIVERED" },
            { id: "m-2", body: "hi", direction: "INCOMING", delivery: "SENT" }
        ]
        var transfers = [
            { id: "t-1", direction: "INCOMING", state: "OFFERED", name: "notes.txt",
              size: 12, receivedBytes: 0, peerReceived: false, expired: false }
        ]
        stubFacade._publish(messages, transfers)
        compare(AppState.chatMessages.length, 2)
        compare(AppState.chatMessages[0].body, "hello peer")
        compare(AppState.chatMessages[1].direction, "INCOMING")
        compare(AppState.transfers.length, 1)
        compare(AppState.transfers[0].name, "notes.txt")
        compare(AppState.transfers[0].state, "OFFERED")
    }

    function test_bridgeAdoptsStateOnCreation() {
        // The view can appear long after the core published its snapshot.
        stubFacade.chatMessages = [{ id: "m-1", body: "early", direction: "INCOMING", delivery: "SENT" }]
        bridge.facade = stubFacade
        tryCompare(AppState, "chatMessages", stubFacade.chatMessages)
    }

    function test_actionsForwardThroughTheFacade() {
        bridge.facade = stubFacade

        bridge.sendMessage("hello peer")
        bridge.refresh()
        compare(stubFacade.calls, ["sendChatMessage:hello peer", "refreshDirectState", "refreshProfileState"])
    }

    function test_transferActionsForwardThroughTheFacade() {
        bridge.facade = stubFacade

        bridge.offerFile("/tmp/relatorio.pdf")
        bridge.acceptTransfer("t-1")
        bridge.rejectTransfer("t-2")
        bridge.cancelTransfer("t-3")
        compare(stubFacade.calls, [
            "offerLocalFile:/tmp/relatorio.pdf",
            "acceptTransfer:t-1",
            "rejectTransfer:t-2",
            "cancelTransfer:t-3"
        ])

        // Dead core: the offers and verdicts must not reach a corpse.
        stubFacade.calls = []
        stubFacade.coreReady = false
        bridge.offerFile("/tmp/relatorio.pdf")
        bridge.cancelTransfer("t-1")
        stubFacade.coreReady = true
        compare(stubFacade.calls, [])
    }

    function test_transferDirectoryMirrorsSettings() {
        // The bridge exposes exactly the typed settings property, and an
        // empty string still means "platform default" — never a made-up path.
        stubFacade.settings.transferDirectory = "/tmp/downloads"
        compare(bridge.transferDirectory, "")

        bridge.facade = stubFacade
        compare(bridge.transferDirectory, "/tmp/downloads")

        bridge.setTransferDirectory("/home/test/Downloads")
        compare(stubFacade.settings.transferDirectory, "/home/test/Downloads")
        compare(bridge.transferDirectory, "/home/test/Downloads")

        bridge.facade = null
        compare(bridge.transferDirectory, "")
    }

    function test_coreFaultDropsMirroredState() {
        bridge.facade = stubFacade
        stubFacade._publish(
            [{ id: "m-1", body: "hello", direction: "OUTGOING", delivery: "SENT" }],
            [{ id: "t-1", direction: "OUTGOING", state: "ACTIVE", name: "a.bin",
               size: 3, receivedBytes: 0, peerReceived: false, expired: false }])
        compare(AppState.chatMessages.length, 1)
        compare(AppState.transfers.length, 1)

        // A dead core takes its session with it; no stale transcript or
        // transfer may survive the outage.
        stubFacade.coreReady = false
        tryCompare(AppState, "chatMessages", [])
        compare(AppState.transfers.length, 0)
        stubFacade.coreReady = true
    }

    function test_directRefusalsSurfaceAsLocalizedToasts() {
        bridge.facade = stubFacade

        stubFacade.requestFailed("chat.send", "error.chat.tooLarge")
        compare(lastToast.category, "direct")
        compare(lastToast.titleKey, "error.chat.tooLarge")

        stubFacade.requestFailed("transfer.offer_local", "error.transfer.tooLarge")
        compare(lastToast.titleKey, "error.transfer.tooLarge")

        // A missing key still surfaces instead of failing silently.
        stubFacade.requestFailed("transfer.accept", "")
        compare(lastToast.titleKey, "error.core.unavailable")

        // Other families belong to their own bridges.
        stubFacade.requestFailed("pairing.submit", "error.server.unavailable")
        compare(lastToast.titleKey, "error.core.unavailable")
    }

    // The peer's public profile lands on the session partner profile: name,
    // status, avatar and GIF type update every surface at once. Presence is
    // never carried by profile content.
    function test_partnerProfileMirrorsIntoAppState() {
        bridge.facade = stubFacade
        AppState.updatePartnerProfile({ name: "Taylor", presence: "online" })

        stubFacade._publishProfile({
            displayName: "Taylor R.",
            statusMessage: "Building a cabin",
            avatarType: "gif",
            avatar: "data:image/gif;base64,R0lGODlhAQABAIAAAP///wAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw=="
        })
        compare(AppState.partnerName, "Taylor R.")
        compare(AppState.partnerProfile.status, "Building a cabin")
        compare(AppState.partnerProfile.avatarType, "gif")
        verify(String(AppState.partnerProfile.avatar).indexOf("data:image/gif") === 0)
        // Presence still belongs to the contacts bridge.
        compare(AppState.partnerState, "online")

        // An empty display name never blanks a name learned earlier.
        stubFacade._publishProfile({ displayName: "", statusMessage: "Away", avatarType: "image", avatar: "" })
        compare(AppState.partnerName, "Taylor R.")
        compare(AppState.partnerProfile.status, "Away")

        // Avatar removal returns the peer to initials honestly.
        compare(AppState.partnerProfile.avatar, "")
        compare(AppState.partnerProfile.avatarType, "image")
    }
}
