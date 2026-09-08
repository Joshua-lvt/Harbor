pragma ComponentBehavior: Bound
import QtQuick

// Schematic position panel for a shared phone fix. Deliberately NOT a map:
// no tiles, no network, no geocoding — just a local grid with the phone at
// the center, an accuracy ring, and the coordinates the view prints as text
// below. It can only ever show what the peer actually shared, so there is
// nothing here to fake, and nothing leaves the machine to render it.
Item {
    id: root

    property double latitude: 0
    property double longitude: 0
    property double accuracyMeters: -1
    readonly property bool hasFix: isFinite(latitude) && isFinite(longitude)
        && isFinite(accuracyMeters) && accuracyMeters >= 0
        && Math.abs(latitude) <= 90 && Math.abs(longitude) <= 180

    implicitWidth: 480
    implicitHeight: 300

    Accessible.role: Accessible.Graphic
    Accessible.name: I18n.t("mobile.location.mapLabel")
    Accessible.description: hasFix
        ? I18n.t("mobile.location.mapDescription", {
            latitude: root.latitude.toFixed(5),
            longitude: root.longitude.toFixed(5),
            accuracy: Math.round(root.accuracyMeters)
        })
        : I18n.t("mobile.location.waiting")

    Canvas {
        id: canvas

        anchors.fill: parent
        renderTarget: Canvas.FramebufferObject
        onPaint: {
            var ctx = getContext("2d")
            var w = width
            var h = height
            var cx = w / 2
            var cy = h / 2
            ctx.clearRect(0, 0, w, h)

            // Panel. (Plain rects: the Canvas 2D dialect here predates
            // rounded-path helpers; the card behind already rounds.)
            ctx.fillStyle = Theme.surfaceSunken
            ctx.fillRect(0, 0, w, h)
            ctx.strokeStyle = Theme.borderSubtle
            ctx.lineWidth = 1
            ctx.strokeRect(0.5, 0.5, w - 1, h - 1)

            // Grid.
            ctx.strokeStyle = Theme.divider
            ctx.lineWidth = 1
            var step = 32
            ctx.beginPath()
            for (var gx = step; gx < w; gx += step) {
                ctx.moveTo(gx + 0.5, 0)
                ctx.lineTo(gx + 0.5, h)
            }
            for (var gy = step; gy < h; gy += step) {
                ctx.moveTo(0, gy + 0.5)
                ctx.lineTo(w, gy + 0.5)
            }
            ctx.stroke()

            // Reference rings.
            var outer = Math.min(w, h) / 2 - Theme.sp3
            ctx.strokeStyle = Theme.textFaint
            ctx.lineWidth = 1
            for (var ring = 1; ring <= 2; ++ring) {
                ctx.beginPath()
                ctx.arc(cx, cy, outer * ring / 2, 0, Math.PI * 2)
                ctx.stroke()
            }

            if (root.hasFix) {
                // Accuracy disc: logarithmic so a 10 m fix and a 10 km fix
                // both read, without pretending survey precision.
                var capped = Math.min(1, Math.log10(1 + root.accuracyMeters) / 4)
                var disc = 20 + (outer - 20) * capped
                ctx.fillStyle = Theme.withOpacity(Theme.accent, 0.16)
                ctx.beginPath()
                ctx.arc(cx, cy, disc, 0, Math.PI * 2)
                ctx.fill()
                ctx.strokeStyle = Theme.accent
                ctx.lineWidth = 1.5
                ctx.beginPath()
                ctx.arc(cx, cy, disc, 0, Math.PI * 2)
                ctx.stroke()

                // Phone marker: ring plus dot, color plus shape (never color
                // alone) so state does not depend on color vision.
                ctx.strokeStyle = Theme.textPrimary
                ctx.lineWidth = 2
                ctx.beginPath()
                ctx.arc(cx, cy, 13, 0, Math.PI * 2)
                ctx.stroke()
                ctx.fillStyle = Theme.accent
                ctx.beginPath()
                ctx.arc(cx, cy, 5, 0, Math.PI * 2)
                ctx.fill()
            }
        }

        onWidthChanged: requestPaint()
        onHeightChanged: requestPaint()
    }

    Connections {
        target: Theme
        function onDarkChanged() { canvas.requestPaint() }
        function onAccentChanged() { canvas.requestPaint() }
    }
    onLatitudeChanged: canvas.requestPaint()
    onLongitudeChanged: canvas.requestPaint()
    onAccuracyMetersChanged: canvas.requestPaint()
}
