pragma ComponentBehavior: Bound

import Harbor 2.0
import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts

// Settings are mirrored through HarborSettingsBridge when the core is live.
// Without the core, the same controls remain deterministic for previews and
// tests; no mock feedback is shown over the real persistence path.
HarborStateLayer {
    id: root

    property string category: "general"
    property bool copyFeedbackVisible: false
    property bool capturingPttKey: false

    // Human label for the persisted push-to-talk key: legacy names, common
    // codes, printable characters, or a raw code fallback.
    function pttKeyLabel() {
        var raw = String(AppState.pushToTalkKey)
        if (raw === "Space")
            return "Space"
        if (raw === "Return")
            return "Return"
        if (raw === "Enter")
            return "Enter"
        var code = parseInt(raw, 10)
        if (isNaN(code))
            return raw
        if (code === 32)
            return "Space"
        if (code === 16777220)
            return "Return"
        if (code === 16777221)
            return "Enter"
        if (code === 16777217)
            return "Tab"
        if (code >= 16777264 && code <= 16777275)
            return "F" + (code - 16777263)
        if (code > 32 && code < 127)
            return String.fromCharCode(code).toUpperCase()
        return raw
    }

    readonly property bool compact: width < Theme.breakpointCompact

    // qmllint disable unqualified
    readonly property bool hasCore: typeof HarborCore !== "undefined"
    readonly property bool liveCall: hasCore && HarborCore.coreReady
    // qmllint enable unqualified

    Timer {
        id: copyFeedbackTimer
        interval: 2000
        onTriggered: root.copyFeedbackVisible = false
    }

    HarborCallBridge {
        id: realCall

        // qmllint disable unqualified
        facade: root.liveCall ? HarborCore : null
        // qmllint enable unqualified
    }

    // Same contract, two providers: production core or deterministic mock.
    readonly property var callProvider: root.hasCore ? realCall : MockController

    // The simulation labels itself as one; a real call must never borrow
    // those strings, so each provider reads its own honest status family.
    readonly property string statusFamily: root.hasCore ? "call.status.live" : "call.status"

    function noteSettingChanged(key) {
        if (!root.hasCore)
            MockController.markSettingChanged(key)
    }

    readonly property var categories: [
        { key: "general", icon: "settings" },
        { key: "profile", icon: "user" },
        { key: "appearance", icon: "app" },
        { key: "audio", icon: "volume" },
        { key: "notifications", icon: "activity" },
        { key: "privacy", icon: "lock" }
    ]

    pageState: AppState.pageState("settings")
    title: pageState === "empty" ? I18n.t("state.empty.title")
         : pageState === "error" ? I18n.t("state.error.title") : ""
    description: pageState === "empty" ? I18n.t("state.empty.description")
              : pageState === "error" ? I18n.t("state.error.description") : ""
    actionText: pageState === "error" ? I18n.t("common.actions.retry") : ""
    onActionTriggered: {
        if (root.hasCore)
            HarborCore.retryCore()
        else
            MockController.transitionPage("settings", "content")
    }

    // The core enforces the same conditions at the media boundary; muting
    // always releases PTT even if this view is still open.
    readonly property bool pttAvailable: AppState.callState === "connected"
                                       && !AppState.microphoneMuted
                                       && AppState.connectionState === "connected"
                                       && AppState.pushToTalkEnabled

    function callStatusKey() {
        switch (AppState.callState) {
        case "connecting": return root.statusFamily + ".opening"
        case "connected": return root.statusFamily + ".connected"
        case "unavailable":
            return root.liveCall ? "call.status.live.failed" : "call.status.unavailable"
        default: return root.statusFamily + ".ready"
        }
    }

    function callStatusTone() {
        switch (AppState.callState) {
        case "connecting": return "accent"
        case "connected": return "success"
        case "unavailable": return "danger"
        default: return "neutral"
        }
    }

    function updateStateText(status) {
        switch (String(status)) {
        case "checking": return I18n.t("update.settings.stateChecking")
        case "available": return I18n.t("update.settings.stateAvailable")
        case "downloading": return I18n.t("update.settings.stateDownloading")
        case "ready": return I18n.t("update.settings.stateReady")
        case "applying": return I18n.t("update.settings.stateApplying")
        default: return I18n.t("update.settings.stateIdle")
        }
    }

    HarborPage {
        id: page

        width: parent.width
        height: parent.height
        accessibleName: I18n.t("settings.title")

        HarborPageHeader {
            title: I18n.t("settings.title")
            subtitle: I18n.t("settings.subtitle")
            iconName: "settings"

            HarborBadge {
                tone: root.hasCore ? "success" : (MockController.settingsApplied ? "success" : "neutral")
                showDot: true
                compact: true
                text: I18n.t(root.hasCore ? "settings.saved" : MockController.settingsFeedbackKey,
                             root.hasCore ? {} : MockController.settingsFeedbackParams)
            }
        }

        // The live core confirms durable settings through the bridge. The
        // fallback notice is only relevant to test/preview providers.
        Rectangle {
            Layout.fillWidth: true
            visible: !root.liveCall
            implicitHeight: noticeRow.implicitHeight + Theme.sp3 * 2
            radius: Theme.radiusSmall
            color: Theme.surfaceInteractive
            border.width: 1
            border.color: Theme.borderSubtle

            RowLayout {
                id: noticeRow

                anchors.fill: parent
                anchors.margins: Theme.sp3
                spacing: Theme.sp3

                HarborIcon {
                    name: "info"
                    color: Theme.accent
                    implicitWidth: 18
                    implicitHeight: 18
                    Accessible.ignored: true
                }

                Text {
                    Layout.fillWidth: true
                    text: I18n.t("settings.sessionNotice")
                    color: Theme.textPrimary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontSmall
                    wrapMode: Text.Wrap
                }
            }
        }

        ListView {
            id: compactTabs

            Layout.fillWidth: true
            Layout.preferredHeight: Theme.hitTarget
            visible: root.compact
            orientation: ListView.Horizontal
            spacing: Theme.sp2
            clip: true
            model: root.categories

            delegate: HarborChoiceChip {
                required property var modelData

                width: implicitWidth
                text: I18n.t("settings.category." + modelData.key)
                checked: root.category === modelData.key
                autoExclusive: true
                onClicked: root.category = modelData.key
            }
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: Theme.sp4

            Rectangle {
                Layout.preferredWidth: 238
                Layout.fillHeight: true
                visible: !root.compact
                radius: Theme.radius
                color: Theme.surface
                border.width: 1
                border.color: Theme.borderSubtle

                ListView {
                    id: categoryNav

                    anchors.fill: parent
                    anchors.margins: Theme.sp2
                    spacing: Theme.sp1
                    clip: true
                    model: root.categories

                    delegate: AbstractButton {
                        id: navButton

                        required property var modelData

                        width: categoryNav.width
                        height: 58
                        hoverEnabled: true
                        focusPolicy: Qt.StrongFocus
                        Accessible.role: Accessible.MenuItem
                        Accessible.name: I18n.t("settings.category." + modelData.key)
                        onClicked: root.category = modelData.key

                        background: Rectangle {
                            radius: Theme.radiusSmall
                            color: root.category === navButton.modelData.key
                                   ? Theme.surfaceInteractive
                                   : navButton.hovered ? Theme.surfaceHover : "transparent"
                            border.width: root.category === navButton.modelData.key ? 1 : 0
                            border.color: Theme.accent

                            Behavior on color {
                                ColorAnimation {
                                    duration: Theme.duration(Theme.motionFast)
                                }
                            }
                        }

                        contentItem: RowLayout {
                            spacing: Theme.sp3

                            HarborIcon {
                                name: navButton.modelData.icon
                                color: root.category === navButton.modelData.key
                                       ? Theme.accent : Theme.iconSecondary
                                implicitWidth: 18
                                implicitHeight: 18
                                Accessible.ignored: true
                            }

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 1

                                Text {
                                    Layout.fillWidth: true
                                    text: I18n.t("settings.category." + navButton.modelData.key)
                                    color: root.category === navButton.modelData.key
                                           ? Theme.textPrimary : Theme.textSecondary
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontBody
                                    font.weight: Font.DemiBold
                                    elide: Text.ElideRight
                                }

                                Text {
                                    Layout.fillWidth: true
                                    text: I18n.t("settings.category." + navButton.modelData.key + ".subtitle")
                                    color: Theme.textMuted
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontTiny
                                    elide: Text.ElideRight
                                }
                            }
                        }
                    }
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: Theme.sp4

                // ── General preview ─────────────────────────────────────
                ColumnLayout {
                    Layout.fillWidth: true
                    visible: root.category === "general"
                    spacing: Theme.sp4

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("update.settings.title")
                        description: I18n.t("update.settings.description")
                        iconName: "refresh"

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp3

                                Text {
                                    text: I18n.t("update.settings.version")
                                    color: Theme.textSecondary
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontBody
                                }

                                Item { Layout.fillWidth: true }

                                // qmllint disable unqualified
                                Text {
                                    text: typeof HarborUpdater !== "undefined"
                                        ? HarborUpdater.currentVersion : ""
                                    color: Theme.textPrimary
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontBody
                                    font.weight: Font.DemiBold
                                }
                                // qmllint enable unqualified
                            }

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp3

                                Text {
                                    // qmllint disable unqualified
                                    text: typeof HarborUpdater === "undefined"
                                        ? I18n.t("update.settings.stateIdle")
                                        : (HarborUpdater.status === "error"
                                            ? I18n.t(String(HarborUpdater.errorKey || "update.error.network"))
                                            : root.updateStateText(HarborUpdater.status))
                                    // qmllint enable unqualified
                                    color: Theme.textSecondary
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontBody
                                    wrapMode: Text.Wrap
                                    Layout.fillWidth: true
                                }

                                HarborButton {
                                    text: I18n.t("update.settings.check")
                                    variant: "secondary"
                                    // qmllint disable unqualified
                                    enabled: typeof HarborUpdater !== "undefined"
                                             && (HarborUpdater.status === "idle"
                                                 || HarborUpdater.status === "error")
                                    onClicked: {
                                        if (HarborUpdater.status === "error")
                                            HarborUpdater.retry()
                                        else
                                            HarborUpdater.checkForUpdates()
                                    }
                                    // qmllint enable unqualified
                                }
                            }
                        }
                    }

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("settings.general.startup.title")
                        description: I18n.t("settings.general.startup.description")
                        iconName: "settings"

                        HarborSettingRow {
                            Layout.fillWidth: true
                            label: I18n.t("settings.general.startWithSystem.title")
                            description: I18n.t("settings.general.startWithSystem.description")

                            HarborToggle {
                                checked: AppState.startWithSystem
                                onToggled: {
                                    AppState.startWithSystem = checked
                                    root.noteSettingChanged("settings.saving.startup")
                                }
                            }
                        }

                        HarborSettingRow {
                            Layout.fillWidth: true
                            showDivider: false
                            label: I18n.t("settings.general.autoConnect.title")
                            description: I18n.t("settings.general.autoConnect.description",
                                                { name: AppState.partnerName })

                            HarborToggle {
                                checked: AppState.autoConnect
                                onToggled: {
                                    AppState.autoConnect = checked
                                    root.noteSettingChanged("settings.saving.connection")
                                }
                            }
                        }
                    }

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("settings.general.window.title")
                        iconName: "maximize"

                        HarborSettingRow {
                            Layout.fillWidth: true
                            label: I18n.t("settings.general.minimizeToTray.title")
                            description: I18n.t("settings.general.minimizeToTray.description")

                            HarborToggle {
                                checked: AppState.minimizeToTray
                                onToggled: {
                                    AppState.minimizeToTray = checked
                                    root.noteSettingChanged("settings.saving.window")
                                }
                            }
                        }

                        HarborSettingRow {
                            Layout.fillWidth: true
                            showDivider: false
                            label: I18n.t("settings.general.closeToTray.title")
                            description: I18n.t("settings.general.closeToTray.description")

                            HarborToggle {
                                checked: AppState.closeToTray
                                onToggled: {
                                    AppState.closeToTray = checked
                                    root.noteSettingChanged("settings.saving.window")
                                }
                            }
                        }
                    }

                    // Control-plane server configuration lives outside the
                    // product UI by design: pairing and calls never ask for
                    // addresses, fingerprints, or TLS details. Operators use
                    // the documented `server.configure` channel instead.

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("settings.widget.title")
                        description: I18n.t("settings.widget.description")
                        iconName: "app"

                        HarborSettingRow {
                            Layout.fillWidth: true
                            label: I18n.t("settings.widget.enabled.title")
                            description: I18n.t("settings.widget.enabled.description")

                            HarborToggle {
                                checked: AppState.widgetEnabled
                                onToggled: {
                                    AppState.widgetEnabled = checked
                                    root.noteSettingChanged("settings.saving.window")
                                }
                            }
                        }

                        HarborSelect {
                            Layout.fillWidth: true
                            label: I18n.t("settings.widget.position.title")
                            model: [
                                { key: "topLeft", labelKey: "settings.widget.position.topLeft" },
                                { key: "topRight", labelKey: "settings.widget.position.topRight" },
                                { key: "bottomLeft", labelKey: "settings.widget.position.bottomLeft" },
                                { key: "bottomRight", labelKey: "settings.widget.position.bottomRight" }
                            ]
                            textRole: ""
                            valueRole: "key"
                            translationKeyRole: "labelKey"
                            currentIndex: Math.max(0, ["topLeft", "topRight", "bottomLeft", "bottomRight"].indexOf(AppState.widgetPosition))
                            onActivated: {
                                AppState.widgetPosition = ["topLeft", "topRight", "bottomLeft", "bottomRight"][index] || "bottomRight"
                                root.noteSettingChanged("settings.saving.window")
                            }
                        }

                        HarborSettingRow {
                            Layout.fillWidth: true
                            label: I18n.t("settings.widget.activity.title")
                            description: I18n.t("settings.widget.activity.description")

                            HarborToggle {
                                checked: AppState.widgetShowActivity
                                onToggled: {
                                    AppState.widgetShowActivity = checked
                                    root.noteSettingChanged("settings.saving.window")
                                }
                            }
                        }

                        HarborSettingRow {
                            Layout.fillWidth: true
                            label: I18n.t("settings.widget.avatar.title")
                            description: I18n.t("settings.widget.avatar.description")

                            HarborToggle {
                                checked: AppState.widgetShowAvatar
                                onToggled: {
                                    AppState.widgetShowAvatar = checked
                                    root.noteSettingChanged("settings.saving.window")
                                }
                            }
                        }

                        HarborSettingRow {
                            Layout.fillWidth: true
                            showDivider: false
                            label: I18n.t("settings.widget.callPresence.title")
                            description: I18n.t("settings.widget.callPresence.description")

                            HarborToggle {
                                checked: AppState.widgetShowCallPresence
                                onToggled: {
                                    AppState.widgetShowCallPresence = checked
                                    root.noteSettingChanged("settings.saving.window")
                                }
                            }
                        }
                    }
                }

                // ── Appearance ──────────────────────────────────────────
                ColumnLayout {
                    Layout.fillWidth: true
                    visible: root.category === "appearance"
                    spacing: Theme.sp4

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("settings.appearance.colorMode.title")
                        description: I18n.t("settings.appearance.colorMode.description")
                        iconName: "app"

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            HarborChoiceChip {
                                Layout.fillWidth: true
                                text: I18n.t("settings.appearance.moonlight")
                                checked: AppState.appearanceMode === "dark"
                                autoExclusive: true
                                onClicked: {
                                    AppState.appearanceMode = "dark"
                                    root.noteSettingChanged("settings.saving.theme")
                                }
                            }

                            HarborChoiceChip {
                                Layout.fillWidth: true
                                text: I18n.t("settings.appearance.daylight")
                                checked: AppState.appearanceMode === "light"
                                autoExclusive: true
                                onClicked: {
                                    AppState.appearanceMode = "light"
                                    root.noteSettingChanged("settings.saving.theme")
                                }
                            }

                            HarborChoiceChip {
                                Layout.fillWidth: true
                                text: I18n.t("settings.appearance.system")
                                checked: AppState.appearanceMode === "system"
                                autoExclusive: true
                                onClicked: {
                                    AppState.appearanceMode = "system"
                                    root.noteSettingChanged("settings.saving.theme")
                                }
                            }
                        }
                    }

                    // Accent: presets, custom color, and strength. Every
                    // interactive surface reads Theme.accent, so the choice
                    // applies live across buttons, icons, switches, sliders,
                    // highlights, and the sidebar selection.
                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("settings.appearance.accentColor.title")
                        description: I18n.t("settings.appearance.accentColor.description")
                        iconName: "online"

                        Flow {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            Repeater {
                                model: Theme.accentPresetList

                                delegate: Rectangle {
                                    id: swatch

                                    required property var modelData

                                    implicitWidth: 44
                                    implicitHeight: 44
                                    radius: Theme.radiusSmall
                                    color: swatch.modelData.color
                                    border.width: AppState.accentColor === swatch.modelData.key ? 3 : 1
                                    border.color: AppState.accentColor === swatch.modelData.key
                                                  ? Theme.textPrimary : Theme.surfaceBorder
                                    Accessible.role: Accessible.Button
                                    Accessible.name: I18n.t("settings.appearance.accentColor." + swatch.modelData.key)

                                    MouseArea {
                                        anchors.fill: parent
                                        cursorShape: Qt.PointingHandCursor
                                        onClicked: {
                                            AppState.accentColor = swatch.modelData.key
                                            root.noteSettingChanged("settings.saving.accent")
                                        }
                                    }
                                }
                            }

                            Rectangle {
                                implicitWidth: 44
                                implicitHeight: 44
                                radius: Theme.radiusSmall
                                color: /^#[0-9a-fA-F]{6}$/.test(AppState.accentColor)
                                       ? AppState.accentColor : Theme.surfaceStrong
                                border.width: /^#[0-9a-fA-F]{6}$/.test(AppState.accentColor) ? 3 : 1
                                border.color: /^#[0-9a-fA-F]{6}$/.test(AppState.accentColor)
                                              ? Theme.textPrimary : Theme.surfaceBorder
                                Accessible.role: Accessible.Button
                                Accessible.name: I18n.t("settings.appearance.accentColor.custom")

                                Text {
                                    anchors.centerIn: parent
                                    visible: !/^#[0-9a-fA-F]{6}$/.test(AppState.accentColor)
                                    text: "+"
                                    color: Theme.textSecondary
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontTitle
                                }

                                MouseArea {
                                    anchors.fill: parent
                                    cursorShape: Qt.PointingHandCursor
                                    onClicked: accentDialog.open()
                                }
                            }
                        }

                        ColorDialog {
                            id: accentDialog

                            title: I18n.t("settings.appearance.accentColor.custom")
                            onAccepted: {
                                AppState.accentColor = String(selectedColor).slice(0, 7)
                                root.noteSettingChanged("settings.saving.accent")
                            }
                        }

                        HarborSlider {
                            Layout.fillWidth: true
                            label: I18n.t("settings.appearance.accent.soft")
                            from: 25
                            to: 100
                            stepSize: 5
                            unit: "%"
                            value: Math.round(AppState.accentIntensity * 100)
                            onMoved: {
                                AppState.accentIntensity = value / 100
                                root.noteSettingChanged("settings.saving.accent")
                            }
                        }
                    }

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("settings.appearance.glass.title")
                        description: I18n.t("settings.appearance.glass.description")
                        iconName: "app"

                        HarborSlider {
                            Layout.fillWidth: true
                            label: I18n.t("settings.appearance.glass.strength")
                            from: 20
                            to: 100
                            stepSize: 5
                            unit: "%"
                            value: Math.round(AppState.glassIntensity * 100)
                            onMoved: {
                                AppState.glassIntensity = Math.max(0.2, Math.min(1, value / 100))
                                root.noteSettingChanged("settings.saving.accent")
                            }
                        }
                    }

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("settings.appearance.ocean.title")
                        description: I18n.t("settings.appearance.ocean.description")
                        iconName: "activity"

                        Flow {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            Repeater {
                                model: Theme.oceanVariantList

                                delegate: HarborChoiceChip {
                                    id: oceanChip

                                    required property string modelData

                                    text: I18n.t("settings.appearance.ocean." + oceanChip.modelData)
                                    checked: AppState.oceanVariant === oceanChip.modelData
                                    autoExclusive: true
                                    onClicked: {
                                        AppState.oceanVariant = oceanChip.modelData
                                        root.noteSettingChanged("settings.saving.theme")
                                    }
                                }
                            }
                        }
                    }

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("settings.appearance.shape.title")
                        description: I18n.t("settings.appearance.shape.description")
                        iconName: "settings"

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            HarborChoiceChip {
                                Layout.fillWidth: true
                                text: I18n.t("settings.appearance.shape.soft")
                                checked: AppState.cornerRadius !== "medium"
                                autoExclusive: true
                                onClicked: {
                                    AppState.cornerRadius = "soft"
                                    root.noteSettingChanged("settings.saving.theme")
                                }
                            }

                            HarborChoiceChip {
                                Layout.fillWidth: true
                                text: I18n.t("settings.appearance.shape.medium")
                                checked: AppState.cornerRadius === "medium"
                                autoExclusive: true
                                onClicked: {
                                    AppState.cornerRadius = "medium"
                                    root.noteSettingChanged("settings.saving.theme")
                                }
                            }
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            HarborChoiceChip {
                                Layout.fillWidth: true
                                text: I18n.t("settings.appearance.density.comfortable")
                                checked: AppState.density !== "compact"
                                autoExclusive: true
                                onClicked: {
                                    AppState.density = "comfortable"
                                    root.noteSettingChanged("settings.saving.theme")
                                }
                            }

                            HarborChoiceChip {
                                Layout.fillWidth: true
                                text: I18n.t("settings.appearance.density.compact")
                                checked: AppState.density === "compact"
                                autoExclusive: true
                                onClicked: {
                                    AppState.density = "compact"
                                    root.noteSettingChanged("settings.saving.theme")
                                }
                            }
                        }
                    }

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("settings.appearance.atmosphere.title")
                        iconName: "refresh"

                        HarborSettingRow {
                            Layout.fillWidth: true
                            showDivider: false
                            label: I18n.t("settings.appearance.ambient.title")
                            description: I18n.t("settings.appearance.ambient.description")

                            HarborToggle {
                                checked: AppState.backgroundAnimation
                                onToggled: {
                                    AppState.backgroundAnimation = checked
                                    root.noteSettingChanged("settings.saving.atmosphere")
                                }
                            }
                        }


                        HarborSettingRow {
                            Layout.fillWidth: true
                            showDivider: false
                            label: I18n.t("settings.appearance.reduceMotion.title")
                            description: I18n.t("settings.appearance.reduceMotion.description")

                            HarborToggle {
                                checked: AppState.reducedMotion
                                onToggled: {
                                    AppState.reducedMotion = checked
                                    root.noteSettingChanged("settings.saving.motion")
                                }
                            }
                        }
                    }

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("settings.appearance.contrast.title")
                        description: I18n.t("settings.appearance.contrast.description")
                        iconName: "check-circle"

                        HarborSettingRow {
                            Layout.fillWidth: true
                            showDivider: false
                            label: I18n.t("settings.appearance.contrast.title")

                            HarborToggle {
                                checked: AppState.higherContrast
                                onToggled: {
                                    AppState.higherContrast = checked
                                    root.noteSettingChanged("settings.saving.theme")
                                }
                            }
                        }
                    }
                }

                // ── Language ────────────────────────────────────────────
                ColumnLayout {
                    Layout.fillWidth: true
                    visible: root.category === "profile"
                    spacing: Theme.sp4

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("settings.profile.title")
                        description: I18n.t("settings.profile.description")
                        iconName: "user"

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp4

                            HarborAvatar {
                                avatarSize: 72
                                initials: AppState.selfProfile.initials
                                source: AppState.selfProfile.avatar
                                avatarType: AppState.selfProfile.avatarType
                                status: "online"
                                accessibleName: I18n.t("a11y.avatarFor", { name: AppState.selfProfile.name })
                            }

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp1

                                Text {
                                    Layout.fillWidth: true
                                    text: AppState.selfName.length > 0
                                          ? AppState.selfProfile.name
                                          : I18n.t("settings.profile.noname")
                                    color: AppState.selfName.length > 0
                                           ? Theme.textPrimary : Theme.textSecondary
                                    font.family: Theme.fontFamilyDisplay
                                    font.pixelSize: Theme.fontBody
                                    font.weight: Font.DemiBold
                                    font.italic: AppState.selfName.length === 0
                                }
                                Text {
                                    Layout.fillWidth: true
                                    text: AppState.selfProfile.status.length > 0
                                          ? AppState.selfProfile.status
                                          : I18n.t("profile.noStatus")
                                    color: Theme.textSecondary
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontSmall
                                    wrapMode: Text.WordWrap
                                }
                                Text {
                                    Layout.fillWidth: true
                                    visible: AppState.harborId.length > 0
                                    text: AppState.harborId
                                    color: Theme.textFaint
                                    font.family: Theme.fontFamilyMonospace
                                    font.pixelSize: Theme.fontTiny
                                    elide: Text.ElideMiddle
                                }
                            }
                        }

                        HarborButton {
                            variant: "secondary"
                            iconName: "user"
                            text: I18n.t("settings.profile.edit")
                            onClicked: AppState.navigate("profile")
                        }
                    }
                }

                // ── Call preview ────────────────────────────────────────
                ColumnLayout {
                    Layout.fillWidth: true
                    visible: root.category === "audio"
                    spacing: Theme.sp4

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t(root.liveCall ? "settings.audio.devices.live.title"
                                                       : "settings.audio.devices.title")
                        description: I18n.t(root.liveCall ? "settings.audio.devices.live.description"
                                                             : "settings.audio.devices.description")
                        iconName: "volume"

                        GridLayout {
                            Layout.fillWidth: true
                            columns: root.compact ? 1 : 2
                            columnSpacing: Theme.sp4
                            rowSpacing: Theme.sp3

                            HarborSelect {
                                Layout.fillWidth: true
                                label: I18n.t("settings.audio.input")
                                model: root.callProvider.audioInputOptions
                                valueRole: "id"
                                textRole: root.liveCall ? "name" : ""
                                translationKeyRole: root.liveCall ? "" : "labelKey"
                                translationParamsRole: root.liveCall ? "" : "labelParams"
                                currentIndex: Math.max(0, root.callProvider.audioOptionIndex(
                                                            root.callProvider.audioInputOptions,
                                                            AppState.inputDevice))
                                onActivated: (index) => {
                                    if (root.liveCall)
                                        root.callProvider.selectAudioDevices(currentValue,
                                                                             AppState.outputDevice)
                                    else {
                                        AppState.inputDevice = currentValue
                                        root.noteSettingChanged("settings.saving.input")
                                    }
                                }
                            }

                            HarborSelect {
                                Layout.fillWidth: true
                                label: I18n.t("settings.audio.output")
                                model: root.callProvider.audioOutputOptions
                                valueRole: "id"
                                textRole: root.liveCall ? "name" : ""
                                translationKeyRole: root.liveCall ? "" : "labelKey"
                                translationParamsRole: root.liveCall ? "" : "labelParams"
                                currentIndex: Math.max(0, root.callProvider.audioOptionIndex(
                                                            root.callProvider.audioOutputOptions,
                                                            AppState.outputDevice))
                                onActivated: (index) => {
                                    if (root.liveCall)
                                        root.callProvider.selectAudioDevices(AppState.inputDevice,
                                                                             currentValue)
                                    else {
                                        AppState.outputDevice = currentValue
                                        root.noteSettingChanged("settings.saving.output")
                                    }
                                }
                            }
                        }
                    }

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t(root.liveCall ? "settings.audio.levels.live.title"
                                                       : "settings.audio.levels.title")
                        iconName: "mic"

                        HarborSlider {
                            Layout.fillWidth: true
                            label: I18n.t("call.audio.microphone")
                            from: 0
                            to: 100
                            stepSize: 1
                            unit: "%"
                            value: Math.round(AppState.microphoneVolume * 100)
                            onMoved: {
                                if (root.liveCall)
                                    root.callProvider.setAudioVolumes(value / 100,
                                                                      AppState.outputVolume)
                                else {
                                    AppState.microphoneVolume = value / 100
                                    root.noteSettingChanged("settings.saving.microphoneLevel")
                                }
                            }
                        }

                        HarborSlider {
                            Layout.fillWidth: true
                            label: I18n.t("call.audio.output")
                            from: 0
                            to: 100
                            stepSize: 1
                            unit: "%"
                            value: Math.round(AppState.outputVolume * 100)
                            onMoved: {
                                if (root.liveCall)
                                    root.callProvider.setAudioVolumes(AppState.microphoneVolume,
                                                                      value / 100)
                                else {
                                    AppState.outputVolume = value / 100
                                    root.noteSettingChanged("settings.saving.outputLevel")
                                }
                            }
                        }
                    }

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t(root.liveCall ? "settings.audio.voice.live.title"
                                                       : "settings.audio.voice.title")
                        iconName: "mic-off"

                        HarborSettingRow {
                            Layout.fillWidth: true
                            showDivider: false
                            label: I18n.t("settings.audio.voiceActivation.title")
                            description: I18n.t("settings.audio.voiceActivation.description")

                            HarborToggle {
                                checked: AppState.voiceActivation
                                onToggled: {
                                    AppState.voiceActivation = checked
                                    root.noteSettingChanged("settings.saving.voice")
                                }
                            }
                        }
                    }

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("call.ptt.title")
                        description: I18n.t("settings.call.ptt.description")
                        iconName: "mic"

                        HarborSettingRow {
                            Layout.fillWidth: true
                            label: I18n.t("settings.call.ptt.enable.title")
                            description: I18n.t("settings.call.ptt.enable.description")

                            HarborToggle {
                                checked: AppState.pushToTalkEnabled
                                onToggled: AppState.pushToTalkEnabled = checked
                            }
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 0

                                Text {
                                    text: I18n.t("settings.call.ptt.key.title")
                                    color: Theme.textPrimary
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontSmall
                                    font.weight: Font.DemiBold
                                }

                                Text {
                                    text: I18n.t("settings.call.ptt.key.description")
                                    color: Theme.textSecondary
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontTiny
                                    wrapMode: Text.Wrap
                                }
                            }

                            HarborButton {
                                id: pttKeyButton

                                objectName: "pttKeyButton"
                                enabled: AppState.pushToTalkEnabled
                                variant: "secondary"
                                text: root.capturingPttKey
                                      ? I18n.t("settings.call.ptt.key.press")
                                      : root.pttKeyLabel()
                                onClicked: {
                                    root.capturingPttKey = true
                                    pttKeyCatcher.forceActiveFocus()
                                }
                            }
                        }

                        // Invisible key catcher: while capturing, the next
                        // key press becomes the push-to-talk key. Escape
                        // cancels instead of binding.
                        Item {
                            id: pttKeyCatcher

                            objectName: "pttKeyCatcher"
                            focus: root.capturingPttKey
                            Keys.onPressed: event => {
                                if (!root.capturingPttKey)
                                    return
                                event.accepted = true
                                if (event.key === Qt.Key_Escape) {
                                    root.capturingPttKey = false
                                    return
                                }
                                AppState.pushToTalkKey = String(event.key)
                                root.noteSettingChanged("settings.saving.voice")
                                root.capturingPttKey = false
                            }
                            onActiveFocusChanged: {
                                if (!activeFocus)
                                    root.capturingPttKey = false
                            }
                        }

                        // Microphone self-check: real capture looped back
                        // through the selected devices, with a live level.
                        // No call needed; refused honestly while a call owns
                        // the microphone. The poll timer only runs while the
                        // core reports an active test.
                        Timer {
                            id: micTestPollTimer

                            interval: 120
                            repeat: true
                            running: root.liveCall && HarborCore.micTestActive
                            onTriggered: HarborCore.pollMicTest()
                        }

                        HarborAudioVisualizer {
                            Layout.fillWidth: true
                            Layout.preferredHeight: 56
                            level: root.liveCall && HarborCore.micTestActive
                                   ? HarborCore.micTestLevel
                                   : (root.pttAvailable ? root.callProvider.microphoneLevel : 0)
                            running: (root.liveCall && HarborCore.micTestActive) || root.pttAvailable
                            barColor: Theme.accent
                            accessibleName: I18n.t("call.audio.title")
                        }

                        Text {
                            Layout.fillWidth: true
                            text: {
                                if (!root.hasCore)
                                    return I18n.t("call.audio.deviceUnavailable")
                                if (HarborCore.micTestError.length > 0)
                                    return I18n.t(HarborCore.micTestError)
                                if (HarborCore.micTestActive)
                                    return I18n.t("settings.audio.mictest.listening")
                                            + " " + HarborCore.micTestSecondsLeft + " s"
                                if (HarborCore.micTestPeak > 0.01)
                                    return I18n.t("settings.audio.mictest.peak")
                                            + " " + Math.round(HarborCore.micTestPeak * 100) + "%"
                                if (AppState.callState === "connected")
                                    return I18n.t("settings.audio.mictest.unavailableDuringCall")
                                return I18n.t("settings.audio.mictest.description")
                            }
                            color: root.liveCall && HarborCore.micTestError.length > 0
                                   ? Theme.danger : Theme.textMuted
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontSmall
                            wrapMode: Text.Wrap
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp3
                            visible: root.hasCore

                            HarborButton {
                                variant: "secondary"
                                text: root.liveCall && HarborCore.micTestActive
                                      ? I18n.t("settings.audio.mictest.stop")
                                      : I18n.t("settings.audio.mictest.start")
                                enabled: root.liveCall && AppState.callState !== "connected"
                                onClicked: {
                                    if (HarborCore.micTestActive)
                                        HarborCore.stopMicTest()
                                    else
                                        HarborCore.startMicTest(5)
                                }
                            }

                            Text {
                                visible: AppState.pushToTalkActive
                                text: I18n.t("call.ptt.active")
                                color: Theme.success
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontSmall
                                font.weight: Font.DemiBold
                            }

                            Item { Layout.fillWidth: true }
                        }
                    }
                }

                // ── In-app notifications ────────────────────────────────
                ColumnLayout {
                    Layout.fillWidth: true
                    visible: root.category === "notifications"
                    spacing: Theme.sp4

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("settings.notifications.center.title")
                        iconName: "activity"

                        HarborSettingRow {
                            Layout.fillWidth: true
                            label: I18n.t("settings.notifications.allow.title")
                            description: I18n.t("settings.notifications.allow.description")

                            HarborToggle {
                                checked: AppState.notificationsEnabled
                                onToggled: {
                                    AppState.notificationsEnabled = checked
                                    root.noteSettingChanged("settings.saving.notifications")
                                }
                            }
                        }

                        HarborSettingRow {
                            Layout.fillWidth: true
                            label: I18n.t("settings.notifications.sound.title")
                            description: I18n.t("settings.notifications.sound.description")

                            HarborToggle {
                                checked: AppState.notificationSound
                                onToggled: {
                                    AppState.notificationSound = checked
                                    root.noteSettingChanged("settings.saving.sound")
                                }
                            }
                        }

                        HarborSettingRow {
                            Layout.fillWidth: true
                            showDivider: false
                            label: I18n.t("settings.notifications.previews.title")
                            description: I18n.t("settings.notifications.previews.description")

                            HarborToggle {
                                checked: AppState.messagePreviews
                                onToggled: {
                                    AppState.messagePreviews = checked
                                    root.noteSettingChanged("settings.saving.notifications")
                                }
                            }
                        }
                    }

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("settings.notifications.about.title")
                        iconName: "game"

                        HarborSettingRow {
                            Layout.fillWidth: true
                            label: I18n.t("settings.notifications.games")

                            HarborToggle {
                                checked: AppState.gameNotifications
                                onToggled: {
                                    AppState.gameNotifications = checked
                                    root.noteSettingChanged("settings.saving.notifications")
                                }
                            }
                        }

                        HarborSettingRow {
                            Layout.fillWidth: true
                            label: I18n.t("settings.notifications.applications")

                            HarborToggle {
                                checked: AppState.appNotifications
                                onToggled: {
                                    AppState.appNotifications = checked
                                    root.noteSettingChanged("settings.saving.notifications")
                                }
                            }
                        }

                        HarborSettingRow {
                            Layout.fillWidth: true
                            label: I18n.t("settings.notifications.connectionChanges")

                            HarborToggle {
                                checked: AppState.connectionNotifications
                                onToggled: {
                                    AppState.connectionNotifications = checked
                                    root.noteSettingChanged("settings.saving.notifications")
                                }
                            }
                        }
                    }

                    // Presence alerts: three independent toggles. The presence
                    // state machine runs regardless of any of them.
                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("settings.notifications.presence.title")
                        description: I18n.t("settings.notifications.presence.description")
                        iconName: "user"

                        HarborSettingRow {
                            Layout.fillWidth: true
                            label: I18n.t("settings.notifications.presenceOnline.title")
                            description: I18n.t("settings.notifications.presenceOnline.description")

                            HarborToggle {
                                checked: AppState.notifyPartnerOnline
                                onToggled: {
                                    AppState.notifyPartnerOnline = checked
                                    root.noteSettingChanged("settings.saving.notifications")
                                }
                            }
                        }

                        HarborSettingRow {
                            Layout.fillWidth: true
                            label: I18n.t("settings.notifications.presenceAway.title")
                            description: I18n.t("settings.notifications.presenceAway.description")

                            HarborToggle {
                                checked: AppState.notifyPartnerAway
                                onToggled: {
                                    AppState.notifyPartnerAway = checked
                                    root.noteSettingChanged("settings.saving.notifications")
                                }
                            }
                        }

                        HarborSettingRow {
                            Layout.fillWidth: true
                            showDivider: false
                            label: I18n.t("settings.notifications.presenceOffline.title")
                            description: I18n.t("settings.notifications.presenceOffline.description")

                            HarborToggle {
                                checked: AppState.notifyPartnerOffline
                                onToggled: {
                                    AppState.notifyPartnerOffline = checked
                                    root.noteSettingChanged("settings.saving.notifications")
                                }
                            }
                        }
                    }
                }

                // ── Privacy preview ─────────────────────────────────────
                ColumnLayout {
                    Layout.fillWidth: true
                    visible: root.category === "privacy"
                    spacing: Theme.sp4

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("settings.privacy.presence.title")
                        description: I18n.t("settings.privacy.presence.description")
                        iconName: "lock"

                        HarborSettingRow {
                            Layout.fillWidth: true
                            label: I18n.t("settings.privacy.online.title")
                            description: I18n.t("settings.privacy.online.description",
                                                { name: AppState.partnerName })

                            HarborToggle {
                                checked: AppState.presenceVisibility
                                onToggled: {
                                    AppState.presenceVisibility = checked
                                    root.noteSettingChanged("settings.saving.privacy")
                                }
                            }
                        }

                        HarborSettingRow {
                            Layout.fillWidth: true
                            showDivider: false
                            label: I18n.t("settings.privacy.activity.title")
                            description: I18n.t("settings.privacy.activity.description")

                            HarborToggle {
                                checked: AppState.activitySharing
                                onToggled: {
                                    AppState.activitySharing = checked
                                    root.noteSettingChanged("settings.saving.privacy")
                                }
                            }
                        }
                    }

                    HarborSectionCard {
                        Layout.fillWidth: true
                        enabled: AppState.activitySharing
                        opacity: enabled ? 1 : Theme.opacityMuted
                        title: I18n.t("settings.privacy.details.title")
                        iconName: "user"

                        HarborSettingRow {
                            Layout.fillWidth: true
                            label: I18n.t("settings.privacy.gameTitles")

                            HarborToggle {
                                checked: AppState.gameVisibility
                                onToggled: {
                                    AppState.gameVisibility = checked
                                    root.noteSettingChanged("settings.saving.privacy")
                                }
                            }
                        }

                        HarborSettingRow {
                            Layout.fillWidth: true
                            showDivider: false
                            label: I18n.t("settings.privacy.deviceNames")

                            HarborToggle {
                                checked: AppState.deviceVisibility
                                onToggled: {
                                    AppState.deviceVisibility = checked
                                    root.noteSettingChanged("settings.saving.privacy")
                                }
                            }
                        }
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        implicitHeight: privacyNotice.implicitHeight + Theme.sp3 * 2
                        radius: Theme.radiusSmall
                        color: Theme.surfaceInteractive
                        border.width: 1
                        border.color: Theme.accentDeep

                        RowLayout {
                            id: privacyNotice

                            anchors.fill: parent
                            anchors.margins: Theme.sp3
                            spacing: Theme.sp3

                            HarborIcon {
                                name: "lock"
                                color: Theme.accent
                                implicitWidth: 18
                                implicitHeight: 18
                                Accessible.ignored: true
                            }

                            Text {
                                Layout.fillWidth: true
                                text: I18n.t("settings.privacy.neverShares")
                                color: Theme.textPrimary
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontSmall
                                wrapMode: Text.Wrap
                            }
                        }
                    }
                }

                // ── Developer ───────────────────────────────────────────
                ColumnLayout {
                    Layout.fillWidth: true
                    visible: root.category === "developer"
                    spacing: Theme.sp4

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("settings.advanced.diagnostics.title")
                        iconName: "monitor"

                        HarborSettingRow {
                            Layout.fillWidth: true
                            label: I18n.t("settings.advanced.developerPanel.title")
                            description: I18n.t("settings.advanced.developerPanel.description")

                            HarborToggle {
                                checked: AppState.devPanelVisible
                                onToggled: {
                                    AppState.devPanelVisible = checked
                                    root.noteSettingChanged("settings.saving.developer")
                                }
                            }
                        }

                        HarborSettingRow {
                            Layout.fillWidth: true
                            showDivider: false
                            label: I18n.t("settings.advanced.debugLogging.title")
                            description: I18n.t("settings.advanced.debugLogging.description")

                            HarborToggle {
                                checked: AppState.debugMode
                                onToggled: {
                                    AppState.debugMode = checked
                                    root.noteSettingChanged("settings.saving.diagnostics")
                                }
                            }
                        }
                    }

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("settings.advanced.identity.title")
                        description: I18n.t("settings.advanced.identity.description")
                        iconName: "user"

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp3

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 2

                                Text {
                                    text: I18n.t("settings.advanced.identity.title")
                                    color: Theme.textMuted
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontTiny
                                }

                                Text {
                                    text: AppState.harborId
                                    color: Theme.textPrimary
                                    font.family: Theme.fontFamilyMonospace
                                    font.pixelSize: Theme.fontBody
                                    font.weight: Font.DemiBold
                                }

                                Text {
                                    visible: root.copyFeedbackVisible
                                    text: I18n.t("settings.advanced.identity.copied")
                                    color: Theme.success
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontTiny
                                }
                            }

                            HarborButton {
                                variant: "secondary"
                                text: I18n.t("common.actions.copy")
                                onClicked: {
                                    if (root.hasCore)
                                        HarborCore.copyToClipboard(AppState.harborId)
                                    else
                                        MockController.mockCopy(AppState.harborId, "harborId")
                                    root.copyFeedbackVisible = true
                                    copyFeedbackTimer.restart()
                                }
                            }
                        }
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        implicitHeight: resetColumn.implicitHeight + Theme.sp4 * 2
                        radius: Theme.radius
                        color: Theme.withOpacity(Theme.danger, Theme.dark ? 0.12 : 0.08)
                        border.width: 1
                        border.color: Theme.danger

                        RowLayout {
                            id: resetColumn

                            anchors.fill: parent
                            anchors.margins: Theme.sp4
                            spacing: Theme.sp4

                            HarborIcon {
                                name: "restore"
                                color: Theme.danger
                                implicitWidth: 22
                                implicitHeight: 22
                                Accessible.ignored: true
                            }

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: 2

                                Text {
                                    Layout.fillWidth: true
                                    text: I18n.t("settings.advanced.reset.title")
                                    color: Theme.textPrimary
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontBody
                                    font.weight: Font.DemiBold
                                }

                                Text {
                                    Layout.fillWidth: true
                                    text: I18n.t("settings.advanced.reset.description")
                                    color: Theme.textSecondary
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontSmall
                                    wrapMode: Text.Wrap
                                }
                            }

                            HarborButton {
                                visible: !root.hasCore
                                variant: "danger"
                                text: I18n.t("common.actions.reset")
                                onClicked: MockController.resetSession()
                            }
                        }
                    }
                }
            }
        }
    }
}
