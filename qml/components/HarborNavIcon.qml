import QtQuick

// Single visual home for every navigation icon. The box is always square
// and the drawing always fills it uniformly, so the underlying 24x24
// artwork can never stretch: width and height are one value, never two
// independent ones. State changes (normal/hover/active/pressed) only move
// color, opacity and glow — geometry never moves, so layouts never jump.
// The clickable area stays owned by the parent button; this item is only
// the fixed-size, centered glyph inside it.
Item {
    id: root

    property string name: ""
    property bool active: false
    property bool hovered: false
    // Fixed glyph box. 22 keeps the five destinations at the same visual
    // weight without touching the button's own hit-target size.
    property int iconSize: 22
    property color activeColor: Theme.actionPrimary
    property color inactiveColor: Theme.iconSecondary

    // One value drives both axes: a non-square HarborNavIcon cannot exist.
    width: iconSize
    height: iconSize
    implicitWidth: iconSize
    implicitHeight: iconSize

    Accessible.ignored: true

    // Soft glow behind the active destination. Opacity-only: no geometry.
    Rectangle {
        anchors.centerIn: parent
        width: parent.width + 10
        height: parent.height + 10
        radius: (parent.width + 10) / 2
        color: "transparent"
        border.width: root.active ? 1 : 0
        border.color: Qt.rgba(root.activeColor.r, root.activeColor.g,
                              root.activeColor.b, 0.35)
        opacity: root.active ? 1 : 0
        Behavior on opacity {
            NumberAnimation { duration: Theme.duration(Theme.motionFast) }
        }
    }

    HarborIcon {
        anchors.fill: parent
        name: root.name
        color: root.active ? root.activeColor : root.inactiveColor
        opacity: root.active ? 1.0 : (root.hovered ? 0.9 : 0.75)
        decorative: true

        Behavior on opacity {
            NumberAnimation { duration: Theme.duration(Theme.motionFast) }
        }
        Behavior on color {
            ColorAnimation { duration: Theme.duration(Theme.motionFast) }
        }
    }
}
