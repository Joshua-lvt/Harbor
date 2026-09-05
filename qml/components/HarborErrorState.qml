import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

FocusScope {
    id: root

    property string title: I18n.t("state.error.title")
    property string description: I18n.t("state.error.description")
    property string details: ""
    property string retryText: I18n.t("common.actions.retry")
    property bool retrying: false
    signal retryRequested()

    implicitWidth: 340
    implicitHeight: layout.implicitHeight

    Accessible.role: Accessible.AlertMessage
    Accessible.name: title
    Accessible.description: description

    ColumnLayout {
        id: layout

        anchors.left: parent.left
        anchors.right: parent.right
        spacing: Theme.sp3

        Rectangle {
            implicitWidth: 72
            implicitHeight: 72
            radius: 36
            color: Qt.rgba(Theme.danger.r, Theme.danger.g, Theme.danger.b, 0.16)
            border.width: 1
            border.color: Theme.danger
            Layout.alignment: Qt.AlignHCenter

            HarborIcon {
                anchors.centerIn: parent
                name: "error"
                color: Theme.danger
                implicitWidth: 30
                implicitHeight: 30
            }
        }
        Text {
            text: root.title
            color: Theme.textPrimary
            font.family: Theme.fontFamily
            font.pixelSize: Theme.fontHeading
            font.weight: Font.DemiBold
            lineHeight: Theme.lineHeightHeading
            lineHeightMode: Text.ProportionalHeight
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        Text {
            text: root.description
            color: Theme.textSecondary
            font.family: Theme.fontFamily
            font.pixelSize: Theme.fontBody
            lineHeight: Theme.lineHeightBody
            lineHeightMode: Text.ProportionalHeight
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
        Text {
            visible: root.details.length > 0
            text: I18n.t("a11y.errorMessage", { message: root.details })
            color: Theme.textMuted
            font.family: Theme.fontFamilyMonospace
            font.pixelSize: Theme.fontTiny
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WrapAnywhere
            Layout.fillWidth: true
        }
        HarborButton {
            visible: root.retryText.length > 0
            text: root.retryText
            iconName: "refresh"
            busy: root.retrying
            Layout.alignment: Qt.AlignHCenter
            onClicked: root.retryRequested()
        }
    }
}
