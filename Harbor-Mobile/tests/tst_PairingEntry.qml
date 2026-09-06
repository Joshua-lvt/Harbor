// Pairing entry + code-copy contract for the mobile shell.
// The pairing sheet must open from Home and from Settings (regression: the
// sheet silently never opened on device), and the copy button must emit the
// exact shown code (regression: stale/wrong value on paste).
import QtQuick
import QtQuick.Controls
import QtTest

TestCase {
    name: "PairingEntry"

    ApplicationWindow {
        id: win
        visible: true
        width: 412
        height: 900
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

    function createShell() {
        var component = Qt.createComponent("../qml/MobileShell.qml")
        compare(component.status, Component.Ready, component.errorString())
        var shell = component.createObject(win.contentItem, {})
        verify(shell !== null)
        return shell
    }

    function createSheet(props) {
        var component = Qt.createComponent("../qml/components/MobilePairingSheet.qml")
        compare(component.status, Component.Ready, component.errorString())
        var base = {"width": 412, "height": 700, "serverConfigured": true}
        for (var key in props)
            base[key] = props[key]
        var sheet = component.createObject(win.contentItem, base)
        verify(sheet !== null)
        return sheet
    }

    function test_home_button_opens_sheet() {
        var shell = createShell()
        var fired = 0
        // The host owns this half in production; the shell only emits.
        shell.openPairing.connect(() => { fired++; shell.pairingVisible = true })
        var pair = findIn(shell, c => c.text === "Pair with partner" && c.visible && c.enabled)
        verify(pair, "home pairing button exists and is enabled")
        mouseClick(pair, pair.width / 2, pair.height / 2)
        compare(fired, 1)
        tryCompare(shell, "pairingVisible", true)
        verify(pair.visible)
        shell.destroy()
    }

    function test_settings_button_opens_sheet() {
        var shell = createShell()
        shell.openPairing.connect(() => shell.pairingVisible = true)
        var tile = findIn(shell, c => c.text === "Settings")
        verify(tile, "settings nav tile exists")
        mouseClick(tile, tile.width / 2, tile.height / 2)
        var pair = findIn(shell, c => c.text === "Pair with partner" && c.visible && c.enabled)
        verify(pair, "settings pairing button exists and is enabled")
        mouseClick(pair, pair.width / 2, pair.height / 2)
        tryCompare(shell, "pairingVisible", true)
        shell.destroy()
    }

    function test_copy_emits_exact_code() {
        var sheet = createSheet({"code": "482731"})
        var fired = []
        sheet.copyCode.connect(code => fired.push(code))
        var copy = findIn(sheet, c => c.text === "Copy code" && c.visible)
        verify(copy, "copy-code button exists")
        mouseClick(copy, copy.width / 2, copy.height / 2)
        compare(fired, ["482731"])
        var done = findIn(sheet, c => c.text === "Copied" && c.visible)
        verify(done, "copy feedback shows")
        sheet.destroy()
    }

    function test_gate_blocks_flow_and_offers_install() {
        var sheet = createSheet({"tailscaleReady": false})
        var fired = 0
        sheet.installTailscale.connect(() => fired++)
        var install = findIn(sheet, c => c.text === "Install Tailscale" && c.visible)
        verify(install, "install prompt exists while gated")
        var show = findIn(sheet, c => c.text === "Show my code")
        verify(show && !show.enabled, "host flow disabled while gated")
        var enter = findIn(sheet, c => c.text === "Enter code")
        verify(enter && !enter.enabled, "peer flow disabled while gated")
        mouseClick(install, install.width / 2, install.height / 2)
        compare(fired, 1)
        sheet.destroy()
    }
}
