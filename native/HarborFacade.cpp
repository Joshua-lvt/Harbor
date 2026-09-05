#include "HarborFacade.h"

#include "HarborAppIcon.h"
#include "HarborCoreSupervisor.h"
#include "HarborPresenceSource.h"

#include <QBuffer>
#include <QClipboard>
#include <QDateTime>
#include <QFile>
#include <QFileInfo>
#include <QGuiApplication>
#include <QImage>
#include <QImageReader>
#include <QJsonArray>
#include <QJsonObject>
#include <QSysInfo>
#include <QUuid>
#include <cmath>

namespace {
QString stateName(HarborCoreSupervisor::State state)
{
    switch (state) {
    case HarborCoreSupervisor::State::Stopped:
        return QStringLiteral("stopped");
    case HarborCoreSupervisor::State::Starting:
        return QStringLiteral("starting");
    case HarborCoreSupervisor::State::Running:
        return QStringLiteral("running");
    case HarborCoreSupervisor::State::Backoff:
        return QStringLiteral("reconnecting");
    case HarborCoreSupervisor::State::Failed:
        return QStringLiteral("failed");
    }

    return QStringLiteral("failed");
}
} // namespace

HarborFacade::HarborFacade(HarborCoreSupervisor *supervisor, QObject *parent)
    : QObject(parent)
    , m_supervisor(supervisor)
    , m_settings(new HarborSettings(this))
    , m_presenceSource(new HarborPresenceSource(this))
    , m_appIcons(new HarborAppIconProvider(this))
{
    Q_ASSERT(m_supervisor);
    // Detector facts flow one way: snapshot into the core, aggregate back
    // as events. The source itself stays unaware of the protocol.
    connect(m_presenceSource, &HarborPresenceSource::snapshotReady,
            this, &HarborFacade::sendPresenceSense);
    connect(m_supervisor, &HarborCoreSupervisor::stateChanged, this, [this](HarborCoreSupervisor::State state) {
        setCoreState(stateName(state));
        if (state != HarborCoreSupervisor::State::Running) {
            m_pending.clear();
            setIdentityAvailable(false);
            setCoreReady(false);
            resetPairingState();
            if (!m_pairedPeers.isEmpty()) {
                m_pairedPeers.clear();
                emit pairedPeersChanged();
            }
            if (m_pairedPeersResolved) {
                m_pairedPeersResolved = false;
                emit pairedPeersResolvedChanged();
            }
            resetActivityState();
            // The core process owned the call; its death is a call teardown.
            resetCallState();
            resetDirectState();
            resetPresenceState();
            m_presenceSource->stop();
        }
    });
    connect(m_supervisor, &HarborCoreSupervisor::processStarted,
            this, &HarborFacade::sendHello);
    connect(m_supervisor, &HarborCoreSupervisor::envelopeReceived,
            this, &HarborFacade::handleEnvelope);
    connect(m_supervisor, &HarborCoreSupervisor::faulted,
            this, [this](const QString &errorKey) {
                m_pending.clear();
                setIdentityAvailable(false);
                setCoreReady(false);
                resetPairingState();
                if (!m_pairedPeers.isEmpty()) {
                    m_pairedPeers.clear();
                    emit pairedPeersChanged();
                }
                if (m_pairedPeersResolved) {
                    m_pairedPeersResolved = false;
                    emit pairedPeersResolvedChanged();
                }
                resetActivityState();
                resetCallState();
                resetDirectState();
                resetPresenceState();
                m_presenceSource->stop();
                setCoreErrorKey(errorKey);
            });
    // Settings edits only reach the core while it is running.
    connect(m_settings, &HarborSettings::settingChanged, this,
            [this](const QString &key, const QJsonValue &value) {
                if (!m_coreReady)
                    return;
                sendRequest(QStringLiteral("settings.update"),
                            QJsonObject{{key, value}},
                            [this](const QJsonObject &payload) {
                                m_settings->applyDocument(payload);
                            });
            });

    setCoreErrorKey(QStringLiteral("error.core.handshakeUnavailable"));
}

void HarborFacade::pairHostCreate()
{
    requestPairing(QStringLiteral("pairing.create"), QJsonObject{});
}

void HarborFacade::pairEnterCode()
{
    requestPairing(QStringLiteral("pairing.enter_code"), QJsonObject{});
}

void HarborFacade::pairSubmit(const QString &code)
{
    requestPairing(QStringLiteral("pairing.submit"),
                   QJsonObject{{QStringLiteral("code"), code}});
}

void HarborFacade::pairPollIncoming()
{
    requestPairing(QStringLiteral("pairing.incoming"), QJsonObject{});
}

void HarborFacade::pairPollStatus()
{
    requestPairing(QStringLiteral("pairing.status"), QJsonObject{});
}

void HarborFacade::pairAccept()
{
    requestPairing(QStringLiteral("pairing.accept"), QJsonObject{});
}

void HarborFacade::pairDecline()
{
    requestPairing(QStringLiteral("pairing.decline"), QJsonObject{});
}

void HarborFacade::pairCancel()
{
    requestPairing(QStringLiteral("pairing.cancel"), QJsonObject{});
}

void HarborFacade::pairReset()
{
    requestPairing(QStringLiteral("pairing.reset"), QJsonObject{});
}

QString HarborFacade::callState() const
{
    return m_callState;
}

QString HarborFacade::callId() const
{
    return m_callId;
}

bool HarborFacade::callMuted() const
{
    return m_callMuted;
}

QString HarborFacade::callEndReason() const
{
    return m_callEndReason;
}

qreal HarborFacade::callRttMs() const
{
    return m_callRttMs;
}

qreal HarborFacade::callLossPct() const
{
    return m_callLossPct;
}

QString HarborFacade::callQuality() const
{
    return m_callQuality;
}

QString HarborFacade::screenShareState() const
{
    return m_screenShareState;
}

QVariantList HarborFacade::chatMessages() const
{
    return m_chatMessages;
}

QVariantList HarborFacade::transfers() const
{
    return m_transfers;
}

QVariantMap HarborFacade::partnerProfile() const
{
    return m_partnerProfile;
}

QVariantList HarborFacade::audioInputs() const
{
    return m_audioInputs;
}

QVariantList HarborFacade::audioOutputs() const
{
    return m_audioOutputs;
}

QString HarborFacade::audioInputDevice() const
{
    return m_audioInputDevice;
}

QString HarborFacade::audioOutputDevice() const
{
    return m_audioOutputDevice;
}

qreal HarborFacade::inputVolume() const
{
    return m_inputVolume;
}

qreal HarborFacade::outputVolume() const
{
    return m_outputVolume;
}

qreal HarborFacade::voiceLevel() const
{
    return m_voiceLevel;
}

qreal HarborFacade::remoteVoiceLevel() const
{
    return m_remoteVoiceLevel;
}

bool HarborFacade::speaking() const
{
    return m_speaking;
}

bool HarborFacade::remoteSpeaking() const
{
    return m_remoteSpeaking;
}

bool HarborFacade::pttActive() const
{
    return m_pttActive;
}

QVariantList HarborFacade::remoteActivity() const
{
    return m_remoteActivity;
}

QVariantMap HarborFacade::mobileState() const
{
    return m_mobileState;
}

void HarborFacade::refreshDirectState()
{
    if (!m_coreReady)
        return;
    sendRequest(QStringLiteral("direct.state"), QJsonObject{},
                [this](const QJsonObject &payload) { applyDirectState(payload); });
}

