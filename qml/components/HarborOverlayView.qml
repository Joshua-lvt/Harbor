import QtQuick

// Base for full-screen shell overlays. It focuses the page's initial control
// when the layer becomes active, traps Tab/Backtab inside itself, and leaves
// Escape to the shell coordinator — closing and focus restoration are not the
// view's to trigger.
FocusScope {
    id: root

    signal closed()

    // Views bind this to their AppState visibility flag so the trap only
    // engages while the overlay is actually presented.
    property bool overlayActive: true
    property Item initialFocusItem: null
    property Item lastFocusItem: null

    activeFocusOnTab: false
    Keys.priority: Keys.BeforeItem

    // A child Timer rather than Qt.callLater: the shell unmounts lazy layers
    // the moment their flag drops, and a deferred call on a destroyed object
    // raises a TypeError while the timer simply dies with its parent.
    Timer {
        id: initialFocusTimer

        interval: 0
        onTriggered: root._focusInitialItem()
    }

    Component.onCompleted: initialFocusTimer.start()
    onOverlayActiveChanged: {
        if (overlayActive)
            initialFocusTimer.restart()
    }

    function _usable(item) {
        return item && item.visible && item.enabled
    }

    function _isDescendant(item, ancestor) {
        var candidate = item
        while (candidate) {
            if (candidate === ancestor)
                return true
            candidate = candidate.parent
        }
        return false
    }

    function _firstTarget() {
        return _usable(initialFocusItem) ? initialFocusItem : null
    }

    function _lastTarget() {
        return _usable(lastFocusItem) ? lastFocusItem : _firstTarget()
    }

    function _focusInitialItem() {
        if (!overlayActive)
            return
        var target = _firstTarget()
        if (target)
            target.forceActiveFocus(Qt.TabFocusReason)
        else
            root.forceActiveFocus(Qt.TabFocusReason)
    }

    Keys.onPressed: event => {
        if (event.key !== Qt.Key_Tab && event.key !== Qt.Key_Backtab)
            return
        var window = root.Window.window
        var current = window ? window.activeFocusItem : null
        var forward = event.key !== Qt.Key_Backtab
            && !(event.modifiers & Qt.ShiftModifier)
        if (!current || !_isDescendant(current, root)) {
            var fallback = forward ? _firstTarget() : _lastTarget()
            if (fallback)
                fallback.forceActiveFocus(Qt.TabFocusReason)
            event.accepted = true
            return
        }
        var next = current.nextItemInFocusChain(forward)
        if (!next || !_isDescendant(next, root)) {
            var boundary = forward ? _firstTarget() : _lastTarget()
            if (boundary)
                boundary.forceActiveFocus(Qt.TabFocusReason)
            event.accepted = true
        }
    }
}
