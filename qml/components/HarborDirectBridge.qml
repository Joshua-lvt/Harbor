pragma ComponentBehavior: Bound
import QtQml

// Metadata-only bridge for peer-to-peer DataChannel state. The core owns the
// transcript, queue, transfer staging and destination policy; QML can only
// request a bounded message send and mirror sanitized snapshots.
// qmllint disable missing-property
QtObject {
    id: provider

    property QtObject facade: null
    readonly property bool live: facade !== null && facade.coreReady
    property bool hadLive: false

    function refresh() {
        if (live) {
            facade.refreshDirectState()
            facade.refreshProfileState()
        }
    }

    function sendMessage(body) {
        if (live)
            facade.sendChatMessage(String(body))
    }

    function offerFile(path) {
        if (live)
            facade.offerLocalFile(String(path))
    }

    function acceptTransfer(id) {
        if (live)
            facade.acceptTransfer(String(id))
    }

    function rejectTransfer(id) {
        if (live)
            facade.rejectTransfer(String(id))
    }

    function cancelTransfer(id) {
        if (live)
            facade.cancelTransfer(String(id))
    }

    /// Where completed inbound files land; empty means the platform default.
    readonly property string transferDirectory: live ? facade.settings.transferDirectory : ""

    function setTransferDirectory(path) {
        if (live)
            facade.settings.transferDirectory = String(path)
    }

    function _sync() {
        var current = facade
        if (current === null || !current.coreReady) {
            if (hadLive) {
                hadLive = false
                AppState.setDirectState([], [])
            }
            return
        }
        hadLive = true
        AppState.setDirectState(current.chatMessages, current.transfers)
        _syncPartner(current.partnerProfile)
    }

    // PartnerProfileUpdated, folded into the direct sync: the peer's public
    // fields land on the session partner profile. An empty display name
    // never blanks a name learned earlier (pairing or a previous update);
    // avatar and status follow the core's validated snapshot as-is, so a
    // removal returns the peer to initials honestly. Presence stays owned
    // by the contacts bridge — never by profile content.
    function _syncPartner(partner) {
        if (!partner || typeof partner !== "object")
            return
        var patch = {}
        var name = String(partner.displayName || "")
        if (name.length > 0)
            patch.name = name
        patch.status = String(partner.statusMessage || "")
        patch.avatar = String(partner.avatar || "")
        var avatarType = String(partner.avatarType || "image")
        patch.avatarType = avatarType === "gif" ? "gif" : "image"
        AppState.updatePartnerProfile(patch)
    }

    Component.onCompleted: _sync()
    onFacadeChanged: _sync()

    readonly property list<QtObject> wiring: [
        Connections {
            target: provider.facade
            function onDirectChanged() { provider._sync() }
            function onProfileChanged() { provider._sync() }
            function onCoreReadyChanged() { provider._sync() }
            function onRequestFailed(requestType, uiKey) {
                if (String(requestType).indexOf("chat.") !== 0
                    && String(requestType).indexOf("transfer.") !== 0)
                    return
                AppState.requestLocalizedToast("direct",
                                               String(uiKey || "error.core.unavailable"),
                                               {}, "", {})
            }
        }
    ]
}