bool HarborFacade::micTestActive() const
{
    return m_micTestActive;
}

double HarborFacade::micTestLevel() const
{
    return m_micTestLevel;
}

int HarborFacade::micTestSecondsLeft() const
{
    return m_micTestSecondsLeft;
}

double HarborFacade::micTestPeak() const
{
    return m_micTestPeak;
}

QString HarborFacade::micTestError() const
{
    return m_micTestError;
}

void HarborFacade::applyMicTestState(const QJsonObject &payload)
{
    const bool active = payload.value(QStringLiteral("active")).toBool(false);
    const double level = payload.value(QStringLiteral("level")).toDouble(0.0);
    const int secondsLeft = payload.value(QStringLiteral("seconds_left")).toInt(0);
    const double peak = payload.value(QStringLiteral("peak")).toDouble(0.0);
    if (m_micTestActive == active && qFuzzyCompare(m_micTestLevel + 1, level + 1)
        && m_micTestSecondsLeft == secondsLeft && qFuzzyCompare(m_micTestPeak + 1, peak + 1)
        && m_micTestError.isEmpty())
        return;
    m_micTestActive = active;
    m_micTestLevel = level;
    m_micTestSecondsLeft = secondsLeft;
    m_micTestPeak = peak;
    m_micTestError.clear();
    emit micTestChanged();
}

void HarborFacade::applyMicTestError(const QString &uiKey)
{
    if (m_micTestError == uiKey && !m_micTestActive)
        return;
    m_micTestActive = false;
    m_micTestLevel = 0.0;
    m_micTestSecondsLeft = 0;
    m_micTestError = uiKey;
    emit micTestChanged();
}

void HarborFacade::startMicTest(int seconds)
{
    if (!m_coreReady)
        return;
    m_micTestError.clear();
    sendRequest(QStringLiteral("audio.loopback_start"), QJsonObject{{QStringLiteral("seconds"), seconds}},
                [this](const QJsonObject &payload) { applyMicTestState(payload); });
}

void HarborFacade::pollMicTest()
{
    if (!m_coreReady)
        return;
    sendRequest(QStringLiteral("audio.loopback_poll"), QJsonObject{},
                [this](const QJsonObject &payload) { applyMicTestState(payload); });
}

void HarborFacade::stopMicTest()
{
    if (!m_coreReady)
        return;
    sendRequest(QStringLiteral("audio.loopback_stop"), QJsonObject{},
                [this](const QJsonObject &payload) { applyMicTestState(payload); });
}

void HarborFacade::sendChatMessage(const QString &body)
{
    sendRequest(QStringLiteral("chat.send"), QJsonObject{{QStringLiteral("body"), body}},
                [this](const QJsonObject &payload) {
                    applyDirectState(payload.value(QStringLiteral("state")).toObject());
                });
}

void HarborFacade::offerLocalFile(const QString &sourcePath)
{
    // The offer reply carries {transfer_id, state}; only the nested direct
    // snapshot is authoritative UI state.
    sendRequest(QStringLiteral("transfer.offer_local"),
                QJsonObject{{QStringLiteral("source_path"), sourcePath}},
                [this](const QJsonObject &payload) {
                    applyDirectState(payload.value(QStringLiteral("state")).toObject());
                });
}

void HarborFacade::acceptTransfer(const QString &transferId)
{
    transferDecision(QStringLiteral("transfer.accept"), transferId);
}

void HarborFacade::rejectTransfer(const QString &transferId)
{
    transferDecision(QStringLiteral("transfer.reject"), transferId);
}

void HarborFacade::cancelTransfer(const QString &transferId)
{
    transferDecision(QStringLiteral("transfer.cancel"), transferId);
}

/// Every transfer decision reply carries the refreshed direct snapshot;
/// one mapping keeps reply and event payloads identical.
void HarborFacade::transferDecision(const QString &requestType, const QString &transferId)
{
    sendRequest(requestType, QJsonObject{{QStringLiteral("transfer_id"), transferId}},
                [this](const QJsonObject &payload) { applyDirectState(payload); });
}

void HarborFacade::refreshAudioDevices()
{
    if (!m_coreReady)
        return;
    sendRequest(QStringLiteral("audio.devices"), QJsonObject{},
                [this](const QJsonObject &payload) { applyAudioDevices(payload); });
}

void HarborFacade::setAudioVolumes(qreal inputVolume, qreal outputVolume)
{
    // Persisted through the settings mirror; the live call learns it here.
    m_settings->setMicrophoneVolume(inputVolume);
    m_settings->setOutputVolume(outputVolume);
    if (!m_coreReady)
        return;
    sendRequest(QStringLiteral("audio.config"),
                QJsonObject{{QStringLiteral("set"), true},
                            {QStringLiteral("input_volume"), inputVolume},
                            {QStringLiteral("output_volume"), outputVolume}},
                [this](const QJsonObject &payload) { applyAudioConfig(payload); });
}

void HarborFacade::selectAudioDevices(const QString &inputId, const QString &outputId)
{
    m_settings->setInputDevice(inputId);
    m_settings->setOutputDevice(outputId);
    if (!m_coreReady)
        return;
    sendRequest(QStringLiteral("audio.config"),
                QJsonObject{{QStringLiteral("set"), true},
                            {QStringLiteral("input_device"), inputId},
                            {QStringLiteral("output_device"), outputId}},
                [this](const QJsonObject &payload) { applyAudioConfig(payload); });
}

void HarborFacade::setPushToTalkEnabled(bool enabled)
{
    m_settings->setPushToTalkEnabled(enabled);
    if (!m_coreReady)
        return;
    sendRequest(QStringLiteral("call.push_to_talk"),
                QJsonObject{{QStringLiteral("enabled"), enabled}},
                [this](const QJsonObject &payload) { applyPushToTalk(payload); });
}

void HarborFacade::setPushToTalkActive(bool active)
{
    if (!m_coreReady)
        return;
    sendRequest(QStringLiteral("call.push_to_talk"),
                QJsonObject{{QStringLiteral("active"), active}},
                [this](const QJsonObject &payload) { applyPushToTalk(payload); });
}

void HarborFacade::startCall()
{
    // The reply and the following call.state_changed event share one mapping;
    // both carry sanitized state only (no SDP, no candidate material).
    sendRequest(QStringLiteral("call.start"), QJsonObject{},
                [this](const QJsonObject &payload) { applyCallState(payload); });
}

void HarborFacade::acceptIncomingCall()
{
    sendRequest(QStringLiteral("call.accept"), QJsonObject{},
                [this](const QJsonObject &payload) { applyCallState(payload); });
}

void HarborFacade::declineIncomingCall()
{
    sendRequest(QStringLiteral("call.decline"), QJsonObject{},
                [this](const QJsonObject &payload) { applyCallState(payload); });
}

void HarborFacade::endCall()
{
    sendRequest(QStringLiteral("call.end"), QJsonObject{},
                [this](const QJsonObject &payload) { applyCallState(payload); });
}

void HarborFacade::setCallMuted(bool muted)
{
    sendRequest(QStringLiteral("call.mute"),
                QJsonObject{{QStringLiteral("muted"), muted}},
                [this](const QJsonObject &payload) {
                    if (!payload.contains(QStringLiteral("muted")))
                        return;
                    m_callMuted = payload.value(QStringLiteral("muted")).toBool();
                    emit callChanged();
                });
}

