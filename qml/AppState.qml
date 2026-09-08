pragma Singleton
import QtQuick

QtObject {
    id: state

    // Canonical session-state vocabularies. Setters below also normalize a few
    // legacy values so views never have to handle extra states.
    readonly property var connectionStates: ["connecting", "connected", "reconnecting", "disconnected"]
    readonly property var partnerStates: ["online", "idle", "offline"]
    // True once the live core's presence aggregate named a local state. While
    // it holds, mock-era writers (connection events, pairing success) must
    // not invent partner presence — only the real aggregate moves it.
    property bool presenceAuthoritative: false
    readonly property var callStates: ["idle", "connecting", "connected", "incoming", "unavailable"]
    // Screen share is a child of the connected call. The honest live machine
    // is NOT_SHARING ⇄ SHARING: the supported capture path has no permission
    // prompt or source picker to represent, so no such states exist.
    readonly property var callShareStates: ["NOT_SHARING", "SHARING"]
    readonly property var pageStateValues: ["content", "loading", "empty", "error"]

    // Resting defaults are the unproven ones: nothing is connected, nobody
    // is online, no call exists until a real bridge or the user says so.
    // resetSession() seeds the fixture world for previews and tests.
    property string currentView: "home"
    property string connectionState: "connecting"
    property string partnerState: "offline"
    property string callState: "idle"
    // Voice activity is a live peer fact. It stays false until the call bridge
    // receives an authoritative remote level; fixtures never claim a speaker.
    property bool remoteSpeaking: false
    property string callShareState: "NOT_SHARING"
    property string activityState: "playing"

    // Direct peer data is a metadata-only snapshot from HarborFacade. QML
    // never receives local paths, digests, chunks, or transport details.
    property var chatMessages: []
    property var transfers: []

    function setDirectState(messages, nextTransfers) {
        chatMessages = messages || []
        transfers = nextTransfers || []
    }

    // Live transport facts, measured by the call's own worker. "unknown" is
    // the honest resting value: the fixture latency/quality above are the
    // mock world's, never a stand-in for a real sample.
    property real callLatency: 0
    property real callLossPct: 0
    property string callQuality: "unknown"

    function setCallStats(latency, lossPct, quality) {
        callLatency = Number(latency) || 0
        callLossPct = Number(lossPct) || 0
        callQuality = String(quality || "unknown")
        return callQuality
    }

    // Real network diagnostics, measured by the core against the configured
    // control plane. null until the first run; absent keys mean "not
    // measured", which the views render as —, never as a number.
    property var networkDiagnostics: null
    property bool networkDiagnosticsRunning: false

    function setNetworkDiagnostics(diagnostics) {
        networkDiagnostics = diagnostics && typeof diagnostics === "object"
                             ? diagnostics : null
        return networkDiagnostics
    }

    // Window-close policy: hiding to the tray is only honest when the
    // desktop session really exposes a tray icon. Without one, closing must
    // be a real quit, never an invisible orphan. (The shell additionally
    // treats the visible companion widget as a way back, so it may hide
    // while the widget is up even without a tray.)
    function resolveCloseAction(closeToTray, trayAvailable) {
        return (closeToTray && trayAvailable) ? "hide" : "quit"
    }

    // Audio input state is intentionally independent from call state.
    property bool microphoneMuted: false
    property bool pushToTalkActive: false

    // Overlay state remains in memory for the lifetime of this process only.
    property bool onboardingVisible: false
    property bool pairingVisible: false
    property bool notificationsVisible: false
    property bool trayVisible: false
    property bool devPanelVisible: false

    // Profiles are separate snapshots. Legacy scalar properties remain the
    // canonical writable fields for compatibility with the existing views.
    // Defaults stay empty: the settings bridge fills the self profile from
    // the durable core state, and the views carry honest fallbacks. Fixture
    // identities exist only in resetSession(), which the mock provider and
    // tests seed deliberately.
    property string selfName: ""
    property string selfInitials: "?"
    // A fixture status is stored as a catalog key. A profile edit clears that
    // key and leaves the entered text untouched when the locale changes.
    property string selfStatus: ""
    property string selfStatusKey: ""
    readonly property string selfStatusDisplay: selfStatusKey.length > 0
        ? I18n.t(selfStatusKey) : selfStatus
    property url selfAvatar: ""
    property string selfAvatarType: "image"
    property string harborId: ""
    // The real operating-system host name, mirrored from the facade while
    // the core lives; empty without it, so the devices page never invents
    // a machine name.
    property string deviceName: ""

    // The partner identity is session knowledge: pairing or a live presence
    // feed fills it. A durable pair alone never invents a name here — views
    // fall back to neutral copy until this session actually learned it.
    property string partnerName: ""
    property string partnerInitials: "?"
    property string partnerStatus: ""
    property string partnerStatusKey: ""
    readonly property string partnerStatusDisplay: partnerStatusKey.length > 0
        ? I18n.t(partnerStatusKey) : partnerStatus
    property url partnerAvatar: ""
    property string partnerAvatarType: "image"
    property string partnerGame: ""

    readonly property var selfProfile: ({
        "name": selfName,
        "initials": selfInitials,
        "status": selfStatusDisplay,
        "avatar": selfAvatar,
        "avatarType": selfAvatarType,
        "harborId": harborId
    })
    readonly property var partnerProfile: ({
        "name": partnerName,
        "initials": partnerInitials,
        "status": partnerStatusDisplay,
        "avatar": partnerAvatar,
        "avatarType": partnerAvatarType,
        "game": partnerGame,
        "presence": partnerState
    })

    property string sessionTime: "02:14:37"
    property string callTime: "01:08:42"
    property int latency: 18
    property int upload: 42
    property int download: 71
    property int networkQuality: 92

    // Session-only settings. `settings` exposes a grouped snapshot while the
    // flat properties preserve the API already consumed by the UI.
    property string locale: "en"
    property bool higherContrast: false
    property bool background: true
    property bool reducedMotion: false
    property alias backgroundAnimation: state.background

    property bool startWithSystem: true
    property bool minimizeToTray: true
    property bool closeToTray: true
    property bool autoConnect: true
    property bool notificationsEnabled: true
    property bool gameNotifications: true
    property bool appNotifications: true
    property bool connectionNotifications: true
    property bool notificationSound: true
    // Message-notification privacy and the three partner-presence alert
    // toggles. They gate only the notification surface — the presence
    // state machine itself keeps running regardless of any of them.
    property bool messagePreviews: true
    property bool notifyPartnerOnline: true
    property bool notifyPartnerAway: true
    property bool notifyPartnerOffline: true
    property bool presenceVisibility: true
    property bool activitySharing: true
    property bool gameVisibility: true
    property bool deviceVisibility: true
    property bool voiceActivation: true
    property bool debugMode: false
    property real accentIntensity: 0.75
    property real microphoneVolume: 0.72
    property real outputVolume: 0.64
    // IDs remain stable across a live locale change. MockController owns their
    // translated labels, and views resolve those labels when rendering.
    property string inputDevice: "default-microphone"
    property string outputDevice: "harbor-headphones"
    property string pushToTalkKey: "Space"
    // The hold control is offered only while push-to-talk is enabled; the
    // live bridge mirrors the persisted Rust setting over this property.
    property bool pushToTalkEnabled: true
    property string appearanceMode: "dark"
    // Visual personalization: persisted through the core like every other
    // setting, applied live through Theme tokens. Defaults preserve the
    // shipped look exactly.
    property string accentColor: "ocean"
    property real glassIntensity: 1.0
    property real animationIntensity: 1.0
    property bool particlesEnabled: true
    property string oceanVariant: "lagoon"
    property string cornerRadius: "soft"
    property string density: "comfortable"
    // Desktop companion widget. Persisted like every other setting; the
    // widget window itself only mirrors this state, never its own copy.
    property bool widgetEnabled: true
    property string widgetPosition: "bottomRight"
    property bool widgetShowActivity: true
    property bool widgetShowAvatar: true
    property bool widgetShowCallPresence: true

    readonly property var settings: ({
        "locale": locale,
        "higherContrast": higherContrast,
        "background": background,
        "reducedMotion": reducedMotion,
        "startWithSystem": startWithSystem,
        "minimizeToTray": minimizeToTray,
        "closeToTray": closeToTray,
        "autoConnect": autoConnect,
        "notificationsEnabled": notificationsEnabled,
        "notificationSound": notificationSound,
        "appearanceMode": appearanceMode
    })

    // Each page has one of content/loading/empty/error. Replacing the whole map
    // makes state changes observable even when consumers bind through `var`.
    property var pageStates: ({
        "home": "content",
        "call": "content",
        "activity": "content",
        "mobile": "content",
        "profile": "content",
        "settings": "content",
        "notifications": "content",
        "pairing": "content"
    })

    // Centralized session collections. Items are never mutated in place by the
    // helper API; every mutation assigns a fresh array.
    property var activities: [
        { id: "activity-1", category: "game", titleKey: "activity.event.gameOpened", titleParams: { name: "Taylor", game: "Minecraft" }, descriptionKey: "activity.event.playingSharedWorld", descriptionParams: {}, time: "20:14" },
        { id: "activity-2", category: "app", titleKey: "activity.event.appOpened", titleParams: { name: "Taylor", app: "Discord" }, descriptionKey: "activity.event.applicationOpened", descriptionParams: {}, time: "19:57" },
        { id: "activity-3", category: "system", titleKey: "activity.event.wentIdle", titleParams: { name: "Taylor" }, descriptionKey: "activity.event.awayBriefly", descriptionParams: {}, time: "19:32" },
        { id: "activity-4", category: "online", titleKey: "activity.event.cameOnline", titleParams: { name: "Taylor" }, descriptionKey: "activity.event.connectedFromSampleDesktop", descriptionParams: {}, time: "18:45" },
        { id: "activity-5", category: "call", titleKey: "activity.event.voiceRestored", titleParams: {}, descriptionKey: "activity.event.clearAudio", descriptionParams: {}, time: "18:43" }
    ]

    // Sanitized records the paired peer elected to share. They carry only a
    // stable record id, peer label, category/kind, safe label and timestamp;
    // raw process metadata never enters QML.
    property var remoteActivities: []

    // Partner's phone aggregate mirrored from the facade's mobile snapshot
    // (battery, coarse activity, consented location fix, share toggles), or
    // null when the peer never shared. Session-only: never persisted, never
    // history — a side that stops sharing returns to null.
    property var peerPhone: null
    // Unix seconds of the last peer snapshot adoption (for "as of" display).
    property double peerPhoneSeenAt: 0
    // Mirrored phone notifications: display-only, newest last, bounded.
    // Transient by design — never persisted, never merged into the durable
    // notification model, cleared when the facade goes away.
    property var phoneNotices: []

    // Durable pairing facts mirrored from the facade's contacts snapshot: the
    // control plane owns the relationship, QML only mirrors it. An empty list
    // is the honest "not paired yet"; nothing here invents a paired flag.
    property var pairedPeers: []
    // True once a live snapshot arrived this session. The first-run gate
    // waits for it instead of guessing while the fetch is still in flight.
    property bool pairedPeersResolved: false
    // An explicit "continue without pairing" decision, valid for this
    // session only. It never fakes `paired`; unpaired surfaces stay honest.
    property bool pairingBypassed: false

    readonly property bool paired: pairedPeers.length > 0

    function setPairedPeers(entries) {
        // Replacement keeps repeaters and bindings observable; entries were
        // already reduced to {deviceId, harborId} by the facade.
        pairedPeers = Array.isArray(entries) ? entries.slice(0) : []
        pairedPeersResolved = true
        return pairedPeers
    }

    function setDevices(entries) {
        // Real device snapshots replace the fixture world wholesale; reset
        // restores the deterministic fixtures for previews and tests.
        devices = Array.isArray(entries) ? entries.slice(0) : []
        return devices
    }

    function continueWithoutPairing() {
        pairingBypassed = true
    }

    function openPairing() {
        // Single pairing surface: the onboarding screen owns the whole flow
        // (shareable six-digit code plus peer entry) in production, so every
        // entry point lands on the same screen with the same code.
        // PairingView remains only for the deterministic mock provider
        // (tests/previews); production never mounts it.
        onboardingVisible = true
        pairingVisible = false
    }

    property var notifications: [
        { id: "notification-1", category: "game", titleKey: "activity.event.gameOpened", titleParams: { name: "Taylor", game: "Minecraft" }, descriptionKey: "toast.activity.description", descriptionParams: {}, timeKey: "common.time.now", timeParams: {}, unread: true },
        { id: "notification-2", category: "online", titleKey: "toast.connected.title", titleParams: {}, descriptionKey: "toast.connected.description", descriptionParams: { name: "Taylor" }, timeKey: "common.time.minutesAgo", timeParams: { count: 2 }, unread: true },
        { id: "notification-3", category: "network", titleKey: "toast.connected.title", titleParams: {}, descriptionKey: "network.check.complete.description", descriptionParams: {}, timeKey: "common.time.minutesAgo", timeParams: { count: 18 }, unread: false }
    ]

    property var devices: [
        { id: "self-desktop", nameKey: "fixture.device.selfDesktop", nameParams: {}, typeKey: "devices.type.thisDevice", iconName: "monitor", statusKey: "devices.status.connectedNow", statusParams: {}, connected: true, primary: true },
        { id: "partner-laptop", nameKey: "fixture.device.partnerLaptop", nameParams: {}, typeKey: "devices.type.computer", iconName: "laptop", statusKey: "devices.status.onlineLatency", statusParams: { latency: "18 ms" }, connected: true, primary: false },
        { id: "living-room-tablet", nameKey: "fixture.device.livingRoomTablet", nameParams: {}, typeKey: "devices.type.tablet", iconName: "tablet", statusKey: "devices.status.lastSeenYesterday", statusParams: {}, connected: false, primary: false }
    ]

    property var downloadHistory: [32, 44, 39, 55, 49, 63, 58, 71, 66, 75, 69, 71]
    property var uploadHistory: [18, 22, 19, 28, 24, 34, 31, 42, 38, 40, 37, 42]
    property var routeNodes: [
        { id: "local", kind: "local", state: "connected", latency: 0, labelKey: "network.route.node.thisHarbor", labelParams: {} },
        { id: "relay", kind: "route", state: "connected", latency: 9, labelKey: "network.route.node.demoRelay", labelParams: {} },
        { id: "partner", kind: "partner", state: "connected", latency: 18, labelKey: "fixture.device.partnerLaptop", labelParams: {} }
    ]

    readonly property int unreadCount: countUnreadNotifications()

    // Localization-aware toast requests are queued by MockController and
    // observed by HarborToastHost.
    signal localizedToastRequested(string category, string titleKey, var titleParams,
                                   string descriptionKey, var descriptionParams)

    Component.onCompleted: I18n.setLocale(locale)

    onLocaleChanged: {
        I18n.setLocale(locale)
        if (locale !== I18n.locale)
            locale = I18n.locale
    }

    // The session state is the single source of truth for the active theme;
    // views and the developer panel only assign appearanceMode. "system"
    // resolves against the live OS scheme; anything else applies directly.
    onAppearanceModeChanged: {
        Theme.mode = appearanceMode === "system"
                     ? (Theme.systemDark ? "dark" : "light") : appearanceMode
    }

    onConnectionStateChanged: {
        var normalized = normalizeConnectionState(connectionState)
        if (connectionState !== normalized) {
            connectionState = normalized
            return
        }
        if (normalized !== "connected")
            pushToTalkActive = false
        if (normalized === "disconnected")
            setCallState("idle")
    }

    onPartnerStateChanged: {
        var normalized = normalizePartnerState(partnerState)
        if (partnerState !== normalized)
            partnerState = normalized
    }

    onCallStateChanged: {
        // `muted` was historically encoded as a call state. Keep accepting it,
        // but normalize it to connected plus the independent microphone flag.
        if (callState === "muted") {
            microphoneMuted = true
            callState = "connected"
            return
        }
        var normalized = normalizeCallState(callState)
        if (callState !== normalized) {
            callState = normalized
            return
        }
        if (normalized !== "connected") {
            pushToTalkActive = false
            // Ending or losing the call always ends the share with it.
            callShareState = "NOT_SHARING"
        }
    }

    onMicrophoneMutedChanged: {
        if (microphoneMuted)
            pushToTalkActive = false
    }

    onPushToTalkActiveChanged: {
        if (pushToTalkActive && (microphoneMuted || callState !== "connected"
                                 || connectionState !== "connected"))
            pushToTalkActive = false
    }

    function containsValue(values, value) {
        return values.indexOf(value) >= 0
    }

    function normalizeConnectionState(value) {
        if (value === "online") return "connected"
        if (value === "offline") return "disconnected"
        if (value === "retrying") return "reconnecting"
        return containsValue(connectionStates, value) ? value : "disconnected"
    }

    function normalizePartnerState(value) {
        if (value === "away") return "idle"
        if (value === "connected") return "online"
        if (value === "disconnected") return "offline"
        return containsValue(partnerStates, value) ? value : "offline"
    }

    function normalizeCallState(value) {
        if (value === "ended" || value === "disconnected") return "idle"
        if (value === "active") return "connected"
        return containsValue(callStates, value) ? value : "idle"
    }

    function navigate(view) {
        // Technical views are still compiled for support and tests, but are not
        // product destinations. A stale deep link always lands somewhere useful.
        var allowed = ["home", "call", "chat", "activity", "mobile", "profile", "settings"]
        currentView = allowed.indexOf(String(view)) >= 0 ? String(view) : "home"
        notificationsVisible = false
    }

    function setConnection(next) {
        var normalized = normalizeConnectionState(next)
        connectionState = normalized
        // Transport state is not presence evidence. While the real presence
        // aggregate is authoritative, reconnects never touch partnerState.
        if (normalized === "connected") {
            if (!presenceAuthoritative)
                setPartnerState("online")
            requestLocalizedToast("online", "toast.connected.title", {},
                                  "toast.connected.description", { name: partnerName })
        } else if (normalized === "disconnected") {
            setCallState("idle")
            pushToTalkActive = false
            if (!presenceAuthoritative)
                setPartnerState("offline")
            requestLocalizedToast("offline", "toast.disconnected.title", {},
                                  "toast.disconnected.description", {})
        } else if (normalized === "reconnecting") {
            pushToTalkActive = false
            requestLocalizedToast("network", "toast.reconnecting.title", {},
                                  "toast.reconnecting.description", { name: partnerName })
        }
        return normalized
    }

    function setPartnerState(next) {
        partnerState = normalizePartnerState(next)
        return partnerState
    }

    function setCallState(next) {
        if (next === "muted") {
            microphoneMuted = true
            callState = "connected"
        } else {
            callState = normalizeCallState(next)
        }
        if (callState !== "connected") {
            pushToTalkActive = false
            callShareState = "NOT_SHARING"
        }
        return callState
    }

    function setCallShareState(next) {
        var normalized = containsValue(callShareStates, next) ? next : "NOT_SHARING"
        // A share cannot outlive the call it belongs to.
        callShareState = callState === "connected" ? normalized : "NOT_SHARING"
        return callShareState
    }

    function toggleCall() {
        if (callState === "idle" || callState === "unavailable") {
            setCallState("connecting")
            requestLocalizedToast("call", "call.toast.calling.title", { name: partnerName },
                                  "call.toast.calling.description", {})
        } else {
            setCallState("idle")
            requestLocalizedToast("call", "call.toast.ended.title", {},
                                  "call.toast.ended.description", {})
        }
    }

    function completeCallConnection() {
        if (connectionState !== "connected" || partnerState === "offline") {
            setCallState("unavailable")
            return
        }
        setCallState("connected")
        requestLocalizedToast("call", "call.toast.connected.title", {},
                              "call.toast.connected.description", {})
    }

    function toggleMute() {
        microphoneMuted = !microphoneMuted
        return microphoneMuted
    }

    function setPushToTalk(active) {
        pushToTalkActive = Boolean(active) && callState === "connected"
            && connectionState === "connected" && !microphoneMuted
        return pushToTalkActive
    }

    function updateSelfProfile(patch) {
        if (!patch) return selfProfile
        if (patch.name !== undefined) selfName = String(patch.name)
        if (patch.initials !== undefined) selfInitials = String(patch.initials)
        else if (patch.name !== undefined) selfInitials = initialsFor(selfName)
        if (patch.status !== undefined) {
            selfStatusKey = ""
            selfStatus = String(patch.status)
        }
        if (patch.harborId !== undefined) harborId = String(patch.harborId)
        if (patch.avatar !== undefined) selfAvatar = patch.avatar
        if (patch.avatarType !== undefined) {
            var kind = String(patch.avatarType)
            selfAvatarType = kind === "gif" ? "gif" : "image"
        }
        return selfProfile
    }

    function updatePartnerProfile(patch) {
        if (!patch) return partnerProfile
        if (patch.name !== undefined) partnerName = String(patch.name)
        if (patch.initials !== undefined) partnerInitials = String(patch.initials)
        else if (patch.name !== undefined) partnerInitials = initialsFor(partnerName)
        if (patch.status !== undefined) {
            partnerStatusKey = ""
            partnerStatus = String(patch.status)
        }
        if (patch.game !== undefined) partnerGame = String(patch.game)
        if (patch.avatar !== undefined) partnerAvatar = patch.avatar
        if (patch.avatarType !== undefined)
            partnerAvatarType = String(patch.avatarType) === "gif" ? "gif" : "image"
        if (patch.presence !== undefined) setPartnerState(patch.presence)
        return partnerProfile
    }

    function initialsFor(name) {
        var words = String(name).trim().split(/\s+/)
        if (!words.length || !words[0].length) return "?"
        if (words.length === 1) return words[0].slice(0, 2).toUpperCase()
        return words[0].charAt(0).toUpperCase() + words[words.length - 1].charAt(0).toUpperCase()
    }

    function requestLocalizedToast(category, titleKey, titleParams, descriptionKey, descriptionParams) {
        localizedToastRequested(category || "system", titleKey || "", titleParams || {},
                                descriptionKey || "", descriptionParams || {})
    }

    function setRemoteActivities(entries) {
        // Assignment by replacement keeps QML repeaters and filters reliably
        // observable; bridge data was schema-validated by the Rust core.
        remoteActivities = Array.isArray(entries) ? entries.slice(0) : []
        return remoteActivities
    }

    function setPeerPhone(snapshot) {
        // The facade already reduced the snapshot to typed UI facts; a null
        // (or missing peer) means "not sharing" and clears the view.
        if (!snapshot || typeof snapshot !== "object") {
            peerPhone = null
            return peerPhone
        }
        var copy = {}
        for (var key in snapshot)
            copy[key] = snapshot[key]
        peerPhone = copy
        peerPhoneSeenAt = Date.now() / 1000
        return peerPhone
    }

    function pushPhoneNotice(entry) {
        // Display-only FIFO, bounded so a chatty phone cannot grow memory.
        var next = phoneNotices.slice(0)
        var notice = entry && typeof entry === "object" ? entry : {}
        next.push({
            id: String(notice.id || ("phone-" + Date.now() + "-" + next.length)),
            appLabel: String(notice.appLabel || ""),
            title: String(notice.title || ""),
            text: String(notice.text || ""),
            at: Number(notice.at) || Date.now() / 1000
        })
        while (next.length > 8)
            next.shift()
        phoneNotices = next
        return phoneNotices
    }

    function clearPhoneNotices() {
        phoneNotices = []
        return phoneNotices
    }

    function simulateActivity() {
        requestLocalizedToast("game", "toast.activity.title",
                              { name: partnerName, app: "Stardew Valley" },
                              "toast.activity.description", {})
    }

    function pageState(page) {
        var value = pageStates[page]
        return containsValue(pageStateValues, value) ? value : "content"
    }

    function setPageState(page, next) {
        if (!page || !containsValue(pageStateValues, next)) return false
        var updated = cloneObject(pageStates)
        updated[page] = next
        pageStates = updated
        return true
    }

    function cloneObject(source) {
        var copy = {}
        if (!source) return copy
        for (var key in source)
            copy[key] = source[key]
        return copy
    }

    function isCollection(name) {
        return ["activities", "notifications", "devices", "downloadHistory",
                "uploadHistory", "routeNodes"].indexOf(name) >= 0
    }

    function replaceItems(collection, items) {
        if (!isCollection(collection) || !Array.isArray(items)) return false
        state[collection] = items.slice(0)
        return true
    }

    function appendItem(collection, item) {
        if (!isCollection(collection)) return false
        var updated = state[collection].slice(0)
        updated.push(item)
        state[collection] = updated
        return true
    }

    function prependItem(collection, item) {
        if (!isCollection(collection)) return false
        var updated = state[collection].slice(0)
        updated.unshift(item)
        state[collection] = updated
        return true
    }

    function itemIndex(collection, selector) {
        if (!isCollection(collection)) return -1
        var items = state[collection]
        if (typeof selector === "number")
            return selector >= 0 && selector < items.length ? selector : -1
        for (var index = 0; index < items.length; ++index) {
            if (items[index] && items[index].id === selector)
                return index
        }
        return -1
    }

    function updateItem(collection, selector, patch) {
        var index = itemIndex(collection, selector)
        if (index < 0 || !patch) return false
        var updated = state[collection].slice(0)
        var value = updated[index]
        if (value !== null && typeof value === "object" && !Array.isArray(value)) {
            var replacement = cloneObject(value)
            for (var key in patch)
                replacement[key] = patch[key]
            updated[index] = replacement
        } else {
            updated[index] = patch
        }
        state[collection] = updated
        return true
    }

    function removeItem(collection, selector) {
        var index = itemIndex(collection, selector)
        if (index < 0) return false
        var updated = state[collection].slice(0)
        updated.splice(index, 1)
        state[collection] = updated
        return true
    }

    function clearItems(collection) {
        if (!isCollection(collection)) return false
        state[collection] = []
        return true
    }

    function countUnreadNotifications() {
        var count = 0
        for (var index = 0; index < notifications.length; ++index) {
            if (notifications[index].unread === true)
                ++count
        }
        return count
    }

    function markNotificationRead(selector, read) {
        return updateItem("notifications", selector, { "unread": read === undefined ? false : !read })
    }

    function markAllNotificationsRead() {
        var updated = []
        for (var index = 0; index < notifications.length; ++index) {
            var notification = cloneObject(notifications[index])
            notification.unread = false
            updated.push(notification)
        }
        notifications = updated
    }

    function resetSession() {
        currentView = "home"
        connectionState = "connected"
        partnerState = "online"
        callState = "connected"
        activityState = "playing"
        microphoneMuted = false
        pushToTalkActive = false
        setDirectState([], [])
        setCallStats(0, 0, "unknown")
        networkDiagnostics = null
        networkDiagnosticsRunning = false
        pairedPeers = []
        pairedPeersResolved = false
        pairingBypassed = false
        presenceAuthoritative = false
        deviceName = ""

        onboardingVisible = false
        pairingVisible = false
        notificationsVisible = false
        trayVisible = false
        devPanelVisible = false

        selfName = "Jordan"
        selfInitials = "JO"
        selfStatus = ""
        selfStatusKey = "fixture.profile.selfStatus"
        selfAvatar = ""
        selfAvatarType = "image"
        harborId = "HBR-7D92-4A10"
        partnerName = "Taylor"
        partnerInitials = "TA"
        partnerStatus = ""
        partnerStatusKey = "fixture.profile.partnerStatus"
        partnerAvatar = ""
        partnerAvatarType = "image"
        partnerGame = "Minecraft"

        sessionTime = "02:14:37"
        callTime = "01:08:42"
        latency = 18
        upload = 42
        download = 71
        networkQuality = 92

        locale = "en"
        higherContrast = false
        background = true
        reducedMotion = false
        startWithSystem = true
        minimizeToTray = true
        closeToTray = true
        autoConnect = true
        notificationsEnabled = true
        gameNotifications = true
        appNotifications = true
        connectionNotifications = true
        notificationSound = true
        messagePreviews = true
        notifyPartnerOnline = true
        notifyPartnerAway = true
        notifyPartnerOffline = true
        presenceVisibility = true
        activitySharing = true
        gameVisibility = true
        deviceVisibility = true
        voiceActivation = true
        debugMode = false
        accentIntensity = 0.75
        microphoneVolume = 0.72
        outputVolume = 0.64
        inputDevice = "default-microphone"
        outputDevice = "harbor-headphones"
        pushToTalkKey = "Space"
        pushToTalkEnabled = true
        appearanceMode = "dark"
        accentColor = "ocean"
        glassIntensity = 1.0
        animationIntensity = 1.0
        particlesEnabled = true
        oceanVariant = "lagoon"
        cornerRadius = "soft"
        density = "comfortable"
        widgetEnabled = true
        widgetPosition = "bottomRight"
        widgetShowActivity = true
        widgetShowAvatar = true
        widgetShowCallPresence = true

        pageStates = {
            "home": "content", "call": "content", "activity": "content",
            "mobile": "content", "network": "content", "devices": "content", "profile": "content",
            "settings": "content", "notifications": "content", "pairing": "content"
        }

        activities = [
            { id: "activity-1", category: "game", titleKey: "activity.event.gameOpened", titleParams: { name: "Taylor", game: "Minecraft" }, descriptionKey: "activity.event.playingSharedWorld", descriptionParams: {}, time: "20:14" },
            { id: "activity-2", category: "app", titleKey: "activity.event.appOpened", titleParams: { name: "Taylor", app: "Discord" }, descriptionKey: "activity.event.applicationOpened", descriptionParams: {}, time: "19:57" },
            { id: "activity-3", category: "system", titleKey: "activity.event.wentIdle", titleParams: { name: "Taylor" }, descriptionKey: "activity.event.awayBriefly", descriptionParams: {}, time: "19:32" },
            { id: "activity-4", category: "online", titleKey: "activity.event.cameOnline", titleParams: { name: "Taylor" }, descriptionKey: "activity.event.connectedFromSampleDesktop", descriptionParams: {}, time: "18:45" },
            { id: "activity-5", category: "call", titleKey: "activity.event.voiceRestored", titleParams: {}, descriptionKey: "activity.event.clearAudio", descriptionParams: {}, time: "18:43" }
        ]
        remoteActivities = []
        // Fixture partner phone: battery + activity shared, location and
        // notification mirroring off — the honest partial-sharing shape.
        peerPhone = {
            batteryPercent: 73, charging: false, phoneActivity: "ACTIVE",
            lastActiveAt: Date.now() / 1000 - 90, currentApp: "Minecraft",
            locationSharingEnabled: false, location: null,
            notificationSharingEnabled: false, deviceType: "mobile"
        }
        peerPhoneSeenAt = Date.now() / 1000
        phoneNotices = []
        notifications = [
            { id: "notification-1", category: "game", titleKey: "activity.event.gameOpened", titleParams: { name: "Taylor", game: "Minecraft" }, descriptionKey: "toast.activity.description", descriptionParams: {}, timeKey: "common.time.now", timeParams: {}, unread: true },
            { id: "notification-2", category: "online", titleKey: "toast.connected.title", titleParams: {}, descriptionKey: "toast.connected.description", descriptionParams: { name: "Taylor" }, timeKey: "common.time.minutesAgo", timeParams: { count: 2 }, unread: true },
            { id: "notification-3", category: "network", titleKey: "toast.connected.title", titleParams: {}, descriptionKey: "network.check.complete.description", descriptionParams: {}, timeKey: "common.time.minutesAgo", timeParams: { count: 18 }, unread: false }
        ]
        devices = [
            { id: "self-desktop", nameKey: "fixture.device.selfDesktop", nameParams: {}, typeKey: "devices.type.thisDevice", iconName: "monitor", statusKey: "devices.status.connectedNow", statusParams: {}, connected: true, primary: true },
            { id: "partner-laptop", nameKey: "fixture.device.partnerLaptop", nameParams: {}, typeKey: "devices.type.computer", iconName: "laptop", statusKey: "devices.status.onlineLatency", statusParams: { latency: "18 ms" }, connected: true, primary: false },
            { id: "living-room-tablet", nameKey: "fixture.device.livingRoomTablet", nameParams: {}, typeKey: "devices.type.tablet", iconName: "tablet", statusKey: "devices.status.lastSeenYesterday", statusParams: {}, connected: false, primary: false }
        ]
        downloadHistory = [32, 44, 39, 55, 49, 63, 58, 71, 66, 75, 69, 71]
        uploadHistory = [18, 22, 19, 28, 24, 34, 31, 42, 38, 40, 37, 42]
        routeNodes = [
            { id: "local", kind: "local", state: "connected", latency: 0, labelKey: "network.route.node.thisHarbor", labelParams: {} },
            { id: "relay", kind: "route", state: "connected", latency: 9, labelKey: "network.route.node.demoRelay", labelParams: {} },
            { id: "partner", kind: "partner", state: "connected", latency: 18, labelKey: "fixture.device.partnerLaptop", labelParams: {} }
        ]
    }

    // Compatibility with the original prototype API.
}
