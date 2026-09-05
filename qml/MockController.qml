pragma Singleton
import QtQuick

// Deterministic, in-memory orchestration for the Harbor prototype. This object
// deliberately uses only fixed data and functional timers. It never reaches
// into networking, audio, process, file, location, clipboard, or persistence
// services.
QtObject {
    id: controller

    // Durations are public so tests and future developer controls can describe
    // the scenarios without duplicating timing knowledge.
    readonly property int connectDelay: 900
    readonly property int reconnectStageDelay: 650
    readonly property int callDelay: 1200
    readonly property int diagnosticsStageDelay: 400
    readonly property int deviceScanStageDelay: 600
    readonly property int pairingRequestDelay: 2100
    readonly property int notificationInterval: 450
    readonly property int toastDisplayDuration: 2600
    readonly property int pushToTalkPulseDuration: 180
    readonly property int shareToggleDelay: 500
    readonly property int pageTransitionDelay: 900
    readonly property int settingsFeedbackDelay: 700
    readonly property int confirmationDuration: 2600
    readonly property int copyFeedbackDuration: 1600

    // Fixed sequences make every run reproducible. Indices are reset by
    // resetSession(), and no sequence depends on wall-clock time.
    readonly property var microphoneLevelSequence: [0.08, 0.18, 0.31, 0.52, 0.74, 0.61, 0.38, 0.22, 0.45, 0.68, 0.49, 0.27]
    readonly property var outputLevelSequence: [0.16, 0.28, 0.43, 0.35, 0.57, 0.71, 0.54, 0.32, 0.24, 0.47, 0.63, 0.39]
    readonly property var diagnosticsSequence: [
        { progress: 20, stage: "route", latency: 24, quality: 84, download: 58, upload: 33 },
        { progress: 45, stage: "latency", latency: 20, quality: 88, download: 64, upload: 37 },
        { progress: 75, stage: "traffic", latency: 19, quality: 90, download: 68, upload: 40 },
        { progress: 100, stage: "complete", latency: 18, quality: 92, download: 71, upload: 42 }
    ]
    readonly property var pairingCodeSequence: ["HBR-7D92", "HBR-4A10", "HBR-8264", "HBR-3518"]
    readonly property var incomingRequestSequence: [
        { name: "Morgan", initials: "MO", code: "482 731" },
        { name: "Avery", initials: "AV", code: "195 264" },
        { name: "Taylor", initials: "TA", code: "730 418" }
    ]
    readonly property var deviceCandidateSequence: [
        { id: "studio-tablet", nameKey: "fixture.device.studioTablet", nameParams: {}, typeKey: "devices.type.tablet", iconName: "tablet", statusKey: "devices.status.availableToReconnect", statusParams: {}, connected: false, primary: false },
        { id: "travel-laptop", nameKey: "fixture.device.travelLaptop", nameParams: {}, typeKey: "devices.type.computer", iconName: "laptop", statusKey: "devices.status.availableToReconnect", statusParams: {}, connected: false, primary: false }
    ]
    // Fictional audio IDs and localization keys shared by onboarding and
    // settings. They describe mock choices and never enumerate hardware.
    readonly property var audioInputOptions: [
        { id: "default-microphone", labelKey: "fixture.audio.input.defaultMicrophone", labelParams: {} },
        { id: "studio-usb-microphone", labelKey: "fixture.audio.input.studioUsbMicrophone", labelParams: {} },
        { id: "webcam-microphone", labelKey: "fixture.audio.input.webcamMicrophone", labelParams: {} }
    ]
    readonly property var audioOutputOptions: [
        { id: "harbor-headphones", labelKey: "fixture.audio.output.harborHeadphones", labelParams: {} },
        { id: "desktop-speakers", labelKey: "fixture.audio.output.desktopSpeakers", labelParams: {} },
        { id: "hdmi-display", labelKey: "fixture.audio.output.hdmiDisplay", labelParams: {} }
    ]
    function audioOption(options, id) {
        for (var index = 0; index < options.length; ++index) {
            if (options[index].id === id)
                return options[index]
        }
        return null
    }

    function audioOptionIndex(options, id) {
        for (var index = 0; index < options.length; ++index) {
            if (options[index].id === id)
                return index
        }
        return -1
    }

    function audioLabel(options, id) {
        var option = audioOption(options, id)
        return option ? I18n.t(option.labelKey, option.labelParams || {}) : String(id)
    }

    readonly property var notificationSequence: [
        {
            category: "call",
            titleKey: "notifications.preview.title",
            titleParams: {},
            descriptionKey: "notifications.preview.description",
            descriptionParams: {}
        },
        {
            category: "game",
            titleKey: "activity.event.gameOpened",
            titleParams: { name: "Taylor", game: "Stardew Valley" },
            descriptionKey: "toast.activity.description",
            descriptionParams: {}
        },
        {
            category: "online",
            titleKey: "toast.connected.title",
            titleParams: {},
            descriptionKey: "toast.connected.description",
            descriptionParams: { name: "Taylor" }
        },
        {
            category: "network",
            titleKey: "network.check.complete.title",
            titleParams: {},
            descriptionKey: "network.check.complete.description",
            descriptionParams: {}
        }
    ]

    // Connection and call operations.
    property string connectionOperation: "idle" // idle, connecting, reconnecting
    property int connectionStage: 0
    readonly property bool connectionBusy: connectionOperation !== "idle"
    property string callOperation: "idle" // idle, connecting
    readonly property bool callBusy: callOperation !== "idle"
    property string shareOperation: "idle" // idle, starting, stopping
    readonly property bool shareBusy: shareOperation !== "idle"

    // Generated visualization levels. These are display-only numbers; no audio
    // device is opened and no microphone samples are read.
    property real microphoneLevel: 0.0
    // The deterministic fixtures never model voice activity; the honest mock
    // answers are always "not speaking", mirroring the bridge contract so
    // views never read an undefined property.
    readonly property bool speaking: false
    // The deterministic fixtures never model the peer's voice; the honest
    // mock answer is always "not speaking".
    readonly property bool remoteSpeaking: false
    property real outputLevel: 0.0
    property real transmitLevel: 0.0
    property int levelFrame: 0
    readonly property bool levelGenerationRunning: AppState.callState === "connected"

    // Network diagnostics.
    property bool diagnosticsRunning: false
    property int diagnosticsProgress: 0
    property string diagnosticsStage: "idle"
    property int diagnosticsSequenceIndex: 0
    property var diagnosticsResult: ({})

    // Device discovery preview.
    property bool deviceScanRunning: false
    property int deviceScanProgress: 0
    property string deviceScanStage: "idle"
    property string nextDeviceScanOutcome: "none" // none, found
    property var discoveredDevices: []
    property int deviceCandidateIndex: 0

    // Pairing and incoming-request state.
    property string pairingMode: "choice" // choice, qr, request, waiting, error, success, incoming
    property string pairingCode: pairingCodeSequence[0]
    property int pairingCodeIndex: 0
    property int pairingCodeSeconds: 59
    property string enteredPairingCode: ""
    property string pairingErrorKey: ""
    property var pairingErrorParams: ({})
    property string pendingPartnerName: "Avery"
    property bool pairingRequestRunning: false
    property bool incomingRequestScheduled: false
    property var incomingRequest: ({ name: "", initials: "", code: "" })
    property int incomingRequestIndex: 0
    property bool mockCopyFeedbackVisible: false
    property string mockCopyTarget: ""

    // Onboarding's local audio visualization.
    property bool audioTestRunning: false
    property int audioTestLevel: 0
    property int audioTestFrame: 0
    readonly property bool audioTestComplete: audioTestLevel >= 100

    // Notification generation and transient UI state.
    property bool notificationBurstRunning: false
    property int notificationBurstRemaining: 0
    property int notificationSequenceIndex: 0
    property int notificationSerial: 1
    property bool notificationClearArmed: false
    property bool previewNotificationPending: false

    // Canonical toast state. The host renders only activeToast; this controller
    // owns ordering, expiry, dismissal, and compatibility ingestion.
    property var toastQueue: []
    property var activeToast: ({})
    property int toastSerial: 0
    readonly property int pendingToastCount: toastQueue.length
    readonly property bool toastActive: activeToast.toastId !== undefined

    // Page-state transitions and delayed feedback for session-only settings.
    property var pageTransitionQueue: []
    property string activePageTransition: ""
    property string activePageTargetState: ""
    readonly property bool pageTransitionRunning: activePageTransition.length > 0
    property bool settingsApplied: true
    property string settingsFeedbackKey: "settings.savedAutomatically"
    property var settingsFeedbackParams: ({})

    // Session clocks use counters rather than Date or any wall-clock API.
    property int sessionElapsedSeconds: 8077
    property int callElapsedSeconds: 4122

    signal diagnosticsCompleted(var result)
    signal deviceScanCompleted(string outcome, var devices)
    signal pairingCompleted(string partnerName)
    signal incomingPairingDeclined(string partnerName)
    signal notificationAdded(string notificationId)
    signal pageTransitionCompleted(string page, string state)

    function _pad2(value) {
        return value < 10 ? "0" + value : String(value)
    }

    function _clockText(totalSeconds) {
        var safe = Math.max(0, Math.floor(Number(totalSeconds) || 0))
        var hours = Math.floor(safe / 3600)
        var minutes = Math.floor((safe % 3600) / 60)
        var seconds = safe % 60
        return _pad2(hours) + ":" + _pad2(minutes) + ":" + _pad2(seconds)
    }

    function _syncClockLabels() {
        AppState.sessionTime = _clockText(sessionElapsedSeconds)
        AppState.callTime = _clockText(callElapsedSeconds)
    }

    function _applyConnectionState(nextState) {
        var normalized = AppState.normalizeConnectionState(nextState)
        AppState.connectionState = normalized
        if (normalized === "connected") {
            AppState.partnerState = "online"
        } else if (normalized === "reconnecting" || normalized === "connecting") {
            _releasePushToTalk()
        } else {
            AppState.partnerState = "offline"
            AppState.callState = "idle"
            _releasePushToTalk()
            outputLevel = 0
        }
        return normalized
    }

    function setConnectionScenario(nextState, autoComplete) {
        cancelConnectionOperation()
        var normalized = AppState.normalizeConnectionState(nextState)
        if (normalized === "connected") {
            _applyConnectionState("connected")
            queueLocalizedToast("online", "toast.connected.title", {},
                                "toast.connected.description", { name: AppState.partnerName })
        } else if (normalized === "disconnected") {
            _applyConnectionState("disconnected")
            queueLocalizedToast("offline", "toast.disconnected.title", {},
                                "toast.disconnected.description", {})
        } else if (normalized === "reconnecting") {
            _beginReconnect(autoComplete !== false)
        } else {
            _applyConnectionState("connecting")
            if (autoComplete !== false) {
                connectionOperation = "connecting"
                connectionStage = 0
                connectionTimer.interval = connectDelay
                connectionTimer.restart()
            }
        }
        return normalized
    }

    function connect() {
        return setConnectionScenario("connecting", true)
    }

    function reconnect() {
        return setConnectionScenario("reconnecting", true)
    }

    function disconnect() {
        return setConnectionScenario("disconnected", false)
    }

    function _beginReconnect(autoComplete) {
        _applyConnectionState("reconnecting")
        queueLocalizedToast("network", "toast.reconnecting.title", {},
                            "toast.reconnecting.description", { name: AppState.partnerName })
        if (autoComplete) {
            connectionOperation = "reconnecting"
            connectionStage = 0
            connectionTimer.interval = reconnectStageDelay
            connectionTimer.restart()
        }
    }

    function _advanceConnection() {
        if (connectionOperation === "reconnecting" && connectionStage === 0) {
            connectionStage = 1
            _applyConnectionState("connecting")
            connectionTimer.interval = reconnectStageDelay
            connectionTimer.restart()
            return
        }
        if (connectionOperation === "connecting" || connectionOperation === "reconnecting") {
            connectionOperation = "idle"
            connectionStage = 0
            _applyConnectionState("connected")
            queueLocalizedToast("online", "toast.connected.title", {},
                                "toast.connected.description", { name: AppState.partnerName })
        }
    }

    function cancelConnectionOperation() {
        connectionTimer.stop()
        connectionOperation = "idle"
        connectionStage = 0
    }

    function startCall() {
        if (AppState.connectionState !== "connected" || AppState.partnerState === "offline") {
            AppState.callState = "unavailable"
            _releasePushToTalk()
            return false
        }
        if (AppState.callState === "connected" || callBusy)
            return false
        AppState.callState = "connecting"
        callOperation = "connecting"
        queueLocalizedToast("call", "call.toast.calling.title", { name: AppState.partnerName },
                            "call.toast.calling.description", {})
        callTimer.interval = callDelay
        callTimer.restart()
        return true
    }

    function completeCall() {
        callTimer.stop()
        callOperation = "idle"
        if (AppState.connectionState !== "connected" || AppState.partnerState === "offline") {
            AppState.callState = "unavailable"
            _releasePushToTalk()
            return false
        }
        AppState.callState = "connected"
        queueLocalizedToast("call", "call.toast.connected.title", {},
                            "call.toast.connected.description", {})
        return true
    }

    function endCall() {
        callTimer.stop()
        callOperation = "idle"
        AppState.callState = "idle"
        _releasePushToTalk()
        outputLevel = 0
        queueLocalizedToast("call", "call.toast.ended.title", {},
                            "call.toast.ended.description", {})
    }

    function toggleCall() {
        if (AppState.callState === "connected" || AppState.callState === "connecting")
            endCall()
        else
            startCall()
    }

    // Deterministic scenario step: jump the simulated call straight to a
    // terminal state without waiting for the connect timer.
    function forceCallState(state) {
        callTimer.stop()
        callOperation = "idle"
        AppState.setCallState(state)
        _releasePushToTalk()
        return true
    }

    function _releasePushToTalk() {
        pushToTalkPulseTimer.stop()
        AppState.pushToTalkActive = false
        microphoneLevel = 0
        transmitLevel = 0
    }

    // Screen share mirrors the live machine one-to-one: it starts only inside
    // a connected call and never outlives it (AppState resets it on teardown).
    function startScreenShare() {
        if (AppState.callState !== "connected" || shareOperation !== "idle")
            return false
        shareOperation = "starting"
        shareTimer.interval = shareToggleDelay
        shareTimer.restart()
        return true
    }

    function stopScreenShare() {
        if (AppState.callShareState !== "SHARING" || shareOperation !== "idle")
            return false
        shareOperation = "stopping"
        shareTimer.interval = shareToggleDelay
        shareTimer.restart()
        return true
    }

    function _completeShareToggle() {
        var target = shareOperation === "starting" ? "SHARING" : "NOT_SHARING"
        shareOperation = "idle"
        AppState.setCallShareState(target)
    }

    function setMuted(muted) {
        AppState.microphoneMuted = Boolean(muted)
        if (AppState.microphoneMuted)
            _releasePushToTalk()
        return AppState.microphoneMuted
    }

    function toggleMute() {
        return setMuted(!AppState.microphoneMuted)
    }

    function setPushToTalk(active) {
        pushToTalkPulseTimer.stop()
        var enabled = Boolean(active) && AppState.callState === "connected"
            && AppState.connectionState === "connected" && !AppState.microphoneMuted
        AppState.pushToTalkActive = enabled
        if (!enabled) {
            microphoneLevel = 0
            transmitLevel = 0
        }
        return enabled
    }

    // Chat/file fixtures mirror the core's sanitized direct snapshot exactly:
    // the same field names, the same delivery/transfer vocabularies, fixed
    // values only. Nothing here touches the filesystem.
    property int _fixtureChatSequence: 0

    function sendMessage(body) {
        var text = String(body).trim()
        if (text.length === 0)
            return false
        _fixtureChatSequence++
        var messages = AppState.chatMessages.slice(0)
        messages.push({
            id: "fixture-message-" + _fixtureChatSequence,
            body: text,
            direction: "OUTGOING",
            delivery: "DELIVERED",
            timestamp: 1788300000 + _fixtureChatSequence
        })
        AppState.setDirectState(messages, AppState.transfers)
        return true
    }

    function offerFile(path) {
        var name = String(path).split("/").pop()
        if (name.length === 0)
            return false
        _fixtureChatSequence++
        var transfers = AppState.transfers.slice(0)
        transfers.push({
            id: "fixture-transfer-" + _fixtureChatSequence,
            direction: "OUTGOING",
            state: "OFFERED",
            name: name,
            size: 262144,
            receivedBytes: 0,
            peerReceived: false,
            expired: false
        })
        AppState.setDirectState(AppState.chatMessages, transfers)
        return true
    }

    function acceptTransfer(id) {
        return _mutateFixtureTransfer(id, { state: "COMPLETED", receivedBytes: 262144 })
    }

    function rejectTransfer(id) {
        return _mutateFixtureTransfer(id, { state: "CANCELED" })
    }

    function cancelTransfer(id) {
        return _mutateFixtureTransfer(id, { state: "CANCELED" })
    }

    function _mutateFixtureTransfer(id, changes) {
        var transfers = AppState.transfers.slice(0)
        for (var index = 0; index < transfers.length; ++index) {
            if (transfers[index].id !== id)
                continue
            var merged = {}
            for (var field in transfers[index])
                merged[field] = transfers[index][field]
            for (var changedField in changes)
                merged[changedField] = changes[changedField]
            transfers[index] = merged
            AppState.setDirectState(AppState.chatMessages, transfers)
            return true
        }
        return false
    }

    function pulsePushToTalk(duration) {
        if (!setPushToTalk(true))
            return false
        pushToTalkPulseTimer.interval = Math.max(1, Number(duration) || pushToTalkPulseDuration)
        pushToTalkPulseTimer.restart()
        return true
    }

    // Audio fixtures mirror the live contract: selection and volumes land in
    // the session state the same way the bridge lands core echoes.
    function selectAudioDevices(inputId, outputId) {
        if (audioOption(audioInputOptions, inputId) === null
            || audioOption(audioOutputOptions, outputId) === null)
            return false
        AppState.inputDevice = String(inputId)
        AppState.outputDevice = String(outputId)
        return true
    }

    function setAudioVolumes(inputVolume, outputVolume) {
        AppState.microphoneVolume = Math.min(1, Math.max(0, Number(inputVolume)))
        AppState.outputVolume = Math.min(1, Math.max(0, Number(outputVolume)))
        return true
    }

    // An inbound ring is a decision: accepting walks the same simulated
    // connection path as an outgoing call, declining ends it right here.
    function acceptIncomingCall() {
        if (AppState.callState !== "incoming")
            return false
        callTimer.stop()
        callOperation = "connecting"
        AppState.callState = "connecting"
        callTimer.interval = callDelay
        callTimer.restart()
        return true
    }

    function declineIncomingCall() {
        if (AppState.callState !== "incoming")
            return false
        callTimer.stop()
        callOperation = "idle"
        AppState.setCallState("idle")
        return true
    }

    function _advanceGeneratedLevels() {
        if (AppState.callState !== "connected") {
            microphoneLevel = 0
            outputLevel = 0
            transmitLevel = 0
            return
        }
        var index = levelFrame % microphoneLevelSequence.length
        outputLevel = outputLevelSequence[index % outputLevelSequence.length]
        if (AppState.pushToTalkActive && !AppState.microphoneMuted) {
            microphoneLevel = microphoneLevelSequence[index]
            transmitLevel = microphoneLevel
        } else {
            microphoneLevel = 0
            transmitLevel = 0
        }
        levelFrame = (levelFrame + 1) % microphoneLevelSequence.length
    }

    function runDiagnostics() {
        if (diagnosticsRunning)
            return false
        diagnosticsRunning = true
        diagnosticsProgress = 0
        diagnosticsStage = "starting"
        diagnosticsSequenceIndex = 0
        diagnosticsResult = ({})
        diagnosticsTimer.interval = diagnosticsStageDelay
        diagnosticsTimer.restart()
        return true
    }

    function cancelDiagnostics() {
        diagnosticsTimer.stop()
        diagnosticsRunning = false
        diagnosticsProgress = 0
        diagnosticsStage = "idle"
        diagnosticsSequenceIndex = 0
    }

    function _advanceDiagnostics() {
        if (!diagnosticsRunning)
            return
        var sample = diagnosticsSequence[diagnosticsSequenceIndex]
        diagnosticsProgress = sample.progress
        diagnosticsStage = sample.stage
        AppState.latency = sample.latency
        AppState.networkQuality = sample.quality
        AppState.download = sample.download
        AppState.upload = sample.upload
        if (diagnosticsSequenceIndex < diagnosticsSequence.length - 1) {
            diagnosticsSequenceIndex++
            diagnosticsTimer.restart()
            return
        }
        diagnosticsRunning = false
        diagnosticsResult = {
            status: "healthy",
            latency: sample.latency,
            quality: sample.quality,
            download: sample.download,
            upload: sample.upload,
            routeNodes: AppState.routeNodes.slice(0)
        }
        _appendHistorySample("downloadHistory", sample.download, 12)
        _appendHistorySample("uploadHistory", sample.upload, 12)
        queueLocalizedToast("network", "network.check.complete.title", {},
                            "network.check.complete.description", {})
        diagnosticsCompleted(diagnosticsResult)
    }

    function _appendHistorySample(collection, value, maximumLength) {
        var values = AppState[collection].slice(0)
        values.push(value)
        while (values.length > maximumLength)
            values.shift()
        AppState.replaceItems(collection, values)
    }

    function setNextDeviceScanOutcome(outcome) {
        nextDeviceScanOutcome = outcome === "found" ? "found" : "none"
    }

    function scanDevices(outcome) {
        if (deviceScanRunning)
            return false
        if (outcome !== undefined)
            setNextDeviceScanOutcome(outcome)
        deviceScanRunning = true
        deviceScanProgress = 0
        deviceScanStage = "starting"
        discoveredDevices = []
        deviceScanTimer.interval = deviceScanStageDelay
        deviceScanTimer.restart()
        return true
    }

    function cancelDeviceScan() {
        deviceScanTimer.stop()
        deviceScanRunning = false
        deviceScanProgress = 0
        deviceScanStage = "idle"
    }

    function _advanceDeviceScan() {
        if (!deviceScanRunning)
            return
        if (deviceScanProgress === 0) {
            deviceScanProgress = 50
            deviceScanStage = "looking"
            deviceScanTimer.restart()
            return
        }
        deviceScanProgress = 100
        deviceScanStage = "complete"
        deviceScanRunning = false
        if (nextDeviceScanOutcome === "found") {
            var candidate = deviceCandidateSequence[deviceCandidateIndex % deviceCandidateSequence.length]
            deviceCandidateIndex = (deviceCandidateIndex + 1) % deviceCandidateSequence.length
            if (AppState.itemIndex("devices", candidate.id) < 0)
                AppState.appendItem("devices", candidate)
            discoveredDevices = [candidate]
        } else {
            discoveredDevices = []
            queueLocalizedToast("system", "devices.nearby.complete.title", {},
                                "devices.nearby.complete.description", {})
        }
        var completedOutcome = nextDeviceScanOutcome
        nextDeviceScanOutcome = "none"
        deviceScanCompleted(completedOutcome, discoveredDevices.slice(0))
    }

    function setDeviceConnected(deviceId, connected) {
        var index = AppState.itemIndex("devices", deviceId)
        if (index < 0)
            return false
        var isConnected = Boolean(connected)
        return AppState.updateItem("devices", deviceId, {
            connected: isConnected,
            statusKey: isConnected ? "devices.status.connectedNow" : "devices.status.availableToReconnect",
            statusParams: {}
        })
    }

    function removeDevice(deviceId) {
        return AppState.removeItem("devices", deviceId)
    }

    function openPairing(initialMode) {
        cancelPairingRequest()
        incomingRequestTimer.stop()
        incomingRequestScheduled = false
        pairingErrorKey = ""
        pairingErrorParams = ({})
        enteredPairingCode = ""
        pendingPartnerName = "Avery"
        AppState.pairingVisible = true
        setPairingMode(initialMode || "choice")
    }

    function closePairing() {
        cancelPairingRequest()
        pairingMode = "choice"
        pairingCodeSeconds = 59
        mockCopyFeedbackVisible = false
        copyFeedbackTimer.stop()
        AppState.pairingVisible = false
    }

    function setPairingMode(mode) {
        var values = ["choice", "qr", "request", "waiting", "error", "success", "incoming"]
        pairingMode = values.indexOf(mode) >= 0 ? mode : "choice"
        if (pairingMode === "qr")
            pairingCodeSeconds = 59
        return pairingMode
    }

    function beginQrPairing() {
        pairingErrorKey = ""
        pairingErrorParams = ({})
        setPairingMode("qr")
    }

    function refreshPairingCode() {
        pairingCodeIndex = (pairingCodeIndex + 1) % pairingCodeSequence.length
        pairingCode = pairingCodeSequence[pairingCodeIndex]
        pairingCodeSeconds = 59
        return pairingCode
    }

    function normalizePairingCode(value) {
        return String(value || "").toUpperCase().replace(/[^A-Z0-9]/g, "")
    }

    function submitPairingCode(value) {
        cancelPairingRequest()
        var normalized = normalizePairingCode(value)
        enteredPairingCode = normalized
        pairingErrorParams = ({})
        if (normalized.length < 6) {
            pairingErrorKey = "pairing.error.tooShort"
            pairingMode = "error"
            return false
        }
        if (normalized.indexOf("ERR") === 0 || normalized.indexOf("000") >= 0) {
            pairingErrorKey = "pairing.error.expired"
            pairingMode = "error"
            return false
        }
        pairingErrorKey = ""
        pendingPartnerName = normalized.indexOf("TAYLOR") >= 0 ? "Taylor" : "Avery"
        pairingMode = "waiting"
        pairingRequestRunning = true
        pairingRequestTimer.interval = pairingRequestDelay
        pairingRequestTimer.restart()
        return true
    }

    function cancelPairingRequest() {
        pairingRequestTimer.stop()
        pairingRequestRunning = false
        if (pairingMode === "waiting")
            pairingMode = "request"
    }

    function completePairing(partnerName) {
        pairingRequestTimer.stop()
        pairingRequestRunning = false
        pendingPartnerName = String(partnerName || pendingPartnerName || "Avery")
        pairingMode = "success"
        AppState.updatePartnerProfile({
            name: pendingPartnerName,
            initials: AppState.initialsFor(pendingPartnerName),
            presence: "online"
        })
        _applyConnectionState("connected")
        queueLocalizedToast("online", "toast.harborReady.title", {},
                            "toast.harborReady.description", { name: pendingPartnerName })
        pairingCompleted(pendingPartnerName)
        return pendingPartnerName
    }

    function mockCopy(value, target) {
        // Feedback only: the value is intentionally not sent to a clipboard.
        mockCopyTarget = String(target || "pairingCode")
        mockCopyFeedbackVisible = true
        copyFeedbackTimer.interval = copyFeedbackDuration
        copyFeedbackTimer.restart()
        return String(value || "")
    }

    function scheduleIncomingRequest(delay) {
        if (incomingRequestScheduled
                || (AppState.pairingVisible && pairingMode === "incoming"))
            return false
        incomingRequestScheduled = true
        incomingRequestTimer.interval = Math.max(1, Number(delay) || 1200)
        incomingRequestTimer.restart()
        return true
    }

    function showIncomingRequest(request) {
        incomingRequestTimer.stop()
        incomingRequestScheduled = false
        var source = request || incomingRequestSequence[incomingRequestIndex % incomingRequestSequence.length]
        if (!request)
            incomingRequestIndex = (incomingRequestIndex + 1) % incomingRequestSequence.length
        incomingRequest = {
            name: String(source.name || "Someone"),
            initials: String(source.initials || AppState.initialsFor(source.name || "Someone")),
            code: String(source.code || "482 731")
        }
        AppState.pairingVisible = true
        setPairingMode("incoming")
        return incomingRequest
    }

    function acceptIncomingRequest() {
        if (pairingMode !== "incoming")
            return false
        var name = incomingRequest.name
        incomingRequest = ({ name: "", initials: "", code: "" })
        completePairing(name)
        return true
    }

    function declineIncomingRequest() {
        if (pairingMode !== "incoming")
            return false
        var name = incomingRequest.name
        incomingRequest = ({ name: "", initials: "", code: "" })
        closePairing()
        incomingPairingDeclined(name)
        return true
    }

    function startAudioTest() {
        audioTestTimer.stop()
        audioTestRunning = true
        audioTestLevel = 0
        audioTestFrame = 0
        audioTestTimer.restart()
    }

    function cancelAudioTest() {
        audioTestTimer.stop()
        audioTestRunning = false
        audioTestLevel = 0
        audioTestFrame = 0
    }

    function _advanceAudioTest() {
        if (!audioTestRunning)
            return
        var increments = [7, 9, 6, 12, 8, 11, 7, 10, 8, 9, 6, 7]
        audioTestLevel = Math.min(100, audioTestLevel + increments[audioTestFrame % increments.length])
        audioTestFrame++
        if (audioTestLevel >= 100) {
            audioTestRunning = false
            audioTestTimer.stop()
        }
    }

    function previewNotification() {
        if (previewNotificationPending)
            return false
        previewNotificationPending = true
        previewNotificationTimer.restart()
        return true
    }

    function startNotificationBurst(count) {
        if (notificationBurstRunning)
            return false
        var requested = count === undefined ? notificationSequence.length : Math.floor(Number(count))
        notificationBurstRemaining = Math.max(1, Math.min(notificationSequence.length, requested || 1))
        notificationBurstRunning = true
        notificationBurstTimer.interval = notificationInterval
        notificationBurstTimer.restart()
        return true
    }

    function cancelNotificationBurst() {
        notificationBurstTimer.stop()
        notificationBurstRunning = false
        notificationBurstRemaining = 0
    }

    function _addNextNotification(withToast) {
        var template = notificationSequence[notificationSequenceIndex % notificationSequence.length]
        notificationSequenceIndex = (notificationSequenceIndex + 1) % notificationSequence.length
        var id = "mock-notification-" + notificationSerial
        notificationSerial++
        AppState.prependItem("notifications", {
            id: id,
            category: template.category,
            titleKey: template.titleKey,
            titleParams: template.titleParams,
            descriptionKey: template.descriptionKey,
            descriptionParams: template.descriptionParams,
            timeKey: "common.time.now",
            timeParams: {},
            unread: true
        })
        if (withToast !== false)
            queueLocalizedToast(template.category, template.titleKey, template.titleParams,
                                template.descriptionKey, template.descriptionParams)
        notificationAdded(id)
        return id
    }

    function _advanceNotificationBurst() {
        if (!notificationBurstRunning)
            return
        _addNextNotification(true)
        notificationBurstRemaining--
        if (notificationBurstRemaining > 0) {
            notificationBurstTimer.restart()
        } else {
            notificationBurstRunning = false
        }
    }

    function armNotificationClear() {
        notificationClearArmed = true
        notificationClearTimer.interval = confirmationDuration
        notificationClearTimer.restart()
    }

    function clearNotificationsWithConfirmation() {
        if (!notificationClearArmed) {
            armNotificationClear()
            return false
        }
        notificationClearTimer.stop()
        notificationClearArmed = false
        AppState.clearItems("notifications")
        return true
    }

    function markNotificationRead(notificationId, read) {
        return AppState.updateItem("notifications", notificationId,
                                   { unread: read === undefined ? false : !Boolean(read) })
    }

    function markAllNotificationsRead() {
        AppState.markAllNotificationsRead()
    }

    function dismissNotification(notificationId) {
        return AppState.removeItem("notifications", notificationId)
    }

    function queueLocalizedToast(category, titleKey, titleParams, descriptionKey, descriptionParams) {
        var queued = toastQueue.slice(0)
        toastSerial++
        queued.push({
            toastId: "toast-" + toastSerial,
            category: category || "system",
            titleKey: titleKey || "",
            titleParams: titleParams || {},
            descriptionKey: descriptionKey || "",
            descriptionParams: descriptionParams || {}
        })
        toastQueue = queued
        if (!toastTimer.running && !toastActive)
            _dispatchNextToast()
    }

    function _dispatchNextToast() {
        if (toastQueue.length === 0) {
            activeToast = ({})
            return
        }
        var queued = toastQueue.slice(0)
        var next = queued.shift()
        toastQueue = queued
        activeToast = next
        AppState.requestLocalizedToast(next.category, next.titleKey, next.titleParams,
                                       next.descriptionKey, next.descriptionParams)
        toastTimer.interval = toastDisplayDuration
        toastTimer.restart()
    }

    function dismissActiveToast() {
        toastTimer.stop()
        activeToast = ({})
        _dispatchNextToast()
    }

    function clearToastQueue() {
        toastTimer.stop()
        toastQueue = []
        activeToast = ({})
    }

    function setPageState(page, state) {
        return AppState.setPageState(page, state)
    }

    function transitionPage(page, targetState, delay) {
        var validTarget = AppState.containsValue(AppState.pageStateValues, targetState) ? targetState : "content"
        AppState.setPageState(page, "loading")
        var queued = pageTransitionQueue.slice(0)
        queued.push({
            page: String(page),
            state: validTarget,
            delay: Math.max(1, Number(delay) || pageTransitionDelay)
        })
        pageTransitionQueue = queued
        if (!pageStateTimer.running && !pageTransitionRunning)
            _startNextPageTransition()
        return true
    }

    function _startNextPageTransition() {
        if (pageTransitionQueue.length === 0) {
            activePageTransition = ""
            activePageTargetState = ""
            return
        }
        var queued = pageTransitionQueue.slice(0)
        var next = queued.shift()
        pageTransitionQueue = queued
        activePageTransition = next.page
        activePageTargetState = next.state
        pageStateTimer.interval = next.delay
        pageStateTimer.restart()
    }

    function _finishPageTransition() {
        var page = activePageTransition
        var state = activePageTargetState
        AppState.setPageState(page, state)
        activePageTransition = ""
        activePageTargetState = ""
        pageTransitionCompleted(page, state)
        _startNextPageTransition()
    }

    function cancelPageTransitions() {
        pageStateTimer.stop()
        pageTransitionQueue = []
        activePageTransition = ""
        activePageTargetState = ""
    }

    function resetPageStates() {
        cancelPageTransitions()
        var pages = ["home", "call", "activity", "network", "devices", "profile", "settings", "notifications", "pairing"]
        for (var index = 0; index < pages.length; ++index)
            AppState.setPageState(pages[index], "content")
    }

    function markSettingChanged(statusKey, params) {
        settingsApplied = false
        settingsFeedbackKey = statusKey || "settings.saving.startup"
        settingsFeedbackParams = params || {}
        settingsSaveTimer.interval = settingsFeedbackDelay
        settingsSaveTimer.restart()
    }

    function _finishSettingSave() {
        settingsApplied = true
        settingsFeedbackKey = "settings.savedNow"
        settingsFeedbackParams = ({})
    }

    function resetSession() {
        connectionTimer.stop()
        callTimer.stop()
        shareTimer.stop()
        diagnosticsTimer.stop()
        deviceScanTimer.stop()
        pairingRequestTimer.stop()
        incomingRequestTimer.stop()
        copyFeedbackTimer.stop()
        audioTestTimer.stop()
        notificationBurstTimer.stop()
        previewNotificationTimer.stop()
        notificationClearTimer.stop()
        toastTimer.stop()
        pageStateTimer.stop()
        settingsSaveTimer.stop()

        AppState.resetSession()

        // The deterministic world simulates an already-paired session: the
        // fixture peer mirrors partnerName so gate surfaces stay hidden.
        AppState.setPairedPeers([{ deviceId: "fixture-partner-device",
                                   harborId: "HBR-5E20-9B31" }])

        _fixtureChatSequence = 0

        connectionOperation = "idle"
        connectionStage = 0
        callOperation = "idle"
        shareOperation = "idle"
        microphoneLevel = 0
        outputLevel = 0
        transmitLevel = 0
        levelFrame = 0

        diagnosticsRunning = false
        diagnosticsProgress = 0
        diagnosticsStage = "idle"
        diagnosticsSequenceIndex = 0
        diagnosticsResult = ({})

        deviceScanRunning = false
        deviceScanProgress = 0
        deviceScanStage = "idle"
        nextDeviceScanOutcome = "none"
        discoveredDevices = []
        deviceCandidateIndex = 0

        pairingMode = "choice"
        pairingCodeIndex = 0
        pairingCode = pairingCodeSequence[0]
        pairingCodeSeconds = 59
        enteredPairingCode = ""
        pairingErrorKey = ""
        pairingErrorParams = ({})
        pendingPartnerName = "Avery"
        pairingRequestRunning = false
        incomingRequestScheduled = false
        incomingRequest = ({ name: "", initials: "", code: "" })
        incomingRequestIndex = 0
        mockCopyFeedbackVisible = false
        mockCopyTarget = ""

        audioTestRunning = false
        audioTestLevel = 0
        audioTestFrame = 0

        notificationBurstRunning = false
        notificationBurstRemaining = 0
        notificationSequenceIndex = 0
        notificationSerial = 1
        notificationClearArmed = false
        previewNotificationPending = false
        toastQueue = []
        activeToast = ({})

        pageTransitionQueue = []
        activePageTransition = ""
        activePageTargetState = ""
        settingsApplied = true
        settingsFeedbackKey = "settings.savedAutomatically"
        settingsFeedbackParams = ({})

        toastSerial = 0

        sessionElapsedSeconds = 8077
        callElapsedSeconds = 4122
        _syncClockLabels()
    }

    property Timer connectionTimer: Timer {
        onTriggered: controller._advanceConnection()
    }

    property Timer callTimer: Timer {
        onTriggered: controller.completeCall()
    }

    property Timer shareTimer: Timer {
        onTriggered: controller._completeShareToggle()
    }

    // A keyboard or hold-release gap never leaves transmission stuck on.
    property Timer pushToTalkPulseTimer: Timer {
        onTriggered: controller.setPushToTalk(false)
    }

    property Timer levelTimer: Timer {
        interval: 110
        repeat: true
        running: controller.levelGenerationRunning
        onRunningChanged: {
            if (!running) {
                controller.microphoneLevel = 0
                controller.outputLevel = 0
                controller.transmitLevel = 0
            }
        }
        onTriggered: controller._advanceGeneratedLevels()
    }

    property Timer diagnosticsTimer: Timer {
        onTriggered: controller._advanceDiagnostics()
    }

    property Timer deviceScanTimer: Timer {
        onTriggered: controller._advanceDeviceScan()
    }

    property Timer pairingRequestTimer: Timer {
        onTriggered: controller.completePairing(controller.pendingPartnerName)
    }

    property Timer pairingCodeTimer: Timer {
        interval: 1000
        repeat: true
        running: controller.pairingMode === "qr" && AppState.pairingVisible
        onTriggered: {
            if (controller.pairingCodeSeconds > 0)
                controller.pairingCodeSeconds--
            else
                controller.refreshPairingCode()
        }
    }

    property Timer incomingRequestTimer: Timer {
        onTriggered: controller.showIncomingRequest()
    }

    property Timer copyFeedbackTimer: Timer {
        onTriggered: {
            controller.mockCopyFeedbackVisible = false
            controller.mockCopyTarget = ""
        }
    }

    property Timer audioTestTimer: Timer {
        interval: 90
        repeat: true
        onTriggered: controller._advanceAudioTest()
    }

    property Timer notificationBurstTimer: Timer {
        onTriggered: controller._advanceNotificationBurst()
    }

    property Timer previewNotificationTimer: Timer {
        interval: 900
        onTriggered: {
            controller.previewNotificationPending = false
            controller._addNextNotification(true)
        }
    }

    property Timer notificationClearTimer: Timer {
        onTriggered: controller.notificationClearArmed = false
    }

    property Timer toastTimer: Timer {
        onTriggered: {
            controller.activeToast = ({})
            controller._dispatchNextToast()
        }
    }

    property Timer pageStateTimer: Timer {
        onTriggered: controller._finishPageTransition()
    }

    property Timer settingsSaveTimer: Timer {
        onTriggered: controller._finishSettingSave()
    }

    property Timer sessionClockTimer: Timer {
        interval: 1000
        repeat: true
        running: true
        onTriggered: {
            controller.sessionElapsedSeconds++
            if (AppState.callState === "connected")
                controller.callElapsedSeconds++
            controller._syncClockLabels()
        }
    }

    Component.onCompleted: _syncClockLabels()
}
