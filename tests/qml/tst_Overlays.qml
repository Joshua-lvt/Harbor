import QtQuick
import QtTest
import Harbor 2.0

TestCase {
    name: "Overlays"

    function init() {
        MockController.resetSession()
        AppState.reducedMotion = true
        // The desktop companion is a separate top-level window: offscreen it
        // takes window activation from the shell under test. These cases own
        // main-window overlays, so the orthogonal surface stays off here
        // (its own suite covers it with the shell mounted).
        AppState.widgetEnabled = false
    }

    function cleanup() {
        AppState.reducedMotion = false
        MockController.resetSession()
    }

    function test_pairingVisibilityIsSessionState() {
        MockController.openPairing("choice")
        verify(AppState.pairingVisible)
        compare(MockController.pairingMode, "choice")
        MockController.closePairing()
        verify(!AppState.pairingVisible)
    }

    function _createMain() {
        var component = Qt.createComponent("qrc:/qt/qml/Harbor/Main.qml")
        verify(component.status === Component.Ready,
               "Main.qml should be ready: " + component.errorString())
        var main = component.createObject(null)
        verify(main !== null)
        main.requestActivate()
        wait(30)
        return main
    }

    function test_shellEnforcesOverlayExclusivity() {
        var main = _createMain()
        try {
            AppState.notificationsVisible = true
            verify(AppState.notificationsVisible)
            MockController.openPairing()
            verify(AppState.pairingVisible)
            verify(!AppState.notificationsVisible,
                   "opening pairing must close the notification center")

            AppState.onboardingVisible = true
            verify(AppState.onboardingVisible)
            verify(!AppState.pairingVisible,
                   "opening onboarding must close pairing")
        } finally {
            wait(60)
            main.destroy()
        }
    }

    function test_escapeRoutesPairingThroughController() {
        var main = _createMain()
        try {
            MockController.openPairing("qr")
            compare(MockController.pairingMode, "qr")
            main.handleEscape()
            verify(!AppState.pairingVisible)
            compare(MockController.pairingMode, "choice")

            MockController.showIncomingRequest()
            verify(AppState.pairingVisible)
            compare(MockController.pairingMode, "incoming")
            main.handleEscape()
            verify(!AppState.pairingVisible)
            compare(MockController.pairingMode, "choice")
        } finally {
            wait(60)
            main.destroy()
        }
    }

    function test_focusRestoresOnlyAfterOverlaysFullyClose() {
        var main = _createMain()
        try {
            verify(main.overlaysFullyClosed)
            var focusBefore = main.activeFocusItem

            MockController.openPairing()
            verify(main.anyOverlayActive)
            verify(!main.overlaysFullyClosed)
            verify(main.focusBeforeOverlay !== null
                   || focusBefore === null,
                   "launcher focus is registered when the overlay opens")

            MockController.closePairing()
            tryVerify(function() { return main.overlaysFullyClosed })
            verify(!main.anyOverlayActive)
            verify(main.focusBeforeOverlay === null,
                   "stored launcher focus is released after the fade")
            if (focusBefore !== null)
                compare(main.activeFocusItem, focusBefore,
                        "focus returns to the launcher item")
        } finally {
            wait(60)
            main.destroy()
        }
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

    // Escape reaches the shell through the keyboard alone: the focused
    // surface routes it to handleEscape, which closes whatever layer is
    // open, one at a time.
    function test_escapeKeyClosesActiveOverlay() {
        var main = _createMain()
        try {
            var surface = _findFirst(main.contentItem, "shellSurface")
            verify(surface !== null, "shell surface should be reachable by objectName")
            surface.forceActiveFocus()

            MockController.openPairing()
            verify(AppState.pairingVisible)
            var pairingLayer = _findFirst(main.contentItem, "pairingOverlayLayer")
            verify(pairingLayer !== null)
            tryVerify(function() { return pairingLayer.activeFocus })
            KeyTest.click(pairingLayer, Qt.Key_Escape)
            tryVerify(function() { return !AppState.pairingVisible })

            AppState.notificationsVisible = true
            verify(AppState.notificationsVisible)
            var notificationsLayer = _findFirst(main.contentItem, "notificationsOverlayLayer")
            verify(notificationsLayer !== null)
            tryVerify(function() { return notificationsLayer.activeFocus })
            KeyTest.click(notificationsLayer, Qt.Key_Escape)
            tryVerify(function() { return !AppState.notificationsVisible })

            AppState.onboardingVisible = true
            verify(AppState.onboardingVisible)
            var onboardingLayer = _findFirst(main.contentItem, "onboardingOverlayLayer")
            verify(onboardingLayer !== null)
            tryVerify(function() { return onboardingLayer.activeFocus })
            KeyTest.click(onboardingLayer, Qt.Key_Escape)
            tryVerify(function() { return !AppState.onboardingVisible })
        } finally {
            wait(60)
            main.destroy()
        }
    }
}