void HarborFacade::startScreenShare()
{
    // The worker acknowledges only after its capture adapter really acquired
    // the screen; refusals surface through requestFailed with a localized key.
    sendRequest(QStringLiteral("call.share_screen_start"), QJsonObject{},
                [this](const QJsonObject &payload) { applyShareState(payload); });
}

void HarborFacade::stopScreenShare()
{
    sendRequest(QStringLiteral("call.share_screen_stop"), QJsonObject{},
                [this](const QJsonObject &payload) { applyShareState(payload); });
}

void HarborFacade::applyCallState(const QJsonObject &payload)
{
    const QString state = payload.value(QStringLiteral("state")).toString();
    if (state.isEmpty())
        return;
    if (state == QStringLiteral("ENDED")) {
        const QString reason = payload.value(QStringLiteral("reason")).toString();
        if (reason.isEmpty()) {
            // A bare ENDED reply is the local teardown; the core resets to
            // IDLE and reports it as an event.
            resetCallState();
            return;
        }
        // ENDED with a reason is how a peer's refusal lands ("declined",
        // "busy") or how a recovery window expires: a resting terminal fact
        // the user is meant to see, not a silent return to idle.
        bool changed = m_callState != state || !m_callId.isEmpty() || m_callMuted
                       || m_callEndReason != reason
                       || m_screenShareState != QStringLiteral("NOT_SHARING");
        m_callState = state;
        m_callId.clear();
        m_callMuted = false;
        m_callEndReason = reason;
        resetCallStats();
        if (m_screenShareState != QStringLiteral("NOT_SHARING")) {
            m_screenShareState = QStringLiteral("NOT_SHARING");
            emit screenShareChanged();
        }
        if (changed)
            emit callChanged();
        return;
    }

    bool changed = false;
    if (m_callState != state) {
        m_callState = state;
        changed = true;
    }
    const QString callId = payload.value(QStringLiteral("call_id")).toString();
    if (m_callId != callId) {
        m_callId = callId;
        changed = true;
    }
    if (payload.contains(QStringLiteral("muted"))) {
        const bool muted = payload.value(QStringLiteral("muted")).toBool();
        if (m_callMuted != muted) {
            m_callMuted = muted;
            changed = true;
        }
    }
    if (payload.contains(QStringLiteral("reason"))) {
        const QString reason = payload.value(QStringLiteral("reason")).toString();
        // Every new attempt and every fresh ring carries an empty reason, so
        // a previous attempt's explanation never leaks into this one.
        if (m_callEndReason != reason) {
            m_callEndReason = reason;
            changed = true;
        }
    }
    if (payload.contains(QStringLiteral("stats"))) {
        const QJsonValue statsValue = payload.value(QStringLiteral("stats"));
        if (statsValue.isObject())
            applyCallStats(statsValue.toObject());
        else
            resetCallStats();
    }
    // Ending or losing a call always tears screen share down with it.
    if (state == QStringLiteral("IDLE") || state == QStringLiteral("FAILED")) {
        if (m_screenShareState != QStringLiteral("NOT_SHARING")) {
            m_screenShareState = QStringLiteral("NOT_SHARING");
            emit screenShareChanged();
        }
    }
    if (changed)
        emit callChanged();
}

void HarborFacade::applyShareState(const QJsonObject &payload)
{
    const QString state = payload.value(QStringLiteral("state")).toString();
    if (state.isEmpty() || state == m_screenShareState)
        return;
    m_screenShareState = state;
    emit screenShareChanged();
}

void HarborFacade::applyCallStats(const QJsonObject &stats)
{
    const qreal rtt = stats.value(QStringLiteral("rtt_ms")).toDouble();
    const qreal loss = stats.value(QStringLiteral("loss_pct")).toDouble();
    const QString quality = stats.value(QStringLiteral("quality")).toString();
    if (quality.isEmpty())
        return;
    // +1 keeps qFuzzyCompare well-defined around zero.
    if (qFuzzyCompare(m_callRttMs + 1.0, rtt + 1.0)
        && qFuzzyCompare(m_callLossPct + 1.0, loss + 1.0)
        && m_callQuality == quality)
        return;
    m_callRttMs = rtt;
    m_callLossPct = loss;
    m_callQuality = quality;
    emit callStatsChanged();
}

void HarborFacade::resetCallStats()
{
    if (qFuzzyCompare(m_callRttMs + 1.0, 1.0)
        && qFuzzyCompare(m_callLossPct + 1.0, 1.0)
        && m_callQuality == QStringLiteral("unknown"))
        return;
    m_callRttMs = 0.0;
    m_callLossPct = 0.0;
    m_callQuality = QStringLiteral("unknown");
    emit callStatsChanged();
}

void HarborFacade::resetCallState()
{
    bool changed = m_callState != QStringLiteral("IDLE") || !m_callId.isEmpty()
                   || m_callMuted || !m_callEndReason.isEmpty();
    m_callState = QStringLiteral("IDLE");
    m_callId.clear();
    m_callMuted = false;
    m_callEndReason.clear();
    resetCallStats();
    if (m_screenShareState != QStringLiteral("NOT_SHARING")) {
        // Ending a call always tears screen share down with it.
        m_screenShareState = QStringLiteral("NOT_SHARING");
        emit screenShareChanged();
    }
    resetVoiceLevels();
    if (changed)
        emit callChanged();
}

void HarborFacade::applyVoiceLevels(const QJsonObject &payload)
{
    const qreal level = payload.value(QStringLiteral("level")).toDouble();
    const qreal remoteLevel = payload.value(QStringLiteral("remote_level")).toDouble();
    const bool speaking = payload.value(QStringLiteral("speaking")).toBool();
    const bool remoteSpeaking = payload.value(QStringLiteral("remote_speaking")).toBool();
    // +1.0 keeps the qFuzzyCompare ratio well-defined around zero.
    if (qFuzzyCompare(m_voiceLevel + 1.0, level + 1.0)
        && qFuzzyCompare(m_remoteVoiceLevel + 1.0, remoteLevel + 1.0)
        && m_speaking == speaking && m_remoteSpeaking == remoteSpeaking)
        return;
    m_voiceLevel = level;
    m_remoteVoiceLevel = remoteLevel;
    m_speaking = speaking;
    m_remoteSpeaking = remoteSpeaking;
    emit voiceChanged();
}

/// Voice facts belong to the call that measured them; they never survive it.
void HarborFacade::resetVoiceLevels()
{
    if (qFuzzyIsNull(m_voiceLevel) && qFuzzyIsNull(m_remoteVoiceLevel)
        && !m_speaking && !m_remoteSpeaking && !m_pttActive)
        return;
    m_voiceLevel = 0.0;
    m_remoteVoiceLevel = 0.0;
    m_speaking = false;
    m_remoteSpeaking = false;
    m_pttActive = false;
    emit voiceChanged();
}

