import QtQuick
import QtTest
import Harbor 2.0

// Full-shell smoke matrix: every page loads in both locales at every
// supported resolution, and themes, contrast, motion and page states
// compose without breaking the shell.
TestCase {
    id: testCase
    name: "ShellSmoke"
    width: 1280
    height: 800
    visible: true

    readonly property var resolutions: [
        [1024, 640], [1280, 720], [1366, 768],
        [1600, 900], [1920, 1080], [2560, 1440]
    ]
    readonly property var views: ["home", "call", "chat", "activity", "settings"]
    readonly property var locales: ["en", "pt-BR"]

    function init() {
        MockController.resetSession()
    }

    function cleanup() {
        MockController.resetSession()
    }

    function _createMain(width, height) {
        var component = Qt.createComponent("qrc:/qt/qml/Harbor/Main.qml")
        verify(component.status === Component.Ready,
               "Main.qml should be ready: " + component.errorString())
        var main = component.createObject(null, { width: width, height: height })
        verify(main !== null)
        return main
    }

    function test_pagesRenderAcrossResolutionsAndLocales() {
        for (var li = 0; li < locales.length; li++) {
            AppState.locale = locales[li]
            for (var ri = 0; ri < resolutions.length; ri++) {
                var main = _createMain(resolutions[ri][0], resolutions[ri][1])
                try {
                    for (var vi = 0; vi < views.length; vi++) {
                        AppState.navigate(views[vi])
                        wait(20)
                        compare(AppState.currentView, views[vi])
                        verify(main.contentItem !== null)
                        verify(main.contentItem.width > 0)
                    }
                } finally {
                    wait(20)
                    main.destroy()
                }
            }
        }
    }

    // The pairing gate is authoritative core state, never a client guess.
    // Without a live core there is no resolved snapshot, so the shell must
    // stay on Home and never invent a first-run onboarding by itself — even
    // when some unpaired state arrives from elsewhere.
    function test_firstRunGateStaysSilentWithoutLiveCore() {
        AppState.setPairedPeers([])
        verify(AppState.pairedPeersResolved)
        verify(!AppState.paired)
        var main = _createMain(1400, 880)
        try {
            wait(40)
            verify(!AppState.onboardingVisible,
                   "onboarding must not auto-open without a live core")
            verify(!main.pairingGateShown,
                   "the gate decision must wait for the core")
            compare(AppState.currentView, "home")
        } finally {
            wait(20)
            main.destroy()
            MockController.resetSession()
        }
    }

    function test_themeContrastMotionAndPageStates() {
        var main = _createMain(1600, 900)
        try {
            var modes = ["dark", "light"]
            for (var mi = 0; mi < modes.length; mi++) {
                AppState.appearanceMode = modes[mi]
                compare(Theme.mode, modes[mi])
                AppState.higherContrast = !AppState.higherContrast
                AppState.reducedMotion = !AppState.reducedMotion

                var states = ["content", "loading", "empty", "error"]
                for (var si = 0; si < states.length; si++) {
                    MockController.setPageState("home", states[si])
                    wait(20)
                    compare(AppState.pageState("home"), states[si])
                }
            }

            // Overlays open and close on top of the composed shell.
            MockController.openPairing("qr")
            verify(AppState.pairingVisible)
            main.handleEscape()
            verify(!AppState.pairingVisible)

            AppState.notificationsVisible = true
            wait(20)
            AppState.notificationsVisible = false
        } finally {
            wait(20)
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

    // The sidebar's expand button outranks the narrow-window auto collapse:
    // pressed below the auto-collapse width it must open the rail anyway,
    // and the automatic rule re-arms once the window is wide again.
    function test_sidebarExpandOverridesAutoCollapse() {
        var main = _createMain(1600, 900)
        try {
            var sidebar = _findFirst(main.contentItem, "shellSidebar")
            verify(sidebar !== null)
            verify(!sidebar.collapsed, "wide window keeps the rail open")

            // Below the auto-collapse width the rail folds itself.
            main.width = 1000
            tryVerify(function () { return sidebar.collapsed })

            // The explicit expand request wins even on a narrow window.
            var expandButton = _findFirst(main.contentItem, "sidebarExpandButton")
            verify(expandButton !== null)
            mouseClick(expandButton, expandButton.width / 2, expandButton.height / 2)
            tryVerify(function () { return !sidebar.collapsed })

            // Crossing back above the width re-arms the automatic rule.
            main.width = 1600
            tryVerify(function () { return !sidebar.autoCollapsed })
            verify(!sidebar.userExpanded, "the manual override must not outlive the wide window")
            main.width = 1000
            tryVerify(function () { return sidebar.collapsed })
        } finally {
            wait(20)
            main.destroy()
        }
    }

    // Reduced motion disables only decorative animation. Timed, functional
    // mock transitions must still finish so the prototype stays operable.
    function test_reducedMotionStopsDecorationKeepsFunction() {
        var main = _createMain(1600, 900)
        try {
            var particles = _findFirst(main.contentItem, "shellParticles")
            verify(particles !== null, "Shell particles should be reachable")

            AppState.reducedMotion = true
            compare(Theme.duration(150), 0)
            tryCompare(particles, "running", false)

            MockController.setConnectionScenario("connecting", true)
            tryCompare(AppState, "connectionState", "connected")
        } finally {
            AppState.reducedMotion = false
            MockController.resetSession()
            wait(20)
            main.destroy()
        }
    }
}
