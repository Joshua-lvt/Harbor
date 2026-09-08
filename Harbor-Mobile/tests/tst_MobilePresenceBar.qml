// Verifies the persistent presence bar mirrors the committed aggregate
// and nothing else: state text, dot color, blocked copy.
import QtQuick
import QtTest

TestCase {
    name: "MobilePresenceBar"

    // The bar under test, loaded from the mobile shell sources.
    function createBar() {
        var component = Qt.createComponent("../qml/components/MobilePresenceBar.qml")
        compare(component.status, Component.Ready, component.errorString())
        return component.createObject(null)
    }

    function test_online_state() {
        var bar = createBar()
        bar.partnerName = "Taylor"
        bar.partnerState = "online"
        compare(bar.stateText, "Online")
        compare(bar.stateColor, "#4ade80")
        bar.destroy()
    }

    function test_away_state() {
        var bar = createBar()
        bar.partnerState = "idle"
        compare(bar.stateText, "Away")
        compare(bar.stateColor, "#fbbf24")
        bar.destroy()
    }

    function test_offline_state() {
        var bar = createBar()
        bar.partnerState = "offline"
        compare(bar.stateText, "Offline")
        compare(bar.stateColor, "#64748b")
        bar.destroy()
    }

    function test_unknown_state_reads_offline() {
        var bar = createBar()
        bar.partnerState = "connecting"
        compare(bar.stateText, "Offline")
        compare(bar.stateColor, "#64748b")
        bar.destroy()
    }

    function test_state_members_live_on_the_public_root() {
        var bar = createBar()
        verify(bar.stateColor !== undefined)
        verify(bar.stateText !== undefined)
        bar.destroy()
    }
}
