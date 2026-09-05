import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Control {
    id: root

    default property alias content: pageContent.data
    property string accessibleName: ""
    property int pagePadding: Theme.sp5
    property int contentSpacing: Theme.sp5
    property int maximumContentWidth: Theme.maxPageWidth
    property bool centerContent: true
    readonly property alias flickable: scrollView.contentItem

    implicitWidth: 800
    implicitHeight: 600
    padding: 0
    focusPolicy: Qt.NoFocus
    Accessible.role: Accessible.Pane
    Accessible.name: accessibleName

    background: Item {
    }

    contentItem: ScrollView {
        id: scrollView

        clip: true
        contentWidth: availableWidth
        ScrollBar.horizontal.policy: ScrollBar.AlwaysOff
        ScrollBar.vertical.policy: ScrollBar.AsNeeded

        Item {
            implicitWidth: scrollView.availableWidth
            implicitHeight: pageContent.implicitHeight + root.pagePadding * 2

            ColumnLayout {
                id: pageContent

                x: root.centerContent ? Math.max(root.pagePadding, (parent.width - width) / 2) : root.pagePadding
                y: root.pagePadding
                width: Math.max(0, Math.min(root.maximumContentWidth, parent.width - root.pagePadding * 2))
                spacing: root.contentSpacing
            }

        }

    }

}
