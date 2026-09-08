// Harbor mobile host: binds the touch shell to the in-process core and
// the Android facade. This is the only file that knows both sides:
//
//   MobileShell  <->  Host  <->  coreAdapter (Rust) + androidBridge (JNI)
//
// The shell stays injectable/standalone-verifiable; the host owns every
// binding, unit conversion, and policy glue (focus rules for chat
// notifications, mic-muted enforcement display, share-intent fan-out).
import QtQuick
import HarborMobile

MobileShell {
    id: host

    // ---- transports (context properties from main.cpp) ----
    property var core: coreAdapter
    property var platform: androidBridge
    property var pendingRequests: ({})

    // All core requests, including local reads, use the queued path. The
    // response signal carries the request id so overlapping refreshes cannot
    // mix their payloads.
    function request(type, payload, callback) {
        const id = host.core.sendAsync(type, payload || {})
        host.pendingRequests[id] = callback || function(_payload, _error) {}
        return id
    }

    // Outbound chat intents from the shell.
    onSendChat: body => {
        if (body.trim().length === 0 || host.mobileToMobileBlocked)
            return
        host.request("chat.send", {"body": body})
    }
    onSetMicMuted: muted => host.request("call.mute", {"muted": muted})
    onEnterCall: {
        if (!host.platform.ensureCallAudio()) {
            host.callState = "unavailable"
            return
        }
        host.request("call.start", {}, (_reply, error) => {
            if (error.length > 0) {
                host.callState = "unavailable"
                host.platform.stopCallAudio()
            }
        })
    }
    onAcceptIncomingCall: {
        if (!host.platform.ensureCallAudio()) {
            host.callState = "unavailable"
            return
        }
        host.request("call.accept", {}, (_reply, error) => {
            if (error.length > 0) {
                host.callState = "unavailable"
                host.platform.stopCallAudio()
            }
        })
    }
    onDeclineIncomingCall: host.request("call.decline", {})
    onLeaveCall: {
        host.request("call.end", {})
        host.platform.stopCallAudio()
    }
    onRequestTakeover: {
        // Takeover names this install: the core starts a takeover offer,
        // the peer drops the sibling's media first, then rings here.
        const self = host.deviceId
        if (self.length > 0 && host.platform.ensureCallAudio())
            host.request("call.takeover", {"joinDevice": self}, (_reply, error) => {
                if (error.length > 0) {
                    host.callState = "unavailable"
                    host.platform.stopCallAudio()
                }
            })
        else if (self.length > 0)
            host.callState = "unavailable"
    }
    onSetPersistentCall: on => host.updateAppearance("persistentCall", on)
    onSetAppearanceMode: mode => host.updateAppearance("appearanceMode", mode)
    onSetAccentColor: color => host.updateAppearance("accentColor", color)
    onSetAccentIntensity: value => host.updateAppearance("accentIntensity", value)
    onSetOceanVariant: variant => host.updateAppearance("oceanVariant", variant)
    onSetCornerRadius: value => host.updateAppearance("cornerRadius", value)
    onSetDensity: value => host.updateAppearance("density", value)
    onSetHigherContrast: on => host.updateAppearance("higherContrast", on)
    onSetReducedMotion: on => host.updateAppearance("reducedMotion", on)
    onSetAnimationIntensity: value => host.updateAppearance("animationIntensity", value)
    onSetShareLocation: on => {
        host.locationSharing = on
        host.request("settings.update", {"shareLocation": on})
        host.platform.setLocationSharing(on)
    }
    onSetSharePhoneActivity: on => {
        host.phoneActivitySharing = on
        host.request("settings.update", {"sharePhoneActivity": on})
    }
    onSetSharePhoneNotifications: on => {
        host.phoneNotificationsSharing = on
        host.request("settings.update", {"sharePhoneNotifications": on})
        host.platform.setNotificationMirroring(on)
    }
    onOpenChat: {} // bottom-nav Chat tab is the chat surface on mobile
    onOpenSystemSettings: page => host.platform.openSystemSettings(page)
    onRequestOwnNotificationPermission: host.platform.requestOwnNotificationPermission()
    onOpenPairing: {
        host.pairingVisible = true
        host.pairingError = ""
        host.fetchOwnHarborId()
        host.refreshServer()
    }
    onClosePairing: { host.pairingVisible = false; host.pairingReset() }
    onInvitePairing: harborId => host.pairingInvite(harborId)
    onCopyHarborId: host.copyHarborId()
    onAcceptPairing: host.pairingAccept()
    onDeclinePairing: host.pairingDecline()
    onCancelPairing: host.pairingCancel()
    onResetPairing: host.pairingReset()
    onPollPairingStatus: host.pairingPollStatus()
    onPollPairingIncoming: host.pairingPollIncoming()
    onRetryUpdate: host.platform.checkForUpdates()
    onInstallUpdate: host.platform.installUpdate()
    onCheckForUpdates: { host.platform.checkForUpdates(); host.pollUpdate() }

    // Pre-pointed Harbor network: the K11 public IPv6 endpoint — unless this
    // build baked an Oracle default (oracleDefaultAddress context property,
    // from HARBOR_ORACLE_ADDRESS at configure time), which fresh installs
    // prefer. Public pinning material, never a secret.
    readonly property string defaultServerAddress: (typeof oracleDefaultAddress !== "undefined" && String(oracleDefaultAddress).length > 0) ? String(oracleDefaultAddress) : "[2804:d59:8777:ad00:3a30:f9ff:fe3e:de81]:9091"
    readonly property string defaultServerFingerprint: "b9846aed2e97bd741ae5a2a3de9ab37c1831d2372ca67f26f538bd279dd7271f"

    property string deviceId: ""
    property string lastMessageId: ""
    property bool updateInstallFired: false
    Component.onCompleted: {
        // This install is a mobile endpoint, durably. The desktop default
        // never applies here; the core persists it on first run.
        host.request("device.configure", {"deviceType": "mobile"})
        host.ensureDefaultServer()
        host.platform.refresh()
        host.pushPhoneState()
        host.platform.checkForUpdates()
        refreshTimer.start()
    }

    Timer {
        id: refreshTimer
        interval: 15000
        repeat: true
        onTriggered: {
            host.platform.refresh()
            host.pushPhoneState()
            host.pollUpdate()
        }
    }

    // Mandatory update pump, polled: the Java side owns check/download/
    // verify on worker threads and only exposes this state string. A mere
    // error never blocks; discovery auto-downloads, readiness auto-installs.
    function pollUpdate() {
        var state = {}
        try {
            state = JSON.parse(host.platform.updateState() || "{}")
        } catch (e) {
            return
        }
        var status = String(state.status || "idle")
        host.updateStatus = status
        host.updateVersion = String(state.version || "")
        host.updateProgress = Number(state.progress) || 0
        host.updateError = String(state.error || "")
        if (status === "available") {
            host.updateInstallFired = false
            host.platform.downloadUpdate(String(state.url || ""), String(state.sha || ""))
        } else if (status === "ready" && !host.updateInstallFired) {
            host.updateInstallFired = true
            host.platform.installUpdate()
        } else if (status === "idle" || status === "checking") {
            host.updateInstallFired = false
        }
    }

    // First run points at the Harbor network automatically; an existing
    // pin is always respected and never overwritten.
    function ensureDefaultServer() {
        host.request("server.config", {}, (config, error) => {
            if (error.length === 0 && config["configured"] !== true) {
                host.request("server.configure", {
                    "address": host.defaultServerAddress,
                    "fingerprint": host.defaultServerFingerprint
                }, (_reply, configureError) => {
                    if (configureError.length > 0)
                        host.pairingError = configureError
                    host.refreshAll()
                })
                return
            }
            host.serverNeedsMigration = error.length === 0 && config["needs_migration"] === true
            host.refreshAll()
        })
    }

    // Idempotent K11 -> Oracle migration (core `server.migrate`): only an
    // exact legacy endpoint moves, the pin is preserved, custom endpoints
    // are kept. Empty toAddress uses the core canary default; pass the
    // production hostname at cutover. Reconnect afterwards (E5).
    function migrateServer(toAddress) {
        var payload = {}
        if (toAddress !== undefined && String(toAddress).length > 0) {
            payload["to_address"] = String(toAddress)
            payload["address"] = String(toAddress)
        }
        host.request("server.migrate", payload, (reply, error) => {
            if (error.length > 0) {
                host.pairingError = error
                return
            }
            host.serverNeedsMigration = false
            host.refreshAll()
        })
    }

    // One typed writer for every durable toggle: the shell prop applies
    // instantly (the theme follows it), the core persists it, and the reply
    // re-syncs so a rejected value snaps back instead of lying on screen.
    function updateAppearance(key, value) {
        var patch = {}
        patch[key] = value
        host[key] = value
        host.request("settings.update", patch, (_reply, error) => {
            if (error.length > 0)
                host.refreshAppearance()
        })
    }

    function refreshAppearance() {
        host.request("settings.get", {}, settings => {
            host.persistentCall = settings["persistentCall"] !== false
            host.appearanceMode = settings["appearanceMode"] || "dark"
            host.accentColor = settings["accentColor"] || "ocean"
            var accentLevel = Number(settings["accentIntensity"])
            host.accentIntensity = isNaN(accentLevel) ? 0.75 : accentLevel
            host.oceanVariant = settings["oceanVariant"] || "lagoon"
            host.cornerRadius = settings["cornerRadius"] || "soft"
            host.density = settings["density"] || "comfortable"
            host.higherContrast = settings["higherContrast"] === true
            host.reducedMotion = settings["reducedMotion"] === true
            var motionLevel = Number(settings["animationIntensity"])
            host.animationIntensity = isNaN(motionLevel) ? 1.0 : motionLevel
            host.locationSharing = settings["shareLocation"] === true
            host.phoneActivitySharing = settings["sharePhoneActivity"] === true
            host.phoneNotificationsSharing = settings["sharePhoneNotifications"] === true
        })
    }

    function refreshServer() {
        host.request("server.config", {}, (config, error) => {
            if (error.length > 0)
                return
            host.serverConfigured = config["configured"] === true
            host.serverNeedsMigration = config["needs_migration"] === true
            host.request("contacts.list", {}, (contacts, contactsError) => {
                if (contactsError.length > 0)
                    return
                host.sessionValid = (contacts["peers"] || []).length > 0
            })
        })
    }

    // Harbor-ID invitation: no codes anywhere. The full Harbor ID is
    // validated by the sheet before emitting; the core validates again.
    function isValidHarborId(value) {
        return /^harbor-[0-9a-fA-F]{8}$/.test(String(value || ""))
    }

    function pairingInvite(harborId) {
        const original = String(harborId || "")
        if (!host.isValidHarborId(original))
            return false
        host.pairingBusy = true
        host.pairingError = ""
        host.pairingConnecting = true
        host.pairingPeerHarborId = original
        host.request("pairing.invite", {"peer": original}, (reply, error) => {
            host.pairingBusy = false
            host.pairingPhase = reply["phase"] || ""
            if (reply["phase"] === "ACCEPTED") {
                host.pairingFinished()
            } else if (host.pairingPhase.length === 0) {
                host.pairingError = error.length > 0 ? error : qsTr("Connect failed")
                host.pairingConnecting = false
            }
        })
        return true
    }

    function copyHarborId() {
        if (host.platform && host.ownHarborId.length > 0)
            host.platform.copyText(host.ownHarborId)
    }

    function pairingAccept() {
        host.request("pairing.accept", {}, (_reply, error) => {
            if (error.length > 0)
                host.pairingError = error
            else
                host.pairingFinished()
        })
    }

    function pairingDecline() {
        host.request("pairing.decline", {}, (_reply, _error) => host.pairingReset())
    }

    function pairingCancel() {
        host.request("pairing.cancel", {}, (_reply, _error) => host.pairingReset())
    }

    property bool pairingConnecting: false

    function pairingReset() {
        host.request("pairing.reset", {})
        host.pairingPhase = ""
        host.pairingError = ""
        host.pairingConnecting = false
        host.pairingPeerHarborId = ""
        host.incomingHarborId = ""
    }

    function pairingPollStatus() {
        // Only an in-flight invite is polled: without one the core has no
        // request to report and the refusal would be pure noise.
        if (!host.pairingConnecting)
            return
        host.request("pairing.status", {}, (reply, error) => {
            if (reply["phase"])
                host.pairingPhase = reply["phase"]
            else if (error.length > 0)
                host.pairingError = error
            if (reply["phase"] === "ACCEPTED")
                host.pairingFinished()
        })
    }

    function pairingPollIncoming() {
        // Incoming and outgoing pairing share one core session. Never let the
        // periodic incoming read replace an invite that is awaiting status.
        if (host.pairingConnecting)
            return
        host.request("pairing.incoming", {}, reply => {
            const request = reply && typeof reply["request"] === "object"
                ? reply["request"] : null
            const requester = request ? String(request["requester_harbor_id"] || "") : ""
            if (reply["has_request"] === true && requester.length > 0) {
                host.incomingHarborId = requester
                host.pairingPhase = "INCOMING"
            } else if (reply["has_request"] !== true) {
                host.incomingHarborId = ""
            }
        })
    }

    function pairingFinished() {
        host.pairingVisible = false
        host.pairingConnecting = false
        host.pairingReset()
        host.refreshServer()
        host.pullPresence()
    }

    function fetchOwnHarborId() {
        host.request("identity.get", {}, (identity, error) => {
            if (error.length === 0)
                host.ownHarborId = String(identity["harbor_id"] || "")
        })
    }

    function refreshAll() {
        host.refreshServer()
        host.fetchOwnHarborId()
        host.request("device.state", {}, (device, _error) => {
            const dev = device["device"] || {}
            host.deviceId = dev["deviceId"] || ""
            host.mobileToMobileBlocked = device["blocked"] === true
            host.isCompanion = device["mode"] === "companion"
        })
        host.refreshAppearance()
        host.pullChat()
        host.pullPresence()
    }

    function pullChat() {
        host.request("direct.state", {}, direct => {
            const messages = direct["messages"] || []
            host.chatConnected = messages.length > 0 || host.sessionValid
            host.chatMessages = messages.map(m => ({
                "id": m["id"], "body": m["body"] || "",
                "outgoing": m["direction"] === "OUTGOING",
                "delivery": m["delivery"] || "", "timestamp": m["timestamp"] || 0
            }))
            const inbound = messages.filter(m => m["direction"] !== "OUTGOING")
            if (inbound.length > 0) {
                const latest = inbound[inbound.length - 1]
                if (host.lastMessageId.length > 0 && latest["id"] !== host.lastMessageId)
                    host.notifyMessage(latest)
                host.lastMessageId = latest["id"] || host.lastMessageId
            } else if (messages.length === 0) {
                host.lastMessageId = ""
            }
        })
    }

    function pullPresence() {
        host.request("presence.state", {}, presence => {
            const partner = presence["partner"]
            if (partner) {
                const state = String(partner["state"] || "OFFLINE").toLowerCase()
                host.partnerState = state === "idle" ? "idle" : (state === "online" ? "online" : "offline")
            } else host.partnerState = "offline"
            host.syncPresenceBar()
            host.notePresenceTransition()
        })
        host.request("profile.state", {}, profile => {
            const peer = profile["partner"] || {}
            const name = peer["displayName"] || ""
            if (name.length > 0 && name !== host.partnerName) {
                host.partnerName = name
                host.sessionValid = true
            }
            host.syncPresenceBar()
        })
    }

    // The persistent bar mirrors a live pairing only: partner name (or a
    // valid session) with the committed state and the shared current app.
    // Unpaired, the service stops instead of pinning a stale offline row.
    function syncPresenceBar() {
        if (host.partnerName.length > 0 || host.sessionValid) {
            host.platform.updatePresenceBar(host.partnerName,
                host.partnerState === "online" ? "Online" : host.partnerState === "idle" ? "Away" : "Offline",
                host.partnerActivity)
        } else {
            host.platform.hidePresenceBar()
        }
    }

    property string lastPartnerState: ""

    function notePresenceTransition() {
        if (host.lastPartnerState.length > 0 && host.lastPartnerState !== host.partnerState) {
            host.request("settings.get", {}, settings => {
                const key = host.partnerState === "online" ? "notifyPartnerOnline"
                    : host.partnerState === "idle" ? "notifyPartnerAway" : "notifyPartnerOffline"
                if (settings[key] !== false && host.partnerName.length > 0)
                    host.platform.postHarborNotification(host.partnerName,
                        host.partnerState === "online" ? qsTr("Online")
                            : host.partnerState === "idle" ? qsTr("Away") : qsTr("Offline"))
            })
        }
        host.lastPartnerState = host.partnerState
    }

    function notifyMessage(message) {
        host.request("settings.get", {}, settings => {
            if (settings["notificationsEnabled"] === false)
                return
            const preview = settings["messagePreviews"] !== false
                ? String(message["body"] || "") : qsTr("New message")
            host.platform.postHarborNotification(
                host.partnerName.length > 0 ? host.partnerName : "Harbor", preview)
        })
    }

    function pushPhoneState() {
        // Intent AND grant: the core stores nothing without both, and the
        // adapter only observes while the toggles below are ON.
        host.request("settings.get", {}, settings => {
          const status = {
            "schemaVersion": 1,
            "deviceType": "mobile",
            "batteryPercent": host.platform.batteryAvailable ? host.platform.batteryPercent : null,
            "charging": host.platform.batteryCharging,
            "phoneActivity": host.platform.phoneActivity.toUpperCase(),
            "lastActiveAt": settings["sharePhoneActivity"] === true
                && host.platform.usagePermission === "granted"
                && host.platform.lastActiveAt > 0 ? host.platform.lastActiveAt : null,
            "currentApp": settings["sharePhoneActivity"] === true
                && host.platform.usagePermission === "granted"
                ? host.platform.currentApp : null,
            "locationSharingEnabled": settings["shareLocation"] === true
                && host.platform.locationPermission === "granted",
            "location": settings["shareLocation"] === true
                && host.platform.locationPermission === "granted"
                && host.platform.locationAvailable ? {
                    "latitude": host.platform.locationLatitude,
                    "longitude": host.platform.locationLongitude,
                    "accuracyMeters": host.platform.locationAccuracyMeters,
                    "updatedAt": host.platform.locationUpdatedAt
                } : null,
            "notificationSharingEnabled": settings["sharePhoneNotifications"] === true
                && host.platform.notificationPermission === "granted"
          }
          // Location rides only with toggle + live fix; absence is honest.
          host.request("mobile.update", status)
          host.phoneActivityPermission = host.platform.usagePermission
          host.locationPermission = host.platform.locationPermission
          host.notificationPermission = host.platform.notificationPermission
          host.microphonePermission = host.platform.microphonePermission
          host.ownNotificationPermission = host.platform.ownNotificationPermission
          host.backgroundLocationPermission = host.platform.backgroundLocationPermission
          host.batteryOptimizationPermission = host.platform.batteryOptimizationPermission
          // Keep service lifetimes derived from effective permission, not
          // merely from the durable intent. Revoking access stops location
          // and notification observation on the next foreground/timer poll.
          host.platform.syncLocationService(
              settings["shareLocation"] === true
              && host.platform.locationPermission === "granted")
          host.platform.setNotificationMirroring(
              settings["sharePhoneNotifications"] === true
              && host.platform.notificationPermission === "granted")
          host.batteryAvailable = host.platform.batteryAvailable
          host.batteryPercent = host.platform.batteryPercent
          host.batteryCharging = host.platform.batteryCharging
          host.phoneActivity = host.platform.phoneActivity
          host.currentApp = host.platform.currentApp
          host.lastActiveAt = host.platform.lastActiveAt
          host.locationAvailable = host.platform.locationAvailable
          host.locationText = host.platform.locationText
          host.locationUpdatedText = host.platform.locationUpdatedText
          host.locationLatitude = host.platform.locationLatitude
          host.locationLongitude = host.platform.locationLongitude
          host.locationAccuracyMeters = host.platform.locationAccuracyMeters
          host.locationUpdatedAt = host.platform.locationUpdatedAt
        })
    }

    Connections {
        target: host.core
        function onRequestFinished(requestId, type, payload, errorCode) {
            const callback = host.pendingRequests[requestId]
            delete host.pendingRequests[requestId]
            if (callback)
                callback(payload || {}, errorCode || "")
        }
        function onDirectUpdated(snapshot) { host.pullChat() }
        function onPresenceUpdated(sides) { host.pullPresence() }
        function onDeviceUpdated(snapshot) { host.refreshAll() }
        function onMobileUpdated(snapshot) { host.pullPartnerPhone(snapshot) }
        function onProfileUpdated(snapshot) { host.pullPresence() }
        function onCallUpdated(snapshot) {
            const phase = String(snapshot["state"] || "IDLE")
            host.callState = phase === "CONNECTED" ? "connected"
                : (phase === "CONNECTING" || phase === "ACCEPTING" || phase === "RECONNECTING") ? "connecting"
                : phase === "FAILED" ? "unavailable"
                : phase === "INCOMING" ? "incoming" : "idle"
            // The core joins mobile endpoints muted; the switch below only
            // ever reflects that committed fact.
            host.microphoneMuted = snapshot["muted"] === true
            if (phase === "FAILED" || phase === "IDLE" || phase === "ENDED")
                host.platform.stopCallAudio()
        }
    }

    Connections {
        target: host.platform
        function onPlatformChanged() {
            // Settings grants (usage access and notification listener) are
            // changed outside the Activity and have no normal callback.
            host.pushPhoneState()
        }
        function onPhoneNotification(appLabel, title, text, postedAt) {
            // This is an intentionally fire-and-forget, display-only event.
            // It is never added to chatMessages or any local model.
            host.request("mobile.notification", {
                "appLabel": appLabel,
                "title": title,
                "text": text,
                "timestamp": postedAt
            })
        }
    }

    function pullPartnerPhone(snapshot) {
        const peer = (snapshot && snapshot["peer"]) || null
        if (peer) {
            const activity = String(peer["phoneActivity"] || "OFFLINE").toLowerCase()
            host.partnerActivity = peer["currentApp"] && activity === "active"
                ? qsTr("Using %1").arg(peer["currentApp"]) : ""
            host.syncPresenceBar()
        }
    }
}
