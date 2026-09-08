import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Control {
    id: root

    default property alias actions: actionFlow.data
    property string eyebrow: ""
    property string title: ""
    property string subtitle: ""
    property string iconName: ""
    property color iconColor: Theme.iconSecondary
    property int compactBreakpoint: Theme.breakpointCompact
    readonly property bool compact: width < compactBreakpoint

    implicitWidth: 560
    implicitHeight: headerLayout.implicitHeight
    padding: 0
    focusPolicy: Qt.NoFocus
    Accessible.role: Accessible.Pane
    Accessible.name: title
    Accessible.description: subtitle

    background: Item {
    }

    contentItem: GridLayout {
        id: headerLayout

        columns: root.compact ? 1 : 2
        columnSpacing: Theme.sp4
        rowSpacing: Theme.sp3

        RowLayout {
            spacing: Theme.sp3
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignTop

            Rectangle {
                visible: root.iconName.length > 0
                color: Theme.surfaceStrong
                radius: Theme.radius
                border.width: 1
                border.color: Theme.borderSubtle
                implicitWidth: Theme.hitTargetLarge
                implicitHeight: Theme.hitTargetLarge
                Layout.alignment: Qt.AlignTop

                HarborIcon {
                    anchors.centerIn: parent
                    name: root.iconName
                    color: root.iconColor
                    implicitWidth: 24
                    implicitHeight: 24
                }

            }

            ColumnLayout {
                spacing: Theme.sp1
                Layout.fillWidth: true

                Text {
                    visible: root.eyebrow.length > 0
                    text: root.eyebrow
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontLabel
                    font.weight: Font.DemiBold
                    font.capitalization: Font.AllUppercase
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }

                Text {
                    text: root.title
                    color: Theme.textPrimary
                    font.family: Theme.fontFamilyDisplay
                    font.pixelSize: Theme.fontDisplay
                    font.weight: Font.Bold
                    lineHeight: Theme.lineHeightDisplay
                    lineHeightMode: Text.ProportionalHeight
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }

                Text {
                    visible: root.subtitle.length > 0
                    text: root.subtitle
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontSubtitle
                    lineHeight: Theme.lineHeightBody
                    lineHeightMode: Text.ProportionalHeight
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }

            }

        }

        Flow {
            id: actionFlow

            visible: children.length > 0
            spacing: Theme.sp2
            flow: Flow.LeftToRight
            Layout.fillWidth: root.compact
            Layout.alignment: root.compact ? Qt.AlignLeft : Qt.AlignRight | Qt.AlignTop
            Layout.preferredWidth: root.compact ? headerLayout.width : implicitWidth
        }

    }

}
