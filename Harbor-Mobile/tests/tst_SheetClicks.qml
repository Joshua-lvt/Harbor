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

    function test_join_emits_submit() {
        var sheet = createSheet()
        var fired = []
        sheet.submitCode.connect(code => fired.push(code))
        var field = findIn(sheet, c => c.placeholderText === "6-digit code")
        verify(field, "code field exists")
        field.text = "123456"
        tryCompare(field, "text", "123456")
        var join = findIn(sheet, c => c.text === "Join" && c.enabled === true)
        verify(join, "Join button exists and is enabled")
        mouseClick(join, join.width / 2, join.height / 2)
        compare(fired, ["123456"])
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

    function test_show_code_emits_create() {
        var sheet = createSheet()
        var fired = 0
        sheet.createCode.connect(() => fired++)
        var show = findIn(sheet, c => c.text === "Show my code" && c.enabled === true)
        verify(show, "Show-my-code exists and is enabled")
        mouseClick(show, show.width / 2, show.height / 2)
        compare(fired, 1)
        sheet.destroy()
    }
}
