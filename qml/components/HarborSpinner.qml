import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Item {
    id: root

    property int spinnerSize: 32
    property color color: Theme.accent
    property real lineWidth: Math.max(2, spinnerSize / 10)
    property bool running: visible && !AppState.reducedMotion
    property string accessibleName: I18n.t("component.spinner.loading")

    implicitWidth: spinnerSize
    implicitHeight: spinnerSize

    Accessible.role: Accessible.Indicator
    Accessible.name: accessibleName

    Canvas {
        id: canvas
        anchors.fill: parent
        antialiasing: true
        onPaint: {
            const ctx = getContext("2d")
            ctx.clearRect(0, 0, width, height)
            ctx.beginPath()
            ctx.strokeStyle = root.color
            ctx.lineWidth = root.lineWidth
            ctx.lineCap = "round"
            const inset = root.lineWidth / 2 + 1
            ctx.arc(width / 2, height / 2, Math.max(1, width / 2 - inset), -Math.PI * 0.2, Math.PI * 1.25)
            ctx.stroke()
        }
    }

    RotationAnimator on rotation {
        running: root.running
        from: 0
        to: 360
        duration: 850
        loops: Animation.Infinite
    }

    onColorChanged: canvas.requestPaint()
    onLineWidthChanged: canvas.requestPaint()
    onWidthChanged: canvas.requestPaint()
    onHeightChanged: canvas.requestPaint()
}
