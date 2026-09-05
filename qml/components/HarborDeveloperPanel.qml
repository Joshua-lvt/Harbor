pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Scenario controller for the whole prototype. Every control here routes
// through MockController/AppState, so all previews stay deterministic and
// in-memory — nothing touches networking, audio, files, or the OS.
Popup {
    id: panel

    signal onboardingRequested()
    signal pairingRequested()
    signal trayPreviewRequested()

    parent: Overlay.overlay
    anchors.centerIn: parent
    modal: true
    focus: true
    padding: Theme.sp5
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
    width: Math.min(860, parent ? parent.width - Theme.sp5 * 2 : 860)
    height: Math.min(parent ? parent.height - Theme.sp5 * 2 : 640, 680)

    // Popup roots are not Items; the Accessible interface goes on contentItem.
    // The scrim stays transparent because the shell renders one shared scrim.
    enter: Transition {
        NumberAnimation { property: "opacity"; from: 0; to: 1; duration: Theme.duration(Theme.motionNormal) }
        NumberAnimation { property: "scale"; from: 0.97; to: 1; duration: Theme.duration(Theme.motionNormal); easing.type: Theme.animEasing }
    }
    exit: Transition {
        NumberAnimation { property: "opacity"; to: 0; duration: Theme.duration(Theme.motionFast) }
    }

    Overlay.modal: Rectangle { color: "transparent" }

    background: Rectangle {
        radius: Theme.radiusLarge
        color: Theme.surfaceOverlay
        border.width: 1
        border.color: Theme.borderStrong
    }

    contentItem: ColumnLayout {
        id: panelLayout

        spacing: Theme.sp4

        Accessible.role: Accessible.Dialog
        Accessible.name: I18n.t("developer.title")
        Accessible.description: I18n.t("developer.description")

        // Header
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.sp3

            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.sp1

                Text {
                    text: I18n.t("developer.title")
                    color: Theme.textPrimary
                    font.family: Theme.fontFamilyDisplay
                    font.pixelSize: Theme.fontTitle
                    font.weight: Font.Bold
                    Layout.fillWidth: true
                }

                Text {
                    text: I18n.t("developer.description")
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontSmall
                    lineHeight: Theme.lineHeightSmall
                    lineHeightMode: Text.ProportionalHeight
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }
            }

            HarborBadge {
                text: I18n.t("developer.mockBadge")
                tone: "accent"
            }

            HarborIconButton {
                iconName: "close"
                accessibleName: I18n.t("a11y.closeDialog")
                toolTip: I18n.t("common.actions.close")
                onClicked: panel.close()
            }
        }

        // Scenes
        ScrollView {
            id: scenesScroll

            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            ScrollBar.horizontal.policy: ScrollBar.AlwaysOff

            GridLayout {
                id: scenesGrid

                width: Math.max(0, scenesScroll.availableWidth)
                columns: width >= Theme.breakpointCompact ? 2 : 1
                columnSpacing: Theme.sp4
                rowSpacing: Theme.sp4

                // ── Connection ────────────────────────────────────────────
                HarborSectionCard {
                    Layout.fillWidth: true
                    title: I18n.t("developer.scene.connection")
                    iconName: "online"

                    Flow {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        Repeater {
                            model: [
                                { value: "connected", label: I18n.t("common.status.connected") },
                                { value: "connecting", label: I18n.t("common.status.connecting") },
                                { value: "reconnecting", label: I18n.t("common.status.reconnecting") },
                                { value: "disconnected", label: I18n.t("common.status.disconnected") }
                            ]

                            HarborChoiceChip {
                                checkable: false
                                required property var modelData
                                text: modelData.label
                                checked: AppState.connectionState === modelData.value
                                onClicked: MockController.setConnectionScenario(modelData.value, true)
                            }
                        }
                    }
                }

                // ── Presence ─────────────────────────────────────────────
                HarborSectionCard {
                    Layout.fillWidth: true
                    title: I18n.t("developer.scene.presence")
                    iconName: "user"

                    Flow {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        Repeater {
                            model: [
                                { value: "online", label: I18n.t("common.status.online") },
                                { value: "idle", label: I18n.t("common.status.idle") },
                                { value: "offline", label: I18n.t("common.status.offline") }
                            ]

                            HarborChoiceChip {
                                checkable: false
                                required property var modelData
                                text: modelData.label
                                checked: AppState.partnerState === modelData.value
                                onClicked: AppState.setPartnerState(modelData.value)
                            }
                        }
                    }
                }

                // ── Call ─────────────────────────────────────────────────
                HarborSectionCard {
                    Layout.fillWidth: true
                    title: I18n.t("developer.scene.call")
                    iconName: "mic"

                    Flow {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        Repeater {
                            model: [
                                { value: "idle", label: I18n.t("call.status.ready") },
                                { value: "connected", label: I18n.t("call.status.connected") },
                                { value: "unavailable", label: I18n.t("developer.call.unavailable") }
                            ]

                            HarborChoiceChip {
                                checkable: false
                                required property var modelData
                                text: modelData.label
                                checked: AppState.callState === modelData.value
                                onClicked: {
                                    if (modelData.value === "idle")
                                        MockController.endCall()
                                    else if (modelData.value === "connected")
                                        MockController.completeCall()
                                    else
                                        MockController.forceCallState("unavailable")
                                }
                            }
                        }
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp3

                        HarborButton {
                            text: I18n.t("call.action.start")
                            variant: "secondary"
                            onClicked: MockController.startCall()
                        }

                        HarborButton {
                            text: I18n.t("call.action.end")
                            variant: "secondary"
                            onClicked: MockController.endCall()
                        }
                    }
                }

                // ── Mute / PTT ───────────────────────────────────────────
                HarborSectionCard {
                    Layout.fillWidth: true
                    title: I18n.t("developer.scene.mute")
                    iconName: "mic-off"

                    HarborToggle {
                        Layout.fillWidth: true
                        text: I18n.t("call.action.mute")
                        description: I18n.t("call.audio.microphone")
                        checked: AppState.microphoneMuted
                        onToggled: MockController.setMuted(checked)
                    }

                    HarborToggle {
                        Layout.fillWidth: true
                        text: I18n.t("call.ptt.title")
                        description: I18n.t("call.ptt.active")
                        checked: AppState.pushToTalkActive
                        enabled: AppState.callState === "connected" && !AppState.microphoneMuted
                        onToggled: MockController.setPushToTalk(checked)
                    }
                }

                // ── Pairing ──────────────────────────────────────────────
                HarborSectionCard {
                    Layout.fillWidth: true
                    title: I18n.t("developer.scene.pairing")
                    iconName: "lock"

                    Flow {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        HarborButton {
                            text: I18n.t("developer.pairing.openChoice")
                            variant: "secondary"
                            onClicked: {
                                panel.close()
                                panel.pairingRequested()
                            }
                        }

                        HarborButton {
                            text: I18n.t("developer.pairing.openQr")
                            variant: "secondary"
                            onClicked: {
                                panel.close()
                                panel.pairingRequested()
                                MockController.openPairing("qr")
                            }
                        }

                        HarborButton {
                            text: I18n.t("developer.pairing.openRequest")
                            variant: "secondary"
                            onClicked: {
                                panel.close()
                                panel.pairingRequested()
                                MockController.openPairing("request")
                            }
                        }

                        HarborButton {
                            text: I18n.t("developer.pairing.incoming")
                            variant: "secondary"
                            onClicked: {
                                panel.close()
                                panel.pairingRequested()
                                MockController.scheduleIncomingRequest(900)
                            }
                        }

                        HarborButton {
                            text: I18n.t("developer.pairing.success")
                            variant: "secondary"
                            onClicked: {
                                panel.close()
                                panel.pairingRequested()
                                MockController.openPairing()
                                MockController.completePairing("")
                            }
                        }

                        HarborButton {
                            text: I18n.t("developer.pairing.error")
                            variant: "secondary"
                            onClicked: {
                                panel.close()
                                panel.pairingRequested()
                                MockController.openPairing("request")
                                MockController.submitPairingCode("ERR")
                            }
                        }
                    }
                }

                // ── Notifications ────────────────────────────────────────
                HarborSectionCard {
                    Layout.fillWidth: true
                    title: I18n.t("developer.scene.notifications")
                    iconName: "info"

                    Flow {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        HarborButton {
                            text: I18n.t("developer.notifications.preview")
                            variant: "secondary"
                            onClicked: MockController.previewNotification()
                        }

                        HarborButton {
                            text: I18n.t("developer.notifications.burst", { count: 4 })
                            variant: "secondary"
                            onClicked: MockController.startNotificationBurst(4)
                        }

                        HarborButton {
                            text: I18n.t("developer.notifications.markAll")
                            variant: "secondary"
                            onClicked: MockController.markAllNotificationsRead()
                        }

                        HarborButton {
                            text: I18n.t("developer.notifications.clear")
                            variant: "secondary"
                            onClicked: MockController.clearNotificationsWithConfirmation()
                        }
                    }
                }

                // ── Devices ──────────────────────────────────────────────
                HarborSectionCard {
                    Layout.fillWidth: true
                    title: I18n.t("developer.scene.devices")
                    iconName: "monitor"

                    Flow {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        HarborButton {
                            text: I18n.t("developer.devices.scanFound")
                            variant: "secondary"
                            busy: MockController.deviceScanRunning
                            onClicked: MockController.scanDevices("found")
                        }

                        HarborButton {
                            text: I18n.t("developer.devices.scanNone")
                            variant: "secondary"
                            busy: MockController.deviceScanRunning
                            onClicked: MockController.scanDevices("none")
                        }
                    }
                }

                // ── Page states ──────────────────────────────────────────
                HarborSectionCard {
                    id: pagesCard

                    Layout.fillWidth: true
                    title: I18n.t("developer.scene.pages")
                    iconName: "app"

                    property string selectedPage: "home"

                    Flow {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        Repeater {
                            model: [
                                { value: "home", label: I18n.t("sidebar.home") },
                                { value: "call", label: I18n.t("sidebar.call") },
                                { value: "activity", label: I18n.t("sidebar.activity") },
                                { value: "network", label: I18n.t("sidebar.network") },
                                { value: "devices", label: I18n.t("sidebar.devices") },
                                { value: "profile", label: I18n.t("sidebar.profile") },
                                { value: "settings", label: I18n.t("sidebar.settings") }
                            ]

                            HarborChoiceChip {
                                checkable: false
                                required property var modelData
                                text: modelData.label
                                checked: pagesCard.selectedPage === modelData.value
                                onClicked: pagesCard.selectedPage = modelData.value
                            }
                        }
                    }

                    Flow {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        Repeater {
                            model: ["content", "loading", "empty", "error"]

                            HarborChoiceChip {
                                checkable: false
                                required property string modelData
                                text: I18n.t("developer.pageState." + modelData)
                                checked: AppState.pageState(pagesCard.selectedPage) === modelData
                                onClicked: MockController.setPageState(pagesCard.selectedPage, modelData)
                            }
                        }

                        HarborButton {
                            text: I18n.t("developer.pages.transition")
                            variant: "secondary"
                            onClicked: MockController.transitionPage(pagesCard.selectedPage, "content")
                        }
                    }
                }

                // ── Session preferences ──────────────────────────────────
                HarborSectionCard {
                    Layout.fillWidth: true
                    title: I18n.t("developer.scene.session")
                    iconName: "settings"

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        Text {
                            text: I18n.t("settings.language.title")
                            color: Theme.textPrimary
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontSmall
                            font.weight: Font.DemiBold
                        }

                        Flow {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            Repeater {
                                model: [
                                    { value: "en", label: I18n.t("locale.english") },
                                    { value: "pt-BR", label: I18n.t("locale.portugueseBrazil") }
                                ]

                                HarborChoiceChip {
                                    checkable: false
                                    required property var modelData
                                    text: modelData.label
                                    checked: I18n.locale === modelData.value
                                    onClicked: AppState.locale = modelData.value
                                }
                            }
                        }
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        Text {
                            text: I18n.t("settings.appearance.colorMode.title")
                            color: Theme.textPrimary
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontSmall
                            font.weight: Font.DemiBold
                        }

                        Flow {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            Repeater {
                                model: [
                                    { value: "dark", label: I18n.t("settings.appearance.moonlight") },
                                    { value: "light", label: I18n.t("settings.appearance.daylight") }
                                ]

                                HarborChoiceChip {
                                    checkable: false
                                    required property var modelData
                                    text: modelData.label
                                    checked: Theme.mode === modelData.value
                                    onClicked: AppState.appearanceMode = modelData.value
                                }
                            }
                        }
                    }

                    HarborToggle {
                        Layout.fillWidth: true
                        text: I18n.t("settings.appearance.reduceMotion.title")
                        description: I18n.t("settings.appearance.reduceMotion.description")
                        checked: AppState.reducedMotion
                        onToggled: AppState.reducedMotion = checked
                    }

                    HarborToggle {
                        Layout.fillWidth: true
                        text: I18n.t("settings.appearance.ambient.title")
                        description: I18n.t("settings.appearance.ambient.description")
                        checked: AppState.background
                        onToggled: AppState.background = checked
                    }

                    HarborToggle {
                        Layout.fillWidth: true
                        text: I18n.t("settings.appearance.contrast.title")
                        description: I18n.t("settings.appearance.contrast.description")
                        checked: AppState.higherContrast
                        onToggled: AppState.higherContrast = checked
                    }
                }

                // ── Overlays ─────────────────────────────────────────────
                HarborSectionCard {
                    Layout.fillWidth: true
                    title: I18n.t("developer.scene.overlays")
                    iconName: "menu"

                    Flow {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        HarborButton {
                            text: I18n.t("developer.openOnboarding")
                            variant: "secondary"
                            onClicked: {
                                panel.close()
                                panel.onboardingRequested()
                            }
                        }

                        HarborButton {
                            text: I18n.t("developer.openPairing")
                            variant: "secondary"
                            onClicked: {
                                panel.close()
                                panel.pairingRequested()
                            }
                        }

                        HarborButton {
                            text: I18n.t("tray.title")
                            variant: "secondary"
                            onClicked: {
                                panel.close()
                                panel.trayPreviewRequested()
                            }
                        }
                    }
                }

                // ── Gallery ──────────────────────────────────────────────
                HarborSectionCard {
                    Layout.fillWidth: true
                    Layout.columnSpan: scenesGrid.columns
                    title: I18n.t("developer.scene.gallery")
                    iconName: "activity"

                    Flow {
                        Layout.fillWidth: true
                        spacing: Theme.sp3

                        HarborButton { text: I18n.t("common.actions.start") }
                        HarborButton { text: I18n.t("common.actions.cancel"); variant: "secondary" }
                        HarborButton { text: I18n.t("common.actions.remove"); variant: "danger" }
                        HarborButton { text: I18n.t("common.actions.close"); variant: "quiet" }

                        HarborBadge { text: I18n.t("common.status.connected"); tone: "success" }
                        HarborBadge { text: I18n.t("common.status.connecting"); tone: "warning" }
                        HarborBadge { text: I18n.t("common.status.offline"); tone: "neutral" }

                        HarborChoiceChip { text: I18n.t("common.actions.select"); checked: true }
                        HarborChoiceChip { text: I18n.t("common.actions.skip") }

                        HarborAvatar { avatarSize: 36; initials: AppState.selfInitials; status: "online" }
                        HarborAvatar { avatarSize: 36; initials: AppState.partnerInitials; status: "idle" }

                        HarborSpinner { spinnerSize: 22 }

                        HarborAudioVisualizer {
                            implicitWidth: 120
                            implicitHeight: 32
                            level: 0.55
                        }

                        HarborGraph {
                            implicitWidth: 200
                            implicitHeight: 72
                            series: [
                                { label: I18n.t("network.traffic.down"), values: [4, 6, 5, 8, 7, 9, 8], color: Theme.chartSeries1, dashed: false },
                                { label: I18n.t("network.traffic.up"), values: [2, 3, 3, 5, 4, 5, 5], color: Theme.chartSeries2, dashed: true }
                            ]
                        }

                        HarborMockQr {
                            implicitWidth: 84
                            implicitHeight: 84
                            seed: "HARBOR-GALLERY"
                        }
                    }
                }

                // ── Reset ────────────────────────────────────────────────
                HarborSectionCard {
                    Layout.fillWidth: true
                    Layout.columnSpan: scenesGrid.columns
                    title: I18n.t("common.actions.reset")
                    iconName: "refresh"

                    HarborButton {
                        text: I18n.t("developer.reset")
                        variant: "danger"
                        onClicked: MockController.resetSession()
                    }
                }
            }
        }
    }
}
