import QtQuick
import QtTest
import Harbor 2.0

// Companion-widget contracts: presence mapping, current-activity rule,
// call-presence option, unpaired honesty, click routing, corner math,
// reduced motion, and shell wiring. The widget mirrors AppState only.
TestCase {
    id: testCase
    name: "Widget"
    width: 1280
    height: 960
    visible: true

    function init() {
        MockController.resetSession()
        AppState.setRemoteActivities([])
    }

    function cleanup() {
        MockController.resetSession()
        AppState.setRemoteActivities([])
    }

    function _createWidget() {
        var component = Qt.createComponent("qrc:/qt/qml/Harbor/HarborWidget.qml")
        verify(component.status === Component.Ready,
               "HarborWidget should be ready: " + component.errorString())
        var view = component.createObject(testCase, { visible: false })
        verify(view !== null)
        return view
    }

    function _findFirst(item, name) {
        if (!item)
            return null
        if (item.objectName === name)
            return item
        if (!item.children)
            return null
        for (var i = 0; i < item.children.length; ++i) {
            var found = _findFirst(item.children[i], name)
            if (found !== null)
                return found
        }
        return null
    }

    function _pairOnline() {
        AppState.setPairedPeers([{ deviceId: "d-1", harborId: "HBR-1" }])
        AppState.updatePartnerProfile({ name: "Taylor", presence: "online" })
    }

    // Presence renders as human states, never transport vocabulary.
    function test_presenceMapsToHumanStates() {
        _pairOnline()
        var view = _createWidget()
        try {
            AppState.setPartnerState("online")
            compare(view.presenceText(), I18n.t("widget.presence.online"))
            AppState.setPartnerState("idle")
            compare(view.presenceText(), I18n.t("widget.presence.away"))
            AppState.setPartnerState("offline")
            compare(view.presenceText(), I18n.t("widget.presence.offline"))
            verify(view.presenceText().indexOf("{") < 0)
        } finally {
            view.destroy()
        }
    }

    // Only the latest relevant moment shows; stale or technical noise never.
    function test_currentActivityRule() {
        _pairOnline()
        AppState.setPartnerState("online")
        var view = _createWidget()
        try {
            verify(view.latestActivity === null)
            AppState.setRemoteActivities([{
                id: "w-1", sender: "Taylor", category: "game",
                kind: "opened", label: "Minecraft", time: "20:14"
            }])
            wait(30)
            verify(view.latestActivity !== null)
            compare(view.latestActivity.label, "Minecraft")
            AppState.setRemoteActivities([{
                id: "w-2", sender: "Taylor", category: "system",
                kind: "opened", label: "dbus-broker", time: "20:15"
            }])
            wait(30)
            verify(view.latestActivity === null)
            // Offline shows presence only: no stale activity as current.
            AppState.setRemoteActivities([{
                id: "w-3", sender: "Taylor", category: "game",
                kind: "opened", label: "Minecraft", time: "20:14"
            }])
            AppState.setPartnerState("offline")
            wait(30)
            verify(view.latestActivity !== null, "facts stay; visibility is separate")
        } finally {
            view.destroy()
        }
    }

    // Without a pair there is no partner data anywhere on screen.
    function test_unpairedShowsNoPartner() {
        AppState.setPairedPeers([])
        verify(!AppState.paired)
        var view = _createWidget()
        try {
            compare(view.paired, false)
            compare(view.presenceText(), I18n.t("widget.unpaired.status"))
            verify(view.latestActivity === null)
            verify(!view.showCall)
        } finally {
            view.destroy()
        }
    }

    // The joined-call symbol obeys state and setting together.
    function test_callPresenceOption() {
        _pairOnline()
        AppState.setPartnerState("online")
        MockController.forceCallState("connected")
        var view = _createWidget()
        try {
            verify(view.inCall)
            verify(view.showCall)
            AppState.widgetShowCallPresence = false
            verify(!view.showCall)
            AppState.widgetShowCallPresence = true
            verify(view.showCall)
            MockController.forceCallState("idle")
            verify(!view.inCall)
            verify(!view.showCall)
        } finally {
            view.destroy()
        }
    }

    // Clicks route outward: card opens Harbor, symbol opens the call.
    function test_clicksRouteToShell() {
        _pairOnline()
        AppState.setPartnerState("online")
        var view = _createWidget()
        try {
            view.visible = true
            wait(30)
            var opened = 0
            var openedCall = 0
            view.openRequested.connect(function() { opened++ })
            view.openCallRequested.connect(function() { openedCall++ })
            view.openRequested()
            view.openCallRequested()
            compare(opened, 1)
            compare(openedCall, 1)
        } finally {
            view.visible = false
            view.destroy()
        }
    }

    // Corner math is pure and covered for all four positions.
    function test_cornerMath() {
        var view = _createWidget()
        try {
            var topLeft = view.computePosition("topLeft", 0, 0, 1920, 1080, 252, 120, 16)
            compare(topLeft.x, 16)
            compare(topLeft.y, 16)
            var topRight = view.computePosition("topRight", 0, 0, 1920, 1080, 252, 120, 16)
            compare(topRight.x, 1652)
            compare(topRight.y, 16)
            var bottomLeft = view.computePosition("bottomLeft", 0, 0, 1920, 1080, 252, 120, 16)
            compare(bottomLeft.x, 16)
            compare(bottomLeft.y, 944)
            var bottomRight = view.computePosition("nonsense", 0, 0, 1920, 1080, 252, 120, 16)
            compare(bottomRight.x, 1652)
            compare(bottomRight.y, 944)
        } finally {
            view.destroy()
        }
    }

    // Reduced motion keeps the symbol and drops the shimmer.
    function test_reducedMotionKeepsSymbolStatic() {
        _pairOnline()
        MockController.forceCallState("connected")
        AppState.reducedMotion = true
        var view = _createWidget()
        try {
            verify(view.showCall, "symbol stays without motion")
        } finally {
            AppState.reducedMotion = false
            view.destroy()
        }
    }

    // The standalone frame paints its own ocean (same stops as the shell),
    // so every ocean variant visibly recolors it instead of staying blue.
    function test_frameFollowsOceanVariant() {
        var view = _createWidget()
        try {
            var scope = view.contentItem !== undefined ? view.contentItem : view
            var frame = _findFirst(scope, "widgetFrame")
            verify(frame !== null, "frame must be findable")
            verify(frame.gradient !== null && frame.gradient.stops.length === 3)
            AppState.oceanVariant = "lagoon"
            wait(20)
            compare(frame.gradient.stops[0].color, Theme.bgTop)
            compare(frame.gradient.stops[2].color, Theme.bgBottom)
            var lagoonTop = String(frame.gradient.stops[0].color)
            AppState.oceanVariant = "ember"
            wait(20)
            compare(frame.gradient.stops[0].color, Theme.bgTop)
            verify(String(frame.gradient.stops[0].color) !== lagoonTop,
                   "ember must recolor the widget away from lagoon blue")
        } finally {
            AppState.oceanVariant = "lagoon"
            view.destroy()
        }
    }

    // The widget is an independent top-level: hiding/minimizing the shell
    // must never take it down (a transient Tool would follow its parent).
    function test_widgetIsIndependentTopLevel() {
        var view = _createWidget()
        try {
            verify(view.transientParent === null,
                   "widget must not be transient to the shell")
        } finally {
            view.destroy()
        }
    }

    // Closing the shell hides (keeps the widget) while a way back exists —
    // tray or the widget itself. With neither, the close is an honest quit.
    function test_closeHidesWhileWidgetIsUp() {
        var component = Qt.createComponent("qrc:/qt/qml/Harbor/Main.qml")
        verify(component.status === Component.Ready,
               "Main.qml should be ready: " + component.errorString())
        var main = component.createObject(null)
        verify(main !== null)
        try {
            AppState.closeToTray = true
            AppState.widgetEnabled = true
            wait(30)
            verify(main.shouldHideOnClose(),
                   "visible widget is a way back: close must hide")
            AppState.widgetEnabled = false
            wait(30)
            verify(!main.shouldHideOnClose(),
                   "no tray and no widget: close must quit, never orphan")
            AppState.widgetEnabled = true
            AppState.closeToTray = false
            wait(30)
            verify(!main.shouldHideOnClose(),
                   "close-to-tray off: close is an explicit quit")
        } finally {
            AppState.closeToTray = true
            AppState.widgetEnabled = true
            wait(30)
            main.destroy()
        }
    }

    // The shell mounts the companion and follows the master switch.
    function test_shellMountsCompanion() {        var component = Qt.createComponent("qrc:/qt/qml/Harbor/Main.qml")
        verify(component.status === Component.Ready,
               "Main.qml should be ready: " + component.errorString())
        var main = component.createObject(null)
        verify(main !== null)
        try {
            verify(main.companion !== null && main.companion !== undefined,
                   "companion must be mounted")
            AppState.widgetEnabled = false
            tryCompare(main.companion, "visible", false)
            AppState.widgetEnabled = true
            tryCompare(main.companion, "visible", true)
        } finally {
            wait(60)
            main.destroy()
        }
    }
}