void HarborFacade::applyAudioDevices(const QJsonObject &payload)
{
    const auto mapDevice = [](const QJsonObject &entry) {
        return QVariantMap{
            {QStringLiteral("id"), entry.value(QStringLiteral("id")).toString()},
            {QStringLiteral("name"), entry.value(QStringLiteral("name")).toString()},
            {QStringLiteral("isDefault"), entry.value(QStringLiteral("is_default")).toBool()},
        };
    };
    QVariantList inputs;
    QVariantList outputs;
    const QJsonArray inputArray = payload.value(QStringLiteral("inputs")).toArray();
    const QJsonArray outputArray = payload.value(QStringLiteral("outputs")).toArray();
    inputs.reserve(inputArray.size());
    outputs.reserve(outputArray.size());
    for (const QJsonValue &value : inputArray)
        inputs.append(mapDevice(value.toObject()));
    for (const QJsonValue &value : outputArray)
        outputs.append(mapDevice(value.toObject()));
    if (inputs == m_audioInputs && outputs == m_audioOutputs)
        return;
    m_audioInputs = std::move(inputs);
    m_audioOutputs = std::move(outputs);
    emit audioChanged();
}

void HarborFacade::applyAudioConfig(const QJsonObject &payload)
{
    const QString inputDevice = payload.value(QStringLiteral("input_device")).toString();
    const QString outputDevice = payload.value(QStringLiteral("output_device")).toString();
    const qreal inputVolume = payload.value(QStringLiteral("input_volume")).toDouble();
    const qreal outputVolume = payload.value(QStringLiteral("output_volume")).toDouble();
    const bool changed = m_audioInputDevice != inputDevice
                         || m_audioOutputDevice != outputDevice
                         || !qFuzzyCompare(m_inputVolume + 1.0, inputVolume + 1.0)
                         || !qFuzzyCompare(m_outputVolume + 1.0, outputVolume + 1.0);
    m_audioInputDevice = inputDevice;
    m_audioOutputDevice = outputDevice;
    m_inputVolume = inputVolume;
    m_outputVolume = outputVolume;
    if (changed)
        emit audioChanged();
}

void HarborFacade::applyPushToTalk(const QJsonObject &payload)
{
    const QJsonObject mode = payload.value(QStringLiteral("push_to_talk")).toObject();
    const bool active = mode.value(QStringLiteral("active")).toBool(false);
    if (m_pttActive == active)
        return;
    m_pttActive = active;
    emit voiceChanged();
}

void HarborFacade::applyDirectState(const QJsonObject &payload)
{
    const auto messages = payload.value(QStringLiteral("messages")).toArray().toVariantList();
    const auto transfers = payload.value(QStringLiteral("transfers")).toArray().toVariantList();
    if (m_chatMessages == messages && m_transfers == transfers)
        return;
    m_chatMessages = messages;
    m_transfers = transfers;
    emit directChanged();
}

void HarborFacade::resetDirectState()
{
    if (m_chatMessages.isEmpty() && m_transfers.isEmpty())
        return;
    m_chatMessages.clear();
    m_transfers.clear();
    emit directChanged();
}

/// Maps the core's partner-profile snapshot onto the QML-facing partner
/// profile. Only public fields cross: display name, status message, avatar
/// type and avatar bytes. Anything else in the payload is ignored.
void HarborFacade::applyProfileState(const QJsonObject &payload)
{
    const QJsonObject partner = payload.value(QStringLiteral("partner")).toObject();
    QVariantMap profile;
    profile.insert(QStringLiteral("displayName"),
                   partner.value(QStringLiteral("displayName")).toString());
    profile.insert(QStringLiteral("statusMessage"),
                   partner.value(QStringLiteral("statusMessage")).toString());
    const QString avatarType = partner.value(QStringLiteral("avatarType")).toString();
    profile.insert(QStringLiteral("avatarType"),
                   avatarType == QStringLiteral("gif") ? QStringLiteral("gif") : QStringLiteral("image"));
    profile.insert(QStringLiteral("avatar"),
                   partner.value(QStringLiteral("avatar")).toString());
    if (m_partnerProfile == profile)
        return;
    m_partnerProfile = profile;
    emit profileChanged();
}

void HarborFacade::refreshProfileState()
{
    if (!m_coreReady)
        return;
    sendRequest(QStringLiteral("profile.state"), QJsonObject{},
                [this](const QJsonObject &payload) { applyProfileState(payload); });
}

QJsonObject HarborFacade::presence() const
{
    return m_presence;
}

void HarborFacade::refreshPresence()
{
    if (!m_coreReady)
        return;
    sendRequest(QStringLiteral("presence.state"), QJsonObject{},
                [this](const QJsonObject &payload) { applyPresenceState(payload); });
}

/// The aggregate crosses as-is: {"local": {...}, "partner": {...} | null}
/// with committed states, previous states and revisions — no evidence.
void HarborFacade::applyPresenceState(const QJsonObject &payload)
{
    if (m_presence == payload)
        return;
    m_presence = payload;
    emit presenceChanged();
}

void HarborFacade::sendPresenceSense(const QJsonObject &snapshot)
{
    if (!m_coreReady)
        return;
    sendRequest(QStringLiteral("presence.sense"), snapshot,
                [this](const QJsonObject &) { /* commit confirmation arrives as presence.updated */ });
}

void HarborFacade::resetPresenceState()
{
    if (m_presence.isEmpty())
        return;
    m_presence = {};
    emit presenceChanged();
}

void HarborFacade::refreshPairingState()
{
    if (!m_coreReady)
        return;
    sendRequest(QStringLiteral("pairing.state"), QJsonObject{},
                [this](const QJsonObject &payload) { applyPairingState(payload); });
}

void HarborFacade::configureServer(const QString &address, const QString &fingerprint)
{
    if (!m_coreReady)
        return;
    sendRequest(QStringLiteral("server.configure"),
                QJsonObject{{QStringLiteral("address"), address},
                            {QStringLiteral("fingerprint"), fingerprint}},
                [this](const QJsonObject &payload) { applyServerConfig(payload); });
}

void HarborFacade::refreshServerConfig()
{
    if (!m_coreReady)
        return;
    fetchServerConfig();
}

