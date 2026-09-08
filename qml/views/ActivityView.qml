pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Layouts
import Harbor 2.0

Item {
    id: root

    property string activeFilter: "all"
    property string searchText: ""
    readonly property bool compact: width < Theme.breakpointCompact
    readonly property bool hasCore: typeof HarborCore !== "undefined"

    // Same contract, two sources: the supervised core's real local activity
    // engine while it is ready, AppState's deterministic fixtures otherwise.
    // qmllint disable unqualified
    readonly property bool liveActivity: typeof HarborCore !== "undefined" && HarborCore.coreReady
    // qmllint enable unqualified

    HarborActivityBridge {
        id: realActivity

        // qmllint disable unqualified
        facade: root.liveActivity ? HarborCore : null
        // qmllint enable unqualified
    }

    readonly property var filterModel: [
        { key: "all", labelKey: "activity.filter.all", icon: "app" },
        { key: "game", labelKey: "activity.filter.games", icon: "game" },
        { key: "app", labelKey: "activity.filter.apps", icon: "monitor" },
        { key: "online", labelKey: "activity.filter.presence", icon: "online" },
        { key: "call", labelKey: "activity.filter.voice", icon: "mic" }
    ]

    // Week totals are a test-provider fixture. Live local aggregates are
    // not the partner's history, so the card never renders in production.
    readonly property var weekStats: root.hasCore ? [] : [
        { value: "14", labelKey: "common.labels.games" },
        { value: "8", labelKey: "common.labels.applications" },
        { value: "6", labelKey: "common.labels.hours" }
    ]

    // Partner-first: with the supervised core, the main timeline is the
    // paired peer's shared history (validated, metadata-only). Local
    // records stay local — they are never presented as the partner.
    // Without the core, deterministic fixtures keep tests and previews alive.
    readonly property var filteredActivities: {
        var source = root.hasCore ? AppState.remoteActivities : AppState.activities
        return source.filter(function(entry) {
            var categoryMatch = activeFilter === "all" || entry.category === activeFilter
            var query = searchText.toLocaleLowerCase().trim()
            var searchableText = (activityTitle(entry) + " " + activityDescription(entry)).toLocaleLowerCase()
            return categoryMatch && (query.length === 0 || searchableText.indexOf(query) >= 0)
        })
    }

    function activityTitle(entry) {
        if (!entry)
            return ""
        if (entry.titleKey)
            return I18n.t(entry.titleKey, entry.titleParams || {})
        if (entry.label && String(entry.label).length > 0)
            return String(entry.label)
        if (entry.kind)
            return I18n.t("activity.remote.kind." + entry.kind)
        return ""
    }

    function activityDescription(entry) {
        if (!entry)
            return ""
        if (entry.descriptionKey)
            return I18n.t(entry.descriptionKey, entry.descriptionParams || {})
        if (entry.sender)
            return I18n.t("activity.remote.from", { name: entry.sender })
        return ""
    }

    // Real program icon as a data URL, or "" for the category fallback.
    // The keys are theme-safe (`firefox`); resolution is native + cached,
    // so a missing icon degrades to the generic glyph, never a broken image.
    // qmllint disable unqualified
    function appIconFor(entry) {
        if (!entry || !root.liveActivity || typeof HarborCore === "undefined")
            return ""
        var appId = entry.appId || ""
        var iconKey = entry.iconKey || ""
        if (appId.length === 0 && iconKey.length === 0)
            return ""
        try {
            return HarborCore.appIconUrl(appId, iconKey)
        } catch (e) {
            return ""
        }
    }
    // qmllint enable unqualified

    HarborStateLayer {
        anchors.fill: parent
        pageState: AppState.pageState("activity")
        // Page-empty (no activities at all) is distinct from a filter that
        // matches nothing; the in-card empty state handles the latter.
        title: pageState === "empty" ? I18n.t("activity.pageEmpty.title")
             : pageState === "error" ? I18n.t("state.error.title") : ""
        description: pageState === "empty" ? I18n.t("activity.pageEmpty.description")
                  : pageState === "error" ? I18n.t("state.error.description") : ""
        actionText: pageState === "error" ? I18n.t("common.actions.retry") : ""
        onActionTriggered: {
            if (root.hasCore)
                HarborCore.retryCore()
            else
                MockController.transitionPage("activity", "content")
        }

        HarborPage {
            width: parent.width
            height: parent.height
            accessibleName: I18n.t("sidebar.activity")

            HarborPageHeader {
                title: I18n.t("activity.title")
                subtitle: I18n.t("activity.subtitle", { name: AppState.partnerName })
                iconName: "activity"

                // Simulated activity is a test-provider affordance only; the
                // real engine announces itself instead of faking moments.
                HarborButton {
                    variant: "secondary"
                    text: I18n.t("activity.simulate")
                    visible: !root.hasCore
                    onClicked: AppState.simulateActivity()
                }
            }

            HarborSectionCard {
                Layout.fillWidth: true

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.sp3

                    HarborInput {
                        id: searchInput

                        Layout.fillWidth: true
                        placeholderText: I18n.t("activity.search.placeholder")
                        leadingIcon: "search"
                        clearable: true
                        onTextChanged: root.searchText = text
                    }
                }

                // Exclusive filter chips: checkable stays off so the checked
                // binding remains the single source of truth.
                Flow {
                    Layout.fillWidth: true
                    spacing: Theme.sp2

                    Repeater {
                        model: root.filterModel

                        HarborChoiceChip {
                            id: filterChip

                            required property var modelData

                            checkable: false
                            compact: true
                            text: I18n.t(filterChip.modelData.labelKey)
                            iconName: filterChip.modelData.icon
                            checked: root.activeFilter === filterChip.modelData.key
                            onClicked: root.activeFilter = filterChip.modelData.key
                        }
                    }
                }
            }

            HarborSectionCard {
                Layout.fillWidth: true
                title: I18n.t(root.hasCore ? "activity.remote.title"
                                           : "activity.local.title",
                             { name: AppState.partnerName })

                headerActions: HarborBadge {
                        text: I18n.t("common.count.moments", { count: root.filteredActivities.length })
                        tone: "neutral"
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp2
                        visible: root.filteredActivities.length > 0

                        Repeater {
                            model: root.filteredActivities

                            delegate: HarborActivityItem {
                                id: activityDelegate

                                required property var modelData

                                Layout.fillWidth: true
                                category: activityDelegate.modelData.category
                                title: root.activityTitle(activityDelegate.modelData)
                                description: root.activityDescription(activityDelegate.modelData)
                                time: activityDelegate.modelData.time
                                appId: activityDelegate.modelData.appId || ""
                                iconKey: activityDelegate.modelData.iconKey || ""
                                iconUrl: root.appIconFor(activityDelegate.modelData)
                                onClicked: {
                                    root.searchText = ""
                                    root.activeFilter = "all"
                                }
                            }
                        }
                    }

                    HarborEmptyState {
                        Layout.fillWidth: true
                        visible: root.filteredActivities.length === 0
                        iconName: "activity"
                        title: I18n.t("activity.empty.title")
                        description: I18n.t("activity.empty.description")
                    }
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    Layout.preferredWidth: 290
                    spacing: Theme.sp4

                    HarborSectionCard {
                        Layout.fillWidth: true

                        HarborToggle {
                            Layout.fillWidth: true
                            text: I18n.t("activity.sharing.title")
                            description: I18n.t("activity.sharing.description", { name: AppState.partnerName })
                            checked: AppState.activitySharing
                            onToggled: AppState.activitySharing = checked
                        }
                    }

                    // A peer's records are rendered separately from local
                    // history. The core validates and redacts them before
                    // this metadata-only snapshot enters QML.
                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("activity.remote.title", { name: AppState.partnerName })
                        iconName: "activity"

                        HarborEmptyState {
                            Layout.fillWidth: true
                            visible: AppState.remoteActivities.length === 0
                            iconName: "activity"
                            title: I18n.t("activity.remote.empty.title")
                            description: I18n.t("activity.remote.empty.description",
                                                { name: AppState.partnerName })
                        }

                        Repeater {
                            model: AppState.remoteActivities

                            delegate: HarborCard {
                                id: remoteActivityCard

                                required property var modelData

                                Layout.fillWidth: true
                                objectName: "remoteActivity-" + modelData.id

                                RowLayout {
                                    Layout.fillWidth: true
                                    spacing: Theme.sp2

                                    // Real peer app icon when the sending
                                    // device resolved one; otherwise the
                                    // generic category glyph. Never a path.
                                    Image {
                                        Layout.preferredWidth: 24
                                        Layout.preferredHeight: 24
                                        source: root.appIconFor(remoteActivityCard.modelData)
                                        visible: source.toString().length > 0
                                        fillMode: Image.PreserveAspectFit
                                        smooth: true
                                        cache: true
                                        asynchronous: true
                                        Accessible.ignored: true
                                    }

                                    HarborIcon {
                                        name: remoteActivityCard.modelData.category === "game"
                                              ? "game" : remoteActivityCard.modelData.category === "app"
                                                ? "app" : "activity"
                                        color: Theme.iconSecondary
                                        Accessible.ignored: true
                                        visible: root.appIconFor(remoteActivityCard.modelData).length === 0
                                    }

                                    ColumnLayout {
                                        Layout.fillWidth: true
                                        spacing: 0

                                        Text {
                                            Layout.fillWidth: true
                                            text: remoteActivityCard.modelData.label.length > 0
                                                  ? remoteActivityCard.modelData.label
                                                  : I18n.t("activity.remote.kind."
                                                           + remoteActivityCard.modelData.kind)
                                            color: Theme.textPrimary
                                            font.family: Theme.fontFamily
                                            font.pixelSize: Theme.fontSmall
                                            font.weight: Font.DemiBold
                                            elide: Text.ElideRight
                                        }

                                        Text {
                                            Layout.fillWidth: true
                                            text: remoteActivityCard.modelData.sender
                                            color: Theme.textFaint
                                            font.family: Theme.fontFamily
                                            font.pixelSize: Theme.fontTiny
                                            elide: Text.ElideRight
                                        }
                                    }

                                    Text {
                                        text: remoteActivityCard.modelData.time
                                        color: Theme.textFaint
                                        font.family: Theme.fontFamilyMonospace
                                        font.pixelSize: Theme.fontTiny
                                    }
                                }
                            }
                        }
                    }

                    HarborSectionCard {
                        Layout.fillWidth: true
                        // Fixture aggregates are a test affordance only.
                        visible: !root.hasCore
                        title: I18n.t("activity.week.title")

                        RowLayout {
                            Layout.fillWidth: true

                            Repeater {
                                model: root.weekStats

                                ColumnLayout {
                                    id: weekStat

                                    required property var modelData

                                    Layout.fillWidth: true
                                    spacing: Theme.sp1

                                    Text {
                                        Layout.fillWidth: true
                                        text: weekStat.modelData.value
                                        color: Theme.actionPrimary
                                        font.family: Theme.fontFamilyDisplay
                                        font.pixelSize: Theme.fontTitle
                                        font.weight: Font.Bold
                                        horizontalAlignment: Text.AlignHCenter
                                    }
                                    Text {
                                        Layout.fillWidth: true
                                        text: I18n.t(weekStat.modelData.labelKey)
                                        color: Theme.textFaint
                                        font.family: Theme.fontFamily
                                        font.pixelSize: Theme.fontTiny
                                        horizontalAlignment: Text.AlignHCenter
                                    }
                                }
                            }
                        }
                    }

                    HarborSectionCard {
                        Layout.fillWidth: true
                        title: I18n.t("activity.privacy.title")

                        Text {
                            Layout.fillWidth: true
                            text: I18n.t("activity.privacy.description")
                            color: Theme.textSecondary
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontSmall
                            wrapMode: Text.WordWrap
                        }
                    }
                }
            }
        }
    }
