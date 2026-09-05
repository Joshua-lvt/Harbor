// Mobile tab: MY phone — never the partner's. Four honest cards; each
// share toggle is intent, each permission line is platform fact. Sharing
// needs both. Unavailable APIs say so; nothing is invented.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"

ScrollView {
    id: view

    property bool batteryAvailable: false
    property int batteryPercent: 0
    property bool batteryCharging: false
    property string phoneActivity: "offline"
    property string currentApp: ""
    property string lastActiveText: ""
    property bool locationSharing: false
    property bool locationAvailable: false
    property string locationText: ""
    property string locationUpdatedText: ""
    property bool phoneNotificationsSharing: false
    property string phoneActivityPermission: "unknown"
    property string locationPermission: "unknown"
    property string notificationPermission: "unknown"
    property string ownNotificationPermission: "unknown"
    property string backgroundLocationPermission: "unknown"
    property string batteryOptimizationPermission: "unknown"
    // Optional shared theme; without one the view keeps its shipped colors.
    property var theme

    signal setShareLocation(bool on)
    signal setSharePhoneActivity(bool on)
    signal setSharePhoneNotifications(bool on)
    signal openSystemSettings(string page)
    signal requestOwnNotificationPermission()

    readonly property bool locationEffective: view.locationSharing
        && view.locationPermission === "granted"
    readonly property bool activityEffective: view.phoneActivitySharing
        && view.phoneActivityPermission === "granted"
    property bool phoneActivitySharing: false
    readonly property bool notificationsEffective: view.phoneNotificationsSharing
        && view.notificationPermission === "granted"

    function permissionText(state) {
        return state === "granted" ? qsTr("Permission granted")
            : state === "denied" ? qsTr("Permission not granted") : qsTr("Permission unknown")
    }

    ColumnLayout {
        width: view.availableWidth
        spacing: 12

        GroupBox {
            title: qsTr("📱 My phone")
            Layout.fillWidth: true
            ColumnLayout {
                Label {
                    text: view.batteryAvailable
                        ? (view.batteryCharging
                            ? qsTr("🔋 %1% — charging").arg(view.batteryPercent)
                            : qsTr("🔋 %1% — on battery").arg(view.batteryPercent))
                        : qsTr("🔋 Battery unavailable on this device")
                    color: view.theme ? view.theme.textPrimary : "#e6f2f7"
                }
                Label {
                    text: view.phoneActivity === "active"
                        ? (view.currentApp.length > 0
                            ? qsTr("Active — using %1").arg(view.currentApp)
                            : qsTr("Active"))
                        : view.phoneActivity === "idle" ? qsTr("Idle") : qsTr("Phone state unknown")
                    color: view.theme ? view.theme.textPrimary : "#e6f2f7"
                }
                Label {
                    text: view.lastActiveText
                    visible: view.lastActiveText.length > 0
                    color: view.theme ? view.theme.textSecondary : "#9db8c4"
                    opacity: 0.7
                    font.pixelSize: view.theme ? view.theme.fontSmall : 13
                }
            }
        }

        GroupBox {
            title: qsTr("📍 Location")
            Layout.fillWidth: true
            ColumnLayout {
                Switch {
                    text: qsTr("Share location")
                    checked: view.locationEffective
                    onToggled: view.setShareLocation(checked)
                }
                Label {
                    text: view.locationEffective
                        ? (view.locationAvailable ? view.locationText : qsTr("Waiting for a fix…"))
                        : view.locationSharing
                            ? qsTr("Waiting for location permission")
                            : qsTr("Off — no location leaves this phone")
                    color: view.theme ? view.theme.textPrimary : "#e6f2f7"
                }
                Label {
                    text: view.locationUpdatedText
                    visible: view.locationEffective && view.locationUpdatedText.length > 0
                    color: view.theme ? view.theme.textSecondary : "#9db8c4"
                    opacity: 0.7
                    font.pixelSize: view.theme ? view.theme.fontSmall : 13
                }
                Label {
                    text: view.permissionText(view.locationPermission)
                    color: view.theme ? view.theme.textSecondary : "#9db8c4"
                    opacity: 0.7
                    font.pixelSize: view.theme ? view.theme.fontSmall : 13
                }
                MobileButton {
                    text: qsTr("Open location settings")
                    theme: view.theme
                    visible: view.locationSharing && view.locationPermission !== "granted"
                    onClicked: view.openSystemSettings("location")
                }
                MobileButton {
                    text: qsTr("Allow background location")
                    theme: view.theme
                    visible: view.locationSharing
                        && view.locationPermission === "granted"
                        && view.backgroundLocationPermission !== "granted"
                    onClicked: view.openSystemSettings("background_location")
                }
            }
        }

        GroupBox {
            title: qsTr("📲 Phone activity")
            Layout.fillWidth: true
            ColumnLayout {
                Switch {
                    text: qsTr("Share phone activity")
                    checked: view.activityEffective
                    // The switch reflects effective sharing: intent AND grant.
                    onToggled: view.setSharePhoneActivity(checked)
                }
                Label {
                    text: view.permissionText(view.phoneActivityPermission)
                    color: view.theme ? view.theme.textSecondary : "#9db8c4"
                    opacity: 0.7
                    font.pixelSize: view.theme ? view.theme.fontSmall : 13
                }
                MobileButton {
                    text: qsTr("Open usage-access settings")
                    theme: view.theme
                    visible: view.phoneActivityPermission !== "granted"
                    onClicked: view.openSystemSettings("usage")
                }
            }
        }

        GroupBox {
            title: qsTr("🔔 Phone notifications")
            Layout.fillWidth: true
            ColumnLayout {
                Switch {
                    text: qsTr("Share phone notifications")
                    checked: view.notificationsEffective
                    onToggled: view.setSharePhoneNotifications(checked)
                }
                Label {
                    text: view.notificationsEffective
                        ? qsTr("Mirroring to your Harbor PC while enabled. Contents are never stored.")
                        : view.phoneNotificationsSharing
                            ? qsTr("Waiting for notification-access permission")
                            : qsTr("Off — nothing is observed")
                    color: view.theme ? view.theme.textPrimary : "#e6f2f7"
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }
                Label {
                    text: view.permissionText(view.notificationPermission)
                    color: view.theme ? view.theme.textSecondary : "#9db8c4"
                    opacity: 0.7
                    font.pixelSize: view.theme ? view.theme.fontSmall : 13
                }
                MobileButton {
                    text: qsTr("Open notification-access settings")
                    theme: view.theme
                    visible: view.notificationPermission !== "granted"
                    onClicked: view.openSystemSettings("notifications")
                }
                Label {
                    text: qsTr("Harbor alerts: %1").arg(view.permissionText(view.ownNotificationPermission))
                    color: view.theme ? view.theme.textSecondary : "#9db8c4"
                    opacity: 0.7
                    font.pixelSize: view.theme ? view.theme.fontSmall : 13
                }
                MobileButton {
                    text: qsTr("Allow Harbor notifications")
                    theme: view.theme
                    visible: view.ownNotificationPermission !== "granted"
                    onClicked: view.requestOwnNotificationPermission()
                }
            }
        }
    }
}
