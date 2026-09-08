pragma ComponentBehavior: Bound
import QtQml

// Production pairing provider between the supervised Rust core and the
// pairing surfaces (PairingView modal, OnboardingView pairing screen). It
// mirrors the MockController pairing contract one-to-one (modes,
// properties, actions, signals) so views swap providers without knowing
// which one is live.
//
// Pairing is Harbor-ID based: this device shows its own Harbor ID, the user
// types the peer's full Harbor ID, and the core invites it
// (`pairing.invite`). The six-digit code flow is legacy (older clients);
// this provider never starts it.
//
// The core's pairing phases drive the modes: IDLE/ENTERING_CODE→home,
// REQUESTING→connecting, INCOMING_REQUEST→incoming, ACCEPTED→success,
// DECLINED/ERROR→error. (WAITING_APPROVAL only exists in the legacy host
// flow, which this UI never starts; it maps back to home defensively.)
//
// The facade watches pairing.incoming globally, including while this surface
// is closed; connecting watches pairing.status here. Retryable server outages
// never kick the user off an in-flight page. A `connecting` guard prevents
// simultaneous duplicate requests.
//
// The facade is a C++ context property, so this glue file is deliberately
// dynamically typed; qmllint cannot know its members. QtObject has no default
// property, so timers and connections live in an explicit list.
// qmllint disable missing-property
QtObject {
    id: provider

    property QtObject facade: null
    readonly property bool live: facade !== null && facade.coreReady

    // ---- Harbor-ID contract ----------------------------------------------
    readonly property string ownHarborId: live ? String(facade.identityHarborId || "") : ""
    property string enteredHarborId: ""
    // empty | invalid | valid — the view binds button state and helper text.
    readonly property string harborIdFieldState: {
        var value = String(provider.enteredHarborId || "")
        if (value.length === 0)
            return "empty"
        return provider.isValidHarborId(value) ? "valid" : "invalid"
    }
    // Single-flight guard: one invite on the wire at a time.
    property bool connecting: false
    property string pairingMode: "home"
    property string pairingErrorKey: ""
    property var pairingErrorParams: ({})
    property var incomingRequest: ({ harborId: "", name: "", pairingId: "" })
    property bool mockCopyFeedbackVisible: false
    property string mockCopyTarget: ""

    signal pairingCompleted(string partnerHarborId)
    signal incomingPairingDeclined(string partnerHarborId)

    onLiveChanged: {
        if (live)
            _syncFromFacade()
    }
    Component.onCompleted: _syncFromFacade()

    // ---- Contract actions -------------------------------------------------

    /// Canonical Harbor-ID check, shared with the core and server gates:
    /// exactly `harbor-` plus 8 hex characters. Outer whitespace is invalid;
    /// never strips the prefix, never converts to a number.
    function isValidHarborId(value) {
        return /^harbor-[0-9a-fA-F]{8}$/.test(String(value || ""))
    }

    function setPairingMode(mode) {
        // Kept for callers written against the legacy mode vocabulary
        // (developer previews): anything unknown lands on home.
        var values = ["home", "connecting", "success", "error", "incoming"]
        _enterMode(values.indexOf(mode) >= 0 ? mode : "home")
        return pairingMode
    }

    function connectWithHarborId(value) {
        if (!live || provider.connecting)
            return false
        var harborId = String(value || "")
        if (!provider.isValidHarborId(harborId)) {
            provider.enteredHarborId = String(value || "")
            provider.pairingErrorKey = ""
            provider.pairingErrorParams = ({})
            return false
        }
        provider.enteredHarborId = harborId
        provider.pairingErrorKey = ""
        provider.pairingErrorParams = ({})
        provider.connecting = true
        _enterMode("connecting")
        facade.pairInvite(harborId)
        statusPoll.restart()
        return true
    }

    function retryConnect() {
        provider.connecting = false
        return provider.connectWithHarborId(provider.enteredHarborId)
    }

    function cancelPairingRequest() {
        if (!live)
            return
        statusPoll.stop()
        if (provider.connecting || pairingMode === "connecting") {
            provider.connecting = false
            facade.pairCancel()
            _enterMode("home")
        }
    }

    function acceptIncomingRequest() {
        if (!live || pairingMode !== "incoming")
            return false
        facade.pairAccept()
        return true
    }

    function declineIncomingRequest() {
        if (!live || pairingMode !== "incoming")
            return false
        facade.pairDecline()
        return true
    }

    function closePairing() {
        if (!live) {
            AppState.pairingVisible = false
            return
        }
        _stopPollers()
        provider.connecting = false
        if (pairingMode === "incoming")
            facade.pairDecline()
        facade.pairReset()
        _resetLocal()
        AppState.pairingVisible = false
    }

    /// Simulated pairing exists only in the deterministic test provider; the
    /// real one never fakes success.
    function completePairing(partnerHarborId) {
        return ""
    }

    function copyHarborId() {
        return provider.mockCopy(provider.ownHarborId, "harborId")
    }

    function mockCopy(value, target) {
        // Harbor-ID copies must reach the real system clipboard whenever
        // the facade exists — even while the core is reconnecting
        // (live === false). Gating on `live` once showed a "copied"
        // feedback while copying nothing. Pairing actions below still
        // require `live`.
        if (facade)
            facade.copyToClipboard(String(value || ""))
        mockCopyTarget = String(target || "harborId")
        mockCopyFeedbackVisible = true
        copyFeedbackTimer.restart()
        return String(value || "")
    }

    // ---- Internal state machine -------------------------------------------

    function _enterMode(mode) {
        pairingMode = mode
    }

    function _resetLocal() {
        pairingMode = "home"
        enteredHarborId = ""
        connecting = false
        pairingErrorKey = ""
        pairingErrorParams = ({})
        incomingRequest = ({ harborId: "", name: "", pairingId: "" })
        mockCopyFeedbackVisible = false
    }

    function _stopPollers() {
        statusPoll.stop()
    }

    /// The overlay reopening always starts clean: local state resets and the
    /// core session is reset (pairing.reset is local and always succeeds)
    /// before the authoritative state is refreshed.
    function _onOverlayOpened() {
        // An incoming request opens this overlay automatically. Preserve the
        // request instead of resetting it before the user can respond.
        if (facade.pairingPhase === "INCOMING_REQUEST") {
            _syncFromFacade()
            return
        }
        _stopPollers()
        provider.connecting = false
        facade.pairReset()
        _resetLocal()
        facade.refreshPairingState()
    }

    function _syncFromFacade() {
        if (!live)
            return
        var phase = facade.pairingPhase
        if (phase === "IDLE" || phase === "ENTERING_CODE" || phase === "WAITING_APPROVAL") {
            if (pairingMode !== "home")
                _enterMode("home")
        } else if (phase === "REQUESTING") {
            _enterMode("connecting")
            statusPoll.restart()
        } else if (phase === "INCOMING_REQUEST") {
            var peer = facade.pairingIncoming
            var harborId = String(peer.harborId || peer.name || "")
            incomingRequest = {
                harborId: harborId,
                name: harborId,
                pairingId: String(peer.pairingId || "")
            }
            provider.connecting = false
            _enterMode("incoming")
        } else if (phase === "ACCEPTED") {
            _stopPollers()
            provider.connecting = false
            var partner = String(facade.pairingPeerHarborId
                                 || provider.enteredHarborId
                                 || incomingRequest.harborId || "")
            if (partner.length > 0) {
                var patch = { name: partner, initials: AppState.initialsFor(partner) }
                // Fabricated "online" only until real presence takes over;
                // while the aggregate is authoritative it must not fight it.
                if (!AppState.presenceAuthoritative)
                    patch.presence = "online"
                AppState.updatePartnerProfile(patch)
            }
            AppState.setConnection("connected")
            _enterMode("success")
            pairingCompleted(partner)
        } else if (phase === "DECLINED") {
            _stopPollers()
            provider.connecting = false
            pairingErrorKey = "pairing.error.declined"
            pairingErrorParams = ({})
            _enterMode("error")
        } else if (phase === "ERROR") {
            // A retryable outage during polling keeps the current page; the
            // next poll retries and the core session stays intact.
            if (facade.pairingErrorKey === "error.server.unavailable"
                    && (pairingMode === "connecting" || pairingMode === "home"))
                return
            _stopPollers()
            provider.connecting = false
            pairingErrorKey = facade.pairingErrorKey
            pairingErrorParams = ({})
            _enterMode("error")
        }
    }

    readonly property list<QtObject> wiring: [
        Timer {
            id: statusPoll

            interval: 3000
            repeat: true
            running: false
            onTriggered: {
                if (provider.live && provider.pairingMode === "connecting")
                    provider.facade.pairPollStatus()
            }
        },
        Timer {
            id: copyFeedbackTimer

            interval: 2000
            running: false
            onTriggered: provider.mockCopyFeedbackVisible = false
        },
        Connections {
            target: provider.facade

            function onPairingChanged() {
                provider._syncFromFacade()
            }
        },
        Connections {
            target: AppState

            function onPairingVisibleChanged() {
                if (AppState.pairingVisible && provider.live) {
                    provider._onOverlayOpened()
                } else if (!AppState.pairingVisible && provider.live) {
                    // The shell may close the overlay without going through
                    // the view (for example after route exclusivity). Never
                    // leave a core pairing session or poller alive off-screen.
                    provider._stopPollers()
                    provider.connecting = false
                    if (provider.pairingMode === "incoming")
                        provider.facade.pairDecline()
                    provider.facade.pairReset()
                    provider._resetLocal()
                }
            }
        }
    ]
}
