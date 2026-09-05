import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

FocusScope {
    id: root

    default property alias content: contentLayer.data
    property string pageState: "content" // content, loading, empty, error
    property string title: ""
    property string description: ""
    property string details: ""
    property string iconName: pageState === "error" ? "error" : "activity"
    property string loadingText: I18n.t("common.status.loading")
    property string actionText: ""
    property bool actionBusy: false
    property bool showContentWhileLoading: false
    property int statePadding: Theme.sp5
    readonly property bool validState: ["content", "loading", "empty", "error"].indexOf(pageState) >= 0

    signal actionTriggered()

    implicitWidth: 360
    implicitHeight: 240
    Accessible.role: pageState === "error" ? Accessible.AlertMessage : Accessible.Pane
    Accessible.name: pageState === "loading" ? loadingText : title
    Accessible.description: description

    Item {
        id: contentLayer

        anchors.fill: parent
        visible: root.pageState === "content" || (root.pageState === "loading" && root.showContentWhileLoading)
        opacity: root.pageState === "loading" ? Theme.opacityMuted : 1

        Behavior on opacity {
            NumberAnimation {
                duration: Theme.duration(Theme.motionNormal)
                easing.type: Theme.animEasing
            }

        }

    }

    Rectangle {
        anchors.fill: parent
        visible: root.pageState !== "content" && (!root.showContentWhileLoading || root.pageState !== "loading")
        color: "transparent"

        ColumnLayout {
            anchors.centerIn: parent
            width: Math.max(0, Math.min(420, parent.width - root.statePadding * 2))
            spacing: Theme.sp3

            HarborSpinner {
                visible: root.pageState === "loading"
                spinnerSize: 40
                accessibleName: root.loadingText
                Layout.alignment: Qt.AlignHCenter
            }

            Rectangle {
                visible: root.pageState === "empty" || root.pageState === "error"
                implicitWidth: 64
                implicitHeight: 64
                radius: Theme.radiusLarge
                color: root.pageState === "error" ? Theme.withOpacity(Theme.danger, 0.16) : Theme.surfaceStrong
                border.width: 1
                border.color: root.pageState === "error" ? Theme.danger : Theme.borderSubtle
                Layout.alignment: Qt.AlignHCenter

                HarborIcon {
                    anchors.centerIn: parent
                    name: root.iconName
                    color: root.pageState === "error" ? Theme.danger : Theme.iconSecondary
                    implicitWidth: 30
                    implicitHeight: 30
                }

            }

            Text {
                visible: root.pageState === "loading" || root.title.length > 0
                text: root.pageState === "loading" ? root.loadingText : root.title
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
                visible: root.pageState !== "loading" && root.description.length > 0
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
                visible: root.pageState === "error" && root.details.length > 0
                text: root.details
                color: Theme.textMuted
                font.family: Theme.fontFamilyMonospace
                font.pixelSize: Theme.fontTiny
                lineHeight: Theme.lineHeightTiny
                lineHeightMode: Text.ProportionalHeight
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WrapAnywhere
                Layout.fillWidth: true
            }

            HarborButton {
                visible: root.pageState !== "loading" && root.actionText.length > 0
                text: root.actionText
                busy: root.actionBusy
                Layout.alignment: Qt.AlignHCenter
                onClicked: root.actionTriggered()
            }

        }

    }

    Rectangle {
        anchors.fill: parent
        visible: root.pageState === "loading" && root.showContentWhileLoading
        color: Theme.surfaceScrim

        ColumnLayout {
            anchors.centerIn: parent
            spacing: Theme.sp2

            HarborSpinner {
                spinnerSize: 40
                accessibleName: root.loadingText
                Layout.alignment: Qt.AlignHCenter
            }

            Text {
                text: root.loadingText
                color: Theme.textPrimary
                font.family: Theme.fontFamily
                font.pixelSize: Theme.fontBody
                font.weight: Font.DemiBold
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.Wrap
                Layout.maximumWidth: Math.max(0, root.width - root.statePadding * 2)
            }

        }

    }

}
