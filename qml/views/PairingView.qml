import Harbor 2.0
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Pairing overlay: choice → QR/code → waiting → success, plus the incoming
// request card. The provider owns every step, timer, and sequence: the real
// bridge while the supervised core is ready, MockController as the
// deterministic provider for tests and previews. This view only renders state
// and forwards commands. The shell owns the scrim, Escape handling, and
// launcher focus restoration.
HarborOverlayView {
    id: root

    overlayActive: AppState.pairingVisible
    signal paired(string partnerName)

    // qmllint disable unqualified
    readonly property bool livePairing: typeof HarborCore !== "undefined" && HarborCore.coreReady
    // A production facade remains the provider while the core reconnects. The
    // mock is selected only when the application is running without a facade
    // (QML tests and explicit previews), never as a runtime fallback.
    readonly property bool hasCore: typeof HarborCore !== "undefined"
    // qmllint enable unqualified
    // Production pairing is a real code exchange; the fixture flow keeps its
    // demo vocabulary so previews never claim a real exchange happened.
    readonly property string pk: root.hasCore ? "pairing." : "pairing.demo."

    HarborPairingBridge {
        id: realPairing

        // qmllint disable unqualified
        // Keep the facade attached whenever it exists (not only while the
        // core is live) so Harbor-ID / pairing-code copies still reach the
        // real clipboard during reconnects. Pairing actions inside the
        // bridge still require `live`.
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

    initialFocusItem: mode === "choice" ? qrOptionButton
        : mode === "qr" ? copyButton
        : mode === "request" ? codeInput
        : mode === "waiting" ? cancelRequestButton
        : mode === "error" ? tryAgainButton
        : mode === "incoming" ? acceptButton
        : enterButton
    lastFocusItem: mode === "choice" ? codeOptionButton
        : mode === "qr" ? (root.hasCore ? copyButton : simulateScanButton)
        : mode === "request" ? sendRequestButton
        : mode === "waiting" ? cancelRequestButton
        : mode === "error" ? chooseAnotherButton
        : mode === "incoming" ? declineButton
        : enterButton

    Connections {
        target: root.provider

        function onPairingCompleted(partnerName) {
            root.paired(partnerName)
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

    function _qrSecondsText() {
        var seconds = Math.max(0, root.provider.pairingCodeSeconds)
        var minutes = Math.floor(seconds / 60)
        var rest = seconds % 60
        return minutes + ":" + (rest < 10 ? "0" : "") + rest
    }

    // Clicking the dimmed area outside the dialog closes the overlay; the
    // dimming itself comes from the shell's single scrim.
    MouseArea {
        anchors.fill: parent
        onClicked: root._requestClose()
    }

    Rectangle {
        id: dialog

        width: Math.min(parent.width - Theme.sp5 * 2, 760)
        height: Math.min(parent.height - Theme.sp5 * 2, 650)
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

            StackLayout {
                id: pages

                Layout.fillWidth: true
                Layout.fillHeight: true
                currentIndex: root.mode === "choice" ? 0 : root.mode === "qr" ? 1
                    : root.mode === "request" ? 2 : root.mode === "waiting" ? 3
                    : root.mode === "error" ? 4 : root.mode === "incoming" ? 5 : 6

                // Choice -----------------------------------------------------------------
                Item {
                    ScrollView {
                        anchors.fill: parent
                        clip: true
                        contentWidth: availableWidth

                        ColumnLayout {
                            x: Theme.sp5
                            y: Theme.sp5
                            width: Math.max(0, parent.width - Theme.sp5 * 2)
                            spacing: Theme.sp4

                            ColumnLayout {
                                Layout.fillWidth: true
                                spacing: Theme.sp1

                                Text {
                                    Layout.fillWidth: true
                                    text: I18n.t(root.pk + "choice.title")
                                    color: Theme.textPrimary
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontTitle
                                    font.weight: Font.Bold
                                    horizontalAlignment: Text.AlignHCenter
                                    wrapMode: Text.Wrap
                                }

                                Text {
                                    Layout.fillWidth: true
                                    text: I18n.t(root.pk + "choice.description")
                                    color: Theme.textSecondary
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontBody
                                    horizontalAlignment: Text.AlignHCenter
                                    wrapMode: Text.Wrap
                                }
                            }

                            RowLayout {
                                Layout.fillWidth: true
                                Layout.topMargin: Theme.sp3
                                spacing: Theme.sp4

                                Button {
                                    id: qrOptionButton

                                    Layout.fillWidth: true
                                    Layout.preferredHeight: 208
                                    focusPolicy: Qt.StrongFocus
                                    hoverEnabled: true
                                    Accessible.name: I18n.t(root.pk + "choice.qr.title")
                                    Accessible.description: I18n.t(root.pk + "choice.qr.description")
                                    onClicked: root.provider.setPairingMode("qr")

                                    background: Rectangle {
                                        radius: Theme.radius
                                        color: qrOptionButton.hovered || qrOptionButton.visualFocus
                                            ? Theme.surfaceStrong : Theme.surface
                                        border.width: qrOptionButton.visualFocus
                                            ? Theme.focusWidth : 1
                                        border.color: qrOptionButton.visualFocus
                                            ? Theme.focusRing : Theme.borderSubtle

                                        Behavior on color {
                                            ColorAnimation { duration: Theme.duration(Theme.motionFast) }
                                        }
                                    }

                                    contentItem: ColumnLayout {
                                        spacing: Theme.sp2

                                        Rectangle {
                                            Layout.alignment: Qt.AlignHCenter
                                            Layout.preferredWidth: 54
                                            Layout.preferredHeight: 54
                                            radius: Theme.radiusSmall
                                            color: Theme.surfaceInteractive
                                            Accessible.ignored: true

                                            HarborIcon {
                                                anchors.centerIn: parent
                                                name: "app"
                                                color: Theme.accent
                                                implicitWidth: 24
                                                implicitHeight: 24
                                            }
                                        }

                                        Text {
                                            Layout.fillWidth: true
                                            text: I18n.t(root.pk + "choice.qr.title")
                                            color: Theme.textPrimary
                                            font.family: Theme.fontFamily
                                            font.pixelSize: Theme.fontHeading
                                            font.weight: Font.DemiBold
                                            horizontalAlignment: Text.AlignHCenter
                                            wrapMode: Text.Wrap
                                        }

                                        Text {
                                            Layout.fillWidth: true
                                            Layout.fillHeight: true
                                            text: I18n.t(root.pk + "choice.qr.description")
                                            color: Theme.textSecondary
                                            font.family: Theme.fontFamily
                                            font.pixelSize: Theme.fontSmall
                                            horizontalAlignment: Text.AlignHCenter
                                            wrapMode: Text.Wrap
                                        }

                                        Text {
                                            Layout.fillWidth: true
                                            text: I18n.t(root.pk + "choice.qr.action") + "  →"
                                            color: Theme.accent
                                            font.family: Theme.fontFamily
                                            font.pixelSize: Theme.fontSmall
                                            font.weight: Font.DemiBold
                                            horizontalAlignment: Text.AlignHCenter
                                        }
                                    }
                                }

                                Button {
                                    id: codeOptionButton

                                    Layout.fillWidth: true
                                    Layout.preferredHeight: 208
                                    focusPolicy: Qt.StrongFocus
                                    hoverEnabled: true
                                    Accessible.name: I18n.t(root.pk + "choice.code.title")
                                    Accessible.description: I18n.t(root.pk + "choice.code.description")
                                    onClicked: root.provider.setPairingMode("request")

                                    background: Rectangle {
                                        radius: Theme.radius
                                        color: codeOptionButton.hovered || codeOptionButton.visualFocus
                                            ? Theme.surfaceStrong : Theme.surface
                                        border.width: codeOptionButton.visualFocus
                                            ? Theme.focusWidth : 1
                                        border.color: codeOptionButton.visualFocus
                                            ? Theme.focusRing : Theme.borderSubtle

                                        Behavior on color {
                                            ColorAnimation { duration: Theme.duration(Theme.motionFast) }
                                        }
                                    }

                                    contentItem: ColumnLayout {
                                        spacing: Theme.sp2

                                        Rectangle {
                                            Layout.alignment: Qt.AlignHCenter
                                            Layout.preferredWidth: 54
                                            Layout.preferredHeight: 54
                                            radius: Theme.radiusSmall
                                            color: Theme.surfaceInteractive
                                            Accessible.ignored: true

                                            HarborIcon {
                                                anchors.centerIn: parent
                                                name: "lock"
                                                color: Theme.teal
                                                implicitWidth: 24
                                                implicitHeight: 24
                                            }
                                        }

                                        Text {
                                            Layout.fillWidth: true
                                            text: I18n.t(root.pk + "choice.code.title")
                                            color: Theme.textPrimary
                                            font.family: Theme.fontFamily
                                            font.pixelSize: Theme.fontHeading
                                            font.weight: Font.DemiBold
                                            horizontalAlignment: Text.AlignHCenter
                                            wrapMode: Text.Wrap
                                        }

                                        Text {
                                            Layout.fillWidth: true
                                            Layout.fillHeight: true
                                            text: I18n.t(root.pk + "choice.code.description")
                                            color: Theme.textSecondary
                                            font.family: Theme.fontFamily
                                            font.pixelSize: Theme.fontSmall
                                            horizontalAlignment: Text.AlignHCenter
                                            wrapMode: Text.Wrap
                                        }

                                        Text {
                                            Layout.fillWidth: true
                                            text: I18n.t(root.pk + "choice.code.action") + "  →"
                                            color: Theme.teal
                                            font.family: Theme.fontFamily
                                            font.pixelSize: Theme.fontSmall
                                            font.weight: Font.DemiBold
                                            horizontalAlignment: Text.AlignHCenter
                                        }
                                    }
                                }
                            }

                            Text {
                                Layout.alignment: Qt.AlignHCenter
                                Layout.topMargin: Theme.sp2
                                text: I18n.t(root.pk + "security")
                                color: Theme.textMuted
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontTiny
                                wrapMode: Text.Wrap
                            }
                        }
                    }
                }

                // Mock QR ----------------------------------------------------------------
                Item {
                    ScrollView {
                        anchors.fill: parent
                        clip: true
                        contentWidth: availableWidth

                        ColumnLayout {
                            x: Theme.sp5
                            y: Theme.sp4
                            width: Math.max(0, parent.width - Theme.sp5 * 2)
                            spacing: Theme.sp3

                            Text {
                                Layout.fillWidth: true
                                text: I18n.t(root.pk + "qr.title")
                                color: Theme.textPrimary
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontTitle
                                font.weight: Font.Bold
                                horizontalAlignment: Text.AlignHCenter
                                wrapMode: Text.Wrap
                            }

                            Text {
                                Layout.fillWidth: true
                                text: I18n.t(root.pk + "qr.description")
                                color: Theme.textSecondary
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontBody
                                horizontalAlignment: Text.AlignHCenter
                                wrapMode: Text.Wrap
                            }

                            HarborMockQr {
                                Layout.alignment: Qt.AlignHCenter
                                Layout.topMargin: Theme.sp2
                                Layout.preferredWidth: 226
                                Layout.preferredHeight: 226
                                seed: root.provider.pairingCode
                                accessibleDescription: I18n.t(root.pk + "qr.description")
                            }

                            RowLayout {
                                Layout.alignment: Qt.AlignHCenter
                                spacing: Theme.sp3

                                Rectangle {
                                    implicitWidth: codeLabel.implicitWidth + Theme.sp4 * 2
                                    implicitHeight: Theme.hitTarget
                                    radius: Theme.radiusSmall
                                    color: Theme.surfaceSunken
                                    border.width: 1
                                    border.color: Theme.borderSubtle
                                    Accessible.ignored: true

                                    Text {
                                        id: codeLabel

                                        anchors.centerIn: parent
                                        text: root.provider.pairingCode
                                        color: Theme.textPrimary
                                        font.family: Theme.fontFamilyMonospace
                                        font.pixelSize: Theme.fontBody
                                        font.letterSpacing: 1.5
                                        font.weight: Font.Bold
                                        font.features: { "tnum": 1 }
                                    }
                                }

                                HarborButton {
                                    id: copyButton

                                    variant: "secondary"
                                    text: root.provider.mockCopyFeedbackVisible
                                          && root.provider.mockCopyTarget === "pairingCode"
                                        ? I18n.t(root.pk + "qr.copied")
                                        : I18n.t(root.pk + "qr.copy")
                                    onClicked: root.provider.mockCopy(root.provider.pairingCode,
                                                                       "pairingCode")
                                }
                            }

                            Text {
                                Layout.fillWidth: true
                                text: I18n.t(root.pk + "qr.refreshesIn",
                                            { duration: root._qrSecondsText() })
                                color: root.provider.pairingCodeSeconds < 10
                                    ? Theme.warning : Theme.textSecondary
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontSmall
                                horizontalAlignment: Text.AlignHCenter
                            }

                            RowLayout {
                                Layout.alignment: Qt.AlignHCenter
                                spacing: Theme.sp3

                                HarborButton {
                                    variant: "secondary"
                                    text: I18n.t("common.actions.back")
                                    onClicked: root.provider.setPairingMode("choice")
                                }

                                HarborButton {
                                    id: simulateScanButton

                                    // Simulated success is a test-provider
                                    // affordance; the real flow never fakes it.
                                    visible: !root.hasCore
                                    text: !root.hasCore
                                          ? I18n.t(root.pk + "qr.simulate") : ""
                                    onClicked: root.provider.completePairing("Taylor")
                                }
                            }
                        }
                    }
                }

                // Code entry -------------------------------------------------------------
                Item {
                    ColumnLayout {
                        width: Math.min(parent.width - Theme.sp5 * 2, 510)
                        anchors.centerIn: parent
                        spacing: Theme.sp4

                        Rectangle {
                            Layout.alignment: Qt.AlignHCenter
                            Layout.preferredWidth: 68
                            Layout.preferredHeight: 68
                            radius: Theme.radius
                            color: Theme.surfaceInteractive
                            Accessible.ignored: true

                            HarborIcon {
                                anchors.centerIn: parent
                                name: "lock"
                                color: Theme.accent
                                implicitWidth: 30
                                implicitHeight: 30
                            }
                        }

                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp1

                            Text {
                                Layout.fillWidth: true
                                text: I18n.t(root.pk + "request.title")
                                color: Theme.textPrimary
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontTitle
                                font.weight: Font.Bold
                                horizontalAlignment: Text.AlignHCenter
                                wrapMode: Text.Wrap
                            }

                            Text {
                                Layout.fillWidth: true
                                text: I18n.t(root.pk + "request.description")
                                color: Theme.textSecondary
                                font.family: Theme.fontFamily
                                font.pixelSize: Theme.fontBody
                                horizontalAlignment: Text.AlignHCenter
                                wrapMode: Text.Wrap
                            }
                        }

                        HarborInput {
                            id: codeInput

                            Layout.fillWidth: true
                            placeholderText: I18n.t(root.pk + "request.placeholder")
                            onAccepted: root.provider.submitPairingCode(text)
                        }

                        Text {
                            Layout.fillWidth: true
                            visible: !root.hasCore
                            text: I18n.t(root.pk + "request.previewTip")
                            color: Theme.textMuted
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontTiny
                            horizontalAlignment: Text.AlignHCenter
                            wrapMode: Text.Wrap
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp3

                            HarborButton {
                                variant: "secondary"
                                text: I18n.t("common.actions.back")
                                onClicked: root.provider.setPairingMode("choice")
                            }

                            HarborButton {
                                id: sendRequestButton

                                Layout.fillWidth: true
                                text: I18n.t(root.pk + "request.send")
                                onClicked: root.provider.submitPairingCode(codeInput.text)
                            }
                        }
                    }
                }

                // Waiting -----------------------------------------------------------------
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
                            text: I18n.t(root.pk + "waiting.title")
                            color: Theme.textPrimary
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontTitle
                            font.weight: Font.Bold
                            horizontalAlignment: Text.AlignHCenter
                            wrapMode: Text.Wrap
                        }

                        Text {
                            Layout.fillWidth: true
                            text: I18n.t(root.pk + "waiting.description")
                            color: Theme.textSecondary
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontBody
                            horizontalAlignment: Text.AlignHCenter
                            wrapMode: Text.Wrap
                        }

                        Rectangle {
                            Layout.fillWidth: true
                            implicitHeight: Theme.hitTarget
                            radius: Theme.radiusSmall
                            color: Theme.surfaceSunken
                            border.width: 1
                            border.color: Theme.borderSubtle

                            RowLayout {
                                anchors.fill: parent
                                anchors.leftMargin: Theme.sp3
                                anchors.rightMargin: Theme.sp3

                                Text {
                                    text: I18n.t(root.pk + "waiting.invitation")
                                    color: Theme.textSecondary
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontSmall
                                }

                                Item { Layout.fillWidth: true }

                                Text {
                                    text: root.provider.enteredPairingCode
                                    color: Theme.textPrimary
                                    font.family: Theme.fontFamilyMonospace
                                    font.pixelSize: Theme.fontSmall
                                    font.weight: Font.Bold
                                    font.features: { "tnum": 1 }
                                }
                            }
                        }

                        HarborButton {
                            id: cancelRequestButton

                            Layout.alignment: Qt.AlignHCenter
                            variant: "secondary"
                            text: I18n.t(root.pk + "waiting.cancel")
                            onClicked: root.provider.cancelPairingRequest()
                        }
                    }
                }

                // Error -------------------------------------------------------------------
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
                            text: root.pairingErrorKey.length > 0
                                ? I18n.t(root.pairingErrorKey,
                                         root.provider.pairingErrorParams)
                                : I18n.t("pairing.error.notFound")
                            color: Theme.textSecondary
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontBody
                            horizontalAlignment: Text.AlignHCenter
                            wrapMode: Text.Wrap
                        }

                        Rectangle {
                            Layout.fillWidth: true
                            implicitHeight: 58
                            radius: Theme.radiusSmall
                            color: Theme.surfaceSunken
                            border.width: 1
                            border.color: Theme.borderSubtle

                            RowLayout {
                                anchors.fill: parent
                                anchors.margins: Theme.sp3

                                Text {
                                    text: I18n.t("pairing.error.checkFor")
                                    color: Theme.textSecondary
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontSmall
                                }

                                Text {
                                    Layout.fillWidth: true
                                    text: I18n.t("pairing.error.reasons")
                                    color: Theme.textPrimary
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontSmall
                                    wrapMode: Text.Wrap
                                }
                            }
                        }

                        RowLayout {
                            Layout.fillWidth: true
                            spacing: Theme.sp3

                            HarborButton {
                                id: chooseAnotherButton

                                variant: "secondary"
                                text: I18n.t("pairing.error.chooseAnother")
                                onClicked: root.provider.setPairingMode("choice")
                            }

                            HarborButton {
                                id: tryAgainButton

                                Layout.fillWidth: true
                                text: I18n.t("common.actions.retry")
                                onClicked: root.provider.setPairingMode("request")
                            }
                        }
                    }
                }

                // Incoming ----------------------------------------------------------------
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
                            text: root.incoming.name.length > 0
                                ? I18n.t(root.pk + "incoming.description",
                                         { name: root.incoming.name })
                                : I18n.t(root.pk + "incoming.unknown")
                            color: Theme.textSecondary
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontBody
                            horizontalAlignment: Text.AlignHCenter
                            wrapMode: Text.Wrap
                        }

                        RowLayout {
                            Layout.alignment: Qt.AlignHCenter
                            spacing: Theme.sp3

                            HarborAvatar {
                                initials: root.incoming.initials || "?"
                                status: "online"
                                avatarSize: 64
                            }

                            ColumnLayout {
                                spacing: Theme.sp1

                                Text {
                                    text: root.incoming.name.length > 0
                                        ? root.incoming.name : "—"
                                    color: Theme.textPrimary
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontHeading
                                    font.weight: Font.DemiBold
                                }

                                Text {
                                    text: I18n.t(root.pk + "incoming.verify",
                                                 { code: root.incoming.code })
                                    color: Theme.textSecondary
                                    font.family: Theme.fontFamily
                                    font.pixelSize: Theme.fontSmall
                                    wrapMode: Text.Wrap
                                }
                            }
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

                // Success -----------------------------------------------------------------
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

                        Rectangle {
                            Layout.fillWidth: true
                            implicitHeight: 64
                            radius: Theme.radiusSmall
                            color: Theme.surfaceSunken
                            border.width: 1
                            border.color: Theme.success

                            RowLayout {
                                anchors.fill: parent
                                anchors.margins: Theme.sp4

                                Rectangle {
                                    implicitWidth: 10
                                    implicitHeight: 10
                                    radius: 5
                                    color: Theme.success
                                    Accessible.ignored: true
                                }

                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 1

                                    Text {
                                        Layout.fillWidth: true
                                        text: I18n.t(root.pk + "success.secure")
                                        color: Theme.textPrimary
                                        font.family: Theme.fontFamily
                                        font.pixelSize: Theme.fontSmall
                                        font.weight: Font.DemiBold
                                        wrapMode: Text.Wrap
                                    }

                                    Text {
                                        Layout.fillWidth: true
                                        text: I18n.t(root.pk + "success.verified")
                                        color: Theme.textSecondary
                                        font.family: Theme.fontFamily
                                        font.pixelSize: Theme.fontTiny
                                        wrapMode: Text.Wrap
                                    }
                                }
                            }
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
    }
}
