import QtQuick
import QtTest
import Harbor 2.0

// Contract tests for the contacts bridge. The facade is stubbed with the
// exact property surface the real C++ facade exposes, so these tests pin
// the durable paired-peer snapshot mirroring ({deviceId, harborId} only),
// core-fault teardown, facade-loss teardown, and the inert behavior without
// a facade — independently of the supervised Rust core.
TestCase {
    id: root

    name: "HarborContactsBridge"

    QtObject {
        id: stubFacade

        property bool coreReady: true
        // Durable relationship snapshot from the control plane; no keys.
        // Readiness is published before the snapshot signal, matching C++.
        property var pairedPeers: []
        property bool pairedPeersResolved: false
        property string deviceName: ""

        function _publish(peers) {
            stubFacade.pairedPeers = peers
            stubFacade.pairedPeersResolved = true
            stubFacade.pairedPeersResolvedChanged()
            stubFacade.pairedPeersChanged()
        }
    }

    HarborContactsBridge {
        id: bridge
    }

    function init() {
        bridge.facade = null
        stubFacade.coreReady = true
        stubFacade.pairedPeers = []
        stubFacade.pairedPeersResolved = false
        stubFacade.deviceName = ""
        // resetSession leaves pairedPeersResolved false: no snapshot yet,
        // and restores the three fixture devices.
        AppState.resetSession()
        compare(AppState.devices.length, 3)
    }

    function cleanup() {
        bridge.facade = null
        AppState.resetSession()
    }

    function test_inertWithoutFacade() {
        verify(bridge.facade === null)
        verify(!bridge.live)
        // No facade: the bridge never invents a snapshot, so the shell's
        // "not resolved yet" state survives untouched.
        verify(!AppState.pairedPeersResolved)
        verify(!AppState.paired)

        // Attaching a dead core is still inert for adoption: no snapshot
        // may appear out of a corpse.
        stubFacade.coreReady = false
        bridge.facade = stubFacade
        verify(!bridge.live)
        verify(!AppState.pairedPeersResolved)
    }

    function test_snapshotsMirrorIntoAppState() {
        bridge.facade = stubFacade
        verify(bridge.live)

        stubFacade._publish([
            { deviceId: "peer-device", harborId: "HBR-5E20-9B31" }
        ])
        compare(AppState.pairedPeers.length, 1)
        compare(AppState.pairedPeers[0].deviceId, "peer-device")
        compare(AppState.pairedPeers[0].harborId, "HBR-5E20-9B31")
        verify(AppState.pairedPeersResolved)
        verify(AppState.paired)
    }

    function test_emptySnapshotIsStillAuthoritative() {
        bridge.facade = stubFacade
        // An empty snapshot from a live core means "durably unpaired", which
        // is different from "not fetched yet" and unlocks the honest gate.
        stubFacade._publish([])
        compare(AppState.pairedPeers.length, 0)
        verify(AppState.pairedPeersResolved)
        verify(!AppState.paired)
    }

    function test_coreFaultDropsSnapshot() {
        bridge.facade = stubFacade
        stubFacade._publish([{ deviceId: "peer-device", harborId: "HBR-5E20-9B31" }])
        verify(AppState.paired)

        // A dead core takes its view of the relationship with it — the real
        // facade clears its own snapshot on fault, so the stub mirrors that.
        // No stale pairing may survive the outage.
        stubFacade.coreReady = false
        stubFacade.pairedPeers = []
        tryCompare(AppState, "pairedPeers", [])
        verify(AppState.pairedPeersResolved)
        verify(!AppState.paired)

        // Recovery re-mirrors the snapshot the core republishes.
        stubFacade.coreReady = true
        verify(!AppState.paired)
        stubFacade._publish([{ deviceId: "peer-device-2", harborId: "HBR-1111-2222" }])
        compare(AppState.pairedPeers[0].deviceId, "peer-device-2")
        verify(AppState.paired)
    }

    function test_facadeLossDropsSnapshot() {
        bridge.facade = stubFacade
        stubFacade._publish([{ deviceId: "peer-device", harborId: "HBR-5E20-9B31" }])
        verify(AppState.paired)

        bridge.facade = null
        tryCompare(AppState, "pairedPeers", [])
        verify(!AppState.paired)
        // The devices page loses its real facts with the core; the empty
        // state (not a fixture world) is what the user then sees.
        compare(AppState.devices.length, 0)
    }

    function test_devicesPageComposesRealEntries() {
        // Without a core the fixture world stands untouched.
        bridge.facade = stubFacade
        stubFacade.deviceName = "workstation"
        stubFacade._publish([{ deviceId: "peer-device", harborId: "HBR-5E20-9B31" }])

        compare(AppState.devices.length, 2)
        compare(AppState.devices[0].name, "workstation")
        compare(AppState.devices[0].typeKey, "devices.type.thisDevice")
        verify(AppState.devices[0].primary)
        // The peer is named by its durable Harbor id — never a persona name.
        compare(AppState.devices[1].name, "HBR-5E20-9B31")
        verify(AppState.devices[1].manageable === false)

        // Unpaired: this machine is the only real device.
        stubFacade._publish([])
        compare(AppState.devices.length, 1)
        compare(AppState.devices[0].id, "self")
    }
}
