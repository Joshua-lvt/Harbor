pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Multi-series line graph for simulated metrics. Series share one value scale;
// each series is distinguishable by line style (solid/dashed) and point marker
// shape (circle/square), never by color alone. A data-table view mirrors every
// value for non-visual access.
FocusScope {
    id: root

    // [{ label, values, color, dashed }] — color/dashed optional.
    property var series: []
    property string unit: ""
    property int gridLines: 4
    property real minimumValue: 0
    property real maximumValue: NaN
    property int selectedIndex: -1
    property int hoverIndex: -1
    property string interactionMode: "pointer" // pointer, keyboard
    property bool tableVisible: false
    property string accessibleName: I18n.t("component.graph.defaultName")

    signal pointSelected(int index)

    readonly property int sampleCount: {
        var count = 0
        for (var index = 0; index < series.length; ++index) {
            var values = series[index].values || []
            count = Math.max(count, values.length)
        }
        return count
    }
    readonly property real resolvedMaximum: {
        if (!isNaN(maximumValue))
            return maximumValue
        var high = minimumValue + 1
        for (var s = 0; s < series.length; ++s) {
            var values = series[s].values || []
            for (var i = 0; i < values.length; ++i)
                high = Math.max(high, Number(values[i]))
        }
        return high
    }
    readonly property int activeIndex: interactionMode === "keyboard"
        ? selectedIndex : hoverIndex >= 0 ? hoverIndex : selectedIndex

    implicitWidth: 360
    implicitHeight: 220
    activeFocusOnTab: true

    Accessible.role: Accessible.Chart
    Accessible.name: accessibleName
    Accessible.description: accessibleSummary()

    function formattedValue(entry, index) {
        var values = entry && entry.values ? entry.values : []
        if (index < 0 || index >= values.length || !isFinite(Number(values[index])))
            return I18n.t("common.notAvailable")
        return I18n.number(values[index]) + unit
    }

    function accessibleSummary() {
        var parts = []
        if (activeIndex >= 0 && activeIndex < sampleCount) {
            for (var seriesIndex = 0; seriesIndex < series.length; ++seriesIndex) {
                var entry = series[seriesIndex]
                parts.push((entry.label || ("#" + seriesIndex)) + " "
                           + formattedValue(entry, activeIndex))
            }
            return I18n.t("graph.sample") + " " + I18n.number(activeIndex + 1)
                + " — " + parts.join(", ")
        }

        for (var latestIndex = 0; latestIndex < series.length; ++latestIndex) {
            var latestEntry = series[latestIndex]
            var latestValues = latestEntry.values || []
            parts.push((latestEntry.label || ("#" + latestIndex)) + " "
                       + formattedValue(latestEntry, latestValues.length - 1))
        }
        return I18n.t("graph.sample") + ": " + I18n.number(sampleCount)
            + " × " + I18n.number(series.length) + "; "
            + I18n.number(minimumValue) + unit + "–" + I18n.number(resolvedMaximum) + unit
            + (parts.length > 0 ? "; " + parts.join(", ") : "")
    }

    function nearestIndex(x) {
        if (sampleCount <= 1)
            return sampleCount === 1 ? 0 : -1
        var plotWidth = plotArea.width
        if (plotWidth <= 0)
            return -1
        var index = Math.round(x / plotWidth * (sampleCount - 1))
        return Math.max(0, Math.min(sampleCount - 1, index))
    }

    function selectIndex(index) {
        if (sampleCount === 0)
            return
        selectedIndex = Math.max(0, Math.min(sampleCount - 1, index))
        pointSelected(selectedIndex)
    }

    Keys.onLeftPressed: event => {
        interactionMode = "keyboard"
        selectIndex((selectedIndex <= 0 ? sampleCount : selectedIndex) - 1)
        event.accepted = true
    }
    Keys.onRightPressed: event => {
        interactionMode = "keyboard"
        selectIndex(selectedIndex < 0 ? 0 : Math.min(sampleCount - 1, selectedIndex + 1))
        event.accepted = true
    }
    Keys.onPressed: event => {
        if (event.key === Qt.Key_Home) {
            interactionMode = "keyboard"
            selectIndex(0)
            event.accepted = true
        } else if (event.key === Qt.Key_End) {
            interactionMode = "keyboard"
            selectIndex(sampleCount - 1)
            event.accepted = true
        }
    }

    onSeriesChanged: canvas.requestPaint()
    onMinimumValueChanged: canvas.requestPaint()
    onMaximumValueChanged: canvas.requestPaint()
    onSelectedIndexChanged: canvas.requestPaint()
    onHoverIndexChanged: canvas.requestPaint()
    onGridLinesChanged: canvas.requestPaint()

    Rectangle {
        id: card

        anchors.fill: parent
        radius: Theme.radius
        color: Theme.surface
        border.width: root.activeFocus ? Theme.focusWidth : 1
        border.color: root.activeFocus ? Theme.focusRing : Theme.borderSubtle
    }

    ColumnLayout {
        id: graphLayout

        anchors.fill: parent
        anchors.margins: Theme.sp3
        spacing: Theme.sp2

        // Legend + table toggle
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.sp3

            Repeater {
                model: root.series

                RowLayout {
                    id: legendItem

                    required property var modelData
                    spacing: Theme.sp1

                    // Style swatch: solid bar or dashes, matching the line style.
                    Row {
                        spacing: 2
                        Layout.alignment: Qt.AlignVCenter

                        Repeater {
                            model: legendItem.modelData && legendItem.modelData.dashed === true ? 3 : 1

                            Rectangle {
                                required property int index
                                width: legendItem.modelData && legendItem.modelData.dashed === true ? 5 : 16
                                height: 3
                                radius: 1.5
                                color: legendItem.modelData ? legendItem.modelData.color : Theme.chartSeries1
                            }
                        }
                    }

                    Text {
                        text: legendItem.modelData ? String(legendItem.modelData.label || "") : ""
                        color: Theme.textSecondary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontTiny
                        font.weight: Font.Medium
                    }
                }
            }

            Item { Layout.fillWidth: true }

            HarborIconButton {
                iconName: "menu"
                buttonSize: 32
                checkable: true
                checked: root.tableVisible
                accessibleName: root.tableVisible ? I18n.t("graph.hideTable") : I18n.t("graph.showTable")
                toolTip: accessibleName
                onClicked: root.tableVisible = !root.tableVisible
            }
        }

        // Plot area. Axis labels render in their own column outside the
        // clipped Canvas so they are never cut off at the left edge.
        Item {
            id: plotWrapper

            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true

            Item {
                id: axisLabels

                width: 46
                height: parent.height

                Repeater {
                    model: root.gridLines + 1

                    Item {
                        id: axisLabelSlot

                        required property int index

                        readonly property real rowPosition: parent.height * axisLabelSlot.index
                            / Math.max(1, root.gridLines)

                        anchors.right: parent.right
                        anchors.rightMargin: Theme.sp1
                        y: Math.max(0, Math.min(parent.height - height,
                                                axisLabelSlot.rowPosition - height / 2))
                        width: axisLabel.implicitWidth
                        height: axisLabel.implicitHeight

                        Text {
                            id: axisLabel

                            anchors.centerIn: parent
                            text: I18n.number(Math.round(root.resolvedMaximum
                                - (root.resolvedMaximum - root.minimumValue) * axisLabelSlot.index
                                / Math.max(1, root.gridLines)))
                            color: Theme.chartAxis
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontTiny
                        }

                        Accessible.ignored: true
                    }
                }
            }

            Item {
                id: plotArea

                anchors {
                    left: axisLabels.right
                    right: parent.right
                    top: parent.top
                    bottom: parent.bottom
                }

                Canvas {
                    id: canvas

                    anchors.fill: parent
                    antialiasing: true

                    onPaint: {
                        var ctx = getContext("2d")
                        ctx.reset()
                        ctx.clearRect(0, 0, width, height)

                        // Grid lines only; value labels render in axisLabels.
                        ctx.lineWidth = 1
                        for (var row = 0; row <= root.gridLines; ++row) {
                            var gy = row * height / Math.max(1, root.gridLines)
                            ctx.strokeStyle = Theme.chartGrid
                            ctx.beginPath()
                            ctx.moveTo(0, gy)
                            ctx.lineTo(width, gy)
                            ctx.stroke()
                        }

                        if (root.sampleCount === 0 || root.series.length === 0)
                            return

                        function pointX(index) {
                            return root.sampleCount <= 1 ? width / 2 : index * width / (root.sampleCount - 1)
                        }
                        function pointY(value) {
                            var range = Math.max(0.0001, root.resolvedMaximum - root.minimumValue)
                            return height - (Number(value) - root.minimumValue) / range * height
                        }

                        for (var s = 0; s < root.series.length; ++s) {
                            var entry = root.series[s]
                            var values = entry.values || []
                            var color = entry.color || Theme.chartSeries1

                            // Area fill only under the first series keeps the
                            // chart readable when both overlap.
                            if (s === 0 && values.length > 1) {
                                ctx.beginPath()
                                ctx.moveTo(pointX(0), height)
                                for (var i = 0; i < values.length; ++i)
                                    ctx.lineTo(pointX(i), pointY(values[i]))
                                ctx.lineTo(pointX(values.length - 1), height)
                                ctx.closePath()
                                ctx.fillStyle = Qt.rgba(color.r, color.g, color.b, 0.12)
                                ctx.fill()
                            }

                            ctx.beginPath()
                            for (var j = 0; j < values.length; ++j) {
                                var x = pointX(j)
                                var y = pointY(values[j])
                                if (j === 0)
                                    ctx.moveTo(x, y)
                                else
                                    ctx.lineTo(x, y)
                            }
                            ctx.strokeStyle = color
                            ctx.lineWidth = 2.4
                            ctx.lineJoin = "round"
                            ctx.lineCap = "round"
                            if (entry.dashed === true)
                                ctx.setLineDash([6, 4])
                            else
                                ctx.setLineDash([])
                            ctx.stroke()
                            ctx.setLineDash([])

                            // Marker at the active index: circle vs square per series.
                            var active = root.activeIndex
                            if (active >= 0 && active < values.length) {
                                var mx = pointX(active)
                                var my = pointY(values[active])
                                ctx.fillStyle = color
                                ctx.strokeStyle = Theme.chartSurface
                                ctx.lineWidth = 2
                                ctx.beginPath()
                                if (s % 2 === 0) {
                                    ctx.arc(mx, my, 5, 0, Math.PI * 2)
                                } else {
                                    ctx.rect(mx - 4.5, my - 4.5, 9, 9)
                                }
                                ctx.fill()
                                ctx.stroke()
                            }
                        }

                        // Crosshair line at the active index.
                        if (root.activeIndex >= 0 && root.activeIndex < root.sampleCount) {
                            var cx = pointX(root.activeIndex)
                            ctx.strokeStyle = Theme.chartAxis
                            ctx.lineWidth = 1
                            ctx.setLineDash([3, 3])
                            ctx.beginPath()
                            ctx.moveTo(cx, 0)
                            ctx.lineTo(cx, height)
                            ctx.stroke()
                            ctx.setLineDash([])
                        }
                    }

                    MouseArea {
                        anchors.fill: parent
                        hoverEnabled: true
                        cursorShape: Qt.CrossCursor
                        onPositionChanged: mouse => {
                            root.interactionMode = "pointer"
                            root.hoverIndex = root.nearestIndex(mouse.x)
                        }
                        onExited: root.hoverIndex = -1
                        onClicked: mouse => {
                            root.interactionMode = "pointer"
                            root.forceActiveFocus()
                            root.selectIndex(root.nearestIndex(mouse.x))
                        }
                    }
                }
            }
        }

        // Readout for the active sample.
        Rectangle {
            Layout.fillWidth: true
            visible: root.activeIndex >= 0 && root.activeIndex < root.sampleCount
            implicitHeight: readoutRow.implicitHeight + Theme.sp1 * 2
            radius: Theme.radiusSmall
            color: Theme.surfaceSunken

            RowLayout {
                id: readoutRow

                anchors.fill: parent
                anchors.margins: Theme.sp1
                spacing: Theme.sp3

                Text {
                    text: I18n.t("graph.sample") + " " + I18n.number(root.activeIndex + 1)
                    color: Theme.textMuted
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontTiny
                    font.weight: Font.DemiBold
                }

                Repeater {
                    model: root.series

                    RowLayout {
                        id: readoutItem

                        required property var modelData
                        spacing: Theme.sp1

                        Rectangle {
                            implicitWidth: 10
                            implicitHeight: 10
                            radius: readoutItem.modelData && readoutItem.modelData.dashed === true ? 2 : 5
                            color: readoutItem.modelData ? readoutItem.modelData.color : Theme.chartSeries1
                        }

                        Text {
                            text: readoutItem.modelData && root.activeIndex >= 0 && root.activeIndex < root.sampleCount
                                ? (readoutItem.modelData.label || "") + " " + root.formattedValue(readoutItem.modelData, root.activeIndex)
                                : ""
                            color: Theme.textPrimary
                            font.family: Theme.fontFamilyMonospace
                            font.pixelSize: Theme.fontTiny
                            font.features: { "tnum": 1 }
                        }
                    }
                }

                Item { Layout.fillWidth: true }
            }
        }

        // Tabular view of every sample.
        Rectangle {
            id: tableCard

            Layout.fillWidth: true
            visible: root.tableVisible
            implicitHeight: Math.min(190, tableLoader.implicitHeight + 2)
            radius: Theme.radiusSmall
            color: Theme.surfaceSunken
            clip: true
            Accessible.role: Accessible.Table
            Accessible.name: I18n.t("graph.dataTable")

            ColumnLayout {
                id: tableLoader

                width: parent.width
                spacing: 0

                RowLayout {
                    Layout.fillWidth: true
                    Layout.margins: Theme.sp2
                    spacing: Theme.sp3

                    Text {
                        text: I18n.t("graph.sample")
                        color: Theme.textMuted
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontTiny
                        font.weight: Font.DemiBold
                        Layout.preferredWidth: 52
                    }

                    Repeater {
                        model: root.series

                        Text {
                            required property var modelData
                            text: modelData ? String(modelData.label || "") + (root.unit.length > 0 ? " (" + root.unit + ")" : "") : ""
                            color: Theme.textMuted
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontTiny
                            font.weight: Font.DemiBold
                            Layout.fillWidth: true
                        }
                    }
                }

                Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: 1
                    color: Theme.divider
                }

                // Scrollable body for long series.
                Flickable {
                    id: tableBody

                    Layout.fillWidth: true
                    implicitHeight: Math.min(140, tableColumn.implicitHeight)
                    contentHeight: tableColumn.implicitHeight
                    clip: true
                    interactive: contentHeight > height

                    ColumnLayout {
                        id: tableColumn

                        width: tableBody.width
                        spacing: 0

                        Repeater {
                            model: root.sampleCount

                            Item {
                                id: tableRow

                                required property int index
                                readonly property bool isActive: root.activeIndex === index

                                Layout.fillWidth: true
                                implicitHeight: rowTexts.implicitHeight + Theme.sp1

                                Rectangle {
                                    visible: tableRow.isActive
                                    anchors.fill: parent
                                    color: Theme.selection
                                }

                                RowLayout {
                                    id: rowTexts

                                    anchors.fill: parent
                                    spacing: Theme.sp3

                                    Text {
                                        text: I18n.number(tableRow.index + 1)
                                        color: tableRow.isActive ? Theme.textPrimary : Theme.textSecondary
                                        font.family: Theme.fontFamilyMonospace
                                        font.pixelSize: Theme.fontTiny
                                        font.features: { "tnum": 1 }
                                        Layout.preferredWidth: 52 - Theme.sp2
                                        Layout.leftMargin: Theme.sp2
                                    }

                                    Repeater {
                                        model: root.series

                                        Text {
                                            required property var modelData
                                            text: modelData && modelData.values && tableRow.index < modelData.values.length
                                                ? I18n.number(modelData.values[tableRow.index])
                                                : I18n.t("common.notAvailable")
                                            color: tableRow.isActive ? Theme.textPrimary : Theme.textSecondary
                                            font.family: Theme.fontFamilyMonospace
                                            font.pixelSize: Theme.fontTiny
                                            font.features: { "tnum": 1 }
                                            Layout.fillWidth: true
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
