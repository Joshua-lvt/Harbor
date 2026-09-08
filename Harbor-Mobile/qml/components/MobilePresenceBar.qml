// Persistent partner presence bar: the mobile equivalent of glancing at
// the desktop widget. Always visible, never noisy: it mirrors the
// committed presence aggregate and the shared current activity only.
// Tapping opens the chat. It is NOT a notification: nothing here pops,
// sounds, or expires.
import QtQuick
import QtQuick.Layouts

Rectangle {
    id: bar

    property string partnerName: ""
    property string partnerState: "offline"
    property string partnerActivity: ""
    property string avatarSource: ""
    property bool blocked: false
    // Optional shared theme; without one the bar keeps its shipped colors.
    property var theme

    signal openChat()

    implicitHeight: 72
    readonly property color stateColor: bar.theme ? bar.theme.stateColor(bar.partnerState)
        : bar.partnerState === "online" ? "#4ade80"
        : bar.partnerState === "idle" ? "#fbbf24" : "#64748b"
    readonly property string stateText: bar.theme ? bar.theme.stateText(bar.partnerState)
        : bar.partnerState === "online" ? qsTr("Online")
        : bar.partnerState === "idle" ? qsTr("Away") : qsTr("Offline")
    color: bar.theme ? bar.theme.bar : "#0e2736"
    border.color: bar.theme ? bar.theme.borderSubtle : "#2f4f60"
    border.width: 1

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 16
        anchors.rightMargin: 12
        spacing: 12

        Rectangle {
            Layout.preferredWidth: 44
            Layout.preferredHeight: 44
            radius: 22
            color: bar.theme ? bar.theme.card : "#16303f"
            border.color: bar.stateColor
            border.width: 2

            Text {
                anchors.centerIn: parent
                text: bar.partnerName.length > 0 ? bar.partnerName.charAt(0).toUpperCase() : "?"
                color: bar.theme ? bar.theme.textPrimary : "#e6f2f7"
                font.pixelSize: 20
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2

            Text {
                text: bar.blocked ? qsTr("Mobile ↔ Mobile not supported")
                    : (bar.partnerName.length > 0 ? bar.partnerName : qsTr("No partner"))
                color: bar.theme ? bar.theme.textPrimary : "#e6f2f7"
                font.pixelSize: 16
                font.bold: true
                elide: Text.ElideRight
                Layout.fillWidth: true
            }

            RowLayout {
                spacing: 6
                Rectangle {
                    Layout.preferredWidth: 8
                    Layout.preferredHeight: 8
                    radius: 4
                    color: bar.stateColor
                }
                Text {
                    text: bar.partnerActivity.length > 0
                        ? bar.stateText + " · " + bar.partnerActivity
                        : bar.stateText
                    color: bar.theme ? bar.theme.textSecondary : "#9db8c4"
                    font.pixelSize: 13
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
            }
        }

        MobileButton {
            text: qsTr("Chat")
            theme: bar.theme
            implicitWidth: 84
            implicitHeight: 44
            enabled: !bar.blocked && bar.partnerName.length > 0
            onClicked: bar.openChat()
        }
    }
}
