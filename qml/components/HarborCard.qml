import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

FocusScope {
    id: root

    default property alias content: body.data
    property string title: ""
    property string subtitle: ""
    property bool interactive: false
    property bool selected: false
    property int padding: Theme.sp4
    property color surfaceColor: selected ? Theme.surfaceStrong : Theme.surface
    signal clicked()

    implicitWidth: 280
    implicitHeight: cardLayout.implicitHeight + padding * 2
    activeFocusOnTab: interactive

    Accessible.role: interactive ? Accessible.Button : Accessible.Pane
    Accessible.name: title
    Accessible.description: subtitle
    Accessible.focusable: interactive
    Accessible.onPressAction: if (interactive) root.clicked()

    Keys.onSpacePressed: event => {
        if (interactive) {
            root.clicked()
            event.accepted = true
        }
    }
    Keys.onReturnPressed: event => {
        if (interactive) {
            root.clicked()
            event.accepted = true
        }
    }

    Rectangle {
        anchors.fill: parent
        radius: Theme.radius
        color: mouseArea.pressed && root.interactive ? Theme.surfaceHighlight : root.surfaceColor
        border.width: root.activeFocus || root.selected ? 2 : 1
        border.color: root.activeFocus || root.selected ? Theme.accent : Theme.surfaceBorder

        Behavior on color {
            ColorAnimation { duration: AppState.reducedMotion ? 0 : Theme.animFast }
        }
    }

    ColumnLayout {
        id: cardLayout
        anchors.fill: parent
        anchors.margins: root.padding
        spacing: Theme.sp3

        ColumnLayout {
            visible: root.title.length > 0 || root.subtitle.length > 0
            spacing: Theme.sp1
            Layout.fillWidth: true

            Text {
                visible: root.title.length > 0
                text: root.title
                color: Theme.text
                font.pixelSize: Theme.fontHeading
                font.weight: Font.DemiBold
                elide: Text.ElideRight
                Layout.fillWidth: true
            }
            Text {
                visible: root.subtitle.length > 0
                text: root.subtitle
                color: Theme.textDim
                font.pixelSize: Theme.fontSmall
                wrapMode: Text.Wrap
                Layout.fillWidth: true
            }
        }

        Column {
            id: body
            spacing: Theme.sp2
            Layout.fillWidth: true
        }
    }

    MouseArea {
        id: mouseArea
        anchors.fill: parent
        enabled: root.interactive
        hoverEnabled: true
        cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
        onClicked: {
            root.forceActiveFocus()
            root.clicked()
        }
    }
}
