pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Item {
    id: root

    property int count: 18
    property bool running: visible && !AppState.reducedMotion
    property color particleColor: Theme.accentSoft
    property real minimumSize: 2
    property real maximumSize: 7
    property real opacityScale: 0.45

    clip: true
    Accessible.ignored: true

    Repeater {
        model: root.count

        Rectangle {
            id: particle
            required property int index
            readonly property real seed: ((index * 47) % 101) / 101
            readonly property real secondSeed: ((index * 73 + 19) % 103) / 103
            readonly property real baseX: secondSeed * Math.max(0, root.width - width)
            width: root.minimumSize + seed * (root.maximumSize - root.minimumSize)
            height: width
            radius: width / 2
            x: secondSeed * Math.max(0, root.width - width)
            y: root.height + index * 13 % Math.max(1, root.height)
            color: root.particleColor
            opacity: root.opacityScale * (0.35 + seed * 0.65)

            SequentialAnimation on y {
                running: root.running
                loops: Animation.Infinite
                PauseAnimation { duration: particle.secondSeed * 1800 }
                NumberAnimation {
                    from: root.height + particle.height
                    to: -particle.height
                    duration: 5000 + particle.seed * 7000
                    easing.type: Easing.InOutSine
                }
            }
            SequentialAnimation on x {
                running: root.running
                loops: Animation.Infinite
                PauseAnimation { duration: particle.secondSeed * 2200 }
                NumberAnimation { from: particle.baseX; to: particle.baseX + 14; duration: 1800 + particle.seed * 900; easing.type: Easing.InOutSine }
                NumberAnimation { from: particle.baseX + 14; to: particle.baseX; duration: 1800 + particle.seed * 900; easing.type: Easing.InOutSine }
            }
        }
    }
}