QString HarborFacade::importProfileAvatar(const QUrl &fileUrl) const
{
    if (!fileUrl.isLocalFile())
        return {};

    const QFileInfo info(fileUrl.toLocalFile());
    constexpr qint64 maxSourceBytes = 8 * 1024 * 1024;
    constexpr qint64 maxPixels = 16 * 1024 * 1024;
    constexpr int maxDimension = 4096;
    constexpr int outputDimension = 512;
    constexpr int maxEncodedBytes = 700 * 1024;
    if (!info.isFile() || info.size() <= 0 || info.size() > maxSourceBytes)
        return {};

    QImageReader reader(info.absoluteFilePath());
    // A valid GIF keeps its animation: the original bytes are stored as-is
    // (never re-encoded to a still) so every surface can play them. Caps stay
    // well under the core's 4 MiB avatar budget after base64 inflation.
    if (reader.format() == "gif") {
        constexpr qint64 maxGifBytes = 2 * 1024 * 1024;
        constexpr int maxGifFrames = 120;
        if (info.size() > maxGifBytes)
            return {};
        const int frames = reader.imageCount();
        if (frames <= 0 || frames > maxGifFrames)
            return {};
        const QImage first = reader.read();
        if (first.isNull() || first.width() <= 0 || first.height() <= 0
            || first.width() > maxDimension || first.height() > maxDimension
            || qint64(first.width()) * qint64(first.height()) > maxPixels)
            return {};
        QFile file(info.absoluteFilePath());
        if (!file.open(QIODevice::ReadOnly))
            return {};
        const QByteArray raw = file.read(maxGifBytes + 1);
        if (raw.isEmpty() || raw.size() > maxGifBytes)
            return {};
        return QStringLiteral("data:image/gif;base64,")
            + QString::fromLatin1(raw.toBase64());
    }
    reader.setAutoTransform(false);
    const QSize sourceSize = reader.size();
    if (sourceSize.isValid()
        && (sourceSize.width() > maxDimension || sourceSize.height() > maxDimension
            || qint64(sourceSize.width()) * qint64(sourceSize.height()) > maxPixels))
        return {};

    const QImage decoded = reader.read();
    if (decoded.isNull() || decoded.width() <= 0 || decoded.height() <= 0
        || qint64(decoded.width()) * qint64(decoded.height()) > maxPixels)
        return {};

    QImage image = decoded.convertToFormat(QImage::Format_RGBA8888);
    if (image.width() > outputDimension || image.height() > outputDimension)
        image = image.scaled(outputDimension, outputDimension, Qt::KeepAspectRatio,
                             Qt::SmoothTransformation);

    QByteArray encoded;
    for (int dimension = outputDimension; dimension >= 128; dimension /= 2) {
        QImage candidate = image;
        if (candidate.width() > dimension || candidate.height() > dimension)
            candidate = candidate.scaled(dimension, dimension, Qt::KeepAspectRatio,
                                         Qt::SmoothTransformation);
        encoded.clear();
        QBuffer buffer(&encoded);
        if (!buffer.open(QIODevice::WriteOnly) || !candidate.save(&buffer, "PNG", 9))
            return {};
        if (encoded.size() <= maxEncodedBytes)
            return QStringLiteral("data:image/png;base64,")
                + QString::fromLatin1(encoded.toBase64());
    }
    return {};
}

QVariantMap HarborFacade::networkDiagnostics() const
{
    return m_networkDiagnostics;
}

bool HarborFacade::networkDiagnosticsRunning() const
{
    return m_networkDiagnosticsRunning;
}

void HarborFacade::runNetworkDiagnostics()
{
    if (!m_coreReady || m_networkDiagnosticsRunning)
        return;
    m_networkDiagnosticsRunning = true;
    emit networkDiagnosticsChanged();
    // The core opens a real pinned connection and signs one probe exchange;
    // the reply carries measured numbers or an honest unreachable result.
    sendRequest(QStringLiteral("network.diagnostics"), QJsonObject{},
                [this](const QJsonObject &payload) {
                    m_networkDiagnosticsRunning = false;
                    applyNetworkDiagnostics(payload);
                });
}

void HarborFacade::applyNetworkDiagnostics(const QJsonObject &payload)
{
    const QJsonObject server = payload.value(QStringLiteral("server")).toObject();
    const QJsonObject direct = payload.value(QStringLiteral("direct")).toObject();
    QVariantMap diagnostics;
    diagnostics.insert(QStringLiteral("serverConfigured"),
                       server.value(QStringLiteral("configured")).toBool());
    diagnostics.insert(QStringLiteral("serverReachable"),
                       server.value(QStringLiteral("reachable")).toBool());
    if (server.contains(QStringLiteral("handshake_ms")))
        diagnostics.insert(QStringLiteral("handshakeMs"),
                           server.value(QStringLiteral("handshake_ms")).toDouble());
    if (server.contains(QStringLiteral("rtt_ms")))
        diagnostics.insert(QStringLiteral("rttMs"),
                           server.value(QStringLiteral("rtt_ms")).toDouble());
    diagnostics.insert(QStringLiteral("directActive"),
                       direct.value(QStringLiteral("active")).toBool());
    if (direct.contains(QStringLiteral("rtt_ms")))
        diagnostics.insert(QStringLiteral("directRttMs"),
                           direct.value(QStringLiteral("rtt_ms")).toDouble());
    if (direct.contains(QStringLiteral("loss_pct")))
        diagnostics.insert(QStringLiteral("directLossPct"),
                           direct.value(QStringLiteral("loss_pct")).toDouble());
    if (direct.contains(QStringLiteral("quality")))
        diagnostics.insert(QStringLiteral("directQuality"),
                           direct.value(QStringLiteral("quality")).toString());
    m_networkDiagnostics = diagnostics;
    emit networkDiagnosticsChanged();
}

void HarborFacade::refreshPairedPeers()
{
    if (m_coreReady)
        fetchPairedPeers();
}

QString HarborFacade::pairingPhase() const
{
    return m_pairingPhase;
}

QString HarborFacade::pairingRole() const
{
    return m_pairingRole;
}

QVariantMap HarborFacade::pairingIncoming() const
{
    return m_pairingIncoming;
}

QString HarborFacade::pairingCode() const
{
    return m_pairingCode;
}

QString HarborFacade::pairingErrorKey() const
{
    return m_pairingErrorKey;
}

bool HarborFacade::serverConfigured() const
{
    return m_serverConfigured;
}

QString HarborFacade::serverAddress() const
{
    return m_serverAddress;
}

QString HarborFacade::serverFingerprint() const
{
    return m_serverFingerprint;
}

QVariantList HarborFacade::pairedPeers() const
{
    return m_pairedPeers;
}

bool HarborFacade::pairedPeersResolved() const
{
    return m_pairedPeersResolved;
}

QVariantList HarborFacade::activityTimeline() const
{
    return m_activityTimeline;
}

QVariantMap HarborFacade::activityStats() const
{
    return m_activityStats;
}

QString HarborFacade::activityMonitorState() const
{
    return m_activityMonitorState;
}

QString HarborFacade::appIconUrl(const QString &appId, const QString &iconKey) const
{
    if (!m_appIcons)
        return {};
    // Empty keys are the honest "unknown" case: no lookup, no invented icon.
    if (appId.trimmed().isEmpty() && iconKey.trimmed().isEmpty())
        return {};
    return m_appIcons->iconUrl(appId, iconKey);
}

/// Activity is local-only: a single state request repopulates it after a
/// core restart, since the timeline itself lives inside the core process.
void HarborFacade::refreshActivity()
{
    if (!m_coreReady)
        return;
    fetchActivity();
}

void HarborFacade::fetchServerConfig()
{
    sendRequest(QStringLiteral("server.config"), QJsonObject{},
                [this](const QJsonObject &payload) { applyServerConfig(payload); });
}

void HarborFacade::fetchPairedPeers()
{
    sendRequest(QStringLiteral("contacts.list"), QJsonObject{},
                [this](const QJsonObject &payload) { applyPairedPeers(payload); });
}

/// The server's durable relationship snapshot drives pairing gating. Only
/// the public identifiers cross here; an empty list is an honest "not paired".
void HarborFacade::applyPairedPeers(const QJsonObject &payload)
{
    QVariantList peers;
    const QJsonArray entries = payload.value(QStringLiteral("peers")).toArray();
    peers.reserve(entries.size());
    for (const QJsonValue &value : entries) {
        const QJsonObject entry = value.toObject();
        const QString deviceId = entry.value(QStringLiteral("device_id")).toString();
        const QString harborId = entry.value(QStringLiteral("harbor_id")).toString();
        if (deviceId.isEmpty() || harborId.isEmpty())
            continue;
        peers.append(QVariantMap{
            {QStringLiteral("deviceId"), deviceId},
            {QStringLiteral("harborId"), harborId},
        });
    }
    const bool peersChanged = m_pairedPeers != peers;
    m_pairedPeers = peers;
    // Publish readiness first so listeners that only observe pairedPeersChanged
    // never consume the provisional empty snapshot.
    if (!m_pairedPeersResolved) {
        m_pairedPeersResolved = true;
        emit pairedPeersResolvedChanged();
    }
    if (peersChanged)
        emit pairedPeersChanged();
}

