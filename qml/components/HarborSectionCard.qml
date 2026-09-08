import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Control {
    id: root

    default property alias content: body.data
    property alias headerActions: headerActions.data
    property string title: ""
    property string description: ""
    property string iconName: ""
    property color iconColor: Theme.iconSecondary
    property int cardPadding: Theme.sp4
    property int contentSpacing: Theme.sp3
    property color surfaceColor: Theme.surface

    implicitWidth: 360
    implicitHeight: cardLayout.implicitHeight + topPadding + bottomPadding
    padding: cardPadding
    focusPolicy: Qt.NoFocus
    Accessible.role: Accessible.Pane
    Accessible.name: title
    Accessible.description: description

    background: Rectangle {
        color: root.surfaceColor
        radius: Theme.radius
        border.width: 1
        border.color: Theme.borderSubtle
    }

    contentItem: ColumnLayout {
        id: cardLayout

        spacing: root.contentSpacing

        RowLayout {
            visible: root.title.length > 0 || root.description.length > 0 || root.iconName.length > 0 || headerActions.children.length > 0
            spacing: Theme.sp3
            Layout.fillWidth: true

            HarborIcon {
                visible: root.iconName.length > 0
                name: root.iconName
                color: root.iconColor
                implicitWidth: 22
                implicitHeight: 22
                Layout.alignment: Qt.AlignTop
            }

            ColumnLayout {
                spacing: Theme.sp1
                Layout.fillWidth: true

                Text {
                    visible: root.title.length > 0
                    text: root.title
                    color: Theme.textPrimary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontHeading
                    font.weight: Font.DemiBold
                    lineHeight: Theme.lineHeightHeading
                    lineHeightMode: Text.ProportionalHeight
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }

                Text {
                    visible: root.description.length > 0
                    text: root.description
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontSmall
                    lineHeight: Theme.lineHeightSmall
                    lineHeightMode: Text.ProportionalHeight
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }

            }

            Flow {
                id: headerActions

                visible: children.length > 0
                spacing: Theme.sp2
                Layout.alignment: Qt.AlignRight | Qt.AlignTop
            }

        }

        ColumnLayout {
            id: body

            spacing: root.contentSpacing
            Layout.fillWidth: true
        }

    }

}
