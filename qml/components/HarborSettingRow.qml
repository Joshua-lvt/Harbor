import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Control {
    id: root

    // Trailing controls MUST be declared as unqualified children
    // (`HarborSettingRow { HarborToggle {...} }`). Assigning the alias by
    // name (`trailing: HarborToggle {...}`) sends the layout into a silent
    // infinite loop at 100% CPU — no warning is printed.
    default property alias trailing: trailingSlot.data
    property string label: ""
    property string description: ""
    property string iconName: ""
    property color iconColor: Theme.iconSecondary
    property bool clickable: false
    property bool showDivider: true

    signal clicked()

    function activate() {
        if (clickable && enabled)
            clicked();

    }

    implicitWidth: 360
    implicitHeight: Math.max(Theme.hitTarget, rowLayout.implicitHeight + topPadding + bottomPadding)
    topPadding: Theme.sp3
    bottomPadding: Theme.sp3
    leftPadding: 0
    rightPadding: 0
    hoverEnabled: clickable
    focusPolicy: clickable ? Qt.StrongFocus : Qt.NoFocus
    Accessible.role: clickable ? Accessible.Button : Accessible.Pane
    Accessible.name: label
    Accessible.description: description
    Accessible.focusable: clickable
    Accessible.onPressAction: {
        if (clickable) {
            root.activate();
        }
    }
    Keys.onSpacePressed: (event) => {
        if (root.clickable) {
            root.activate();
            event.accepted = true;
        }
    }
    Keys.onReturnPressed: (event) => {
        if (root.clickable) {
            root.activate();
            event.accepted = true;
        }
    }
    Keys.onEnterPressed: (event) => {
        if (root.clickable) {
            root.activate();
            event.accepted = true;
        }
    }

    HoverHandler {
        id: pointer

        enabled: root.clickable && root.enabled
        acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
    }

    TapHandler {
        enabled: root.clickable && root.enabled
        acceptedButtons: Qt.LeftButton
        gesturePolicy: TapHandler.ReleaseWithinBounds
        onTapped: {
            root.forceActiveFocus();
            root.activate();
        }
    }

    background: Rectangle {
        color: root.clickable && pointer.hovered ? Theme.surfaceHover : "transparent"
        radius: Theme.radiusSmall

        Rectangle {
            visible: root.showDivider
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            height: 1
            color: Theme.divider
        }

        Rectangle {
            visible: root.visualFocus
            anchors.fill: parent
            color: "transparent"
            radius: parent.radius
            border.width: Theme.focusWidth
            border.color: Theme.focusRing
        }

        Behavior on color {
            ColorAnimation {
                duration: Theme.duration(Theme.motionFast)
            }

        }

    }

    contentItem: RowLayout {
        id: rowLayout

        spacing: Theme.sp3

        HarborIcon {
            visible: root.iconName.length > 0
            name: root.iconName
            color: root.enabled ? root.iconColor : Theme.iconDisabled
            implicitWidth: 22
            implicitHeight: 22
            Layout.alignment: Qt.AlignTop
            Layout.topMargin: Theme.sp1
        }

        ColumnLayout {
            spacing: Theme.sp1
            Layout.fillWidth: true

            Text {
                text: root.label
                color: root.enabled ? Theme.textPrimary : Theme.textDisabled
                font.family: Theme.fontFamily
                font.pixelSize: Theme.fontBody
                font.weight: Font.Medium
                lineHeight: Theme.lineHeightBody
                lineHeightMode: Text.ProportionalHeight
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }

            Text {
                visible: root.description.length > 0
                text: root.description
                color: root.enabled ? Theme.textSecondary : Theme.textDisabled
                font.family: Theme.fontFamily
                font.pixelSize: Theme.fontSmall
                lineHeight: Theme.lineHeightSmall
                lineHeightMode: Text.ProportionalHeight
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }

        }

        RowLayout {
            id: trailingSlot

            spacing: Theme.sp2
            Layout.alignment: Qt.AlignRight | Qt.AlignVCenter
        }

    }

}
