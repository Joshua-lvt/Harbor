// Mobile Call: mic starts MUTED, always. Unmute is an explicit tap and
// the state is unmissable. Leaving returns to the safe state.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"

ColumnLayout {
    id: view
    spacing: 16

    property string callState: "idle"
    property bool microphoneMuted: true
    property string microphonePermission: "unknown"
    property bool persistentCall: true
    property bool isCompanion: false
    // Optional shared theme; without one the view keeps its shipped colors.
    property var theme

    signal setMicMuted(bool muted)
    signal enterCall()
    signal acceptIncomingCall()
    signal declineIncomingCall()
    signal leaveCall()
    signal requestTakeover()
    signal setPersistentCall(bool on)

    readonly property bool inCall: callState === "connected" || callState === "connecting"

    Label {
        text: qsTr("Incoming call")
        visible: view.callState === "incoming"
        color: view.theme ? view.theme.textPrimary : "#e6f2f7"
        font.pixelSize: view.theme ? view.theme.fontHeading : 22
        font.bold: true
        Layout.fillWidth: true
    }

    RowLayout {
        visible: view.callState === "incoming"
        Layout.fillWidth: true
        MobileButton {
            text: qsTr("Accept")
            theme: view.theme
            primary: true
            Layout.fillWidth: true
            Layout.preferredHeight: view.theme ? view.theme.buttonHeight : 52
            onClicked: view.acceptIncomingCall()
        }
        MobileButton {
            text: qsTr("Decline")
            theme: view.theme
            danger: true
            Layout.fillWidth: true
            Layout.preferredHeight: view.theme ? view.theme.buttonHeight : 52
            onClicked: view.declineIncomingCall()
        }
    }

    Label {
        text: view.inCall ? qsTr("In call") : qsTr("No active call")
        color: view.theme ? view.theme.textPrimary : "#e6f2f7"
        font.pixelSize: view.theme ? view.theme.fontTitle : 24
        font.bold: true
        Layout.fillWidth: true
    }

    Label {
        text: view.microphonePermission === "granted"
            ? qsTr("Microphone permission granted")
            : qsTr("Microphone permission is requested only when you accept or start a call.")
        visible: view.callState === "unavailable" || view.microphonePermission !== "granted"
        color: view.theme ? view.theme.textSecondary : "#e6f2f7"
        wrapMode: Text.WordWrap
        opacity: 0.75
        Layout.fillWidth: true
    }

    MobileButton {
        // Unmissable mute state, large touch target.
        text: view.microphoneMuted ? qsTr("🔇 Muted — tap to unmute") : qsTr("🎙 Live — tap to mute")
        theme: view.theme
        enabled: view.inCall
        Layout.fillWidth: true
        Layout.preferredHeight: view.theme ? view.theme.buttonHeightLarge : 64
        onClicked: view.setMicMuted(!view.microphoneMuted)
    }

    MobileButton {
        text: view.isCompanion && view.inCall ? qsTr("Move call to this phone") : qsTr("Connect")
        theme: view.theme
        visible: !view.inCall || (view.isCompanion && view.inCall)
        Layout.fillWidth: true
        Layout.preferredHeight: view.theme ? view.theme.buttonHeight : 52
        onClicked: (view.isCompanion && view.inCall) ? view.requestTakeover() : view.enterCall()
    }

    MobileButton {
        text: qsTr("Leave")
        theme: view.theme
        danger: true
        visible: view.inCall
        Layout.fillWidth: true
        Layout.preferredHeight: view.theme ? view.theme.buttonHeight : 52
        onClicked: view.leaveCall()
    }

    CheckBox {
        text: qsTr("Persistent call")
        checked: view.persistentCall
        visible: !view.isCompanion
        onToggled: view.setPersistentCall(checked)
    }

    Label {
        text: qsTr("The microphone never transmits outside a call, and it joins every call muted.")
        color: view.theme ? view.theme.textSecondary : "#9db8c4"
        wrapMode: Text.WordWrap
        opacity: 0.7
        font.pixelSize: view.theme ? view.theme.fontSmall : 13
        Layout.fillWidth: true
    }
}
