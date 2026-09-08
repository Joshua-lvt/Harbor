import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Renders the single active toast from MockController's canonical queue. The
// host owns no ordering, expiry, or dismissal state of its own; it resolves
// catalog keys at render time, so switching locale updates a toast that is
// already on screen.
Item {
    id: host

    property int edgeMargin: Theme.sp5
    property int topOffset: Theme.shellTitleBarHeight + Theme.sp2
    // The mock queue is enabled only for QML tests and the internal preview.
    // Production has no fake toast source; native notifications stay external.
    property bool mockProviderEnabled: true

    readonly property var toast: mockProviderEnabled ? MockController.activeToast : ({})
    readonly property bool hasToast: mockProviderEnabled && MockController.toastActive
    readonly property string resolvedTitle: hasToast && toast.titleKey.length > 0
        ? I18n.t(toast.titleKey, toast.titleParams) : ""
    readonly property string resolvedDescription: hasToast && toast.descriptionKey.length > 0
        ? I18n.t(toast.descriptionKey, toast.descriptionParams) : ""
    readonly property color categoryColor: Theme.categoryColor(hasToast ? toast.category : "system")

    implicitWidth: 0
    implicitHeight: 0
    visible: hasToast
    Accessible.role: Accessible.Pane
    Accessible.name: I18n.t("common.labels.notifications")

    Rectangle {
        id: toastCard

        anchors.top: parent.top
        anchors.topMargin: host.topOffset
        anchors.right: parent.right
        anchors.rightMargin: host.edgeMargin
        width: Math.min(380, host.width > 0 ? host.width - host.edgeMargin * 2 : 380)
        height: toastLayout.implicitHeight + Theme.sp3 * 2
        radius: Theme.radius
        color: Theme.surfaceOverlay
        border.width: 1
        border.color: host.categoryColor
        opacity: host.hasToast ? 1 : 0
        scale: host.hasToast ? 1 : 0.97

        Behavior on opacity {
            NumberAnimation { duration: Theme.duration(Theme.motionFast) }
        }
        Behavior on scale {
            NumberAnimation { duration: Theme.duration(Theme.motionNormal); easing.type: Theme.animEasing }
        }

        Accessible.role: Accessible.AlertMessage
        Accessible.name: host.resolvedTitle
        Accessible.description: host.resolvedDescription
        Accessible.ignored: !host.hasToast

        RowLayout {
            id: toastLayout

            anchors.fill: parent
            anchors.margins: Theme.sp3
            spacing: Theme.sp3

            Rectangle {
                implicitWidth: 34
                implicitHeight: 34
                radius: width / 2
                color: Qt.rgba(host.categoryColor.r, host.categoryColor.g, host.categoryColor.b, 0.18)

                HarborIcon {
                    anchors.centerIn: parent
                    name: Theme.categoryIconName(host.hasToast ? host.toast.category : "system")
                    color: host.categoryColor
                    implicitWidth: 18
                    implicitHeight: 18
                }
            }

            ColumnLayout {
                spacing: 2
                Layout.fillWidth: true

                Text {
                    text: host.resolvedTitle
                    visible: text.length > 0
                    color: Theme.textPrimary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontBody
                    font.weight: Font.DemiBold
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }

                Text {
                    text: host.resolvedDescription
                    visible: text.length > 0
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontSmall
                    lineHeight: Theme.lineHeightSmall
                    lineHeightMode: Text.ProportionalHeight
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }
            }

            HarborIconButton {
                id: dismissButton

                Layout.alignment: Qt.AlignTop
                buttonSize: 30
                iconName: "close"
                iconColor: dismissButton.hovered ? Theme.textPrimary : Theme.iconSecondary
                accessibleName: I18n.t("common.actions.dismiss")
                Accessible.description: I18n.t("common.actions.dismiss") + " — " + host.resolvedTitle
                onClicked: MockController.dismissActiveToast()
            }
        }
    }
}
