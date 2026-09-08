import QtQuick
import QtQuick.Layouts

// Desktop companion: partner presence, current activity, and an optional
// joined-call symbol. Purely a mirror of AppState — no timers, no polling,
// no notifications, no controls. It updates only when the session facts it
// reads change, and it follows the global Theme like every other surface.
Window {
    id: root

    flags: Qt.Tool | Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint | Qt.WindowDoesNotAcceptFocus
    // Independent top-level: hiding/minimizing the shell must never take
    // this window down with it (a transient Tool would follow its parent).
    transientParent: null
    color: "transparent"
    title: "Harbor"
    width: 252
    // Height follows the content column (layouts report implicit size from
    // their children); there is no header, footer, or sidebar.
    height: content.implicitHeight + 2 * (8 + Theme.sp3)

    signal openRequested()
    signal openCallRequested()

    readonly property int edgeMargin: 16
    readonly property bool paired: AppState.paired
    // Normalized by AppState: "online" | "idle" | "offline".
    readonly property string presence: AppState.partnerState
    readonly property bool inCall: AppState.callState === "connected"
    readonly property var latestActivity: {
        var timeline = AppState.remoteActivities
        if (!paired || timeline.length === 0)
            return null
        var first = timeline[0]
        if ((first.category === "game" || first.category === "app")
                && String(first.label || "").length > 0)
            return first
        return null
    }
    readonly property bool showCall: paired && inCall && AppState.widgetShowCallPresence
    readonly property string partnerDisplayName: AppState.partnerName.length > 0
        ? AppState.partnerProfile.name : I18n.t("home.partner.unnamed")

    function presenceText() {
        if (!paired)
            return I18n.t("widget.unpaired.status")
        if (presence === "idle")
            return I18n.t("widget.presence.away")
        if (presence === "offline")
            return I18n.t("widget.presence.offline")
        return I18n.t("widget.presence.online")
    }

    function presenceColor() {
        if (!paired)
            return Theme.textFaint
        if (presence === "idle")
            return Theme.idle
        if (presence === "offline")
            return Theme.offline
        return Theme.online
    }

    // Pure corner math, unit-testable without a screen: returns {x, y} for
    // a w×h window inside the available area with an edge margin.
    function computePosition(position, availX, availY, availW, availH, w, h, margin) {
        var left = position === "topLeft" || position === "bottomLeft"
        var top = position === "topLeft" || position === "topRight"
        return {
            x: left ? availX + margin : availX + availW - w - margin,
            y: top ? availY + margin : availY + availH - h - margin
        }
    }

    function place() {
        var at = computePosition(AppState.widgetPosition, Screen.desktopAvailableX,
                                 Screen.desktopAvailableY, Screen.desktopAvailableWidth,
                                 Screen.desktopAvailableHeight, width, height, edgeMargin)
        x = Math.round(at.x)
        y = Math.round(at.y)
    }

    onWidthChanged: place()
    onHeightChanged: place()
    onVisibleChanged: if (visible) place()
    onScreenChanged: place()
    Component.onCompleted: place()
    Connections {
        target: AppState
        function onWidgetPositionChanged() { root.place() }
    }

    // Note: no Accessible block on the root — attached properties
    // require Items. The frame below carries the summary instead.

    Rectangle {
        id: frame

        objectName: "widgetFrame"
        Accessible.role: Accessible.Dialog
        Accessible.name: root.paired
                         ? root.partnerDisplayName + ", " + root.presenceText()
                         : I18n.t("widget.unpaired.title") + ", " + root.presenceText()

        anchors.fill: parent
        anchors.margins: 8
        radius: Theme.radius
        // Standalone window: no ocean behind it, so it paints its own water
        // with the same stops as the shell surface (Main.qml). A fixed glass
        // color here would stay blue for every ocean variant.
        gradient: Gradient {
            GradientStop { position: 0.0; color: Theme.bgTop }
            GradientStop { position: 0.48; color: Theme.bgMid }
            GradientStop { position: 1.0; color: Theme.bgBottom }
        }
        border.width: 1
        border.color: Theme.surfaceBorder

        // One click surface: the widget informs, the main window acts.
        MouseArea {
            anchors.fill: parent
            cursorShape: Qt.PointingHandCursor
            onClicked: root.openRequested()

            Accessible.role: Accessible.Button
            Accessible.name: I18n.t("a11y.openHarbor")
        }

        ColumnLayout {
            id: content

            anchors.fill: parent
            anchors.margins: Theme.sp3
            spacing: Theme.sp2

            // ---- Partner ------------------------------------------------
            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.sp3
                visible: root.paired

                HarborAvatar {
                    avatarSize: 44
                    visible: AppState.widgetShowAvatar
                    initials: AppState.partnerProfile.initials
                    source: AppState.partnerProfile.avatar
                    avatarType: AppState.partnerProfile.avatarType
                    status: root.presence
                    showStatus: false
                    accessibleName: I18n.t("a11y.avatarFor", { name: root.partnerDisplayName })
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 1

                    Text {
                        Layout.fillWidth: true
                        text: root.partnerDisplayName
                        color: Theme.textPrimary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontBody
                        font.weight: Font.DemiBold
                        elide: Text.ElideRight
                    }

                    RowLayout {
                        spacing: Theme.sp2

                        // Shape carries presence alongside color and text:
                        // filled (online), ring (away), faint dot (offline).
                        Item {
                            Layout.preferredWidth: 10
                            Layout.preferredHeight: 10
                            Layout.alignment: Qt.AlignVCenter
                            Accessible.ignored: true

                            Rectangle {
                                anchors.fill: parent
                                radius: width / 2
                                color: root.presence === "online" ? root.presenceColor() : "transparent"
                                border.width: root.presence === "online" ? 0 : 2
                                border.color: root.presenceColor()
                                opacity: root.presence === "offline" ? 0.55 : 1
                            }
                        }

                        Text {
                            text: root.presenceText()
                            color: root.presence === "online" ? Theme.success : Theme.textSecondary
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontSmall
                        }
                    }
                }
            }

            // ---- Unpaired: honest placeholder, never a fake partner -------
            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.sp3
                visible: !root.paired

                HarborLogo {
                    showWordmark: false
                    compact: true
                    Layout.alignment: Qt.AlignVCenter
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 1

                    Text {
                        text: I18n.t("widget.unpaired.title")
                        color: Theme.textPrimary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontBody
                        font.weight: Font.DemiBold
                    }

                    Text {
                        text: I18n.t("widget.unpaired.status")
                        color: Theme.textSecondary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontSmall
                    }
                }
            }

            // ---- Current activity: latest relevant moment only ------------
            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2
                visible: root.paired && root.presence !== "offline"
                         && AppState.widgetShowActivity && root.latestActivity !== null

                HarborIcon {
                    name: root.latestActivity !== null && root.latestActivity.category === "game" ? "game" : "app"
                    color: Theme.iconSecondary
                    implicitWidth: 16
                    implicitHeight: 16
                    Layout.alignment: Qt.AlignVCenter
                    Accessible.ignored: true
                }

                Text {
                    Layout.fillWidth: true
                    text: root.latestActivity !== null ? String(root.latestActivity.label) : ""
                    color: Theme.textPrimary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontSmall
                    font.weight: Font.DemiBold
                    elide: Text.ElideRight
                }
            }

            // ---- Joined call: two dots, one touch -------------------------
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                visible: root.showCall

                Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: 1
                    color: Theme.divider
                    Accessible.ignored: true
                }

                RowLayout {
                    Layout.alignment: Qt.AlignHCenter
                    spacing: 0

                    // Clicking opens the main window straight into the call.
                    // No mute, PTT, timer, or any call control lives here.
                    MouseArea {
                        Layout.preferredWidth: callRow.implicitWidth + Theme.sp4
                        Layout.preferredHeight: callRow.implicitHeight + Theme.sp2
                        cursorShape: Qt.PointingHandCursor
                        onClicked: root.openCallRequested()

                        Accessible.role: Accessible.Button
                        Accessible.name: I18n.t("a11y.openCall")

                        RowLayout {
                            id: callRow

                            anchors.centerIn: parent
                            spacing: 0

                            HarborAvatar {
                                avatarSize: 30
                                visible: AppState.widgetShowAvatar
                                initials: AppState.partnerProfile.initials
                                source: AppState.partnerProfile.avatar
                                avatarType: AppState.partnerProfile.avatarType
                                status: "online"
                                showStatus: false
                                speaking: AppState.remoteSpeaking
                                accessibleName: I18n.t("a11y.avatarFor", { name: root.partnerDisplayName })
                            }

                            Rectangle {
                                id: callLink

                                Layout.preferredWidth: 18
                                Layout.preferredHeight: 2
                                Layout.alignment: Qt.AlignVCenter
                                radius: 1
                                color: Theme.success
                                Accessible.ignored: true

                                SequentialAnimation on opacity {
                                    running: root.showCall && !AppState.reducedMotion
                                    loops: Animation.Infinite
                                    NumberAnimation { to: 0.35; duration: 900 }
                                    NumberAnimation { to: 1; duration: 900 }
                                }
                            }

                            HarborAvatar {
                                avatarSize: 30
                                visible: AppState.widgetShowAvatar
                                initials: AppState.selfProfile.initials
                                source: AppState.selfProfile.avatar
                                avatarType: AppState.selfProfile.avatarType
                                status: "online"
                                showStatus: false
                                accessibleName: I18n.t("a11y.avatarFor", { name: AppState.selfProfile.name })
                            }
                        }
                    }
                }
            }
        }
    }
}
