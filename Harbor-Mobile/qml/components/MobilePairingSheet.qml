// Mobile pairing sheet: Harbor-ID pairing in a phone-sized flow. This
// device shows its own Harbor ID and invites the peer's Harbor ID — no
// codes. A 2 s poller drives status/incoming while open; closing resets
// the local session only — the control plane owns the truth.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Rectangle {
    id: sheet

    property string ownHarborId: ""
    property string incomingHarborId: ""
    property string phase: ""
    property string errorText: ""
    property bool serverConfigured: false
    property bool busy: false
    // Copy-feedback flash (which ID was copied is exactly `ownHarborId`).
    property bool copyFlash: false
    // Optional shared theme; without one the sheet keeps its shipped colors.
    property var theme

    signal connectWithId(string harborId)
    signal copyId()
    signal acceptRequest()
    signal declineRequest()
    signal cancelFlow()
    signal resetFlow()

    // Polling hooks (host connects core calls).
    signal pollStatus()
    signal pollIncoming()
    signal close()

    /// Canonical Harbor-ID check, mirroring the core and server gates.
    function isValidHarborId(value) {
        return /^harbor-[0-9a-fA-F]{8}$/.test(String(value || ""))
    }

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
                    text: qsTr("Connect with someone")
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
                text: qsTr("Your Harbor ID")
                color: sheet.theme ? sheet.theme.textSecondary : "#9db8c4"
                font.pixelSize: sheet.theme ? sheet.theme.fontSmall : 13
                Layout.fillWidth: true
            }

            Label {
                text: sheet.ownHarborId.length > 0 ? sheet.ownHarborId : qsTr("Not available")
                color: sheet.theme ? sheet.theme.textPrimary : "#e6f2f7"
                font.pixelSize: 24
                font.bold: true
                font.family: "monospace"
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                Layout.fillWidth: true
            }

            MobileButton {
                theme: sheet.theme
                text: sheet.copyFlash ? qsTr("Harbor ID copied") : qsTr("Copy Harbor ID")
                enabled: sheet.ownHarborId.length > 0
                Layout.fillWidth: true
                Layout.preferredHeight: 52
                onClicked: {
                    sheet.copyId()
                    sheet.copyFlash = true
                    copyFlashTimer.restart()
                }
            }

            Label {
                text: qsTr("Type the other person's Harbor ID to start pairing.")
                color: sheet.theme ? sheet.theme.textSecondary : "#9db8c4"
                wrapMode: Text.WordWrap
                font.pixelSize: sheet.theme ? sheet.theme.fontSmall : 13
                Layout.fillWidth: true
            }

            RowLayout {
                Layout.fillWidth: true
                TextField {
                    id: harborField
                    Layout.fillWidth: true
                    Layout.preferredHeight: 52
                    placeholderText: qsTr("harbor-xxxxxxxx")
                    inputMethodHints: Qt.ImhNoPredictiveText | Qt.ImhNoAutoUppercase
                    enabled: sheet.serverConfigured && !sheet.busy
                    onAccepted: {
                        if (sheet.isValidHarborId(text))
                            sheet.connectWithId(text)
                    }
                }
                MobileButton {
                    theme: sheet.theme
                    text: qsTr("Connect")
                    Layout.preferredHeight: 52
                    enabled: sheet.serverConfigured && sheet.isValidHarborId(harborField.text) && !sheet.busy
                    onClicked: sheet.connectWithId(harborField.text)
                }
            }

            Label {
                text: qsTr("Format: harbor-xxxxxxxx")
                color: sheet.theme ? sheet.theme.textSecondary : "#9db8c4"
                font.pixelSize: sheet.theme ? sheet.theme.fontSmall : 13
                Layout.fillWidth: true
            }

            Label {
                text: sheet.phase.length > 0 ? qsTr("Status: %1").arg(sheet.phase) : ""
                color: sheet.theme ? sheet.theme.textSecondary : "#9db8c4"
                visible: sheet.phase.length > 0
                Layout.fillWidth: true
            }

            Label {
                text: qsTr("Request from: %1").arg(sheet.incomingHarborId)
                color: sheet.theme ? sheet.theme.textPrimary : "#e6f2f7"
                font.pixelSize: 20
                font.bold: true
                font.family: "monospace"
                visible: sheet.phase === "INCOMING" && sheet.incomingHarborId.length > 0
                wrapMode: Text.WordWrap
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
        id: copyFlashTimer
        interval: 2000
        repeat: false
        running: false
        onTriggered: sheet.copyFlash = false
    }

    Timer {
        interval: 2000
        repeat: true
        running: sheet.visible
        onTriggered: {
            if (!sheet.busy && sheet.phase !== "REQUESTING")
                sheet.pollIncoming()
            sheet.pollStatus()
        }
    }
}
