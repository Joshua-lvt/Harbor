pragma ComponentBehavior: Bound

import Harbor 2.0
import QtQuick
import QtQuick.Layouts
import QtQuick.Dialogs

// Real, persistent profile: display name, status message and avatar
// (image or animated GIF) flow through AppState into the durable core
// settings, so they survive restarts. The same session profile feeds Home,
// Call, Chat and Pairing — never a per-screen copy. Identity (device,
// Harbor ID) and partner state are separate facts shown in their own cards.

HarborStateLayer {
    id: root

    property bool editing: false
    property string draftAvatar: ""
    property string draftAvatarType: "image"
    property string avatarError: ""
    property bool copyFeedbackVisible: false

    readonly property bool compact: width < Theme.breakpointCompact
    readonly property bool hasCore: typeof HarborCore !== "undefined"
    readonly property bool liveCore: hasCore && HarborCore.coreReady
    readonly property string draftName: nameInput.text.trim()
    readonly property string draftStatus: statusInput.text.trim()
    readonly property string effectiveInitials: root.editing && root.draftName.length >= 2
        ? AppState.initialsFor(root.draftName)
        : AppState.selfProfile.initials
    readonly property url effectiveAvatar: root.editing ? root.draftAvatar
                                                         : AppState.selfProfile.avatar
    readonly property string effectiveAvatarType: root.editing ? root.draftAvatarType
                                                               : AppState.selfProfile.avatarType

    function seedDrafts() {
        nameInput.text = AppState.selfProfile.name
        statusInput.text = AppState.selfProfile.status
        draftAvatar = String(AppState.selfProfile.avatar || "")
        draftAvatarType = AppState.selfProfile.avatarType === "gif" ? "gif" : "image"
        avatarError = ""
    }

    function avatarTypeFor(dataUrl) {
        return String(dataUrl || "").indexOf("data:image/gif") === 0 ? "gif" : "image"
    }

    function importAvatar(fileUrl) {
        avatarError = ""
        if (!root.liveCore) {
            avatarError = I18n.t("profile.avatar.unavailable")
            return
        }
        var normalized = HarborCore.importProfileAvatar(fileUrl)
        if (!normalized) {
            avatarError = I18n.t("profile.avatar.invalid")
            return
        }
        // A GIF keeps its animation: the native layer stores the original
        // bytes, and the avatar renders them animated on every surface.
        draftAvatar = normalized
        draftAvatarType = root.avatarTypeFor(normalized)
    }

    function clearAvatar() {
        draftAvatar = ""
        draftAvatarType = "image"
        avatarError = ""
    }

    function saveProfile() {
        var patch = {
            status: root.draftStatus,
            avatar: root.draftAvatar,
            avatarType: root.draftAvatarType
        }
        if (root.draftName.length > 0)
            patch.name = root.draftName
        AppState.updateSelfProfile(patch)
        if (!root.hasCore)
            MockController.queueLocalizedToast(
                "system", "profile.updated.title", {},
                "profile.updated.description", {})
    }

    Timer {
        id: copyFeedbackTimer
        interval: 2000
        onTriggered: root.copyFeedbackVisible = false
    }

    pageState: AppState.pageState("profile")
    title: pageState === "error" ? I18n.t("state.error.title") : ""
    description: pageState === "error" ? I18n.t("state.error.description") : ""
    actionText: pageState === "error" ? I18n.t("common.actions.retry") : ""
    onActionTriggered: {
        if (root.hasCore)
            HarborCore.retryCore()
        else
            MockController.transitionPage("profile", "content")
    }

    HarborPage {
        id: page

        width: parent.width
        height: parent.height
        accessibleName: I18n.t("profile.title")

        // Fields start seeded from the durable profile; drafts are re-seeded
        // every time editing begins so a cancelled edit never leaks.
        Component.onCompleted: root.seedDrafts()

        FileDialog {
            id: avatarDialog

            title: I18n.t("profile.avatar.choose")
            fileMode: FileDialog.OpenFile
            nameFilters: [I18n.t("profile.avatar.images")]
            onAccepted: root.importAvatar(selectedFile)
        }

        HarborPageHeader {
            title: I18n.t("profile.title")
            subtitle: I18n.t("profile.subtitle")
            iconName: "user"

            HarborButton {
                objectName: "profileEditButton"
                text: root.editing ? I18n.t("profile.save") : I18n.t("profile.edit")
                variant: root.editing ? "primary" : "secondary"
                onClicked: {
                    if (root.editing)
                        root.saveProfile()
                    else
                        root.seedDrafts()
                    root.editing = !root.editing
                }
            }
        }

        // ---- Who you are ------------------------------------------------
        HarborSectionCard {
            Layout.fillWidth: true
            Layout.maximumWidth: Theme.maxPageWidth
            title: I18n.t("profile.displayName")
            iconName: "user"

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.sp4

                // Drag an image or GIF straight onto the avatar while editing.
                DropArea {
                    Layout.preferredWidth: root.compact ? 76 : 96
                    Layout.preferredHeight: root.compact ? 76 : 96
                    enabled: root.editing
                    onEntered: drag => {
                        if (!drag.hasUrls || drag.urls.length === 0)
                            drag.accepted = false
                    }
                    onDropped: drop => {
                        if (drop.hasUrls && drop.urls.length > 0)
                            root.importAvatar(drop.urls[0])
                    }

                    HarborAvatar {
                        anchors.fill: parent
                        initials: root.effectiveInitials
                        source: root.effectiveAvatar
                        avatarType: root.effectiveAvatarType
                        status: "online"
                        showStatus: false
                        accessibleName: AppState.selfProfile.name
                    }
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: Theme.sp1

                    Text {
                        Layout.fillWidth: true
                        text: root.editing && root.draftName.length > 0
                              ? root.draftName : AppState.selfProfile.name
                        color: Theme.textPrimary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontTitle
                        font.weight: Font.Bold
                        elide: Text.ElideRight
                    }

                    Text {
                        Layout.fillWidth: true
                        text: root.editing && root.draftStatus.length > 0
                              ? root.draftStatus
                              : (AppState.selfProfile.status.length > 0
                                 ? AppState.selfProfile.status
                                 : I18n.t("profile.noStatus"))
                        color: Theme.textSecondary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontBody
                        wrapMode: Text.Wrap
                        maximumLineCount: 2
                        elide: Text.ElideRight
                    }
                }
            }

            HarborInput {
                id: nameInput

                Layout.fillWidth: true
                label: I18n.t("profile.displayName")
                placeholderText: I18n.t("profile.displayNamePlaceholder")
                readOnly: !root.editing
                onAccepted: statusInput.forceInputFocus()
            }

            HarborInput {
                id: statusInput

                Layout.fillWidth: true
                label: I18n.t("profile.statusMessage")
                placeholderText: I18n.t("profile.statusPlaceholder")
                readOnly: !root.editing
                onAccepted: root.forceActiveFocus()
            }

            RowLayout {
                Layout.fillWidth: true
                visible: root.editing
                spacing: Theme.sp2

                HarborButton {
                    text: I18n.t("profile.changeAvatar")
                    variant: "secondary"
                    onClicked: avatarDialog.open()
                }

                HarborButton {
                    visible: root.draftAvatar.length > 0
                    text: I18n.t("profile.avatar.clear")
                    variant: "quiet"
                    onClicked: root.clearAvatar()
                }
            }

            Text {
                visible: root.avatarError.length > 0
                Layout.fillWidth: true
                text: root.avatarError
                color: Theme.danger
                font.family: Theme.fontFamily
                font.pixelSize: Theme.fontTiny
                wrapMode: Text.Wrap
            }
        }

        // ---- Harbor identity --------------------------------------------
        HarborSectionCard {
            Layout.fillWidth: true
            Layout.maximumWidth: Theme.maxPageWidth
            title: I18n.t("profile.identity.title")
            iconName: "lock"

            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.sp3

                Text {
                    Layout.fillWidth: true
                    text: AppState.harborId.length > 0
                          ? AppState.harborId
                          : I18n.t("common.notAvailable")
                    color: Theme.textPrimary
                    font.family: Theme.fontFamilyMonospace
                    font.pixelSize: Theme.fontBody
                    font.weight: Font.DemiBold
                    elide: Text.ElideRight
                }

                HarborIconButton {
                    iconName: "check"
                    accessibleName: I18n.t("common.actions.copy")
                    toolTip: I18n.t("common.actions.copy")
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

            Text {
                visible: root.copyFeedbackVisible
                text: I18n.t("profile.identity.copyFeedback")
                color: Theme.success
                font.family: Theme.fontFamily
                font.pixelSize: Theme.fontTiny
            }

            Text {
                Layout.fillWidth: true
                text: I18n.t("profile.identity.description")
                color: Theme.textMuted
                font.family: Theme.fontFamily
                font.pixelSize: Theme.fontTiny
                wrapMode: Text.Wrap
            }
        }

        // ---- Partner ------------------------------------------------------
        HarborSectionCard {
            Layout.fillWidth: true
            Layout.maximumWidth: Theme.maxPageWidth
            title: I18n.t("profile.pairedWith")
            iconName: "user"

            RowLayout {
                Layout.fillWidth: true
                visible: AppState.paired
                spacing: Theme.sp3

                HarborAvatar {
                    initials: AppState.partnerProfile.initials
                    source: AppState.partnerProfile.avatar
                    avatarType: AppState.partnerProfile.avatarType
                    status: AppState.partnerState
                    avatarSize: 48
                    accessibleName: AppState.partnerProfile.name
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 1

                    Text {
                        Layout.fillWidth: true
                        text: AppState.partnerName.length > 0
                              ? AppState.partnerProfile.name
                              : I18n.t("home.partner.unnamed")
                        color: Theme.textPrimary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontBody
                        font.weight: Font.DemiBold
                        elide: Text.ElideRight
                    }

                    Text {
                        Layout.fillWidth: true
                        text: I18n.t("common.status." + AppState.partnerState)
                        color: AppState.partnerState === "online" ? Theme.success : Theme.textSecondary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontSmall
                        elide: Text.ElideRight
                    }
                }
            }

            ColumnLayout {
                Layout.fillWidth: true
                visible: !AppState.paired
                spacing: Theme.sp2

                Text {
                    Layout.fillWidth: true
                    text: I18n.t("profile.noPartner.title")
                    color: Theme.textPrimary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontBody
                    font.weight: Font.DemiBold
                }

                Text {
                    Layout.fillWidth: true
                    text: I18n.t("profile.noPartner.description")
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontSmall
                    wrapMode: Text.Wrap
                }

                HarborButton {
                    variant: "primary"
                    iconName: "phone"
                    text: I18n.t("gate.openPairing")
                    onClicked: AppState.openPairing()
                }
            }
        }
    }
}
