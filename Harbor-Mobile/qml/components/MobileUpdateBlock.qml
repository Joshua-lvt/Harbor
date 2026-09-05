// Blocking card for mandatory updates: while the updater reports a
// discovered update (available, downloading, ready) the shell shows only
// this — no skip path. A mere check failure never blocks: offline stays
// usable and the host retries on its own cadence.
import QtQuick
import QtQuick.Layouts

Rectangle {
    id: block

    // "idle" | "checking" | "available" | "downloading" | "ready" | "error"
    property string updateStatus: "idle"
    property string updateVersion: ""
    property real updateProgress: 0
    property string updateError: ""

    signal retryUpdate()
    signal installUpdate()

    readonly property bool blocked: block.updateStatus === "available"
        || block.updateStatus === "downloading" || block.updateStatus === "ready"

    color: "#0a1a24"
    visible: block.blocked

    Accessible.role: Accessible.AlertMessage
    Accessible.name: qsTr("Harbor update")

    ColumnLayout {
        anchors.centerIn: parent
        width: Math.min(parent.width - 48, 380)
        spacing: 14
        visible: block.blocked

        Text {
            text: qsTr("Harbor update")
            color: "#e6f2f7"
            font.pixelSize: 24
            font.bold: true
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        Text {
            text: block.updateVersion.length > 0
                ? qsTr("Version %1 is ready").arg(block.updateVersion)
                : qsTr("Checking for updates…")
            color: "#9db8c4"
            font.pixelSize: 14
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        Rectangle {
            visible: block.updateStatus === "downloading"
            Layout.fillWidth: true
            Layout.preferredHeight: 8
            radius: 4
            color: "#16303f"

            Rectangle {
                anchors.top: parent.top
                anchors.bottom: parent.bottom
                anchors.left: parent.left
                width: parent.width * Math.max(0, Math.min(1, block.updateProgress))
                radius: 4
                color: "#4ade80"
            }
        }

        Text {
            visible: block.updateStatus === "downloading"
            text: qsTr("Downloading… %1%").arg(Math.round(block.updateProgress * 100))
            color: "#9db8c4"
            font.pixelSize: 13
            horizontalAlignment: Text.AlignHCenter
            Layout.fillWidth: true
        }

        MobileButton {
            visible: block.updateStatus === "ready"
            text: qsTr("Install now")
            primary: true
            Layout.fillWidth: true
            Layout.preferredHeight: 52
            onClicked: block.installUpdate()
        }

        Text {
            visible: block.updateStatus === "ready"
            text: qsTr("The system installer opens next. Reopen Harbor afterwards.")
            color: "#9db8c4"
            font.pixelSize: 13
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }

        Text {
            text: qsTr("Updates install automatically and cannot be skipped.")
            color: "#64748b"
            font.pixelSize: 12
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            Layout.fillWidth: true
        }
    }
}
