import QtQuick
import QtQuick.Window
import QtQuick.Controls
import QtQuick.Layouts

Popup {
    id: popup

    default property alias modalContent: customContent.data
    property string title: ""
    property string description: ""
    property string primaryText: I18n.t("common.actions.done")
    property string secondaryText: I18n.t("common.actions.cancel")
    property bool showPrimaryButton: true
    property bool showSecondaryButton: true
    property bool closeOnAccept: true
    property int preferredWidth: 460
    property Item initialFocusItem: null
    property Item lastFocusItem: null
    property Item launcherItem: null
    property string _outcome: "none"

    signal accepted()
    signal rejected()

    parent: Overlay.overlay
    anchors.centerIn: parent
    width: Math.min(preferredWidth, parent ? parent.width - Theme.sp5 * 2 : preferredWidth)
    modal: true
    focus: true
    padding: Theme.sp5
    closePolicy: Popup.CloseOnEscape

    function _isUsable(item) {
        return item && item.visible && item.enabled
    }

    function _firstFocusTarget() {
        if (_isUsable(initialFocusItem)) return initialFocusItem
        if (_isUsable(closeButton)) return closeButton
        if (_isUsable(secondaryButton)) return secondaryButton
        if (_isUsable(primaryButton)) return primaryButton
        return null
    }

    function _lastFocusTarget() {
        if (_isUsable(lastFocusItem)) return lastFocusItem
        if (_isUsable(primaryButton)) return primaryButton
        if (_isUsable(secondaryButton)) return secondaryButton
        if (_isUsable(closeButton)) return closeButton
        return _firstFocusTarget()
    }

    function _isDescendant(item, ancestor) {
        var candidate = item
        while (candidate) {
            if (candidate === ancestor) return true
            candidate = candidate.parent
        }
        return false
    }

    function _trapTab(event) {
        // Attached Window lookup resolves at runtime; qmllint cannot see it.
        var window = modalLayout.Window.window // qmllint disable missing-property
        var current = window ? window.activeFocusItem : null
        var forward = event.key !== Qt.Key_Backtab
            && !(event.modifiers & Qt.ShiftModifier)
        if (!current || !_isDescendant(current, modalLayout)) {
            var fallback = forward ? _firstFocusTarget() : _lastFocusTarget()
            if (fallback) fallback.forceActiveFocus(Qt.TabFocusReason)
            event.accepted = true
            return
        }

        var next = current.nextItemInFocusChain(forward)
        if (!next || !_isDescendant(next, modalLayout)) {
            var boundary = forward ? _firstFocusTarget() : _lastFocusTarget()
            if (boundary) boundary.forceActiveFocus(Qt.TabFocusReason)
            event.accepted = true
        }
    }

    function acceptModal() {
        if (_outcome !== "none") return
        _outcome = "accepted"
        accepted()
        if (closeOnAccept) close()
    }

    function rejectModal() {
        if (_outcome !== "none") return
        _outcome = "rejected"
        rejected()
        close()
    }

    onOpened: {
        _outcome = "none"
        Qt.callLater(function() {
            var target = _firstFocusTarget()
            if (target) target.forceActiveFocus(Qt.TabFocusReason)
        })
    }
    onAboutToHide: {
        if (_outcome === "none") {
            _outcome = "rejected"
            rejected()
        }
    }
    onClosed: {
        if (_isUsable(launcherItem))
            launcherItem.forceActiveFocus(Qt.TabFocusReason)
    }

    enter: Transition {
        NumberAnimation { property: "opacity"; from: 0; to: 1; duration: Theme.duration(Theme.motionNormal) }
        NumberAnimation { property: "scale"; from: 0.96; to: 1; duration: Theme.duration(Theme.motionNormal); easing.type: Theme.animEasing }
    }
    exit: Transition {
        NumberAnimation { property: "opacity"; to: 0; duration: Theme.duration(Theme.motionFast) }
    }

    // Main owns the only visible scrim. This transparent modal item still lets
    // Qt block pointer input outside the popup.
    Overlay.modal: Rectangle { color: "transparent" }

    background: Rectangle {
        radius: Theme.radiusLarge
        color: Theme.surfaceOverlay
        border.width: 1
        border.color: Theme.borderStrong
    }

    contentItem: ColumnLayout {
        id: modalLayout

        spacing: Theme.sp4
        Keys.priority: Keys.BeforeItem
        Keys.onPressed: event => {
            if (event.key === Qt.Key_Tab || event.key === Qt.Key_Backtab) {
                popup._trapTab(event)
            }
        }

        Accessible.role: Accessible.Dialog
        Accessible.name: popup.title
        Accessible.description: popup.description

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.sp3

            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.sp1

                Text {
                    visible: popup.title.length > 0
                    text: popup.title
                    color: Theme.textPrimary
                    font.family: Theme.fontFamilyDisplay
                    font.pixelSize: Theme.fontTitle
                    font.weight: Font.DemiBold
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }

                Text {
                    visible: popup.description.length > 0
                    text: popup.description
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontBody
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }
            }

            HarborIconButton {
                id: closeButton

                iconName: "close"
                accessibleName: I18n.t("a11y.closeDialog")
                toolTip: I18n.t("common.actions.close")
                Layout.alignment: Qt.AlignTop
                onClicked: popup.rejectModal()
            }
        }

        Column {
            id: customContent

            spacing: Theme.sp3
            Layout.fillWidth: true
        }

        RowLayout {
            visible: popup.showPrimaryButton || popup.showSecondaryButton
            spacing: Theme.sp2
            Layout.fillWidth: true

            Item { Layout.fillWidth: true }

            HarborButton {
                id: secondaryButton

                visible: popup.showSecondaryButton
                text: popup.secondaryText
                variant: "secondary"
                onClicked: popup.rejectModal()
            }

            HarborButton {
                id: primaryButton

                visible: popup.showPrimaryButton
                text: popup.primaryText
                onClicked: popup.acceptModal()
            }
        }
    }
}
