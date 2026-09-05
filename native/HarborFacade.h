#pragma once

#include <QHash>
#include <QJsonObject>
#include <QObject>
#include <QString>
#include <QUrl>
#include <QVariantMap>
#include <functional>

#include "HarborSettings.h"

class HarborCoreSupervisor;
class HarborPresenceSource;
class HarborAppIconProvider;

/// Typed, QML-facing surface over the supervised Rust core.
///
/// The facade never exposes raw protocol JSON: every request is correlated by
/// id, every response is mapped onto typed properties or structured error
/// keys, and private key material never crosses this boundary.
class HarborFacade final : public QObject
{
    Q_OBJECT
    Q_PROPERTY(QString coreState READ coreState NOTIFY coreStateChanged FINAL)
    Q_PROPERTY(bool coreReady READ coreReady NOTIFY coreReadyChanged FINAL)
    Q_PROPERTY(QString coreErrorKey READ coreErrorKey NOTIFY coreErrorKeyChanged FINAL)
    Q_PROPERTY(bool identityAvailable READ identityAvailable NOTIFY identityAvailableChanged FINAL)
    Q_PROPERTY(QString identityDeviceId READ identityDeviceId NOTIFY identityChanged FINAL)
    Q_PROPERTY(QString identityHarborId READ identityHarborId NOTIFY identityChanged FINAL)
    /// The operating system's real host name for this machine. Static fact,
    /// no IPC: the devices page names "this device" with it instead of a
    /// fixture label.
    Q_PROPERTY(QString deviceName READ deviceName CONSTANT FINAL)
    Q_PROPERTY(QString identityPublicKey READ identityPublicKey NOTIFY identityChanged FINAL)
    Q_PROPERTY(HarborSettings *settings READ settings CONSTANT FINAL)
    Q_PROPERTY(QString pairingPhase READ pairingPhase NOTIFY pairingChanged FINAL)
    Q_PROPERTY(QString pairingRole READ pairingRole NOTIFY pairingChanged FINAL)
    Q_PROPERTY(QString pairingCode READ pairingCode NOTIFY pairingChanged FINAL)
    Q_PROPERTY(QString pairingErrorKey READ pairingErrorKey NOTIFY pairingChanged FINAL)
    /// The most recent incoming pairing request: {name, pairingId} or empty.
    Q_PROPERTY(QVariantMap pairingIncoming READ pairingIncoming NOTIFY pairingChanged FINAL)
    Q_PROPERTY(bool serverConfigured READ serverConfigured NOTIFY serverChanged FINAL)
    Q_PROPERTY(QString serverAddress READ serverAddress NOTIFY serverChanged FINAL)
    /// Public SHA-256 certificate pin for the configured control-plane server.
    Q_PROPERTY(QString serverFingerprint READ serverFingerprint NOTIFY serverChanged FINAL)
    /// Latest real network diagnostics: a measured pinned TLS handshake and
    /// signed control-plane exchange, plus the live call's own transport
    /// facts. Empty until the first run; keys are camelCase.
    Q_PROPERTY(QVariantMap networkDiagnostics READ networkDiagnostics NOTIFY networkDiagnosticsChanged FINAL)
    Q_PROPERTY(bool networkDiagnosticsRunning READ networkDiagnosticsRunning NOTIFY networkDiagnosticsChanged FINAL)
    /// Durable paired-peer snapshot as {deviceId, harborId}. This excludes
    /// keys and gates peer-only UI affordances without a QML-owned flag.
    Q_PROPERTY(QVariantList pairedPeers READ pairedPeers NOTIFY pairedPeersChanged FINAL)
    Q_PROPERTY(bool pairedPeersResolved READ pairedPeersResolved NOTIFY pairedPeersResolvedChanged FINAL)
    /// Local activity entries as {id, category, titleKey, titleParams,
    /// descriptionKey, descriptionParams, occurredAt}; keys localize in QML.
    Q_PROPERTY(QVariantList activityTimeline READ activityTimeline NOTIFY activityChanged FINAL)
    /// Rolling-week activity totals: {games, apps, hours}.
    Q_PROPERTY(QVariantMap activityStats READ activityStats NOTIFY activityChanged FINAL)
    /// The local process monitor's state: idle/running/unsupported/unavailable.
    Q_PROPERTY(QString activityMonitorState READ activityMonitorState NOTIFY activityChanged FINAL)
    /// Core call lifecycle: IDLE/CONNECTING/RECONNECTING/CONNECTED/FAILED. The QML bridge
    /// maps this onto the existing AppState vocabulary; QML never sees SDP.
    Q_PROPERTY(QString callState READ callState NOTIFY callChanged FINAL)
    /// Opaque core-generated call id; empty when no call is active.
    Q_PROPERTY(QString callId READ callId NOTIFY callChanged FINAL)
    Q_PROPERTY(bool callMuted READ callMuted NOTIFY callChanged FINAL)
    /// Screen share exists only inside an active call: NOT_SHARING/SHARING.
    /// Capture is unavailable today, so SHARING is never fabricated.
    Q_PROPERTY(QString screenShareState READ screenShareState NOTIFY screenShareChanged FINAL)
    /// Sanitized direct DataChannel snapshot. It contains no file paths,
    /// hashes, chunks or worker transport details.
    Q_PROPERTY(QVariantList chatMessages READ chatMessages NOTIFY directChanged FINAL)
    Q_PROPERTY(QVariantList transfers READ transfers NOTIFY directChanged FINAL)
    /// Microphone self-check: live capture level, countdown, and peak from
    /// real devices. All zeros while idle; errors arrive as localized keys.
    Q_PROPERTY(bool micTestActive READ micTestActive NOTIFY micTestChanged FINAL)
    Q_PROPERTY(double micTestLevel READ micTestLevel NOTIFY micTestChanged FINAL)
    Q_PROPERTY(int micTestSecondsLeft READ micTestSecondsLeft NOTIFY micTestChanged FINAL)
    Q_PROPERTY(double micTestPeak READ micTestPeak NOTIFY micTestChanged FINAL)
    Q_PROPERTY(QString micTestError READ micTestError NOTIFY micTestChanged FINAL)
    /// The paired peer's public profile as {displayName, statusMessage,
    /// avatarType, avatar}. It arrives peer-to-peer over the direct channel
    /// and is cached durably by the core; it never carries identity, keys,
    /// revisions, or hashes across this boundary.
    Q_PROPERTY(QVariantMap partnerProfile READ partnerProfile NOTIFY profileChanged FINAL)
    /// The session's real audio devices as {id, name, isDefault}; empty until
    /// the core answers audio.devices.
    Q_PROPERTY(QVariantList audioInputs READ audioInputs NOTIFY audioChanged FINAL)
    Q_PROPERTY(QVariantList audioOutputs READ audioOutputs NOTIFY audioChanged FINAL)
    /// The live call's effective devices ("" = session default) and per-stream
    /// volumes as the worker really applies them, not the settings draft.
    Q_PROPERTY(QString audioInputDevice READ audioInputDevice NOTIFY audioChanged FINAL)
    Q_PROPERTY(QString audioOutputDevice READ audioOutputDevice NOTIFY audioChanged FINAL)
    Q_PROPERTY(qreal inputVolume READ inputVolume NOTIFY audioChanged FINAL)
    Q_PROPERTY(qreal outputVolume READ outputVolume NOTIFY audioChanged FINAL)
    /// Live voice facts measured on the audio the call already captures;
    /// no second capture stream exists.
    Q_PROPERTY(qreal voiceLevel READ voiceLevel NOTIFY voiceChanged FINAL)
    Q_PROPERTY(qreal remoteVoiceLevel READ remoteVoiceLevel NOTIFY voiceChanged FINAL)
    Q_PROPERTY(bool speaking READ speaking NOTIFY voiceChanged FINAL)
    Q_PROPERTY(bool remoteSpeaking READ remoteSpeaking NOTIFY voiceChanged FINAL)
    /// True while the push-to-talk key is held (only meaningful when the
    /// persisted push-to-talk mode is enabled).
    Q_PROPERTY(bool pttActive READ pttActive NOTIFY voiceChanged FINAL)
    /// Schema-validated activity records the paired peer shared this call.
    Q_PROPERTY(QVariantList remoteActivity READ remoteActivity NOTIFY activityChanged FINAL)
    /// Committed presence aggregate for both sides: {local: {state,
    /// previousState, changed, revision}, partner: {...} | null}. The
    /// private multi-signal evidence behind it never crosses this boundary.
    Q_PROPERTY(QJsonObject presence READ presence NOTIFY presenceChanged FINAL)
    /// Validated phone aggregates: {own: {...} | null, peer: {...} | null}.
    /// Each side mirrors the paired phone's MobileStatus — battery, coarse
    /// activity, consented location fix, share toggles — or null when that
    /// side never shared. Contents stay typed here; text renders in QML.
    Q_PROPERTY(QVariantMap mobileState READ mobileState NOTIFY mobileChanged FINAL)
    /// Why the last attempt failed or was refused: "" or a stable code such
    /// as "declined" or "busy". A resting fact the UI may show after ENDED.
    Q_PROPERTY(QString callEndReason READ callEndReason NOTIFY callChanged FINAL)
    /// Live transport facts the media worker measured on its own connection:
    /// round-trip time and cumulative loss, plus the core's honest verdict
    /// ("good"/"fair"/"poor"; "unknown" until the first real sample).
    Q_PROPERTY(qreal callRttMs READ callRttMs NOTIFY callStatsChanged FINAL)
    Q_PROPERTY(qreal callLossPct READ callLossPct NOTIFY callStatsChanged FINAL)
    Q_PROPERTY(QString callQuality READ callQuality NOTIFY callStatsChanged FINAL)

public:
    explicit HarborFacade(HarborCoreSupervisor *supervisor, QObject *parent = nullptr);

