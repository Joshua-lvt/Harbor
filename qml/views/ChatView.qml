pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Dialogs
import QtQuick.Layouts
import Harbor 2.0

Item {
    id: root

    readonly property bool compact: width < Theme.breakpointCompact
    // The direct channels ride the call's peer connection; without it the
    // composer still queues locally (WAITING_FOR_CONNECTION), so the page
    // stays usable but says so honestly.
    readonly property bool channelOpen: AppState.connectionState === "connected"
        && AppState.callState === "connected"

    readonly property bool hasCore: typeof HarborCore !== "undefined"
    // qmllint disable unqualified
    readonly property bool liveDirect: root.hasCore && HarborCore.coreReady
    // qmllint enable unqualified

    HarborDirectBridge {
        id: realDirect

        // qmllint disable unqualified
        facade: root.liveDirect ? HarborCore : null
        // qmllint enable unqualified
    }

    // Reconnects must not expose the fixture provider in a live session.
    readonly property var directProvider: root.hasCore ? realDirect : MockController

    function formatSize(bytes) {
        var value = Number(bytes) || 0
        if (value < 1024)
            return value + " B"
        if (value < 1024 * 1024)
            return (value / 1024).toFixed(1) + " KB"
        if (value < 1024 * 1024 * 1024)
            return (value / (1024 * 1024)).toFixed(1) + " MB"
        return (value / (1024 * 1024 * 1024)).toFixed(2) + " GB"
    }

    function formatTime(epochSeconds) {
        var date = new Date(Number(epochSeconds) * 1000)
        if (isNaN(date.getTime()))
            return ""
        return Qt.formatTime(date)
    }

    function localPath(url) {
        var text = String(url)
        if (text.indexOf("file://") === 0)
            return decodeURIComponent(text.substring(7))
        return text
    }

    function isPreviewableImage(name) {
        return /\.(png|jpe?g|gif|webp|bmp)$/i.test(String(name || ""))
    }

    // Thumbnail for a verified, completed image/GIF in its destination.
    // Only metadata the view already holds (directory + safe name) is used.
    function previewUrl(name) {
        if (!root.liveDirect)
            return ""
        var dir = String(realDirect.transferDirectory || "")
        if (dir.length === 0)
            return ""
        var clean = dir.replace(/\/+$/, "") + "/" + String(name || "")
        return "file://" + clean
    }

    function offerFile(url) {
        var path = root.localPath(url)
        if (path.length === 0)
            return
        root.directProvider.offerFile(path)
    }

    // One file at a time: the core's offer flow is per-file on purpose.
    DropArea {
        anchors.fill: parent
        onEntered: (drag) => {
            if (!drag.hasUrls || drag.urls.length === 0)
                drag.accepted = false
        }
        onDropped: (drop) => {
            if (!drop.hasUrls)
                return
            for (var i = 0; i < drop.urls.length; ++i)
                root.offerFile(drop.urls[i])
        }
    }

    HarborStateLayer {
        anchors.fill: parent
        pageState: "content"

        HarborPage {
            width: parent.width
            height: parent.height
            accessibleName: I18n.t("sidebar.chat")

            HarborPageHeader {
                title: I18n.t("chat.title")
                subtitle: I18n.t("chat.subtitle")
                iconName: "chat"
            }

            // Chat and files ride the paired peer connection; without the
            // control plane's durable relationship there is no channel to
            // open. The gate states that honestly instead of queueing into
            // the void.
            HarborSectionCard {
                Layout.fillWidth: true
                visible: !AppState.paired
                objectName: "chatUnpairedGate"
                title: I18n.t("chat.unpaired.title")
                iconName: "chat"

                Text {
                    Layout.fillWidth: true
                    text: I18n.t("chat.unpaired.description")
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontBody
                    wrapMode: Text.WordWrap
                }

                HarborButton {
                    objectName: "chatUnpairedPairButton"
                    variant: "primary"
                    iconName: "chat"
                    text: I18n.t("gate.openPairing")
                    onClicked: AppState.openPairing()
                }
            }

            // Honest channel line: open with the peer, or queuing locally.
            HarborSectionCard {
                Layout.fillWidth: true

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.sp2

                    // Partner avatar from the single session profile —
                    // animated when the partner uses a GIF.
                    HarborAvatar {
                        avatarSize: 36
                        initials: AppState.partnerProfile.initials
                        source: AppState.partnerProfile.avatar
                        avatarType: AppState.partnerProfile.avatarType
                        status: AppState.partnerState
                        accessibleName: I18n.t("a11y.avatarFor", { name: AppState.partnerProfile.name })
                    }

                    HarborBadge {
                        text: root.channelOpen ? I18n.t("chat.channel.online")
                                               : I18n.t("chat.channel.offline")
                        tone: root.channelOpen ? "success" : "neutral"
                    }

                    Text {
                        Layout.fillWidth: true
                        text: root.channelOpen
                              ? (AppState.partnerName.length > 0
                                 ? I18n.t("chat.connection.connected", { name: AppState.partnerName })
                                 : I18n.t("chat.connection.open"))
                              : I18n.t("chat.connection.waiting")
                        color: Theme.textSecondary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontSmall
                        elide: Text.ElideRight
                    }
                }
            }

            // One conversation column: text, images, GIFs and files all live
            // here as messages. There is no separate Files or Downloads page.
            ColumnLayout {
                Layout.fillWidth: true
                spacing: Theme.sp4

                // Transcript: the metadata-only mirror the core maintains.
                HarborSectionCard {
                    Layout.fillWidth: true
                    Layout.preferredHeight: root.compact ? 420 : 520
                    title: I18n.t("chat.title")

                    HarborEmptyState {
                        Layout.fillWidth: true
                        visible: AppState.chatMessages.length === 0
                        iconName: "chat"
                        title: I18n.t("chat.empty.title")
                        description: I18n.t("chat.empty.description",
                                            { name: AppState.partnerName })
                    }

                    ListView {
                        id: transcript

                        objectName: "chatTranscript"
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        spacing: Theme.sp2
                        clip: true
                        model: AppState.chatMessages
                        // Newest message pinned to the composer; no scroll
                        // bookkeeping needed.
                        verticalLayoutDirection: ListView.BottomToTop
                        Accessible.role: Accessible.List
                        Accessible.name: I18n.t("chat.title")

                        delegate: Item {
                            id: bubbleRow

                            required property var modelData

                            width: ListView.view.width
                            height: bubble.height + Theme.sp2

                            Rectangle {
                                id: bubble

                                readonly property bool outgoing:
                                    bubbleRow.modelData.direction === "OUTGOING"

                                anchors.horizontalCenter: undefined
                                x: outgoing ? parent.width - width : 0
                                width: Math.min(
                                    messageText.implicitWidth + Theme.sp6,
                                    parent.width - Theme.sp8)
                                height: messageText.paintedHeight + deliveryTag.height
                                        + Theme.sp4
                                radius: Theme.radiusSmall
                                color: outgoing ? Theme.surfaceStrong : Theme.surface
                                border.width: 1
                                border.color: Theme.borderSubtle

                                Text {
                                    id: messageText

                                    x: Theme.sp2
                                    y: Theme.sp1
                                    width: bubble.width - Theme.sp4
                                    text: bubbleRow.modelData.body
                                    color: Theme.text
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontBody
                                    wrapMode: Text.WordWrap
                                }

                                Text {
                                    id: deliveryTag

                                    anchors.right: parent.right
                                    anchors.bottom: parent.bottom
                                    anchors.margins: Theme.sp1
                                    text: bubble.outgoing
                                          ? I18n.t("chat.delivery."
                                                   + String(bubbleRow.modelData.delivery))
                                          : root.formatTime(bubbleRow.modelData.timestamp)
                                    color: Theme.textFaint
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontTiny
                                }
                            }
                        }
                    }

                    // Composer: bounded and sanitized in the core; QML only
                    // forwards the raw text.
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        HarborIconButton {
                            iconName: "attach"
                            accessibleName: I18n.t("chat.composer.attach")
                            toolTip: I18n.t("chat.composer.attach")
                            onClicked: attachDialog.open()
                        }

                        HarborInput {
                            id: composer

                            objectName: "chatComposer"
                            Layout.fillWidth: true
                            placeholderText: AppState.partnerName.length > 0
                                ? I18n.t("chat.composer.placeholder",
                                         { name: AppState.partnerName })
                                : I18n.t("chat.composer.placeholderGeneric")
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
                            onClicked: composer.accepted()
                        }
                    }

                    // Files are messages in the same conversation. The core
                    // still owns staging, checksums and transfer policy; this
                    // card exposes only safe metadata and actions. Nothing is
                    // written to its destination before the receiver accepts.
                    Repeater {
                        model: AppState.transfers
                        delegate: HarborCard {
                            id: attachmentCard

                            required property var modelData

                            Layout.fillWidth: true
                            objectName: "transferCard-" + modelData.id

                            readonly property bool incoming:
                                attachmentCard.modelData.direction === "INCOMING"
                            readonly property string transferState:
                                String(attachmentCard.modelData.state)
                            readonly property real transferProgress:
                                Number(attachmentCard.modelData.size) > 0
                                ? Number(attachmentCard.modelData.receivedBytes)
                                  / Number(attachmentCard.modelData.size) : 0
                            readonly property bool isMedia:
                                root.isPreviewableImage(attachmentCard.modelData.name)

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp2

                                // Image/GIF preview once the verified file
                                // sits in its destination.
                                Image {
                                    Layout.fillWidth: true
                                    Layout.preferredHeight: 180
                                    visible: attachmentCard.isMedia
                                             && attachmentCard.transferState === "COMPLETED"
                                    source: root.previewUrl(attachmentCard.modelData.name)
                                    fillMode: Image.PreserveAspectFit
                                    asynchronous: true
                                    Accessible.role: Accessible.Graphic
                                    Accessible.name: attachmentCard.modelData.name
                                }

                                RowLayout {
                                    Layout.fillWidth: true
                                    spacing: Theme.sp2
                                    HarborIcon { name: attachmentCard.isMedia ? "app" : "file"; color: Theme.accent }
                                    ColumnLayout {
                                        Layout.fillWidth: true
                                        Text {
                                            Layout.fillWidth: true
                                            text: attachmentCard.incoming && attachmentCard.transferState === "OFFERED"
                                                  ? I18n.t("chat.transfer.incomingOffer",
                                                           { name: AppState.partnerName })
                                                  : attachmentCard.modelData.name
                                            color: Theme.textPrimary
                                            font.family: Theme.fontFamily
                                            font.pixelSize: Theme.fontSmall
                                            font.weight: Font.DemiBold
                                            elide: Text.ElideMiddle
                                        }
                                        Text {
                                            Layout.fillWidth: true
                                            text: attachmentCard.transferState === "ACTIVE"
                                                  ? I18n.t("chat.transfer.progress", {
                                                      received: root.formatSize(
                                                          attachmentCard.modelData.receivedBytes),
                                                      total: root.formatSize(
                                                          attachmentCard.modelData.size) })
                                                  : attachmentCard.modelData.expired
                                                    ? I18n.t("chat.transfer.expired")
                                                    : attachmentCard.transferState === "OFFERED"
                                                      ? root.formatSize(attachmentCard.modelData.size)
                                                        + " · "
                                                        + I18n.t("chat.transfer.state.OFFERED")
                                                      : I18n.t("chat.transfer.state."
                                                               + attachmentCard.transferState)
                                            color: Theme.textFaint
                                            font.family: Theme.fontFamily
                                            font.pixelSize: Theme.fontTiny
                                        }
                                    }
                                }

                                Rectangle {
                                    Layout.fillWidth: true
                                    implicitHeight: 4
                                    radius: 2
                                    color: Theme.surfaceStrong
                                    visible: attachmentCard.transferState === "ACTIVE"

                                    Rectangle {
                                        anchors.left: parent.left
                                        anchors.top: parent.top
                                        anchors.bottom: parent.bottom
                                        width: parent.width * Math.max(
                                            0, Math.min(1, attachmentCard.transferProgress))
                                        radius: 2
                                        color: Theme.actionPrimary
                                    }
                                }

                                Flow {
                                    Layout.fillWidth: true
                                    spacing: Theme.sp2

                                    HarborButton {
                                        visible: attachmentCard.incoming
                                                 && attachmentCard.transferState === "OFFERED"
                                                 && !attachmentCard.modelData.expired
                                        variant: "primary"
                                        text: I18n.t("chat.transfer.accept")
                                        onClicked: root.directProvider.acceptTransfer(
                                                       attachmentCard.modelData.id)
                                    }

                                    HarborButton {
                                        visible: attachmentCard.incoming
                                                 && attachmentCard.transferState === "OFFERED"
                                                 && !attachmentCard.modelData.expired
                                        variant: "secondary"
                                        text: I18n.t("chat.transfer.decline")
                                        onClicked: root.directProvider.rejectTransfer(
                                                       attachmentCard.modelData.id)
                                    }

                                    HarborButton {
                                        visible: attachmentCard.transferState === "ACTIVE"
                                                 || (attachmentCard.transferState === "OFFERED"
                                                     && !attachmentCard.incoming)
                                        variant: "danger"
                                        text: I18n.t("chat.transfer.cancel")
                                        onClicked: root.directProvider.cancelTransfer(
                                                       attachmentCard.modelData.id)
                                    }
                                }
                            }
                        }
                    }

                    // Transfer destination, chosen in this view.
                    RowLayout {
                        Layout.fillWidth: true
                        visible: root.liveDirect
                        spacing: Theme.sp2

                        Text {
                            Layout.fillWidth: true
                            text: realDirect.transferDirectory.length > 0
                                  ? realDirect.transferDirectory
                                  : I18n.t("chat.transfer.destination.default")
                            color: Theme.textSecondary
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontTiny
                            elide: Text.ElideMiddle
                        }

                        HarborButton {
                            variant: "quiet"
                            iconName: "file"
                            text: I18n.t("chat.transfer.destination.choose")
                            onClicked: folderDialog.open()
                        }
                    }
                }
            }
        }
    }

    FileDialog {
        id: attachDialog

        title: I18n.t("chat.transfer.attachDialog")
        fileMode: FileDialog.OpenFiles
        onAccepted: {
            for (var i = 0; i < selectedFiles.length; ++i)
                root.offerFile(selectedFiles[i])
        }
    }

    FolderDialog {
        id: folderDialog

        title: I18n.t("chat.transfer.destination.dialog")
        onAccepted: realDirect.setTransferDirectory(root.localPath(selectedFolder))
    }
}
