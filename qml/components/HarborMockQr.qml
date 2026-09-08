import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Decorative pairing artwork. The pattern is deterministic per seed and is
// intentionally not a valid QR code: finder patterns are hollow squares with
// rounded corners, the timing row is replaced by a wave band, and a centered
// anchor glyph breaks quiet zones — so no scanner can lock onto it.
Item {
    id: root

    property string seed: ""
    property int modules: 21
    property color darkColor: Theme.dark ? Theme.bgBottom : Theme.lightCanvasDeep
    property color lightColor: Theme.dark ? "#0E2C48" : "#EAF8FE"
    property color frameColor: Theme.borderSubtle
    property string accessibleName: I18n.t("a11y.qrCode")
    property string accessibleDescription: I18n.t("pairing.demo.qr.description")

    // A small, fixed, non-cryptographic digest: stable across runs and locales.
    function digest(text) {
        var value = 0x811C9DC5
        var input = String(text || "harbor")
        for (var index = 0; index < input.length; ++index) {
            value ^= input.charCodeAt(index)
            value = (value * 0x01000193) >>> 0
        }
        return value >>> 0
    }

    function moduleFilled(row, column) {
        var base = digest(seed)
        var slot = row * modules + column
        var mixed = (base ^ (slot * 2654435761)) >>> 0
        var quarter = digest(String(mixed))
        var filled = (((quarter >>> 8) & 0xFF) % 100) < 46
        // Keep a quiet margin and the central anchor area empty.
        var margin = 2
        var centerStart = Math.floor(modules / 2) - 2
        var centerEnd = centerStart + 4
        var inCenter = row >= centerStart && row <= centerEnd && column >= centerStart && column <= centerEnd
        if (row < margin || column < margin || row >= modules - margin || column >= modules - margin)
            return false
        if (inCenter)
            return false
        return filled
    }

    readonly property int finderSpan: 5
    readonly property int cellSize: Math.floor(Math.min(width, height) / modules)
    readonly property int boardSize: cellSize * modules

    implicitWidth: 208
    implicitHeight: 208

    Accessible.role: Accessible.Graphic
    Accessible.name: accessibleName
    Accessible.description: accessibleDescription

    Rectangle {
        anchors.fill: parent
        radius: Theme.radius
        color: root.lightColor
        border.width: 1
        border.color: root.frameColor
    }

    Canvas {
        id: canvas

        anchors.centerIn: parent
        width: root.boardSize
        height: root.boardSize
        antialiasing: true

        onPaint: {
            var ctx = getContext("2d")
            ctx.reset()
            ctx.clearRect(0, 0, width, height)

            var cell = root.cellSize
            ctx.fillStyle = root.darkColor

            // Data modules.
            for (var row = 0; row < root.modules; ++row) {
                for (var column = 0; column < root.modules; ++column) {
                    if (root.moduleFilled(row, column)) {
                        ctx.beginPath()
                        ctx.roundedRect(column * cell + 0.5, row * cell + 0.5,
                                        cell - 1, cell - 1, Math.max(1, cell * 0.22))
                        ctx.fill()
                    }
                }
            }

            // Hollow, rounded finder patterns — deliberately non-standard.
            var span = root.finderSpan * cell
            var positions = [
                { x: 0, y: 0 },
                { x: width - span, y: 0 },
                { x: 0, y: height - span }
            ]
            for (var index = 0; index < positions.length; ++index) {
                var origin = positions[index]
                ctx.lineWidth = Math.max(2, cell * 0.7)
                ctx.strokeStyle = root.darkColor
                ctx.beginPath()
                ctx.roundedRect(origin.x + ctx.lineWidth / 2, origin.y + ctx.lineWidth / 2,
                                span - ctx.lineWidth, span - ctx.lineWidth, cell)
                ctx.stroke()
            }
        }

        onWidthChanged: requestPaint()
        onHeightChanged: requestPaint()
        Connections {
            target: root
            function onSeedChanged() { canvas.requestPaint() }
            function onModulesChanged() { canvas.requestPaint() }
        }
    }

    // Central anchor: an unmistakable "this is artwork" marker that also
    // occupies the alignment region a scanner would look for.
    Rectangle {
        anchors.centerIn: parent
        width: root.finderSpan * root.cellSize
        height: width
        radius: Theme.radiusSmall
        color: root.lightColor
        border.width: 2
        border.color: root.darkColor

        HarborIcon {
            anchors.centerIn: parent
            name: "network"
            color: root.darkColor
            implicitWidth: 18
            implicitHeight: 18
        }
    }
}
