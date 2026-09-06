pragma ComponentBehavior: Bound

import Harbor 2.0
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// First-run pairing: a single, calm screen. No wizard, no server details.
// The user sees their own Harbor ID, types the partner's six-digit pairing
// code (or shares their own), and connects. The provider owns the handshake:
// the real HarborPairingBridge while the supervised core is ready,
// MockController as the deterministic provider for tests and previews.
HarborOverlayView {
    id: root

    overlayActive: AppState.onboardingVisible
    signal completed()
    signal dismissed()

    // qmllint disable unqualified
    readonly property bool hasCore: typeof HarborCore !== "undefined"
    readonly property bool liveCore: hasCore && HarborCore.coreReady
    // qmllint enable unqualified

    HarborPairingBridge {
        id: realPairing

        // qmllint disable unqualified
        // Keep the facade attached whenever it exists (not only while the
        // core is live) so the Harbor-ID copy still reaches the real
        // clipboard during reconnects. Pairing actions still require `live`.
        facade: root.hasCore ? HarborCore : null
        // qmllint enable unqualified
    }

    // Same contract; production keeps its real provider through reconnects.
    readonly property var provider: root.hasCore ? realPairing : MockController
    readonly property bool pairingUnavailable: root.hasCore && !root.liveCore

    property string peerCode: ""
    property string noticeMessage: ""
    property string noticeTone: "neutral" // neutral | danger | success

    readonly property bool compact: width < 760

    // Visual pairing state for this single screen, derived from the
    // provider's mode plus transport failures. Human copy only — no
    // technical state names reach the user.
    readonly property string uiState: {
        if (root.pairingUnavailable)
            return "SERVER_UNAVAILABLE"
        var mode = root.provider.pairingMode
        if (mode === "success")
            return "SUCCESS"
        if (mode === "incoming")
            return "INCOMING"
        if (mode === "waiting")
            return "WAITING_APPROVAL"
        if (mode === "qr")
            return "WAITING_APPROVAL"
        if (mode === "request" || mode === "choice")
            return root.peerCode.length > 0 ? "ENTERING_CODE" : "INITIAL"
        if (mode === "error") {
            var key = String(root.provider.pairingErrorKey || "")
            if (key.indexOf("declined") >= 0)
                return "DECLINED"
            if (key.indexOf("tooShort") >= 0 || key.indexOf("notFound") >= 0
                    || key.indexOf("expired") >= 0)
                return "INVALID_CODE"
            if (key.indexOf("server") >= 0 || key.indexOf("unavailable") >= 0)
                return "SERVER_UNAVAILABLE"
            return "ERROR"
        }
        return "INITIAL"
    }

    readonly property string selfHarborId: AppState.harborId
    readonly property string selfPairingCode: root.provider.pairingCode || ""
    readonly property bool busy: root.uiState === "WAITING_APPROVAL"
    readonly property var incoming: root.provider.incomingRequest

    initialFocusItem: peerCodeInput
    lastFocusItem: continueButton

    Accessible.role: Accessible.Dialog
    Accessible.name: I18n.t("onboarding.single.title")

    function showNotice(message, tone) {
        noticeMessage = message || ""
        noticeTone = tone || "neutral"
    }

    function copySelfId() {
        if (root.selfHarborId.length === 0)
            return
        root.provider.mockCopy(root.selfHarborId, "harborId")
        showNotice(I18n.t("onboarding.single.copied"), "success")
    }

    function copyPairingCode() {
        if (root.selfPairingCode.length === 0)
            return
        root.provider.mockCopy(root.selfPairingCode, "pairingCode")
        showNotice(I18n.t("onboarding.single.codeCopied"), "success")
    }

    function ensureHostCode() {
        // Host flow: publish our own six-digit code for the partner to type.
        // Only the real/test provider knows how; failures surface via mode.
        if (root.provider.pairingMode === "choice")
            root.provider.setPairingMode("qr")
    }

    function connectWithCode() {
        showNotice("", "neutral")
        var code = String(root.peerCode || "").replace(/[^0-9]/g, "")
        if (code.length !== 6) {
            showNotice(I18n.t("pairing.error.tooShort"), "danger")
            return false
        }
        if (root.provider.pairingMode === "qr" || root.provider.pairingMode === "choice")
            root.provider.setPairingMode("request")
        return root.provider.submitPairingCode(code)
    }

    function cancelWaiting() {
        root.provider.cancelPairingRequest()
        showNotice("", "neutral")
    }

    function continueWithoutPairing() {
        root.provider.closePairing()
        AppState.onboardingVisible = false
        if (!AppState.paired)
            AppState.continueWithoutPairing()
        AppState.navigate("home")
        dismissed()
    }

    function finish() {
        AppState.onboardingVisible = false
        AppState.navigate("home")
        completed()
    }

    function skip() {
        continueWithoutPairing()
    }

    Connections {
        target: root.provider

        function onPairingCompleted(partnerName) {
            root.showNotice(I18n.t("pairing.success.paired", { name: partnerName }), "success")
            // Let the success moment land before entering the space.
            successTimer.restart()
        }
    }

    Timer {
        id: successTimer
        interval: 900
        repeat: false
        onTriggered: root.finish()
    }

    onOverlayActiveChanged: {
        if (overlayActive) {
            peerCode = ""
            showNotice("", "neutral")
            if (root.hasCore && root.liveCore)
                ensureHostCode()
        } else {
            successTimer.stop()
        }
    }

    Rectangle {
        id: dialog
        width: Math.min(parent.width - (root.compact ? Theme.sp4 * 2 : Theme.sp6 * 2), 560)
        height: Math.min(parent.height - (root.compact ? Theme.sp4 * 2 : Theme.sp5 * 2), 640)
        anchors.centerIn: parent
        radius: Theme.radiusLarge
        color: Theme.surfaceOverlay
        border.width: 1
        border.color: Theme.borderStrong
        clip: true

        MouseArea { anchors.fill: parent; onClicked: mouse => mouse.accepted = true }

        ScrollView {
            anchors.fill: parent
            clip: true
            contentWidth: availableWidth

            ColumnLayout {
                x: Theme.sp5
                y: Theme.sp5
                width: Math.max(0, parent.width - Theme.sp5 * 2)
                spacing: Theme.sp4

                HarborLogo {
                    Layout.alignment: Qt.AlignHCenter
                    showWordmark: true
                    compact: false
                }

                Text {
                    Layout.fillWidth: true
                    text: I18n.t("onboarding.single.title")
                    color: Theme.textPrimary
                    font.family: Theme.fontFamilyDisplay
                    font.pixelSize: Theme.fontDisplay
                    font.weight: Font.Bold
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                }

                Text {
                    Layout.fillWidth: true
                    text: I18n.t("onboarding.single.subtitle")
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontBody
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                }

                // Your shareable pairing code (host flow) --------------------
                // This six-digit code is the hero of the screen: it is the
                // only thing the other person ever types. The Harbor ID
                // below is device identity, never a pairing code.
                HarborSectionCard {
                    Layout.fillWidth: true
                    title: I18n.t("onboarding.single.shareCode")

                    Text {
                        Layout.fillWidth: true
                        horizontalAlignment: Text.AlignHCenter
                        text: root.selfPairingCode.length > 0
                               ? root.selfPairingCode
                               : I18n.t("common.notAvailable")
                        color: Theme.textPrimary
                        font.family: Theme.fontFamilyMonospace
                        font.pixelSize: Theme.fontDisplay
                        font.weight: Font.Bold
                        font.letterSpacing: 4
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        Item { Layout.fillWidth: true }

                        HarborButton {
                            variant: "secondary"
                            text: I18n.t("pairing.qr.copy")
                            enabled: root.selfPairingCode.length > 0
                            onClicked: root.copyPairingCode()
                        }

                        Item { Layout.fillWidth: true }
                    }

                    Text {
                        Layout.fillWidth: true
                        text: I18n.t("onboarding.single.shareHint")
                        color: Theme.textSecondary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontSmall
                        horizontalAlignment: Text.AlignHCenter
                        wrapMode: Text.Wrap
                    }
                }

                // Partner code ---------------------------------------------------
                HarborSectionCard {
                    Layout.fillWidth: true
                    title: I18n.t("onboarding.single.partnerCode")

                    HarborInput {
                        id: peerCodeInput
                        objectName: "onboardingPeerCodeInput"
                        Layout.fillWidth: true
                        placeholderText: I18n.t("pairing.request.placeholder")
                        text: root.peerCode
                        inputMethodHints: Qt.ImhDigitsOnly
                        onTextEdited: value => {
                            root.peerCode = String(value || "").replace(/[^0-9]/g, "").slice(0, 6)
                            root.showNotice("", "neutral")
                        }
                        onAccepted: root.connectWithCode()
                    }

                    HarborButton {
                        id: connectButton
                        objectName: "onboardingConnectButton"
                        Layout.fillWidth: true
                        variant: "primary"
                        text: root.uiState === "WAITING_APPROVAL"
                               ? I18n.t("onboarding.single.waiting")
                               : I18n.t("pairing.request.send")
                        busy: root.busy
                        enabled: !root.busy && !root.pairingUnavailable
                        onClicked: root.connectWithCode()
                    }

                    HarborButton {
                        visible: root.uiState === "WAITING_APPROVAL"
                        Layout.fillWidth: true
                        variant: "secondary"
                        text: I18n.t("pairing.waiting.cancel")
                        onClicked: root.cancelWaiting()
                    }

                    Text {
                        Layout.fillWidth: true
                        text: I18n.t("onboarding.single.codeHint")
                        color: Theme.textMuted
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontSmall
                        horizontalAlignment: Text.AlignHCenter
                        wrapMode: Text.Wrap
                    }

                    Text {
                        Layout.fillWidth: true
                        visible: root.noticeMessage.length > 0
                        text: root.noticeMessage
                        color: root.noticeTone === "danger" ? Theme.danger
                               : root.noticeTone === "success" ? Theme.success
                               : Theme.textSecondary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontSmall
                        horizontalAlignment: Text.AlignHCenter
                        wrapMode: Text.Wrap
                    }

                    Text {
                        Layout.fillWidth: true
                        visible: root.uiState === "WAITING_APPROVAL"
                        text: I18n.t("onboarding.single.waitingDetail")
                        color: Theme.textSecondary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontSmall
                        horizontalAlignment: Text.AlignHCenter
                        wrapMode: Text.Wrap
                    }

                    Text {
                        Layout.fillWidth: true
                        visible: root.uiState === "DECLINED"
                        text: I18n.t("pairing.error.declined")
                        color: Theme.danger
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontSmall
                        horizontalAlignment: Text.AlignHCenter
                        wrapMode: Text.Wrap
                    }

                    Text {
                        Layout.fillWidth: true
                        visible: root.uiState === "INVALID_CODE"
                        text: root.provider.pairingErrorKey.length > 0
                               ? I18n.t(root.provider.pairingErrorKey)
                               : I18n.t("pairing.error.tooShort")
                        color: Theme.danger
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontSmall
                        horizontalAlignment: Text.AlignHCenter
                        wrapMode: Text.Wrap
                    }

                    Text {
                        Layout.fillWidth: true
                        visible: root.uiState === "SERVER_UNAVAILABLE"
                        text: I18n.t("error.server.unavailable")
                        color: Theme.danger
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontSmall
                        horizontalAlignment: Text.AlignHCenter
                        wrapMode: Text.Wrap
                    }

                    Text {
                        Layout.fillWidth: true
                        visible: root.uiState === "SUCCESS"
                        text: I18n.t("pairing.success.subtitle")
                        color: Theme.success
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontSmall
                        font.weight: Font.DemiBold
                        horizontalAlignment: Text.AlignHCenter
                        wrapMode: Text.Wrap
                    }
                }

                // Device identity (informational only: never a pairing code) -
                HarborSectionCard {
                    Layout.fillWidth: true
                    title: I18n.t("onboarding.single.yourId")

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        Text {
                            Layout.fillWidth: true
                            text: root.selfHarborId.length > 0
                                   ? root.selfHarborId
                                   : I18n.t("common.notAvailable")
                            color: Theme.textMuted
                            font.family: Theme.fontFamilyMonospace
                            font.pixelSize: Theme.fontSmall
                            elide: Text.ElideMiddle
                        }

                        HarborButton {
                            id: copyIdButton
                            objectName: "onboardingCopyIdButton"
                            variant: "quiet"
                            text: I18n.t("common.actions.copy")
                            onClicked: root.copySelfId()
                        }
                    }
                }

                // Incoming request ------------------------------------------------
                HarborSectionCard {
                    Layout.fillWidth: true
                    visible: root.uiState === "INCOMING"
                    title: I18n.t("pairing.incoming.title")
                    iconName: "phone"

                    Text {
                        Layout.fillWidth: true
                        text: root.incoming && String(root.incoming.name || "").length > 0
                               ? I18n.t("pairing.incoming.description",
                                        { name: String(root.incoming.name) })
                               : I18n.t("pairing.incoming.unknown")
                        color: Theme.textSecondary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontBody
                        horizontalAlignment: Text.AlignHCenter
                        wrapMode: Text.Wrap
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp2

                        HarborButton {
                            Layout.fillWidth: true
                            variant: "primary"
                            text: I18n.t("pairing.incoming.accept")
                            onClicked: root.provider.acceptIncomingRequest()
                        }

                        HarborButton {
                            Layout.fillWidth: true
                            variant: "secondary"
                            text: I18n.t("pairing.incoming.decline")
                            onClicked: root.provider.declineIncomingRequest()
                        }
                    }
                }

                HarborButton {
                    id: continueButton
                    objectName: "onboardingContinueButton"
                    Layout.alignment: Qt.AlignHCenter
                    variant: "quiet"
                    text: I18n.t("onboarding.single.continueWithout")
                    onClicked: root.continueWithoutPairing()
                }
            }
        }
    }
}