void HarborFacade::applyServerConfig(const QJsonObject &payload)
{
    const bool configured = payload.value(QStringLiteral("configured")).toBool(false);
    const QString address = payload.value(QStringLiteral("address")).toString();
    const QString fingerprint = payload.value(QStringLiteral("fingerprint")).toString();
    if (m_serverConfigured == configured && m_serverAddress == address
        && m_serverFingerprint == fingerprint)
        return;
    m_serverConfigured = configured;
    m_serverAddress = address;
    m_serverFingerprint = fingerprint;
    emit serverChanged();
    // The paired relationship lives behind the configured server: a fresh
    // configuration means the previous snapshot can no longer be trusted.
    fetchPairedPeers();
}

void HarborFacade::fetchActivity()
{
    sendRequest(QStringLiteral("activity.state"), QJsonObject{},
                [this](const QJsonObject &payload) { applyActivity(payload); });
}

/// The core's snapshot {monitor, timeline, stats, remote} is mapped once
/// here; entry fields stay keyed (title_key/title_params/…) and localize in
/// QML. Remote records carry labels the peer already sanitized, plus the
/// theme-safe app identity/icon keys for real program icons.
void HarborFacade::applyActivity(const QJsonObject &payload)
{
    QVariantList timeline;
    const QJsonArray entries = payload.value(QStringLiteral("timeline")).toArray();
    timeline.reserve(entries.size());
    for (const QJsonValue &value : entries) {
        const QJsonObject entry = value.toObject();
        timeline.append(QVariantMap{
            {QStringLiteral("id"), entry.value(QStringLiteral("id")).toString()},
            {QStringLiteral("category"), entry.value(QStringLiteral("category")).toString()},
            {QStringLiteral("label"), entry.value(QStringLiteral("label")).toString()},
            {QStringLiteral("appId"), entry.value(QStringLiteral("app_id")).toString()},
            {QStringLiteral("iconKey"), entry.value(QStringLiteral("icon")).toString()},
            {QStringLiteral("titleKey"), entry.value(QStringLiteral("title_key")).toString()},
            {QStringLiteral("titleParams"), entry.value(QStringLiteral("title_params")).toObject().toVariantMap()},
            {QStringLiteral("descriptionKey"), entry.value(QStringLiteral("description_key")).toString()},
            {QStringLiteral("descriptionParams"), entry.value(QStringLiteral("description_params")).toObject().toVariantMap()},
            {QStringLiteral("occurredAt"), entry.value(QStringLiteral("occurred_at")).toDouble()},
        });
    }

    QVariantList remote;
    const QJsonArray remoteEntries = payload.value(QStringLiteral("remote")).toArray();
    remote.reserve(remoteEntries.size());
    for (const QJsonValue &value : remoteEntries) {
        const QJsonObject entry = value.toObject();
        remote.append(QVariantMap{
            {QStringLiteral("id"), entry.value(QStringLiteral("id")).toString()},
            {QStringLiteral("sender"), entry.value(QStringLiteral("sender")).toString()},
            {QStringLiteral("category"), entry.value(QStringLiteral("category")).toString()},
            {QStringLiteral("kind"), entry.value(QStringLiteral("kind")).toString()},
            {QStringLiteral("label"), entry.value(QStringLiteral("label")).toString()},
            {QStringLiteral("appId"), entry.value(QStringLiteral("app_id")).toString()},
            {QStringLiteral("iconKey"), entry.value(QStringLiteral("icon")).toString()},
            {QStringLiteral("occurredAt"), entry.value(QStringLiteral("occurred_at")).toDouble()},
        });
    }

    const QJsonObject stats = payload.value(QStringLiteral("stats")).toObject();
    QVariantMap statsMap{
        {QStringLiteral("games"), stats.value(QStringLiteral("games")).toInt()},
        {QStringLiteral("apps"), stats.value(QStringLiteral("apps")).toInt()},
        {QStringLiteral("hours"), stats.value(QStringLiteral("hours")).toDouble()},
    };

    const QString monitor = payload.value(QStringLiteral("monitor")).toString();
    const bool changed = timeline != m_activityTimeline || statsMap != m_activityStats
                         || monitor != m_activityMonitorState || remote != m_remoteActivity;
    m_activityTimeline = std::move(timeline);
    m_activityStats = std::move(statsMap);
    m_remoteActivity = std::move(remote);
    if (!monitor.isEmpty())
        m_activityMonitorState = monitor;
    if (changed)
        emit activityChanged();
}

/// The activity engine lives inside the core process; a restart empties the
/// timeline until the new engine scans. That gap is shown, never papered over.
void HarborFacade::resetActivityState()
{
    m_activityTimeline.clear();
    m_activityStats.clear();
    m_remoteActivity.clear();
    m_activityMonitorState = QStringLiteral("idle");
    emit activityChanged();
}

/// A fresh core process owns no phone facts; drop anything from a dead one
/// instead of showing a stale battery or location.
void HarborFacade::resetMobileState()
{
    if (m_mobileState.isEmpty())
        return;
    m_mobileState.clear();
    emit mobileChanged();
}

void HarborFacade::refreshMobile()
{
    if (!m_coreReady)
        return;
    fetchMobile();
}

void HarborFacade::fetchMobile()
{
    sendRequest(QStringLiteral("mobile.state"), QJsonObject{},
                [this](const QJsonObject &payload) { applyMobile(payload); });
}

namespace {

/// One side of the mobile snapshot, reduced to typed UI facts. Unknown or
/// mistyped members fall back to honest empties; the core already validated
/// ranges, this only defends the boundary types.
QVariantMap sanitizePhoneStatus(const QJsonObject &status)
{
    QVariantMap out;
    const int battery = status.value(QStringLiteral("batteryPercent")).toInt(-1);
    out.insert(QStringLiteral("batteryPercent"),
               (battery >= 0 && battery <= 100) ? QVariant(battery) : QVariant());
    out.insert(QStringLiteral("charging"),
               status.value(QStringLiteral("charging")).toBool(false));
    const QString activity =
        status.value(QStringLiteral("phoneActivity")).toString(QStringLiteral("OFFLINE")).toUpper();
    out.insert(QStringLiteral("phoneActivity"),
               (activity == QStringLiteral("ACTIVE") || activity == QStringLiteral("IDLE"))
                   ? activity
                   : QStringLiteral("OFFLINE"));
    out.insert(QStringLiteral("lastActiveAt"),
               status.value(QStringLiteral("lastActiveAt")).toDouble(0));
    out.insert(QStringLiteral("currentApp"),
               status.value(QStringLiteral("currentApp")).toString());
    out.insert(QStringLiteral("locationSharingEnabled"),
               status.value(QStringLiteral("locationSharingEnabled")).toBool(false));
    QVariantMap fix;
    const QJsonObject location = status.value(QStringLiteral("location")).toObject();
    const double latitude = location.value(QStringLiteral("latitude")).toDouble(qQNaN());
    const double longitude = location.value(QStringLiteral("longitude")).toDouble(qQNaN());
    const double accuracy = location.value(QStringLiteral("accuracyMeters")).toDouble(-1);
    if (std::isfinite(latitude) && std::isfinite(longitude) && accuracy >= 0
        && qAbs(latitude) <= 90 && qAbs(longitude) <= 180) {
        fix.insert(QStringLiteral("latitude"), latitude);
        fix.insert(QStringLiteral("longitude"), longitude);
        fix.insert(QStringLiteral("accuracyMeters"), accuracy);
        fix.insert(QStringLiteral("updatedAt"),
                   location.value(QStringLiteral("updatedAt")).toDouble(0));
    }
    out.insert(QStringLiteral("location"), fix.isEmpty() ? QVariant() : QVariant(fix));
    out.insert(QStringLiteral("notificationSharingEnabled"),
               status.value(QStringLiteral("notificationSharingEnabled")).toBool(false));
    out.insert(QStringLiteral("deviceType"),
               status.value(QStringLiteral("deviceType")).toString());
    return out;
}

} // namespace

