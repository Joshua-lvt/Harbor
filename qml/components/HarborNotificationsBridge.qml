pragma ComponentBehavior: Bound
import QtQml

// The notification service. This bridge owns policy — which events deserve a
// notification, which surface carries it, and the user's durable toggles —
// and resolves localized texts through the same catalogs as the UI.
//
// Four event families qualify:
//   1. new incoming chat messages (suppressed while the chat is focused and
//      the shell is active; shown when unfocused or minimized),
//   2. the partner's ONLINE/AWAY/OFFLINE transitions (three independent
//      toggles; the presence machine runs regardless),
//   3. an incoming call (its ringing surface stays in the call flow),
//   4. an ephemeral phone-notification display event from a paired mobile.
// Everything else — games, apps, transfers, activity, reconnects, transport
// and signaling facts — is deliberately silent.
//
// Surfaces: the in-app HarborNotificationWidget is primary (its window is
// mounted by Main.qml and reads `queue`); the native desktop notification is
// the fallback while the shell is not the active window. Sounds ride the
// native HarborSounds adapter when present.
//
// Bodies stay generic on purpose: a desktop notification can be visible to
// other people, so message contents never appear on the lock screen — the
// previews toggle only trades message text for a generic line.
// qmllint disable missing-property
QtObject {
    id: service

    property QtObject facade: null
    property QtObject notifier: null
    // Wired by Main.qml: whether the shell window can show in-app state
    // right now (active and not minimized).
    property bool windowActive: true

    readonly property bool live: facade !== null && facade.coreReady

    // Notification kinds. QUICK_MESSAGE is a protocol placeholder only:
    // quick messages have no surface yet, so nothing emits that kind today.
    readonly property string kindMessage: "message"
    readonly property string kindPresence: "presence"
    readonly property string kindCall: "call"
    readonly property string kindQuickMessage: "quickMessage"
    readonly property string kindPhone: "phone"
    readonly property var kinds: [kindMessage, kindPresence, kindCall, kindQuickMessage, kindPhone]

    // Cards for HarborNotificationWidget: {id, kind, title, body, avatar}.
    // The widget stacks everything queued and dismisses by id.
    property var queue: []
    readonly property int queueDepth: queue.length

    function dismiss(id) {
        var next = []
        for (var i = 0; i < queue.length; ++i) {
            if (String(queue[i].id) !== String(id))
                next.push(queue[i])
        }
        queue = next
    }

    function dismissAll() {
        queue = []
    }

    // Dedup memory: the call state already on record, every chat message id
    // already consumed, and the partner presence already mirrored.
    property string seenCallState: ""
    property var seenMessageIds: ({})
    property string seenPartnerState: ""
    property int nextCardId: 0

    function _resyncFromSession() {
        // (Re)attach consumes history silently: only facts that arrive while
        // watching become notifications, and a fresh session starts with an
        // empty widget. Detaching also surrenders presence authority — the
        // mock world may speak again.
        seenCallState = AppState.callState
        seenMessageIds = {}
        for (var i = 0; i < AppState.chatMessages.length; ++i)
            seenMessageIds[String(AppState.chatMessages[i].id)] = true
        seenPartnerState = ""
        queue = []
        AppState.presenceAuthoritative = false
    }

    // Widget first; native only while the user cannot see the shell. The
    // master switch is policy, not memory: handlers keep advancing dedup
    // state while silenced, so re-enabling never replays stale events.
    function _dispatch(kind, title, body, soundKey) {
        if (AppState.notificationsEnabled === false)
            return
        nextCardId += 1
        var card = {
            id: "card-" + nextCardId,
            kind: kind,
            title: title,
            body: body,
            avatar: String(AppState.partnerAvatar || "")
        }
        queue = queue.concat([card])
        if (AppState.notificationSound !== false
                && typeof HarborSounds !== "undefined"
                && HarborSounds.available)
            HarborSounds.play(soundKey)
        if (!windowActive && notifier !== null && notifier.available)
            notifier.notify(title, body, "im")
    }

    // True while the chat surface is the user's active focus: new messages
    // are already visible there, so a notification would only duplicate.
    readonly property bool chatFocused: AppState.currentView === "chat" && windowActive

    onLiveChanged: _resyncFromSession()
    Component.onCompleted: _resyncFromSession()

    // Committed presence aggregate {local, partner}: the single authoritative
    // source for the partner state. Mirrors it into AppState and announces
    // transitions through the three independent toggles.
    function _applyPresenceAggregate() {
        if (!live)
            return
        var aggregate = facade.presence
        var local = aggregate && aggregate.local && aggregate.local.state
                    ? String(aggregate.local.state).toLowerCase() : ""
        AppState.presenceAuthoritative = local !== ""
        var partner = aggregate && aggregate.partner && aggregate.partner.state
                      ? String(aggregate.partner.state).toLowerCase() : ""
        if (partner === "")
            return
        // The first observation is a silent baseline; only transitions ring.
        if (seenPartnerState !== "" && partner !== seenPartnerState) {
            if (partner === "online" && AppState.notifyPartnerOnline !== false)
                _dispatch(kindPresence,
                          I18n.t("notify.presence.online.title", { name: AppState.partnerName }),
                          I18n.t("notify.presence.online.body"), "presence")
            else if (partner === "away" && AppState.notifyPartnerAway !== false)
                _dispatch(kindPresence,
                          I18n.t("notify.presence.away.title", { name: AppState.partnerName }),
                          I18n.t("notify.presence.away.body"), "presence")
            else if (partner === "offline" && AppState.notifyPartnerOffline !== false)
                _dispatch(kindPresence,
                          I18n.t("notify.presence.offline.title", { name: AppState.partnerName }),
                          I18n.t("notify.presence.offline.body"), "presence")
        }
        seenPartnerState = partner
        AppState.setPartnerState(partner)
    }

    readonly property list<QtObject> wiring: [
        Connections {
            target: AppState

            function onCallStateChanged() {
                // Only the arrival of an incoming call rings; the caller sees
                // the rest in the call surface itself. Without the live core
                // (mock world, tests) or with the toggle off, memory still
                // advances so nothing replays.
                if (!service.live || AppState.connectionNotifications === false) {
                    service.seenCallState = AppState.callState
                    return
                }
                if (service.seenCallState !== "incoming"
                        && AppState.callState === "incoming")
                    service._dispatch(service.kindCall,
                                      I18n.t("notify.call.title"),
                                      I18n.t("notify.call.body", { name: AppState.partnerName }),
                                      "call")
                service.seenCallState = AppState.callState
            }

            function onChatMessagesChanged() {
                service._applyChatMessages()
            }

        },

        Connections {
            target: service.facade

            function onPresenceChanged() {
                service._applyPresenceAggregate()
            }

            // Phone notifications arrive as an unsolicited facade event,
            // not as AppState history. Keep this handler on the real facade
            // connection so the display-only event reaches the transient
            // widget without ever being persisted or replayed.
            function onPhoneNotification(notification) {
                service._applyPhoneNotification(notification)
            }
        }
    ]

    // Set-based sweep: chatMessages is oldest-first, so every id enters the
    // memory map and only genuinely-new INCOMING messages become cards.
    function _applyChatMessages() {
        var messages = AppState.chatMessages
        var liveIds = {}
        var arrivals = []
        for (var i = 0; i < messages.length; ++i) {
            var message = messages[i]
            var id = String(message.id)
            liveIds[id] = true
            if (message.direction === "INCOMING" && seenMessageIds[id] !== true)
                arrivals.push(message)
        }
        seenMessageIds = liveIds
        // The mock world never notifies; the chat surface suppresses what
        // the user is already looking at.
        if (arrivals.length === 0 || !live || chatFocused)
            return
        for (var j = 0; j < arrivals.length; ++j) {
            var preview = AppState.messagePreviews !== false
                          ? String(arrivals[j].body || "")
                          : I18n.t("notify.message.private.body")
            _dispatch(kindMessage,
                      I18n.t("notify.message.title", { name: AppState.partnerName }),
                      preview, "message")
        }
    }

    // Phone notifications are an ephemeral display surface. Do not mirror
    // them into AppState.notifications or the chat model; a restart must
    // therefore have nothing old to replay.
    function _applyPhoneNotification(notification) {
        if (!live || AppState.notificationsEnabled === false || !notification)
            return
        var app = String(notification.appLabel || "Harbor phone")
        var title = String(notification.title || "")
        var body = String(notification.text || "")
        if (title.length === 0 && body.length === 0)
            return
        _dispatch(kindPhone, app + (title.length > 0 ? " — " + title : ""), body, "message")
    }
}
