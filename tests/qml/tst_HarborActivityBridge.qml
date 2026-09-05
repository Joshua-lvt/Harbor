import QtQuick
import QtTest
import Harbor 2.0

// Contract tests for the activity bridge. The facade is stubbed with the
// exact property surface the real C++ facade exposes, so these tests pin the
// snapshot→AppState mapping, the self-name injection, local time formatting,
// and the week totals independently of the supervised Rust core.
TestCase {
    id: root

    name: "HarborActivityBridge"

    QtObject {
        id: stubFacade

        property bool coreReady: true
        property string activityMonitorState: "running"
        property var activityTimeline: []
        property var activityStats: ({ games: 0, apps: 0, hours: 0 })
        // Strictly sanitized peer snapshot: the real facade never contains
        // process identifiers, paths, command lines, or titles.
        property var remoteActivity: []
        property var calls: []

        signal activityChanged

        function refreshActivity() {
            var next = stubFacade.calls.slice()
            next.push("refreshActivity")
            stubFacade.calls = next
        }

        function _publish() {
            stubFacade.activityChanged()
        }
    }

    HarborActivityBridge {
        id: bridge
    }

    property var pristineActivities
    property var pristineRemoteActivities
    property string pristineSelfName

    function init() {
        pristineActivities = AppState.activities
        pristineRemoteActivities = AppState.remoteActivities
        pristineSelfName = AppState.selfName
        bridge.facade = null
        stubFacade.calls = []
        stubFacade.coreReady = true
        stubFacade.activityMonitorState = "running"
        stubFacade.activityTimeline = []
        stubFacade.activityStats = ({ games: 0, apps: 0, hours: 0 })
        stubFacade.remoteActivity = []
        AppState.activities = []
        AppState.setRemoteActivities([])
    }

    function cleanup() {
        bridge.facade = null
        AppState.selfName = pristineSelfName
        AppState.activities = pristineActivities
        AppState.setRemoteActivities(pristineRemoteActivities)
    }

    function _hhmm(secondsSinceEpoch) {
        var date = new Date(secondsSinceEpoch * 1000)
        var hours = date.getHours()
        var minutes = date.getMinutes()
        return (hours < 10 ? "0" : "") + hours + ":" + (minutes < 10 ? "0" : "") + minutes
    }

    function test_inertWithoutFacade() {
        verify(bridge.facade === null)
        verify(!bridge.live)
        // No facade: snapshots and refreshes are no-ops, never invented data.
        stubFacade.activityTimeline = [{
            id: "a1", category: "game",
            titleKey: "activity.event.gameOpened", titleParams: { game: "Minecraft" },
            descriptionKey: "activity.event.gameLaunched", descriptionParams: {},
            occurredAt: 1756600000
        }]
        stubFacade._publish()
        compare(AppState.activities.length, 0)
        bridge.refresh()
        compare(stubFacade.calls, [])
        compare(bridge.weekStats, [])
    }

    function test_snapshotMapsIntoAppStateWithSelfName() {
        bridge.facade = stubFacade
        verify(bridge.live)

        stubFacade.activityTimeline = [
            {
                id: "a1", category: "game",
                titleKey: "activity.event.gameOpened", titleParams: { game: "Minecraft" },
                descriptionKey: "activity.event.gameLaunched", descriptionParams: {},
                occurredAt: 1756600140
            },
            {
                id: "a2", category: "system",
                titleKey: "activity.event.monitorStarted", titleParams: {},
                descriptionKey: "", descriptionParams: {},
                occurredAt: 1756600080
            }
        ]
        stubFacade._publish()

        compare(AppState.activities.length, 2)
        var game = AppState.activities[0]
        compare(game.id, "a1")
        compare(game.category, "game")
        compare(game.titleKey, "activity.event.gameOpened")
        compare(game.titleParams.game, "Minecraft")
        // The engine does not know display names; the bridge injects the self
        // name so "{name} opened {game}" resolves.
        compare(game.titleParams.name, AppState.selfName)
        compare(game.descriptionKey, "activity.event.gameLaunched")
        compare(game.time, _hhmm(1756600140))

        var system = AppState.activities[1]
        compare(system.category, "system")
        compare(system.titleParams.name, AppState.selfName)
        compare(system.descriptionKey, "")
    }

    function test_redactedGameTitleStillResolvesName() {
        bridge.facade = stubFacade

        stubFacade.activityTimeline = [{
            id: "a3", category: "game",
            titleKey: "activity.event.gameOpenedGeneric", titleParams: {},
            descriptionKey: "activity.event.gameLaunched", descriptionParams: {},
            occurredAt: 1756600200
        }]
        stubFacade._publish()

        var entry = AppState.activities[0]
        compare(entry.titleKey, "activity.event.gameOpenedGeneric")
        // Only the name: the hidden game title arrives as no parameter at all.
        compare(Object.keys(entry.titleParams).length, 1)
        compare(entry.titleParams.name, AppState.selfName)
    }

    function test_snapshotIsFreshOnBridgeCreation() {
        // The view (and its bridge) can be created long after the facade
        // already published snapshots; creation must adopt current state.
        stubFacade.activityTimeline = [{
            id: "a4", category: "app",
            titleKey: "activity.event.appOpened", titleParams: { app: "Discord" },
            descriptionKey: "activity.event.applicationOpened", descriptionParams: {},
            occurredAt: 1756600260
        }]
        bridge.facade = stubFacade
        tryCompare(AppState.activities, "length", 1)
        compare(AppState.activities[0].titleParams.app, "Discord")
    }

    function test_readyFlipResnapsTheStore() {
        stubFacade.activityTimeline = []
        bridge.facade = stubFacade
        tryCompare(AppState.activities, "length", 0)

        stubFacade.coreReady = false
        stubFacade.coreReady = true
        stubFacade.activityTimeline = [{
            id: "a5", category: "app",
            titleKey: "activity.event.appOpened", titleParams: { app: "Terminal" },
            descriptionKey: "activity.event.applicationOpened", descriptionParams: {},
            occurredAt: 1756600320
        }]
        stubFacade.coreReadyChanged()
        tryCompare(AppState.activities, "length", 1)
    }

    function test_coreFaultClearsTheLiveSnapshot() {
        bridge.facade = stubFacade
        stubFacade.activityTimeline = [{
            id: "a6", category: "app",
            titleKey: "activity.event.appOpened", titleParams: { app: "Terminal" },
            descriptionKey: "activity.event.applicationOpened", descriptionParams: {},
            occurredAt: 1756600380
        }]
        stubFacade._publish()
        tryCompare(AppState.activities, "length", 1)

        // A stopped core cannot leave its last private snapshot pretending to
        // be fresh; the bridge clears it before the view can fall back.
        stubFacade.coreReady = false
        tryCompare(AppState.activities, "length", 0)
        stubFacade.coreReady = true
    }

    function test_remoteRecordsRemainSeparatedAndSanitized() {
        bridge.facade = stubFacade
        stubFacade.remoteActivity = [{
            id: "remote-1", sender: "Taylor", category: "game", kind: "opened",
            label: "A game", occurredAt: 1756600400
        }]
        stubFacade._publish()

        compare(AppState.remoteActivities.length, 1)
        var record = AppState.remoteActivities[0]
        compare(record.id, "remote-1")
        compare(record.sender, "Taylor")
        compare(record.category, "game")
        compare(record.kind, "opened")
        compare(record.label, "A game")
        compare(record.time, _hhmm(1756600400))
        // The bridge presents peer data separately; it cannot contaminate the
        // local timeline or pass on hidden process material.
        verify(record.pid === undefined)
        verify(record.path === undefined)
        verify(record.commandLine === undefined)

        stubFacade.coreReady = false
        tryCompare(AppState.remoteActivities, "length", 0)
        stubFacade.coreReady = true
    }

    function test_weekStatsComeFromTheCoreWindow() {
        bridge.facade = stubFacade

        stubFacade.activityStats = ({ games: 3, apps: 5, hours: 2.5 })
        var stats = bridge.weekStats
        compare(stats.length, 3)
        compare(stats[0].value, "3")
        compare(stats[0].labelKey, "common.labels.games")
        compare(stats[1].value, "5")
        compare(stats[1].labelKey, "common.labels.applications")
        compare(stats[2].value, "2.5")
        compare(stats[2].labelKey, "common.labels.hours")

        // Whole hours render without a decimal tail, matching the fixtures.
        stubFacade.activityStats = ({ games: 1, apps: 2, hours: 6 })
        compare(bridge.weekStats[2].value, "6")

        // Garbage from the boundary degrades to honest zeros, never NaN.
        stubFacade.activityStats = ({ games: "x", apps: null, hours: -4 })
        compare(bridge.weekStats[0].value, "0")
        compare(bridge.weekStats[1].value, "0")
        compare(bridge.weekStats[2].value, "0")
    }

    function test_refreshGoesThroughTheFacade() {
        bridge.refresh()
        compare(stubFacade.calls, [])

        bridge.facade = stubFacade
        bridge.refresh()
        compare(stubFacade.calls, ["refreshActivity"])
    }
}
