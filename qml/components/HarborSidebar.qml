pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

FocusScope {
    id: root

    readonly property var model: [
        { key: "home", labelKey: "sidebar.home", icon: "app" },
        { key: "call", labelKey: "sidebar.call", icon: "mic" },
        { key: "chat", labelKey: "sidebar.chat", icon: "chat" },
        { key: "activity", labelKey: "sidebar.activity", icon: "activity" },
        { key: "mobile", labelKey: "sidebar.mobile", icon: "phone" },
        { key: "settings", labelKey: "sidebar.settings", icon: "settings" }
    ]
    property string currentView: "home"
    property bool userCollapsed: false
    property bool autoCollapse: true
    // Set by the expand button: the explicit request outranks the width rule
    // until the window is wide again (which re-arms the auto collapse).
    property bool userExpanded: false
    readonly property var appWindow: ApplicationWindow.window
    readonly property bool autoCollapsed: autoCollapse && appWindow !== null
        && appWindow.width < sidebarAutoCollapseWidth
    property bool collapsed: userCollapsed || (autoCollapsed && !userExpanded)
    onAutoCollapsedChanged: {
        if (!autoCollapsed)
            userExpanded = false
    }
    property string partnerName: AppState.partnerName
    property string partnerInitials: AppState.partnerInitials
    signal navigationRequested(string view)
    signal profileRequested()

    // Below this window width the rail gives the page channel its space back.
    readonly property int sidebarAutoCollapseWidth: 1220

    implicitWidth: collapsed ? Theme.shellSidebarCompactWidth : Theme.shellSidebarWidth
    implicitHeight: 520

    Behavior on implicitWidth {
        NumberAnimation {
            duration: Theme.duration(Theme.motionNormal)
            easing.type: Theme.animEasing
        }
    }

    Rectangle {
        anchors.fill: parent
        color: Theme.surface
        border.width: 1
        border.color: Theme.surfaceBorder
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.sp3
        spacing: Theme.sp3

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.sp2

            HarborLogo {
                showWordmark: !root.collapsed
                compact: root.collapsed
                Layout.fillWidth: !root.collapsed
            }

            HarborIconButton {
                visible: !root.collapsed
                iconName: "chevron-left"
                accessibleName: I18n.t("sidebar.collapse")
                toolTip: I18n.t("sidebar.collapse")
                buttonSize: 32
                Layout.alignment: Qt.AlignRight | Qt.AlignVCenter
                onClicked: root.userCollapsed = true
            }
        }

        HarborIconButton {
            objectName: "sidebarExpandButton"
            visible: root.collapsed
            iconName: "chevron-right"
            accessibleName: I18n.t("sidebar.expand")
            toolTip: I18n.t("sidebar.expand")
            buttonSize: 32
            Layout.alignment: Qt.AlignHCenter
            onClicked: {
                root.userCollapsed = false
                root.userExpanded = true
            }
        }

        ListView {
            id: navigationList

            model: root.model
            clip: true
            spacing: Theme.sp1
            Layout.fillWidth: true
            Layout.fillHeight: true
            boundsBehavior: Flickable.StopAtBounds

            ScrollBar.vertical: ScrollBar {
                policy: ScrollBar.AsNeeded
            }

            delegate: Button {
                id: navButton

                required property var modelData

                width: ListView.view ? ListView.view.width : 0
                implicitHeight: Theme.hitTarget
                checkable: true
                checked: root.currentView === String(navButton.modelData.key || "")
                hoverEnabled: true
                focusPolicy: Qt.StrongFocus
                Accessible.name: I18n.t(navButton.modelData.labelKey)
                Accessible.role: Accessible.Button
                Accessible.checked: checked

                contentItem: RowLayout {
                    spacing: Theme.sp3

                    // Fixed square slot; the glyph inside is always square
                    // and centered, so artwork keeps its aspect ratio.
                    Item {
                        Layout.preferredWidth: 24
                        Layout.preferredHeight: 24
                        Layout.alignment: Qt.AlignVCenter

                        HarborNavIcon {
                            anchors.centerIn: parent
                            name: String(navButton.modelData.icon || "activity")
                            active: navButton.checked
                            hovered: navButton.hovered
                            iconSize: 22
                        }
                    }

                    Text {
                        visible: !root.collapsed
                        text: I18n.t(navButton.modelData.labelKey)
                        color: navButton.checked ? Theme.textPrimary : Theme.textSecondary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontBody
                        font.weight: navButton.checked ? Font.DemiBold : Font.Normal
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                    }
                }

                background: Rectangle {
                    radius: Theme.radiusSmall
                    color: navButton.checked ? Theme.surfaceStrong
                         : navButton.hovered ? Theme.surface : "transparent"
                    border.width: navButton.visualFocus ? Theme.focusWidth : 0
                    border.color: Theme.focusRing
                }

                ToolTip.visible: root.collapsed && hovered
                ToolTip.text: I18n.t(navButton.modelData.labelKey)
                ToolTip.delay: 500

                onClicked: {
                    root.forceActiveFocus()
                    root.currentView = String(navButton.modelData.key || "")
                    root.navigationRequested(root.currentView)
                }
            }
        }

        Button {
            id: profileButton

            Layout.fillWidth: true
            implicitHeight: Theme.hitTargetLarge
            hoverEnabled: true
            focusPolicy: Qt.StrongFocus
            Accessible.name: I18n.t("sidebar.openProfile")
            Accessible.description: I18n.t("a11y.partnerPresence", {
                name: root.partnerName,
                status: I18n.t("common.status." + AppState.partnerState)
            })

            contentItem: RowLayout {
                spacing: Theme.sp2

                HarborAvatar {
                    avatarSize: 36
                    initials: root.partnerInitials
                    source: AppState.partnerProfile.avatar
                    avatarType: AppState.partnerProfile.avatarType
                    status: AppState.partnerState
                    accessibleName: I18n.t("a11y.avatarFor", { name: root.partnerName })
                }

                ColumnLayout {
                    visible: !root.collapsed
                    spacing: 0
                    Layout.fillWidth: true

                    Text {
                        text: root.partnerName
                        color: Theme.textPrimary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontSmall
                        font.weight: Font.DemiBold
                        elide: Text.ElideRight
                        Layout.fillWidth: true
                    }

                    Text {
                        text: I18n.t("common.status." + AppState.partnerState)
                        color: Theme.textFaint
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontTiny
                    }
                }
            }

            background: Rectangle {
                radius: Theme.radiusSmall
                color: profileButton.hovered ? Theme.surfaceStrong : "transparent"
                border.width: profileButton.visualFocus ? Theme.focusWidth : 0
                border.color: Theme.focusRing
            }

            onClicked: {
                root.forceActiveFocus()
                root.profileRequested()
            }
        }
    }
}
