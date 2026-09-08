// Probes pairing-sheet button delivery with synthetic mouse clicks.
import QtQuick
import QtQuick.Controls
import QtTest

TestCase {
    name: "SheetClicks"

    ApplicationWindow {
        id: win
        visible: true
        width: 412
        height: 900
    }

    function createSheet() {
        var component = Qt.createComponent("../qml/components/MobilePairingSheet.qml")
        compare(component.status, Component.Ready, component.errorString())
        var sheet = component.createObject(win.contentItem, {
            "anchors.fill": undefined,
            "width": 412, "height": 700,
            "serverConfigured": true
        })
        verify(sheet !== null)
        return sheet
    }

    function findIn(item, predicate) {
        if (predicate(item))
            return item
        for (var i = 0; i < item.children.length; i++) {
            var found = findIn(item.children[i], predicate)
            if (found)
                return found
        }
        return null
    }

    function test_connect_emits_harbor_id() {
        var sheet = createSheet()
        var fired = []
        sheet.connectWithId.connect(harborId => fired.push(harborId))
        var field = findIn(sheet, c => c.placeholderText === "harbor-xxxxxxxx")
        verify(field, "Harbor ID field exists")
        field.text = "harbor-d31846b8"
        var connect = findIn(sheet, c => c.text === "Connect" && c.enabled === true)
        verify(connect, "Connect button exists and is enabled")
        mouseClick(connect, connect.width / 2, connect.height / 2)
        compare(fired, ["harbor-d31846b8"])
        sheet.destroy()
    }

    function test_copy_emits() {
        var sheet = createSheet()
        sheet.ownHarborId = "harbor-d31846b8"
        var fired = 0
        sheet.copyId.connect(() => fired++)
        var copy = findIn(sheet, c => c.text === "Copy Harbor ID" && c.enabled === true)
        verify(copy, "Copy button exists and is enabled")
        mouseClick(copy, copy.width / 2, copy.height / 2)
        compare(fired, 1)
        sheet.destroy()
    }

    function test_cancel_emits() {
        var sheet = createSheet()
        var fired = 0
        sheet.cancelFlow.connect(() => fired++)
        var cancel = findIn(sheet, c => c.text === "Cancel" && c.enabled === true)
        verify(cancel, "Cancel button exists and is enabled")
        mouseClick(cancel, cancel.width / 2, cancel.height / 2)
        compare(fired, 1)
        sheet.destroy()
    }

}
