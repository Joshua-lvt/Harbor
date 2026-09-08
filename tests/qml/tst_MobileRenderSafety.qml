// Regression tests for the Android scene-graph artifact: controls stay in
// explicit, bounded mobile surfaces and advanced transforms can be disabled
// independently of the production renderer.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtTest

TestCase {
    name: "MobileRenderSafety"

    ApplicationWindow {
        id: window
        width: 360
        height: 720
        visible: true
    }

    function createButton(properties) {
        var component = Qt.createComponent(
            "../../Harbor-Mobile/qml/components/MobileButton.qml")
        compare(component.status, Component.Ready, component.errorString())
        var control = component.createObject(window, properties)
        verify(control !== null)
        return control
    }

    function createPresenceBar(width) {
        var component = Qt.createComponent(
            "../../Harbor-Mobile/qml/components/MobilePresenceBar.qml")
        compare(component.status, Component.Ready, component.errorString())
        var bar = component.createObject(window.contentItem, {
            "width": width
        })
        verify(bar !== null)
        bar.partnerName = "Taylor"
        bar.partnerState = "online"
        return bar
    }

    function test_button_has_bounded_flat_surface() {
        var button = createButton({ "text": "Pair", "primary": true })
        compare(button.implicitHeight, 52)
        compare(button.background.radius, 16)
        verify(button.background.width > 0)
        verify(button.background.height > 0)
        compare(button.background.color, "#4ade80")
        compare(button.contentItem.text, "Pair")
        button.destroy()
    }

    function test_button_click_reaches_signal() {
        var button = createButton({ "text": "Pair", "primary": true })
        var clicks = 0
        button.clicked.connect(() => clicks++)
        mouseClick(button, button.width / 2, button.height / 2)
        compare(clicks, 1)
        button.destroy()
    }

    function test_disabled_button_does_not_emit() {
        var button = createButton({ "enabled": false })
        var clicks = 0
        button.clicked.connect(() => clicks++)
        mouseClick(button, button.width / 2, button.height / 2)
        compare(clicks, 0)
        button.destroy()
    }

    function test_advanced_transform_has_diagnostic_off_switch() {
        var animated = createButton({ "advancedEffects": true })
        mousePress(animated, animated.width / 2, animated.height / 2)
        tryCompare(animated, "pressed", true)
        tryCompare(animated, "scale", 0.98)
        mouseRelease(animated, animated.width / 2, animated.height / 2)
        animated.destroy()

        var flat = createButton({ "advancedEffects": false })
        mousePress(flat, flat.width / 2, flat.height / 2)
        tryCompare(flat, "pressed", true)
        tryCompare(flat, "scale", 1.0)
        mouseRelease(flat, flat.width / 2, flat.height / 2)
        flat.destroy()
    }

    function test_presence_bar_tracks_available_width() {
        var widths = [320, 412, 640]
        for (var i = 0; i < widths.length; ++i) {
            var bar = createPresenceBar(widths[i])
            compare(bar.width, widths[i])
            compare(bar.height, 72)
            var layout = null
            for (var j = 0; j < bar.children.length; ++j) {
                if (bar.children[j] instanceof RowLayout) {
                    layout = bar.children[j]
                    break
                }
            }
            verify(layout, "responsive layout exists")
            verify(layout.x >= 0)
            verify(layout.width <= bar.width)
            verify(layout.x + layout.width <= bar.width + 0.5)
            bar.destroy()
    }

    function test_mobile_visual_effects_policy_is_singleton_and_mobile_safe() {
        var qml = new XMLHttpRequest()
        qml.open("GET", "../../Harbor-Mobile/qml/HarborVisualEffects.qml", false)
        qml.send(null)
        compare(qml.status, 200)
        verify(qml.responseText.indexOf("pragma Singleton") >= 0)
        verify(qml.responseText.indexOf("Qt.platform.os === \"android\"") >= 0)
        verify(qml.responseText.indexOf("harborRender.effectsDisabled") >= 0)
        verify(qml.responseText.indexOf("!isAndroid && !diagnosticsActive") >= 0)
    }
}
}
