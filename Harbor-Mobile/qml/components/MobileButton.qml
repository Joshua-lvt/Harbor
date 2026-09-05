// Harbor mobile action button.
//
// Material's Android button can blend into the app background when the
// theme is customized. This control keeps Qt input semantics while making
// the touch target, border, and pressed state explicit.
import QtQuick
import QtQuick.Controls

Control {
    id: root

    property string text: ""
    property bool primary: false
    property bool danger: false
    // Optional shared theme; without one the control keeps its shipped colors.
    property var theme
    property bool advancedEffects: typeof HarborVisualEffects !== "undefined"
        ? HarborVisualEffects.advancedEffects : true // qmllint disable unqualified
    property bool pressed: false
    signal clicked()

    implicitWidth: Math.max(160, label.implicitWidth + leftPadding + rightPadding + 32)
    implicitHeight: root.theme ? root.theme.buttonHeight : 52

    opacity: enabled ? 1.0 : 0.72
    scale: root.advancedEffects && root.pressed && !(root.theme && root.theme.reducedMotion) ? 0.98 : 1.0
    Behavior on scale {
        enabled: root.advancedEffects
        NumberAnimation { duration: 80 }
    }

    background: Rectangle {
        radius: root.theme ? root.theme.radius : 16
        border.width: 1
        border.color: root.primary || root.danger ? "transparent"
            : (root.theme ? root.theme.borderSubtle : "#2f4f60")
        color: {
            if (!root.enabled)
                return "#1c3a4d"
            if (root.pressed)
                return root.primary ? (root.theme ? root.theme.accentDeep : "#6ff295")
                    : (root.danger ? "#f87171" : (root.theme ? root.theme.card : "#20485d"))
            if (root.primary)
                return root.theme ? root.theme.accent : "#4ade80"
            if (root.danger)
                return root.theme ? root.theme.actionDanger : "#ef4444"
            return root.theme ? root.theme.card : "#16303f"
        }
    }

    contentItem: Label {
        id: label
        text: root.text
        font.pixelSize: 16
        font.weight: Font.DemiBold
        color: root.primary ? (root.theme ? root.theme.accentText : "#08202c")
            : root.danger ? (root.theme ? root.theme.actionDangerText : "#08202c")
            : (root.theme ? root.theme.textPrimary : "#e6f2f7")
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }

    MouseArea {
        id: interaction
        anchors.fill: parent
        enabled: root.enabled
        onPressed: root.pressed = true
        onReleased: root.pressed = false
        onCanceled: root.pressed = false
        onClicked: root.clicked()
    }
}
