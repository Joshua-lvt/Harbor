import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Control {
    id: control

    property string tone: "accent" // accent, success, warning, danger, neutral
    property string text: ""
    property bool showDot: false
    property bool compact: false
    property color badgeColor: {
        if (tone === "success") return Theme.success
        if (tone === "warning") return Theme.warning
        if (tone === "danger") return Theme.danger
        if (tone === "neutral") return Theme.offline
        return Theme.accent
    }

    implicitWidth: contentItem.implicitWidth + leftPadding + rightPadding
    implicitHeight: compact ? 22 : 28
    leftPadding: compact ? Theme.sp2 : Theme.sp3
    rightPadding: leftPadding

    Accessible.role: Accessible.StaticText
    Accessible.name: text

    contentItem: RowLayout {
        spacing: Theme.sp1
        Rectangle {
            visible: control.showDot
            implicitWidth: 7
            implicitHeight: 7
            radius: 4
            color: control.badgeColor
        }
        Text {
            text: control.text
            color: Theme.text
            font.pixelSize: control.compact ? Theme.fontTiny : Theme.fontSmall
            font.weight: Font.DemiBold
            elide: Text.ElideRight
        }
    }

    background: Rectangle {
        radius: Theme.radiusPill
        color: Qt.rgba(control.badgeColor.r, control.badgeColor.g, control.badgeColor.b, Theme.dark ? 0.2 : 0.16)
        border.width: 1
        border.color: Qt.rgba(control.badgeColor.r, control.badgeColor.g, control.badgeColor.b, 0.55)
    }
}
