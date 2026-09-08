import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Slider {
    id: control

    property string label: ""
    property string unit: ""
    property int decimals: 0
    property bool showValue: true
    readonly property string displayValue: Number(value).toFixed(decimals) + unit

    focusPolicy: Qt.StrongFocus
    implicitWidth: 240
    implicitHeight: Math.max(Theme.hitTarget, label.length > 0 || showValue ? 58 : Theme.hitTarget)
    topPadding: label.length > 0 || showValue ? 27 : 8
    leftPadding: 8
    rightPadding: 8

    Accessible.name: label
    Accessible.description: displayValue

    Text {
        visible: control.label.length > 0
        anchors.left: parent.left
        anchors.top: parent.top
        text: control.label
        color: Theme.text
        font.pixelSize: Theme.fontBody
        font.weight: Font.Medium
    }
    Text {
        visible: control.showValue
        anchors.right: parent.right
        anchors.top: parent.top
        text: control.displayValue
        color: Theme.textDim
        font.pixelSize: Theme.fontSmall
    }

    background: Rectangle {
        x: control.leftPadding
        y: control.topPadding + (control.availableHeight - height) / 2
        width: control.availableWidth
        height: 6
        radius: 3
        color: Theme.surfaceStrong
        border.width: 1
        border.color: Theme.surfaceBorder

        Rectangle {
            width: control.visualPosition * parent.width
            height: parent.height
            radius: parent.radius
            color: Theme.accent
        }
    }

    handle: Rectangle {
        x: control.leftPadding + control.visualPosition * (control.availableWidth - width)
        y: control.topPadding + (control.availableHeight - height) / 2
        implicitWidth: control.pressed ? 22 : 20
        implicitHeight: implicitWidth
        radius: width / 2
        color: "white"
        border.width: control.visualFocus ? 3 : 2
        border.color: Theme.accentDeep

        Behavior on implicitWidth {
            NumberAnimation { duration: Theme.duration(Theme.motionFast) }
        }
    }
}