    QString coreState() const;
    bool coreReady() const;
    QString coreErrorKey() const;

    bool identityAvailable() const;
    QString identityDeviceId() const;
    QString identityHarborId() const;
    QString deviceName() const;
    QString identityPublicKey() const;

    HarborSettings *settings() const;

    QString pairingPhase() const;
    QString pairingRole() const;
    QString pairingCode() const;
    QString pairingErrorKey() const;
    QVariantMap pairingIncoming() const;
    bool serverConfigured() const;
    QString serverAddress() const;
    QString serverFingerprint() const;
    QVariantMap networkDiagnostics() const;
    bool networkDiagnosticsRunning() const;
    QVariantList pairedPeers() const;
    bool pairedPeersResolved() const;

    QVariantList activityTimeline() const;
    QVariantMap activityStats() const;
    QString activityMonitorState() const;
    /// Real program icon for an activity entry as a small PNG data URL.
    /// `appId`/`iconKey` are the theme-safe keys the core sends
    /// (e.g. `firefox`); empty means "unknown, use the category glyph".
    /// Resolved natively per OS, cached, never touches the network.
    Q_INVOKABLE QString appIconUrl(const QString &appId, const QString &iconKey) const;

    Q_INVOKABLE void refreshIdentity();
    Q_INVOKABLE void refreshActivity();
    Q_INVOKABLE void refreshMobile();
    Q_INVOKABLE void retryCore();
    Q_INVOKABLE void shutdownCore();
    Q_INVOKABLE void copyToClipboard(const QString &text);