/// Both phone aggregates arrive together ({own, peer}, either null until
/// that side shares). A side that stops sharing returns to null rather than
/// lingering with stale facts.
void HarborFacade::applyMobile(const QJsonObject &payload)
{
    QVariantMap next;
    const QJsonObject own = payload.value(QStringLiteral("own")).toObject();
    const QJsonObject peer = payload.value(QStringLiteral("peer")).toObject();
    next.insert(QStringLiteral("own"), own.isEmpty() ? QVariant() : QVariant(sanitizePhoneStatus(own)));
    next.insert(QStringLiteral("peer"), peer.isEmpty() ? QVariant() : QVariant(sanitizePhoneStatus(peer)));
    if (next == m_mobileState)
        return;
    m_mobileState = std::move(next);
    emit mobileChanged();
}

/// One mapping for every pairing reply: the core's response payload always
/// carries the authoritative session snapshot fields it changed.
void HarborFacade::applyPairingState(const QJsonObject &payload)
{
    const QString phase = payload.value(QStringLiteral("phase")).toString();
    const QString role = payload.value(QStringLiteral("role")).toString(m_pairingRole);
    const QString code = payload.value(QStringLiteral("code")).toString(m_pairingCode);
    const QString errorKey = payload.value(QStringLiteral("error_key")).toString();

    QVariantMap incoming;
    if (payload.contains(QStringLiteral("request"))) {
        const QJsonObject request = payload.value(QStringLiteral("request")).toObject();
        incoming.insert(QStringLiteral("name"),
                        request.value(QStringLiteral("requester")).toString());
        incoming.insert(QStringLiteral("pairingId"),
                        request.value(QStringLiteral("pairing_id")).toString());
    }

    const bool changed = phase != m_pairingPhase || role != m_pairingRole
                         || code != m_pairingCode || errorKey != m_pairingErrorKey
                         || incoming != m_pairingIncoming;
    m_pairingPhase = phase;
    m_pairingRole = role;
    m_pairingCode = code;
    m_pairingErrorKey = errorKey;
    m_pairingIncoming = incoming;
    if (changed)
        emit pairingChanged();
    if (phase == QStringLiteral("ACCEPTED")) {
        // A fresh pairing relationship is durable on the server; refresh the
        // gated snapshot immediately so the UI unlocks without a restart.
        fetchPairedPeers();
        emit pairingCompleted();
    }
}

void HarborFacade::requestPairing(const QString &requestType, const QJsonObject &payload)
{
    if (!m_coreReady)
        return;
    sendRequest(requestType, payload,
                [this](const QJsonObject &reply) { applyPairingState(reply); });
}

/// The pairing session lives inside the core process; when that process is
/// gone, its in-flight state is gone with it.
void HarborFacade::resetPairingState()
{
    m_pairingPhase = QStringLiteral("IDLE");
    m_pairingRole.clear();
    m_pairingCode.clear();
    m_pairingErrorKey.clear();
    m_pairingIncoming.clear();
    emit pairingChanged();
}

QString HarborFacade::coreState() const
{
    return m_coreState;
}

bool HarborFacade::coreReady() const
{
    return m_coreReady;
}

QString HarborFacade::coreErrorKey() const
{
    return m_coreErrorKey;
}

bool HarborFacade::identityAvailable() const
{
    return m_identityAvailable;
}

QString HarborFacade::identityDeviceId() const
{
    return m_deviceId;
}

QString HarborFacade::identityHarborId() const
{
    return m_harborId;
}

QString HarborFacade::deviceName() const
{
    return QSysInfo::machineHostName();
}

QString HarborFacade::identityPublicKey() const
{
    return m_publicKey;
}

HarborSettings *HarborFacade::settings() const
{
    return m_settings;
}

void HarborFacade::refreshIdentity()
{
    if (m_coreReady)
        fetchIdentity();
}

void HarborFacade::retryCore()
{
    setCoreErrorKey({});
    m_supervisor->start();
}

void HarborFacade::shutdownCore()
{
    m_presenceSource->stop();
    m_supervisor->stop();
}

void HarborFacade::copyToClipboard(const QString &text)
{
    QGuiApplication::clipboard()->setText(text);
}

void HarborFacade::setCoreState(const QString &state)
{
    if (m_coreState == state)
        return;

    m_coreState = state;
    emit coreStateChanged();
}

void HarborFacade::setCoreReady(bool ready)
{
    if (m_coreReady == ready)
        return;

    m_coreReady = ready;
    emit coreReadyChanged();
}

void HarborFacade::setCoreErrorKey(const QString &errorKey)
{
    if (m_coreErrorKey == errorKey)
        return;

    m_coreErrorKey = errorKey;
    emit coreErrorKeyChanged();
}

void HarborFacade::setIdentityAvailable(bool available)
{
    if (m_identityAvailable == available)
        return;

    m_identityAvailable = available;
    emit identityAvailableChanged();
}

void HarborFacade::sendHello()
{
    // The facade announces only capabilities the core foundation actually
    // serves today; presence/session are added with their phases.
    const QJsonObject envelope{
        {QStringLiteral("v"), 1},
        {QStringLiteral("type"), QStringLiteral("core.hello")},
        {QStringLiteral("request_id"), QUuid::createUuid().toString(QUuid::WithoutBraces)},
        {QStringLiteral("timestamp"), QDateTime::currentDateTimeUtc().toString(Qt::ISODateWithMs)},
        {QStringLiteral("payload"), QJsonObject{
            {QStringLiteral("client"), QStringLiteral("harbor-ui")},
            {QStringLiteral("protocol_min"), 1},
            {QStringLiteral("protocol_max"), 1},
            {QStringLiteral("capabilities"), QJsonArray{
                QStringLiteral("identity"),
                QStringLiteral("settings"),
                QStringLiteral("server"),
                QStringLiteral("pairing"),
                QStringLiteral("activity"),
                QStringLiteral("call-bootstrap"),
                QStringLiteral("direct-chat"),
                QStringLiteral("direct-transfer"),
            }},
        }},
    };

    if (!m_supervisor->sendEnvelope(envelope))
        setCoreErrorKey(QStringLiteral("error.core.handshakeUnavailable"));
}

