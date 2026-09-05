pragma ComponentBehavior: Bound
import QtQml

// Production call provider between the supervised Rust core and the call
// surfaces (CallView, SettingsView). It mirrors the MockController call
// contract one-to-one (properties, actions) so the views swap providers
// without knowing which one is live.
//
// The core owns the authoritative call state; this bridge mirrors its
// sanitized snapshots (IDLE/INCOMING/CONNECTING/RECONNECTING/CONNECTED/FAILED +
// muted) onto the AppState vocabulary the views already read. No SDP,
// candidate, or media worker material crosses here. A dead core takes its call
// with it, so the mirrored state drops to idle instead of surviving as a
// stale fact.
//
// The worker's audio facts are mirrored as bounded, sanitized numbers: device
// ids/names, the two volumes, and the live voice levels behind the speaking
// indicators. Push-to-talk forwards the hold through the core (mute always
// wins), screen share is mirrored from the share machine
// (NOT_SHARING ⇄ SHARING), and their refusals localize through requestFailed.
//
// The facade is a C++ context property, so this glue file is deliberately
// dynamically typed; qmllint cannot know its members.
// qmllint disable missing-property
QtObject {
    id: provider

    property QtObject facade: null
    readonly property bool live: facade !== null && facade.coreReady
    // Tracks mirrored live state so a later core fault clears it instead of
    // leaving a stale connecting/unavailable call on screen.
    property bool hadLive: false

    // ---- MockController call contract -------------------------------------
    readonly property bool callBusy: AppState.callState === "connecting"
    // Live capture level from the worker's voice facts; flat zero while inert.
    readonly property real microphoneLevel: live ? facade.voiceLevel : 0.0
    readonly property bool speaking: live ? facade.speaking : false
    readonly property bool remoteSpeaking: live ? facade.remoteSpeaking : false
    // Sanitized device lists ({id, name, isDefault}); empty means the worker
    // has none and the views say so.
    readonly property var audioInputOptions: live ? facade.audioInputs : []
    readonly property var audioOutputOptions: live ? facade.audioOutputs : []
    // Share requests are in flight only until the core replies; the bridge has
    // no extra phase to represent.
    readonly property bool shareBusy: false

    // A screen-reader or test activation sends one bounded hold owned here.
    property Timer pttPulseTimer: Timer {
        interval: 800
        repeat: false
        onTriggered: provider.setPushToTalk(false)
    }

    function audioOptionIndex(options, id) {
        for (var index = 0; index < options.length; ++index) {
            if (String(options[index].id) === String(id))
                return index
        }
        return -1
    }

    function audioLabel(options, id) {
        var index = audioOptionIndex(options, id)
        if (index < 0)
            return String(id)
        var device = options[index]
        return device.isDefault
               ? I18n.t("call.audio.defaultDevice") + " · " + String(device.name)
               : String(device.name)
    }

    function startCall() {
        if (!live)
            return
        // The reply and the following call.state_changed event both land in
        // onCallChanged; failures arrive through requestFailed with a key.
        facade.startCall()
    }

    function endCall() {
        if (!live)
            return
        facade.endCall()
    }

    function toggleMute() {
        if (!live)
            return false
        // The facade property is the authority; the mirrored reply confirms.
        facade.setCallMuted(!facade.callMuted)
        return facade.callMuted
    }

    function setMuted(muted) {
        if (!live)
            return
        facade.setCallMuted(Boolean(muted))
    }

    // Push to talk forwards the hold through the core. Mute wins before the
    // request: a muted microphone never transmits, live or otherwise.
    function setPushToTalk(active) {
        if (!live)
            return false
        pttPulseTimer.stop()
        var allowed = Boolean(active) && AppState.callState === "connected"
            && AppState.connectionState === "connected"
            && !AppState.microphoneMuted && AppState.pushToTalkEnabled
        facade.setPushToTalkActive(allowed)
        AppState.pushToTalkActive = allowed
        return allowed
    }

    function pulsePushToTalk(durationMs) {
        if (!setPushToTalk(true))
            return false
        pttPulseTimer.interval = Math.max(100, Number(durationMs) || 800)
        pttPulseTimer.restart()
        return true
    }

    // Audio surfaces: the AppState edit persists through the settings mirror;
    // the facade call applies it to the live worker immediately.
    function selectAudioDevices(inputId, outputId) {
        if (!live)
            return
        AppState.inputDevice = String(inputId)
        AppState.outputDevice = String(outputId)
        facade.selectAudioDevices(String(inputId), String(outputId))
    }

    function setAudioVolumes(inputVolume, outputVolume) {
        if (!live)
            return
        AppState.microphoneVolume = Math.min(1, Math.max(0, Number(inputVolume)))
        AppState.outputVolume = Math.min(1, Math.max(0, Number(outputVolume)))
        facade.setAudioVolumes(AppState.microphoneVolume, AppState.outputVolume)
    }

    // An inbound ring is a decision, never an event: both verdicts are
    // explicit user actions routed to the core.
    function acceptIncomingCall() {
        if (!live)
            return false
        facade.acceptIncomingCall()
        return true
    }

    function declineIncomingCall() {
        if (!live)
            return false
        facade.declineIncomingCall()
        return true
    }

    // Screen share is a child of the connected call; the core enforces that
    // and refuses honestly when the capture boundary is unavailable.
    function startScreenShare() {
        if (!live)
            return
        facade.startScreenShare()
    }

    function stopScreenShare() {
        if (!live)
            return
        facade.stopScreenShare()
    }

    // ---- Mapping ----------------------------------------------------------

    function _syncCall() {
        // Read the facade directly: inside change notifications the `live`
        // binding still holds its stale value, so it may claim a dead core
        // is live or miss a fresh one. Only the property itself is current.
        var current = facade
        if (current === null || !current.coreReady) {
            if (hadLive) {
                hadLive = false
                AppState.setCallState("idle")
                AppState.microphoneMuted = false
                AppState.remoteSpeaking = false
                AppState.setCallShareState("NOT_SHARING")
                AppState.setCallStats(0, 0, "unknown")
            }
            return
        }
        hadLive = true
        var state = String(current.callState)
        // An inbound ring mirrors as an explicit incoming state the views
        // must answer; it expires in the core if nobody does.
        if (state === "INCOMING")
            AppState.setCallState("incoming")
        else if (state === "CONNECTING" || state === "RECONNECTING")
            AppState.setCallState("connecting")
        else if (state === "CONNECTED")
            AppState.setCallState("connected")
        else if (state === "FAILED")
            AppState.setCallState("unavailable")
        else
            AppState.setCallState("idle")
        if (current.callMuted !== AppState.microphoneMuted)
            AppState.microphoneMuted = current.callMuted
        if (String(current.screenShareState) !== AppState.callShareState)
            AppState.setCallShareState(String(current.screenShareState))
        // The worker's hold state is authoritative; a refused hold echoes
        // back released.
        if (current.pttActive !== AppState.pushToTalkActive)
            AppState.pushToTalkActive = current.pttActive
        if (Boolean(current.remoteSpeaking) !== AppState.remoteSpeaking)
            AppState.remoteSpeaking = Boolean(current.remoteSpeaking)
        // Transport facts ride the same snapshots and the stats event; the
        // setter's equality keeps idle re-mirrors from churning bindings.
        AppState.setCallStats(current.callRttMs, current.callLossPct,
                              current.callQuality)
    }

    // Device/volume echoes from the worker converge into the session state
    // the settings mirror owns; setter equality keeps the loop closed.
    function _syncAudio() {
        var current = facade
        if (current === null || !current.coreReady)
            return
        if (Math.abs(Number(current.inputVolume) - AppState.microphoneVolume) > 0.0001)
            AppState.microphoneVolume = current.inputVolume
        if (Math.abs(Number(current.outputVolume) - AppState.outputVolume) > 0.0001)
            AppState.outputVolume = current.outputVolume
        if (String(current.audioInputDevice).length > 0
            && current.audioInputDevice !== AppState.inputDevice)
            AppState.inputDevice = String(current.audioInputDevice)
        if (String(current.audioOutputDevice).length > 0
            && current.audioOutputDevice !== AppState.outputDevice)
            AppState.outputDevice = String(current.audioOutputDevice)
    }

    // A bridge can appear long after the core published its first state.
    Component.onCompleted: _syncCall()
    onFacadeChanged: _syncCall()

    readonly property list<QtObject> wiring: [
        Connections {
            target: provider.facade

            function onCallChanged() {
                provider._syncCall()
            }

            function onCallStatsChanged() {
                provider._syncCall()
            }

            function onAudioChanged() {
                provider._syncAudio()
            }

            function onScreenShareChanged() {
                provider._syncCall()
            }

            function onCoreReadyChanged() {
                provider._syncCall()
            }

            // Every call.* refusal carries a localized ui_key from the core;
            // unknown keys still surface instead of failing silently. Other
            // families belong to their own bridges.
            function onRequestFailed(requestType, uiKey) {
                if (String(requestType).indexOf("call.") !== 0)
                    return
                AppState.requestLocalizedToast("call",
                                               String(uiKey || "error.call.unavailable"),
                                               {}, "", {})
            }
        }
    ]
}
