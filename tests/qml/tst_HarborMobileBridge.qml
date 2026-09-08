import QtQuick
import QtTest
import Harbor 2.0

// Contract tests for the phone bridge. The facade is stubbed with the exact
// property surface the real C++ facade exposes, so these tests pin the peer
// snapshot mirroring, the display-only notice FIFO, core-fault teardown,
// facade-loss teardown, and the inert behavior without a facade —
// independently of the supervised Rust core.
TestCase {
    id: root

    name: "HarborMobileBridge"

    QtObject {
        id: stubFacade

        property bool coreReady: true
        // Sanitized {own, peer} snapshot; either side null until shared.
        property var mobileState: ({ own: null, peer: null })

        signal mobileChanged()
        signal phoneNotification(var payload)

        function refreshMobile() {}
    }

    HarborMobileBridge {
        id: bridge
    }

    function _peer(overrides) {
        var base = {
            batteryPercent: 73, charging: false, phoneActivity: "ACTIVE",
            lastActiveAt: 1700000000, currentApp: "Minecraft",
            locationSharingEnabled: false, location: null,
            notificationSharingEnabled: false, deviceType: "mobile"
        }
        overrides = overrides || {}
        for (var key in overrides)
            base[key] = overrides[key]
        return base
    }

    function init() {
        bridge.facade = null
        stubFacade.coreReady = true
        stubFacade.mobileState = { own: null, peer: null }
        AppState.resetSession()
        AppState.setPeerPhone(null)
        AppState.clearPhoneNotices()
    }

    function cleanup() {
        bridge.facade = null
        AppState.resetSession()
    }

    function test_inertWithoutFacade() {
        verify(bridge.facade === null)
        verify(!bridge.live)
        // No facade: the bridge never invents a snapshot.
        verify(AppState.peerPhone === null)
        verify(AppState.phoneNotices.length === 0)

        // Attaching a dead core is still inert: no snapshot may appear out
        // of a corpse.
        stubFacade.coreReady = false
        bridge.facade = stubFacade
        verify(!bridge.live)
        verify(AppState.peerPhone === null)
    }

    function test_peerSnapshotMirrorsIntoAppState() {
        bridge.facade = stubFacade
        verify(bridge.live)

        stubFacade.mobileState = { own: null, peer: _peer() }
        stubFacade.mobileChanged()
        verify(AppState.peerPhone !== null)
        compare(AppState.peerPhone.batteryPercent, 73)
        compare(AppState.peerPhone.phoneActivity, "ACTIVE")
        compare(AppState.peerPhone.currentApp, "Minecraft")
        verify(AppState.peerPhoneSeenAt > 0)
    }

    function test_nullPeerClearsInsteadOfLingering() {
        bridge.facade = stubFacade
        stubFacade.mobileState = { own: null, peer: _peer() }
        stubFacade.mobileChanged()
        verify(AppState.peerPhone !== null)

        // The peer stopped sharing: the view returns to null, never stale.
        stubFacade.mobileState = { own: null, peer: null }
        stubFacade.mobileChanged()
        verify(AppState.peerPhone === null)
    }

    function test_malformedSnapshotIsNullSafe() {
        bridge.facade = stubFacade
        stubFacade.mobileState = { own: null, peer: { batteryPercent: 500, phoneActivity: "dancing" } }
        stubFacade.mobileChanged()
        // Invalid members fall back to honest empties, never crash.
        verify(AppState.peerPhone === null || AppState.peerPhone.batteryPercent === null
               || AppState.peerPhone.batteryPercent === undefined)
        stubFacade.mobileState = "nonsense"
        stubFacade.mobileChanged()
        verify(AppState.peerPhone === null)
    }

    function test_noticesAreBoundedDisplayOnlyFifo() {
        bridge.facade = stubFacade
        for (var i = 0; i < 10; i++) {
            bridge._pushNotice({ appLabel: "App" + i, title: "T" + i, text: "B" + i,
                                 timestamp: 1700000000 + i })
        }
        compare(AppState.phoneNotices.length, 8)
        // Oldest two dropped, order preserved.
        compare(AppState.phoneNotices[0].appLabel, "App2")
        compare(AppState.phoneNotices[7].appLabel, "App9")
        // Millisecond stamps (Android) normalize to seconds.
        bridge._pushNotice({ appLabel: "Ms", title: "T", text: "B", timestamp: 1700000000000 })
        compare(AppState.phoneNotices[7].at, 1700000000)
    }

    function test_noticeSignalPushesDisplayOnlyNotice() {
        bridge.facade = stubFacade
        verify(bridge.live)
        stubFacade.phoneNotification({ appLabel: "Chat", title: "Taylor",
                                       text: "hey", timestamp: 1700000001 })
        compare(AppState.phoneNotices.length, 1)
        compare(AppState.phoneNotices[0].appLabel, "Chat")
        compare(AppState.phoneNotices[0].title, "Taylor")
    }

    function test_coreFaultClearsInsteadOfLeavingStaleFacts() {
        bridge.facade = stubFacade
        stubFacade.mobileState = { own: null, peer: _peer() }
        stubFacade.mobileChanged()
        bridge._pushNotice({ appLabel: "App", title: "T", text: "B", timestamp: 1700000000 })
        verify(AppState.peerPhone !== null)
        verify(AppState.phoneNotices.length === 1)

        stubFacade.coreReady = false
        stubFacade.coreReadyChanged()
        verify(AppState.peerPhone === null)
        verify(AppState.phoneNotices.length === 0)
    }

    function test_facadeLossClears() {
        bridge.facade = stubFacade
        stubFacade.mobileState = { own: null, peer: _peer() }
        stubFacade.mobileChanged()
        verify(AppState.peerPhone !== null)

        bridge.facade = null
        verify(AppState.peerPhone === null)
        verify(AppState.phoneNotices.length === 0)
    }
}
