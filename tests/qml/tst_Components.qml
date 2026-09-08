import QtQuick
import QtTest
import Harbor 2.0

// Shared-component contracts: geometry guarantees plus the keyboard
// activation story every interactive control is required to honor. Key
// events go through the KeyTest injector because Qt 6.11's TestCase can
// only send keys to its own window.
TestCase {
    id: testCase
    name: "Components"
    width: 640
    height: 480
    visible: true

    Component {
        id: iconButtonComponent
        HarborIconButton {
            iconName: "close"
            accessibleName: "Close"
            buttonSize: 28
        }
    }

    Component {
        id: navIconComponent
        HarborNavIcon { name: "chat" }
    }

    // Navigation glyphs share one square box: states may move color and
    // opacity, never geometry, so artwork keeps its aspect ratio and
    // layouts never jump between normal/hover/active.
    // Theme tokens follow personalization live: presets and custom colors
    // recolor the accent family, shape/density switch geometry tokens,
    // glass scales surface alphas, and "system" resolves via the OS scheme.
    function test_themeTokensFollowPersonalization() {
        try {
            AppState.accentColor = "violet"
            compare(String(Theme.accentBase), "#845ef7")
            var violetAccent = Theme.accent

            AppState.accentColor = "#ff0000"
            verify(Theme.accent.r > Theme.accent.b, "custom red stays red")

            AppState.accentColor = "not-a-color"
            compare(String(Theme.accentBase), "#3aa9dc")

            AppState.accentColor = "violet"
            compare(Theme.accent, violetAccent)

            AppState.cornerRadius = "medium"
            verify(Theme.radius < 14 && Theme.radiusSmall < 8)
            AppState.cornerRadius = "soft"
            compare(Theme.radius, 14)

            AppState.density = "compact"
            compare(Theme.hitTarget, 36)
            AppState.density = "comfortable"
            compare(Theme.hitTarget, 44)

            AppState.glassIntensity = 0.5
            verify(Theme.surface.a > 0 && Theme.surface.a < 0.35)
            AppState.glassIntensity = 1.0

            AppState.appearanceMode = "system"
            Theme.systemDark = true
            compare(Theme.mode, "dark")
            Theme.systemDark = false
            compare(Theme.mode, "light")
        } finally {
            MockController.resetSession()
            Theme.systemDark = false
        }
    }

    function test_navIconKeepsSquareGeometry() {
        var icon = createTemporaryObject(navIconComponent, testCase)
        verify(icon !== null)
        compare(icon.width, icon.height)
        compare(icon.width, 22)
        icon.iconSize = 30
        compare(icon.width, 30)
        compare(icon.height, 30)
        icon.active = true
        icon.hovered = true
        compare(icon.width, icon.height)
        compare(icon.width, 30)
    }

    Component {
        id: toggleComponent
        HarborToggle { text: "Background" }
    }

    Component {
        id: sliderComponent
        HarborSlider {
            label: "Volume"
            from: 0
            to: 1
            stepSize: 0.2
            value: 0.4
        }
    }

    Component {
        id: chipComponent
        HarborChoiceChip { text: "Games" }
    }

    Component {
        id: graphComponent
        HarborGraph {
            accessibleName: "Metric chart"
            series: [
                { label: "Download", values: [10, 40, 25, 60], color: Theme.chartSeries1 },
                { label: "Upload", values: [5, 20, 30, 45], color: Theme.chartSeries2, dashed: true }
            ]
        }
    }

    Component {
        id: localizedSelectComponent
        HarborSelect {
            model: MockController.audioInputOptions
            valueRole: "id"
            translationKeyRole: "labelKey"
            translationParamsRole: "labelParams"
            currentIndex: 0
        }
    }

    function test_iconButtonHasMinimumTarget() {
        var button = createTemporaryObject(iconButtonComponent, testCase)
        verify(button !== null)
        verify(button.width >= Theme.hitTarget)
        verify(button.height >= Theme.hitTarget)
    }

    function test_iconButtonActivatesWithKeyboard() {
        var button = createTemporaryObject(iconButtonComponent, testCase)
        verify(button !== null)
        var clicks = 0
        button.clicked.connect(function() { clicks++ })
        button.forceActiveFocus()
        KeyTest.click(button, Qt.Key_Space)
        compare(clicks, 1)
        button.forceActiveFocus()
        KeyTest.click(button, Qt.Key_Return)
        compare(clicks, 2)
    }

    function test_toggleFlipsWithKeyboard() {
        var toggle = createTemporaryObject(toggleComponent, testCase)
        verify(toggle !== null)
        toggle.forceActiveFocus()
        compare(toggle.checked, false)
        KeyTest.click(toggle, Qt.Key_Space)
        tryCompare(toggle, "checked", true)
        KeyTest.click(toggle, Qt.Key_Space)
        tryCompare(toggle, "checked", false)
    }

    function test_sliderMovesWithArrowKeys() {
        var slider = createTemporaryObject(sliderComponent, testCase)
        verify(slider !== null)
        slider.forceActiveFocus()
        KeyTest.click(slider, Qt.Key_Right)
        tryCompare(slider, "value", 0.6)
        KeyTest.click(slider, Qt.Key_Left)
        tryCompare(slider, "value", 0.4)
    }

    function test_choiceChipActivatesWithKeyboard() {
        var chip = createTemporaryObject(chipComponent, testCase)
        verify(chip !== null)
        var clicks = 0
        chip.clicked.connect(function() { clicks++ })
        chip.forceActiveFocus()
        KeyTest.click(chip, Qt.Key_Space)
        tryVerify(function() { return clicks === 1 })
    }

    function test_graphSelectionFollowsArrowKeys() {
        var graph = createTemporaryObject(graphComponent, testCase)
        verify(graph !== null)
        graph.forceActiveFocus()
        compare(graph.selectedIndex, -1)
        KeyTest.click(graph, Qt.Key_Right)
        tryCompare(graph, "selectedIndex", 0)
        KeyTest.click(graph, Qt.Key_Right)
        tryCompare(graph, "selectedIndex", 1)
        KeyTest.click(graph, Qt.Key_Left)
        tryCompare(graph, "selectedIndex", 0)
        KeyTest.click(graph, Qt.Key_End)
        tryCompare(graph, "selectedIndex", 3)
    }

    function test_selectTranslatesDescriptorWithoutChangingValue() {
        var select = createTemporaryObject(localizedSelectComponent, testCase)
        verify(select !== null)
        compare(select.currentValue, "default-microphone")
        AppState.locale = "en"
        tryCompare(I18n, "locale", "en")
        compare(select.optionText(select.currentIndex), "Default Microphone")
        AppState.locale = "pt-BR"
        tryCompare(I18n, "locale", "pt-BR")
        compare(select.optionText(select.currentIndex), "Microfone padrão")
        compare(select.currentValue, "default-microphone")
        AppState.locale = "en"
    }
}