    /// Pins the control-plane server this device pairs through. The
    /// fingerprint is the hex SHA-256 of the server certificate (public
    /// pinning material, never a secret).
    Q_INVOKABLE void configureServer(const QString &address, const QString &fingerprint);
    Q_INVOKABLE void refreshServerConfig();
    /// Reads one local image through the native boundary, strips metadata,
    /// bounds its dimensions, and returns only a small embedded PNG data URL.
    /// File URLs and source paths never enter settings or the peer protocol.
    Q_INVOKABLE QString importProfileAvatar(const QUrl &fileUrl) const;
    Q_INVOKABLE void runNetworkDiagnostics();
    Q_INVOKABLE void refreshPairedPeers();
    Q_INVOKABLE void refreshPairingState();
    /// Host flow: register a fresh six-digit code and wait for approval.
    Q_INVOKABLE void pairHostCreate();
    /// Peer flow: open code entry, submit, watch the host's decision.
    Q_INVOKABLE void pairEnterCode();
    Q_INVOKABLE void pairSubmit(const QString &code);
    Q_INVOKABLE void pairPollIncoming();
    Q_INVOKABLE void pairPollStatus();
    Q_INVOKABLE void pairAccept();
    Q_INVOKABLE void pairDecline();
    Q_INVOKABLE void pairCancel();
    Q_INVOKABLE void pairReset();

    QString callState() const;
    QString callId() const;
    bool callMuted() const;
    QString callEndReason() const;
    qreal callRttMs() const;
    qreal callLossPct() const;
    QString callQuality() const;
    QString screenShareState() const;
    QVariantList chatMessages() const;
    QVariantList transfers() const;
    bool micTestActive() const;
    double micTestLevel() const;
    int micTestSecondsLeft() const;
    double micTestPeak() const;
    QString micTestError() const;
    QVariantMap partnerProfile() const;
    QVariantList audioInputs() const;
    QVariantList audioOutputs() const;
    QString audioInputDevice() const;
    QString audioOutputDevice() const;
    qreal inputVolume() const;
    qreal outputVolume() const;
    qreal voiceLevel() const;
    qreal remoteVoiceLevel() const;
    bool speaking() const;
    bool remoteSpeaking() const;
    bool pttActive() const;
    QVariantList remoteActivity() const;
    QJsonObject presence() const;
    QVariantMap mobileState() const;

