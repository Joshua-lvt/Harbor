// Mobile pairing sheet: the same identity pairing as desktop, in a
// phone-sized flow. Two roles: show my code (host) or enter theirs
// (peer). A 2 s poller drives status/incoming while open; closing resets
// the local session only — the control plane owns the truth.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Rectangle {
    id: sheet

    // "host" shows our code, "peer" enters theirs.
    property string role: "peer"
    property string code: ""
    property string phase: ""
    property string errorText: ""
    property bool serverConfigured: false
    property bool busy: false
    // Optional shared theme; without one the sheet keeps its shipped colors.
    property var theme

    signal createCode()
    signal submitCode(string code)
    signal acceptRequest()
    signal declineRequest()
    signal cancelFlow()
    signal resetFlow()

    // Polling hooks (host connects core calls).
    signal pollStatus()
    signal pollIncoming()
    signal close()

    color: sheet.theme ? sheet.theme.bar : "#0e2736"
    radius: sheet.theme ? sheet.theme.radiusLarge : 18
    border.color: sheet.theme ? sheet.theme.borderSubtle : "#2f4f60"
    border.width: 1

    ScrollView {
        id: scroller
        anchors.fill: parent

        ColumnLayout {
            width: scroller.availableWidth
            spacing: 12

            RowLayout {
                Layout.fillWidth: true
                Label {
                    text: qsTr("Pair with your partner")
                    color: sheet.theme ? sheet.theme.textPrimary : "#e6f2f7"
                    font.pixelSize: sheet.theme ? sheet.theme.fontHeading : 22
                    font.bold: true
                    Layout.fillWidth: true
                }
                MobileButton {
                theme: sheet.theme
                    text: qsTr("Close")
                    Layout.preferredHeight: 44
                    onClicked: sheet.close()
                }
            }

        Label {
            text: qsTr("Pairing links identities, not devices. Your phone joins the same relationship your PC uses.")
            color: sheet.theme ? sheet.theme.textSecondary : "#9db8c4"
            wrapMode: Text.WordWrap
            opacity: 0.7
            font.pixelSize: sheet.theme ? sheet.theme.fontSmall : 13
            Layout.fillWidth: true
        }

        Label {
            text: qsTr("Harbor network is not ready yet. Reopen this sheet to retry.")
            color: sheet.theme ? sheet.theme.textSecondary : "#9db8c4"
            visible: !sheet.serverConfigured
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        RowLayout {
            Layout.fillWidth: true
            MobileButton {
                theme: sheet.theme
                text: qsTr("Show my code")
                enabled: sheet.serverConfigured && !sheet.busy
                onClicked: { sheet.role = "host"; sheet.createCode() }
            }
            MobileButton {
                theme: sheet.theme
                text: qsTr("Enter code")
                enabled: sheet.serverConfigured && !sheet.busy
                onClicked: { sheet.role = "peer"; sheet.resetFlow() }
            }
        }

        Label {
            text: sheet.code
            color: sheet.theme ? sheet.theme.textPrimary : "#e6f2f7"
            visible: sheet.role === "host" && sheet.code.length > 0
            font.pixelSize: sheet.theme ? sheet.theme.fontDisplay : 40
            font.bold: true
            horizontalAlignment: Text.AlignHCenter
            Layout.fillWidth: true
        }

        RowLayout {
            visible: sheet.role === "peer"
            Layout.fillWidth: true
            TextField {
                id: codeField
                Layout.fillWidth: true
                Layout.preferredHeight: 52
                placeholderText: qsTr("6-digit code")
                inputMethodHints: Qt.ImhDigitsOnly
                maximumLength: 6
                enabled: sheet.serverConfigured && !sheet.busy
                onAccepted: {
                    if (text.length === 6)
                        sheet.submitCode(text)
                }
            }
            MobileButton {
                theme: sheet.theme
                text: qsTr("Join")
                Layout.preferredHeight: 52
                enabled: codeField.text.length === 6 && !sheet.busy
                onClicked: sheet.submitCode(codeField.text)
            }
        }

        Label {
            text: sheet.phase.length > 0 ? qsTr("Status: %1").arg(sheet.phase) : ""
            color: sheet.theme ? sheet.theme.textSecondary : "#9db8c4"
            visible: sheet.phase.length > 0
            Layout.fillWidth: true
        }

        Label {
            text: sheet.errorText
            color: sheet.theme ? sheet.theme.danger : "#f87171"
            visible: sheet.errorText.length > 0
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        RowLayout {
            Layout.fillWidth: true
            MobileButton {
                theme: sheet.theme
                text: qsTr("Accept")
                visible: sheet.phase === "INCOMING"
                onClicked: sheet.acceptRequest()
            }
            MobileButton {
                theme: sheet.theme
                text: qsTr("Decline")
                visible: sheet.phase === "INCOMING"
                onClicked: sheet.declineRequest()
            }
            MobileButton {
                theme: sheet.theme
                text: qsTr("Cancel")
                onClicked: sheet.cancelFlow()
            }
        }
        }
    }

    Timer {
        interval: 2000
        repeat: true
        running: sheet.visible
        onTriggered: {
            if (sheet.role === "host")
                sheet.pollIncoming()
            else
                sheet.pollStatus()
        }
    }
}
