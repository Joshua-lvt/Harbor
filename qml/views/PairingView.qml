import Harbor 2.0
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Pairing overlay: your Harbor ID, the peer's Harbor ID, connecting state,
// success, error, and the incoming-request card. The provider owns every
// step and sequence: the real bridge while the supervised core is ready,
// MockController as the deterministic provider for tests and previews. This
// view only renders state and forwards commands. The shell owns the scrim,
// Escape handling, and launcher focus restoration.
//
// Pairing is Harbor-ID based: no codes are shown, typed, or exchanged here.
// A dedicated backdrop behind the dialog guarantees page content can never
// show through, and the footer (continue without pairing) lives outside the
// scroll area so it is never cut off.
HarborOverlayView {
    id: root

    overlayActive: AppState.pairingVisible
    signal paired(string partnerHarborId)

    // qmllint disable unqualified
    readonly property bool livePairing: typeof HarborCore !== "undefined" && HarborCore.coreReady
    // A production facade remains the provider while the core reconnects. The
    // mock is selected only when the application is running without a facade
    // (QML tests and explicit previews), never as a runtime fallback.
    readonly property bool hasCore: typeof HarborCore !== "undefined"
    // qmllint enable unqualified
    // Production pairing is a real Harbor-ID invitation; the fixture flow
    // keeps its demo vocabulary so previews never claim a real exchange
    // happened.
    readonly property string pk: root.hasCore ? "pairing." : "pairing.demo."

    HarborPairingBridge {
        id: realPairing

        // qmllint disable unqualified
        // Keep the facade attached whenever it exists (not only while the
        // core is live) so Harbor-ID copies still reach the real clipboard
        // during reconnects. Pairing actions inside the bridge still require
        // `live`.
        facade: root.hasCore ? HarborCore : null
        // qmllint enable unqualified
    }

    // Same contract; production keeps its real provider through reconnects.
    readonly property var provider: root.hasCore ? realPairing : MockController
    readonly property bool pairingUnavailable: root.hasCore && !root.livePairing

    readonly property string mode: root.pairingUnavailable ? "error" : root.provider.pairingMode
    readonly property string pairingErrorKey: root.pairingUnavailable
        ? "error.core.unavailable" : root.provider.pairingErrorKey
    readonly property var incoming: root.provider.incomingRequest
    readonly property string ownHarborId: root.provider.ownHarborId || ""
    readonly property string fieldState: root.provider.harborIdFieldState || "empty"

    initialFocusItem: root.mode === "home" ? harborIdInput
        : root.mode === "connecting" ? cancelButton
        : root.mode === "error" ? retryButton
        : root.mode === "incoming" ? acceptButton
        : enterButton
    lastFocusItem: root.mode === "home" ? connectButton
        : root.mode === "connecting" ? cancelButton
        : root.mode === "error" ? backHomeButton
        : root.mode === "incoming" ? declineButton
        : enterButton

    Connections {
        target: root.provider

        function onPairingCompleted(partnerHarborId) {
            root.paired(partnerHarborId)
        }
    }

    function _requestClose() {
        root.provider.closePairing()
        root.closed()
    }

    function _enterHarbor() {
        root.provider.closePairing()
        AppState.navigate("home")
        root.closed()
    }

    function _continueWithoutPairing() {
        root.provider.closePairing()
        AppState.navigate("home")
        root.closed()
    }

    function _connect() {
        root.provider.connectWithHarborId(harborIdInput.text)
    }

    function _errorMessage() {
        var key = String(root.pairingErrorKey || "")
        if (key.length > 0)
            return I18n.t(key, root.provider.pairingErrorParams)
        return I18n.t("pairing.error.notFound")
    }

    // Dedicated backdrop: dims everything behind the dialog and swallows
    // clicks, so page content can never bleed through regardless of the
    // shell scrim state.
    Rectangle {
        anchors.fill: parent
        color: "#CC04182B"
        Accessible.ignored: true

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onClicked: root._requestClose()
        }
    }

    Rectangle {
        id: dialog

        width: Math.min(parent.width - Theme.sp5 * 2, 600)
        height: Math.min(parent.height - Theme.sp5 * 2, 700)
        anchors.centerIn: parent
        radius: Theme.radiusLarge
        color: Theme.surfaceOverlay
        border.width: 1
        border.color: Theme.borderStrong
        clip: true

        Accessible.role: Accessible.Dialog
        Accessible.name: I18n.t(root.pk + "title")
        Accessible.description: I18n.t(root.pk + "subtitle")

        MouseArea {
            anchors.fill: parent
            acceptedButtons: Qt.AllButtons
            onClicked: mouse => mouse.accepted = true
        }

        Rectangle {
            anchors.left: parent.left
            anchors.right: parent.right
            anchors.top: parent.top
            height: 4
            radius: 2
            color: Theme.accent
            Accessible.ignored: true
        }

        ColumnLayout {
            anchors.fill: parent
            spacing: 0

            // Header
            RowLayout {
                Layout.fillWidth: true
                Layout.leftMargin: Theme.sp5
                Layout.rightMargin: Theme.sp3
                Layout.topMargin: Theme.sp3
                spacing: Theme.sp3

                Rectangle {
                    Layout.preferredWidth: 38
                    Layout.preferredHeight: 38
                    radius: Theme.radiusSmall
                    color: Theme.surfaceInteractive
                    Accessible.ignored: true

                    HarborIcon {
                        anchors.centerIn: parent
                        name: "online"
                        color: Theme.accent
                        implicitWidth: 20
                        implicitHeight: 20
                    }
                }

                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 1

                    Text {
                        Layout.fillWidth: true
                        text: root.mode === "success"
                            ? I18n.t(root.pk + "success.title") : I18n.t(root.pk + "title")
                        color: Theme.textPrimary
                        font.family: Theme.fontFamilyDisplay
                        font.pixelSize: Theme.fontHeading
                        font.weight: Font.Bold
                        wrapMode: Text.Wrap
                    }

                    Text {
                        Layout.fillWidth: true
                        text: root.mode === "success"
                            ? I18n.t(root.pk + "success.subtitle") : I18n.t(root.pk + "subtitle")
                        color: Theme.textSecondary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontSmall
                        wrapMode: Text.Wrap
                    }
                }

                HarborIconButton {
                    iconName: "close"
                    accessibleName: I18n.t("a11y.closeDialog")
                    toolTip: I18n.t("common.actions.close")
                    onClicked: root._requestClose()
                }
            }

            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 1
                Layout.topMargin: Theme.sp3
                color: Theme.divider
                Accessible.ignored: true
            }

            ScrollView {
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                contentWidth: availableWidth

                StackLayout {
                    id: pages

                    width: parent.width
                    // Tall content scrolls; short pages still fill the
                    // viewport so centered layouts keep working.
                    height: Math.max(implicitHeight, parent.height)
                    currentIndex: root.mode === "home" ? 0 : root.mode === "connecting" ? 1
                        : root.mode === "error" ? 2 : root.mode === "incoming" ? 3 : 4

                    // Home: own Harbor ID + peer Harbor ID ----------------------
                    Item {
                        ColumnLayout {
                            x: Theme.sp5
                            y: Theme.sp4
                            width: Math.max(0, parent.width - Theme.sp5 * 2)
                            spacing: Theme.sp4

                            // Own Harbor ID card
                            Rectangle {
                                Layout.fillWidth: true
                                radius: Theme.radius
                                color: Theme.surfaceSunken
                                border.width: 1
                                border.color: Theme.borderSubtle

                                ColumnLayout {
                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    anchors.top: parent.top
                                    anchors.bottom: parent.bottom
                                    anchors.margins: Theme.sp4
                                    spacing: Theme.sp2

                                    Text {
                                        Layout.fillWidth: true
                                        text: I18n.t(root.pk + "ownCard.title")
                                        color: Theme.textSecondary
                                        font.family: Theme.fontFamily
                                        font.pixelSize: Theme.fontSmall
                                        font.weight: Font.DemiBold
                                        font.capitalization: Font.AllUppercase
                                    }

                                    Text {
                                        Layout.fillWidth: true
                                        text: I18n.t(root.pk + "ownCard.share")
                                        color: Theme.textSecondary
                                        font.family: Theme.fontFamily
                                        font.pixelSize: Theme.fontSmall
                                        wrapMode: Text.Wrap
                                    }

                                    Text {
                                        Layout.fillWidth: true
                                        horizontalAlignment: Text.AlignHCenter
                                        text: root.ownHarborId.length > 0 ? root.ownHarborId : "—"
                                        color: Theme.textPrimary
                                        font.family: Theme.fontFamilyMonospace
                                        font.pixelSize: Theme.fontTitle
                                        font.weight: Font.Bold
                                        font.letterSpacing: 0.5
                                        wrapMode: Text.Wrap
                                        Accessible.role: Accessible.StaticText
                                        Accessible.name: I18n.t(root.pk + "ownCard.title")
                                        Accessible.description: root.ownHarborId
                                    }

                                    HarborButton {
                                        id: copyButton

                                        Layout.alignment: Qt.AlignHCenter
                                        variant: "secondary"
                                        enabled: root.ownHarborId.length > 0
                                        text: root.provider.mockCopyFeedbackVisible
                                              && root.provider.mockCopyTarget === "harborId"
                                            ? I18n.t(root.pk + "ownCard.copied")
                                            : I18n.t(root.pk + "ownCard.copy")
                                        Accessible.description: I18n.t(root.pk + "ownCard.share")
                                        onClicked: root.provider.copyHarborId()
                                    }

                                    Text {
                                        Layout.fillWidth: true
                                        horizontalAlignment: Text.AlignHCenter
                                        text: I18n.t(root.pk + "ownCard.note")
                                        color: Theme.textMuted
                                        font.family: Theme.fontFamily
                                        font.pixelSize: Theme.fontTiny
                                        wrapMode: Text.Wrap
                                    }
                                }
                            }

                            // Peer Harbor ID card
                            Rectangle {
                                Layout.fillWidth: true
                                radius: Theme.radius
                                color: Theme.surfaceSunken
                                border.width: 1
                                border.color: root.fieldState === "invalid"
                                              ? Theme.danger : Theme.borderSubtle

                                ColumnLayout {
                                    anchors.left: parent.left
                                    anchors.right: parent.right
                                    anchors.top: parent.top
                                    anchors.bottom: parent.bottom
                                    anchors.margins: Theme.sp4
                                    spacing: Theme.sp2

                                    Text {
                                        Layout.fillWidth: true
                                        text: I18n.t(root.pk + "input.title")
                                        color: Theme.textSecondary
                                        font.family: Theme.fontFamily
                                        font.pixelSize: Theme.fontSmall
                                        font.weight: Font.DemiBold
                                        font.capitalization: Font.AllUppercase
                                    }

                                    Text {
                                        Layout.fillWidth: true
                                        text: I18n.t(root.pk + "input.description")
                                        color: Theme.textSecondary
                                        font.family: Theme.fontFamily
                                        font.pixelSize: Theme.fontSmall
                                        wrapMode: Text.Wrap
                                    }

                                    HarborInput {
                                        id: harborIdInput

                                        Layout.fillWidth: true
                                        placeholderText: I18n.t(root.pk + "input.placeholder")
                                        text: root.provider.enteredHarborId
                                        inputMethodHints: Qt.ImhNoPredictiveText | Qt.ImhNoAutoUppercase
                                        helperText: root.fieldState === "invalid"
                                                    ? I18n.t(root.pk + "input.invalid")
                                                    : I18n.t(root.pk + "input.formatHint")
                                        errorText: root.fieldState === "invalid"
                                                   ? I18n.t(root.pk + "input.invalid") : ""
                                        Accessible.description: I18n.t(root.pk + "input.description")
                                        onAccepted: root._connect()
                                        onTextEdited: root.provider.enteredHarborId = harborIdInput.text
                                    }

                                    HarborButton {
                                        id: connectButton

                                        Layout.fillWidth: true
                                        enabled: root.fieldState === "valid" && !root.provider.connecting
                                        text: I18n.t(root.pk + "connect")
                                        onClicked: root._connect()
                                    }
                                }
                            }
                        }
                    }

                    // Connecting ------------------------------------------------
                    Item {
                        ColumnLayout {
                            width: Math.min(parent.width - Theme.sp5 * 2, 480)
                            anchors.centerIn: parent
                            spacing: Theme.sp4

                            Item {
                                Layout.alignment: Qt.AlignHCenter
                                Layout.preferredWidth: 84
                                Layout.preferredHeight: 84

                                HarborSpinner {
                                    anchors.centerIn: parent
                                    spinnerSize: 64
                                }
                            }

                            Text {
                                Layout.fillWidth: true
                                text: I18n.t(root.pk + "connecting.title")
                                color: Theme.textPrimary
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontTitle
                                font.weight: Font.Bold
                                horizontalAlignment: Text.AlignHCenter
                                wrapMode: Text.Wrap
                            }

                            Text {
                                Layout.fillWidth: true
                                text: I18n.t(root.pk + "connecting.description")
                                color: Theme.textSecondary
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontBody
                                horizontalAlignment: Text.AlignHCenter
                                wrapMode: Text.Wrap
                            }

                            Text {
                                Layout.fillWidth: true
                                horizontalAlignment: Text.AlignHCenter
                                text: root.provider.enteredHarborId
                                color: Theme.textPrimary
                                font.family: Theme.fontFamilyMonospace
                                font.pixelSize: Theme.fontBody
                                font.weight: Font.DemiBold
                                wrapMode: Text.Wrap
                            }

                            HarborButton {
                                id: cancelButton

                                Layout.alignment: Qt.AlignHCenter
                                variant: "secondary"
                                text: I18n.t(root.pk + "connecting.cancel")
                                onClicked: root.provider.cancelPairingRequest()
                            }
                        }
                    }

                    // Error ------------------------------------------------------
                    Item {
                        ColumnLayout {
                            width: Math.min(parent.width - Theme.sp5 * 2, 480)
                            anchors.centerIn: parent
                            spacing: Theme.sp4

                            Rectangle {
                                Layout.alignment: Qt.AlignHCenter
                                Layout.preferredWidth: 76
                                Layout.preferredHeight: 76
                                radius: Theme.radius
                                color: Theme.surfaceInteractive
                                border.width: 1
                                border.color: Theme.danger
                                Accessible.ignored: true

                                HarborIcon {
                                    anchors.centerIn: parent
                                    name: "error"
                                    color: Theme.danger
                                    implicitWidth: 32
                                    implicitHeight: 32
                                }
                            }

                            Text {
                                Layout.fillWidth: true
                                text: I18n.t("pairing.error.title")
                                color: Theme.textPrimary
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontTitle
                                font.weight: Font.Bold
                                horizontalAlignment: Text.AlignHCenter
                                wrapMode: Text.Wrap
                            }

                            Text {
                                Layout.fillWidth: true
                                text: root._errorMessage()
                                color: Theme.textSecondary
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontBody
                                horizontalAlignment: Text.AlignHCenter
                                wrapMode: Text.Wrap
                            }

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp3

                                HarborButton {
                                    id: backHomeButton

                                    Layout.fillWidth: true
                                    variant: "secondary"
                                    text: I18n.t("pairing.error.chooseAnother")
                                    onClicked: root.provider.setPairingMode("home")
                                }

                                HarborButton {
                                    id: retryButton

                                    Layout.fillWidth: true
                                    text: I18n.t("common.actions.retry")
                                    onClicked: root.provider.retryConnect()
                                }
                            }
                        }
                    }

                    // Incoming ----------------------------------------------------
                    Item {
                        ColumnLayout {
                            width: Math.min(parent.width - Theme.sp5 * 2, 500)
                            anchors.centerIn: parent
                            spacing: Theme.sp4

                            Rectangle {
                                Layout.alignment: Qt.AlignHCenter
                                Layout.preferredWidth: 68
                                Layout.preferredHeight: 68
                                radius: Theme.radius
                                color: Theme.surfaceInteractive
                                border.width: 2
                                border.color: Theme.accent
                                Accessible.ignored: true

                                HarborIcon {
                                    anchors.centerIn: parent
                                    name: "user"
                                    color: Theme.accent
                                    implicitWidth: 30
                                    implicitHeight: 30
                                }
                            }

                            Text {
                                Layout.fillWidth: true
                                text: I18n.t(root.pk + "incoming.title")
                                color: Theme.textPrimary
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontTitle
                                font.weight: Font.Bold
                                horizontalAlignment: Text.AlignHCenter
                                wrapMode: Text.Wrap
                            }

                            Text {
                                Layout.fillWidth: true
                                text: I18n.t(root.pk + "incoming.description",
                                             { name: root.incoming.harborId || root.incoming.name })
                                color: Theme.textSecondary
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontBody
                                horizontalAlignment: Text.AlignHCenter
                                wrapMode: Text.Wrap
                            }

                            Text {
                                Layout.fillWidth: true
                                horizontalAlignment: Text.AlignHCenter
                                text: root.incoming.harborId || root.incoming.name || "—"
                                color: Theme.textPrimary
                                font.family: Theme.fontFamilyMonospace
                                font.pixelSize: Theme.fontHeading
                                font.weight: Font.DemiBold
                                wrapMode: Text.Wrap
                            }

                            Text {
                                Layout.fillWidth: true
                                horizontalAlignment: Text.AlignHCenter
                                text: I18n.t(root.pk + "incoming.confirm")
                                color: Theme.textMuted
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontSmall
                                wrapMode: Text.Wrap
                            }

                            RowLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp3

                                HarborButton {
                                    id: declineButton

                                    Layout.fillWidth: true
                                    variant: "secondary"
                                    text: I18n.t("pairing.incoming.decline")
                                    onClicked: root.provider.declineIncomingRequest()
                                }

                                HarborButton {
                                    id: acceptButton

                                    Layout.fillWidth: true
                                    text: I18n.t(root.pk + "incoming.accept")
                                    onClicked: root.provider.acceptIncomingRequest()
                                }
                            }
                        }
                    }

                    // Success ------------------------------------------------------
                    Item {
                        ColumnLayout {
                            width: Math.min(parent.width - Theme.sp5 * 2, 500)
                            anchors.centerIn: parent
                            spacing: Theme.sp4

                            RowLayout {
                                Layout.alignment: Qt.AlignHCenter
                                spacing: Theme.sp2

                                HarborAvatar {
                                    initials: AppState.selfInitials
                                    source: AppState.selfProfile.avatar
                                    avatarType: AppState.selfProfile.avatarType
                                    status: "online"
                                    avatarSize: 70
                                }

                                HarborAvatar {
                                    initials: AppState.initialsFor(AppState.partnerName)
                                    source: AppState.partnerProfile.avatar
                                    avatarType: AppState.partnerProfile.avatarType
                                    status: "online"
                                    avatarSize: 70
                                }

                                Rectangle {
                                    implicitWidth: 28
                                    implicitHeight: 28
                                    radius: 14
                                    color: Theme.success
                                    border.width: 3
                                    border.color: Theme.surfaceOverlay
                                    Accessible.ignored: true

                                    HarborIcon {
                                        anchors.centerIn: parent
                                        name: "check"
                                        color: "white"
                                        implicitWidth: 14
                                        implicitHeight: 14
                                    }
                                }
                            }

                            Text {
                                Layout.fillWidth: true
                                text: I18n.t(root.pk + "success.paired",
                                             { name: AppState.partnerName })
                                color: Theme.textPrimary
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontTitle
                                font.weight: Font.Bold
                                horizontalAlignment: Text.AlignHCenter
                                wrapMode: Text.Wrap
                            }

                            Text {
                                Layout.fillWidth: true
                                text: I18n.t(root.pk + "success.description")
                                color: Theme.textSecondary
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontBody
                                horizontalAlignment: Text.AlignHCenter
                                wrapMode: Text.Wrap
                            }

                            HarborButton {
                                id: enterButton

                                Layout.fillWidth: true
                                text: I18n.t(root.pk + "success.enter")
                                onClicked: root._enterHarbor()
                            }
                        }
                    }
                }
            }

            // Footer: always visible, never scrolled away ----------------------
            Rectangle {
                Layout.fillWidth: true
                Layout.preferredHeight: 1
                color: Theme.divider
                Accessible.ignored: true
            }

            ColumnLayout {
                Layout.fillWidth: true
                Layout.leftMargin: Theme.sp5
                Layout.rightMargin: Theme.sp5
                Layout.topMargin: Theme.sp3
                Layout.bottomMargin: Theme.sp3
                spacing: Theme.sp1

                Text {
                    Layout.fillWidth: true
                    horizontalAlignment: Text.AlignHCenter
                    text: I18n.t(root.pk + "continueWithoutPairing.description")
                    color: Theme.textMuted
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontTiny
                    wrapMode: Text.Wrap
                }

                HarborButton {
                    id: continueButton

                    Layout.alignment: Qt.AlignHCenter
                    variant: "secondary"
                    text: I18n.t(root.pk + "continueWithoutPairing")
                    onClicked: root._continueWithoutPairing()
                }
            }
        }
    }
}
