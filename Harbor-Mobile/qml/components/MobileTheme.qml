// Mobile visual foundation, mirroring the desktop Harbor tokens.
// Same personalization inputs (appearance mode, accent, ocean variant,
// corners, density, contrast, motion), same defaults, same palettes —
// rendered as flat bounded surfaces per the mobile render-safety contract
// (no scene-graph effects, so there is no transparency-blur layer here:
// surfaces are solid colors sampled from the same ocean stops).
import QtQuick

QtObject {
    id: theme

    // ── Personalization inputs (durable, live; shell binds these) ──
    // "dark" | "light" | "system". Unknown values resolve to light.
    property string appearanceMode: "dark"
    // Live OS scheme, fed by the shell. Flipping it re-resolves "system".
    property bool systemDark: false
    // Preset key or custom #RRGGBB. Unknown values fall back to ocean.
    property string accentColor: "ocean"
    // Accent strength scales saturation toward gray; 1.0 is the pure base.
    property real accentIntensity: 0.75
    // "lagoon" | "abyss" | "sunrise" | "rose" | "ember" | "onyx" | "forest" | "dusk"
    property string oceanVariant: "lagoon"
    // "soft" | "medium"
    property string cornerRadius: "soft"
    // "comfortable" | "compact"
    property string density: "comfortable"
    property bool higherContrast: false
    property bool reducedMotion: false
    // Animation strength scales every duration; reduced motion wins outright.
    property real animationIntensity: 1.0

    readonly property bool themeDark: appearanceMode === "dark"
        || (appearanceMode === "system" && systemDark)

    // ── Accent ──────────────────────────────────────────────────────
    readonly property var accentBases: ({
        "ocean": "#3AA9DC",
        "cyan": "#22B8CF",
        "aqua": "#3ED6B8",
        "blue": "#4A90E2",
        "teal": "#20C997",
        "violet": "#845EF7",
        "pink": "#E87BA4"
    })
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
        var key = String(theme.accentColor || "ocean")
        if (theme.accentBases[key] !== undefined)
            return theme.accentBases[key]
        if (/^#[0-9a-fA-F]{6}$/.test(key))
            return key
        return theme.accentBases["ocean"]
    }
    readonly property real accentSaturation: Math.max(0.15, Math.min(1, theme.accentIntensity))
    readonly property color accentTuned: {
        var base = theme.accentBase
        if (base.hslSaturation <= 0)
            return base
        return Qt.hsla(base.hslHue,
                        Math.max(0, Math.min(1, base.hslSaturation * theme.accentSaturation)),
                        base.hslLightness, 1)
    }
    readonly property color accent: theme.themeDark ? theme.accentTuned : Qt.darker(theme.accentTuned, 1.3)
    readonly property color accentDeep: Qt.darker(theme.accent, 1.35)
    readonly property color accentText: theme.themeDark ? "#04182B" : "#FFFFFF"

    // ── Ocean stops (same water as the desktop shell) ───────────────
    readonly property var oceanPalettes: {
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
    readonly property var oceanStops: {
        var stops = theme.oceanPalettes[theme.oceanVariant] || theme.oceanPalettes["lagoon"]
        return theme.themeDark ? stops.dark : stops.light
    }
    readonly property color bgTop: oceanStops[0]
    readonly property color bgMid: oceanStops[1]
    readonly property color bgBottom: oceanStops[2]

    // Flat surfaces sampled from the same stops (solid fills only).
    readonly property color background: bgTop
    readonly property color bar: bgMid
    readonly property color card: bgBottom
    readonly property color bubbleIn: bgBottom
    // Outgoing bubbles share the primary-button pairing: accent fill with
    // the accent text color on top.
    readonly property color bubbleOut: accent

    readonly property color borderSubtle: theme.themeDark ? "#2f4f60" : "#7A214B63"
    readonly property color borderStrong: theme.higherContrast
        ? (theme.themeDark ? "#E6FFFFFF" : "#E60B3049")
        : (theme.themeDark ? "#4a6b7d" : "#7A214B63")

    // ── Text ────────────────────────────────────────────────────────
    readonly property color text: theme.higherContrast
        ? (theme.themeDark ? "#FFFFFFFF" : "#061D2B")
        : (theme.themeDark ? "#EAF6FC" : "#0B3049")
    readonly property color textDim: theme.higherContrast
        ? (theme.themeDark ? "#D7EAF4" : "#244F68")
        : (theme.themeDark ? "#9CC3D8" : "#4A7089")
    readonly property color textFaint: theme.higherContrast
        ? (theme.themeDark ? "#B8D3E1" : "#456B82")
        : (theme.themeDark ? "#789CB2" : "#688CA2")
    readonly property color textPrimary: text
    readonly property color textSecondary: textDim
    readonly property color textMuted: textFaint

    // ── Status (fixed and exclusive to state, like the desktop) ─────
    readonly property color online: theme.themeDark ? "#40C057" : "#2F9E44"
    readonly property color idle: theme.themeDark ? "#FCC419" : "#D97706"
    readonly property color offline: theme.themeDark ? "#7894AD" : "#607D94"
    readonly property color success: online
    readonly property color warning: idle
    readonly property color danger: theme.themeDark ? "#FF6B6B" : "#D94848"
    readonly property color actionDanger: theme.themeDark ? "#FA5252" : "#C92A2A"
    readonly property color actionDangerText: "#FFFFFF"

    // ── Type scale (same steps as the desktop shell) ────────────────
    readonly property int fontDisplay: 34
    readonly property int fontTitle: 24
    readonly property int fontHeading: 18
    readonly property int fontBody: 14
    readonly property int fontSmall: 12
    readonly property int fontTiny: 11

    // ── Shape ───────────────────────────────────────────────────────
    readonly property bool softCorners: theme.cornerRadius !== "medium"
    readonly property int radiusSmall: softCorners ? 8 : 6
    readonly property int radius: softCorners ? 14 : 10
    readonly property int radiusLarge: softCorners ? 22 : 16
    readonly property int radiusPill: 999

    // ── Spacing ─────────────────────────────────────────────────────
    readonly property int sp1: 4
    readonly property int sp2: 8
    readonly property int sp3: 12
    readonly property int sp4: 16
    readonly property int sp5: 24

    // ── Touch targets follow density ────────────────────────────────
    readonly property bool compactDensity: theme.density === "compact"
    readonly property int buttonHeight: compactDensity ? 46 : 52
    readonly property int buttonHeightLarge: compactDensity ? 56 : 64
    readonly property int navHeight: compactDensity ? 52 : 60

    // ── Motion ──────────────────────────────────────────────────────
    readonly property real motionScale: theme.reducedMotion
        ? 0 : Math.max(0, Math.min(1, theme.animationIntensity))
    readonly property int motionFast: 120
    readonly property int motionNormal: 220

    function duration(milliseconds) {
        return Math.max(0, Math.round(milliseconds * theme.motionScale))
    }

    // Top-water preview for one ocean variant (settings chips).
    function oceanTop(variant) {
        var stops = theme.oceanPalettes[variant] || theme.oceanPalettes["lagoon"]
        return theme.themeDark ? stops.dark[0] : stops.light[0]
    }

    function stateColor(state) {
        switch (String(state).toLowerCase()) {
        case "online": return theme.online
        case "idle": return theme.idle
        default: return theme.offline
        }
    }

    function stateText(state) {
        switch (String(state).toLowerCase()) {
        case "online": return qsTr("Online")
        case "idle": return qsTr("Away")
        default: return qsTr("Offline")
        }
    }
}
