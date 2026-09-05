// The appearance chips must emit their intent on click: ocean variant,
// accent preset, and the mode/corner/density buttons.
import QtQuick
import QtQuick.Controls
import QtTest

TestCase {
    name: "MobileSettingsChips"

    ApplicationWindow {
        id: win
        visible: true
        width: 412
        // Tall enough that the ocean chips sit inside the window: clicks
        // outside the window rect are dropped, which would fake a failure.
        height: 2000
    }

    function createSettings() {
        var themeComp = Qt.createComponent("../qml/components/MobileTheme.qml")
        compare(themeComp.status, Component.Ready, themeComp.errorString())
        var theme = themeComp.createObject(null)
        verify(theme !== null)
        var comp = Qt.createComponent("../qml/views/MobileSettingsView.qml")
        compare(comp.status, Component.Ready, comp.errorString())
        var view = comp.createObject(win.contentItem, {
            "width": 412, "height": 1900,
            "theme": theme, "oceanVariant": "lagoon",
            "accentColor": "ocean", "appearanceMode": "dark"
        })
        verify(view !== null)
        // Let the layout polish: geometry queried before the first polish
        // pass is stale and clicks map to the wrong point.
        wait(400)
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

    function chipWithLabel(view, labelText) {
        return findIn(view, function (c) {
            if (c.objectName !== "" || c.children === undefined)
                return false
            for (var i = 0; i < c.children.length; i++) {
                var child = c.children[i]
                if (child.text !== undefined && String(child.text) === labelText)
                    return true
            }
            return false
        })
    }

    function test_ocean_chip_emits_variant() {
        var view = createSettings()
        var fired = []
        view.setOceanVariant.connect(function (v) { fired.push(v) })
        var ember = chipWithLabel(view, "Ember")
        verify(ember, "Ember chip exists")
        mouseClick(ember, ember.width / 2, ember.height / 2)
        compare(fired, ["ember"])
        view.destroy()
    }

    function test_accent_swatch_emits_preset() {
        var view = createSettings()
        var fired = []
        view.setAccentColor.connect(function (v) { fired.push(v) })
        var violet = findIn(view, function (c) {
            return c.color !== undefined
                && String(c.color).toLowerCase() === "#845ef7"
                && c.width === 44
        })
        verify(violet, "violet swatch exists")
        mouseClick(violet, violet.width / 2, violet.height / 2)
        compare(fired, ["violet"])
        view.destroy()
    }

    function test_mode_button_emits() {
        var view = createSettings()
        var fired = []
        view.setAppearanceMode.connect(function (v) { fired.push(v) })
        var light = findIn(view, function (c) { return c.text === "Light" })
        verify(light, "Light button exists")
        mouseClick(light, light.width / 2, light.height / 2)
        compare(fired, ["light"])
        view.destroy()
    }
}
