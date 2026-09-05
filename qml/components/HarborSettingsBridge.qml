pragma ComponentBehavior: Bound
import QtQml

// Production bridge between the supervised Rust core and AppState.
//
// While a ready facade is attached, durable settings flow core → AppState and
// AppState edits are forwarded to the typed settings mirror, which persists
// them through the core. With no facade (tests, core down) every binding is
// inert and the deterministic mock provider stays the single QML truth.
// `locale` rides the same durable path: the chosen language survives
// restarts while dynamic switching stays UI-owned.
//
// The facade is a C++ context property, so this glue file is deliberately
// dynamically typed; qmllint cannot know its members. QtObject has no default
// property, so the wiring objects live in an explicit list instead of being
// declared as children.
// qmllint disable missing-property
QtObject {
    id: bridge

    property QtObject facade: null

    readonly property QtObject settings: facade ? facade.settings : null
    readonly property bool bound: settings !== null && settings.loaded

    readonly property list<QtObject> wiring: [
        // Identity: the real device id replaces the fixture while the core
        // lives.
        Binding {
            target: AppState
            property: "harborId"
            value: bridge.facade ? bridge.facade.identityHarborId : ""
            when: bridge.facade !== null
        },

        // Profile metadata is local durable state. An empty display name or
        // avatar keeps the QML fallback without inventing a peer identity.
        Binding {
            target: AppState
            property: "selfName"
            value: bridge.settings ? bridge.settings.displayName : ""
            when: bridge.bound
        },
        // The durable status line. While bound, the fixture status key is
        // cleared so the real value is never shadowed by mock copy.
        Binding {
            target: AppState
            property: "selfStatus"
            value: bridge.settings ? bridge.settings.statusMessage : ""
            when: bridge.bound
        },
        Binding {
            target: AppState
            property: "selfStatusKey"
            value: ""
            when: bridge.bound
        },
        Binding {
            target: AppState
            property: "selfAvatar"
            value: bridge.settings ? bridge.settings.avatar : ""
            when: bridge.bound
        },
        Binding {
            target: AppState
            property: "selfAvatarType"
            value: bridge.settings && bridge.settings.avatarType === "gif" ? "gif" : "image"
            when: bridge.bound
        },

        // Core → AppState.
        Binding { target: AppState; property: "higherContrast"; value: bridge.settings ? bridge.settings.higherContrast : false; when: bridge.bound },
        Binding { target: AppState; property: "background"; value: bridge.settings ? bridge.settings.background : false; when: bridge.bound },
        Binding { target: AppState; property: "reducedMotion"; value: bridge.settings ? bridge.settings.reducedMotion : false; when: bridge.bound },
        Binding { target: AppState; property: "startWithSystem"; value: bridge.settings ? bridge.settings.startWithSystem : false; when: bridge.bound },
        Binding { target: AppState; property: "minimizeToTray"; value: bridge.settings ? bridge.settings.minimizeToTray : false; when: bridge.bound },
        Binding { target: AppState; property: "closeToTray"; value: bridge.settings ? bridge.settings.closeToTray : false; when: bridge.bound },
        Binding { target: AppState; property: "autoConnect"; value: bridge.settings ? bridge.settings.autoConnect : false; when: bridge.bound },
        Binding { target: AppState; property: "notificationsEnabled"; value: bridge.settings ? bridge.settings.notificationsEnabled : false; when: bridge.bound },
        Binding { target: AppState; property: "gameNotifications"; value: bridge.settings ? bridge.settings.gameNotifications : false; when: bridge.bound },
        Binding { target: AppState; property: "appNotifications"; value: bridge.settings ? bridge.settings.appNotifications : false; when: bridge.bound },
        Binding { target: AppState; property: "connectionNotifications"; value: bridge.settings ? bridge.settings.connectionNotifications : false; when: bridge.bound },
        Binding { target: AppState; property: "notificationSound"; value: bridge.settings ? bridge.settings.notificationSound : false; when: bridge.bound },
        Binding { target: AppState; property: "messagePreviews"; value: bridge.settings ? bridge.settings.messagePreviews : false; when: bridge.bound },
        Binding { target: AppState; property: "notifyPartnerOnline"; value: bridge.settings ? bridge.settings.notifyPartnerOnline : false; when: bridge.bound },
        Binding { target: AppState; property: "notifyPartnerAway"; value: bridge.settings ? bridge.settings.notifyPartnerAway : false; when: bridge.bound },
        Binding { target: AppState; property: "notifyPartnerOffline"; value: bridge.settings ? bridge.settings.notifyPartnerOffline : false; when: bridge.bound },
        Binding { target: AppState; property: "presenceVisibility"; value: bridge.settings ? bridge.settings.presenceVisibility : false; when: bridge.bound },
        Binding { target: AppState; property: "activitySharing"; value: bridge.settings ? bridge.settings.activitySharing : false; when: bridge.bound },
        Binding { target: AppState; property: "gameVisibility"; value: bridge.settings ? bridge.settings.gameVisibility : false; when: bridge.bound },
        Binding { target: AppState; property: "deviceVisibility"; value: bridge.settings ? bridge.settings.deviceVisibility : false; when: bridge.bound },
        Binding { target: AppState; property: "voiceActivation"; value: bridge.settings ? bridge.settings.voiceActivation : false; when: bridge.bound },
        Binding { target: AppState; property: "debugMode"; value: bridge.settings ? bridge.settings.debugMode : false; when: bridge.bound },
        Binding { target: AppState; property: "accentIntensity"; value: bridge.settings ? bridge.settings.accentIntensity : 0; when: bridge.bound },
        Binding { target: AppState; property: "microphoneVolume"; value: bridge.settings ? bridge.settings.microphoneVolume : 0; when: bridge.bound },
        Binding { target: AppState; property: "outputVolume"; value: bridge.settings ? bridge.settings.outputVolume : 0; when: bridge.bound },
        Binding { target: AppState; property: "inputDevice"; value: bridge.settings ? bridge.settings.inputDevice : ""; when: bridge.bound },
        Binding { target: AppState; property: "outputDevice"; value: bridge.settings ? bridge.settings.outputDevice : ""; when: bridge.bound },
        Binding { target: AppState; property: "pushToTalkKey"; value: bridge.settings ? bridge.settings.pushToTalkKey : ""; when: bridge.bound },
        Binding { target: AppState; property: "pushToTalkEnabled"; value: bridge.settings ? bridge.settings.pushToTalkEnabled : false; when: bridge.bound },
        Binding { target: AppState; property: "appearanceMode"; value: bridge.settings ? bridge.settings.appearanceMode : ""; when: bridge.bound },
        Binding { target: AppState; property: "accentColor"; value: bridge.settings ? bridge.settings.accentColor : "ocean"; when: bridge.bound },
        Binding { target: AppState; property: "glassIntensity"; value: bridge.settings ? bridge.settings.glassIntensity : 1.0; when: bridge.bound },
        Binding { target: AppState; property: "animationIntensity"; value: bridge.settings ? bridge.settings.animationIntensity : 1.0; when: bridge.bound },
        Binding { target: AppState; property: "particlesEnabled"; value: bridge.settings ? bridge.settings.particlesEnabled : true; when: bridge.bound },
        Binding { target: AppState; property: "oceanVariant"; value: bridge.settings ? bridge.settings.oceanVariant : "lagoon"; when: bridge.bound },
        Binding { target: AppState; property: "cornerRadius"; value: bridge.settings ? bridge.settings.cornerRadius : "soft"; when: bridge.bound },
        Binding { target: AppState; property: "density"; value: bridge.settings ? bridge.settings.density : "comfortable"; when: bridge.bound },
        Binding { target: AppState; property: "widgetEnabled"; value: bridge.settings ? bridge.settings.widgetEnabled : true; when: bridge.bound },
        Binding { target: AppState; property: "widgetPosition"; value: bridge.settings ? bridge.settings.widgetPosition : "bottomRight"; when: bridge.bound },
        Binding { target: AppState; property: "widgetShowActivity"; value: bridge.settings ? bridge.settings.widgetShowActivity : true; when: bridge.bound },
        Binding { target: AppState; property: "widgetShowAvatar"; value: bridge.settings ? bridge.settings.widgetShowAvatar : true; when: bridge.bound },
        Binding { target: AppState; property: "widgetShowCallPresence"; value: bridge.settings ? bridge.settings.widgetShowCallPresence : true; when: bridge.bound },
        Binding { target: AppState; property: "locale"; value: bridge.settings ? bridge.settings.locale : "en"; when: bridge.bound },

        // AppState → core. Setter equality guards make the echo a no-op, so a
        // user edit, the durable write, and the authoritative re-apply
        // converge without loops.
        Connections {
            target: AppState

            function onSelfNameChanged() {
                AppState.selfInitials = AppState.initialsFor(AppState.selfName)
                if (bridge.bound)
                    bridge.settings.displayName = AppState.selfName
            }
            function onSelfStatusChanged() {
                if (bridge.bound)
                    bridge.settings.statusMessage = AppState.selfStatus
            }
            function onSelfAvatarChanged() { if (bridge.bound) bridge.settings.avatar = String(AppState.selfAvatar) }
            function onSelfAvatarTypeChanged() { if (bridge.bound) bridge.settings.avatarType = AppState.selfAvatarType }
            function onHigherContrastChanged() { if (bridge.bound) bridge.settings.higherContrast = AppState.higherContrast }
            function onBackgroundChanged() { if (bridge.bound) bridge.settings.background = AppState.background }
            function onReducedMotionChanged() { if (bridge.bound) bridge.settings.reducedMotion = AppState.reducedMotion }
            function onStartWithSystemChanged() { if (bridge.bound) bridge.settings.startWithSystem = AppState.startWithSystem }
            function onMinimizeToTrayChanged() { if (bridge.bound) bridge.settings.minimizeToTray = AppState.minimizeToTray }
            function onCloseToTrayChanged() { if (bridge.bound) bridge.settings.closeToTray = AppState.closeToTray }
            function onAutoConnectChanged() { if (bridge.bound) bridge.settings.autoConnect = AppState.autoConnect }
            function onNotificationsEnabledChanged() { if (bridge.bound) bridge.settings.notificationsEnabled = AppState.notificationsEnabled }
            function onGameNotificationsChanged() { if (bridge.bound) bridge.settings.gameNotifications = AppState.gameNotifications }
            function onAppNotificationsChanged() { if (bridge.bound) bridge.settings.appNotifications = AppState.appNotifications }
            function onConnectionNotificationsChanged() { if (bridge.bound) bridge.settings.connectionNotifications = AppState.connectionNotifications }
            function onNotificationSoundChanged() { if (bridge.bound) bridge.settings.notificationSound = AppState.notificationSound }
            function onMessagePreviewsChanged() { if (bridge.bound) bridge.settings.messagePreviews = AppState.messagePreviews }
            function onNotifyPartnerOnlineChanged() { if (bridge.bound) bridge.settings.notifyPartnerOnline = AppState.notifyPartnerOnline }
            function onNotifyPartnerAwayChanged() { if (bridge.bound) bridge.settings.notifyPartnerAway = AppState.notifyPartnerAway }
            function onNotifyPartnerOfflineChanged() { if (bridge.bound) bridge.settings.notifyPartnerOffline = AppState.notifyPartnerOffline }
            function onPresenceVisibilityChanged() { if (bridge.bound) bridge.settings.presenceVisibility = AppState.presenceVisibility }
            function onActivitySharingChanged() { if (bridge.bound) bridge.settings.activitySharing = AppState.activitySharing }
            function onGameVisibilityChanged() { if (bridge.bound) bridge.settings.gameVisibility = AppState.gameVisibility }
            function onDeviceVisibilityChanged() { if (bridge.bound) bridge.settings.deviceVisibility = AppState.deviceVisibility }
            function onVoiceActivationChanged() { if (bridge.bound) bridge.settings.voiceActivation = AppState.voiceActivation }
            function onDebugModeChanged() { if (bridge.bound) bridge.settings.debugMode = AppState.debugMode }
            function onAccentIntensityChanged() { if (bridge.bound) bridge.settings.accentIntensity = AppState.accentIntensity }
            function onMicrophoneVolumeChanged() { if (bridge.bound) bridge.settings.microphoneVolume = AppState.microphoneVolume }
            function onOutputVolumeChanged() { if (bridge.bound) bridge.settings.outputVolume = AppState.outputVolume }
            function onInputDeviceChanged() { if (bridge.bound) bridge.settings.inputDevice = AppState.inputDevice }
            function onOutputDeviceChanged() { if (bridge.bound) bridge.settings.outputDevice = AppState.outputDevice }
            function onPushToTalkKeyChanged() { if (bridge.bound) bridge.settings.pushToTalkKey = AppState.pushToTalkKey }
            function onPushToTalkEnabledChanged() { if (bridge.bound) bridge.settings.pushToTalkEnabled = AppState.pushToTalkEnabled }
            function onAppearanceModeChanged() { if (bridge.bound) bridge.settings.appearanceMode = AppState.appearanceMode }
            function onAccentColorChanged() { if (bridge.bound) bridge.settings.accentColor = AppState.accentColor }
            function onGlassIntensityChanged() { if (bridge.bound) bridge.settings.glassIntensity = AppState.glassIntensity }
            function onAnimationIntensityChanged() { if (bridge.bound) bridge.settings.animationIntensity = AppState.animationIntensity }
            function onParticlesEnabledChanged() { if (bridge.bound) bridge.settings.particlesEnabled = AppState.particlesEnabled }
            function onOceanVariantChanged() { if (bridge.bound) bridge.settings.oceanVariant = AppState.oceanVariant }
            function onCornerRadiusChanged() { if (bridge.bound) bridge.settings.cornerRadius = AppState.cornerRadius }
            function onDensityChanged() { if (bridge.bound) bridge.settings.density = AppState.density }
            function onWidgetEnabledChanged() { if (bridge.bound) bridge.settings.widgetEnabled = AppState.widgetEnabled }
            function onWidgetPositionChanged() { if (bridge.bound) bridge.settings.widgetPosition = AppState.widgetPosition }
            function onWidgetShowActivityChanged() { if (bridge.bound) bridge.settings.widgetShowActivity = AppState.widgetShowActivity }
            function onWidgetShowAvatarChanged() { if (bridge.bound) bridge.settings.widgetShowAvatar = AppState.widgetShowAvatar }
            function onWidgetShowCallPresenceChanged() { if (bridge.bound) bridge.settings.widgetShowCallPresence = AppState.widgetShowCallPresence }
            function onLocaleChanged() { if (bridge.bound) bridge.settings.locale = AppState.locale }
        }
    ]
}
