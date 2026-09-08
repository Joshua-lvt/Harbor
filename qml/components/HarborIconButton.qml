import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Button {
    id: control

    // Prefer iconName (HarborIcon) for new UI; iconText remains for legacy
    // text-glyph call sites.
    property string iconName: ""
    property string iconText: ""
    property string accessibleName: ""
    property string toolTip: ""
    property int buttonSize: 40
    property color iconColor: checked ? Theme.actionPrimary : Theme.iconSecondary
    property color fillColor: checked ? Theme.surfaceStrong : "transparent"

    readonly property bool usesIcon: iconName.length > 0

    implicitWidth: Math.max(Theme.hitTarget, buttonSize)
    implicitHeight: Math.max(Theme.hitTarget, buttonSize)
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus

    Accessible.name: accessibleName.length > 0 ? accessibleName : (toolTip.length > 0 ? toolTip : text)
    // Keep icon-only actions equivalent to labeled buttons for keyboard use.
    Keys.onReturnPressed: event => {
        if (control.enabled)
            control.clicked()
        event.accepted = true
    }
    Keys.onEnterPressed: event => {
        if (control.enabled)
            control.clicked()
        event.accepted = true
    }

    ToolTip.visible: hovered && toolTip.length > 0
    ToolTip.text: toolTip
    ToolTip.delay: 500

    contentItem: Item {
        HarborIcon {
            visible: control.usesIcon
            anchors.centerIn: parent
            name: control.iconName
            color: control.enabled ? control.iconColor : Theme.iconDisabled
            implicitWidth: Math.round(control.buttonSize * 0.45)
            implicitHeight: Math.round(control.buttonSize * 0.45)
        }

        Text {
            visible: !control.usesIcon
            anchors.centerIn: parent
            text: control.iconText
            color: control.enabled ? control.iconColor : Theme.iconDisabled
            font.pixelSize: Math.round(control.buttonSize * 0.45)
            font.weight: Font.DemiBold
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
        }
    }

    background: Rectangle {
        radius: Theme.radiusSmall
        color: control.down ? Theme.surfacePressed
              : control.hovered ? Theme.surfaceHover : control.fillColor
        border.width: control.visualFocus ? Theme.focusWidth : 0
        border.color: Theme.focusRing

        Behavior on color {
            ColorAnimation { duration: Theme.duration(Theme.motionFast) }
        }
    }
}
