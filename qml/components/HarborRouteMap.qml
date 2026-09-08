pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Abstract, fully simulated route preview between the local harbor and the
// partner. It renders whatever node data the view supplies; it never probes,
// measures, or claims a real network path.
FocusScope {
    id: root

    // Each node: { label, iconName, latencyMs, active, emphasized }
    property var nodes: []
    // connected | reconnecting | offline
    property string linkState: "connected"
    property int nodeSize: 48
    property int linkLength: 64
    property string accessibleName: I18n.t("network.routeDetails.title")
    property string accessibleDescription: ""

    readonly property int nodeCount: nodes.length
    readonly property color linkColor: linkState === "connected" ? Theme.actionPrimary
        : linkState === "reconnecting" ? Theme.warning : Theme.offline
    readonly property string summary: {
        var labels = []
        for (var index = 0; index < nodes.length; ++index)
            labels.push(String(nodes[index].label || ""))
        return labels.join("  →  ")
    }

    implicitWidth: row.implicitWidth
    implicitHeight: row.implicitHeight

    Accessible.role: Accessible.Graphic
    Accessible.name: accessibleName
    Accessible.description: accessibleDescription.length > 0 ? accessibleDescription : summary

    RowLayout {
        id: row

        anchors.fill: parent
        spacing: Theme.sp2

        Repeater {
            model: root.nodeCount

            // Node + trailing link, so links are laid out between columns.
            RowLayout {
                id: nodeAndLink

                required property int index
                readonly property var node: root.nodes[index]
                readonly property bool isLast: index === root.nodeCount - 1
                readonly property color nodeColor: node && node.active !== false ? root.linkColor : Theme.offline

                spacing: Theme.sp2

                ColumnLayout {
                    id: nodeColumn

                    spacing: Theme.sp2
                    Layout.alignment: Qt.AlignHCenter | Qt.AlignVCenter

                    Rectangle {
                        implicitWidth: root.nodeSize
                        implicitHeight: root.nodeSize
                        radius: width / 2
                        color: Theme.surfaceOverlay
                        border.width: nodeAndLink.node && nodeAndLink.node.emphasized === true ? 3 : 2
                        border.color: nodeAndLink.nodeColor
                        Layout.alignment: Qt.AlignHCenter

                        HarborIcon {
                            anchors.centerIn: parent
                            name: nodeAndLink.node ? String(nodeAndLink.node.iconName || "network") : "network"
                            color: nodeAndLink.nodeColor
                            implicitWidth: Math.round(root.nodeSize * 0.42)
                            implicitHeight: Math.round(root.nodeSize * 0.42)
                        }

                        SequentialAnimation on opacity {
                            running: root.linkState === "reconnecting" && !AppState.reducedMotion
                            loops: Animation.Infinite
                            NumberAnimation { to: 0.45; duration: 620 }
                            NumberAnimation { to: 1; duration: 620 }
                        }
                    }

                    Text {
                        text: nodeAndLink.node ? String(nodeAndLink.node.label || "") : ""
                        color: Theme.textPrimary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontSmall
                        font.weight: Font.Medium
                        horizontalAlignment: Text.AlignHCenter
                        wrapMode: Text.Wrap
                        Layout.maximumWidth: 140
                        Layout.alignment: Qt.AlignHCenter
                    }

                    Text {
                        visible: nodeAndLink.node && Number(nodeAndLink.node.latencyMs) > 0
                        text: visible ? I18n.unit(nodeAndLink.node.latencyMs, "ms") : ""
                        color: Theme.textMuted
                        font.family: Theme.fontFamilyMonospace
                        font.pixelSize: Theme.fontTiny
                        font.features: { "tnum": 1 }
                        Layout.alignment: Qt.AlignHCenter
                    }
                }

                // Link segment drawn as discrete dashes: shape carries the
                // state, so color is never the only signal.
                Row {
                    id: link

                    visible: !nodeAndLink.isLast
                    Layout.alignment: Qt.AlignVCenter
                    Layout.preferredWidth: root.linkLength
                    spacing: 5
                    opacity: root.linkState === "offline" ? Theme.opacityMuted : 1

                    Repeater {
                        // A broken link shows half as many dashes.
                        model: root.linkState === "offline" ? 4 : 8

                        Rectangle {
                            required property int index
                            width: Math.max(3, (root.linkLength - 7 * 5) / 8)
                            height: root.linkState === "offline" && index % 2 === 1 ? 1 : 3
                            radius: height / 2
                            y: 1
                            color: root.linkColor
                        }
                    }

                    SequentialAnimation on opacity {
                        running: root.linkState === "reconnecting" && !AppState.reducedMotion
                        loops: Animation.Infinite
                        NumberAnimation { to: 0.35; duration: 520 }
                        NumberAnimation { to: 1; duration: 520 }
                    }
                }
            }
        }
    }
}
