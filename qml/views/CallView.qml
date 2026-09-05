import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Harbor 2.0

// A private voice space for two people: two avatars, essential controls,
// and screen share as a child of the call — never a separate page.
// Layout states: NORMAL, SCREEN_SHARE, SCREEN_SHARE_CHAT,
// SCREEN_SHARE_FULLSCREEN, SCREEN_SHARE_FULLSCREEN_CHAT.
Item {
    id: root

    readonly property bool compact: width < Theme.breakpointCompact
    readonly property bool activeCall: AppState.callState === "connected"
    readonly property bool sharing: AppState.callShareState === "SHARING"
    readonly property bool hasCore: typeof HarborCore !== "undefined"

    // qmllint disable unqualified
    readonly property bool liveCall: typeof HarborCore !== "undefined" && HarborCore.coreReady
    // qmllint enable unqualified

    HarborCallBridge {
        id: realCall

        // qmllint disable unqualified
        facade: root.liveCall ? HarborCore : null
        // qmllint enable unqualified
    }

    // Reconnects remain on the real provider; fixtures are test-only.
    readonly property var callProvider: root.hasCore ? realCall : MockController

    HarborDirectBridge {
        id: realDirect

        // qmllint disable unqualified
        facade: root.liveCall ? HarborCore : null
        // qmllint enable unqualified
    }

    readonly property var directProvider: root.hasCore ? realDirect : MockController

    property bool sideChatOpen: false
    property bool shareFullscreen: false

    // The share surface replaces the avatars; opening the side chat keeps
    // the share alive beside the conversation.
    readonly property string callLayout: {
        if (!root.sharing)
            return "NORMAL"
        if (root.shareFullscreen)
            return root.sideChatOpen ? "SCREEN_SHARE_FULLSCREEN_CHAT" : "SCREEN_SHARE_FULLSCREEN"
        return root.sideChatOpen ? "SCREEN_SHARE_CHAT" : "SCREEN_SHARE"
    }

    // A held PTT request is valid only in a connected, unmuted call. The
    // core repeats this policy at the media boundary, so mute always wins.
    readonly property bool pttAvailable: activeCall
                                       && AppState.connectionState === "connected"
                                       && !AppState.microphoneMuted
                                       && AppState.pushToTalkEnabled

    // The simulation labels itself as one; a real call must never borrow
    // those strings, so each provider reads its own honest status family.
    readonly property string statusFamily: root.hasCore ? "call.status.live" : "call.status"

    // The persisted key is a Qt key code string (legacy "Space" /
    // "Return" / "Enter" names still read). Return and Enter stay as
    // reachable fallbacks while the PTT control holds focus.
    function pttKeyCode() {
        var raw = String(AppState.pushToTalkKey)
        if (raw === "Space")
            return Qt.Key_Space
        if (raw === "Return")
            return Qt.Key_Return
        if (raw === "Enter")
            return Qt.Key_Enter
        var code = parseInt(raw, 10)
        return isNaN(code) ? Qt.Key_Space : code
    }

    function isPushToTalkKey(key) {
        return key === pttKeyCode() || key === Qt.Key_Return || key === Qt.Key_Enter
    }

    function statusKey() {
        if (AppState.callState === "connected") {
            // The worker refuses calls it cannot open devices for, so a live
            // "connected" means capture and playback are genuinely running.
            return AppState.microphoneMuted ? root.statusFamily + ".muted"
                                            : root.statusFamily + ".connected"
        }
        if (AppState.callState === "connecting") return root.statusFamily + ".opening"
        if (AppState.callState === "incoming") return root.statusFamily + ".incoming"
        if (AppState.callState === "unavailable")
            return root.liveCall ? "call.status.live.failed" : "call.status.unavailable"
        return root.statusFamily + ".ready"
    }

    function statusText() {
        if (AppState.callState === "incoming")
            return I18n.t(statusKey(), { name: AppState.partnerName })
        if (AppState.callState === "unavailable")
            return root.liveCall
                    ? I18n.t("call.status.live.failed")
                    : I18n.t("call.status.unavailable", { name: AppState.partnerName })
        return I18n.t(statusKey())
    }

    function statusTone() {
        if (AppState.callState === "connected") return AppState.microphoneMuted ? "warning" : "success"
        if (AppState.callState === "connecting" || AppState.callState === "incoming") return "accent"
        if (AppState.callState === "unavailable") return "danger"
        return "neutral"
    }

    function formatTime(epochSeconds) {
        var date = new Date(Number(epochSeconds) * 1000)
        if (isNaN(date.getTime()))
            return ""
        return Qt.formatTime(date)
    }

    // Ending or losing the call always ends the share side state with it.
    onActiveCallChanged: {
        if (!activeCall) {
            sideChatOpen = false
            shareFullscreen = false
        }
    }

    HarborStateLayer {
        anchors.fill: parent
        pageState: AppState.pageState("call")
        title: pageState === "empty" ? I18n.t("state.empty.title")
             : pageState === "error" ? I18n.t("state.error.title") : ""
        description: pageState === "error" ? I18n.t("state.error.description") : ""
        actionText: pageState === "error" ? I18n.t("common.actions.retry") : ""
        onActionTriggered: {
            if (root.hasCore)
                HarborCore.retryCore()
            else
                MockController.transitionPage("call", "content")
        }

        HarborPage {
            width: parent.width
            height: parent.height
            accessibleName: I18n.t("sidebar.call")

            HarborPageHeader {
                title: I18n.t("call.title")
                subtitle: I18n.t("call.subtitle")
                iconName: "mic"

                HarborBadge {
                    tone: root.statusTone()
                    showDot: true
                    text: root.statusText()
                    Accessible.description: I18n.t("a11y.callStatus", { status: root.statusText() })
                }
            }

            // Voice rides a direct peer connection; without the control
            // plane's durable pairing there is nobody to call. The gate is
            // authoritative state, never a fabricated availability.
            HarborSectionCard {
                Layout.fillWidth: true
                visible: !AppState.paired
                objectName: "callUnpairedGate"
                title: I18n.t("call.unpaired.title")
                iconName: "phone"

                Text {
                    Layout.fillWidth: true
                    text: I18n.t("call.unpaired.description")
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontBody
                    wrapMode: Text.WordWrap
                }

                HarborButton {
                    objectName: "callUnpairedPairButton"
                    variant: "primary"
                    iconName: "phone"
                    text: I18n.t("gate.openPairing")
                    onClicked: AppState.openPairing()
                }
            }

            // The ring is not media: the core holds only authenticated offer
            // metadata until the person here explicitly accepts. Busy peers
            // were rejected in signaling before this card could be shown.
            HarborSectionCard {
                Layout.fillWidth: true
                visible: AppState.callState === "incoming"
                title: I18n.t("call.incoming.title")
                iconName: "phone"

                Text {
                    Layout.fillWidth: true
                    text: I18n.t("call.incoming.description", { name: AppState.partnerName })
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontBody
                    wrapMode: Text.WordWrap
                }

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.sp2

                    HarborButton {
                        objectName: "incomingCallAccept"
                        variant: "primary"
                        iconName: "phone"
                        text: I18n.t("call.incoming.accept")
                        onClicked: root.callProvider.acceptIncomingCall()
                    }

                    HarborButton {
                        objectName: "incomingCallDecline"
                        variant: "secondary"
                        text: I18n.t("call.incoming.decline")
                        onClicked: root.callProvider.declineIncomingCall()
                    }
                }
            }

            // ---- Two-person space --------------------------------------
            HarborSectionCard {
                Layout.fillWidth: true
                Layout.fillHeight: root.sharing && root.shareFullscreen
                visible: AppState.paired && AppState.callState !== "incoming"

                // Avatars: the heart of the call. While nobody shares, both
                // people sit side by side in the center.
                RowLayout {
                    Layout.fillWidth: true
                    visible: !root.sharing
                    spacing: Theme.sp4

                    // Partner ------------------------------------------------
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        Item {
                            Layout.fillWidth: true
                            Layout.preferredHeight: root.compact ? 190 : 230

                            Rectangle {
                                anchors.centerIn: parent
                                width: root.compact ? 165 : 200
                                height: width
                                radius: width / 2
                                color: "transparent"
                                border.width: 1
                                border.color: Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, 0.18)
                                scale: root.activeCall && root.callProvider.remoteSpeaking ? 1.05 : 0.96
                                Behavior on scale {
                                    NumberAnimation {
                                        duration: Theme.duration(Theme.motionSlow)
                                        easing.type: Theme.animEasing
                                    }
                                }
                            }

                            HarborAvatar {
                                anchors.centerIn: parent
                                avatarSize: root.compact ? 104 : 128
                                initials: AppState.partnerProfile.initials
                                source: AppState.partnerProfile.avatar
                                avatarType: AppState.partnerProfile.avatarType
                                status: AppState.partnerState
                                speaking: root.activeCall && root.callProvider.remoteSpeaking
                                accessibleName: I18n.t("a11y.avatarFor", { name: AppState.partnerProfile.name })
                            }
                        }

                        Text {
                            Layout.fillWidth: true
                            text: AppState.partnerName.length > 0
                                   ? AppState.partnerProfile.name
                                   : I18n.t("home.partner.unnamed")
                            color: Theme.textPrimary
                            font.family: Theme.fontFamilyDisplay
                            font.pixelSize: Theme.fontTitle
                            font.weight: Font.Bold
                            horizontalAlignment: Text.AlignHCenter
                            elide: Text.ElideRight
                        }

                        Text {
                            Layout.fillWidth: true
                            text: root.activeCall
                                   ? (root.callProvider.remoteSpeaking
                                      ? I18n.t("call.speaking.remote", { name: AppState.partnerName })
                                      : root.statusText())
                                   : root.statusText()
                            color: root.callProvider.remoteSpeaking ? Theme.success : Theme.textSecondary
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontSmall
                            horizontalAlignment: Text.AlignHCenter
                        }
                    }

                    // Self ---------------------------------------------------
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        Item {
                            Layout.fillWidth: true
                            Layout.preferredHeight: root.compact ? 190 : 230

                            Rectangle {
                                anchors.centerIn: parent
                                width: root.compact ? 165 : 200
                                height: width
                                radius: width / 2
                                color: "transparent"
                                border.width: 1
                                border.color: Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, 0.18)
                                scale: root.activeCall && !AppState.microphoneMuted
                                       && root.callProvider.speaking ? 1.05 : 0.96
                                Behavior on scale {
                                    NumberAnimation {
                                        duration: Theme.duration(Theme.motionSlow)
                                        easing.type: Theme.animEasing
                                    }
                                }
                            }

                            HarborAvatar {
                                anchors.centerIn: parent
                                avatarSize: root.compact ? 104 : 128
                                initials: AppState.selfProfile.initials
                                source: AppState.selfProfile.avatar
                                avatarType: AppState.selfProfile.avatarType
                                status: "online"
                                speaking: root.activeCall && !AppState.microphoneMuted
                                          && root.callProvider.speaking
                                muted: root.activeCall && AppState.microphoneMuted
                                accessibleName: I18n.t("a11y.avatarFor", { name: AppState.selfProfile.name })
                            }
                        }

                        Text {
                            Layout.fillWidth: true
                            text: AppState.selfName.length > 0
                                   ? AppState.selfProfile.name
                                   : I18n.t("settings.profile.title")
                            color: Theme.textPrimary
                            font.family: Theme.fontFamilyDisplay
                            font.pixelSize: Theme.fontTitle
                            font.weight: Font.Bold
                            horizontalAlignment: Text.AlignHCenter
                            elide: Text.ElideRight
                        }

                        Text {
                            Layout.fillWidth: true
                            text: !root.activeCall ? root.statusText()
                                   : AppState.microphoneMuted ? I18n.t("common.status.muted")
                                   : AppState.pushToTalkActive ? I18n.t("call.ptt.active")
                                   : I18n.t("call.status.live.connected")
                            color: AppState.microphoneMuted ? Theme.warning : Theme.textSecondary
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontSmall
                            horizontalAlignment: Text.AlignHCenter
                        }
                    }
                }

                // Share surface: while anyone shares, the avatars step aside
                // and the transmission owns the room. Stopping the share
                // never ends the call; ending the call ends both.
                RowLayout {
                    Layout.fillWidth: true
                    Layout.fillHeight: root.shareFullscreen
                    visible: root.sharing
                    spacing: Theme.sp3

                    ColumnLayout {
                        Layout.fillWidth: true
                        Layout.fillHeight: root.shareFullscreen
                        spacing: Theme.sp2

                        Rectangle {
                            id: shareSurface

                            Layout.fillWidth: true
                            Layout.preferredHeight: root.shareFullscreen ? parent.height - shareControlsRow.height - Theme.sp2 : (root.compact ? 260 : 340)
                            Layout.fillHeight: root.shareFullscreen
                            radius: Theme.radiusSmall
                            color: Theme.surfaceSunken
                            border.width: 1
                            border.color: Theme.borderSubtle
                            clip: true
                            Accessible.role: Accessible.Graphic
                            Accessible.name: I18n.t("call.share.active.title")

                            ColumnLayout {
                                anchors.centerIn: parent
                                width: Math.min(parent.width - Theme.sp6, 420)
                                spacing: Theme.sp2

                                HarborIcon {
                                    Layout.alignment: Qt.AlignHCenter
                                    name: "monitor"
                                    color: Theme.accent
                                    implicitWidth: 44
                                    implicitHeight: 44
                                }

                                Text {
                                    Layout.fillWidth: true
                                    text: I18n.t("call.share.active.title")
                                    color: Theme.textPrimary
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontBody
                                    font.weight: Font.DemiBold
                                    horizontalAlignment: Text.AlignHCenter
                                }

                                Text {
                                    Layout.fillWidth: true
                                    text: I18n.t("call.share.active.description", { name: AppState.partnerName })
                                    color: Theme.textSecondary
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontSmall
                                    horizontalAlignment: Text.AlignHCenter
                                    wrapMode: Text.Wrap
                                }
                            }
                        }

                        // Discreet share controls: they stay compact and never
                        // cover the transmission.
                        RowLayout {
                            id: shareControlsRow

                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            HarborIconButton {
                                iconName: "monitor"
                                accessibleName: I18n.t("a11y.stopScreenShare")
                                toolTip: I18n.t("call.share.stop")
                                onClicked: root.callProvider.stopScreenShare()
                            }

                            HarborIconButton {
                                iconName: root.shareFullscreen ? "chevron-down" : "chevron-right"
                                accessibleName: root.shareFullscreen
                                                 ? I18n.t("common.actions.close")
                                                 : I18n.t("common.actions.open")
                                toolTip: accessibleName
                                onClicked: root.shareFullscreen = !root.shareFullscreen
                            }

                            HarborIconButton {
                                iconName: "chat"
                                accessibleName: I18n.t("sidebar.chat")
                                toolTip: I18n.t("sidebar.chat")
                                checked: root.sideChatOpen
                                onClicked: root.sideChatOpen = !root.sideChatOpen
                            }

                            Item { Layout.fillWidth: true }

                            HarborButton {
                                variant: "secondary"
                                text: I18n.t("call.share.stop")
                                onClicked: root.callProvider.stopScreenShare()
                            }
                        }
                    }

                    // Side chat: the conversation beside the transmission.
                    // Resizable by drag, collapsible, reopenable — the share
                    // keeps running either way, including in fullscreen.
                    ColumnLayout {
                        visible: root.sideChatOpen
                        Layout.preferredWidth: root.compact ? 260 : 320
                        Layout.maximumWidth: 420
                        Layout.fillHeight: true
                        spacing: Theme.sp2

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            Text {
                                Layout.fillWidth: true
                                text: I18n.t("sidebar.chat")
                                color: Theme.textPrimary
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontBody
                                font.weight: Font.DemiBold
                            }

                            HarborIconButton {
                                iconName: "close"
                                buttonSize: 32
                                accessibleName: I18n.t("common.actions.close")
                                onClicked: root.sideChatOpen = false
                            }
                        }

                        ListView {
                            id: sideTranscript

                            Layout.fillWidth: true
                            Layout.fillHeight: true
                            Layout.preferredHeight: 220
                            clip: true
                            spacing: Theme.sp2
                            model: AppState.chatMessages
                            verticalLayoutDirection: ListView.BottomToTop
                            Accessible.role: Accessible.List
                            Accessible.name: I18n.t("sidebar.chat")

                            delegate: Item {
                                id: sideRow

                                required property var modelData

                                width: ListView.view.width
                                height: sideBubble.height + 4

                                Rectangle {
                                    id: sideBubble

                                    readonly property bool outgoing:
                                        sideRow.modelData.direction === "OUTGOING"

                                    x: outgoing ? parent.width - width : 0
                                    width: Math.min(parent.width - 16,
                                                    sideText.implicitWidth + Theme.sp4)
                                    height: sideText.paintedHeight + sideMeta.height + Theme.sp3
                                    radius: Theme.radiusSmall
                                    color: outgoing ? Theme.surfaceStrong : Theme.surface
                                    border.width: 1
                                    border.color: Theme.borderSubtle

                                    Text {
                                        id: sideText

                                        x: Theme.sp2
                                        y: Theme.sp1
                                        width: parent.width - Theme.sp4
                                        text: sideRow.modelData.body
                                        color: Theme.text
                                        font.family: Theme.fontFamily
                                        font.pixelSize: Theme.fontSmall
                                        wrapMode: Text.WordWrap
                                    }

                                    Text {
                                        id: sideMeta

                                        anchors.right: parent.right
                                        anchors.bottom: parent.bottom
                                        anchors.margins: 4
                                        text: sideBubble.outgoing
                                              ? I18n.t("chat.delivery."
                                                       + String(sideRow.modelData.delivery))
                                              : root.formatTime(sideRow.modelData.timestamp)
                                        color: Theme.textFaint
                                        font.family: Theme.fontFamily
                                        font.pixelSize: Theme.fontTiny
                                    }
                                }
                            }
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp2

                            HarborInput {
                                id: sideComposer

                                Layout.fillWidth: true
                                placeholderText: I18n.t("chat.composer.placeholderGeneric")
                                onAccepted: {
                                    var body = text.trim()
                                    if (body.length === 0)
                                        return
                                    root.directProvider.sendMessage(body)
                                    text = ""
                                }
                            }

                            HarborButton {
                                variant: "primary"
                                iconName: "send"
                                text: I18n.t("chat.composer.send")
                                onClicked: sideComposer.accepted()
                            }
                        }
                    }
                }

                // Voice strip: microphone, push-to-talk, share, call. Device
                // choice, volumes and voice activation live in Settings.
                Text {
                    Layout.fillWidth: true
                    visible: root.activeCall && !root.sharing
                    text: AppState.callTime
                    color: Theme.textFaint
                    font.family: Theme.fontFamilyMonospace
                    font.pixelSize: Theme.fontSmall
                    font.features: { "tnum": 1 }
                    horizontalAlignment: Text.AlignHCenter
                }

                HarborAudioVisualizer {
                    Layout.fillWidth: true
                    Layout.preferredHeight: 32
                    visible: root.activeCall && !root.sharing
                    barCount: 24
                    level: root.activeCall
                           ? (AppState.microphoneMuted ? 0
                              : root.callProvider.microphoneLevel)
                           : 0
                    running: root.activeCall && !AppState.microphoneMuted
                    accessibleName: I18n.t("call.visualizer.notice")
                }

                RowLayout {
                    Layout.alignment: Qt.AlignHCenter
                    visible: AppState.paired && !root.sharing
                    spacing: Theme.sp3

                    HarborIconButton {
                        visible: root.activeCall
                        buttonSize: 58
                        iconName: AppState.microphoneMuted ? "mic-off" : "mic"
                        iconColor: AppState.microphoneMuted ? Theme.warning : Theme.iconSecondary
                        accessibleName: AppState.microphoneMuted
                                       ? I18n.t("a11y.unmuteMicrophone")
                                       : I18n.t("a11y.muteMicrophone")
                        toolTip: accessibleName
                        onClicked: root.callProvider.toggleMute()
                    }

                    HarborIconButton {
                        id: shareControl

                        visible: root.activeCall
                        enabled: !root.callProvider.shareBusy
                        buttonSize: 58
                        objectName: "shareControl"
                        iconName: "monitor"
                        iconColor: Theme.iconSecondary
                        accessibleName: I18n.t("a11y.startScreenShare")
                        toolTip: accessibleName
                        onClicked: root.callProvider.startScreenShare()
                    }

                    HarborButton {
                        Layout.preferredWidth: root.activeCall ? 150 : 190
                        variant: root.activeCall ? "danger" : "primary"
                        busy: AppState.callState === "connecting"
                        text: AppState.callState === "connecting" ? I18n.t("call.action.connecting")
                              : root.activeCall ? I18n.t("call.action.end")
                              : I18n.t("call.action.start")
                        onClicked: {
                            if (root.activeCall)
                                root.callProvider.endCall()
                            else
                                root.callProvider.startCall()
                        }
                    }

                    // Push-to-talk: press-and-hold with pointer or
                    // Space/Enter. A screen-reader activation sends one
                    // bounded transmission owned by the controller.
                    Control {
                        id: pttControl

                        objectName: "pttControl"

                        property bool pointerHeld: false

                        // The hold control only exists while the feature
                        // is enabled; a disabled PTT shows nothing here
                        // instead of a dead button.
                        visible: root.activeCall && AppState.pushToTalkEnabled
                        enabled: root.pttAvailable
                        focusPolicy: Qt.StrongFocus
                        implicitWidth: 58
                        implicitHeight: 58

                        Accessible.role: Accessible.Button
                        Accessible.name: I18n.t("a11y.pushToTalk")
                        Accessible.description: AppState.pushToTalkActive
                            ? I18n.t("call.ptt.active") : I18n.t("call.ptt.hold")
                        Accessible.onPressAction: root.callProvider.pulsePushToTalk(800)

                        onActiveFocusChanged: {
                            if (!activeFocus)
                                root.callProvider.setPushToTalk(false)
                        }
                        Component.onDestruction: root.callProvider.setPushToTalk(false)

                        Keys.onPressed: event => {
                            if (root.isPushToTalkKey(event.key)
                                    && !event.isAutoRepeat && root.pttAvailable) {
                                root.callProvider.setPushToTalk(true)
                                event.accepted = true
                            }
                        }
                        Keys.onReleased: event => {
                            if (root.isPushToTalkKey(event.key)) {
                                root.callProvider.setPushToTalk(false)
                                event.accepted = true
                            }
                        }

                        background: Rectangle {
                            radius: 29
                            color: pttControl.pointerHeld ? Theme.surfacePressed
                                   : AppState.pushToTalkActive ? Theme.accentDeep
                                   : Theme.surfaceStrong
                            border.width: pttControl.visualFocus ? Theme.focusWidth : 1
                            border.color: pttControl.visualFocus ? Theme.focusRing
                                       : AppState.pushToTalkActive ? Theme.accent : Theme.surfaceBorder
                        }

                        contentItem: ColumnLayout {
                            spacing: 0

                            Text {
                                Layout.alignment: Qt.AlignHCenter
                                text: I18n.t("call.ptt.label")
                                color: AppState.pushToTalkActive ? "white" : Theme.textPrimary
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontTiny
                                font.weight: Font.Bold
                            }
                            HarborIcon {
                                Layout.alignment: Qt.AlignHCenter
                                implicitWidth: 12
                                implicitHeight: 12
                                name: AppState.pushToTalkActive ? "mic" : "chevron-down"
                                color: AppState.pushToTalkActive ? "white" : Theme.iconSecondary
                                Accessible.ignored: true
                            }
                        }

                        MouseArea {
                            anchors.fill: parent
                            enabled: pttControl.enabled
                            cursorShape: Qt.PointingHandCursor
                            onPressed: {
                                pttControl.forceActiveFocus()
                                pttControl.pointerHeld = true
                                root.callProvider.setPushToTalk(true)
                            }
                            onReleased: {
                                pttControl.pointerHeld = false
                                root.callProvider.setPushToTalk(false)
                            }
                            onCanceled: {
                                pttControl.pointerHeld = false
                                root.callProvider.setPushToTalk(false)
                            }
                        }
                    }
                }

                // Essential voice levels without leaving the call. Full
                // device choice lives in Settings > Audio.
                RowLayout {
                    Layout.fillWidth: true
                    visible: root.activeCall && !root.sharing
                    spacing: Theme.sp3

                    HarborSlider {
                        Layout.fillWidth: true
                        label: I18n.t("call.audio.microphone")
                        from: 0
                        to: 100
                        unit: "%"
                        value: Math.round(AppState.microphoneVolume * 100)
                        onMoved: {
                            if (root.liveCall)
                                root.callProvider.setAudioVolumes(value / 100,
                                                                  AppState.outputVolume)
                            else
                                AppState.microphoneVolume = value / 100
                        }
                    }

                    HarborSlider {
                        Layout.fillWidth: true
                        label: I18n.t("call.audio.output")
                        from: 0
                        to: 100
                        unit: "%"
                        value: Math.round(AppState.outputVolume * 100)
                        onMoved: {
                            if (root.liveCall)
                                root.callProvider.setAudioVolumes(AppState.microphoneVolume,
                                                                  value / 100)
                            else
                                AppState.outputVolume = value / 100
                        }
                    }
                }

                HarborButton {
                    Layout.alignment: Qt.AlignHCenter
                    visible: root.activeCall && !root.sharing
                    variant: "quiet"
                    text: I18n.t("call.audio.settings")
                    onClicked: AppState.navigate("settings")
                }
            }
        }
    }
}
