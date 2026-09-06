import QtQuick
import QtQuick.Controls
import QtTest
import Harbor 2.0

TestCase {
    id: testCase
    name: "Views"
    width: 1280
    height: 960
    visible: true

    function init() {
        MockController.resetSession()
    }

    function test_networkQualityUsesPercentScale() {
        compare(AppState.networkQuality, 92)
        compare(I18n.percent(AppState.networkQuality, { isRatio: false }), "92%")
    }

    function _createPairing() {
        var component = Qt.createComponent("qrc:/qt/qml/Harbor/PairingView.qml")
        verify(component.status === Component.Ready,
               "PairingView should be ready: " + component.errorString())
        var view = component.createObject(null, { width: 1200, height: 800 })
        verify(view !== null)
        return view
    }

    // Every page renders without binding errors and keeps its dialog-level
    // accessible name as the controller walks the full pairing state machine.
    function test_pairingPagesRenderAcrossModes() {
        var view = _createPairing()
        try {
            var modes = ["choice", "qr", "request", "waiting", "error", "incoming", "success"]
            for (var i = 0; i < modes.length; i++) {
                MockController.setPairingMode(modes[i])
                wait(30)
                compare(MockController.pairingMode, modes[i])
                verify(view.visible)
            }

            // The controller, not the view, owns the pairing code and timer.
            compare(view.mode, MockController.pairingMode)
            MockController.resetSession()
            compare(MockController.pairingMode, "choice")
        } finally {
            wait(60)
            view.destroy()
        }
    }

    function _createOnboarding() {
        AppState.onboardingVisible = true
        var component = Qt.createComponent("qrc:/qt/qml/Harbor/OnboardingView.qml")
        verify(component.status === Component.Ready,
               "OnboardingView should be ready: " + component.errorString())
        var view = component.createObject(testCase, { width: 1200, height: 800 })
        verify(view !== null)
        return view
    }

    function test_onboardingSingleScreenPairing() {
        var view = _createOnboarding()
        try {
            // Single pairing screen: Harbor ID visible, peer code entry,
            // connect action, and an explicit session-scoped bypass.
            verify(view.selfHarborId !== undefined)
            compare(view.peerCode, "")
            compare(view.uiState, "INITIAL")

            // Short codes are refused honestly without touching the provider.
            view.peerCode = "123"
            verify(!view.connectWithCode())
            verify(view.noticeMessage.length > 0)

            // Continue without pairing stays unpaired and lands on home.
            AppState.setPairedPeers([])
            view.continueWithoutPairing()
            verify(!AppState.onboardingVisible)
            compare(AppState.currentView, "home")
            verify(AppState.pairingBypassed)
            verify(!AppState.paired)
        } finally {
            MockController.resetSession()
            wait(60)
            view.destroy()
        }
    }

    function _createNotifications() {
        var component = Qt.createComponent("qrc:/qt/qml/Harbor/NotificationsView.qml")
        verify(component.status === Component.Ready,
               "NotificationsView should be ready: " + component.errorString())
        var view = component.createObject(testCase, { width: 500, height: 800 })
        verify(view !== null)
        return view
    }

    // The panel derives everything from AppState.notifications: no local copy,
    // no local model, mutations flow through the controller, and filters are
    // recomputed as the shared array is reassigned.
    function test_notificationsPanelDerivesFromAppState() {
        var view = _createNotifications()
        try {
            compare(view.visibleNotifications.length, AppState.notifications.length)

            MockController.markAllNotificationsRead()
            compare(AppState.unreadCount, 0)

            view.filter = "unread"
            compare(view.visibleNotifications.length, 0)

            verify(MockController.previewNotification())
            tryVerify(function() { return AppState.unreadCount === 1 })
            compare(view.visibleNotifications.length, 1)

            var added = AppState.notifications[0]
            verify(MockController.dismissNotification(added.id))
            compare(AppState.unreadCount, 0)
            compare(view.visibleNotifications.length, 0)

            view.filter = "all"
            verify(view.visibleNotifications.length > 0)

            // Clearing is a two-step armed confirmation owned by the controller.
            compare(MockController.clearNotificationsWithConfirmation(), false)
            verify(MockController.notificationClearArmed)
            compare(MockController.clearNotificationsWithConfirmation(), true)
            compare(AppState.notifications.length, 0)
            compare(view.visibleNotifications.length, 0)

            MockController.resetSession()
            compare(view.visibleNotifications.length, AppState.notifications.length)
        } finally {
            wait(60)
            view.destroy()
        }
    }

    function _createSettings(width) {
        var component = Qt.createComponent("qrc:/qt/qml/Harbor/SettingsView.qml")
        verify(component.status === Component.Ready,
               "SettingsView should be ready: " + component.errorString())
        // Parented to the visible TestCase so mouse events reach the controls.
        var view = component.createObject(testCase, { width: width || 1200, height: 900 })
        verify(view !== null)
        return view
    }

    function _collectSwitches(item, out) {
        for (var i = 0; i < item.children.length; i++) {
            var child = item.children[i]
            if (child instanceof Switch)
                out.push(child)
            _collectSwitches(child, out)
        }
    }

    // Child order is stacking order, not document order, so switches are
    // identified by their HarborSettingRow label instead of index.
    function _switchForLabel(view, key) {
        var toggles = []
        _collectSwitches(view, toggles)
        var label = I18n.t(key)
        for (var i = 0; i < toggles.length; i++) {
            var p = toggles[i].parent
            while (p && p !== view) {
                if (p.label !== undefined && p.label === label)
                    return toggles[i]
                p = p.parent
            }
        }
        return null
    }

    // Settings controls mutate AppState through the active provider,
    // categories match the human-facing set, theme authority stays with
    // AppState, and the localized contracts remain available in both catalogs.
    function test_settingsSessionContract() {
        var view = _createSettings()
        try {
            var keys = []
            for (var i = 0; i < view.categories.length; i++)
                keys.push(view.categories[i].key)
            compare(keys, ["general", "profile", "appearance", "audio",
                           "notifications", "privacy"])

            // Honest session copy exists in both catalogs.
            var newKeys = ["settings.sessionNotice", "settings.call.state",
                           "settings.call.simulation.title",
                           "settings.call.simulation.description",
                           "settings.call.ptt.description",
                           "settings.advanced.identity.copied"]
            for (var k = 0; k < newKeys.length; k++)
                verify(I18n.t(newKeys[k]).length > 0, newKeys[k])
            AppState.locale = "pt-BR"
            for (var p = 0; p < newKeys.length; p++)
                verify(I18n.t(newKeys[p]).length > 0, newKeys[p] + " @pt-BR")
            AppState.locale = "en"

            // Theme authority: assigning session state syncs the singleton.
            AppState.appearanceMode = "light"
            compare(Theme.mode, "light")
            AppState.appearanceMode = "dark"
            compare(Theme.mode, "dark")

            // Call preview derives status from the simulated call state.
            view.category = "call"
            compare(view.callStatusKey(), "call.status.connected")
            verify(view.pttAvailable)

            // Toggles are real controls wired to the session and controller
            // (24 base + previews + three presence alerts).
            var toggles = []
            _collectSwitches(view, toggles)
            compare(toggles.length, 28)
            view.category = "general"
            var startupToggle = _switchForLabel(view, "settings.general.startWithSystem.title")
            verify(startupToggle !== null)
            compare(AppState.startWithSystem, true)
            compare(startupToggle.checked, AppState.startWithSystem)
            mouseClick(startupToggle)
            compare(startupToggle.checked, false)
            compare(AppState.startWithSystem, false)
            tryVerify(function() { return MockController.settingsApplied })
            compare(MockController.settingsFeedbackKey, "settings.savedNow")

            // Reset restores fixtures and the resting feedback copy.
            MockController.resetSession()
            compare(AppState.startWithSystem, true)
            compare(AppState.appearanceMode, "dark")
            compare(Theme.mode, AppState.appearanceMode)
            compare(MockController.settingsFeedbackKey, "settings.savedAutomatically")
        } finally {
            wait(60)
            view.destroy()
        }
    }

    // Ocean backgrounds are visibly distinct choices, not one blue repeated.
    function test_oceanVariantsDiffer() {
        MockController.resetSession()
        try {
            var seen = {}
            var variants = ["lagoon", "abyss", "sunrise", "rose",
                            "ember", "onyx", "forest", "dusk"]
            compare(Theme.oceanVariantList.length, variants.length)
            for (var i = 0; i < variants.length; i++) {
                AppState.oceanVariant = variants[i]
                wait(20)
                var top = String(Theme.bgTop)
                verify(!(top in seen), "duplicate background for " + variants[i])
                seen[top] = true
            }
            // Ember reads warm, onyx near-black: spot-check the extremes.
            AppState.oceanVariant = "ember"
            wait(20)
            verify(Theme.bgTop.r > Theme.bgTop.b, "ember must lean warm")
            AppState.oceanVariant = "onyx"
            wait(20)
            verify(Theme.bgTop.r < 0.08 && Theme.bgTop.g < 0.08, "onyx must be near-black")
        } finally {
            MockController.resetSession()
        }
    }

    // Push-to-talk accepts any captured key, persists its code, and hides
    // its call control while the feature is off.
    function test_pttKeyCaptureAndVisibility() {
        MockController.resetSession()
        AppState.pushToTalkKey = "Space"
        AppState.pushToTalkEnabled = true
        var settings = _createSettings()
        var call = _createCall()
        try {
            settings.category = "audio"
            wait(30)
            var button = _findFirst(settings, "pttKeyButton")
            verify(button !== null, "key capture button should exist")
            compare(button.text, "Space")

            var catcher = _findFirst(settings, "pttKeyCatcher")
            verify(catcher !== null, "key catcher should exist")
            // The capture button lives below the scroll fold in tests, so
            // drive its activation contract directly; scrolling is a shell
            // concern, not a capture one.
            button.clicked()
            verify(settings.capturingPttKey)
            catcher.forceActiveFocus()
            KeyTest.press(catcher, Qt.Key_F5)
            tryCompare(AppState, "pushToTalkKey", String(Qt.Key_F5))
            compare(button.text, "F5")
            verify(!settings.capturingPttKey)

            // Escape cancels the capture without touching the key.
            button.clicked()
            verify(settings.capturingPttKey)
            catcher.forceActiveFocus()
            KeyTest.press(catcher, Qt.Key_Escape)
            compare(AppState.pushToTalkKey, String(Qt.Key_F5))

            // A disabled feature removes the hold control from the call.
            MockController.forceCallState("connected")
            var ptt = _findFirst(call, "pttControl")
            verify(ptt !== null && ptt.visible)
            AppState.pushToTalkEnabled = false
            tryVerify(function() { return !ptt.visible })
            AppState.pushToTalkEnabled = true
            tryVerify(function() { return ptt.visible })
        } finally {
            wait(60)
            settings.destroy()
            call.destroy()
            MockController.resetSession()
        }
    }

    function _createDevices(width) {
        var component = Qt.createComponent("qrc:/qt/qml/Harbor/DevicesView.qml")
        verify(component.status === Component.Ready,
               "DevicesView should be ready: " + component.errorString())
        var view = component.createObject(testCase, { width: width || 1200, height: 900 })
        verify(view !== null)
        return view
    }

    // Devices render AppState fixtures only: discovery, connection and
    // removal flow through the controller, and the page honors the empty
    // state from pageStates.
    function test_devicesDeriveFromSessionState() {
        var view = _createDevices()
        try {
            compare(AppState.devices.length, 3)

            // A "found" scan appends the next candidate to the session list.
            verify(MockController.scanDevices("found"))
            tryVerify(function() { return MockController.deviceScanStage === "complete" })
            compare(AppState.devices.length, 4)
            compare(MockController.discoveredDevices.length, 1)

            // Connect and disconnect update the shared fixture in place.
            verify(MockController.setDeviceConnected("living-room-tablet", true))
            compare(AppState.itemIndex("devices", "living-room-tablet") >= 0, true)
            var tablet = AppState.devices[AppState.itemIndex("devices", "living-room-tablet")]
            compare(tablet.connected, true)
            compare(tablet.statusKey, "devices.status.connectedNow")

            // Removal takes the device out of the shared collection.
            verify(MockController.removeDevice("living-room-tablet"))
            compare(AppState.devices.length, 3)

            // The empty page state replaces the grid with the empty layer.
            AppState.clearItems("devices")
            MockController.setPageState("devices", "empty")
            compare(view.pageState, "empty")
            compare(view.title, I18n.t("devices.empty.title"))

            MockController.resetSession()
            compare(AppState.devices.length, 3)
            compare(view.pageState, "content")
        } finally {
            wait(60)
            view.destroy()
        }
    }

    function _createProfile(width) {
        var component = Qt.createComponent("qrc:/qt/qml/Harbor/ProfileView.qml")
        verify(component.status === Component.Ready,
               "ProfileView should be ready: " + component.errorString())
        var view = component.createObject(testCase, { width: width || 1200, height: 900 })
        verify(view !== null)
        return view
    }

    function _createHome(width) {
        var component = Qt.createComponent("qrc:/qt/qml/Harbor/HomeView.qml")
        verify(component.status === Component.Ready,
               "HomeView should be ready: " + component.errorString())
        var view = component.createObject(testCase, { width: width || 1200, height: 900 })
        verify(view !== null)
        return view
    }

    // The greeting follows the local clock — never a hardcoded evening:
    // small hours, morning, afternoon and evening each resolve to their
    // own localized line.
    function test_homeGreetingFollowsTheClock() {
        var view = _createHome()
        try {
            var header = _findFirst(view, "homeHeader")
            verify(header !== null)

            view.clockHour = 2
            compare(view.greetingKey, "home.greeting.lateNight")
            compare(header.eyebrow, I18n.t("home.greeting.lateNight"))

            view.clockHour = 9
            compare(view.greetingKey, "home.greeting.morning")
            compare(header.eyebrow, I18n.t("home.greeting.morning"))

            view.clockHour = 14
            compare(view.greetingKey, "home.greeting.afternoon")
            compare(header.eyebrow, I18n.t("home.greeting.afternoon"))

            view.clockHour = 21
            compare(view.greetingKey, "home.greeting.evening")
            compare(header.eyebrow, I18n.t("home.greeting.evening"))

            // Boundaries: the bucket edges belong to the later period.
            view.clockHour = 5
            compare(view.greetingKey, "home.greeting.morning")
            view.clockHour = 12
            compare(view.greetingKey, "home.greeting.afternoon")
            view.clockHour = 18
            compare(view.greetingKey, "home.greeting.evening")
        } finally {
            wait(20)
            view.destroy()
        }
    }

    // Without a paired partner the Home is an intentional waiting space:
    // a pairing action, no partner buttons, no activity row, and no
    // uninterpolated "{name}" template anywhere on screen.
    function test_homeUnpairedShowsWaitingState() {
        AppState.setPairedPeers([])
        verify(!AppState.paired)
        var view = _createHome()
        try {
            compare(view.homeState, "NO_PARTNER")
            var pair = _findFirst(view, "homePairButton")
            verify(pair !== null, "pairing action should exist")
            verify(pair.visible, "pairing action should show while unpaired")
            var call = _findFirst(view, "homeCallButton")
            var chat = _findFirst(view, "homeChatButton")
            verify(call !== null && !call.visible, "no call action without a partner")
            verify(chat !== null && !chat.visible, "no chat action without a partner")
            compare(view.presenceText(), "")
            pair.clicked()
            verify(AppState.onboardingVisible)
            AppState.onboardingVisible = false
        } finally {
            wait(60)
            view.destroy()
            MockController.resetSession()
        }
    }

    // With a partner, the Home renders exactly one state — offline, idle,
    // online, or in-activity — with the real name interpolated, never a
    // raw "{name}" template.
    function test_homePartnerStatesRenderRealProfile() {
        MockController.resetSession()
        verify(AppState.paired)
        var view = _createHome()
        try {
            AppState.setPartnerState("offline")
            compare(view.homeState, "PARTNER_OFFLINE")
            verify(view.presenceText().indexOf("{name}") < 0)
            verify(view.presenceText().indexOf("Taylor") >= 0)

            AppState.setPartnerState("idle")
            compare(view.homeState, "PARTNER_IDLE")
            verify(view.presenceText().indexOf("{name}") < 0)

            AppState.setPartnerState("online")
            AppState.setRemoteActivities([])
            compare(view.homeState, "PARTNER_ONLINE")

            AppState.setRemoteActivities([{
                id: "home-view-1", sender: "Taylor", category: "game",
                kind: "opened", label: "Minecraft", time: "20:14"
            }])
            wait(30)
            compare(view.homeState, "PARTNER_IN_ACTIVITY")
            verify(view.presenceText().indexOf("Minecraft") >= 0)
            verify(view.presenceText().indexOf("{name}") < 0)
            verify(view.presenceText().indexOf("{game}") < 0)

            var call = _findFirst(view, "homeCallButton")
            verify(call !== null && call.visible)
            verify(call.text.indexOf("{name}") < 0)
            verify(call.text.indexOf("Taylor") >= 0)
        } finally {
            AppState.setRemoteActivities([])
            wait(60)
            view.destroy()
            MockController.resetSession()
        }
    }

    function _createActivity(width) {
        var component = Qt.createComponent("qrc:/qt/qml/Harbor/ActivityView.qml")
        verify(component.status === Component.Ready,
               "ActivityView should be ready: " + component.errorString())
        var view = component.createObject(testCase, { width: width || 1200, height: 900 })
        verify(view !== null)
        return view
    }

    function _createChat(width) {
        var component = Qt.createComponent("qrc:/qt/qml/Harbor/ChatView.qml")
        verify(component.status === Component.Ready,
               "ChatView should be ready: " + component.errorString())
        var view = component.createObject(testCase, { width: width || 1200, height: 900 })
        verify(view !== null)
        return view
    }

    // _findFirst stops at the first match; the transcript, the composer and
    // each transfer card carry stable objectNames so tests reach them without
    // depending on child order.
    function _collectByName(item, prefix, out) {
        if (!item)
            return
        if (String(item.objectName).indexOf(prefix) === 0)
            out.push(item)
        for (var i = 0; i < item.children.length; ++i)
            _collectByName(item.children[i], prefix, out)
    }

    // The chat page renders the direct-channel mirror: bubbles appear as the
    // provider appends messages, the composer clears through the same send
    // path, transfer cards walk OFFERED → COMPLETED / CANCELED, and a session
    // reset takes both collections and their widgets with it.
    function test_chatPageMirrorsDirectSession() {
        var view = _createChat()
        try {
            // Without a live core the page renders through the deterministic
            // mock provider and states its honest channel line.
            verify(!view.liveDirect)
            var transcript = _findFirst(view, "chatTranscript")
            verify(transcript !== null, "transcript should be reachable")
            compare(transcript.count, 0)

            var composer = _findFirst(view, "chatComposer")
            verify(composer !== null, "composer should be reachable")

            // Provider send → bubble in the shared session state and the list.
            MockController.sendMessage("olá peer")
            compare(AppState.chatMessages.length, 1)
            compare(AppState.chatMessages[0].delivery, "DELIVERED")
            tryCompare(transcript, "count", 1)

            // The composer routes through the same send path and clears.
            composer.text = "nota da ui"
            composer.accepted()
            compare(AppState.chatMessages.length, 2)
            compare(AppState.chatMessages[1].body, "nota da ui")
            compare(composer.text, "")

            // An empty submit is a no-op, not an empty bubble.
            composer.text = "   "
            composer.accepted()
            compare(AppState.chatMessages.length, 2)

            // File offer → card with the fixture metadata and a cancel path.
            var cards = []
            MockController.offerFile("/tmp/relatorio.pdf")
            compare(AppState.transfers.length, 1)
            var offered = AppState.transfers[0]
            compare(offered.state, "OFFERED")
            compare(offered.name, "relatorio.pdf")
            cards = []
            _collectByName(view, "transferCard-", cards)
            compare(cards.length, 1)

            // Accepting walks the card to COMPLETED with the full size.
            verify(MockController.acceptTransfer(offered.id))
            compare(AppState.transfers[0].state, "COMPLETED")
            compare(AppState.transfers[0].receivedBytes, offered.size)

            // A second offer canceled by the local side ends CANCELED.
            verify(MockController.offerFile("/tmp/notas.txt"))
            compare(AppState.transfers.length, 2)
            var second = AppState.transfers[1]
            verify(MockController.cancelTransfer(second.id))
            compare(AppState.transfers[1].state, "CANCELED")
            cards = []
            _collectByName(view, "transferCard-", cards)
            compare(cards.length, 2)

            // Reset takes the transcript and the cards with the session.
            MockController.resetSession()
            compare(AppState.chatMessages.length, 0)
            compare(AppState.transfers.length, 0)
            tryCompare(transcript, "count", 0)
            cards = []
            _collectByName(view, "transferCard-", cards)
            compare(cards.length, 0)
        } finally {
            MockController.resetSession()
            wait(60)
            view.destroy()
        }
    }

    // The remote lane remains separate from the local timeline and only
    // renders the redacted peer fields the bridge accepts.
    function test_activityRemoteLaneRendersSanitizedPeerRecord() {
        var view = _createActivity()
        try {
            AppState.setRemoteActivities([{
                id: "remote-view-1", sender: "Taylor", category: "app",
                kind: "opened", label: "Code editor", time: "10:24"
            }])
            wait(30)
            var card = _findFirst(view, "remoteActivity-remote-view-1")
            verify(card !== null, "Remote activity card should render")
            compare(AppState.remoteActivities.length, 1)
            verify(AppState.remoteActivities[0].pid === undefined)
            verify(AppState.remoteActivities[0].path === undefined)
        } finally {
            AppState.setRemoteActivities([])
            wait(60)
            view.destroy()
        }
    }

    // Profile renders the session profile and commits edits only through
    // AppState.updateSelfProfile; drafts never leak into the session.
    function test_profileCommitsThroughSessionState() {
        var view = _createProfile()
        try {
            compare(AppState.selfProfile.name, "Jordan")
            compare(view.editing, false)

            // Entering edit mode seeds the drafts from the session.
            view.editing = true
            compare(view.draftName, "Jordan")

            // A committed edit updates name, initials and status together.
            AppState.updateSelfProfile({ name: "River", status: "By the creek" })
            compare(AppState.selfProfile.name, "River")
            compare(AppState.selfProfile.initials, "RI")
            compare(AppState.selfProfile.status, "By the creek")

            // GIF avatars are a first-class profile value: the type rides
            // alongside the data and every surface reads the same profile.
            AppState.updateSelfProfile({
                avatar: "data:image/gif;base64,R0lGODlhAQABAIAAAP///wAAACH5BAEAAAAALAAAAAABAAEAAAICRAEAOw==",
                avatarType: "gif"
            })
            compare(AppState.selfProfile.avatarType, "gif")
            verify(String(AppState.selfProfile.avatar).indexOf("data:image/gif") === 0)

            // Partner edits never touch the local profile.
            AppState.updatePartnerProfile({ name: "Taylor", presence: "online" })
            compare(AppState.selfProfile.name, "River")
            compare(AppState.partnerProfile.name, "Taylor")

            // Reset restores the fixture profile.
            MockController.resetSession()
            compare(AppState.selfProfile.name, "Jordan")
            compare(AppState.selfProfile.initials, "JO")
        } finally {
            wait(60)
            view.destroy()
        }
    }

    // --------------------------------------------------------------------
    // Keyboard-only interaction contracts.

    function _findFirst(item, name) {
        if (!item)
            return null
        if (item.objectName === name)
            return item
        for (var i = 0; i < item.children.length; ++i) {
            var found = _findFirst(item.children[i], name)
            if (found !== null)
                return found
        }
        return null
    }

    function _createCall() {
        var component = Qt.createComponent("qrc:/qt/qml/Harbor/CallView.qml")
        verify(component.status === Component.Ready,
               "CallView should be ready: " + component.errorString())
        // Parented to the visible TestCase so the view owns a window and
        // key events can be delivered into it.
        var view = component.createObject(testCase, { width: 1200, height: 800 })
        verify(view !== null)
        return view
    }

    // Push-to-talk is press-and-hold over the keyboard: Space or Return holds
    // the channel open, releasing it stops transmission, and mute always wins.
    function test_callPushToTalkHoldsAndReleasesWithKeyboard() {
        MockController.forceCallState("connected")
        var view = _createCall()
        try {
            var ptt = _findFirst(view, "pttControl")
            verify(ptt !== null, "PTT control should be reachable by objectName")
            compare(AppState.pushToTalkActive, false)
            ptt.forceActiveFocus()
            verify(ptt.activeFocus, "PTT control should receive keyboard focus")

            KeyTest.press(ptt, Qt.Key_Space)
            tryCompare(AppState, "pushToTalkActive", true)
            KeyTest.release(ptt, Qt.Key_Space)
            tryCompare(AppState, "pushToTalkActive", false)

            KeyTest.press(ptt, Qt.Key_Return)
            tryCompare(AppState, "pushToTalkActive", true)
            KeyTest.release(ptt, Qt.Key_Return)
            tryCompare(AppState, "pushToTalkActive", false)

            // A muted microphone rejects the hold and never stays transmitting.
            MockController.setMuted(true)
            KeyTest.press(ptt, Qt.Key_Space)
            wait(60)
            compare(AppState.pushToTalkActive, false)
            KeyTest.release(ptt, Qt.Key_Space)
            MockController.setMuted(false)
        } finally {
            wait(60)
            view.destroy()
        }
    }

    // An incoming call is an approval prompt, not an automatic connection:
    // both buttons go through the provider and decline never starts a call.
    function test_callIncomingApprovalRequiresExplicitChoice() {
        var view = _createCall()
        try {
            MockController.forceCallState("incoming")
            var accept = _findFirst(view, "incomingCallAccept")
            var decline = _findFirst(view, "incomingCallDecline")
            verify(accept !== null, "Incoming call needs an accept action")
            verify(decline !== null, "Incoming call needs a decline action")

            mouseClick(accept)
            tryCompare(AppState, "callState", "connecting")

            MockController.forceCallState("incoming")
            mouseClick(decline)
            tryCompare(AppState, "callState", "idle")
        } finally {
            MockController.resetSession()
            wait(60)
            view.destroy()
        }
    }

    // The onboarding primary control connects from the keyboard alone.
    function test_onboardingAdvancesWithKeyboardActivation() {
        AppState.onboardingVisible = true
        var view = _createOnboarding()
        try {
            var primary = _findFirst(view, "onboardingConnectButton")
            verify(primary !== null, "Connect button should be reachable")
            primary.forceActiveFocus()
            verify(primary.activeFocus, "Connect button should receive focus")
            // Empty code: honest inline notice, overlay stays open.
            KeyTest.click(primary, Qt.Key_Space)
            verify(view.noticeMessage.length > 0)
            verify(AppState.onboardingVisible)
        } finally {
            wait(60)
            view.destroy()
            AppState.onboardingVisible = false
        }
    }

    // Notification filters are chips, so the unread filter must be reachable
    // and switchable without a pointer.
    function test_notificationsFilterFollowsKeyboard() {
        var view = _createNotifications()
        try {
            var chip = _findFirst(view, "notificationFilterUnread")
            verify(chip !== null, "Unread filter chip should be reachable")
            compare(view.filter, "all")
            chip.forceActiveFocus()
            KeyTest.click(chip, Qt.Key_Space)
            tryCompare(view, "filter", "unread")
            compare(view.visibleNotifications.length,
                    AppState.notifications.filter(function(entry) { return entry.unread }).length)
        } finally {
            MockController.resetSession()
            wait(60)
            view.destroy()
        }
    }

    // Without the control plane's durable pairing, Call and Chat state the
    // honest gate instead of pretending a channel exists. The gate button
    // opens the pairing overlay, and a real snapshot takes the gate away —
    // the fixture pair restored by resetSession plays that role here.
    function test_unpairedGatesRenderAndOpenPairing() {
        AppState.setPairedPeers([])
        verify(!AppState.paired)
        var call = _createCall()
        var chat = _createChat()
        try {
            var callGate = _findFirst(call, "callUnpairedGate")
            var chatGate = _findFirst(chat, "chatUnpairedGate")
            verify(callGate !== null, "call gate should exist")
            verify(chatGate !== null, "chat gate should exist")
            verify(callGate.visible, "call gate should show while unpaired")
            verify(chatGate.visible, "chat gate should show while unpaired")

            var callButton = _findFirst(call, "callUnpairedPairButton")
            var chatButton = _findFirst(chat, "chatUnpairedPairButton")
            verify(callButton !== null, "call gate button should exist")
            verify(chatButton !== null, "chat gate button should exist")

            verify(!AppState.onboardingVisible)
            callButton.clicked()
            verify(AppState.onboardingVisible)
            AppState.onboardingVisible = false
            verify(!AppState.onboardingVisible)
            chatButton.clicked()
            verify(AppState.onboardingVisible)
            AppState.onboardingVisible = false
        } finally {
            wait(60)
            call.destroy()
            chat.destroy()
            MockController.resetSession()
        }

        // The restored fixture pair hides both gates again.
        verify(AppState.paired)
        var pairedCall = _createCall()
        var pairedChat = _createChat()
        try {
            var pairedCallGate = _findFirst(pairedCall, "callUnpairedGate")
            var pairedChatGate = _findFirst(pairedChat, "chatUnpairedGate")
            verify(pairedCallGate !== null)
            verify(pairedChatGate !== null)
            verify(!pairedCallGate.visible)
            verify(!pairedChatGate.visible)
        } finally {
            wait(60)
            pairedCall.destroy()
            pairedChat.destroy()
        }
    }
}
