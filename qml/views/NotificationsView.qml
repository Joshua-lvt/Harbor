import Harbor 2.0
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Notification center panel content. The shell provides the layer, backdrop,
// scrim, and Escape handling; AppState.notifications is the single source of
// data and MockController owns every mutation. Only the active filter is local
// state — it is pure presentation.
HarborOverlayView {
    id: root

    overlayActive: AppState.notificationsVisible

    property string filter: "all"

    readonly property var notifications: AppState.notifications
    readonly property int unreadCount: AppState.unreadCount

    function matches(category, unread) {
        if (filter === "all")
            return true

        if (filter === "unread")
            return unread === true

        if (filter === "activity")
            return category === "game" || category === "app" || category === "call"

        if (filter === "connection")
            return category === "online" || category === "offline" || category === "network" || category === "system"

        return true
    }

    // AppState reassigns the array on every mutation, so this binding tracks
    // mark/dismiss/clear/preview without any local copy.
    readonly property var visibleNotifications: {
        var result = []
        var source = root.notifications
        for (var i = 0; i < source.length; ++i) {
            var entry = source[i]
            if (matches(entry.category, entry.unread))
                result.push(entry)
        }
        return result
    }

    initialFocusItem: allChip
    lastFocusItem: settingsLink

    function closeCenter() {
        root.closed()
    }

    Accessible.role: Accessible.Dialog
    Accessible.name: I18n.t("notifications.title")

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // Header --------------------------------------------------------------
        RowLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: 78
            Layout.leftMargin: Theme.sp5
            Layout.rightMargin: Theme.sp3
            spacing: Theme.sp3

            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2

                RowLayout {
                    spacing: Theme.sp2

                    Text {
                        text: I18n.t("notifications.title")
                        color: Theme.text
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontTitle
                        font.weight: Font.Bold
                    }

                    Rectangle {
                        visible: root.unreadCount > 0
                        implicitWidth: Math.max(25, unreadLabel.implicitWidth + 12)
                        implicitHeight: 24
                        radius: 12
                        color: Theme.accent
                        Accessible.ignored: true

                        Text {
                            id: unreadLabel

                            anchors.centerIn: parent
                            text: root.unreadCount
                            color: "white"
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontSmall
                            font.weight: Font.Bold
                        }
                    }
                }

                Text {
                    Layout.fillWidth: true
                    text: root.unreadCount > 0
                        ? I18n.t("notifications.header.waiting", { count: root.unreadCount })
                        : I18n.t("notifications.caughtUp")
                    color: Theme.textDim
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontSmall
                }
            }

            HarborIconButton {
                iconName: "close"
                accessibleName: I18n.t("a11y.closeDialog")
                toolTip: I18n.t("common.actions.close")
                onClicked: root.closeCenter()
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 1
            color: Theme.surfaceBorder
            Accessible.ignored: true
        }

        // Filters + summary ---------------------------------------------------
        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: AppState.notificationsEnabled ? 104 : 66
            color: "transparent"

            ColumnLayout {
                anchors.fill: parent
                anchors.leftMargin: Theme.sp4
                anchors.rightMargin: Theme.sp4
                anchors.topMargin: Theme.sp2
                anchors.bottomMargin: Theme.sp2
                spacing: Theme.sp2

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.sp2

                    HarborChoiceChip {
                        id: allChip

                        text: I18n.t("notifications.filter.all")
                        checked: root.filter === "all"
                        autoExclusive: true
                        onClicked: root.filter = "all"
                    }

                    HarborChoiceChip {
                        id: unreadChip

                        objectName: "notificationFilterUnread"
                        text: I18n.t("notifications.filter.unread")
                        checked: root.filter === "unread"
                        autoExclusive: true
                        onClicked: root.filter = "unread"
                    }

                    HarborChoiceChip {
                        text: I18n.t("notifications.filter.activity")
                        checked: root.filter === "activity"
                        autoExclusive: true
                        onClicked: root.filter = "activity"
                    }

                    HarborChoiceChip {
                        text: I18n.t("notifications.filter.connection")
                        checked: root.filter === "connection"
                        autoExclusive: true
                        onClicked: root.filter = "connection"
                    }
                }

                RowLayout {
                    Layout.fillWidth: true
                    visible: AppState.notificationsEnabled
                    spacing: Theme.sp2

                    Text {
                        Layout.fillWidth: true
                        text: I18n.t("notifications.header.matching",
                                     { count: root.visibleNotifications.length })
                        color: Theme.textFaint
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontTiny
                    }

                    HarborButton {
                        variant: "quiet"
                        text: I18n.t("notifications.markAllRead")
                        enabled: root.unreadCount > 0
                        onClicked: MockController.markAllNotificationsRead()
                    }

                    HarborButton {
                        variant: "quiet"
                        text: MockController.notificationClearArmed
                              ? I18n.t("notifications.confirmClear")
                              : I18n.t("notifications.clearAll")
                        enabled: root.notifications.length > 0
                        foregroundColor: MockController.notificationClearArmed
                                         ? Theme.danger : Theme.textDim
                        onClicked: MockController.clearNotificationsWithConfirmation()
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 1
            color: Theme.surfaceBorder
            Accessible.ignored: true
        }

        // List ----------------------------------------------------------------
        Item {
            Layout.fillWidth: true
            Layout.fillHeight: true

            HarborEmptyState {
                anchors.centerIn: parent
                width: Math.min(parent.width - Theme.sp5 * 2, 360)
                visible: !AppState.notificationsEnabled
                iconName: "offline"
                title: I18n.t("notifications.paused.title")
                description: I18n.t("notifications.paused.description")
                actionText: I18n.t("notifications.resume")
                onActionTriggered: AppState.notificationsEnabled = true
            }

            HarborEmptyState {
                anchors.centerIn: parent
                width: Math.min(parent.width - Theme.sp5 * 2, 360)
                visible: AppState.notificationsEnabled && root.visibleNotifications.length === 0
                iconName: "check-circle"
                title: root.notifications.length === 0
                    ? I18n.t("notifications.empty.title")
                    : I18n.t("notifications.filteredEmpty.title")
                description: root.notifications.length === 0
                    ? I18n.t("notifications.empty.description")
                    : I18n.t("notifications.filteredEmpty.description")
                actionText: root.notifications.length === 0
                    ? I18n.t("notifications.preview")
                    : I18n.t("common.actions.showAll")
                onActionTriggered: {
                    if (root.notifications.length === 0)
                        MockController.previewNotification()
                    else
                        root.filter = "all"
                }
            }

            ListView {
                id: notificationList

                anchors.fill: parent
                visible: AppState.notificationsEnabled && root.visibleNotifications.length > 0
                clip: true
                spacing: Theme.sp2
                topMargin: Theme.sp3
                bottomMargin: Theme.sp4
                leftMargin: Theme.sp4
                rightMargin: Theme.sp4
                model: root.visibleNotifications
                boundsBehavior: Flickable.StopAtBounds

                ScrollBar.vertical: ScrollBar {
                    policy: ScrollBar.AsNeeded
                }

                delegate: Rectangle {
                    id: notificationDelegate

                    required property var modelData

                    readonly property var entry: modelData
                    readonly property color entryColor: Theme.categoryColor(entry.category)

                    width: ListView.view.width - ListView.view.leftMargin - ListView.view.rightMargin
                    height: 96
                    radius: Theme.radius
                    color: entry.unread ? Theme.surfaceStrong : Theme.surface
                    border.width: 1
                    border.color: entry.unread ? Theme.accentDeep : Theme.surfaceBorder

                    Accessible.role: Accessible.ListItem
                    Accessible.name: I18n.t(entry.titleKey, entry.titleParams)

                    Behavior on color {
                        ColorAnimation { duration: Theme.duration(Theme.motionFast) }
                    }

                    Rectangle {
                        visible: notificationDelegate.entry.unread
                        anchors.left: parent.left
                        anchors.top: parent.top
                        anchors.bottom: parent.bottom
                        width: 3
                        color: notificationDelegate.entryColor
                        Accessible.ignored: true
                    }

                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: Theme.sp3
                        anchors.rightMargin: Theme.sp2
                        anchors.topMargin: Theme.sp3
                        anchors.bottomMargin: Theme.sp3
                        spacing: Theme.sp3

                        Rectangle {
                            Layout.preferredWidth: 42
                            Layout.preferredHeight: 42
                            radius: 13
                            color: Qt.rgba(notificationDelegate.entryColor.r,
                                           notificationDelegate.entryColor.g,
                                           notificationDelegate.entryColor.b,
                                           Theme.dark ? 0.24 : 0.16)
                            Accessible.ignored: true

                            HarborIcon {
                                anchors.centerIn: parent
                                name: Theme.categoryIconName(notificationDelegate.entry.category)
                                color: notificationDelegate.entryColor
                                implicitWidth: 20
                                implicitHeight: 20
                            }
                        }

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 3

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp2

                                Text {
                                    Layout.fillWidth: true
                                    text: I18n.t(notificationDelegate.entry.titleKey,
                                                 notificationDelegate.entry.titleParams)
                                    color: Theme.text
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontBody
                                    font.weight: notificationDelegate.entry.unread ? Font.Bold : Font.DemiBold
                                    elide: Text.ElideRight
                                }

                                Text {
                                    text: I18n.t(notificationDelegate.entry.timeKey,
                                                 notificationDelegate.entry.timeParams)
                                    color: Theme.textFaint
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontTiny
                                }
                            }

                            Text {
                                Layout.fillWidth: true
                                text: I18n.t(notificationDelegate.entry.descriptionKey,
                                             notificationDelegate.entry.descriptionParams)
                                color: Theme.textDim
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontSmall
                                elide: Text.ElideRight
                            }

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp2

                                Rectangle {
                                    visible: notificationDelegate.entry.unread
                                    implicitWidth: 4
                                    implicitHeight: 4
                                    radius: 2
                                    color: Theme.accent
                                    Accessible.ignored: true
                                }

                                Text {
                                    visible: notificationDelegate.entry.unread
                                    text: I18n.t("notifications.unread")
                                    color: Theme.accent
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontTiny
                                    font.weight: Font.DemiBold
                                }

                                Item { Layout.fillWidth: true }

                                HarborButton {
                                    visible: notificationDelegate.entry.unread
                                    variant: "quiet"
                                    text: I18n.t("notifications.markRead")
                                    onClicked: MockController.markNotificationRead(notificationDelegate.entry.id)
                                }

                                HarborIconButton {
                                    iconName: "close"
                                    buttonSize: 30
                                    accessibleName: I18n.t("notifications.dismiss")
                                    toolTip: I18n.t("notifications.dismiss")
                                    onClicked: MockController.dismissNotification(notificationDelegate.entry.id)
                                }
                            }
                        }
                    }

                    // The whole card marks the item read; the dedicated button
                    // stays available for keyboard and assistive tech.
                    MouseArea {
                        anchors.fill: parent
                        z: -1
                        hoverEnabled: true
                        cursorShape: notificationDelegate.entry.unread ? Qt.PointingHandCursor : Qt.ArrowCursor
                        onClicked: {
                            if (notificationDelegate.entry.unread)
                                MockController.markNotificationRead(notificationDelegate.entry.id)
                        }
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: 1
            color: Theme.surfaceBorder
            Accessible.ignored: true
        }

        // Footer --------------------------------------------------------------
        RowLayout {
            Layout.fillWidth: true
            Layout.preferredHeight: 72
            Layout.leftMargin: Theme.sp4
            Layout.rightMargin: Theme.sp4
            spacing: Theme.sp3

            HarborToggle {
                Layout.fillWidth: true
                text: I18n.t("notifications.allow")
                description: AppState.notificationsEnabled
                    ? I18n.t("notifications.enabledSummary")
                    : I18n.t("notifications.pausedSummary")
                checked: AppState.notificationsEnabled
                onToggled: AppState.notificationsEnabled = checked
            }

            HarborButton {
                id: settingsLink

                variant: "quiet"
                text: I18n.t("common.actions.settings") + "  ›"
                onClicked: {
                    root.closed()
                    AppState.navigate("settings")
                }
            }
        }
    }
}
