pragma Singleton
import QtQuick

// Harbor visual foundation. Existing token names remain as compatibility aliases.
QtObject {
    id: theme

    // "dark" | "light". Unknown values intentionally resolve to light.
    // "system" never lands here: AppState resolves it against the OS scheme
    // before assigning.
    property string mode: "dark"
    readonly property bool dark: mode === "dark"
    // Live OS scheme, fed by the shell. Flipping it re-resolves "system".
    property bool systemDark: false

    onSystemDarkChanged: {
        if (AppState.appearanceMode === "system")
            mode = systemDark ? "dark" : "light"
    }

    property bool higherContrast: AppState.higherContrast

    // ── Personalization inputs (durable, live) ──────────────────────────
    // Preset key or custom #RRGGBB. Unknown values fall back to ocean.
    readonly property var accentBases: ({
        "ocean": "#3AA9DC",
        "cyan": "#22B8CF",
        "aqua": "#3ED6B8",
        "blue": "#4A90E2",
        "teal": "#20C997",
        "violet": "#845EF7",
        "pink": "#E87BA4"
    })
    // Ordered swatch model for the Appearance settings. Single source of
    // truth: the view never hardcodes preset colors.
    readonly property var accentPresetList: [
        { key: "ocean", color: "#3AA9DC" },
        { key: "cyan", color: "#22B8CF" },
        { key: "aqua", color: "#3ED6B8" },
        { key: "blue", color: "#4A90E2" },
        { key: "teal", color: "#20C997" },
        { key: "violet", color: "#845EF7" },
        { key: "pink", color: "#E87BA4" }
    ]
    readonly property var oceanVariantList: [
        "lagoon", "abyss", "sunrise", "rose",
        "ember", "onyx", "forest", "dusk"
    ]
    readonly property color accentBase: {
        var key = String(AppState.accentColor || "ocean")
        if (accentBases[key] !== undefined)
            return accentBases[key]
        if (/^#[0-9a-fA-F]{6}$/.test(key))
            return key
        return accentBases["ocean"]
    }
    // Accent strength scales saturation toward gray; 1.0 is the pure base.
    readonly property real accentSaturation: Math.max(0.15, Math.min(1, AppState.accentIntensity))
    readonly property color accentTuned: {
        var base = accentBase
        if (base.hslSaturation <= 0)
            return base
        return Qt.hsla(base.hslHue,
                        Math.max(0, Math.min(1, base.hslSaturation * accentSaturation)),
                        base.hslLightness, 1)
    }
    // Glass strength multiplies surface alphas; 1.0 preserves the shipped look.
    readonly property real glassAlpha: Math.max(0.2, Math.min(1, AppState.glassIntensity))
    // String literals do not expose color channels to JS member access, so
    // the value crosses the C++ boundary once (identity lighten) and the
    // resulting real color provides defined channels below.
    function glassed(baseColor) {
        var solid = Qt.lighter(baseColor, 1.0)
        return Qt.rgba(solid.r, solid.g, solid.b, solid.a * glassAlpha)
    }

    // ── Foundation palette ────────────────────────────────────────────
    readonly property color darkCanvas:         "#04182B"
    readonly property color darkCanvasRaised:   "#07263F"
    readonly property color darkCanvasDeep:     "#0A3355"
    readonly property color lightCanvas:        "#DFF3FB"
    readonly property color lightCanvasRaised:  "#BEE7F5"
    readonly property color lightCanvasDeep:    "#8FD3EC"

    readonly property color brandCyan:          "#22B8CF"
    readonly property color brandCyanStrong:    "#0C8599"
    readonly property color brandTeal:          "#20C997"
    readonly property color brandViolet:        "#845EF7"

    // Existing brand/status names. accent and its family derive from the
    // chosen base so one choice recolors every interactive surface; status
    // colors stay fixed and exclusive to state.
    readonly property color accent: dark ? accentTuned : Qt.darker(accentTuned, 1.3)
    readonly property color accentDeep: Qt.darker(accent, 1.35)
    readonly property color accentSoft: dark ? Qt.lighter(accentTuned, 1.25)
                                            : Qt.lighter(accent, 1.3)
    readonly property color teal:        brandTeal
    readonly property color success:     dark ? "#40C057" : "#2B8A3E"
    readonly property color warning:     dark ? "#FCC419" : "#D97706"
    readonly property color danger:      dark ? "#FF6B6B" : "#D94848"
    readonly property color info:        dark ? "#4DABF7" : "#1971C2"
    readonly property color online:      dark ? "#40C057" : "#2F9E44"
    readonly property color idle:        dark ? "#FCC419" : "#D97706"
    readonly property color offline:     dark ? "#7894AD" : "#607D94"

    // ── Semantic backgrounds and surfaces ────────────────────────────
    // Ocean variations. lagoon is the shipped water-blue look; abyss drops
    // to near-black teal depths; sunrise breaks into a violet dawn. The
    // three are deliberately far apart so the choice is unmistakable.
    readonly property var oceanStops: {
        var palettes = {
            "lagoon": {
                dark: ["#062544", "#0A3258", "#0E4069"],
                light: ["#D8EFFA", "#B9E2F4", "#8CCFEB"]
            },
            "abyss": {
                dark: ["#010D16", "#02222E", "#053743"],
                light: ["#C2DEEA", "#9CC9DC", "#77B2CC"]
            },
            "sunrise": {
                dark: ["#131A3A", "#2C2352", "#54304D"],
                light: ["#FBF0DC", "#F5E2C4", "#EDCB9E"]
            },
            "rose": {
                dark: ["#2E1226", "#471A38", "#5E2547"],
                light: ["#F9E3EC", "#F3C9DB", "#E8A9C2"]
            },
            "ember": {
                dark: ["#2B0F0A", "#4A1A10", "#662216"],
                light: ["#F9E4D8", "#F0C4AC", "#E5A180"]
            },
            "onyx": {
                dark: ["#0A0A0C", "#161618", "#202024"],
                light: ["#E2E4E8", "#C9CCD3", "#AEB3BC"]
            },
            "forest": {
                dark: ["#0A2018", "#0F3826", "#155239"],
                light: ["#DCEFE2", "#BEE0C8", "#9CCBA8"]
            },
            "dusk": {
                dark: ["#1B1B2E", "#2A2A4A", "#3B3B63"],
                light: ["#E4E4F0", "#CFCFE6", "#B5B5D6"]
            }
        }
        var stops = palettes[AppState.oceanVariant] || palettes["lagoon"]
        return dark ? stops.dark : stops.light
    }
    readonly property color bgTop:       oceanStops[0]
    readonly property color bgMid:       oceanStops[1]
    readonly property color bgBottom:    oceanStops[2]
    readonly property color background:  bgTop
    readonly property color backgroundRaised: bgMid
    readonly property color backgroundAccent: bgBottom

    readonly property color surface: glassed(higherContrast
        ? (dark ? "#D9162E43" : "#F2FFFFFF")
        : (dark ? "#22FFFFFF" : "#BFFFFFFF"))
    readonly property color surfaceStrong: glassed(higherContrast
        ? (dark ? "#F21B3850" : "#FFFFFFFF")
        : (dark ? "#33FFFFFF" : "#E0FFFFFF"))
    readonly property color surfaceBorder: glassed(higherContrast
        ? (dark ? "#A8FFFFFF" : "#B80B3049")
        : (dark ? "#33FFFFFF" : "#99FFFFFF"))
    readonly property color surfaceHighlight: glassed(dark ? "#40FFFFFF" : "#FFFFFFFF")

    readonly property color surfaceSunken: glassed(dark ? "#52020F1C" : "#661B5B78")
    readonly property color surfaceOverlay: glassed(dark ? "#F20A2943" : "#F7F4FBFE")
    readonly property color surfaceScrim: dark ? "#99020B14" : "#660B3049"
    readonly property color surfaceInteractive: glassed(dark ? "#2BFFFFFF" : "#CFFFFFFF")
    readonly property color surfaceHover: glassed(dark ? "#3DFFFFFF" : "#E8FFFFFF")
    readonly property color surfacePressed: glassed(dark ? "#52FFFFFF" : "#FFFFFFFF")
    readonly property color borderSubtle: surfaceBorder
    readonly property color borderStrong: higherContrast
        ? (dark ? "#E6FFFFFF" : "#E60B3049")
        : (dark ? "#66FFFFFF" : "#7A214B63")
    readonly property color divider: dark ? "#29FFFFFF" : "#38214B63"

    // ── Text and content ──────────────────────────────────────────────
    readonly property color text:      higherContrast
        ? (dark ? "#FFFFFFFF" : "#061D2B")
        : (dark ? "#EAF6FC" : "#0B3049")
    readonly property color textDim:   higherContrast
        ? (dark ? "#D7EAF4" : "#244F68")
        : (dark ? "#9CC3D8" : "#4A7089")
    readonly property color textFaint: higherContrast
        ? (dark ? "#B8D3E1" : "#456B82")
        : (dark ? "#789CB2" : "#688CA2")
    readonly property color textPrimary: text
    readonly property color textSecondary: textDim
    readonly property color textMuted: textFaint
    readonly property color textDisabled: dark ? "#70899A" : "#728897"
    readonly property color textInverse: dark ? "#062033" : "#F4FBFE"
    readonly property color textLink: dark ? accentSoft : accentDeep
    readonly property color iconPrimary: text
    readonly property color iconSecondary: textDim
    readonly property color iconDisabled: textDisabled

    // ── Action colors (accent-driven) ───────────────────────────────
    readonly property color actionPrimary: accent
    readonly property color actionPrimaryHover: Qt.lighter(accent, 1.12)
    readonly property color actionPrimaryPressed: Qt.darker(accent, 1.12)
    readonly property color actionPrimaryText: dark ? "#04182B" : "#FFFFFF"
    readonly property color actionSecondary: surfaceStrong
    readonly property color actionSecondaryHover: surfaceHover
    readonly property color actionSecondaryPressed: surfacePressed
    readonly property color actionSecondaryText: textPrimary
    readonly property color actionDanger: dark ? "#FA5252" : "#C92A2A"
    readonly property color actionDangerHover: dark ? "#FF6B6B" : "#B42323"
    readonly property color actionDangerPressed: dark ? "#E03131" : "#9E1F1F"
    readonly property color actionDangerText: "#FFFFFF"
    readonly property color actionDisabled: dark ? "#24445A" : "#AEC4D0"
    readonly property color actionDisabledText: textDisabled

    // ── Focus and selection ───────────────────────────────────────────
    readonly property color focus: higherContrast
        ? (dark ? "#FFFFFF" : "#003D52")
        : accent
    readonly property color focusRing: focus
    readonly property color selection: Qt.rgba(accent.r, accent.g, accent.b,
                                               dark ? 0.4 : 0.32)
    readonly property int focusWidth: higherContrast ? 3 : 2
    readonly property int focusOffset: 2

    // ── Chart colors ──────────────────────────────────────────────────
    // Fixed categorical order. Do not recycle colors for a ninth series.
    readonly property color chartSeries1: dark ? "#3987E5" : "#2A78D6"
    readonly property color chartSeries2: dark ? "#D95926" : "#EB6834"
    readonly property color chartSeries3: dark ? "#199E70" : "#109768"
    readonly property color chartSeries4: dark ? "#C98500" : "#EDA100"
    readonly property color chartSeries5: dark ? "#D55181" : "#E87BA4"
    readonly property color chartSeries6: "#008300"
    readonly property color chartSeries7: dark ? "#9085E9" : "#4A3AA7"
    readonly property color chartSeries8: dark ? "#E66767" : "#E34948"
    readonly property var chartSeries: [
        chartSeries1, chartSeries2, chartSeries3, chartSeries4,
        chartSeries5, chartSeries6, chartSeries7, chartSeries8
    ]
    readonly property color chartSurface: dark ? "#101F2B" : "#FCFCFB"
    readonly property color chartGrid: dark ? "#365064" : "#D4E1E7"
    readonly property color chartAxis: dark ? "#7894A8" : "#607D8E"
    readonly property color chartPositive: success
    readonly property color chartNegative: danger
    readonly property color chartNeutral: dark ? "#7894A8" : "#78909C"
    readonly property color chartDivergingLow: chartSeries1
    readonly property color chartDivergingMid: dark ? "#405565" : "#E2EBEF"
    readonly property color chartDivergingHigh: chartSeries8
    readonly property var chartSequential: dark
        ? ["#184F95", "#1C5CAB", "#256ABF", "#2A78D6", "#3987E5", "#6DA7EC"]
        : ["#CDE2FB", "#9EC5F4", "#86B6EF", "#5598E7", "#2A78D6", "#184F95"]

    // ── Typography ────────────────────────────────────────────────────
    // Fonts are bundled under the SIL OFL so the interface renders
    // consistently without depending on the host's installed families.
    property FontLoader uiFontLoader: FontLoader {
        source: Qt.resolvedUrl("fonts/InterVariable.ttf")
    }
    property FontLoader dataFontLoader: FontLoader {
        source: Qt.resolvedUrl("fonts/JetBrainsMono-Regular.ttf")
    }

    readonly property string fontFamily: uiFontLoader.status === FontLoader.Ready
        ? uiFontLoader.name : "Sans Serif"
    readonly property string fontFamilyDisplay: fontFamily
    readonly property string fontFamilyMonospace: dataFontLoader.status === FontLoader.Ready
        ? dataFontLoader.name : "Monospace"

    // Existing type scale names.
    readonly property int fontDisplay: 34
    readonly property int fontTitle:   24
    readonly property int fontHeading: 18
    readonly property int fontBody:    14
    readonly property int fontSmall:   12
    readonly property int fontTiny:    11

    readonly property int fontHero:    44
    readonly property int fontSubtitle: 16
    readonly property int fontLabel:   13
    readonly property int fontCaption: fontSmall

    readonly property int weightRegular: Font.Normal
    readonly property int weightMedium: Font.Medium
    readonly property int weightSemibold: Font.DemiBold
    readonly property int weightBold: Font.Bold

    // Multipliers for Text.lineHeight with Text.ProportionalHeight.
    readonly property real lineHeightDisplay: 1.15
    readonly property real lineHeightTitle: 1.22
    readonly property real lineHeightHeading: 1.28
    readonly property real lineHeightBody: 1.45
    readonly property real lineHeightSmall: 1.40
    readonly property real lineHeightTiny: 1.35

    // ── Shape, spacing, and layout ────────────────────────────────────
    // Corner rounding follows the density of personalization: soft is the
    // shipped look, medium tightens every rounded surface at once.
    readonly property bool softCorners: AppState.cornerRadius !== "medium"
    readonly property int radiusSmall:  softCorners ? 8 : 6
    readonly property int radius:       softCorners ? 14 : 10
    readonly property int radiusLarge:  softCorners ? 22 : 16
    readonly property int radiusPill:   999

    readonly property int sp1: 4
    readonly property int sp2: 8
    readonly property int sp3: 12
    readonly property int sp4: 16
    readonly property int sp5: 24
    readonly property int sp6: 32
    readonly property int sp7: 48
    readonly property int sp8: 64
    readonly property int sp9: 80

    readonly property int hitTarget: AppState.density === "compact" ? 36 : 44
    readonly property int hitTargetCompact: 36
    readonly property int hitTargetLarge: 52

    // Breakpoints are evaluated against the page content area, not the window.
    // compact < 840; medium 840–1179; wide 1180–1479; ultrawide >= 1480.
    readonly property int breakpointCompact: 840
    readonly property int breakpointMedium: 1180
    readonly property int breakpointWide: 1480
    readonly property int breakpointUltraWide: breakpointWide

    readonly property int shellTitleBarHeight: 48
    readonly property int shellSidebarWidth: 248
    readonly property int shellSidebarCompactWidth: 76
    readonly property int shellStatusBarHeight: 34
    readonly property int shellMinimumWidth: 1024
    readonly property int shellMinimumHeight: 640
    readonly property int maxPageWidth: 1240

    // ── Opacity ───────────────────────────────────────────────────────
    readonly property real opacityDisabled: higherContrast ? 0.62 : 0.44
    readonly property real opacityMuted: higherContrast ? 0.78 : 0.68
    readonly property real opacitySubtle: higherContrast ? 0.32 : 0.18
    readonly property real opacityHover: 0.10
    readonly property real opacityPressed: 0.18
    readonly property real opacityScrim: dark ? 0.68 : 0.42

    // ── Motion ────────────────────────────────────────────────────────
    // Animation strength scales every duration; reduced motion still wins
    // outright with zero. Functional state changes always complete — only
    // their travel time scales.
    readonly property real motionScale: AppState.reducedMotion
        ? 0 : Math.max(0, Math.min(1, AppState.animationIntensity))
    readonly property int animFast:   120
    readonly property int animNormal: 220
    readonly property int animSlow:   380
    readonly property int animEasing: Easing.OutCubic
    readonly property int motionInstant: 0
    readonly property int motionFast: animFast
    readonly property int motionNormal: animNormal
    readonly property int motionSlow: animSlow

    function duration(milliseconds) {
        return Math.max(0, Math.round(milliseconds * motionScale))
    }

    function withOpacity(baseColor, value) {
        const amount = Math.max(0, Math.min(1, Number(value)))
        return Qt.rgba(baseColor.r, baseColor.g, baseColor.b, amount)
    }

    function statusColor(status) {
        switch (String(status).toLowerCase()) {
        case "connected":
        case "available":
        case "active":
        case "online":
        case "success": return success
        case "connecting":
        case "reconnecting":
        case "away":
        case "idle":
        case "pending": return warning
        case "error":
        case "failed":
        case "muted":
        case "danger": return danger
        case "info": return info
        case "disabled":
        case "disconnected":
        case "offline":
        case "unavailable": return offline
        default: return textDim
        }
    }

    function statusIconName(status) {
        switch (String(status).toLowerCase()) {
        case "connected":
        case "available":
        case "active":
        case "online":
        case "success": return "check-circle"
        case "connecting":
        case "reconnecting":
        case "pending": return "refresh"
        case "away":
        case "idle": return "clock"
        case "error":
        case "failed":
        case "danger": return "error"
        case "muted": return "mic-off"
        case "info": return "info"
        default: return "offline"
        }
    }

    // Text-glyph compatibility for current components. New UI should use
    // statusIconName()/categoryIconName() with HarborIcon.
    function statusIcon(status) {
        switch (statusIconName(status)) {
        case "check-circle": return "●"
        case "refresh": return "↻"
        case "clock": return "◷"
        case "error": return "!"
        case "mic-off": return "×"
        case "info": return "i"
        default: return "○"
        }
    }

    function categoryColor(category) {
        switch (String(category).toLowerCase()) {
        case "game": return brandViolet
        case "app": return accent
        case "online": return online
        case "offline": return offline
        case "call": return teal
        case "network": return warning
        case "system": return textDim
        case "security": return danger
        default: return accent
        }
    }

    function categoryIconName(category) {
        switch (String(category).toLowerCase()) {
        case "game": return "game"
        case "app": return "app"
        case "online": return "online"
        case "offline": return "offline"
        case "call": return "mic"
        case "network": return "network"
        case "system": return "settings"
        case "security": return "lock"
        default: return "activity"
        }
    }

    function toneColor(tone) {
        switch (String(tone).toLowerCase()) {
        case "success":
        case "positive": return success
        case "warning": return warning
        case "danger":
        case "negative":
        case "error": return danger
        case "info": return info
        case "neutral": return offline
        default: return accent
        }
    }
}
