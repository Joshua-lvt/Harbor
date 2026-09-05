pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Preview-only stand-in for system-tray behavior. No QSystemTrayIcon exists in
// this prototype: the flyout shows what a tray menu would offer, and every
// action just reports back to the caller.
Popup {
    id: popup

    property string partnerName: AppState.partnerName
    property bool connectionOnline: AppState.connectionState === "connected"

    signal openRequested()
    signal minimizeRequested()
    signal quitRequested()

    parent: Overlay.overlay
    anchors.centerIn: parent
    modal: true
    focus: true
    padding: Theme.sp5
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutside
    width: Math.min(440, parent ? parent.width - Theme.sp5 * 2 : 440)

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
        id: layout

        spacing: Theme.sp4

        Accessible.role: Accessible.Dialog
        Accessible.name: I18n.t("tray.preview.title")

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.sp3

            HarborLogo {
                compact: true
                showWordmark: false
            }

            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.sp1

                Text {
                    text: I18n.t("tray.preview.title")
                    color: Theme.textPrimary
                    font.family: Theme.fontFamilyDisplay
                    font.pixelSize: Theme.fontTitle
                    font.weight: Font.Bold
                    lineHeight: Theme.lineHeightTitle
                    lineHeightMode: Text.ProportionalHeight
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }

                Text {
                    text: I18n.t("tray.description")
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontBody
                    lineHeight: Theme.lineHeightBody
                    lineHeightMode: Text.ProportionalHeight
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }
            }

            HarborIconButton {
                iconName: "close"
                accessibleName: I18n.t("a11y.closeDialog")
                toolTip: I18n.t("common.actions.close")
                buttonSize: 34
                Layout.alignment: Qt.AlignTop
                onClicked: popup.close()
            }
        }

        // The mock tray menu itself, mirroring tray.* strings.
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: menuColumn.implicitHeight + Theme.sp2 * 2
            radius: Theme.radius
            color: Theme.surfaceInteractive
            border.width: 1
            border.color: Theme.borderSubtle

            ColumnLayout {
                id: menuColumn

                anchors.fill: parent
                anchors.margins: Theme.sp2
                spacing: Theme.sp1

                HarborBadge {
                    text: I18n.t("developer.mockBadge")
                    tone: "accent"
                    compact: true
                    Layout.margins: Theme.sp1
                }

                Repeater {
                    model: [
                        { icon: "online", label: I18n.t("tray.open"), action: "open" },
                        { icon: "minus", label: I18n.t("tray.minimize"), action: "minimize" },
                        { icon: "close", label: I18n.t("tray.quit"), action: "quit" }
                    ]

                    HarborSettingRow {
                        required property var modelData
                        Layout.fillWidth: true
                        clickable: true
                        showDivider: modelData.action !== "quit"
                        iconName: modelData.icon
                        iconColor: modelData.action === "quit" ? Theme.actionDanger : Theme.iconSecondary
                        label: modelData.label
                        onClicked: {
                            popup.close()
                            if (modelData.action === "open")
                                popup.openRequested()
                            else if (modelData.action === "minimize")
                                popup.minimizeRequested()
                            else
                                popup.quitRequested()
                        }
                    }
                }
            }
        }

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.sp2

            Rectangle {
                implicitWidth: 8
                implicitHeight: 8
                radius: 4
                color: popup.connectionOnline ? Theme.online
                    : AppState.connectionState === "reconnecting" ? Theme.warning : Theme.offline
            }

            Text {
                Layout.fillWidth: true
                text: AppState.connectionState === "connected"
                    ? I18n.t("shell.status.connectedTo", { name: popup.partnerName })
                    : AppState.connectionState === "reconnecting"
                      ? I18n.t("shell.status.reconnecting")
                      : AppState.connectionState === "connecting"
                        ? I18n.t("shell.status.opening")
                        : I18n.t("shell.status.notConnected")
                color: Theme.textSecondary
                font.family: Theme.fontFamily
                font.pixelSize: Theme.fontSmall
                wrapMode: Text.Wrap
            }
        }

        Text {
            Layout.fillWidth: true
            text: I18n.t("shell.footer.prototype")
            color: Theme.textFaint
            font.family: Theme.fontFamily
            font.pixelSize: Theme.fontTiny
            font.letterSpacing: 1.1
            horizontalAlignment: Text.AlignHCenter
        }
    }
}
