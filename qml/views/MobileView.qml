pragma ComponentBehavior: Bound

import Harbor 2.0
import QtQuick
import QtQuick.Layouts

// Partner's phone: the shared MobileStatus the paired peer's phone publishes
// through the core, rendered as four honest cards — battery, activity, map,
// mirrored notifications. Everything here is session-only: a side that stops
// sharing returns its card to the matching empty state, and losing the core
// clears the whole page instead of leaving a stale battery or location.
//
// Same contract, two sources: the supervised core's real snapshot while it
// is ready, AppState's deterministic fixtures otherwise.
HarborStateLayer {
    id: root

    readonly property bool hasCore: typeof HarborCore !== "undefined"
    readonly property var peer: AppState.peerPhone || {}
    readonly property bool sharing: AppState.peerPhone !== null && AppState.peerPhone !== undefined
    readonly property bool paired: AppState.paired
    property bool copyFeedbackVisible: false

    // Paired devices always see the page: per-card honest empty states plus
    // the connect-phone card when nothing is shared yet. Only the unpaired
    // gate hides the content.
    pageState: !root.paired ? "empty" : "content"
    title: !root.paired ? I18n.t("mobile.empty.title") : I18n.t("mobile.unpaired.title")
    description: !root.paired ? I18n.t("mobile.empty.description") : I18n.t("mobile.unpaired.description")
    iconName: "phone"
    actionText: !root.paired ? I18n.t("mobile.empty.action") : ""
    onActionTriggered: AppState.openPairing()

    function peerName() {
        return AppState.partnerName.length > 0 ? AppState.partnerName : I18n.t("home.partner.unnamed")
    }

    function formatClock(secondsSinceEpoch) {
        var date = new Date((Number(secondsSinceEpoch) || 0) * 1000)
        var hours = date.getHours()
        var minutes = date.getMinutes()
        return (hours < 10 ? "0" : "") + hours + ":" + (minutes < 10 ? "0" : "") + minutes
    }

    function agoText(secondsSinceEpoch) {
        var at = Number(secondsSinceEpoch) || 0
        if (at <= 0)
            return ""
        var age = Date.now() / 1000 - at
        if (age < 45)
            return I18n.t("common.time.now")
        if (age < 90)
            return I18n.t("common.time.secondsAgo", { count: Math.round(age) })
        var minutes = Math.floor(age / 60)
        if (minutes < 60)
            return I18n.t("common.time.minutesAgo", { count: minutes })
        return I18n.t("common.time.hoursAgo", { count: Math.floor(minutes / 60) })
    }

    function activityKey() {
        switch (String(root.peer.phoneActivity || "OFFLINE")) {
        case "ACTIVE": return "mobile.activity.active"
        case "IDLE": return "mobile.activity.idle"
        default: return "mobile.activity.offline"
        }
    }

    function coordinatesText() {
        var fix = root.peer.location
        if (!fix)
            return ""
        return Number(fix.latitude).toFixed(5) + ", " + Number(fix.longitude).toFixed(5)
    }

    function copyCoordinates() {
        var text = root.coordinatesText()
        if (text.length === 0)
            return
        // qmllint disable unqualified
        if (root.hasCore)
            HarborCore.copyToClipboard(text)
        else
            MockController.mockCopy(text, "coordinates")
        // qmllint enable unqualified
        root.copyFeedbackVisible = true
        copyFeedbackTimer.restart()
    }

    Timer {
        id: copyFeedbackTimer

        interval: 1600
        onTriggered: root.copyFeedbackVisible = false
    }

    HarborPage {
        id: page

        width: parent.width
        height: parent.height
        accessibleName: I18n.t("mobile.title")

        HarborPageHeader {
            title: I18n.t("mobile.title")
            subtitle: I18n.t("mobile.subtitle", { name: root.peerName() })
            iconName: "phone"
        }

        // ---- Connect your phone -------------------------------------------
        // Linking is guidance, not protocol: install Harbor on Android and
        // pair it with the same six-digit code. A phone only ever works
        // alongside a PC — phone-to-phone is refused by the core.
        HarborSectionCard {
            Layout.fillWidth: true
            Layout.maximumWidth: Theme.maxPageWidth
            visible: !root.sharing
            title: I18n.t("mobile.connect.title")
            iconName: "phone"

            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2

                Text {
                    text: I18n.t("mobile.connect.description")
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontBody
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }

                HarborButton {
                    text: I18n.t("mobile.connect.action")
                    variant: "secondary"
                    onClicked: AppState.openPairing()
                }

                Text {
                    text: I18n.t("mobile.rulesNote")
                    color: Theme.textFaint
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontSmall
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }
            }
        }

        // ---- Battery ------------------------------------------------------
        HarborSectionCard {
            Layout.fillWidth: true
            Layout.maximumWidth: Theme.maxPageWidth
            title: I18n.t("mobile.battery.title")
            iconName: "info"

            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.sp3
                    visible: root.peer.batteryPercent !== null && root.peer.batteryPercent !== undefined

                    Text {
                        text: root.peer.batteryPercent + "%"
                        color: Theme.textPrimary
                        font.family: Theme.fontFamilyDisplay
                        font.pixelSize: Theme.fontDisplay
                        font.weight: Font.Bold
                        Accessible.ignored: true
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp1

                        Text {
                            text: root.peer.charging === true
                                ? I18n.t("mobile.battery.charging")
                                : I18n.t("mobile.battery.onBattery")
                            color: Theme.textSecondary
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontBody
                        }

                        // Level bar: width carries the value alongside text
                        // and color, so nothing depends on color alone.
                        Rectangle {
                            Layout.fillWidth: true
                            Layout.preferredHeight: 8
                            radius: 4
                            color: Theme.surfaceSunken
                            Accessible.ignored: true

                            Rectangle {
                                anchors.top: parent.top
                                anchors.bottom: parent.bottom
                                anchors.left: parent.left
                                width: parent.width * Math.max(0, Math.min(1, Number(root.peer.batteryPercent || 0) / 100))
                                radius: 4
                                color: Number(root.peer.batteryPercent) <= 15 ? Theme.danger : Theme.accent
                            }
                        }
                    }
                }

                Text {
                    visible: root.peer.batteryPercent === null || root.peer.batteryPercent === undefined
                    text: I18n.t("mobile.battery.unavailable")
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontBody
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }
            }
        }

        // ---- Phone activity -------------------------------------------------
        HarborSectionCard {
            Layout.fillWidth: true
            Layout.maximumWidth: Theme.maxPageWidth
            title: I18n.t("mobile.activity.title")
            iconName: "clock"

            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.sp2

                    // Shape carries the state alongside color and text:
                    // filled (active), ring (idle), faint dot (offline).
                    Item {
                        Layout.preferredWidth: 10
                        Layout.preferredHeight: 10
                        Layout.alignment: Qt.AlignVCenter
                        Accessible.ignored: true

                        Rectangle {
                            anchors.fill: parent
                            radius: width / 2
                            color: String(root.peer.phoneActivity) === "ACTIVE" ? Theme.online : "transparent"
                            border.width: String(root.peer.phoneActivity) === "ACTIVE" ? 0 : 2
                            border.color: String(root.peer.phoneActivity) === "IDLE" ? Theme.idle : Theme.offline
                            opacity: String(root.peer.phoneActivity) === "OFFLINE" ? 0.55 : 1
                        }
                    }

                    Text {
                        text: I18n.t(root.activityKey())
                        color: String(root.peer.phoneActivity) === "ACTIVE" ? Theme.success : Theme.textSecondary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontBody
                        font.weight: Font.DemiBold
                        Layout.fillWidth: true
                    }
                }

                Text {
                    visible: String(root.peer.currentApp || "").length > 0
                    text: I18n.t("mobile.activity.using", { app: String(root.peer.currentApp || "") })
                    color: Theme.textPrimary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontBody
                    font.weight: Font.DemiBold
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }

                Text {
                    visible: Number(root.peer.lastActiveAt) > 0
                    text: I18n.t("mobile.activity.lastSeen", {
                        time: root.formatClock(root.peer.lastActiveAt),
                        ago: root.agoText(root.peer.lastActiveAt)
                    })
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontSmall
                    Layout.fillWidth: true
                }

                Text {
                    visible: String(root.peer.phoneActivity || "OFFLINE") === "OFFLINE"
                             && String(root.peer.currentApp || "").length === 0
                             && !(Number(root.peer.lastActiveAt) > 0)
                    text: I18n.t("mobile.activity.unknown")
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontBody
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }
            }
        }

        // ---- Location ---------------------------------------------------------
        HarborSectionCard {
            Layout.fillWidth: true
            Layout.maximumWidth: Theme.maxPageWidth
            title: I18n.t("mobile.location.title")
            iconName: "info"

            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.sp3

                Text {
                    visible: root.peer.locationSharingEnabled !== true
                    text: I18n.t("mobile.location.off")
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontBody
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }

                RowLayout {
                    visible: root.peer.locationSharingEnabled === true && !root.peer.location
                    Layout.fillWidth: true
                    spacing: Theme.sp3

                    HarborSpinner {
                        spinnerSize: 24
                        accessibleName: I18n.t("mobile.location.waiting")
                        Layout.alignment: Qt.AlignVCenter
                    }

                    Text {
                        text: I18n.t("mobile.location.waiting")
                        color: Theme.textSecondary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontBody
                        wrapMode: Text.Wrap
                        Layout.fillWidth: true
                    }
                }

                ColumnLayout {
                    visible: root.peer.locationSharingEnabled === true && root.peer.location
                    Layout.fillWidth: true
                    spacing: Theme.sp2

                    HarborPhoneMap {
                        latitude: root.peer.location ? Number(root.peer.location.latitude) : 0
                        longitude: root.peer.location ? Number(root.peer.location.longitude) : 0
                        accuracyMeters: root.peer.location ? Number(root.peer.location.accuracyMeters) : -1
                        Layout.fillWidth: true
                        Layout.preferredHeight: 260
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp3

                        Text {
                            text: root.coordinatesText()
                            color: Theme.textPrimary
                            font.family: Theme.fontFamilyMonospace
                            font.pixelSize: Theme.fontBody
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }

                        HarborButton {
                            objectName: "copyCoordinatesButton"
                            text: root.copyFeedbackVisible ? I18n.t("mobile.location.copied")
                                                          : I18n.t("mobile.location.copy")
                            variant: "secondary"
                            onClicked: root.copyCoordinates()
                        }
                    }

                    Text {
                        text: root.peer.location ? I18n.t("mobile.location.accuracy", {
                            value: I18n.unit(Math.round(Number(root.peer.location.accuracyMeters)), "m")
                        }) : ""
                        color: Theme.textSecondary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontSmall
                        Layout.fillWidth: true
                    }

                    Text {
                        text: root.peer.location && Number(root.peer.location.updatedAt) > 0
                            ? I18n.t("mobile.location.updated", {
                                time: root.formatClock(root.peer.location.updatedAt),
                                ago: root.agoText(root.peer.location.updatedAt)
                            })
                            : I18n.t("mobile.location.updatedUnknown")
                        color: Theme.textMuted
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontSmall
                        Layout.fillWidth: true
                    }
                }
            }
        }

        // ---- Mirrored notifications -------------------------------------------
        HarborSectionCard {
            Layout.fillWidth: true
            Layout.maximumWidth: Theme.maxPageWidth
            title: I18n.t("mobile.notifications.title")
            iconName: "chat"

            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.sp2

                Text {
                    visible: root.peer.notificationSharingEnabled !== true
                    text: I18n.t("mobile.notifications.off")
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontBody
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }

                Text {
                    visible: root.peer.notificationSharingEnabled === true && AppState.phoneNotices.length === 0
                    text: I18n.t("mobile.notifications.empty")
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontBody
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }

                Repeater {
                    model: root.peer.notificationSharingEnabled === true ? AppState.phoneNotices : []

                    ColumnLayout {
                        required property var modelData

                        Layout.fillWidth: true
                        spacing: 2

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            Text {
                                text: String(modelData.appLabel || "")
                                color: Theme.textPrimary
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontBody
                                font.weight: Font.DemiBold
                                elide: Text.ElideRight
                                Layout.fillWidth: true
                            }

                            Text {
                                text: root.formatClock(modelData.at)
                                color: Theme.textFaint
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontSmall
                            }
                        }

                        Text {
                            visible: String(modelData.title || "").length > 0
                            text: String(modelData.title || "")
                            color: Theme.textPrimary
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontSmall
                            font.weight: Font.DemiBold
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }

                        Text {
                            visible: String(modelData.text || "").length > 0
                            text: String(modelData.text || "")
                            color: Theme.textSecondary
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontSmall
                            wrapMode: Text.Wrap
                            maximumLineCount: 3
                            elide: Text.ElideRight
                            Layout.fillWidth: true
                        }

                        Rectangle {
                            Layout.fillWidth: true
                            Layout.topMargin: Theme.sp2
                            implicitHeight: 1
                            color: Theme.divider
                            Accessible.ignored: true
                        }
                    }
                }

                Text {
                    visible: root.peer.notificationSharingEnabled === true
                    text: I18n.t("mobile.notifications.note")
                    color: Theme.textFaint
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontTiny
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }
            }
        }

        Text {
            Layout.fillWidth: true
            Layout.maximumWidth: Theme.maxPageWidth
            text: I18n.t("mobile.privacyNote", { name: root.peerName() })
            color: Theme.textFaint
            font.family: Theme.fontFamily
            font.pixelSize: Theme.fontSmall
            wrapMode: Text.Wrap
        }
    }
}
