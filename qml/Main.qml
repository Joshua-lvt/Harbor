pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Harbor 2.0

ApplicationWindow {
    id: window

    width: 1024
    height: 640
    minimumWidth: Theme.shellMinimumWidth
    minimumHeight: Theme.shellMinimumHeight
    visible: true
    title: I18n.t("shell.windowTitle")
    color: "transparent"
    flags: Qt.Window | Qt.FramelessWindowHint

    readonly property bool isMaximized: visibility === Window.Maximized

    // Breakpoints describe the space the page actually owns (window minus
    // title bar, sidebar, and status bar), never the raw window size.
    readonly property string contentBreakpoint: {
        var pageWidth = contentArea.width
        if (pageWidth < Theme.breakpointCompact) return "compact"
        if (pageWidth < Theme.breakpointMedium) return "medium"
        if (pageWidth < Theme.breakpointWide) return "wide"
        return "ultrawide"
    }

    // One scrim, one focus bookkeeper, one Escape authority. Overlay visibility
    // itself lives in AppState so the shell, shortcuts, and the mock controller
    // always agree.
    readonly property bool anyOverlayActive: AppState.notificationsVisible
        || AppState.pairingVisible
        || AppState.onboardingVisible || AppState.trayVisible
        || (AppState.devPanelVisible && !hasCoreFacade)

    // An overlay is only gone once its exit fade finished and the lazy tree
    // unmounted. Input blocking and focus restoration key off this instead of
    // the raw flags, so nothing behind a still-fading layer can react early.
    readonly property bool overlaysFullyClosed: !anyOverlayActive
        && !notificationsLoader.active && !pairingLoader.active && !onboardingLoader.active
        && !trayPreviewLoader.active && !developerPanelLoader.active

    property Item focusBeforeOverlay: null

    // The real core only exists in the application (main.cpp installs the
    // HarborCore context property); QML tests run without it, keeping the
    // deterministic mock provider the single source of truth there.
    // qmllint disable unqualified
    readonly property bool coreReady: typeof HarborCore !== "undefined" && HarborCore.coreReady
    readonly property bool hasCoreFacade: typeof HarborCore !== "undefined"
    readonly property bool waitingForLiveSnapshot: hasCoreFacade
        && (!coreReady || (!AppState.pairedPeersResolved && !AppState.pairingBypassed))
    readonly property bool coreUnavailable: hasCoreFacade && !coreReady
        && (HarborCore.coreState === "failed" || HarborCore.coreState === "reconnecting")
    // qmllint enable unqualified
    // Real system-tray adapter (absent in QML tests, where the mock provider
    // and this shell's preview flyout remain the deterministic truth).
    // qmllint disable unqualified
    readonly property bool systemTrayAvailable: typeof HarborTray !== "undefined" && HarborTray.available
    // qmllint enable unqualified

    // OS color scheme for the "system" appearance mode. The binding tracks
    // live switches; Theme re-resolves only while "system" is selected.
    Binding {
        target: Theme
        property: "systemDark"
        value: Application.styleHints.colorScheme === Qt.Dark
    }

    // Core ⇄ AppState bridge. Inert until the real core reports ready; with
    // no facade the deterministic mock provider stays the single QML truth.
    // qmllint disable unqualified
    HarborSettingsBridge {
        facade: window.coreReady ? HarborCore : null
    }
    // qmllint enable unqualified

    // Direct chat/transfer facts mirror the metadata-only core snapshot for
    // future existing-surface controls; no QML file or network API is used.
    // qmllint disable unqualified
    HarborDirectBridge {
        facade: window.coreReady ? HarborCore : null
    }
    // qmllint enable unqualified

    // Session-wide call mirroring: an inbound ring must reach the
    // notification service (and the sidebar) no matter which page is up,
    // not only while a call surface is mounted. All state writes are
    // equality-guarded, so this instance and the views' own providers
    // converge without churn.
    // qmllint disable unqualified
    HarborCallBridge {
        facade: window.coreReady ? HarborCore : null
    }
    // qmllint enable unqualified

    // Durable pairing facts: the control plane's relationship snapshot gates
    // peer-only surfaces. QML mirrors it; it never invents a paired flag.
    // qmllint disable unqualified
    HarborContactsBridge {
        facade: window.coreReady ? HarborCore : null
    }
    // qmllint enable unqualified

    // Partner's phone facts: the peer's shared MobileStatus plus display-only
    // phone notifications. Session-only mirrors; a side that stops sharing
    // returns to null instead of lingering.
    // qmllint disable unqualified
    HarborMobileBridge {
        facade: window.coreReady ? HarborCore : null
    }
    // qmllint enable unqualified

    // Desktop notifications: real core events, durable settings, localized
    // texts. The in-app widget is the primary surface; the native adapter
    // only posts while the shell cannot show its own card.
    HarborNotificationsBridge {
        id: notificationsBridge

        // qmllint disable unqualified
        facade: window.coreReady ? HarborCore : null
        // qmllint enable unqualified
        notifier: typeof HarborNotifications !== "undefined"
                  ? HarborNotifications : null
        // The chat is only "focused" while this shell can actually show it.
        windowActive: window.active && window.visibility !== Window.Minimized
    }

    // The temporary notification stack: its own frameless window pinned to
    // the top-right of this shell's screen, alive only while cards queue.
    HarborNotificationWidget {
        // qmllint disable unqualified
        screen: window.screen
        // qmllint enable unqualified
        cards: notificationsBridge.queue
        onCardActivated: function (kind) {
            if (kind === "message")
                AppState.navigate("chat")
            window.show()
            window.raise()
            window.requestActivate()
        }
        // qmllint disable unqualified
        onCardDismissed: function (id) { notificationsBridge.dismiss(id) }
        // qmllint enable unqualified
    }

    // First-run gate: an unpaired device with a live core is walked through
    // onboarding — which offers the real pairing flow — exactly once per
    // session. It waits for the snapshot to resolve, and never fires without
    // the core (tests and previews keep their deterministic fixtures).
    property bool pairingGateShown: false

    function _runPairingGate() {
        if (pairingGateShown)
            return
        if (!coreReady || !AppState.pairedPeersResolved)
            return
        pairingGateShown = true
        if (!AppState.paired && !AppState.pairingBypassed)
            AppState.onboardingVisible = true
    }

    Connections {
        target: AppState

        function onPairedPeersChanged() {
            window._runPairingGate()
        }

        function onPairedPeersResolvedChanged() {
            window._runPairingGate()
        }
    }

    Connections {
        // coreReady belongs to the shell window, not AppState. Keeping this
        // handler on the matching target avoids a silent no-op when the real
        // facade becomes ready after the initial QML construction.
        target: window

        function onCoreReadyChanged() {
            window._runPairingGate()
        }
    }

    // Close-to-tray keeps the session alive behind a visible affordance:
    // the native tray icon, or the companion widget (whose click reopens
    // the shell). Hiding with neither would orphan the app invisibly, so
    // that case stays an honest quit. Explicit Quit always ends everything,
    // widget included.
    function shouldHideOnClose() {
        if (!AppState.closeToTray)
            return false
        if (window.systemTrayAvailable)
            return true
        return window.companion !== null && window.companion !== undefined
               && window.companion.visible
    }

    function requestClose() {
        // Production with a live affordance hides to it; without one the
        // close is an honest quit via onClosing. The in-app preview flyout
        // is a test/preview affordance only and never stands in for the tray.
        if (hasCoreFacade && (systemTrayAvailable
                              || (companionWidget && companionWidget.visible))) {
            window.hide()
            return
        }
        AppState.trayVisible = true
    }

    // Window close (WM button, Alt+F4) follows the stored close-to-tray
    // preference — hiding while a tray icon or the companion widget gives a
    // way back. With neither, hiding would orphan the app invisibly, so the
    // close is an honest quit: aboutToQuit shuts the core, its call, its
    // share and the media worker down.
    onClosing: close => {
        close.accepted = !window.shouldHideOnClose()
        if (!close.accepted)
            window.hide()
    }

    // qmllint disable unqualified
    Connections {
        target: typeof HarborTray !== "undefined" ? HarborTray : null

        function onOpenRequested() {
            window.showNormal()
            window.raise()
            window.requestActivate()
        }
    }
    // qmllint enable unqualified

    // The tray icon lives for as long as the application does; availability
    // is decided by the session, never fabricated.
    Component.onCompleted: {
        // qmllint disable unqualified
        if (typeof HarborTray !== "undefined")
            HarborTray.setActive(true)
        if (typeof HarborUpdater !== "undefined") {
            HarborUpdater.setCallActive(AppState.callState === "connected")
            HarborUpdater.checkForUpdates()
        }
        // qmllint enable unqualified
        window._runPairingGate()
    }

    // Priority: modal popups close themselves through their close policy
    // (they live in the overlay and never reach this handler), so this covers
    // the shell layers next in line: pairing → onboarding → notifications →
    // developer panel → the page itself.
    function handleEscape() {
        if (AppState.pairingVisible) {
            // PairingView owns the provider reset so Escape cannot leave a live
            // request in the core. The fallback only covers the lazy teardown.
            // qmllint disable missing-property
            if (pairingLoader.item && pairingLoader.item._requestClose)
                pairingLoader.item._requestClose()
            else
                AppState.pairingVisible = false
            // qmllint enable missing-property
            return
        }
        if (AppState.onboardingVisible) {
            AppState.onboardingVisible = false
            // Dismissing the walkthrough is an explicit "not now": the
            // session continues unpaired, with peer-only surfaces saying so.
            if (!AppState.paired)
                AppState.continueWithoutPairing()
            return
        }
        if (AppState.notificationsVisible) {
            AppState.notificationsVisible = false
            return
        }
        if (AppState.devPanelVisible) {
            AppState.devPanelVisible = false
            return
        }
        // Views own a root-level escapeRequested() signal, but a Loader cannot
        // know the concrete type, so this stays a feature-checked dynamic call.
        // qmllint disable missing-property
        if (viewLoader.item && viewLoader.item.escapeRequested)
            viewLoader.item.escapeRequested()
        // qmllint enable missing-property
    }

    // Shell overlays are exclusive and the most recently opened layer wins:
    // opening one closes the others so layers never stack ambiguously. Popups
    // (tray, developer panel) may coexist briefly with a closing layer because
    // they own their own transitions.
    function _enforceOverlayExclusivity(opened) {
        if (!opened)
            return
        if (opened !== "pairing" && AppState.pairingVisible) {
            // PairingView resets its active provider before hiding the layer.
            // qmllint disable missing-property
            if (pairingLoader.item && pairingLoader.item._requestClose)
                pairingLoader.item._requestClose()
            else
                AppState.pairingVisible = false
            // qmllint enable missing-property
        }
        if (opened !== "onboarding")
            AppState.onboardingVisible = false
        if (opened !== "notifications")
            AppState.notificationsVisible = false
    }

    onAnyOverlayActiveChanged: {
        if (anyOverlayActive && !focusBeforeOverlay && window.activeFocusItem)
            focusBeforeOverlay = window.activeFocusItem
    }

    onOverlaysFullyClosedChanged: {
        if (overlaysFullyClosed && focusBeforeOverlay) {
            focusBeforeOverlay.forceActiveFocus()
            focusBeforeOverlay = null
        }
    }

    Rectangle {
        id: windowSurface

        objectName: "shellSurface"
        anchors.fill: parent
        radius: window.isMaximized ? 0 : Theme.radiusLarge
        clip: true
        border.width: window.isMaximized ? 0 : 1
        border.color: Theme.surfaceBorder

        // ApplicationWindow is not an Item, so shell-level key handling
        // attaches to the surface — the common ancestor of every layer and
        // the sidebar. Modal popups live in the overlay branch and never
        // reach it, which keeps their close policies first in the Escape
        // priority.
        Keys.onEscapePressed: window.handleEscape()

        gradient: Gradient {
            GradientStop { position: 0.0; color: Theme.bgTop }
            GradientStop { position: 0.48; color: Theme.bgMid }
            GradientStop { position: 1.0; color: Theme.bgBottom }
        }

        HarborParticles {
            objectName: "shellParticles"
            anchors.fill: parent
            // Particle density follows animation strength; reduced motion
            // or the particles switch stops them outright.
            count: Math.max(4, Math.round((window.width < Theme.breakpointMedium ? 13 : 22)
                                          * (0.35 + 0.65 * AppState.animationIntensity)))
            opacityScale: Theme.dark ? 0.23 : 0.16
            running: AppState.backgroundAnimation && AppState.particlesEnabled
                     && !AppState.reducedMotion
        }

        // Soft atmospheric light pools keep the background aquatic without
        // competing with content.
        Rectangle {
            width: Math.min(620, parent.width * 0.52)
            height: width
            radius: width / 2
            x: parent.width * 0.34
            y: -height * 0.7
            color: Theme.dark ? "#1522B8CF" : "#32FFFFFF"
            Accessible.ignored: true
        }

        ColumnLayout {
            anchors.fill: parent
            spacing: 0

            HarborTitleBar {
                id: titleBar

                Layout.fillWidth: true
                Layout.preferredHeight: implicitHeight
                targetWindow: window
                maximized: window.isMaximized
                onMinimizeRequested: window.showMinimized()
                onMaximizeRequested: window.showMaximized()
                onRestoreRequested: window.showNormal()
                onCloseRequested: window.requestClose()
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 1
                color: Theme.surfaceBorder
            }

            RowLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: 0

                HarborSidebar {
                    id: sidebar

                    objectName: "shellSidebar"
                    Layout.fillHeight: true
                    Layout.preferredWidth: implicitWidth
                    currentView: AppState.currentView
                    onNavigationRequested: function(view) { AppState.navigate(view) }
                    onProfileRequested: AppState.navigate("profile")
                }

                Rectangle {
                    Layout.preferredWidth: 1
                    Layout.fillHeight: true
                    color: Theme.surfaceBorder
                }

                Item {
                    id: contentArea

                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true

                    // The page channel keeps the reading width bounded on
                    // ultrawide screens; below 1280x720 the page scrolls
                    // internally through HarborPage instead of growing the
                    // shell.
                    Item {
                        id: pageChannel

                        anchors.horizontalCenter: parent.horizontalCenter
                        width: Math.min(parent.width,
                                        Theme.maxPageWidth + Theme.sp5 * 2)
                        height: parent.height

                        Loader {
                            id: viewLoader

                            anchors.fill: parent
                            asynchronous: true
                            sourceComponent: {
                                switch (AppState.currentView) {
                                case "call": return callComponent
                                case "chat": return chatComponent
                                case "activity": return activityComponent
                                case "mobile": return mobileComponent
                                case "settings": return settingsComponent
                                case "profile": return profileComponent
                                default: return homeComponent
                                }
                            }

                            onLoaded: {
                                item.opacity = 0
                                viewEnter.restart()
                            }

                            NumberAnimation {
                                id: viewEnter

                                target: viewLoader.item
                                property: "opacity"
                                from: 0
                                to: 1
                                duration: Theme.duration(Theme.motionNormal)
                                easing.type: Theme.animEasing
                            }
                        }

                        Component { id: homeComponent; HomeView {} }
                        Component { id: callComponent; CallView {} }
                        Component { id: chatComponent; ChatView {} }
                        Component { id: activityComponent; ActivityView {} }
                        Component { id: mobileComponent; MobileView {} }
                        // Network and Devices remain compiled for diagnostics and tests,
                        // but are intentionally not reachable from the product shell.
                        Component { id: profileComponent; ProfileView {} }
                        Component { id: settingsComponent; SettingsView {} }
                    }
                }
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: Theme.shellStatusBarHeight
                color: Theme.surface

                RowLayout {
                    anchors.fill: parent
                    anchors.leftMargin: Theme.sp4
                    anchors.rightMargin: Theme.sp2
                    spacing: Theme.sp2

                    // Shape and label carry the state alongside color: filled
                    // dot for a live link, hollow ring while negotiating,
                    // muted dot when offline.
                    Rectangle {
                        implicitWidth: 8
                        implicitHeight: 8
                        radius: 4
                        color: AppState.connectionState === "connected" ? Theme.online
                              : AppState.connectionState === "reconnecting" ? Theme.warning
                              : AppState.connectionState === "connecting" ? Theme.accent
                              : Theme.offline
                        border.width: AppState.connectionState === "connecting" ? 1 : 0
                        border.color: Theme.accent

                        SequentialAnimation on opacity {
                            running: AppState.connectionState === "reconnecting" && !AppState.reducedMotion
                            loops: Animation.Infinite
                            NumberAnimation { to: 0.35; duration: 650 }
                            NumberAnimation { to: 1; duration: 650 }
                        }
                    }

                    Text {
                        text: AppState.connectionState === "connected"
                              ? (AppState.partnerName.length > 0
                                 ? I18n.t("shell.status.connectedTo", { name: AppState.partnerName })
                                 : I18n.t("common.status.connected"))
                              : AppState.connectionState === "reconnecting"
                                ? I18n.t("shell.status.reconnecting")
                                : AppState.connectionState === "connecting"
                                  ? I18n.t("shell.status.opening")
                                  : I18n.t("shell.status.notConnected")
                        color: Theme.textSecondary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontTiny
                    }

                    Item { Layout.fillWidth: true }

                    Item {
                        implicitWidth: notificationsButton.width
                        implicitHeight: Theme.shellStatusBarHeight

                        HarborIconButton {
                            id: notificationsButton

                            anchors.centerIn: parent
                            visible: !window.hasCoreFacade
                            iconName: "activity"
                            buttonSize: 30
                            checked: AppState.notificationsVisible
                            accessibleName: AppState.unreadCount > 0
                                ? I18n.t("a11y.notificationCount", { count: AppState.unreadCount })
                                  + " — " + I18n.t("shell.notifications.open")
                                : I18n.t("shell.notifications.open")
                            onClicked: AppState.notificationsVisible = !AppState.notificationsVisible
                        }

                        Rectangle {
                            visible: !window.hasCoreFacade && AppState.unreadCount > 0
                            width: unreadCountLabel.implicitWidth + 10
                            height: 16
                            radius: 8
                            anchors.right: parent.right
                            anchors.rightMargin: 2
                            anchors.top: parent.top
                            anchors.topMargin: 3
                            color: Theme.actionDanger
                            border.width: 1
                            border.color: Theme.surfaceOverlay

                            Text {
                                id: unreadCountLabel

                                anchors.centerIn: parent
                                text: AppState.unreadCount
                                color: Theme.actionDangerText
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontTiny
                                font.weight: Font.DemiBold
                            }
                        }
                    }

                    Text {
                        visible: AppState.connectionState !== "connected"
                        text: AppState.connectionState === "reconnecting"
                              ? I18n.t("shell.status.reconnecting")
                              : I18n.t("shell.status.notConnected")
                        color: Theme.textFaint
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontTiny
                    }
                }
            }
        }

        // Never expose the QML fixture state while the supervised core is
        // starting or its first authoritative snapshot is still pending.
        Item {
            id: liveStartupLayer

            anchors.fill: parent
            z: 35
            visible: window.waitingForLiveSnapshot
            Rectangle {
                anchors.fill: parent
                color: Theme.surface
            }
            HarborEmptyState {
                anchors.centerIn: parent
                width: Math.min(parent.width - Theme.sp8, 520)
                iconName: window.coreUnavailable ? "alert" : "app"
                title: I18n.t(window.coreUnavailable
                               ? "state.error.title" : "state.loading.title")
                description: I18n.t(window.coreUnavailable
                                     ? "error.core.unavailable"
                                     : "state.loading.description")
                actionText: window.coreUnavailable
                            ? I18n.t("common.actions.retry") : ""
                onActionTriggered: {
                    if (window.coreUnavailable)
                        HarborCore.retryCore()
                }
            }
        }

        // Shared scrim for every overlay. Modal popups keep their own Overlay.modal
        // transparent so the dimming always comes from this single source. It stays
        // visible through exit fades until overlaysFullyClosed flips.
        Rectangle {
            anchors.fill: parent
            z: 40
            visible: !window.overlaysFullyClosed
            color: Theme.surfaceScrim
            opacity: window.anyOverlayActive ? 1 : 0
            Behavior on opacity {
                NumberAnimation { duration: Theme.duration(Theme.motionFast) }
            }

            // Absorbs stray clicks so nothing behind an overlay reacts.
            MouseArea { anchors.fill: parent }
        }

        // Notification center slides above the application without changing pages.
        Item {
            id: notificationsLayer

            anchors.fill: parent
            z: 50
            objectName: "notificationsOverlayLayer"
            onVisibleChanged: {
                // Run after AppState's overlay-change handler stores the
                // launcher's focus, then establish the modal keyboard scope.
                if (visible && AppState.notificationsVisible)
                    Qt.callLater(() => {
                        if (visible && AppState.notificationsVisible)
                            forceActiveFocus()
                    })
            }
            Keys.onEscapePressed: event => {
                window.handleEscape()
                event.accepted = true
            }
            visible: opacity > 0
            opacity: AppState.notificationsVisible ? 1 : 0
            Behavior on opacity {
                NumberAnimation { duration: Theme.duration(Theme.motionNormal) }
            }

            Loader {
                id: notificationsLoader

                active: AppState.notificationsVisible || notificationsLayer.opacity > 0.01
                anchors.fill: parent

                sourceComponent: Component {
                    Item {
                        anchors.fill: parent

                        // Clicking the dimmed area dismisses the center.
                        MouseArea {
                            anchors.fill: parent
                            onClicked: AppState.notificationsVisible = false
                        }

                        Rectangle {
                            id: notificationPanel

                            anchors.top: parent.top
                            anchors.right: parent.right
                            anchors.bottom: parent.bottom
                            width: Math.min(520, parent.width * 0.92)
                            color: Theme.surfaceOverlay
                            border.width: 1
                            border.color: Theme.surfaceBorder

                            MouseArea { anchors.fill: parent }

                            NotificationsView {
                                anchors.fill: parent
                                onClosed: AppState.notificationsVisible = false
                            }
                        }
                    }
                }
            }
        }

        // Pairing and onboarding are complete mock flows kept separate from
        // navigation; both stay lazy so a session that never opens them never
        // pays for their trees.
        Item {
            id: pairingLayer

            anchors.fill: parent
            z: 70
            objectName: "pairingOverlayLayer"
            onVisibleChanged: {
                // Run after AppState's overlay-change handler stores the
                // launcher's focus, then establish the modal keyboard scope.
                if (visible && AppState.pairingVisible)
                    Qt.callLater(() => {
                        if (visible && AppState.pairingVisible)
                            forceActiveFocus()
                    })
            }
            Keys.onEscapePressed: event => {
                window.handleEscape()
                event.accepted = true
            }
            visible: opacity > 0
            opacity: AppState.pairingVisible ? 1 : 0
            Behavior on opacity {
                NumberAnimation { duration: Theme.duration(Theme.motionNormal) }
            }

            Loader {
                id: pairingLoader

                active: AppState.pairingVisible || pairingLayer.opacity > 0.01
                anchors.fill: parent

                sourceComponent: Component {
                    PairingView {
                        anchors.fill: parent
                        anchors.margins: Math.max(Theme.sp3,
                            Math.min(Theme.sp6, parent.width * 0.035))
                        onClosed: AppState.pairingVisible = false
                    }
                }
            }
        }

        Item {
            id: onboardingLayer

            anchors.fill: parent
            z: 60
            objectName: "onboardingOverlayLayer"
            onVisibleChanged: {
                // Run after AppState's overlay-change handler stores the
                // launcher's focus, then establish the modal keyboard scope.
                if (visible && AppState.onboardingVisible)
                    Qt.callLater(() => {
                        if (visible && AppState.onboardingVisible)
                            forceActiveFocus()
                    })
            }
            Keys.onEscapePressed: event => {
                window.handleEscape()
                event.accepted = true
            }
            visible: opacity > 0
            opacity: AppState.onboardingVisible ? 1 : 0
            Behavior on opacity {
                NumberAnimation { duration: Theme.duration(Theme.motionNormal) }
            }

            Loader {
                id: onboardingLoader

                active: AppState.onboardingVisible || onboardingLayer.opacity > 0.01
                anchors.fill: parent

                sourceComponent: Component {
                    OnboardingView {
                        anchors.fill: parent
                        anchors.margins: Theme.sp4
                    }
                }
            }
        }
    }

    // Modal popups: Qt keeps them in the overlay (above every layer here) and
    // their close policies resolve Escape before the shell handler runs.
    Loader {
        id: trayPreviewLoader

        // Typed view of `item` so the popup API is checkable; null while
        // the loader is inactive.
        readonly property HarborTrayPreview trayPopup: item as HarborTrayPreview

        // Stays mounted while the popup's exit transition is still running so
        // closing never cuts the fade short.
        active: AppState.trayVisible || (trayPopup !== null && trayPopup.visible)
        sourceComponent: Component {
            HarborTrayPreview {
                onClosed: AppState.trayVisible = false
                onOpenRequested: window.showNormal()
                onMinimizeRequested: window.showMinimized()
                onQuitRequested: Qt.quit()
            }
        }
        onLoaded: (item as HarborTrayPreview).open()
    }

    // Desktop companion: a separate small window mirroring partner
    // presence. Same process, same state, no polling; closing the main
    // window to the tray (or to this widget) keeps it, quitting ends it
    // with everything else.
    readonly property HarborWidget companion: companionWidget

    HarborWidget {
        id: companionWidget

        objectName: "companionWidget"
        visible: AppState.widgetEnabled
        onOpenRequested: {
            AppState.navigate("home")
            window.showNormal()
            window.raise()
            window.requestActivate()
        }
        onOpenCallRequested: {
            AppState.navigate("call")
            window.showNormal()
            window.raise()
            window.requestActivate()
        }
    }

    Loader {
        id: developerPanelLoader

        readonly property HarborDeveloperPanel devPanel: item as HarborDeveloperPanel

        // Test/preview only: never mount mock controls in production.
        active: !window.hasCoreFacade
                && (AppState.devPanelVisible || (devPanel !== null && devPanel.visible))
        sourceComponent: Component {
            HarborDeveloperPanel {
                onClosed: AppState.devPanelVisible = false
                onOnboardingRequested: {
                    AppState.devPanelVisible = false
                    AppState.onboardingVisible = true
                }
                onPairingRequested: {
                    AppState.devPanelVisible = false
                    AppState.openPairing()
                }
                onTrayPreviewRequested: {
                    AppState.devPanelVisible = false
                    AppState.trayVisible = true
                }
            }
        }
        onLoaded: (item as HarborDeveloperPanel).open()
    }

    // Popups own their visibility; AppState flags stay authoritative for the
    // scrim and shortcuts, so both directions are mirrored back. The same
    // connection point enforces shell-overlay exclusivity.
    Connections {
        target: AppState

        function onPairingVisibleChanged() {
            window._enforceOverlayExclusivity(AppState.pairingVisible ? "pairing" : null)
        }
        function onOnboardingVisibleChanged() {
            window._enforceOverlayExclusivity(AppState.onboardingVisible ? "onboarding" : null)
        }

        function onNotificationsVisibleChanged() {
            window._enforceOverlayExclusivity(AppState.notificationsVisible ? "notifications" : null)
        }

        function onTrayVisibleChanged() {
            if (!AppState.trayVisible && trayPreviewLoader.trayPopup
                    && trayPreviewLoader.trayPopup.opened)
                trayPreviewLoader.trayPopup.close()
        }

        function onDevPanelVisibleChanged() {
            if (!AppState.devPanelVisible && developerPanelLoader.devPanel
                    && developerPanelLoader.devPanel.opened)
                developerPanelLoader.devPanel.close()
        }

        // Restarts never drop media: the updater holds a ready update while
        // a call is active and applies it the moment the call ends.
        function onCallStateChanged() {
            // qmllint disable unqualified
            if (typeof HarborUpdater !== "undefined")
                HarborUpdater.setCallActive(AppState.callState === "connected")
            // qmllint enable unqualified
        }
    }

    HarborToastHost {
        parent: Overlay.overlay
        anchors.fill: parent
        z: 1000
        mockProviderEnabled: !window.hasCoreFacade
    }

    // Mandatory updates sit above everything, including toasts: while one
    // is discovered there is no interaction with anything behind it.
    HarborUpdateOverlay {
        parent: Overlay.overlay
        anchors.fill: parent
        z: 1100
        // qmllint disable unqualified
        updater: typeof HarborUpdater !== "undefined" ? HarborUpdater : null
        // qmllint enable unqualified
    }

    // Shell-wide keyboard escape authority. Overlay views are siblings of the
    // surface that hosts page focus, so this WindowShortcut covers both trees;
    // modal popups still consume Escape through their own close policies first.
    Shortcut {
        sequence: "Escape"
        context: Qt.WindowShortcut
        onActivated: window.handleEscape()
    }
    Shortcut {
        sequence: "Ctrl+Shift+D"
        // Developer panel is a test/preview affordance only. In production
        // (real facade present) mock controls must never appear.
        enabled: !window.hasCoreFacade
        onActivated: AppState.devPanelVisible = !AppState.devPanelVisible
    }
    Shortcut {
        sequence: "Ctrl+K"
        onActivated: AppState.openPairing()
    }
    Shortcut {
        sequence: "Ctrl+N"
        onActivated: if (!window.hasCoreFacade)
            AppState.notificationsVisible = !AppState.notificationsVisible
    }
}
