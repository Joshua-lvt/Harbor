// Pairing entry + Harbor-ID copy contract for the mobile shell.
// The pairing sheet must open from Home and from Settings (regression: the
// sheet silently never opened on device), and the copy button must emit the
// copy signal for exactly the shown Harbor ID (regression: stale/wrong value
// on paste).
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

    function createHost() {
        var component = Qt.createComponent("../qml/Host/HarborMobileHost.qml")
        compare(component.status, Component.Ready, component.errorString())
        var host = component.createObject(win.contentItem, {"core": null, "platform": null})
        verify(host !== null)
        return host
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

    function test_copy_emits_for_shown_harbor_id() {
        var sheet = createSheet({"ownHarborId": "harbor-d31846b8"})
        var fired = 0
        sheet.copyId.connect(() => fired++)
        var copy = findIn(sheet, c => c.text === "Copy Harbor ID" && c.visible)
        verify(copy, "copy-ID button exists")
        mouseClick(copy, copy.width / 2, copy.height / 2)
        compare(fired, 1)
        var done = findIn(sheet, c => c.text === "Harbor ID copied" && c.visible)
        verify(done, "copy feedback shows")
        sheet.destroy()
    }

    function test_only_server_readiness_gates_pairing() {
        var sheet = createSheet({"serverConfigured": false, "ownHarborId": "harbor-d31846b8"})
        var field = findIn(sheet, c => c.placeholderText === "harbor-xxxxxxxx")
        verify(field && !field.enabled, "ID entry disabled without a server")
        var join = findIn(sheet, c => c.text === "Connect")
        verify(join && !join.enabled, "connect disabled without a server")
        sheet.serverConfigured = true
        tryCompare(field, "enabled", true)
        sheet.destroy()
    }

    function test_connect_needs_a_valid_harbor_id() {
        var sheet = createSheet({"ownHarborId": "harbor-d31846b8"})
        var fired = []
        sheet.connectWithId.connect(harborId => fired.push(harborId))
        var field = findIn(sheet, c => c.placeholderText === "harbor-xxxxxxxx")
        verify(field, "ID field exists")
        field.text = "123456"
        var join = findIn(sheet, c => c.text === "Connect")
        verify(join && !join.enabled, "connect disabled for malformed IDs")
        field.text = " harbor-d31846b8"
        verify(!join.enabled, "connect disabled for leading whitespace")
        field.text = "harbor-d31846b8 "
        verify(!join.enabled, "connect disabled for trailing whitespace")
        field.text = "harbor-d31846b8"
        tryCompare(join, "enabled", true)
        mouseClick(join, join.width / 2, join.height / 2)
        compare(fired, ["harbor-d31846b8"])
        sheet.destroy()
    }

    function test_incoming_shows_requester_and_consent_buttons() {
        var sheet = createSheet({
            "phase": "INCOMING",
            "incomingHarborId": "harbor-ABCDEF12"
        })
        var requester = findIn(sheet, c => c.text === "Request from: harbor-ABCDEF12" && c.visible)
        verify(requester, "requester's full Harbor ID is visible")
        var accept = findIn(sheet, c => c.text === "Accept" && c.visible)
        var decline = findIn(sheet, c => c.text === "Decline" && c.visible)
        verify(accept, "accept consent remains visible")
        verify(decline, "decline consent remains visible")
        sheet.destroy()
    }

    function test_incoming_poll_is_suppressed_during_outgoing_flow() {
        var sheet = createSheet({"phase": "REQUESTING", "busy": true})
        var incomingPolls = 0
        sheet.pollIncoming.connect(() => incomingPolls++)
        sheet.visible = true
        wait(2200)
        compare(incomingPolls, 0)
        sheet.destroy()
    }

    function test_host_rejects_padded_invites_before_requesting() {
        var host = createHost()
        var requests = 0
        host.request = function(_type, _payload, _callback) { requests++ }
        compare(host.pairingInvite(" harbor-d31846b8"), false)
        compare(host.pairingInvite("harbor-d31846b8 "), false)
        compare(requests, 0)
        compare(host.pairingBusy, false)
        compare(host.pairingConnecting, false)
        compare(host.pairingInvite("harbor-d31846b8"), true)
        compare(requests, 1)
        host.destroy()
    }
}
