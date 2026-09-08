import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Item {
    id: root

    property bool showWordmark: true
    property bool compact: false
    property string wordmark: "Harbor"
    property color accentColor: Theme.accent

    implicitWidth: showWordmark ? mark.width + label.implicitWidth + Theme.sp2 : mark.width
    implicitHeight: compact ? 32 : 40

    Accessible.role: Accessible.Graphic
    Accessible.name: wordmark

    RowLayout {
        anchors.fill: parent
        spacing: Theme.sp2

        Item {
            id: mark
            implicitWidth: root.compact ? 30 : 38
            implicitHeight: implicitWidth

            // Product mark: the shark artwork, clipped to a squircle so the
            // transparent source corners never show, at any size.
            Rectangle {
                anchors.fill: parent
                radius: width * 0.24
                color: "transparent"
                clip: true

                Image {
                    objectName: "logoMarkImage"
                    anchors.fill: parent
                    // Module-root-relative like Theme's fonts: QML files use
                    // flat resource aliases, so this resolves inside Harbor/.
                    source: Qt.resolvedUrl("images/harbor.png")
                    fillMode: Image.PreserveAspectCrop
                    asynchronous: true
                    Accessible.ignored: true
                }
            }
        }

        Text {
            id: label
            visible: root.showWordmark
            text: root.wordmark
            color: Theme.text
            font.pixelSize: root.compact ? Theme.fontHeading : Theme.fontTitle
            font.weight: Font.Bold
            font.letterSpacing: 0.3
            Layout.alignment: Qt.AlignVCenter
        }
    }
}
