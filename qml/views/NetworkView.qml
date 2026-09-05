pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Harbor 2.0

Item {
    id: root

    readonly property bool compact: width < Theme.breakpointCompact
    // The live core swaps this page's fixtures for real facts: the check
    // button measures a real pinned exchange and the metric cards render
    // what was measured (— until then). The preview world stays labeled.
    readonly property bool live: typeof HarborCore !== "undefined" && HarborCore.coreReady

    HarborNetworkBridge {
        id: networkBridge
        facade: root.live ? HarborCore : null
    }

    function msValue(measured) {
        return measured !== undefined
               ? I18n.number(Math.round(Number(measured) * 10) / 10) + " ms"
               : I18n.t("common.notAvailable")
    }

    function stateTitleKey() {
        switch (AppState.connectionState) {
        case "connected": return "network.state.steady"
        case "connecting": return "network.state.opening"
        case "reconnecting": return "network.state.finding"
        default: return "network.state.offline"
        }
    }

    function stateTone() {
        if (AppState.connectionState === "connected") return "success"
        if (AppState.connectionState === "connecting" || AppState.connectionState === "reconnecting") return "warning"
        return "danger"
    }

    // HarborRouteMap contract: { label, iconName, latencyMs, active, emphasized }
    readonly property var routeMapNodes: AppState.routeNodes.map(function(node) {
        return {
            label: I18n.t(node.labelKey,
                          node.kind === "partner"
                          ? { name: AppState.partnerName }
                          : node.labelParams || {}),
            iconName: node.kind === "local" ? "app" : node.kind === "route" ? "network" : "user",
            latencyMs: Number(node.latency) || 0,
            active: node.state !== "offline",
            emphasized: node.kind === "partner"
        }
    })

    // Leaving the page stops the deterministic sequence: the scenario only
    // advances while this view is alive.
    Component.onDestruction: MockController.cancelDiagnostics()

    HarborStateLayer {
        anchors.fill: parent
        pageState: AppState.pageState("network")
        title: pageState === "empty" ? I18n.t("state.empty.title")
             : pageState === "error" ? I18n.t("state.error.title") : ""
        description: pageState === "error" ? I18n.t("state.error.description") : ""
        actionText: pageState === "error" ? I18n.t("common.actions.retry") : ""
        onActionTriggered: MockController.transitionPage("network", "content")

        HarborPage {
            width: parent.width
            height: parent.height
            accessibleName: I18n.t("sidebar.network")

            HarborPageHeader {
                title: I18n.t("network.title")
                subtitle: I18n.t("network.subtitle")
                iconName: "network"

                HarborButton {
                    variant: "secondary"
                    readonly property bool running: root.live
                        ? AppState.networkDiagnosticsRunning
                        : MockController.diagnosticsRunning
                    busy: running
                    enabled: !running
                    text: running
                          ? I18n.t(root.live ? "network.check.live.running"
                                             : "network.check.running")
                          : I18n.t(root.live ? "network.check.live.run"
                                             : "network.check.run")
                    onClicked: root.live ? networkBridge.run()
                                         : MockController.runDiagnostics()
                }
            }

            HarborSectionCard {
                Layout.fillWidth: true

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.sp4

                    // Glyph shape tracks the state (check / refresh / plug),
                    // so color never carries the meaning alone.
                    Rectangle {
                        Layout.alignment: Qt.AlignTop
                        implicitWidth: root.compact ? 54 : 64
                        implicitHeight: implicitWidth
                        radius: width / 2
                        color: Theme.surfaceSunken
                        border.width: 2

                        readonly property color ringColor: AppState.connectionState === "connected" ? Theme.success
                            : AppState.connectionState === "disconnected" ? Theme.danger : Theme.warning

                        border.color: ringColor

                        HarborIcon {
                            anchors.centerIn: parent
                            name: AppState.connectionState === "connected" ? "check-circle"
                                : AppState.connectionState === "disconnected" ? "offline" : "refresh"
                            color: parent.ringColor
                            implicitWidth: 26
                            implicitHeight: 26
                        }
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp1

                        Text {
                            Layout.fillWidth: true
                            text: I18n.t(root.stateTitleKey())
                            color: Theme.textPrimary
                            font.family: Theme.fontFamilyDisplay
                            font.pixelSize: root.compact ? Theme.fontHeading : Theme.fontTitle
                            font.weight: Font.Bold
                            wrapMode: Text.WordWrap
                        }

                        Text {
                            Layout.fillWidth: true
                            text: AppState.connectionState === "connected"
                                  ? I18n.t("network.route.connectedFor", { duration: AppState.sessionTime })
                                  : I18n.t("network.route.retrying")
                            color: Theme.textSecondary
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontSmall
                            wrapMode: Text.WordWrap
                        }

                        HarborBadge {
                            showDot: true
                            tone: root.stateTone()
                            text: AppState.connectionState === "connected" ? I18n.t("common.status.connected")
                                : AppState.connectionState === "disconnected" ? I18n.t("common.status.disconnected")
                                : I18n.t("common.status.reconnecting")
                            Accessible.description: I18n.t("a11y.connectionStatus", { status: text })
                        }
                    }

                    HarborButton {
                        Layout.alignment: Qt.AlignTop
                        variant: "secondary"
                        text: AppState.connectionState === "connected"
                              ? I18n.t("common.actions.reconnect") : I18n.t("common.actions.connect")
                        onClicked: {
                            if (AppState.connectionState === "connected")
                                MockController.reconnect()
                            else
                                MockController.connect()
                        }
                    }
                }

                // Deterministic check progress: fixed stages, no measurements.
                // The live world measures instead of animating, so this
                // staged sequence belongs to the preview only.
                ColumnLayout {
                    Layout.fillWidth: true
                    visible: !root.live && MockController.diagnosticsRunning
                    spacing: Theme.sp1

                    Text {
                        text: MockController.diagnosticsStage === "route" ? I18n.t("network.check.stage.route")
                              : MockController.diagnosticsStage === "latency" ? I18n.t("network.check.stage.latency")
                              : MockController.diagnosticsStage === "traffic" ? I18n.t("network.check.stage.traffic")
                              : I18n.t("network.check.stage.complete")
                        color: Theme.textSecondary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontSmall
                    }

                    ProgressBar {
                        Layout.fillWidth: true
                        from: 0
                        to: 100
                        value: MockController.diagnosticsProgress
                        Accessible.name: I18n.t("network.check.run")
                        Accessible.description: I18n.t("a11y.progress", {
                            label: parent.children[0].text,
                            value: Math.round(MockController.diagnosticsProgress)
                        })
                    }
                }
            }

            GridLayout {
                id: cardsGrid

                Layout.fillWidth: true
                columns: root.compact ? 2 : 4
                columnSpacing: Theme.sp3
                rowSpacing: Theme.sp3

                // The live world reports only measured facts; — marks what
                // no real check has measured yet. The preview world keeps
                // its labeled fixture cards.
                Repeater {
                    model: root.live ? diagCardsModel : null

                    HarborMetricCard {
                        required property var modelData
                        Layout.fillWidth: true
                        label: I18n.t(modelData.labelKey)
                        iconName: modelData.iconName
                        value: modelData.value
                        unit: modelData.unit || ""
                        trendText: modelData.trendText || ""
                    }
                }

                readonly property var diagCardsModel: {
                    if (!root.live)
                        return []
                    var diag = AppState.networkDiagnostics
                    var configured = diag !== null && diag.serverConfigured
                    var reachable = diag !== null && diag.serverReachable
                    var directActive = diag !== null && diag.directActive
                    return [
                        {
                            labelKey: "network.diag.control",
                            iconName: "network",
                            value: !configured ? I18n.t("network.diag.unconfigured")
                                 : reachable ? I18n.t("network.diag.online")
                                 : I18n.t("network.diag.offline")
                        },
                        {
                            labelKey: "network.diag.controlRtt",
                            iconName: "clock",
                            value: reachable && diag.rttMs !== undefined
                                   ? root.msValue(diag.rttMs)
                                   : I18n.t("common.notAvailable"),
                            trendText: I18n.t("network.metric.live.measured")
                        },
                        {
                            labelKey: "network.diag.handshake",
                            iconName: "lock",
                            value: reachable && diag.handshakeMs !== undefined
                                   ? root.msValue(diag.handshakeMs)
                                   : I18n.t("common.notAvailable"),
                            trendText: I18n.t("network.metric.live.measured")
                        },
                        {
                            labelKey: "network.diag.direct",
                            iconName: "user",
                            value: directActive ? root.msValue(diag.directRttMs)
                                   : I18n.t("network.diag.directIdle"),
                            trendText: directActive
                                       ? I18n.t("network.diag.quality."
                                                + (diag.directQuality || "poor"))
                                       : ""
                        }
                    ]
                }

                readonly property var previewCardsModel: [
                    {
                        labelKey: "common.labels.quality",
                        iconName: "check-circle",
                        value: I18n.percent(AppState.networkQuality, { isRatio: false }),
                        progress: AppState.networkQuality / 100
                    },
                    {
                        labelKey: "common.labels.latency",
                        iconName: "clock",
                        value: I18n.number(AppState.latency),
                        unit: " ms",
                        trendText: I18n.t("network.metric.roundTrip")
                    },
                    {
                        labelKey: "common.labels.download",
                        iconName: "chevron-down",
                        value: I18n.number(AppState.download),
                        unit: " Mb/s",
                        trendText: I18n.t("network.metric.current")
                    },
                    {
                        labelKey: "common.labels.upload",
                        iconName: "chevron-up",
                        value: I18n.number(AppState.upload),
                        unit: " Mb/s",
                        trendText: I18n.t("network.metric.current")
                    }
                ]
            }

            // Fixture cards for the preview world only: they never pose as
            // measurements next to a live core.
            Repeater {
                model: root.live ? null : cardsGrid.previewCardsModel

                HarborMetricCard {
                    required property var modelData
                    Layout.fillWidth: true
                    label: I18n.t(modelData.labelKey)
                    iconName: modelData.iconName
                    value: modelData.value
                    unit: modelData.unit || ""
                    progress: modelData.progress !== undefined ? modelData.progress : 0
                    trendText: modelData.trendText || ""
                }
            }

            GridLayout {
                Layout.fillWidth: true
                columns: root.compact ? 1 : 2
                columnSpacing: Theme.sp4
                rowSpacing: Theme.sp4

                HarborSectionCard {
                    Layout.fillWidth: true
                    Layout.preferredWidth: 620
                    title: I18n.t("network.traffic.title")

                    HarborGraph {
                        Layout.fillWidth: true
                        Layout.preferredHeight: 240
                        unit: " Mb/s"
                        accessibleName: I18n.t("network.traffic.title")
                        series: [
                            { label: I18n.t("network.traffic.down"), values: AppState.downloadHistory, color: Theme.chartSeries1, dashed: false },
                            { label: I18n.t("network.traffic.up"), values: AppState.uploadHistory, color: Theme.chartSeries2, dashed: true }
                        ]
                    }

                    RowLayout {
                        Layout.fillWidth: true

                        Text {
                            Layout.fillWidth: true
                            text: I18n.t("network.traffic.secondsAgo")
                            color: Theme.textFaint
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontTiny
                        }

                        Text {
                            text: I18n.t("network.traffic.now")
                            color: Theme.textFaint
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontTiny
                        }
                    }
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    Layout.preferredWidth: 330
                    spacing: Theme.sp4

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("network.route.title")

                        HarborRouteMap {
                            Layout.fillWidth: true
                            Layout.topMargin: Theme.sp2
                            nodeSize: 44
                            linkLength: 34
                            nodes: root.routeMapNodes
                            linkState: AppState.connectionState === "connected" ? "connected"
                                     : AppState.connectionState === "disconnected" ? "offline" : "reconnecting"
                        }
                    }

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("network.routeDetails.title")

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            Repeater {
                                model: [
                                    { labelKey: "common.labels.transport", valueKey: "network.route.transportValue" },
                                    { labelKey: "common.labels.encryption", valueKey: "network.route.encryptionValue" },
                                    { labelKey: "common.labels.route", valueKey: "network.route.routeValue" },
                                    { labelKey: "common.labels.region", valueKey: "network.route.regionValue" }
                                ]

                                RowLayout {
                                    id: routeRow

                                    required property var modelData

                                    Layout.fillWidth: true

                                    Text {
                                        Layout.fillWidth: true
                                        text: I18n.t(routeRow.modelData.labelKey)
                                        color: Theme.textSecondary
                                        font.family: Theme.fontFamily
                                        font.pixelSize: Theme.fontSmall
                                    }

                                    Text {
                                        text: I18n.t(routeRow.modelData.valueKey)
                                        color: Theme.textPrimary
                                        font.family: Theme.fontFamily
                                        font.pixelSize: Theme.fontSmall
                                        font.weight: Font.DemiBold
                                    }
                                }
                            }
                        }
                    }

                    HarborSectionCard {
                        Layout.fillWidth: true

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp3

                            Rectangle {
                                implicitWidth: 40
                                implicitHeight: 40
                                radius: Theme.radius
                                color: Theme.surfaceSunken

                                HarborIcon {
                                    anchors.centerIn: parent
                                    name: "lock"
                                    color: Theme.iconSecondary
                                    implicitWidth: 20
                                    implicitHeight: 20
                                }
                            }

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp1

                                Text {
                                    Layout.fillWidth: true
                                    text: I18n.t("network.private.title")
                                    color: Theme.textPrimary
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontBody
                                    font.weight: Font.DemiBold
                                    wrapMode: Text.WordWrap
                                }

                                Text {
                                    Layout.fillWidth: true
                                    text: I18n.t("network.private.description")
                                    color: Theme.textSecondary
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontSmall
                                    wrapMode: Text.WordWrap
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
