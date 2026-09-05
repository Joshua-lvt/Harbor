import QtQuick
import QtTest
import Harbor 2.0

TestCase {
    name: "MockController"

    function init() {
        MockController.resetSession()
        MockController.clearToastQueue()
    }

    function cleanup() {
        MockController.resetSession()
    }

    function test_muteReleasesPushToTalk() {
        AppState.callState = "connected"
        verify(MockController.setPushToTalk(true))
        verify(AppState.pushToTalkActive)
        MockController.setMuted(true)
        verify(AppState.microphoneMuted)
        verify(!AppState.pushToTalkActive)
    }

    function test_disconnectReleasesCallAndPushToTalk() {
        AppState.callState = "connected"
        MockController.setPushToTalk(true)
        MockController.disconnect()
        compare(AppState.callState, "idle")
        verify(!AppState.pushToTalkActive)
    }

    function test_pairingCodeSequenceIsDeterministic() {
        compare(MockController.pairingCode, "HBR-7D92")
        compare(MockController.refreshPairingCode(), "HBR-4A10")
        MockController.resetSession()
        compare(MockController.pairingCode, "HBR-7D92")
        compare(MockController.refreshPairingCode(), "HBR-4A10")
    }

    function test_pulsePushToTalkReleasesItself() {
        AppState.callState = "connected"
        verify(MockController.pulsePushToTalk())
        verify(AppState.pushToTalkActive)
        wait(MockController.pushToTalkPulseDuration + 120)
        verify(!AppState.pushToTalkActive)
    }

    function test_pushToTalkPulseRejectedWhenNotInCall() {
        AppState.callState = "idle"
        verify(!MockController.pulsePushToTalk())
        verify(!AppState.pushToTalkActive)
    }

    function test_incomingPairingFlowUsesPairingMode() {
        var declined = []
        MockController.incomingPairingDeclined.connect(function(partnerName) {
            declined.push(partnerName)
        })

        verify(MockController.scheduleIncomingRequest(50))
        wait(80)
        verify(AppState.pairingVisible)
        compare(MockController.pairingMode, "incoming")
        compare(MockController.incomingRequest.name, "Morgan")

        verify(MockController.declineIncomingRequest())
        compare(MockController.pairingMode, "choice")
        verify(!AppState.pairingVisible)
        compare(declined, ["Morgan"])

        // The same sequence index advances, so the next request is deterministic.
        MockController.showIncomingRequest()
        compare(MockController.incomingRequest.name, "Avery")
        verify(MockController.acceptIncomingRequest())
        compare(MockController.pairingMode, "success")
        verify(AppState.pairingVisible)
        compare(AppState.partnerName, "Avery")
    }

    function test_openPairingCancelsScheduledIncoming() {
        verify(MockController.scheduleIncomingRequest(50))
        MockController.openPairing()
        verify(!MockController.incomingRequestScheduled)
        compare(MockController.pairingMode, "choice")
        wait(120)
        compare(MockController.pairingMode, "choice")
    }

    function test_toastQueueIsCanonicalAndFifo() {
        verify(!MockController.toastActive)
        MockController.queueLocalizedToast("online", "toast.connected.title", {},
                                           "toast.connected.description", { name: "Taylor" })
        MockController.queueLocalizedToast("network", "toast.reconnecting.title", {},
                                           "toast.reconnecting.description", {})
        verify(MockController.toastActive)
        compare(MockController.activeToast.titleKey, "toast.connected.title")
        compare(MockController.pendingToastCount, 1)

        MockController.dismissActiveToast()
        compare(MockController.activeToast.titleKey, "toast.reconnecting.title")
        compare(MockController.pendingToastCount, 0)

        MockController.clearToastQueue()
        verify(!MockController.toastActive)
    }

    function test_settingsFeedbackContract() {
        verify(MockController.settingsApplied)
        MockController.markSettingChanged("settings.saving.theme")
        verify(!MockController.settingsApplied)
        compare(MockController.settingsFeedbackKey, "settings.saving.theme")
        MockController.resetSession()
        verify(MockController.settingsApplied)
        compare(MockController.settingsFeedbackKey, "settings.savedAutomatically")
    }

    function test_notificationMutationsUseAppState() {
        var initialCount = AppState.notifications.length
        var id = MockController._addNextNotification(false)
        compare(AppState.notifications.length, initialCount + 1)
        verify(AppState.unreadCount >= 1)
        verify(MockController.dismissNotification(id))
        compare(AppState.notifications.length, initialCount)
    }

    function test_resetCancelsOperations() {
        MockController.runDiagnostics()
        MockController.scanDevices("found")
        MockController.startAudioTest()
        MockController.resetSession()
        verify(!MockController.diagnosticsRunning)
        verify(!MockController.deviceScanRunning)
        verify(!MockController.audioTestRunning)
        compare(MockController.pairingCode, "HBR-7D92")
    }

    function test_audioFixturesUseIdsAndTranslationKeys() {
        var input = MockController.audioInputOptions
        var output = MockController.audioOutputOptions
        compare(input[0].id, "default-microphone")
        compare(output[0].id, "harbor-headphones")
        compare(input[0].labelKey, "fixture.audio.input.defaultMicrophone")
        compare(output[0].labelKey, "fixture.audio.output.harborHeadphones")
        compare(MockController.audioOptionIndex(input, "webcam-microphone"), 2)
        compare(MockController.audioOptionIndex(output, "missing"), -1)

        AppState.locale = "pt-BR"
        tryCompare(I18n, "locale", "pt-BR")
        compare(MockController.audioLabel(input, AppState.inputDevice), "Microfone padrão")
        compare(MockController.audioLabel(output, AppState.outputDevice), "Fones de ouvido Harbor")
    }

    function test_discoveryFixturesUseNameKeys() {
        compare(MockController.deviceCandidateSequence[0].nameKey,
                "fixture.device.studioTablet")
        compare(MockController.deviceCandidateSequence[1].nameKey,
                "fixture.device.travelLaptop")
    }
}