    /// Local call bootstrap against the private media worker. Success lands
    /// in CONNECTING; a real peer connection still needs signaling.
    Q_INVOKABLE void startCall();
    Q_INVOKABLE void endCall();
    Q_INVOKABLE void setCallMuted(bool muted);
    /// An offered call rings as INCOMING; nothing about the media path
    /// exists until this device's user approves it here.
    Q_INVOKABLE void acceptIncomingCall();
    Q_INVOKABLE void declineIncomingCall();
    /// Refused by the core while no call is CONNECTED, and by the media
    /// worker until a native capture adapter exists. The refusal surfaces as
    /// a localized error key, never as a fabricated sharing state.
    Q_INVOKABLE void startScreenShare();
    Q_INVOKABLE void stopScreenShare();
    Q_INVOKABLE void refreshDirectState();
    /// Starts a bounded microphone self-check (seconds clamped by the core
    /// to 3..15). Refused honestly while a call owns the devices.
    Q_INVOKABLE void startMicTest(int seconds);
    /// Polls the running self-check; a finished test reports its final
    /// snapshot once, then idles.
    Q_INVOKABLE void pollMicTest();
    /// Ends the self-check early, reporting the peak measured so far.
    Q_INVOKABLE void stopMicTest();
    Q_INVOKABLE void sendChatMessage(const QString &body);
    /// Offers a local file to the peer. The core refuses oversized sources
    /// and unsafe names before any byte is read; reply and events carry the
    /// sanitized transfer snapshot only.
    Q_INVOKABLE void offerLocalFile(const QString &sourcePath);
    Q_INVOKABLE void acceptTransfer(const QString &transferId);
    Q_INVOKABLE void rejectTransfer(const QString &transferId);
    Q_INVOKABLE void cancelTransfer(const QString &transferId);
    /// Refreshes the cached partner-profile snapshot from the core.
    Q_INVOKABLE void refreshProfileState();
    /// Re-reads the committed presence aggregate (reconnecting UIs only;
    /// live transitions arrive as events).
    Q_INVOKABLE void refreshPresence();

    /// Enumerates the session's real audio devices through the core.
    Q_INVOKABLE void refreshAudioDevices();
    /// Persists and live-applies the per-stream volumes (0..1).
    Q_INVOKABLE void setAudioVolumes(qreal inputVolume, qreal outputVolume);
    /// Persists and live-applies a device pair ("" keeps the session
    /// default). A failed swap on a live call is reported, not faked.
    Q_INVOKABLE void selectAudioDevices(const QString &inputId, const QString &outputId);
    /// Persists the push-to-talk mode and updates a live call's gate.
    Q_INVOKABLE void setPushToTalkEnabled(bool enabled);
    /// Key down/up for the push-to-talk gate; ignored when disabled.
    Q_INVOKABLE void setPushToTalkActive(bool active);

signals:
    void coreStateChanged();
    void coreReadyChanged();
    void coreErrorKeyChanged();
    void identityAvailableChanged();
    void identityChanged();
    /// A request failed at the core; `uiKey` localizes the failure.
    void requestFailed(const QString &requestType, const QString &uiKey);
    void pairingChanged();
    void serverChanged();
    void pairedPeersChanged();
    void pairedPeersResolvedChanged();
    /// Local activity timeline, stats or monitor state changed.
    void activityChanged();
    /// Call state, id or mute flag changed.
    void callChanged();
    /// Screen-share state changed (only ever inside an active call).
    void screenShareChanged();
    void directChanged();
    /// Microphone self-check facts changed (level, countdown, peak, error).
    void micTestChanged();
    /// The paired peer shared newer public-profile state.
    void profileChanged();
    /// The committed presence aggregate changed (either side).
    void presenceChanged();
    /// Either phone aggregate changed (own or peer, possibly to null).
    void mobileChanged();
    /// Audio device lists, effective device selection or volumes changed.
    void audioChanged();
    /// Live voice-level facts changed.
    void voiceChanged();
    /// Live transport facts (RTT, loss, quality verdict) changed.
    void callStatsChanged();
    /// One display-only phone notification. It is never copied into
    /// AppState's durable notification/history model.
    void phoneNotification(const QJsonObject &notification);
    void networkDiagnosticsChanged();
    /// The pairing session reached ACCEPTED on this device.
    void pairingCompleted();

private:
    void setCoreState(const QString &state);
    void setCoreReady(bool ready);
    void setCoreErrorKey(const QString &errorKey);
    void setIdentityAvailable(bool available);
    void sendHello();
    void sendRequest(const QString &requestType, const QJsonObject &payload,
                     std::function<void(const QJsonObject &)> onPayload);
    void handleEnvelope(const QJsonObject &envelope);
    void handleCoreReady(const QJsonObject &payload);
    void fetchIdentity();
    void fetchSettings();
    void applyIdentity(const QJsonObject &payload);
    void fetchServerConfig();
    void applyServerConfig(const QJsonObject &payload);
    void fetchPairedPeers();
    void applyPairedPeers(const QJsonObject &payload);
    void fetchActivity();
    void applyActivity(const QJsonObject &payload);
    void fetchMobile();
    void applyMobile(const QJsonObject &payload);
    void resetMobileState();
    void resetActivityState();
    void applyCallState(const QJsonObject &payload);
    void applyShareState(const QJsonObject &payload);
    void resetCallState();
    void applyVoiceLevels(const QJsonObject &payload);
    void resetVoiceLevels();
    void applyCallStats(const QJsonObject &payload);
    void resetCallStats();
    void applyNetworkDiagnostics(const QJsonObject &payload);
    void applyAudioDevices(const QJsonObject &payload);
    void applyAudioConfig(const QJsonObject &payload);
    void applyPushToTalk(const QJsonObject &payload);
    void transferDecision(const QString &requestType, const QString &transferId);
    void applyDirectState(const QJsonObject &payload);
    void resetDirectState();
    void applyMicTestState(const QJsonObject &payload);
    void applyMicTestError(const QString &uiKey);
    void applyProfileState(const QJsonObject &payload);
    void applyPresenceState(const QJsonObject &payload);
    /// Native detector tick: pushes one private snapshot into the core.
    void sendPresenceSense(const QJsonObject &snapshot);
    void resetPresenceState();
    void requestPairing(const QString &requestType, const QJsonObject &payload);
    void applyPairingState(const QJsonObject &payload);
    void resetPairingState();

