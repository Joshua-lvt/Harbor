pragma ComponentBehavior: Bound

import Harbor 2.0
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Deterministic device preview: fixtures and every scan/connect transition
// live in AppState.devices and MockController. The view only renders state
// and forwards intents — no local model, no local timer, no hardware or
// network access.
HarborStateLayer {
    id: root

    readonly property bool compact: width < Theme.breakpointCompact

    pageState: AppState.pageState("devices")
    title: pageState === "empty" ? I18n.t("devices.empty.title")
         : pageState === "error" ? I18n.t("state.error.title") : ""
    description: pageState === "empty" ? I18n.t("devices.empty.description")
              : pageState === "error" ? I18n.t("state.error.description") : ""
    actionText: pageState === "error" ? I18n.t("common.actions.retry") : ""
    onActionTriggered: MockController.transitionPage("devices", "content")

    HarborPage {
        id: page

        width: parent.width
        height: parent.height
        accessibleName: I18n.t("devices.title")

        HarborPageHeader {
            title: I18n.t("devices.title")
            subtitle: I18n.t("devices.subtitle")
            iconName: "laptop"

            HarborButton {
                text: I18n.t("devices.pair")
                iconName: "plus"
                onClicked: AppState.openPairing()
            }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.sp3

            Text {
                Layout.fillWidth: true
                text: I18n.t("devices.paired.title")
                color: Theme.textPrimary
                font.family: Theme.fontFamily
                font.pixelSize: Theme.fontHeading
                font.weight: Font.DemiBold
            }

            Text {
                text: I18n.t("devices.paired.count", { count: AppState.devices.length })
                color: Theme.textMuted
                font.family: Theme.fontFamily
                font.pixelSize: Theme.fontSmall
            }
        }

        GridLayout {
            Layout.fillWidth: true
            columns: root.compact ? 1 : 3
            columnSpacing: Theme.sp4
            rowSpacing: Theme.sp4
            visible: AppState.devices.length > 0

            Repeater {
                model: AppState.devices

                delegate: HarborDeviceCard {
                    id: deviceCard

                    required property var modelData

                    Layout.fillWidth: true
                    // Live entries carry their real name (host name, durable
                    // Harbor id); fixture entries resolve through their keys.
                    deviceName: deviceCard.modelData.name
                                ? deviceCard.modelData.name
                                : I18n.t(deviceCard.modelData.nameKey,
                                         deviceCard.modelData.id === "partner-laptop"
                                         ? { name: AppState.partnerName }
                                         : deviceCard.modelData.nameParams || {})
                    deviceType: I18n.t(deviceCard.modelData.typeKey)
                    status: deviceCard.modelData.connected ? "online" : "offline"
                    statusDetail: I18n.t(deviceCard.modelData.statusKey,
                                         deviceCard.modelData.statusParams)
                    iconName: deviceCard.modelData.iconName
                    // Real presence has no connect/disconnect switch: those
                    // actions exist only in the deterministic fixture world.
                    showAction: !deviceCard.modelData.primary
                                && deviceCard.modelData.manageable !== false

                    onConnectRequested: MockController.setDeviceConnected(deviceCard.modelData.id, true)
                    onDisconnectRequested: MockController.setDeviceConnected(deviceCard.modelData.id, false)
                    onManageRequested: MockController.queueLocalizedToast(
                        "system", "devices.manage.title", {},
                        "devices.manage.description", {})
                }
            }
        }

        HarborEmptyState {
            Layout.fillWidth: true
            visible: AppState.devices.length === 0
            iconName: "monitor"
            title: I18n.t("devices.empty.title")
            description: I18n.t("devices.empty.description")
            actionText: I18n.t("devices.pair")
            onActionTriggered: AppState.openPairing()
        }

        HarborSectionCard {
            Layout.fillWidth: true
            title: I18n.t("devices.audio.title")
            iconName: "volume"

            headerActions: HarborBadge {
                tone: AppState.microphoneMuted ? "warning" : "success"
                showDot: true
                compact: true
                text: AppState.microphoneMuted
                      ? I18n.t("common.status.offline")
                      : I18n.t("devices.audio.ready")
            }

            HarborSettingRow {
                Layout.fillWidth: true
                iconName: "mic"
                label: I18n.t("settings.audio.input")
                description: MockController.audioLabel(MockController.audioInputOptions,
                                                       AppState.inputDevice)

                HarborBadge {
                    compact: true
                    showDot: true
                    tone: AppState.microphoneMuted ? "warning" : "success"
                    text: AppState.microphoneMuted
                          ? I18n.t("call.action.unmute")
                          : I18n.t("common.status.connected")
                }
            }

            HarborSettingRow {
                Layout.fillWidth: true
                showDivider: false
                iconName: "volume"
                label: I18n.t("settings.audio.output")
                description: MockController.audioLabel(MockController.audioOutputOptions,
                                                       AppState.outputDevice)

                Text {
                    text: I18n.percent(AppState.outputVolume)
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontSmall
                }
            }
        }

        GridLayout {
            Layout.fillWidth: true
            columns: root.compact ? 1 : 2
            columnSpacing: Theme.sp4
            rowSpacing: Theme.sp4

            HarborSectionCard {
                Layout.fillWidth: true
                title: I18n.t("devices.nearby.title")
                description: I18n.t("devices.nearby.description")
                iconName: "network"

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.sp3

                    Text {
                        Layout.fillWidth: true
                        visible: !MockController.deviceScanRunning
                        text: MockController.deviceScanStage === "complete"
                              && MockController.discoveredDevices.length > 0
                              ? I18n.t("devices.nearby.found.description",
                                       { name: I18n.t(MockController.discoveredDevices[0].nameKey,
                                                      MockController.discoveredDevices[0].nameParams || {}) })
                              : I18n.t("devices.nearby.description")
                        color: Theme.textSecondary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontSmall
                        wrapMode: Text.Wrap
                    }

                    Text {
                        visible: MockController.deviceScanRunning
                        text: I18n.t("devices.nearby.looking")
                        color: Theme.textMuted
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontSmall
                    }
                }

                ProgressBar {
                    Layout.fillWidth: true
                    visible: MockController.deviceScanRunning
                    from: 0
                    to: 100
                    value: MockController.deviceScanProgress
                    Accessible.name: I18n.t("devices.nearby.scanning")
                }

                HarborButton {
                    Layout.fillWidth: true
                    text: MockController.deviceScanRunning
                          ? I18n.t("devices.nearby.scanning")
                          : I18n.t("devices.nearby.scan")
                    busy: MockController.deviceScanRunning
                    onClicked: MockController.scanDevices()
                }
            }

            HarborSectionCard {
                Layout.fillWidth: true
                title: I18n.t("devices.presence.title")
                description: I18n.t("devices.presence.description")
                iconName: "user"

                HarborSettingRow {
                    Layout.fillWidth: true
                    showDivider: false
                    label: I18n.t("devices.presence.title")
                    description: I18n.t("devices.presence.description")

                    HarborToggle {
                        checked: AppState.deviceVisibility
                        onToggled: {
                            AppState.deviceVisibility = checked
                            MockController.markSettingChanged("settings.saving.privacy")
                        }
                    }
                }
            }
        }
    }
}
