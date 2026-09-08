import QtQuick
import QtTest
import Harbor 2.0

// Contract tests for the core ⇄ AppState settings bridge. The facade is
// stubbed with the exact property surface the real C++ mirror exposes, so
// these tests pin the wiring independently of the supervised Rust process.
TestCase {
    name: "HarborSettingsBridge"

    readonly property string fixtureHarborId: "HBR-7D92-4A10"

    // Stub mirror: defaults match AppState's deterministic fixtures, which
    // themselves match the Rust core defaults.
    QtObject {
        id: stubSettings

        property bool loaded: false
        property string locale: "en"
        property string displayName: ""
        property string statusMessage: ""
        property string avatar: ""
        property string appearanceMode: "dark"
        property string accentColor: "ocean"
        property real glassIntensity: 1.0
        property real animationIntensity: 1.0
        property bool particlesEnabled: true
        property string oceanVariant: "lagoon"
        property string cornerRadius: "soft"
        property string density: "comfortable"
        property bool widgetEnabled: true
        property string widgetPosition: "bottomRight"
        property bool widgetShowActivity: true
        property bool widgetShowAvatar: true
        property bool widgetShowCallPresence: true
        property string avatarType: "image"
        property bool higherContrast: false
        property bool background: true
        property bool reducedMotion: false
        property bool startWithSystem: true
        property bool minimizeToTray: true
        property bool closeToTray: true
        property bool autoConnect: true
        property bool notificationsEnabled: true
        property bool gameNotifications: true
        property bool appNotifications: true
        property bool connectionNotifications: true
        property bool notificationSound: true
        property bool messagePreviews: true
        property bool notifyPartnerOnline: true
        property bool notifyPartnerAway: true
        property bool notifyPartnerOffline: true
        property bool presenceVisibility: true
        property bool activitySharing: true
        property bool gameVisibility: true
        property bool deviceVisibility: true
        property bool voiceActivation: true
        property bool debugMode: false
        property real accentIntensity: 0.75
        property real microphoneVolume: 0.72
        property real outputVolume: 0.64
        property string inputDevice: "default-microphone"
        property string outputDevice: "harbor-headphones"
        property string pushToTalkKey: "Space"
        property bool pushToTalkEnabled: true
    }

    QtObject {
        id: stubFacade

        property string identityHarborId: "harbor-bridge-test"
        property QtObject settings: stubSettings
    }

    HarborSettingsBridge {
        id: bridge
    }

    function init() {
        bridge.facade = null
        stubSettings.loaded = false
        stubSettings.locale = "en"
        stubSettings.displayName = ""
        stubSettings.statusMessage = ""
        stubSettings.avatar = ""
        stubSettings.avatarType = "image"
        stubSettings.appearanceMode = "dark"
        stubSettings.accentColor = "ocean"
        stubSettings.glassIntensity = 1.0
        stubSettings.animationIntensity = 1.0
        stubSettings.particlesEnabled = true
        stubSettings.oceanVariant = "lagoon"
        stubSettings.cornerRadius = "soft"
        stubSettings.density = "comfortable"
        stubSettings.widgetEnabled = true
        stubSettings.widgetPosition = "bottomRight"
        stubSettings.widgetShowActivity = true
        stubSettings.widgetShowAvatar = true
        stubSettings.widgetShowCallPresence = true
        stubSettings.pushToTalkKey = "Space"
        stubSettings.pushToTalkEnabled = true
        stubFacade.identityHarborId = "harbor-bridge-test"
        AppState.harborId = fixtureHarborId
        AppState.selfName = "Jordan"
        AppState.selfStatus = ""
        AppState.selfStatusKey = ""
        AppState.selfAvatar = ""
        AppState.selfAvatarType = "image"
        AppState.appearanceMode = "dark"
        AppState.accentColor = "ocean"
        AppState.glassIntensity = 1.0
        AppState.animationIntensity = 1.0
        AppState.particlesEnabled = true
        AppState.oceanVariant = "lagoon"
        AppState.cornerRadius = "soft"
        AppState.density = "comfortable"
        AppState.widgetEnabled = true
        AppState.widgetPosition = "bottomRight"
        AppState.widgetShowActivity = true
        AppState.widgetShowAvatar = true
        AppState.widgetShowCallPresence = true
    }

    function cleanup() {
        bridge.facade = null
        AppState.harborId = fixtureHarborId
        AppState.selfName = "Jordan"
        AppState.selfStatus = ""
        AppState.selfStatusKey = ""
        AppState.selfAvatar = ""
        AppState.selfAvatarType = "image"
        AppState.appearanceMode = "dark"
        AppState.accentColor = "ocean"
        AppState.glassIntensity = 1.0
        AppState.animationIntensity = 1.0
        AppState.particlesEnabled = true
        AppState.oceanVariant = "lagoon"
        AppState.cornerRadius = "soft"
        AppState.density = "comfortable"
        AppState.widgetEnabled = true
        AppState.widgetPosition = "bottomRight"
        AppState.widgetShowActivity = true
        AppState.widgetShowAvatar = true
        AppState.widgetShowCallPresence = true
    }

    function test_inertWithoutFacade() {
        verify(bridge.facade === null)
        verify(!bridge.bound)
        // Mock-driven edits must never reach a facade that does not exist.
        AppState.appearanceMode = "light"
        // Nothing asserted on the stub: absence of a crash and the remaining
        // defaults are the contract here.
        compare(AppState.appearanceMode, "light")
    }

    function test_boundRequiresTheLoadedDocument() {
        bridge.facade = stubFacade
        verify(!bridge.bound)
        // While the document is not loaded, a core-side value must not leak
        // into AppState.
        stubSettings.appearanceMode = "light"
        compare(AppState.appearanceMode, "dark")

        stubSettings.loaded = true
        tryCompare(bridge, "bound", true)
        tryCompare(AppState, "appearanceMode", "light")
    }

    function test_identityReplacesFixtureWhileCoreLives() {
        compare(AppState.harborId, fixtureHarborId)
        bridge.facade = stubFacade
        tryCompare(AppState, "harborId", "harbor-bridge-test")
        stubFacade.identityHarborId = "harbor-bridge-next"
        tryCompare(AppState, "harborId", "harbor-bridge-next")
    }

    function test_settingsFlowCoreToAppState() {
        bridge.facade = stubFacade
        stubSettings.loaded = true
        tryCompare(bridge, "bound", true)

        stubSettings.reducedMotion = true
        tryCompare(AppState, "reducedMotion", true)
        stubSettings.accentIntensity = 0.5
        tryCompare(AppState, "accentIntensity", 0.5)
        stubSettings.pushToTalkKey = "V"
        tryCompare(AppState, "pushToTalkKey", "V")
        stubSettings.pushToTalkEnabled = false
        tryCompare(AppState, "pushToTalkEnabled", false)
        stubSettings.displayName = "Ari"
        tryCompare(AppState, "selfName", "Ari")
        stubSettings.statusMessage = "Building something new"
        tryCompare(AppState, "selfStatus", "Building something new")
        stubSettings.avatar = "data:image/png;base64,AA=="
        tryCompare(AppState, "selfAvatar", "data:image/png;base64,AA==")
        stubSettings.avatarType = "gif"
        tryCompare(AppState, "selfAvatarType", "gif")
        stubSettings.accentColor = "violet"
        tryCompare(AppState, "accentColor", "violet")
        stubSettings.glassIntensity = 0.5
        tryCompare(AppState, "glassIntensity", 0.5)
        stubSettings.animationIntensity = 0.4
        tryCompare(AppState, "animationIntensity", 0.4)
        stubSettings.particlesEnabled = false
        tryCompare(AppState, "particlesEnabled", false)
        stubSettings.oceanVariant = "abyss"
        tryCompare(AppState, "oceanVariant", "abyss")
        stubSettings.cornerRadius = "medium"
        tryCompare(AppState, "cornerRadius", "medium")
        stubSettings.density = "compact"
        tryCompare(AppState, "density", "compact")
        stubSettings.widgetEnabled = false
        tryCompare(AppState, "widgetEnabled", false)
        stubSettings.widgetPosition = "topLeft"
        tryCompare(AppState, "widgetPosition", "topLeft")
        stubSettings.widgetShowActivity = false
        tryCompare(AppState, "widgetShowActivity", false)
        stubSettings.widgetShowAvatar = false
        tryCompare(AppState, "widgetShowAvatar", false)
        stubSettings.widgetShowCallPresence = false
        tryCompare(AppState, "widgetShowCallPresence", false)
        stubSettings.messagePreviews = false
        tryCompare(AppState, "messagePreviews", false)
        stubSettings.notifyPartnerOnline = false
        tryCompare(AppState, "notifyPartnerOnline", false)
        stubSettings.notifyPartnerAway = false
        tryCompare(AppState, "notifyPartnerAway", false)
        stubSettings.notifyPartnerOffline = false
        tryCompare(AppState, "notifyPartnerOffline", false)
    }

    // The chosen language rides the durable settings: the core's document
    // decides the session's locale, and a user switch is forwarded back so
    // it survives restarts. Without the core the session keeps its locale.
    function test_localeRidesTheDurableSettings() {
        compare(AppState.locale, "en")
        stubSettings.locale = "pt-BR"
        bridge.facade = stubFacade
        stubSettings.loaded = true
        tryCompare(bridge, "bound", true)
        tryCompare(AppState, "locale", "pt-BR")

        AppState.locale = "en"
        tryCompare(stubSettings, "locale", "en")

        bridge.facade = null
        verify(!bridge.bound)
        AppState.locale = "pt-BR"
        compare(stubSettings.locale, "en")
        AppState.locale = "en"
    }

    function test_settingsFlowAppStateToCore() {
        bridge.facade = stubFacade
        stubSettings.loaded = true
        tryCompare(bridge, "bound", true)

        AppState.microphoneVolume = 0.3
        tryCompare(stubSettings, "microphoneVolume", 0.3)
        AppState.appearanceMode = "light"
        tryCompare(stubSettings, "appearanceMode", "light")
        AppState.notificationsEnabled = false
        tryCompare(stubSettings, "notificationsEnabled", false)
        AppState.pushToTalkEnabled = true
        tryCompare(stubSettings, "pushToTalkEnabled", true)
        AppState.selfName = "Riley"
        tryCompare(stubSettings, "displayName", "Riley")
        AppState.updateSelfProfile({ status: "By the creek" })
        tryCompare(stubSettings, "statusMessage", "By the creek")
        AppState.selfAvatar = "data:image/jpeg;base64,AA=="
        tryCompare(stubSettings, "avatar", "data:image/jpeg;base64,AA==")
        AppState.selfAvatarType = "gif"
        tryCompare(stubSettings, "avatarType", "gif")
        AppState.accentColor = "#3AA9DC"
        tryCompare(stubSettings, "accentColor", "#3AA9DC")
        AppState.glassIntensity = 0.6
        tryCompare(stubSettings, "glassIntensity", 0.6)
        AppState.animationIntensity = 0.7
        tryCompare(stubSettings, "animationIntensity", 0.7)
        AppState.particlesEnabled = false
        tryCompare(stubSettings, "particlesEnabled", false)
        AppState.oceanVariant = "sunrise"
        tryCompare(stubSettings, "oceanVariant", "sunrise")
        AppState.cornerRadius = "medium"
        tryCompare(stubSettings, "cornerRadius", "medium")
        AppState.density = "compact"
        tryCompare(stubSettings, "density", "compact")
        AppState.widgetEnabled = false
        tryCompare(stubSettings, "widgetEnabled", false)
        AppState.widgetPosition = "topLeft"
        tryCompare(stubSettings, "widgetPosition", "topLeft")
        AppState.widgetShowActivity = false
        tryCompare(stubSettings, "widgetShowActivity", false)
        AppState.widgetShowAvatar = false
        tryCompare(stubSettings, "widgetShowAvatar", false)
        AppState.widgetShowCallPresence = false
        tryCompare(stubSettings, "widgetShowCallPresence", false)
        AppState.messagePreviews = false
        tryCompare(stubSettings, "messagePreviews", false)
        AppState.notifyPartnerOnline = false
        tryCompare(stubSettings, "notifyPartnerOnline", false)
        AppState.notifyPartnerAway = false
        tryCompare(stubSettings, "notifyPartnerAway", false)
        AppState.notifyPartnerOffline = false
        tryCompare(stubSettings, "notifyPartnerOffline", false)
    }

    function test_detachedCoreStopsTrackingAppState() {
        bridge.facade = stubFacade
        stubSettings.loaded = true
        tryCompare(bridge, "bound", true)

        AppState.debugMode = true
        tryCompare(stubSettings, "debugMode", true)

        // Simulate the core going down: edits stay local and the mirror
        // keeps its last known values.
        bridge.facade = null
        verify(!bridge.bound)
        AppState.debugMode = false
        tryCompare(stubSettings, "debugMode", true)
    }
}