    HarborCoreSupervisor *m_supervisor = nullptr;
    HarborSettings *m_settings = nullptr;
    QHash<QString, std::function<void(const QJsonObject &)>> m_pending;
    QString m_coreState = QStringLiteral("stopped");
    bool m_coreReady = false;
    QString m_coreErrorKey;
    bool m_identityAvailable = false;
    QString m_deviceId;
    QString m_harborId;
    QString m_publicKey;
    QString m_pairingPhase = QStringLiteral("IDLE");
    QString m_pairingRole;
    QString m_pairingCode;
    QVariantMap m_networkDiagnostics;
    bool m_networkDiagnosticsRunning = false;
    QString m_pairingErrorKey;
    QVariantMap m_pairingIncoming;
    bool m_serverConfigured = false;
    QString m_serverAddress;
    QString m_serverFingerprint;
    QVariantList m_pairedPeers;
    bool m_pairedPeersResolved = false;
    QVariantList m_activityTimeline;
    QVariantMap m_activityStats;
    QString m_activityMonitorState = QStringLiteral("idle");
    QString m_callState = QStringLiteral("IDLE");
    QString m_callId;
    bool m_callMuted = false;
    QString m_callEndReason;
    qreal m_callRttMs = 0.0;
    qreal m_callLossPct = 0.0;
    QString m_callQuality = QStringLiteral("unknown");
    QString m_screenShareState = QStringLiteral("NOT_SHARING");
    QVariantList m_chatMessages;
    QVariantList m_transfers;
    bool m_micTestActive = false;
    double m_micTestLevel = 0.0;
    int m_micTestSecondsLeft = 0;
    double m_micTestPeak = 0.0;
    QString m_micTestError;
    QVariantMap m_partnerProfile;
    QVariantList m_audioInputs;
    QVariantList m_audioOutputs;
    QString m_audioInputDevice;
    QString m_audioOutputDevice;
    qreal m_inputVolume = 1.0;
    qreal m_outputVolume = 1.0;
    qreal m_voiceLevel = 0.0;
    qreal m_remoteVoiceLevel = 0.0;
    bool m_speaking = false;
    bool m_remoteSpeaking = false;
    bool m_pttActive = false;
    QVariantList m_remoteActivity;
    QJsonObject m_presence;
    QVariantMap m_mobileState;
    HarborPresenceSource *m_presenceSource = nullptr;
    HarborAppIconProvider *m_appIcons = nullptr;
};
