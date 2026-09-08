import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Item {
    id: root

    property color waveColor: Theme.accent
    property color secondaryColor: Theme.teal
    property real amplitude: 10
    property real frequency: 1.7
    property real lineWidth: 2
    property real phase: 0
    property bool running: visible && !AppState.reducedMotion
    property bool showSecondaryWave: true

    implicitWidth: 240
    implicitHeight: 56
    Accessible.ignored: true

    onWaveColorChanged: canvas.requestPaint()
    onSecondaryColorChanged: canvas.requestPaint()
    onAmplitudeChanged: canvas.requestPaint()
    onFrequencyChanged: canvas.requestPaint()
    onPhaseChanged: canvas.requestPaint()
    onWidthChanged: canvas.requestPaint()
    onHeightChanged: canvas.requestPaint()

    NumberAnimation on phase {
        running: root.running
        from: 0
        to: Math.PI * 2
        duration: 2400
        loops: Animation.Infinite
    }

    Canvas {
        id: canvas
        anchors.fill: parent
        antialiasing: true

        function drawWave(ctx, color, phaseOffset, amplitudeScale) {
            ctx.beginPath()
            ctx.strokeStyle = color
            ctx.lineWidth = root.lineWidth
            for (let x = 0; x <= width; x += 2) {
                const angle = x / Math.max(1, width) * Math.PI * 2 * root.frequency + root.phase + phaseOffset
                const y = height / 2 + Math.sin(angle) * root.amplitude * amplitudeScale
                if (x === 0) ctx.moveTo(x, y)
                else ctx.lineTo(x, y)
            }
            ctx.stroke()
        }

        onPaint: {
            const ctx = getContext("2d")
            ctx.clearRect(0, 0, width, height)
            drawWave(ctx, root.waveColor, 0, 1)
            if (root.showSecondaryWave) drawWave(ctx, root.secondaryColor, Math.PI * 0.65, 0.58)
        }
    }
}
