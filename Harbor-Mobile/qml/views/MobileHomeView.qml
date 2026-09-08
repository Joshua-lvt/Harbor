// Mobile Home: partner first, actions second. Never a tech dashboard.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"

ScrollView {
    id: view

    property bool sessionValid: false
    property string partnerName: ""
    property string partnerState: "offline"
    property string partnerActivity: ""
    property string partnerAvatar: ""
    property bool isCompanion: false
    property string callState: "idle"
    property bool blocked: false
    // Optional shared theme; without one the view keeps its shipped colors.
    property var theme

    signal openChat()
    signal enterCall()
    signal requestTakeover()
    signal openPairing()

    readonly property bool inCall: callState === "connected" || callState === "connecting"

    ColumnLayout {
        width: view.availableWidth
        spacing: 16

        Label {
            text: view.blocked
                ? qsTr("Harbor Mobile currently requires a desktop peer.")
                : (view.sessionValid
                    ? (view.partnerName.length > 0 ? view.partnerName : qsTr("Partner"))
                    : qsTr("Not paired yet"))
            font.pixelSize: view.theme ? view.theme.fontTitle : 26
            font.bold: true
            color: view.theme ? view.theme.textPrimary : "#e6f2f7"
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        Label {
            text: view.partnerActivity.length > 0 ? view.partnerActivity : ""
            visible: view.partnerActivity.length > 0 && !view.blocked
            color: view.theme ? view.theme.textSecondary : "#9db8c4"
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        MobileButton {
            text: qsTr("Pair with partner")
            theme: view.theme
            primary: true
            visible: !view.sessionValid
            Layout.fillWidth: true
            Layout.preferredHeight: view.theme ? view.theme.buttonHeight : 52
            onClicked: view.openPairing()
        }

        MobileButton {
            text: qsTr("Open chat")
            theme: view.theme
            enabled: view.sessionValid && !view.blocked
            Layout.fillWidth: true
            Layout.preferredHeight: view.theme ? view.theme.buttonHeight : 52
            onClicked: view.openChat()
        }

        // Companion: the PC holds media until this phone takes over.
        // Standalone with persistent call: the call is simply there.
        MobileButton {
            text: view.isCompanion && view.inCall
                ? qsTr("Enter call on this phone")
                : qsTr("Call")
            theme: view.theme
            enabled: view.sessionValid && !view.blocked && view.callState !== "unavailable"
            Layout.fillWidth: true
            Layout.preferredHeight: view.theme ? view.theme.buttonHeight : 52
            onClicked: (view.isCompanion && view.inCall) ? view.requestTakeover() : view.enterCall()
        }

        Label {
            text: view.isCompanion
                ? qsTr("This phone extends your Harbor PC. Entering the call moves audio here; the PC leaves the call media.")
                : qsTr("Standalone phone. The call stays up while the session is valid.")
            color: view.theme ? view.theme.textSecondary : "#9db8c4"
            wrapMode: Text.WordWrap
            opacity: 0.7
            font.pixelSize: view.theme ? view.theme.fontSmall : 13
            Layout.fillWidth: true
        }
    }
}
