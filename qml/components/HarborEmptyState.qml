import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

FocusScope {
    id: root

    property string iconName: "activity"
    property string title: I18n.t("state.empty.title")
    property string description: ""
    property string actionText: ""
    property string secondaryActionText: ""
    signal actionTriggered()
    signal secondaryActionTriggered()

    implicitWidth: 320
    implicitHeight: layout.implicitHeight

    Accessible.role: Accessible.Pane
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
            color: Theme.surfaceStrong
            border.width: 1
            border.color: Theme.surfaceBorder
            Layout.alignment: Qt.AlignHCenter

            HarborIcon {
                anchors.centerIn: parent
                name: root.iconName
                color: Theme.actionPrimary
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
            visible: root.description.length > 0
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
        RowLayout {
            visible: root.actionText.length > 0 || root.secondaryActionText.length > 0
            spacing: Theme.sp2
            Layout.alignment: Qt.AlignHCenter
            HarborButton {
                visible: root.secondaryActionText.length > 0
                text: root.secondaryActionText
                variant: "secondary"
                onClicked: root.secondaryActionTriggered()
            }
            HarborButton {
                visible: root.actionText.length > 0
                text: root.actionText
                onClicked: root.actionTriggered()
            }
        }
    }
}
