import QtQuick
import QtTest
import Harbor 2.0

// Contract tests for the call bridge. The facade is stubbed with the exact
// property surface the real C++ facade exposes, so these tests pin the
// core-state→AppState mapping, the mute mirror, core-fault teardown, honest
// refusal of push to talk, and the localized surfacing of call.* errors,
// independently of the supervised Rust core.
TestCase {
    id: root

    name: "HarborCallBridge"

    QtObject {
        id: stubFacade

        property bool coreReady: true
        property string callState: "IDLE"
        property string callId: ""
        property bool callMuted: false
        property string screenShareState: "NOT_SHARING"
        property var audioInputs: []
        property var audioOutputs: []
        property string audioInputDevice: ""
        property string audioOutputDevice: ""
        property real inputVolume: 0.72
        property real outputVolume: 0.64
        property real voiceLevel: 0.0
        property real remoteVoiceLevel: 0.0
        property bool speaking: false
        property bool remoteSpeaking: false
        property bool pttActive: false
        property real callRttMs: 0.0
        property real callLossPct: 0.0
        property string callQuality: "unknown"
        property var calls: []

        signal callChanged
        signal screenShareChanged
        signal audioChanged
        signal callStatsChanged
        // coreReadyChanged comes from the coreReady property itself.
        signal requestFailed(string requestType, string uiKey)

        function startCall() {
            var next = stubFacade.calls.slice()
            next.push("startCall")
            stubFacade.calls = next
            stubFacade.callState = "CONNECTING"
            stubFacade.callChanged()
        }

        function endCall() {
            var next = stubFacade.calls.slice()
            next.push("endCall")
            stubFacade.calls = next
            stubFacade.callState = "IDLE"
            stubFacade.callChanged()
        }

        function setCallMuted(muted) {
            var next = stubFacade.calls.slice()
            next.push("setCallMuted:" + muted)
            stubFacade.calls = next
            stubFacade.callMuted = muted
            stubFacade.callChanged()
        }

        function setPushToTalkActive(active) {
            var next = stubFacade.calls.slice()
            next.push("setPushToTalkActive:" + active)
            stubFacade.calls = next
            stubFacade.pttActive = active
        }

        function setAudioVolumes(inputVolume, outputVolume) {
            var next = stubFacade.calls.slice()
            next.push("setAudioVolumes:" + inputVolume + ":" + outputVolume)
            stubFacade.calls = next
            stubFacade.inputVolume = inputVolume
            stubFacade.outputVolume = outputVolume
            stubFacade.audioChanged()
        }

        function selectAudioDevices(inputId, outputId) {
            var next = stubFacade.calls.slice()
            next.push("selectAudioDevices:" + inputId + ":" + outputId)
            stubFacade.calls = next
            stubFacade.audioInputDevice = inputId
            stubFacade.audioOutputDevice = outputId
            stubFacade.audioChanged()
        }

        function acceptIncomingCall() {
            var next = stubFacade.calls.slice()
            next.push("acceptIncomingCall")
            stubFacade.calls = next
            stubFacade.callState = "CONNECTING"
            stubFacade.callChanged()
        }

        function declineIncomingCall() {
            var next = stubFacade.calls.slice()
            next.push("declineIncomingCall")
            stubFacade.calls = next
            stubFacade.callState = "IDLE"
            stubFacade.callChanged()
        }

        function startScreenShare() {
            var next = stubFacade.calls.slice()
            next.push("startScreenShare")
            stubFacade.calls = next
            stubFacade.screenShareState = "SHARING"
            stubFacade.screenShareChanged()
        }

        function stopScreenShare() {
            var next = stubFacade.calls.slice()
            next.push("stopScreenShare")
            stubFacade.calls = next
            stubFacade.screenShareState = "NOT_SHARING"
            stubFacade.screenShareChanged()
        }

        function _publish(state) {
            stubFacade.callState = state
            stubFacade.callChanged()
        }

        function _publishStats(rttMs, lossPct, quality) {
            stubFacade.callRttMs = rttMs
            stubFacade.callLossPct = lossPct
            stubFacade.callQuality = quality
            stubFacade.callStatsChanged()
        }
    }

    HarborCallBridge {
        id: bridge
    }

    property var lastToast: null

    Connections {
        target: AppState

        function onLocalizedToastRequested(category, titleKey, titleParams,
                                           descriptionKey, descriptionParams) {
            root.lastToast = { category: category, titleKey: titleKey }
        }
    }

    property string pristineCallState
    property bool pristineMuted

    function init() {
        pristineCallState = AppState.callState
        pristineMuted = AppState.microphoneMuted
        bridge.facade = null
        stubFacade.calls = []
        stubFacade.coreReady = true
        stubFacade.callState = "IDLE"
        stubFacade.callId = ""
        stubFacade.callMuted = false
        stubFacade.screenShareState = "NOT_SHARING"
        stubFacade.audioInputs = []
        stubFacade.audioOutputs = []
        stubFacade.audioInputDevice = ""
        stubFacade.audioOutputDevice = ""
        stubFacade.inputVolume = 0.72
        stubFacade.outputVolume = 0.64
        stubFacade.voiceLevel = 0
        stubFacade.remoteVoiceLevel = 0
        stubFacade.speaking = false
        stubFacade.remoteSpeaking = false
        stubFacade.pttActive = false
        stubFacade.callRttMs = 0.0
        stubFacade.callLossPct = 0.0
        stubFacade.callQuality = "unknown"
        lastToast = null
        AppState.callState = "idle"
        AppState.microphoneMuted = false
        AppState.setCallShareState("NOT_SHARING")
    }

    function cleanup() {
        bridge.facade = null
        AppState.callState = pristineCallState
        AppState.microphoneMuted = pristineMuted
    }

    function test_inertWithoutFacade() {
        verify(bridge.facade === null)
        verify(!bridge.live)
        // No facade: actions are no-ops and never invent state.
        bridge.startCall()
        bridge.endCall()
        compare(bridge.toggleMute(), false)
        bridge.setMuted(true)
        bridge.startScreenShare()
        bridge.stopScreenShare()
        compare(stubFacade.calls, [])
        compare(AppState.callState, "idle")
        compare(AppState.microphoneMuted, false)
        compare(AppState.callShareState, "NOT_SHARING")
    }

    function test_coreStatesMapOntoAppState() {
        bridge.facade = stubFacade
        verify(bridge.live)

        stubFacade._publish("CONNECTING")
        compare(AppState.callState, "connecting")
        stubFacade._publish("CONNECTED")
        compare(AppState.callState, "connected")
        // A lost direct path is recovery, not a dropped call: the channel
        // stays in the connecting family while the agent re-probes.
        stubFacade._publish("RECONNECTING")
        compare(AppState.callState, "connecting")
        stubFacade._publish("CONNECTED")
        compare(AppState.callState, "connected")
        stubFacade._publish("FAILED")
        compare(AppState.callState, "unavailable")
        stubFacade._publish("IDLE")
        compare(AppState.callState, "idle")
        // Unknown boundary material degrades to idle, never to a guess.
        stubFacade._publish("SOMETHING_ELSE")
        compare(AppState.callState, "idle")
    }

    function test_bridgeAdoptsStateOnCreation() {
        // The view can be opened long after the core published its state.
        stubFacade.callState = "CONNECTING"
        bridge.facade = stubFacade
        tryCompare(AppState, "callState", "connecting")
    }

    function test_muteMirrorsFromTheCore() {
        bridge.facade = stubFacade

        stubFacade.setCallMuted(true)
        compare(AppState.microphoneMuted, true)
        stubFacade.setCallMuted(false)
        compare(AppState.microphoneMuted, false)
    }

    function test_actionsForwardThroughTheFacade() {
        bridge.facade = stubFacade

        bridge.startCall()
        tryCompare(AppState, "callState", "connecting")
        compare(bridge.callBusy, true)

        bridge.setMuted(true)
        compare(stubFacade.calls, ["startCall", "setCallMuted:true"])
        compare(AppState.microphoneMuted, true)

        // Toggling flips the facade's truth, not AppState's local copy.
        bridge.toggleMute()
        compare(stubFacade.calls[stubFacade.calls.length - 1], "setCallMuted:false")
        compare(AppState.microphoneMuted, false)

        bridge.endCall()
        tryCompare(AppState, "callState", "idle")
        compare(bridge.callBusy, false)
    }

    function test_coreFaultDropsMirroredCall() {
        bridge.facade = stubFacade
        stubFacade._publish("CONNECTING")
        stubFacade.setCallMuted(true)
        compare(AppState.callState, "connecting")
        compare(AppState.microphoneMuted, true)

        // The share exists only inside a connected call.
        stubFacade.startScreenShare()
        compare(AppState.callShareState, "NOT_SHARING")
        stubFacade._publish("CONNECTED")
        stubFacade.startScreenShare()
        compare(AppState.callShareState, "SHARING")

        // A dead core takes its call — and its share — with it; no stale
        // connecting or sharing state may survive the outage.
        stubFacade.coreReady = false
        tryCompare(AppState, "callState", "idle")
        compare(AppState.microphoneMuted, false)
        tryCompare(AppState, "callShareState", "NOT_SHARING")
        stubFacade.coreReady = true
    }

    function test_shareMirrorsFromTheCoreAndDiesWithTheCall() {
        bridge.facade = stubFacade
        stubFacade._publish("CONNECTED")

        bridge.startScreenShare()
        compare(stubFacade.calls, ["startScreenShare"])
        tryCompare(AppState, "callShareState", "SHARING")

        bridge.stopScreenShare()
        tryCompare(AppState, "callShareState", "NOT_SHARING")

        // Sharing again, then the call ending tears the share down with it —
        // even if the share mirror still claims SHARING.
        bridge.startScreenShare()
        tryCompare(AppState, "callShareState", "SHARING")
        stubFacade._publish("IDLE")
        compare(AppState.callShareState, "NOT_SHARING")
    }

    function test_callRefusalsSurfaceAsLocalizedToasts() {
        bridge.facade = stubFacade

        stubFacade.requestFailed("call.share_screen_start", "error.call.screenShareUnavailable")
        compare(lastToast.category, "call")
        compare(lastToast.titleKey, "error.call.screenShareUnavailable")

        stubFacade.requestFailed("call.start", "error.call.unavailable")
        compare(lastToast.titleKey, "error.call.unavailable")

        // A missing key still surfaces instead of failing silently.
        stubFacade.requestFailed("call.end", "")
        compare(lastToast.titleKey, "error.call.unavailable")

        // Other families belong to their own bridges.
        stubFacade.requestFailed("pairing.submit", "error.server.unavailable")
        compare(lastToast.titleKey, "error.call.unavailable")
    }

    function test_pushToTalkForwardsOnlyOnAnUnmutedActiveCall() {
        bridge.facade = stubFacade
        stubFacade._publish("CONNECTED")
        // In production the contacts bridge mirrors the live core into the
        // shell-level connection; this stub world establishes it directly.
        AppState.setConnection("connected")

        verify(bridge.setPushToTalk(true))
        compare(stubFacade.calls[stubFacade.calls.length - 1], "setPushToTalkActive:true")
        compare(AppState.pushToTalkActive, true)

        // The bridge refuses locally before forwarding when mute wins.
        stubFacade.setCallMuted(true)
        compare(bridge.setPushToTalk(true), false)
        compare(AppState.pushToTalkActive, false)

        stubFacade.setCallMuted(false)
        verify(bridge.pulsePushToTalk(100))
        tryVerify(function() { return !AppState.pushToTalkActive })
        compare(stubFacade.calls[stubFacade.calls.length - 1], "setPushToTalkActive:false")
    }

    function test_audioProviderMirrorsSanitizedWorkerFacts() {
        bridge.facade = stubFacade
        stubFacade.audioInputs = [{ id: "input-1", name: "Studio microphone", isDefault: true }]
        stubFacade.audioOutputs = [{ id: "output-1", name: "Desktop speakers", isDefault: false }]
        stubFacade.audioInputDevice = "input-1"
        stubFacade.audioOutputDevice = "output-1"
        stubFacade.inputVolume = 0.5
        stubFacade.outputVolume = 0.8
        stubFacade.voiceLevel = 0.4
        stubFacade.speaking = true
        stubFacade.remoteSpeaking = true
        stubFacade.audioChanged()

        compare(bridge.microphoneLevel, 0.4)
        compare(bridge.speaking, true)
        compare(bridge.remoteSpeaking, true)
        compare(bridge.audioInputOptions.length, 1)
        compare(bridge.audioOutputOptions.length, 1)
        compare(bridge.audioLabel(bridge.audioInputOptions, "input-1"),
                I18n.t("call.audio.defaultDevice") + " · Studio microphone")
        compare(AppState.microphoneVolume, 0.5)
        compare(AppState.outputVolume, 0.8)
        compare(AppState.inputDevice, "input-1")
        compare(AppState.outputDevice, "output-1")

        bridge.setAudioVolumes(0.25, 0.75)
        bridge.selectAudioDevices("input-1", "output-1")
        verify(stubFacade.calls.indexOf("setAudioVolumes:0.25:0.75") >= 0)
        verify(stubFacade.calls.indexOf("selectAudioDevices:input-1:output-1") >= 0)
    }

    function test_incomingCallRequiresExplicitVerdict() {
        bridge.facade = stubFacade
        stubFacade._publish("INCOMING")
        compare(AppState.callState, "incoming")

        verify(bridge.acceptIncomingCall())
        compare(stubFacade.calls[stubFacade.calls.length - 1], "acceptIncomingCall")
        compare(AppState.callState, "connecting")

        stubFacade._publish("INCOMING")
        verify(bridge.declineIncomingCall())
        compare(stubFacade.calls[stubFacade.calls.length - 1], "declineIncomingCall")
        compare(AppState.callState, "idle")
    }

    function test_transportStatsMirrorIntoAppState() {
        bridge.facade = stubFacade
        stubFacade._publish("CONNECTED")
        compare(AppState.callQuality, "unknown")

        stubFacade._publishStats(42.0, 1.0, "good")
        compare(AppState.callLatency, 42.0)
        compare(AppState.callLossPct, 1.0)
        compare(AppState.callQuality, "good")

        // A degraded sample replaces the old one — the surface shows the
        // current transport, never a best-of history.
        stubFacade._publishStats(600.0, 20.0, "poor")
        compare(AppState.callQuality, "poor")
        compare(AppState.callLatency, 600.0)
    }

    function test_coreFaultDropsTransportStats() {
        bridge.facade = stubFacade
        stubFacade._publish("CONNECTED")
        stubFacade._publishStats(42.0, 1.0, "good")
        verify(AppState.callQuality === "good")

        // A dead core takes its measurements with it: no stale verdict may
        // survive the outage.
        stubFacade.coreReady = false
        stubFacade.callState = "IDLE"
        stubFacade.callChanged()
        tryCompare(AppState, "callQuality", "unknown")
        compare(AppState.callLatency, 0)
        compare(AppState.callLossPct, 0)
        compare(AppState.callState, "idle")

        // Recovery re-mirrors as fresh facts arrive.
        stubFacade.coreReady = true
        stubFacade._publish("CONNECTED")
        stubFacade._publishStats(30.0, 0.5, "good")
        compare(AppState.callLatency, 30.0)
    }
}
