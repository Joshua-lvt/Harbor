import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

HarborCard {
    id: root

    property string deviceName: ""
    property string deviceType: ""
    property string status: "offline" // online, connecting, offline
    property string statusDetail: ""
    property string lastSeen: ""
    property string iconName: "monitor"
    property bool showAction: true
    signal connectRequested()
    signal disconnectRequested()
    signal manageRequested()

    title: deviceName
    subtitle: deviceType + (statusDetail.length > 0 ? " · " + statusDetail : "")
    implicitWidth: 280

    Accessible.description: deviceType + ", " + statusBadge.text

    RowLayout {
        width: parent.width
        spacing: Theme.sp3

        Rectangle {
            implicitWidth: 52
            implicitHeight: 52
            radius: Theme.radius
            color: Theme.surfaceStrong
            border.width: 1
            border.color: Theme.surfaceBorder

            HarborIcon {
                anchors.centerIn: parent
                name: root.iconName
                color: root.status === "online" ? Theme.actionPrimary : Theme.iconSecondary
                implicitWidth: 24
                implicitHeight: 24
            }

            Rectangle {
                width: 11
                height: 11
                radius: 6
                anchors.right: parent.right
                anchors.rightMargin: -2
                anchors.bottom: parent.bottom
                anchors.bottomMargin: -2
                // Shape + color both signal the state: filled ring for online,
                // hollow for connecting, crossed dot for offline.
                color: root.status === "online" ? Theme.online
                    : root.status === "connecting" ? "transparent" : Theme.offline
                border.width: 2
                border.color: root.status === "online" ? Theme.online
                    : root.status === "connecting" ? Theme.warning : Theme.offline
            }
        }

        HarborBadge {
            id: statusBadge

            text: root.status === "online" ? I18n.t("common.status.connected")
                : root.status === "connecting" ? I18n.t("common.status.connecting")
                : I18n.t("common.status.offline")
            tone: root.status === "online" ? "success"
                : root.status === "connecting" ? "warning" : "neutral"
            showDot: true
            compact: true
            Layout.fillWidth: true
        }
    }

    RowLayout {
        visible: root.showAction
        width: parent.width
        spacing: Theme.sp2

        HarborButton {
            text: I18n.t("common.actions.manage")
            variant: "quiet"
            Layout.fillWidth: true
            onClicked: root.manageRequested()
        }
        HarborButton {
            text: root.status === "online" ? I18n.t("common.actions.disconnect") : I18n.t("common.actions.connect")
            variant: root.status === "online" ? "secondary" : "primary"
            busy: root.status === "connecting"
            Layout.fillWidth: true
            onClicked: {
                if (root.status === "online") root.disconnectRequested()
                else root.connectRequested()
            }
        }
    }
}
