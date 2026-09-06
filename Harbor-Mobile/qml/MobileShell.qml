// Harbor Mobile shell: touch-first bottom navigation.
//
// Standalone-verifiable: only QtQuick/Controls/Layouts imports. The host
// (the `harbor-mobile` CMake target, desktop preview or Android APK)
// injects live state through the root properties below, bound to
// AppState, the typed facade bridges, and the Android C++ adapters.
// Nothing here invents state: every view renders its injected props and
// honest empty/unavailable fallbacks.
//
// Localization: qsTr() catalogs (lupdate-ready). The desktop shell uses
// the Harbor I18n singleton; the mobile module keeps standard qsTr so it
// lints and ships without the desktop module.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Controls.Material
import QtQuick.Layouts
import HarborMobile
import "components"

ApplicationWindow {
    id: shell
    visible: true
    width: 412
    height: 892
    title: qsTr("Harbor")
    // Set the native window clear color explicitly.  ApplicationWindow's
    // Controls background is a separate style surface on Android and, with
    // Qt's Android renderer, could leave its default surface visible as a
    // diagonal half-screen during composition.
    color: appTheme.background
    Material.theme: appTheme.themeDark ? Material.Dark : Material.Light
    Material.primary: appTheme.bar
    Material.accent: appTheme.accent
    Material.background: appTheme.background
    Material.foreground: appTheme.textPrimary

    // ---- Injected Harbor state (host binds these) ----
    property string partnerName: ""
    // "online" | "idle" | "offline" — the committed aggregate only.
    property string partnerState: "offline"
    property string partnerActivity: ""
    property string partnerAvatar: ""
    property bool sessionValid: false
    property bool isCompanion: false
    property var chatMessages: []
    property bool chatConnected: false
    // "idle" | "connecting" | "connected" | "incoming" | "unavailable"
    property string callState: "idle"
    property bool microphoneMuted: true
    property string microphonePermission: "unknown"
    property bool persistentCall: true
    property bool batteryAvailable: false
    property int batteryPercent: 0
    property bool batteryCharging: false
    property string phoneActivity: "offline" // "active" | "idle" | "offline"
    property string currentApp: ""
    property string lastActiveText: ""
    property double lastActiveAt: 0
    property bool locationSharing: false
    property bool locationAvailable: false
    property string locationText: ""
    property string locationUpdatedText: ""
    property double locationLatitude: 0
    property double locationLongitude: 0
    property double locationAccuracyMeters: -1
    property double locationUpdatedAt: 0
    property bool phoneNotificationsSharing: false
    property bool phoneActivitySharing: false
    property string phoneActivityPermission: "unknown" // "granted" | "denied" | "unknown"
    property string locationPermission: "unknown"
    property string notificationPermission: "unknown"
    property string ownNotificationPermission: "unknown"
    property string backgroundLocationPermission: "unknown"
    property string batteryOptimizationPermission: "unknown"
    property bool mobileToMobileBlocked: false
    property bool pairingVisible: false
    property bool serverConfigured: false
    // Tailnet gate mirror (host binds the platform reading). True by
    // default so the standalone shell never blocks pairing in previews.
    property bool tailscaleInstalled: true

    // ---- Mandatory updates (blocking once discovered) ----
    property string updateStatus: "idle"
    property string updateVersion: ""
    property real updateProgress: 0
    property string updateError: ""

    signal retryUpdate()
    signal installUpdate()
    signal checkForUpdates()

    // ---- Appearance (durable core settings, same meaning as desktop) ----
    property string appearanceMode: "dark"
    property string accentColor: "ocean"
    property real accentIntensity: 0.75
    property string oceanVariant: "lagoon"
    property string cornerRadius: "soft"
    property string density: "comfortable"
    property bool higherContrast: false
    property bool reducedMotion: false
    property real animationIntensity: 1.0

    signal setAppearanceMode(string mode)
    signal setAccentColor(string color)
    signal setAccentIntensity(real value)
    signal setOceanVariant(string variant)
    signal setCornerRadius(string value)
    signal setDensity(string value)
    signal setHigherContrast(bool on)
    signal setReducedMotion(bool on)
    signal setAnimationIntensity(real value)

    MobileTheme {
        id: appTheme
        appearanceMode: shell.appearanceMode
        systemDark: Application.styleHints.colorScheme === Qt.Dark
        accentColor: shell.accentColor
        accentIntensity: shell.accentIntensity
        oceanVariant: shell.oceanVariant
        cornerRadius: shell.cornerRadius
        density: shell.density
        higherContrast: shell.higherContrast
        reducedMotion: shell.reducedMotion
        animationIntensity: shell.animationIntensity
    }

    // ---- Outbound intents (host connects to facade/core) ----
    signal sendChat(string body)
    signal setMicMuted(bool muted)
    signal enterCall()
    signal acceptIncomingCall()
    signal declineIncomingCall()
    signal leaveCall()
    signal requestTakeover()
    signal setPersistentCall(bool on)
    signal setShareLocation(bool on)
    signal setSharePhoneActivity(bool on)
    signal setSharePhoneNotifications(bool on)
    signal openChat()
    signal openSystemSettings(string page)
    signal requestOwnNotificationPermission()
    signal openPairing()
    signal closePairing()
    signal createPairingCode()
    signal submitPairingCode(string code)
    signal copyPairingCode(string code)
    signal openTailscaleStore()
    signal acceptPairing()
    signal declinePairing()
    signal cancelPairing()
    signal resetPairing()
    signal pollPairingStatus()
    signal pollPairingIncoming()

    property string pairingCode: ""
    property string pairingPhase: ""
    property string pairingError: ""
    property string pairingRole: "peer"
    property bool pairingBusy: false

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        MobilePresenceBar {
            Layout.fillWidth: true
            theme: appTheme
            partnerName: shell.partnerName
            partnerState: shell.partnerState
            partnerActivity: shell.partnerActivity
            avatarSource: shell.partnerAvatar
            blocked: shell.mobileToMobileBlocked
            onOpenChat: { shell.openChat(); nav.currentIndex = 1 }
        }

        StackLayout {
            id: pages
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.margins: 16
            currentIndex: nav.currentIndex

            MobileHomeView {
                theme: appTheme
                sessionValid: shell.sessionValid
                partnerName: shell.partnerName
                partnerState: shell.partnerState
                partnerActivity: shell.partnerActivity
                partnerAvatar: shell.partnerAvatar
                isCompanion: shell.isCompanion
                callState: shell.callState
                blocked: shell.mobileToMobileBlocked
                onOpenChat: { shell.openChat(); nav.currentIndex = 1 }
                onEnterCall: shell.enterCall()
                onRequestTakeover: shell.requestTakeover()
                onOpenPairing: shell.openPairing()
            }
            MobileChatView {
                theme: appTheme
                messages: shell.chatMessages
                connected: shell.chatConnected
                blocked: shell.mobileToMobileBlocked
                onSendChat: body => shell.sendChat(body)
            }
            MobileCallView {
                theme: appTheme
                callState: shell.callState
                microphoneMuted: shell.microphoneMuted
                microphonePermission: shell.microphonePermission
                persistentCall: shell.persistentCall
                isCompanion: shell.isCompanion
                onSetMicMuted: muted => shell.setMicMuted(muted)
                onEnterCall: shell.enterCall()
                onAcceptIncomingCall: shell.acceptIncomingCall()
                onDeclineIncomingCall: shell.declineIncomingCall()
                onLeaveCall: shell.leaveCall()
                onRequestTakeover: shell.requestTakeover()
                onSetPersistentCall: on => shell.setPersistentCall(on)
            }
            MobileTabView {
                theme: appTheme
                batteryAvailable: shell.batteryAvailable
                batteryPercent: shell.batteryPercent
                batteryCharging: shell.batteryCharging
                phoneActivity: shell.phoneActivity
                currentApp: shell.currentApp
                lastActiveText: shell.lastActiveText
                locationSharing: shell.locationSharing
                locationAvailable: shell.locationAvailable
                locationText: shell.locationText
                locationUpdatedText: shell.locationUpdatedText
                phoneNotificationsSharing: shell.phoneNotificationsSharing
                phoneActivitySharing: shell.phoneActivitySharing
                phoneActivityPermission: shell.phoneActivityPermission
                locationPermission: shell.locationPermission
                notificationPermission: shell.notificationPermission
                ownNotificationPermission: shell.ownNotificationPermission
                backgroundLocationPermission: shell.backgroundLocationPermission
                batteryOptimizationPermission: shell.batteryOptimizationPermission
                onSetShareLocation: on => shell.setShareLocation(on)
                onSetSharePhoneActivity: on => shell.setSharePhoneActivity(on)
                onSetSharePhoneNotifications: on => shell.setSharePhoneNotifications(on)
                onOpenSystemSettings: page => shell.openSystemSettings(page)
                onRequestOwnNotificationPermission: shell.requestOwnNotificationPermission()
            }
            MobileSettingsView {
                theme: appTheme
                persistentCall: shell.persistentCall
                updateStatus: shell.updateStatus
                updateError: shell.updateError
                appearanceMode: shell.appearanceMode
                accentColor: shell.accentColor
                accentIntensity: shell.accentIntensity
                oceanVariant: shell.oceanVariant
                cornerRadius: shell.cornerRadius
                density: shell.density
                higherContrast: shell.higherContrast
                reducedMotion: shell.reducedMotion
                animationIntensity: shell.animationIntensity
                locationSharing: shell.locationSharing
                phoneActivitySharing: shell.phoneActivitySharing
                phoneNotificationsSharing: shell.phoneNotificationsSharing
                phoneActivityPermission: shell.phoneActivityPermission
                locationPermission: shell.locationPermission
                notificationPermission: shell.notificationPermission
                ownNotificationPermission: shell.ownNotificationPermission
                backgroundLocationPermission: shell.backgroundLocationPermission
                batteryOptimizationPermission: shell.batteryOptimizationPermission
                onSetPersistentCall: on => shell.setPersistentCall(on)
                onSetShareLocation: on => shell.setShareLocation(on)
                onSetSharePhoneActivity: on => shell.setSharePhoneActivity(on)
                onSetSharePhoneNotifications: on => shell.setSharePhoneNotifications(on)
                onOpenSystemSettings: page => shell.openSystemSettings(page)
                onRequestOwnNotificationPermission: shell.requestOwnNotificationPermission()
                onSetAppearanceMode: mode => shell.setAppearanceMode(mode)
                onSetAccentColor: color => shell.setAccentColor(color)
                onSetAccentIntensity: value => shell.setAccentIntensity(value)
                onSetOceanVariant: variant => shell.setOceanVariant(variant)
                onSetCornerRadius: value => shell.setCornerRadius(value)
                onSetDensity: value => shell.setDensity(value)
                onSetHigherContrast: on => shell.setHigherContrast(on)
                onSetReducedMotion: on => shell.setReducedMotion(on)
                onSetAnimationIntensity: value => shell.setAnimationIntensity(value)
                onCheckForUpdates: shell.checkForUpdates()
                onOpenPairing: shell.openPairing()
            }
        }

        // Bottom navigation is drawn as explicit interactive tiles so every
        // target has a visible surface, independent of platform button styles.
        Item {
            id: nav
            property int currentIndex: 0
            Layout.fillWidth: true
            Layout.preferredHeight: appTheme.navHeight + nav.SafeArea.margins.bottom

            Rectangle {
                id: navBar
                anchors.left: parent.left
                anchors.right: parent.right
                anchors.top: parent.top
                height: appTheme.navHeight
                color: appTheme.bar
                border.width: 1
                border.color: appTheme.borderSubtle

                Rectangle {
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.top: parent.top
                    height: 1
                    color: appTheme.accent
                    opacity: 0.35
                }

                Repeater {
                    model: [qsTr("Home"), qsTr("Chat"), qsTr("Call"), qsTr("Phone"), qsTr("Settings")]

                    Item {
                        required property int index
                        required property string modelData
                        x: width * index
                        y: 0
                        width: navBar.width / 5
                        height: navBar.height

                        Rectangle {
                            anchors.fill: parent
                            anchors.margins: 7
                            radius: appTheme.radius
                            color: navTileTap.pressed
                                ? appTheme.accentDeep
                                : (nav.currentIndex === parent.index ? appTheme.card : "transparent")
                            border.width: nav.currentIndex === parent.index ? 1 : 0
                            border.color: appTheme.accent
                        }

                        Rectangle {
                            anchors.horizontalCenter: parent.horizontalCenter
                            anchors.top: parent.top
                            anchors.topMargin: 2
                            width: Math.min(parent.width - 28, 38)
                            height: 3
                            radius: 2
                            color: appTheme.accent
                            visible: nav.currentIndex === parent.index
                        }

                        Label {
                            anchors.centerIn: parent
                            text: parent.modelData
                            font.pixelSize: 13
                            font.weight: nav.currentIndex === parent.index ? Font.DemiBold : Font.Normal
                            color: nav.currentIndex === parent.index ? appTheme.textPrimary : appTheme.textSecondary
                        }

                        TapHandler {
                            id: navTileTap
                            gesturePolicy: TapHandler.ReleaseWithinBounds
                            onTapped: nav.currentIndex = parent.index
                        }
                    }
                }
            }
        }

    } // ColumnLayout

    // Pairing overlay: full window, dimmed behind, so the flow always has
    // room and never fights the page layout for space.
    Rectangle {
        anchors.fill: parent
        visible: shell.pairingVisible
        color: "#000000"
        opacity: 0.55
        z: 10
    }

    MobilePairingSheet {
        id: pairingSheet
        anchors.fill: parent
        anchors.margins: 16
        visible: shell.pairingVisible
        z: 11
        theme: appTheme
        role: shell.pairingRole
        code: shell.pairingCode
        phase: shell.pairingPhase
        errorText: shell.pairingError
        serverConfigured: shell.serverConfigured
        tailscaleReady: shell.tailscaleInstalled
        busy: shell.pairingBusy
            onCreateCode: shell.createPairingCode()
            onSubmitCode: code => shell.submitPairingCode(code)
            onCopyCode: code => shell.copyPairingCode(code)
            onInstallTailscale: shell.openTailscaleStore()
            onAcceptRequest: shell.acceptPairing()
            onDeclineRequest: shell.declinePairing()
            onCancelFlow: shell.cancelPairing()
            onClose: shell.closePairing()
            onResetFlow: shell.resetPairing()
            onPollStatus: shell.pollPairingStatus()
            onPollIncoming: shell.pollPairingIncoming()
    }

    // Mandatory updates cover everything (including pairing): no taps pass.
    MobileUpdateBlock {
        anchors.fill: parent
        z: 20
        updateStatus: shell.updateStatus
        updateVersion: shell.updateVersion
        updateProgress: shell.updateProgress
        updateError: shell.updateError
        onRetryUpdate: shell.retryUpdate()
        onInstallUpdate: shell.installUpdate()
    }
}