void HarborFacade::sendRequest(const QString &requestType, const QJsonObject &payload,
                               std::function<void(const QJsonObject &)> onPayload)
{
    const QString requestId = QUuid::createUuid().toString(QUuid::WithoutBraces);
    const QJsonObject envelope{
        {QStringLiteral("v"), 1},
        {QStringLiteral("type"), requestType},
        {QStringLiteral("request_id"), requestId},
        {QStringLiteral("timestamp"), QDateTime::currentDateTimeUtc().toString(Qt::ISODateWithMs)},
        {QStringLiteral("payload"), payload},
    };

    m_pending.insert(requestId, std::move(onPayload));
    if (!m_supervisor->sendEnvelope(envelope)) {
        m_pending.remove(requestId);
        emit requestFailed(requestType, QStringLiteral("error.core.unavailable"));
    }
}

void HarborFacade::handleEnvelope(const QJsonObject &envelope)
{
    const QString type = envelope.value(QStringLiteral("type")).toString();

    // Correlated replies are consumed by their pending request; anything else
    // (including the handshake reply to core.hello) falls through to the
    // type-based handling below.
    const QString replyTo = envelope.value(QStringLiteral("reply_to")).toString();
    if (!replyTo.isEmpty() && m_pending.contains(replyTo)) {
        const auto handler = m_pending.take(replyTo);
        if (envelope.contains(QStringLiteral("error"))) {
            const QJsonObject error = envelope.value(QStringLiteral("error")).toObject();
            const QString uiKey = error.value(QStringLiteral("ui_key")).toString();
            // A refused pairing request leaves the core session in ERROR; the
            // facade mirrors that locally instead of waiting for a refresh.
            if (type.startsWith(QStringLiteral("pairing."))) {
                m_pairingPhase = QStringLiteral("ERROR");
                m_pairingErrorKey = uiKey;
                emit pairingChanged();
            }
            // A refused mic-test request lands on the test card itself,
            // where the countdown would otherwise spin without facts.
            if (type.startsWith(QStringLiteral("audio.loopback"))) {
                applyMicTestError(uiKey.isEmpty() ? QStringLiteral("error.core.unavailable") : uiKey);
            }
            if (type == QStringLiteral("contacts.list")) {
                // An unavailable or unauthorized contacts snapshot is still a
                // terminal answer for this bootstrap attempt. Open onboarding
                // with an actionable error instead of deadlocking first run.
                if (m_pairedPeersResolved) {
                    m_pairedPeersResolved = false;
                    emit pairedPeersResolvedChanged();
                }
                m_pairedPeers.clear();
                m_pairedPeersResolved = true;
                emit pairedPeersChanged();
                emit pairedPeersResolvedChanged();
                setCoreErrorKey(uiKey.isEmpty() ? QStringLiteral("error.core.unavailable") : uiKey);
            }
            emit requestFailed(type, uiKey);
            return;
        }
        handler(envelope.value(QStringLiteral("payload")).toObject());
        return;
    }

    if (type == QStringLiteral("core.ready")) {
        handleCoreReady(envelope.value(QStringLiteral("payload")).toObject());
        return;
    }

    // The monitor pushes snapshots as events between requests; one shared
    // mapping keeps event and reply payloads identical.
    if (type == QStringLiteral("activity.updated")) {
        applyActivity(envelope.value(QStringLiteral("payload")).toObject());
        return;
    }

    // Call lifecycle events are push-only facts from the core: the initial
    // bootstrap reply, asynchronous worker connection states, and teardowns.
    if (type == QStringLiteral("call.state_changed")) {
        applyCallState(envelope.value(QStringLiteral("payload")).toObject());
        return;
    }
    if (type == QStringLiteral("call.share_state_changed")) {
        applyShareState(envelope.value(QStringLiteral("payload")).toObject());
        return;
    }
    if (type == QStringLiteral("direct.updated")) {
        applyDirectState(envelope.value(QStringLiteral("payload")).toObject());
        return;
    }
    if (type == QStringLiteral("profile.updated")) {
        applyProfileState(envelope.value(QStringLiteral("payload")).toObject());
        return;
    }
    // Committed presence transitions (either side) arrive push-only; QML
    // never polls. The payload is already the sanitized aggregate.
    if (type == QStringLiteral("presence.updated")) {
        applyPresenceState(envelope.value(QStringLiteral("payload")).toObject());
        return;
    }
    // Voice facts measured on the call's own captured audio, at the worker's
    // bounded cadence (~10 Hz); the mapping coalesces to property changes.
    if (type == QStringLiteral("voice.level")) {
        applyVoiceLevels(envelope.value(QStringLiteral("payload")).toObject());
        return;
    }
    // Transport facts the worker measured on its own connection, at the
    // core's bounded cadence; the mapping coalesces to property changes.
    if (type == QStringLiteral("call.stats_changed")) {
        const QJsonObject payload = envelope.value(QStringLiteral("payload")).toObject();
        const QJsonValue stats = payload.value(QStringLiteral("stats"));
        if (stats.isObject())
            applyCallStats(stats.toObject());
        else
            resetCallStats();
        return;
    }
    if (type == QStringLiteral("phone.notification")) {
        emit phoneNotification(envelope.value(QStringLiteral("payload")).toObject());
        return;
    }
    // Either phone aggregate changed (own or peer, possibly to null); the
    // bridge mirrors the sanitized snapshot into AppState.
    if (type == QStringLiteral("mobile.updated")) {
        applyMobile(envelope.value(QStringLiteral("payload")).toObject());
        return;
    }

    // Unsolicited event types are rejected so the facade never guesses.
}

void HarborFacade::handleCoreReady(const QJsonObject &payload)
{
    if (payload.value(QStringLiteral("protocol")).toInt(-1) != 1
        || payload.value(QStringLiteral("service")).toString() != QStringLiteral("harbor-core")) {
        setCoreReady(false);
        setCoreErrorKey(QStringLiteral("error.core.handshakeInvalid"));
        return;
    }

    setCoreErrorKey({});
    setCoreReady(true);
    // A fresh core process owns no call; drop any state from a dead one.
    resetCallState();
    resetMobileState();
    fetchIdentity();
    fetchSettings();
    fetchServerConfig();
    refreshPairingState();
    fetchPairedPeers();
    fetchActivity();
    fetchMobile();
    refreshDirectState();
    refreshProfileState();
    refreshAudioDevices();
    refreshPresence();
    // Sensing begins only once the core can consume snapshots.
    m_presenceSource->start();
}

void HarborFacade::fetchIdentity()
{
    sendRequest(QStringLiteral("identity.get"), QJsonObject{},
                [this](const QJsonObject &payload) { applyIdentity(payload); });
}

void HarborFacade::fetchSettings()
{
    sendRequest(QStringLiteral("settings.get"), QJsonObject{},
                [this](const QJsonObject &payload) { m_settings->applyDocument(payload); });
}

void HarborFacade::applyIdentity(const QJsonObject &payload)
{
    const QString deviceId = payload.value(QStringLiteral("device_id")).toString();
    const QString harborId = payload.value(QStringLiteral("harbor_id")).toString();
    const QString publicKey = payload.value(QStringLiteral("public_key")).toString();
    if (deviceId.isEmpty() || harborId.isEmpty() || publicKey.isEmpty()) {
        setIdentityAvailable(false);
        return;
    }

    m_deviceId = deviceId;
    m_harborId = harborId;
    m_publicKey = publicKey;
    emit identityChanged();
    setIdentityAvailable(true);
}
