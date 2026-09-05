// Settings never exposes the server: the app ships pre-pointed at the
// Harbor network, and the product UI shows no addresses or fingerprints.
import QtQuick
import QtQuick.Controls
import QtTest

TestCase {
    name: "MobileSettingsNoServer"

    ApplicationWindow {
        id: win
        visible: true
        width: 412
        height: 900
    }

    function createSettings() {
        var component = Qt.createComponent("../qml/views/MobileSettingsView.qml")
        compare(component.status, Component.Ready, component.errorString())
        var view = component.createObject(win.contentItem, {
            "width": 412, "height": 700
        })
        verify(view !== null)
        return view
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

    function test_no_server_signal_or_fields() {
        var view = createSettings()
        verify(view.saveServer === undefined, "saveServer must not exist")
        verify(view.serverAddress === undefined, "serverAddress must not exist")
        verify(findIn(view, function (c) { return c.text === "Save server" }) === null)
        verify(findIn(view, function (c) {
            return c.placeholderText !== undefined
                && String(c.placeholderText).indexOf("fingerprint") >= 0
        }) === null)
        view.destroy()
    }

    function test_host_prepoints_default_network() {
        var qml = new XMLHttpRequest()
        qml.open("GET", "../qml/Host/HarborMobileHost.qml", false)
        qml.send(null)
        compare(qml.status, 200)
        verify(qml.responseText.indexOf("100.114.220.46:9091") >= 0,
               "host must pre-point the Harbor network")
        verify(qml.responseText.indexOf("b9846aed2e97bd741ae5a2a3de9ab37c1831d2372ca67f26f538bd279dd7271f") >= 0,
               "host must carry the pinned fingerprint")
        verify(qml.responseText.indexOf("applyServerConfig") < 0,
               "manual server entry must be gone")
    }
}
