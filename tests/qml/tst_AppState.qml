import QtQuick
import QtTest
import Harbor 2.0

TestCase {
    name: "AppState"

    function init() {
        MockController.resetSession()
    }

    function cleanup() {
        MockController.resetSession()
    }

    function test_stateNormalization() {
        compare(AppState.normalizeConnectionState("online"), "connected")
        compare(AppState.normalizeConnectionState("invalid"), "disconnected")
        compare(AppState.normalizePartnerState("away"), "idle")
        compare(AppState.normalizeCallState("active"), "connected")
    }

    function test_collectionReplacement() {
        var before = AppState.notifications
        verify(AppState.appendItem("notifications", {
            id: "test-notification",
            unread: true
        }))
        verify(before !== AppState.notifications)
        compare(AppState.itemIndex("notifications", "test-notification"), AppState.notifications.length - 1)
        verify(AppState.removeItem("notifications", "test-notification"))
    }

    function test_unreadCountTracksCollection() {
        compare(AppState.unreadCount, 2)
        AppState.markAllNotificationsRead()
        compare(AppState.unreadCount, 0)
    }

    function test_pageStateContract() {
        verify(AppState.setPageState("home", "loading"))
        compare(AppState.pageState("home"), "loading")
        verify(!AppState.setPageState("home", "invalid"))
        compare(AppState.pageState("home"), "loading")
    }

    function test_closeActionNeverHidesWithoutATray() {
        // Close-to-tray is only real when the platform actually renders a
        // tray icon; otherwise closing must quit so the app is never left
        // running invisibly.
        compare(AppState.resolveCloseAction(true, true), "hide")
        compare(AppState.resolveCloseAction(false, true), "quit")
        compare(AppState.resolveCloseAction(true, false), "quit")
        compare(AppState.resolveCloseAction(false, false), "quit")
    }

    function test_profileSnapshotsRemainSeparate() {
        AppState.updateSelfProfile({ name: "River" })
        compare(AppState.selfProfile.name, "River")
        compare(AppState.partnerProfile.name, "Taylor")
    }

    function test_fixtureLabelsUseStableKeysAndAudioIds() {
        compare(AppState.selfStatusKey, "fixture.profile.selfStatus")
        compare(AppState.partnerStatusKey, "fixture.profile.partnerStatus")
        compare(AppState.inputDevice, "default-microphone")
        compare(AppState.outputDevice, "harbor-headphones")
        compare(AppState.devices[0].nameKey, "fixture.device.selfDesktop")
        compare(AppState.routeNodes[2].labelKey, "fixture.device.partnerLaptop")
    }

    function test_pairedPeersGateContract() {
        // The deterministic fixture world is already paired: the gate
        // stays hidden in tests exactly as it stays hidden after a real
        // pairing snapshot lands.
        verify(AppState.pairedPeersResolved)
        verify(AppState.paired)

        // A snapshot replaces, never merges; empty means honestly unpaired.
        AppState.setPairedPeers([])
        compare(AppState.pairedPeers.length, 0)
        verify(AppState.pairedPeersResolved)
        verify(!AppState.paired)

        // Malformed input is treated as an empty snapshot, never a crash.
        AppState.setPairedPeers("garbage")
        compare(AppState.pairedPeers.length, 0)

        // The bypass is session-scoped and never fabricates a pair.
        AppState.continueWithoutPairing()
        verify(AppState.pairingBypassed)
        verify(!AppState.paired)

        AppState.resetSession()
        verify(!AppState.pairedPeersResolved)
        verify(!AppState.pairingBypassed)
        verify(!AppState.paired)

        // Seeding again restores the fixture pair.
        MockController.resetSession()
        verify(AppState.paired)
    }

    function test_fixtureStatusUpdatesButEditedStatusRemainsLiteral() {
        AppState.locale = "pt-BR"
        tryCompare(I18n, "locale", "pt-BR")
        compare(AppState.selfStatusDisplay, "Curtindo a calmaria perto da água")
        compare(AppState.partnerStatusDisplay,
                "Construindo uma pequena cabana à beira do lago")

        AppState.updateSelfProfile({ status: "Custom status" })
        AppState.updatePartnerProfile({ status: "Partner custom status" })
        compare(AppState.selfStatusKey, "")
        compare(AppState.partnerStatusKey, "")
        AppState.locale = "en"
        tryCompare(I18n, "locale", "en")
        compare(AppState.selfStatusDisplay, "Custom status")
        compare(AppState.partnerStatusDisplay, "Partner custom status")
    }
}
