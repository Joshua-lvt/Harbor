import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

AbstractButton {
    id: control

    property string description: ""
    property string iconName: ""
    property color accentColor: Theme.actionPrimary
    property bool compact: false

    function activate() {
        if (!enabled)
            return ;

        if (checkable)
            checked = !checked;

        clicked();
    }

    checkable: true
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    implicitWidth: Math.max(Theme.hitTarget, contentItem.implicitWidth + leftPadding + rightPadding)
    implicitHeight: Math.max(Theme.hitTarget, contentItem.implicitHeight + topPadding + bottomPadding)
    leftPadding: compact ? Theme.sp3 : Theme.sp4
    rightPadding: leftPadding
    topPadding: Theme.sp2
    bottomPadding: Theme.sp2
    Accessible.role: checkable ? Accessible.CheckBox : Accessible.Button
    Accessible.name: text
    Accessible.description: description
    Accessible.checked: checked
    Accessible.onPressAction: control.activate()
    Keys.onSpacePressed: (event) => {
        control.activate();
        event.accepted = true;
    }
    Keys.onReturnPressed: (event) => {
        control.activate();
        event.accepted = true;
    }
    Keys.onEnterPressed: (event) => {
        control.activate();
        event.accepted = true;
    }

    contentItem: RowLayout {
        spacing: Theme.sp2

        HarborIcon {
            visible: control.iconName.length > 0
            name: control.iconName
            color: control.enabled ? (control.checked ? Theme.actionPrimaryText : control.accentColor) : Theme.iconDisabled
            implicitWidth: 18
            implicitHeight: 18
            Layout.alignment: Qt.AlignVCenter
        }

        Text {
            text: control.text
            color: !control.enabled ? Theme.textDisabled : control.checked ? Theme.actionPrimaryText : Theme.textPrimary
            font.family: Theme.fontFamily
            font.pixelSize: Theme.fontLabel
            font.weight: Font.DemiBold
            wrapMode: Text.Wrap
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
            Layout.fillWidth: true
        }

    }

    background: Rectangle {
        radius: Theme.radiusPill
        color: !control.enabled ? Theme.actionDisabled : control.down ? (control.checked ? Theme.actionPrimaryPressed : Theme.surfacePressed) : control.hovered ? (control.checked ? Theme.actionPrimaryHover : Theme.surfaceHover) : control.checked ? Theme.actionPrimary : Theme.surfaceInteractive
        border.width: control.visualFocus ? Theme.focusWidth : 1
        border.color: control.visualFocus ? Theme.focusRing : control.checked ? control.accentColor : Theme.borderSubtle
        opacity: control.enabled ? 1 : Theme.opacityDisabled

        Behavior on color {
            ColorAnimation {
                duration: Theme.duration(Theme.motionFast)
            }

        }

    }

}
