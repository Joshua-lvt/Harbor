import QtQuick
import QtTest
import Harbor 2.0

// The mandatory-update overlay only ever appears for a discovered update
// (or a failure with a retry path): idle stays invisible, and the error
// card offers exactly one action — trying the channel again.
TestCase {
    id: testCase
    name: "HarborUpdateOverlay"
    width: 1280
    height: 960
    visible: true

    QtObject {
        id: stubUpdater

        property string status: "idle"
        property string availableVersion: ""
        property real progress: 0
        property string errorKey: ""
        property bool updateRequired: false
        property bool waitingForCall: false
    }

    HarborUpdateOverlay {
        id: overlay

        width: 1280
        height: 960
        updater: stubUpdater
    }

    function init() {
        stubUpdater.status = "idle"
        stubUpdater.availableVersion = ""
        stubUpdater.progress = 0
        stubUpdater.errorKey = ""
        stubUpdater.updateRequired = false
        stubUpdater.waitingForCall = false
        AppState.resetSession()
    }

    function cleanup() {
        AppState.resetSession()
    }

    function _texts(item, out) {
        out = out || []
        if (item && item.text !== undefined && String(item.text).length > 0)
            out.push(String(item.text))
        var children = item ? item.children : []
        for (var i = 0; i < children.length; ++i)
            _texts(children[i], out)
        return out
    }

    function _hasText(needle) {
        var texts = _texts(overlay)
        for (var i = 0; i < texts.length; ++i) {
            if (texts[i].indexOf(needle) >= 0)
                return true
        }
        return false
    }

    function test_idleStaysInvisible() {
        verify(!overlay.required)
        // The fade starts at full opacity by Item default and settles out.
        tryCompare(overlay, "visible", false)
    }

    function test_discoveredUpdateBlocks() {
        stubUpdater.status = "available"
        stubUpdater.availableVersion = "2.2.0"
        stubUpdater.updateRequired = true
        verify(overlay.required)
        tryCompare(overlay, "visible", true)
        verify(_hasText("2.2.0"))
        verify(_hasText(I18n.t("update.mandatoryNote")))
    }

    function test_downloadingShowsProgress() {
        stubUpdater.status = "downloading"
        stubUpdater.availableVersion = "2.2.0"
        stubUpdater.updateRequired = true
        stubUpdater.progress = 0.5
        tryCompare(overlay, "visible", true)
        verify(_hasText("50%"))
    }

    function test_waitingForCallExplainsItself() {
        stubUpdater.status = "ready"
        stubUpdater.updateRequired = true
        stubUpdater.waitingForCall = true
        tryCompare(overlay, "visible", true)
        verify(_hasText(I18n.t("update.waitingForCall")))
    }

    function test_errorNeverBricksTheApp() {
        // A failed check is not a discovery: the app stays usable, the
        // Settings card carries the error with the retry action, and the
        // updater looks again on its own cadence.
        stubUpdater.status = "error"
        stubUpdater.errorKey = "update.error.network"
        verify(!overlay.required)
        tryCompare(overlay, "visible", false)
    }
}
