pragma ComponentBehavior: Bound
import QtQml

// Production pairing provider between the supervised Rust core and the
// PairingView. It mirrors the MockController pairing contract one-to-one
// (modes, properties, actions, signals) so the view swaps providers without
// knowing which one is live.
//
// The core's pairing phases drive the modes: IDLE→choice, ENTERING_CODE→
// request, WAITING_APPROVAL→qr, REQUESTING→waiting, INCOMING_REQUEST→
// incoming, ACCEPTED→success, DECLINED/ERROR→error. A host's own decline
// closes the overlay (the mock's behavior); a peer's declined request lands
// on the error page with a real key.
//
// Polling keeps the flows alive: a host watches pairing.incoming while
// showing the code, a peer watches pairing.status while waiting. Retryable
// server outages (error.server.unavailable) never kick the user off an
// in-flight page — the next poll retries.
//
// The facade is a C++ context property, so this glue file is deliberately
// dynamically typed; qmllint cannot know its members. QtObject has no default
// property, so timers and connections live in an explicit list.
// qmllint disable missing-property
QtObject {
    id: provider

    property QtObject facade: null
    readonly property bool live: facade !== null && facade.coreReady

    // Code time-to-live as the server enforces it (5 minutes).
    readonly property int codeTtlSeconds: 300

    // ---- MockController pairing contract ---------------------------------
    property string pairingMode: "choice"
    readonly property string pairingCode: live ? facade.pairingCode : ""
    property int pairingCodeSeconds: 0
    property string enteredPairingCode: ""
    property string pairingErrorKey: ""
    property var pairingErrorParams: ({})
    property var incomingRequest: ({ name: "", initials: "", code: "" })
    property bool mockCopyFeedbackVisible: false
    property string mockCopyTarget: ""

    signal pairingCompleted(string partnerName)
    signal incomingPairingDeclined(string partnerName)

    // ---- Contract actions -------------------------------------------------

    function setPairingMode(mode) {
        if (!live)
            return ""
        if (mode === "qr") {
            facade.pairHostCreate()
            pairingCodeSeconds = codeTtlSeconds
            _enterMode("qr")
            incomingPoll.restart()
        } else if (mode === "request") {
            facade.pairEnterCode()
            _enterMode("request")
        } else if (mode === "choice") {
            facade.pairReset()
            _stopPollers()
            _resetLocal()
        }
        return pairingMode
    }

    function closePairing() {
        if (!live) {
            AppState.pairingVisible = false
            return
        }
        _stopPollers()
        countdown.stop()
        facade.pairReset()
        _resetLocal()
        AppState.pairingVisible = false
    }

    /// Simulated pairing exists only in the deterministic test provider; the
    /// real one never fakes success.
    function completePairing(partnerName) {
        return ""
    }

    function normalizePairingCode(value) {
        // Real pairing codes are six digits.
        return String(value || "").replace(/[^0-9]/g, "")
    }

    function submitPairingCode(value) {
        if (!live)
            return false
        var normalized = normalizePairingCode(value)
        enteredPairingCode = normalized
        pairingErrorParams = ({})
        if (normalized.length !== 6) {
            pairingErrorKey = "pairing.error.tooShort"
            _enterMode("error")
            return false
        }
        pairingErrorKey = ""
        _enterMode("waiting")
        facade.pairSubmit(normalized)
        statusPoll.restart()
        return true
    }

    function cancelPairingRequest() {
        if (!live)
            return
        statusPoll.stop()
        if (pairingMode === "waiting") {
            facade.pairCancel()
            enteredPairingCode = ""
            _enterMode("request")
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

    function mockCopy(value, target) {
        if (live)
            facade.copyToClipboard(String(value || ""))
        mockCopyTarget = String(target || "pairingCode")
        mockCopyFeedbackVisible = true
        copyFeedbackTimer.restart()
        return String(value || "")
    }

    // ---- Internal state machine -------------------------------------------

    function _enterMode(mode) {
        pairingMode = mode
    }

    function _resetLocal() {
        pairingMode = "choice"
        pairingCodeSeconds = 0
        enteredPairingCode = ""
        pairingErrorKey = ""
        pairingErrorParams = ({})
        incomingRequest = ({ name: "", initials: "", code: "" })
        mockCopyFeedbackVisible = false
    }

    function _stopPollers() {
        incomingPoll.stop()
        statusPoll.stop()
    }

    /// The overlay reopening always starts clean: local state resets and the
    /// core session is reset (pairing.reset is local and always succeeds)
    /// before the authoritative state is refreshed.
    function _onOverlayOpened() {
        _stopPollers()
        countdown.stop()
        facade.pairReset()
        _resetLocal()
        facade.refreshPairingState()
    }

    function _syncFromFacade() {
        if (!live)
            return
        var phase = facade.pairingPhase
        if (phase === "IDLE") {
            _enterMode("choice")
        } else if (phase === "ENTERING_CODE") {
            _enterMode("request")
        } else if (phase === "WAITING_APPROVAL") {
            pairingCodeSeconds = codeTtlSeconds
            _enterMode("qr")
            countdown.restart()
            incomingPoll.restart()
        } else if (phase === "REQUESTING") {
            _enterMode("waiting")
            statusPoll.restart()
        } else if (phase === "INCOMING_REQUEST") {
            var peer = facade.pairingIncoming
            incomingRequest = {
                name: String(peer.name || ""),
                initials: AppState.initialsFor(String(peer.name || "")),
                // The host verifies against the code it is displaying.
                code: facade.pairingCode
            }
            _enterMode("incoming")
        } else if (phase === "ACCEPTED") {
            _stopPollers()
            var name = String(incomingRequest.name || "")
            if (name.length > 0) {
                var patch = { name: name, initials: AppState.initialsFor(name) }
                // Fabricated "online" only until real presence takes over;
                // while the aggregate is authoritative it must not fight it.
                if (!AppState.presenceAuthoritative)
                    patch.presence = "online"
                AppState.updatePartnerProfile(patch)
            }
            AppState.setConnection("connected")
            _enterMode("success")
            pairingCompleted(name)
        } else if (phase === "DECLINED") {
            _stopPollers()
            if (facade.pairingRole === "host") {
                // The host's own refusal ends the request, like the mock's.
                var declined = String(incomingRequest.name || "")
                _resetLocal()
                AppState.pairingVisible = false
                incomingPairingDeclined(declined)
            } else {
                pairingErrorKey = "pairing.error.declined"
                pairingErrorParams = ({})
                _enterMode("error")
            }
        } else if (phase === "ERROR") {
            // A retryable outage during polling keeps the current page; the
            // next poll retries and the core session stays intact.
            if (facade.pairingErrorKey === "error.server.unavailable"
                    && (pairingMode === "qr" || pairingMode === "waiting"))
                return
            _stopPollers()
            pairingErrorKey = facade.pairingErrorKey
            pairingErrorParams = ({})
            _enterMode("error")
        }
    }

    readonly property list<QtObject> wiring: [
        Timer {
            id: countdown

            interval: 1000
            repeat: true
            running: false
            onTriggered: {
                if (provider.pairingCodeSeconds > 0)
                    provider.pairingCodeSeconds--
                if (provider.pairingCodeSeconds <= 0)
                    countdown.stop()
            }
        },
        Timer {
            id: incomingPoll

            interval: 3000
            repeat: true
            running: false
            onTriggered: {
                if (provider.live && provider.pairingMode === "qr")
                    provider.facade.pairPollIncoming()
            }
        },
        Timer {
            id: statusPoll

            interval: 3000
            repeat: true
            running: false
            onTriggered: {
                if (provider.live && provider.pairingMode === "waiting")
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
                    countdown.stop()
                    provider.facade.pairReset()
                    provider._resetLocal()
                }
            }
        }
    ]
}
