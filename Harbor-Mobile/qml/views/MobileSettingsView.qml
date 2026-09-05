// Mobile Settings: Harbor toggles plus permission states. No tech detail,
// just intent switches and where to grant them.
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"

ScrollView {
    id: view

    property bool persistentCall: true
    property bool locationSharing: false
    property bool phoneActivitySharing: false
    property bool phoneNotificationsSharing: false
    // Appearance (durable core settings, same meaning as the desktop shell).
    property string appearanceMode: "dark"
    property string accentColor: "ocean"
    property real accentIntensity: 0.75
    property string oceanVariant: "lagoon"
    property string cornerRadius: "soft"
    property string density: "comfortable"
    property bool higherContrast: false
    property bool reducedMotion: false
    property real animationIntensity: 1.0
    property string updateStatus: "idle"
    property string updateError: ""
    // Optional shared theme; without one the view keeps its shipped colors.
    property var theme
    property string phoneActivityPermission: "unknown"
    property string locationPermission: "unknown"
    property string notificationPermission: "unknown"
    property string ownNotificationPermission: "unknown"
    property string backgroundLocationPermission: "unknown"
    property string batteryOptimizationPermission: "unknown"

    signal setPersistentCall(bool on)
    signal setAppearanceMode(string mode)
    signal setAccentColor(string color)
    signal setAccentIntensity(real value)
    signal setOceanVariant(string variant)
    signal setCornerRadius(string value)
    signal setDensity(string value)
    signal setHigherContrast(bool on)
    signal setReducedMotion(bool on)
    signal setAnimationIntensity(real value)
    signal checkForUpdates()
    signal setShareLocation(bool on)
    signal setSharePhoneActivity(bool on)
    signal setSharePhoneNotifications(bool on)
    signal openSystemSettings(string page)
    signal requestOwnNotificationPermission()

    readonly property bool locationEffective: view.locationSharing
        && view.locationPermission === "granted"
    readonly property bool activityEffective: view.phoneActivitySharing
        && view.phoneActivityPermission === "granted"
    readonly property bool notificationsEffective: view.phoneNotificationsSharing
        && view.notificationPermission === "granted"

    ColumnLayout {
        width: view.availableWidth
        spacing: 12

        GroupBox {
            title: qsTr("Call")
            Layout.fillWidth: true
            CheckBox {
                text: qsTr("Persistent call")
                checked: view.persistentCall
                onToggled: view.setPersistentCall(checked)
            }
        }

        GroupBox {
            title: qsTr("App updates")
            Layout.fillWidth: true
            ColumnLayout {
                Label {
                    text: view.updateStatus === "error" && view.updateError.length > 0
                        ? view.updateError : qsTr("New releases install by themselves.")
                    wrapMode: Text.WordWrap
                    Layout.fillWidth: true
                }
                MobileButton {
                    text: qsTr("Check for updates")
                    theme: view.theme
                    Layout.fillWidth: true
                    Layout.preferredHeight: 48
                    onClicked: view.checkForUpdates()
                }
            }
        }

        GroupBox {
            title: qsTr("Appearance")
            Layout.fillWidth: true
            ColumnLayout {
                // Bound to the group width: rows share it instead of
                // sizing to their implicit total and spilling off-screen.
                width: parent ? parent.width : undefined
                spacing: 10

                Label {
                    text: qsTr("Mode")
                    color: view.theme ? view.theme.textSecondary : "#9db8c4"
                    font.pixelSize: view.theme ? view.theme.fontSmall : 13
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8
                    Repeater {
                        model: ["dark", "light", "system"]
                        MobileButton {
                            required property string modelData
                            text: modelData === "dark" ? qsTr("Dark")
                                : modelData === "light" ? qsTr("Light") : qsTr("System")
                            theme: view.theme
                            primary: view.appearanceMode === modelData
                            Layout.fillWidth: true
                            Layout.minimumWidth: 0
                            Layout.preferredHeight: 44
                            onClicked: view.setAppearanceMode(modelData)
                        }
                    }
                }

                Label {
                    text: qsTr("Accent")
                    color: view.theme ? view.theme.textSecondary : "#9db8c4"
                    font.pixelSize: view.theme ? view.theme.fontSmall : 13
                }
                Flow {
                    Layout.fillWidth: true
                    spacing: 10
                    Repeater {
                        model: view.theme ? view.theme.accentPresetList : []
                        Rectangle {
                            required property var modelData
                            width: 44
                            height: 36
                            radius: view.theme ? view.theme.radiusSmall : 8
                            color: modelData.color
                            border.width: view.accentColor === modelData.key ? 3 : 1
                            border.color: view.accentColor === modelData.key
                                ? (view.theme ? view.theme.textPrimary : "#e6f2f7") : "#2f4f60"
                            // MouseArea, not TapHandler: tap handlers lose the
                            // touch grab to the surrounding flickable, while
                            // the press-and-release click delivery works.
                            MouseArea {
                                anchors.fill: parent
                                onClicked: view.setAccentColor(parent.modelData.key)
                            }
                        }
                    }
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8
                    TextField {
                        id: customAccentField
                        Layout.fillWidth: true
                        Layout.preferredHeight: 48
                        placeholderText: qsTr("#RRGGBB")
                        maximumLength: 7
                    }
                    MobileButton {
                        text: qsTr("Apply")
                        theme: view.theme
                        Layout.preferredHeight: 48
                        enabled: /^#[0-9a-fA-F]{6}$/.test(customAccentField.text)
                        onClicked: {
                            view.setAccentColor(customAccentField.text)
                            customAccentField.text = ""
                        }
                    }
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8
                    Label {
                        text: qsTr("Strength %1%").arg(Math.round(view.accentIntensity * 100))
                        color: view.theme ? view.theme.textSecondary : "#9db8c4"
                        font.pixelSize: view.theme ? view.theme.fontSmall : 13
                        Layout.preferredWidth: 110
                    }
                    Slider {
                        Layout.fillWidth: true
                        from: 15
                        to: 100
                        stepSize: 5
                        value: Math.round(view.accentIntensity * 100)
                        onMoved: view.setAccentIntensity(Math.max(0.15, Math.min(1, value / 100)))
                    }
                }

                Label {
                    text: qsTr("Ocean background")
                    color: view.theme ? view.theme.textSecondary : "#9db8c4"
                    font.pixelSize: view.theme ? view.theme.fontSmall : 13
                }
                Flow {
                    Layout.fillWidth: true
                    spacing: 8
                    Repeater {
                        model: view.theme ? view.theme.oceanVariantList : []
                        Rectangle {
                            required property string modelData
                            width: 84
                            height: 40
                            radius: view.theme ? view.theme.radiusSmall : 8
                            color: view.theme ? view.theme.oceanTop(modelData) : "#0a1a24"
                            border.width: view.oceanVariant === modelData ? 3 : 1
                            border.color: view.oceanVariant === modelData
                                ? (view.theme ? view.theme.accent : "#4ade80") : "#2f4f60"
                            Label {
                                anchors.centerIn: parent
                                text: parent.modelData.charAt(0).toUpperCase() + parent.modelData.slice(1)
                                color: view.theme ? view.theme.textPrimary : "#e6f2f7"
                                font.pixelSize: 12
                            }
                            // MouseArea, not TapHandler: tap handlers lose the
                            // touch grab to the surrounding flickable, while
                            // the press-and-release click delivery works.
                            MouseArea {
                                anchors.fill: parent
                                onClicked: view.setOceanVariant(parent.modelData)
                            }
                        }
                    }
                }

                Label {
                    text: qsTr("Corners")
                    color: view.theme ? view.theme.textSecondary : "#9db8c4"
                    font.pixelSize: view.theme ? view.theme.fontSmall : 13
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8
                    Repeater {
                        model: ["soft", "medium"]
                        MobileButton {
                            required property string modelData
                            text: modelData === "soft" ? qsTr("Soft") : qsTr("Medium")
                            theme: view.theme
                            primary: view.cornerRadius === modelData
                            Layout.fillWidth: true
                            Layout.minimumWidth: 0
                            Layout.preferredHeight: 44
                            onClicked: view.setCornerRadius(modelData)
                        }
                    }
                }

                Label {
                    text: qsTr("Density")
                    color: view.theme ? view.theme.textSecondary : "#9db8c4"
                    font.pixelSize: view.theme ? view.theme.fontSmall : 13
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8
                    Repeater {
                        model: ["comfortable", "compact"]
                        MobileButton {
                            required property string modelData
                            text: modelData === "comfortable" ? qsTr("Comfortable") : qsTr("Compact")
                            theme: view.theme
                            primary: view.density === modelData
                            Layout.fillWidth: true
                            Layout.minimumWidth: 0
                            Layout.preferredHeight: 44
                            onClicked: view.setDensity(modelData)
                        }
                    }
                }

                Switch {
                    text: qsTr("Higher contrast")
                    checked: view.higherContrast
                    onToggled: view.setHigherContrast(checked)
                }
                Switch {
                    text: qsTr("Reduce motion")
                    checked: view.reducedMotion
                    onToggled: view.setReducedMotion(checked)
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8
                    Label {
                        text: qsTr("Animation %1%").arg(Math.round(view.animationIntensity * 100))
                        color: view.theme ? view.theme.textSecondary : "#9db8c4"
                        font.pixelSize: view.theme ? view.theme.fontSmall : 13
                        Layout.preferredWidth: 110
                    }
                    Slider {
                        Layout.fillWidth: true
                        from: 0
                        to: 100
                        stepSize: 5
                        value: Math.round(view.animationIntensity * 100)
                        onMoved: view.setAnimationIntensity(Math.max(0, Math.min(1, value / 100)))
                    }
                }
            }
        }

        GroupBox {
            title: qsTr("Phone sharing")
            Layout.fillWidth: true
            ColumnLayout {
                CheckBox {
                    text: qsTr("Share location")
                    checked: view.locationEffective
                    onToggled: view.setShareLocation(checked)
                }
                CheckBox {
                    text: qsTr("Share phone activity")
                    checked: view.activityEffective
                    onToggled: view.setSharePhoneActivity(checked)
                }
                CheckBox {
                    text: qsTr("Share phone notifications")
                    checked: view.notificationsEffective
                    onToggled: view.setSharePhoneNotifications(checked)
                }
            }
        }

        GroupBox {
            title: qsTr("Permissions")
            Layout.fillWidth: true
            ColumnLayout {
                MobileButton {
                    theme: view.theme
                    text: qsTr("Location access")
                    Layout.fillWidth: true
                    onClicked: view.openSystemSettings("location")
                }
                MobileButton {
                    theme: view.theme
                    text: qsTr("Usage access")
                    Layout.fillWidth: true
                    onClicked: view.openSystemSettings("usage")
                }
                MobileButton {
                    theme: view.theme
                    text: qsTr("Notification access")
                    Layout.fillWidth: true
                    onClicked: view.openSystemSettings("notifications")
                }
                MobileButton {
                    theme: view.theme
                    text: qsTr("Background location")
                    visible: view.locationPermission === "granted" && view.backgroundLocationPermission !== "granted"
                    Layout.fillWidth: true
                    onClicked: view.openSystemSettings("background_location")
                }
                MobileButton {
                    theme: view.theme
                    text: qsTr("Allow Harbor notifications")
                    visible: view.ownNotificationPermission !== "granted"
                    Layout.fillWidth: true
                    onClicked: view.requestOwnNotificationPermission()
                }
                MobileButton {
                    theme: view.theme
                    text: qsTr("Battery optimization")
                    visible: view.batteryOptimizationPermission !== "granted"
                    Layout.fillWidth: true
                    onClicked: view.openSystemSettings("battery")
                }
            }
        }
    }
}
