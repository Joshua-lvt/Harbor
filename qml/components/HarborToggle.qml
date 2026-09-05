import QtQuick
import QtQuick.Controls

Switch {
    id: control

    property string description: ""

    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    spacing: Theme.sp3
    // Keep implicit sizing independent from the assigned control width.
    // Deriving it from child widths that themselves follow parent.width can
    // leave layouts polishing forever when several switches share a layout.
    implicitWidth: Math.max(text.length > 0 || description.length > 0 ? 180 : track.implicitWidth,
                            Math.min(320, Math.max(titleText.implicitWidth,
                                                  descriptionText.visible
                                                  ? descriptionText.implicitWidth : 0)
                                          + track.implicitWidth + spacing))
    implicitHeight: Math.max(Theme.hitTarget, contentItem.implicitHeight)

    Accessible.name: text
    Accessible.description: description

    indicator: Rectangle {
        id: track
        implicitWidth: 48
        implicitHeight: 28
        x: control.mirrored ? control.width - width : 0
        y: (control.height - height) / 2
        radius: height / 2
        color: control.checked ? Theme.accentDeep : Theme.surfaceStrong
        border.width: control.visualFocus ? 2 : 1
        border.color: control.visualFocus ? Theme.accent : Theme.surfaceBorder

        Rectangle {
            width: 20
            height: 20
            radius: 10
            y: 4
            x: 4 + control.visualPosition * (track.width - width - 8)
            color: control.checked ? "white" : Theme.textDim

            Behavior on x {
                NumberAnimation { duration: Theme.duration(Theme.motionNormal); easing.type: Theme.animEasing }
            }
        }
    }

    contentItem: Column {
        leftPadding: control.mirrored ? 0 : track.width + control.spacing
        rightPadding: control.mirrored ? track.width + control.spacing : 0
        anchors.verticalCenter: parent.verticalCenter
        spacing: 2

        Text {
            id: titleText

            width: Math.max(0, control.availableWidth - parent.leftPadding - parent.rightPadding)
            text: control.text
            color: control.enabled ? Theme.text : Theme.textFaint
            font.pixelSize: Theme.fontBody
            font.weight: Font.Medium
            wrapMode: Text.Wrap
        }
        Text {
            id: descriptionText

            width: Math.max(0, control.availableWidth - parent.leftPadding - parent.rightPadding)
            visible: control.description.length > 0
            text: control.description
            color: Theme.textDim
            font.pixelSize: Theme.fontSmall
            wrapMode: Text.Wrap
        }
    }
}
