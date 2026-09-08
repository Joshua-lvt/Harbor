import QtQuick
import QtQuick.Layouts
import Harbor 2.0

Item {
    id: root

    readonly property bool compact: width < Theme.breakpointCompact
    readonly property bool hasCore: typeof HarborCore !== "undefined"

    // Explicit home states. Each renders only its own data: unpaired never
    // borrows partner fields, and presence never borrows activity fields.
    readonly property string homeState: {
        if (!AppState.paired)
            return "NO_PARTNER"
        if (AppState.partnerState === "offline")
            return "PARTNER_OFFLINE"
        if (AppState.partnerState === "idle")
            return "PARTNER_IDLE"
        var latest = AppState.remoteActivities.length > 0
                     ? AppState.remoteActivities[0] : null
        if (latest && (latest.category === "game" || latest.category === "app")
                && String(latest.label || "").length > 0)
            return "PARTNER_IN_ACTIVITY"
        return "PARTNER_ONLINE"
    }

    readonly property var latestActivity: AppState.remoteActivities.length > 0
                                          ? AppState.remoteActivities[0] : null

    // Time-of-day greeting. The local hour drives the bucket (small hours
    // 0–4, morning 5–11, afternoon 12–17, evening 18–23) and a minute
    // cadence re-evaluates it while the page is up, so a session left open
    // across a boundary turns the greeting on its own.
    property int clockHour: new Date().getHours()
    readonly property string greetingKey: {
        if (clockHour >= 5 && clockHour < 12)
            return "home.greeting.morning"
        if (clockHour >= 12 && clockHour < 18)
            return "home.greeting.afternoon"
        if (clockHour >= 18)
            return "home.greeting.evening"
        return "home.greeting.lateNight"
    }

    Timer {
        interval: 60000
        repeat: true
        running: root.visible
        onTriggered: root.clockHour = new Date().getHours()
    }
    readonly property string partnerDisplayName: AppState.partnerName.length > 0
        ? AppState.partnerProfile.name : I18n.t("home.partner.unnamed")

    function presenceText() {
        switch (root.homeState) {
        case "PARTNER_OFFLINE":
            return I18n.t("home.state.offline", { name: root.partnerDisplayName })
        case "PARTNER_IDLE":
            return I18n.t("home.state.idle", { name: root.partnerDisplayName })
        case "PARTNER_IN_ACTIVITY": {
            var latest = root.latestActivity
            if (!latest)
                return I18n.t("home.partner.onlineWithYou")
            var label = String(latest.label || "")
            if (latest.category === "game")
                return I18n.t("home.state.playing", { name: root.partnerDisplayName, game: label })
            return I18n.t("home.state.using", { name: root.partnerDisplayName, app: label })
        }
        case "PARTNER_ONLINE":
            return I18n.t("home.partner.onlineWithYou")
        default:
            return ""
        }
    }

    function connectionTone() {
        if (!AppState.paired)
            return "neutral"
        if (AppState.connectionState === "connected") return "success"
        if (AppState.connectionState === "reconnecting") return "warning"
        if (AppState.connectionState === "connecting") return "accent"
        return "neutral"
    }

    function connectionText() {
        if (!AppState.paired)
            return I18n.t("home.connection.unpaired")
        return I18n.t("home.connection." + AppState.connectionState)
    }

    HarborStateLayer {
        anchors.fill: parent
        pageState: AppState.pageState("home")
        title: pageState === "error" ? I18n.t("state.error.title") : ""
        description: pageState === "error" ? I18n.t("state.error.description") : ""
        actionText: pageState === "error" ? I18n.t("common.actions.retry") : ""
        onActionTriggered: {
            if (root.hasCore)
                HarborCore.retryCore()
            else
                MockController.transitionPage("home", "content")
        }

        HarborPage {
            width: parent.width
            height: parent.height
            accessibleName: I18n.t("sidebar.home")

            HarborPageHeader {
                objectName: "homeHeader"
                eyebrow: I18n.t(root.greetingKey)
                title: I18n.t("home.hero.calm")
                subtitle: AppState.paired
                          ? I18n.t("home.connection." + AppState.connectionState)
                          : I18n.t("home.connection.unpaired")

                HarborBadge {
                    tone: root.connectionTone()
                    showDot: true
                    text: root.connectionText()
                }
            }

            // ---- NO_PARTNER: an intentional waiting space, not an error ----
            ColumnLayout {
                Layout.fillWidth: true
                Layout.maximumWidth: Theme.maxPageWidth
                visible: root.homeState === "NO_PARTNER"
                spacing: Theme.sp4

                Item { Layout.preferredHeight: Theme.sp4 }

                HarborLogo {
                    Layout.alignment: Qt.AlignHCenter
                    showWordmark: false
                    compact: true
                }

                Text {
                    Layout.fillWidth: true
                    text: I18n.t("home.unpaired.title")
                    color: Theme.textPrimary
                    font.family: Theme.fontFamilyDisplay
                    font.pixelSize: root.compact ? Theme.fontTitle : Theme.fontDisplay
                    font.weight: Font.Bold
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.WordWrap
                }

                Text {
                    Layout.fillWidth: true
                    text: I18n.t("home.unpaired.description")
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontBody
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                }

                HarborButton {
                    objectName: "homePairButton"
                    Layout.alignment: Qt.AlignHCenter
                    variant: "primary"
                    iconName: "phone"
                    text: I18n.t("gate.openPairing")
                    onClicked: AppState.openPairing()
                }

                HarborButton {
                    Layout.alignment: Qt.AlignHCenter
                    variant: "quiet"
                    text: I18n.t("onboarding.single.continueWithout")
                    onClicked: AppState.continueWithoutPairing()
                }

                Item { Layout.preferredHeight: Theme.sp4 }
            }

            // ---- Partner space: only with a real paired partner ------------
            HarborSectionCard {
                Layout.fillWidth: true
                Layout.maximumWidth: Theme.maxPageWidth
                visible: root.homeState !== "NO_PARTNER"

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.sp5

                    HarborAvatar {
                        avatarSize: root.compact ? 92 : 132
                        initials: AppState.partnerProfile.initials
                        source: AppState.partnerProfile.avatar
                        avatarType: AppState.partnerProfile.avatarType
                        status: AppState.partnerState
                        speaking: AppState.callState === "connected" && !!AppState.remoteSpeaking
                        accessibleName: I18n.t("a11y.avatarFor", { name: root.partnerDisplayName })
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        Text {
                            Layout.fillWidth: true
                            text: root.partnerDisplayName
                            color: Theme.textPrimary
                            font.family: Theme.fontFamilyDisplay
                            font.pixelSize: root.compact ? Theme.fontTitle : Theme.fontDisplay
                            font.weight: Font.Bold
                            wrapMode: Text.WordWrap
                        }

                        Text {
                            Layout.fillWidth: true
                            text: root.presenceText()
                            color: AppState.partnerState === "online" ? Theme.success : Theme.textSecondary
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontBody
                            wrapMode: Text.WordWrap
                        }

                        Text {
                            Layout.fillWidth: true
                            visible: AppState.partnerProfile.status.length > 0
                            text: AppState.partnerProfile.status
                            color: Theme.textSecondary
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontSmall
                            wrapMode: Text.WordWrap
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            HarborButton {
                                objectName: "homeCallButton"
                                iconName: "mic"
                                text: AppState.partnerName.length > 0
                                      ? I18n.t("home.call.start", { name: root.partnerDisplayName })
                                      : I18n.t("home.call.open")
                                onClicked: AppState.navigate("call")
                            }

                            HarborButton {
                                objectName: "homeChatButton"
                                variant: "secondary"
                                iconName: "chat"
                                text: I18n.t("home.chat.open")
                                onClicked: AppState.navigate("chat")
                            }
                        }
                    }
                }

                Rectangle {
                    Layout.fillWidth: true
                    implicitHeight: 1
                    color: Theme.divider
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.sp3

                    HarborIcon {
                        name: "activity"
                        color: Theme.accent
                        implicitWidth: 20
                        implicitHeight: 20
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0

                        Text {
                            text: I18n.t("home.activity.title", { name: root.partnerDisplayName })
                            color: Theme.textFaint
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontTiny
                        }

                        Text {
                            Layout.fillWidth: true
                            text: AppState.remoteActivities.length > 0
                                  ? AppState.remoteActivities[0].label
                                  : I18n.t("home.activity.empty")
                            color: Theme.textPrimary
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontBody
                            font.weight: Font.DemiBold
                            elide: Text.ElideRight
                        }
                    }

                    HarborButton {
                        variant: "quiet"
                        text: I18n.t("common.actions.viewAll")
                        onClicked: AppState.navigate("activity")
                    }
                }
            }
        }
    }
}
