pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Bar visualizer for generated preview levels. The values shown are display
// numbers produced by MockController; no audio device is ever opened. Bar
// heights change with data, never as decoration: reduced motion makes the
// transitions instant instead of freezing the data.
Item {
    id: root

    // Explicit series wins; otherwise bars are derived deterministically from
    // `level` through a fixed weighting pattern (no random sources).
    property var series: []
    property real level: 0
    property int barCount: 12
    property color barColor: Theme.actionPrimary
    property color trackColor: Theme.surfaceSunken
    property bool running: true
    property string accessibleName: I18n.t("call.audio.title")

    readonly property int resolvedCount: series.length > 0 ? series.length : Math.max(1, barCount)
    readonly property real normalizedLevel: Math.max(0, Math.min(1, level))

    implicitWidth: 220
    implicitHeight: 56

    Accessible.role: Accessible.Indicator
    Accessible.name: accessibleName
    Accessible.description: I18n.t("call.audio.microphone") + ": " + I18n.percent(normalizedLevel, { isRatio: true })

    function resolvedValue(index) {
        if (series.length > 0) {
            var raw = Number(series[index])
            return isNaN(raw) ? 0 : Math.max(0, Math.min(1, raw))
        }
        // Fixed symmetric weights keep the shape stable frame to frame.
        var weights = [0.42, 0.68, 0.88, 1.0, 0.86, 0.62,
                       0.62, 0.86, 1.0, 0.88, 0.68, 0.42,
                       0.5, 0.74, 0.78, 0.7]
        var weight = weights[index % weights.length]
        return normalizedLevel * weight
    }

    Row {
        id: barRow

        anchors.fill: parent
        spacing: root.width > 0 && root.resolvedCount > 0
            ? Math.max(2, Math.floor(root.width * 0.012))
            : 2

        Repeater {
            model: root.resolvedCount

            Item {
                id: barSlot

                required property int index

                width: Math.max(2, (barRow.width - barRow.spacing * (root.resolvedCount - 1)) / root.resolvedCount)
                height: barRow.height
                Accessible.ignored: true

                Rectangle {
                    id: track

                    anchors.fill: parent
                    radius: width / 2
                    color: root.trackColor
                }

                Rectangle {
                    id: bar

                    property real fraction: root.running ? root.resolvedValue(barSlot.index) : 0

                    anchors.bottom: parent.bottom
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: parent.width
                    height: Math.max(0, Math.min(1, fraction)) * parent.height
                    radius: width / 2
                    color: root.barColor
                    opacity: fraction > 0 ? 1 : Theme.opacitySubtle

                    Behavior on height {
                        NumberAnimation {
                            duration: Theme.duration(140)
                            easing.type: Theme.animEasing
                        }
                    }
                }
            }
        }
    }
}
