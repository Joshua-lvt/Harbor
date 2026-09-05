import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

HarborCard {
    id: root

    property string label: ""
    property string value: "—"
    property string unit: ""
    property string iconText: ""
    property string iconName: ""
    property string trendText: ""
    property string trendTone: "neutral" // positive, negative, neutral
    property real progress: -1
    property color accentColor: Theme.accent

    implicitWidth: 210

    Accessible.role: Accessible.StaticText
    Accessible.name: (label.length > 0 ? label + ": " : "") + value + unit
    Accessible.description: trendText

    RowLayout {
        width: parent.width
        spacing: Theme.sp3

        Rectangle {
            visible: root.iconName.length > 0 || root.iconText.length > 0
            implicitWidth: 42
            implicitHeight: 42
            radius: Theme.radiusSmall
            color: Qt.rgba(root.accentColor.r, root.accentColor.g, root.accentColor.b, 0.16)

            HarborIcon {
                visible: root.iconName.length > 0
                anchors.centerIn: parent
                name: root.iconName
                color: root.accentColor
                implicitWidth: 22
                implicitHeight: 22
            }

            Text {
                visible: root.iconName.length === 0
                anchors.centerIn: parent
                text: root.iconText
                color: root.accentColor
                font.pixelSize: Theme.fontHeading
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 1
            Text {
                visible: root.label.length > 0
                text: root.label
                color: Theme.textFaint
                font.family: Theme.fontFamily
                font.pixelSize: Theme.fontTiny
                elide: Text.ElideRight
                Layout.fillWidth: true
            }
            RowLayout {
                spacing: Theme.sp1
                Text {
                    text: root.value
                    color: Theme.text
                    font.pixelSize: Theme.fontTitle
                    font.weight: Font.Bold
                }
                Text {
                    visible: root.unit.length > 0
                    text: root.unit
                    color: Theme.textDim
                    font.pixelSize: Theme.fontSmall
                    Layout.alignment: Qt.AlignBottom
                    Layout.bottomMargin: 3
                }
            }
            Text {
                visible: root.trendText.length > 0
                text: root.trendText
                color: root.trendTone === "positive" ? Theme.success
                     : root.trendTone === "negative" ? Theme.danger : Theme.textDim
                font.pixelSize: Theme.fontTiny
                elide: Text.ElideRight
                Layout.fillWidth: true
            }
        }
    }

    Rectangle {
        visible: root.progress >= 0
        width: parent.width
        height: 5
        radius: 3
        color: Theme.surfaceStrong
        Rectangle {
            width: Math.max(0, Math.min(1, root.progress)) * parent.width
            height: parent.height
            radius: parent.radius
            color: root.accentColor

            Behavior on width {
                NumberAnimation { duration: AppState.reducedMotion ? 0 : Theme.animNormal; easing.type: Theme.animEasing }
            }
        }
    }
}
