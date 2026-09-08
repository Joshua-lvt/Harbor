import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Button {
    id: control

    property string variant: "primary" // primary, secondary, quiet, danger
    property string iconText: ""
    property string iconName: ""
    property bool busy: false
    property bool pill: false
    property color foregroundColor: variant === "primary" || variant === "danger" ? "white" : Theme.text
    property color fillColor: {
        if (variant === "danger") return Theme.danger
        if (variant === "secondary") return Theme.surfaceStrong
        if (variant === "quiet") return "transparent"
        return Theme.accentDeep
    }

    enabled: !busy
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    implicitWidth: Math.max(112, contentItem.implicitWidth + leftPadding + rightPadding)
    implicitHeight: Math.max(Theme.hitTarget, contentItem.implicitHeight + topPadding + bottomPadding)
    leftPadding: Theme.sp4
    rightPadding: Theme.sp4
    topPadding: Theme.sp2
    bottomPadding: Theme.sp2

    Accessible.name: text
    Accessible.description: busy ? I18n.t("a11y.working") : text
    // Qt Quick Controls maps Space itself; Return/Enter are explicit so all
    // Harbor buttons offer the same keyboard activation contract.
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

    contentItem: RowLayout {
        spacing: Theme.sp2

        HarborSpinner {
            visible: control.busy
            spinnerSize: 18
            color: control.foregroundColor
            accessibleName: I18n.t("a11y.working")
            Layout.alignment: Qt.AlignVCenter
        }

        HarborIcon {
            visible: control.iconName.length > 0 && !control.busy
            name: control.iconName
            color: control.foregroundColor
            implicitWidth: 18
            implicitHeight: 18
            Layout.alignment: Qt.AlignVCenter
        }

        Text {
            visible: control.iconText.length > 0 && !control.busy
            text: control.iconText
            color: control.foregroundColor
            font.pixelSize: Theme.fontBody + 2
            Layout.alignment: Qt.AlignVCenter
        }

        Text {
            text: control.text
            color: control.foregroundColor
            font.pixelSize: Theme.fontBody
            font.weight: Font.DemiBold
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
    }

    background: Rectangle {
        radius: control.pill ? Theme.radiusPill : Theme.radiusSmall
        color: !control.enabled ? Theme.surface : control.down ? Qt.darker(control.fillColor, 1.18)
              : control.hovered ? Qt.lighter(control.fillColor, 1.12) : control.fillColor
        border.width: control.visualFocus || control.variant === "secondary" || control.variant === "quiet" ? 1 : 0
        border.color: control.visualFocus ? Theme.accent : Theme.surfaceBorder
        opacity: control.enabled ? 1 : 0.55

        Behavior on color {
            ColorAnimation { duration: Theme.duration(Theme.motionFast) }
        }
    }
}
