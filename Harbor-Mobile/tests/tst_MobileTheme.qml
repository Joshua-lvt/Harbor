// The mobile theme mirrors the desktop tokens: same ocean water, same
// accent presets, same shape/density steps — as flat solid surfaces.
import QtQuick
import QtTest

TestCase {
    name: "MobileTheme"

    function createTheme() {
        var component = Qt.createComponent("../qml/components/MobileTheme.qml")
        compare(component.status, Component.Ready, component.errorString())
        var theme = component.createObject(null)
        verify(theme !== null)
        return theme
    }

    function test_defaults_match_desktop() {
        var theme = createTheme()
        compare(theme.appearanceMode, "dark")
        compare(theme.accentColor, "ocean")
        compare(theme.oceanVariant, "lagoon")
        compare(theme.cornerRadius, "soft")
        compare(theme.density, "comfortable")
        // Lagoon dark water, like the desktop shell.
        compare(String(theme.background).toLowerCase(), "#062544")
        compare(String(theme.accentBase).toLowerCase(), "#3aa9dc")
        compare(theme.radius, 14)
        compare(theme.buttonHeight, 52)
        theme.destroy()
    }

    function test_ocean_variants_recolor_water() {
        var theme = createTheme()
        var lagoon = String(theme.bgTop)
        theme.oceanVariant = "ember"
        compare(String(theme.bgTop).toLowerCase(), String(theme.oceanTop("ember")).toLowerCase())
        verify(String(theme.bgTop) !== lagoon, "ember must leave lagoon blue")
        verify(theme.bgTop.r > theme.bgTop.b, "ember must lean warm")
        theme.oceanVariant = "onyx"
        verify(theme.bgTop.r < 0.08 && theme.bgTop.g < 0.08, "onyx must be near-black")
        // Unknown names fall back to lagoon, never to an empty color.
        theme.oceanVariant = "nonsense"
        compare(String(theme.bgTop), lagoon)
        theme.destroy()
    }

    function test_light_mode_uses_light_water() {
        var theme = createTheme()
        theme.appearanceMode = "light"
        compare(String(theme.background).toLowerCase(), "#d8effa")
        theme.appearanceMode = "system"
        theme.systemDark = true
        compare(String(theme.background).toLowerCase(), "#062544")
        theme.systemDark = false
        compare(String(theme.background).toLowerCase(), "#d8effa")
        theme.destroy()
    }

    function test_accent_presets_and_custom_hex() {
        var theme = createTheme()
        compare(theme.accentPresetList.length, 7)
        theme.accentColor = "violet"
        compare(String(theme.accentBase).toLowerCase(), "#845ef7")
        theme.accentColor = "#123ABC"
        compare(String(theme.accentBase).toLowerCase(), "#123abc")
        theme.accentColor = "nonsense"
        compare(String(theme.accentBase).toLowerCase(), "#3aa9dc")
        theme.destroy()
    }

    function test_shape_and_density_steps() {
        var theme = createTheme()
        theme.cornerRadius = "medium"
        compare(theme.radius, 10)
        compare(theme.radiusSmall, 6)
        compare(theme.radiusLarge, 16)
        theme.density = "compact"
        compare(theme.buttonHeight, 46)
        compare(theme.navHeight, 52)
        theme.destroy()
    }

    function test_state_mapping() {
        var theme = createTheme()
        compare(theme.stateText("online"), "Online")
        compare(theme.stateText("idle"), "Away")
        compare(theme.stateText("nonsense"), "Offline")
        verify(theme.stateColor("online") !== undefined)
        theme.destroy()
    }

    function test_reduced_motion_zeroes_durations() {
        var theme = createTheme()
        verify(theme.duration(220) > 0)
        theme.reducedMotion = true
        compare(theme.duration(220), 0)
        theme.destroy()
    }
}
