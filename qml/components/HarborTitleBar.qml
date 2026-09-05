import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

FocusScope {
    id: root

    property var targetWindow: null
    property string title: I18n.t("shell.windowTitle")
    property bool showLogo: true
    property bool showMinimize: true
    property bool showMaximize: true
    property bool showClose: true
    property bool maximized: false
    signal minimizeRequested()
    signal maximizeRequested()
    signal restoreRequested()
    signal closeRequested()

    implicitHeight: Theme.shellTitleBarHeight
    implicitWidth: 640

    Rectangle {
        anchors.fill: parent
        color: Theme.surface
        border.width: 0
    }

    MouseArea {
        anchors.fill: parent
        acceptedButtons: Qt.LeftButton
        onPressed: {
            if (root.targetWindow && root.targetWindow.startSystemMove)
                root.targetWindow.startSystemMove()
        }
        onDoubleClicked: {
            if (root.maximized) root.restoreRequested()
            else root.maximizeRequested()
        }
    }

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Theme.sp3
        anchors.rightMargin: Theme.sp1
        spacing: Theme.sp2

        HarborLogo {
            visible: root.showLogo
            compact: true
            showWordmark: false
            Layout.preferredWidth: 30
        }

        Text {
            text: root.title
            color: Theme.textSecondary
            font.family: Theme.fontFamily
            font.pixelSize: Theme.fontSmall
            font.weight: Font.DemiBold
            elide: Text.ElideRight
            Layout.fillWidth: true
        }

        HarborIconButton {
            visible: root.showMinimize
            iconName: "minus"
            accessibleName: I18n.t("shell.titlebar.minimize")
            toolTip: I18n.t("shell.titlebar.minimize")
            buttonSize: root.height
            onClicked: root.minimizeRequested()
        }

        HarborIconButton {
            visible: root.showMaximize
            iconName: root.maximized ? "restore" : "maximize"
            accessibleName: root.maximized ? I18n.t("shell.titlebar.restore") : I18n.t("shell.titlebar.maximize")
            toolTip: accessibleName
            buttonSize: root.height
            onClicked: {
                if (root.maximized) root.restoreRequested()
                else root.maximizeRequested()
            }
        }

        HarborIconButton {
            visible: root.showClose
            iconName: "close"
            accessibleName: I18n.t("shell.titlebar.close")
            toolTip: I18n.t("shell.titlebar.close")
            buttonSize: root.height
            iconColor: hovered ? Theme.actionDangerText : Theme.iconSecondary
            fillColor: hovered ? Theme.actionDanger : "transparent"
            onClicked: root.closeRequested()
        }
    }
}
