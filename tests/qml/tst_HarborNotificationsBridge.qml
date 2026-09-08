import QtQuick
import QtTest
import Harbor 2.0

// Contract tests for the notification service. The facade is stubbed with
// the exact property surface the real C++ facade exposes ({coreReady,
// presence, presenceChanged}) and the notifier with the adapter's
// {available, notify()} — so these tests pin the policy independent of the
// desktop session bus:
//
//   - only messages, incoming calls and partner presence transitions notify
//     (transfers, activity, connection churn stay silent),
//   - dedup memory advances even while silenced,
//   - the chat-focused surface suppresses message cards,
//   - the previews toggle trades message text for a generic line,
//   - presence authority mirrors the aggregate and blocks mock writers.
//
// The primary surface is the widget queue (`queue`); the native adapter only
// receives a copy while the shell window cannot show it.
TestCase {
    id: root

    name: "HarborNotificationsBridge"

    QtObject {
        id: stubFacade

        property bool coreReady: true
        // The property's change signal is the same one the real facade
        // exposes: assigning `presence` IS the push event.
        property var presence: ({})
        signal phoneNotification(var notification)

        function emitPresence(next) {
            presence = next
        }
    }

    QtObject {
        id: stubNotifier

        property bool available: true
        property var calls: []

        function notify(title, body, category) {
            var next = stubNotifier.calls.slice(0)
            next.push({ title: title, body: body, category: category })
            stubNotifier.calls = next
        }

        function _reset() {
            stubNotifier.available = true
            stubNotifier.calls = []
        }
    }

    HarborNotificationsBridge {
        id: bridge
        facade: null
        notifier: stubNotifier
    }

    function init() {
        bridge.facade = null
        bridge.windowActive = true
        stubFacade.coreReady = true
        stubFacade.presence = ({})
        stubNotifier._reset()
        AppState.resetSession()
        // Fixture-free surfaces: the bridge must only ever react to facts
        // published after it started watching.
        AppState.setDirectState([], [])
        compare(bridge.queueDepth, 0)
        compare(stubNotifier.calls.length, 0)
    }

    function cleanup() {
        bridge.facade = null
        AppState.resetSession()
    }

    // ── Capability and session hygiene ───────────────────────────────

    function test_inertWithoutLiveCore() {
        // No facade: nothing may notify out of the mock world, even when
        // events would otherwise qualify.
        AppState.callState = "incoming"
        AppState.setDirectState([{ id: "m1", direction: "INCOMING", body: "hi" }], [])
        compare(bridge.queueDepth, 0)

        // Attaching a dead core stays inert.
        stubFacade.coreReady = false
        bridge.facade = stubFacade
        verify(!bridge.live)
        AppState.callState = "connected"
        AppState.callState = "incoming"
        AppState.setDirectState([{ id: "m2", direction: "INCOMING", body: "hi" }], [])
        compare(bridge.queueDepth, 0)
    }

    function test_attachConsumesHistorySilently() {
        // History already on record when the bridge starts watching must
        // never replay as notifications.
        AppState.callState = "incoming"
        AppState.setDirectState([{ id: "m1", direction: "INCOMING", body: "hi" }], [])
        stubFacade.emitPresence({
            local: { state: "ONLINE" },
            partner: { state: "OFFLINE" }
        })
        bridge.facade = stubFacade
        verify(bridge.live)
        compare(bridge.queueDepth, 0)
    }

    // ── Messages ─────────────────────────────────────────────────────

    function test_newIncomingMessageNotifiesOnce() {
        bridge.facade = stubFacade

        AppState.setDirectState(
            [{ id: "m1", direction: "INCOMING", body: "hello there" }], [])
        compare(bridge.queueDepth, 1)
        compare(bridge.queue[0].kind, "message")
        compare(bridge.queue[0].title, "New message from Taylor")
        compare(bridge.queue[0].body, "hello there")

        // Republishing the same history (redraw, refresh) is silent.
        AppState.setDirectState(
            [{ id: "m1", direction: "INCOMING", body: "hello there" }], [])
        compare(bridge.queueDepth, 1)

        // The user's own outgoing messages never notify.
        AppState.setDirectState(
            [{ id: "m1", direction: "INCOMING", body: "hello there" },
             { id: "m2", direction: "OUTGOING", body: "hey" }], [])
        compare(bridge.queueDepth, 1)

        // A genuinely new incoming message does — and dedup scans every id,
        // not just the newest, so a mid-history arrival cannot hide it.
        AppState.setDirectState(
            [{ id: "m1", direction: "INCOMING", body: "hello there" },
             { id: "m2", direction: "OUTGOING", body: "hey" },
             { id: "m3", direction: "INCOMING", body: "again" }], [])
        compare(bridge.queueDepth, 2)
        compare(bridge.queue[1].body, "again")
    }

    function test_chatFocusSuppressesMessageCards() {
        bridge.facade = stubFacade
        AppState.currentView = "chat"

        // Chat focused with the shell active: the message is already on
        // screen, no card.
        AppState.setDirectState([{ id: "m1", direction: "INCOMING", body: "hi" }], [])
        compare(bridge.queueDepth, 0)

        // New messages arrive while unfocused, then the user returns to the
        // chat: the newest one is already suppressed by focus again.
        AppState.currentView = "home"
        AppState.setDirectState([{ id: "m2", direction: "INCOMING", body: "ho" }], [])
        compare(bridge.queueDepth, 1)
        AppState.currentView = "chat"
        AppState.setDirectState([{ id: "m3", direction: "INCOMING", body: "hey" }], [])
        compare(bridge.queueDepth, 1)

        // Minimized with the chat as the current view: nothing is on
        // screen, so the card shows.
        bridge.windowActive = false
        AppState.setDirectState([{ id: "m4", direction: "INCOMING", body: "hello" }], [])
        compare(bridge.queueDepth, 2)
    }

    function test_previewPrivacyTradesTextForGenericLine() {
        bridge.facade = stubFacade

        AppState.messagePreviews = false
        AppState.setDirectState([{ id: "m1", direction: "INCOMING", body: "secret plans" }], [])
        compare(bridge.queueDepth, 1)
        compare(bridge.queue[0].body, "New message")

        AppState.messagePreviews = true
        AppState.setDirectState([{ id: "m2", direction: "INCOMING", body: "visible text" }], [])
        compare(bridge.queueDepth, 2)
        compare(bridge.queue[1].body, "visible text")
    }

    // ── Incoming calls ───────────────────────────────────────────────

    function test_incomingCallRingsOncePerCall() {
        bridge.facade = stubFacade

        AppState.callState = "incoming"
        compare(bridge.queueDepth, 1)
        compare(bridge.queue[0].kind, "call")
        compare(bridge.queue[0].title, "Incoming call")
        compare(bridge.queue[0].body, "Taylor is calling. Answer in Harbor.")

        // The connected↔connecting churn of one live call never rings.
        AppState.callState = "connecting"
        AppState.callState = "connected"
        compare(bridge.queueDepth, 1)

        // A genuinely new incoming call rings again.
        AppState.callState = "incoming"
        compare(bridge.queueDepth, 2)
    }

    function test_connectionChurnNeverNotifies() {
        bridge.facade = stubFacade

        // Reconnects and transport facts are technical events: the toast
        // surface may show them, notifications never do.
        AppState.setConnection("disconnected")
        AppState.setConnection("reconnecting")
        AppState.setConnection("connected")
        compare(bridge.queueDepth, 0)
        compare(stubNotifier.calls.length, 0)
    }

    function test_transfersNeverNotify() {
        bridge.facade = stubFacade

        // Transfers left the notify list: they surface in chat only.
        AppState.setDirectState([],
            [{ id: "t1", direction: "INCOMING", state: "OFFERED", name: "a.png" }])
        AppState.setDirectState([],
            [{ id: "t1", direction: "INCOMING", state: "COMPLETED", name: "a.png" }])
        compare(bridge.queueDepth, 0)
        compare(stubNotifier.calls.length, 0)
    }

    // ── Presence transitions ─────────────────────────────────────────

    function test_presenceTransitionsNotifyOnceEach() {
        bridge.facade = stubFacade

        // Baseline: the first aggregate observation is silent.
        stubFacade.emitPresence({
            local: { state: "ONLINE" },
            partner: { state: "ONLINE" }
        })
        compare(bridge.queueDepth, 0)

        // ONLINE → AWAY → OFFLINE → ONLINE, one card each.
        stubFacade.emitPresence({
            local: { state: "ONLINE" },
            partner: { state: "AWAY" }
        })
        compare(bridge.queueDepth, 1)
        compare(bridge.queue[0].kind, "presence")
        compare(bridge.queue[0].title, "Taylor is away")

        stubFacade.emitPresence({
            local: { state: "ONLINE" },
            partner: { state: "OFFLINE" }
        })
        compare(bridge.queueDepth, 2)
        compare(bridge.queue[1].title, "Taylor is offline")

        stubFacade.emitPresence({
            local: { state: "ONLINE" },
            partner: { state: "ONLINE" }
        })
        compare(bridge.queueDepth, 3)
        compare(bridge.queue[2].title, "Taylor is online")

        // Republished identical states never ring again.
        stubFacade.emitPresence({
            local: { state: "ONLINE" },
            partner: { state: "ONLINE" }
        })
        compare(bridge.queueDepth, 3)
    }

    function test_presenceTogglesGateIndependently() {
        bridge.facade = stubFacade
        stubFacade.emitPresence({
            local: { state: "ONLINE" },
            partner: { state: "ONLINE" }
        })

        // Away silenced, others live.
        AppState.notifyPartnerAway = false
        stubFacade.emitPresence({
            local: { state: "ONLINE" },
            partner: { state: "AWAY" }
        })
        compare(bridge.queueDepth, 0)

        // The machine keeps running: offline still announces.
        stubFacade.emitPresence({
            local: { state: "ONLINE" },
            partner: { state: "OFFLINE" }
        })
        compare(bridge.queueDepth, 1)
        compare(bridge.queue[0].title, "Taylor is offline")
    }

    function test_presenceMirrorsIntoAppStateWithAuthority() {
        bridge.facade = stubFacade

        verify(!AppState.presenceAuthoritative)
        stubFacade.emitPresence({
            local: { state: "ONLINE" },
            partner: { state: "AWAY" }
        })
        verify(AppState.presenceAuthoritative)
        // The widget vocabulary maps away → idle.
        compare(AppState.partnerState, "idle")

        // While authoritative, mock-era writers cannot invent presence:
        // a transport reconnect must not flip the partner back online.
        AppState.setConnection("disconnected")
        compare(AppState.partnerState, "idle")
        AppState.setConnection("connected")
        compare(AppState.partnerState, "idle")

        // Detaching the core surrenders authority.
        bridge.facade = null
        verify(!AppState.presenceAuthoritative)
    }

    // ── Gates and surfaces ───────────────────────────────────────────

    function test_masterSwitchSilencesEverythingButMemoryAdvances() {
        bridge.facade = stubFacade
        stubFacade.emitPresence({
            local: { state: "ONLINE" },
            partner: { state: "ONLINE" }
        })
        AppState.notificationsEnabled = false

        AppState.callState = "incoming"
        AppState.setDirectState([{ id: "m1", direction: "INCOMING", body: "hi" }], [])
        stubFacade.emitPresence({
            local: { state: "ONLINE" },
            partner: { state: "AWAY" }
        })
        compare(bridge.queueDepth, 0)

        // Re-enabling never replays the stale events, but new ones flow.
        AppState.notificationsEnabled = true
        compare(bridge.queueDepth, 0)
        AppState.callState = "connected"
        AppState.callState = "incoming"
        compare(bridge.queueDepth, 1)
    }

    function test_nativeFallbackOnlyWhileShellInactive() {
        bridge.facade = stubFacade

        // Shell visible: widget only, the session bus stays silent.
        AppState.setDirectState([{ id: "m1", direction: "INCOMING", body: "hi" }], [])
        compare(bridge.queueDepth, 1)
        compare(stubNotifier.calls.length, 0)

        // Shell minimized: the same card is copied to the native adapter.
        bridge.windowActive = false
        AppState.setDirectState([{ id: "m2", direction: "INCOMING", body: "ho" }], [])
        compare(bridge.queueDepth, 2)
        compare(stubNotifier.calls.length, 1)
        compare(stubNotifier.calls[0].title, "New message from Taylor")

        // An unavailable adapter degrades to the widget alone.
        stubNotifier.available = false
        AppState.setDirectState([{ id: "m3", direction: "INCOMING", body: "hey" }], [])
        compare(bridge.queueDepth, 3)
        compare(stubNotifier.calls.length, 1)
    }

    function test_dismissRemovesOnlyTheNamedCard() {
        bridge.facade = stubFacade
        AppState.setDirectState([{ id: "m1", direction: "INCOMING", body: "hi" }], [])
        AppState.callState = "incoming"
        compare(bridge.queueDepth, 2)

        bridge.dismiss(bridge.queue[0].id)
        compare(bridge.queueDepth, 1)
        compare(bridge.queue[0].kind, "call")

        bridge.dismissAll()
        compare(bridge.queueDepth, 0)
    }

    // Phone notifications use the same transient queue as desktop events,
    // but must never become AppState history or chat data. The signal is
    // deliberately emitted by the facade stub: this guards the production
    // Connections target and prevents regressions back to AppState wiring.
    function test_phoneNotificationIsTransientDisplayOnly() {
        bridge.facade = stubFacade
        var notificationsBefore = AppState.notifications.length
        stubFacade.phoneNotification({
            appLabel: "Messages",
            title: "New message",
            text: "display only",
            timestamp: 1725000000
        })

        compare(bridge.queueDepth, 1)
        compare(bridge.queue[0].kind, "phone")
        compare(bridge.queue[0].title, "Messages — New message")
        compare(bridge.queue[0].body, "display only")
        compare(AppState.notifications.length, notificationsBefore)
        verify(JSON.stringify(AppState.chatMessages).indexOf("display only") < 0)

        bridge.facade = null
        compare(bridge.queueDepth, 0)
        AppState.resetSession()
        // Reattaching starts from a clean transient surface; old phone
        // contents are not replayed after a process/session restart.
        bridge.facade = stubFacade
        compare(bridge.queueDepth, 0)
    }

    // ── Reserved vocabulary ──────────────────────────────────────────

    function test_quickMessageKindIsAReservedPlaceholder() {
        // QUICK_MESSAGE has no surface yet; the kind exists so a future
        // sender slots in without a policy rewrite, and nothing emits it.
        compare(bridge.kindQuickMessage, "quickMessage")
        verify(bridge.kinds.indexOf("message") >= 0)
        verify(bridge.kinds.indexOf("presence") >= 0)
        verify(bridge.kinds.indexOf("call") >= 0)
    }
}
