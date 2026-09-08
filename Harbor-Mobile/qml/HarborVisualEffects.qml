// Central policy for mobile-safe rendering. Mobile starts conservative:
// flat surfaces and no scene-graph layer effects. Desktop may opt into
// richer visuals without sharing an Android-specific render pipeline.
pragma Singleton
import QtQuick

QtObject {
    readonly property bool isAndroid: Qt.platform.os === "android"
    readonly property bool diagnosticsActive:
        typeof harborRender !== "undefined" && harborRender.effectsDisabled === true // qmllint disable unqualified
    readonly property bool advancedEffects:
        !isAndroid && !diagnosticsActive
}
