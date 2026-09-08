// In-process core facade: framed requests in, correlated replies plus
// routed events out. Frame splitting mirrors the desktop supervisor's
// decoder (4-byte big-endian length, 1 MiB cap); a truncated tail is
// dropped with a warning, never parsed.
#include "HarborCoreAdapter.h"
#include "harbor_core_ffi.h"

#include <QDateTime>
#include <QDebug>
#include <QDir>
#include <QJsonDocument>
#include <QUuid>

namespace {

QByteArray takeBuffer(uint8_t *data, size_t len)
{
    if (!data || len == 0)
        return {};
    QByteArray out(reinterpret_cast<const char *>(data), static_cast<int>(len));
    harbor_core_free(data, len);
    return out;
}

QByteArray frameEnvelope(const QString &type, const QJsonObject &payload,
                         const QString &requestId)
{
    QJsonObject envelope;
    envelope[QStringLiteral("v")] = 1;
    envelope[QStringLiteral("type")] = type;
    envelope[QStringLiteral("request_id")] = requestId;
    envelope[QStringLiteral("timestamp")] =
        QDateTime::currentDateTimeUtc().toString(Qt::ISODateWithMs);
    envelope[QStringLiteral("payload")] = payload;
    const QByteArray json =
        QJsonDocument(envelope).toJson(QJsonDocument::Compact);
    QByteArray frame;
    const quint32 len = static_cast<quint32>(json.size());
    frame.append(static_cast<char>((len >> 24) & 0xFF));
    frame.append(static_cast<char>((len >> 16) & 0xFF));
    frame.append(static_cast<char>((len >> 8) & 0xFF));
    frame.append(static_cast<char>(len & 0xFF));
    frame.append(json);
    return frame;
}

} // namespace

struct HarborCoreAdapter::CoreState {
    HarborCoreState *handle = nullptr;
};

class HarborCoreAdapter::Ticker final : public QObject
{
    Q_OBJECT
public:
    explicit Ticker(HarborCoreAdapter *owner)
        : m_owner(owner)
    {
        connect(&m_timer, &QTimer::timeout, this, &Ticker::tick);
    }

public slots:
    void start() { m_timer.start(1000); }
    void tick() { m_owner->tick(); }
    void send(const QString &type, const QJsonObject &payload, const QString &requestId)
    {
        const QJsonObject result = m_owner->sendWithId(type, payload, requestId);
        m_owner->finishRequest(requestId, type, result, m_owner->lastError());
    }

private:
    HarborCoreAdapter *m_owner;
    QTimer m_timer;
};

HarborCoreAdapter::HarborCoreAdapter(const QString &stateDir, QObject *parent)
    : QObject(parent)
{
    auto *inner = new CoreState;
    const QByteArray dir = QDir::toNativeSeparators(stateDir).toUtf8();
    inner->handle = harbor_core_create(dir.isEmpty() ? nullptr : dir.constData());
    m_core = inner;
    if (!alive())
        qWarning() << "harbor-core failed to start for" << stateDir;
    // Ticker is deliberately parentless so it can move to the worker
    // thread.  It still needs the adapter back-pointer: a null owner would
    // crash on the first timer tick.
    auto *ticker = new Ticker(this);
    m_ticker = ticker;
    ticker->moveToThread(&m_thread);
    m_thread.start();
    QMetaObject::invokeMethod(ticker, "start", Qt::QueuedConnection);
}

HarborCoreAdapter::~HarborCoreAdapter()
{
    m_thread.quit();
    m_thread.wait();
    delete m_ticker;
    m_ticker = nullptr;
    if (m_core) {
        harbor_core_destroy(m_core->handle);
        delete m_core;
    }
}

bool HarborCoreAdapter::alive() const
{
    return m_core && m_core->handle;
}

bool HarborCoreAdapter::setMediaWorkerPath(const QString &path)
{
    if (!alive() || path.isEmpty())
        return false;
    const QByteArray encoded = path.toUtf8();
    QMutexLocker lock(&m_guard);
    return harbor_core_set_media_worker(m_core->handle, encoded.constData());
}

QString HarborCoreAdapter::sendAsync(const QString &type, const QJsonObject &payload)
{
    const QString requestId = QUuid::createUuid().toString(QUuid::WithoutBraces);
    if (!m_ticker || !alive()) {
        // Do not emit synchronously here. The QML helper records its
        // request-id callback immediately after sendAsync() returns; a
        // direct emission would race that registration and leave, for
        // example, pairingBusy stuck forever when the core is unavailable.
        QMetaObject::invokeMethod(this, [this, requestId, type] {
            emit requestFinished(requestId, type, {},
                                 QStringLiteral("core_unavailable"));
        }, Qt::QueuedConnection);
        return requestId;
    }
    {
        QMutexLocker lock(&m_guard);
        m_pendingRequestIds.insert(requestId);
    }
    QMetaObject::invokeMethod(m_ticker, "send", Qt::QueuedConnection,
                              Q_ARG(QString, type), Q_ARG(QJsonObject, payload),
                              Q_ARG(QString, requestId));
    // Core requests run off the UI thread, but a control-plane operation can
    // still outlive a network outage. Bound the UI-side wait so a spinner or
    // pairing flow never remains pending forever. The worker may finish later;
    // claimRequest makes that late result harmless.
    QTimer::singleShot(10000, this, [this, requestId, type] {
        if (claimRequest(requestId))
            emit requestFinished(requestId, type, {}, QStringLiteral("timeout"));
    });
    return requestId;
}

void HarborCoreAdapter::finishRequest(const QString &requestId, const QString &type,
                                      const QJsonObject &payload,
                                      const QString &errorCode)
{
    if (claimRequest(requestId))
        emit requestFinished(requestId, type, payload, errorCode);
}

bool HarborCoreAdapter::claimRequest(const QString &requestId)
{
    QMutexLocker lock(&m_guard);
    return m_pendingRequestIds.remove(requestId) > 0;
}

void HarborCoreAdapter::setLastError(const QString &code)
{
    if (m_lastError == code)
        return;
    m_lastError = code;
    emit lastErrorChanged();
}

QJsonObject HarborCoreAdapter::send(const QString &type, const QJsonObject &payload)
{
    return sendWithId(type, payload,
                      QUuid::createUuid().toString(QUuid::WithoutBraces));
}

QJsonObject HarborCoreAdapter::sendWithId(const QString &type, const QJsonObject &payload,
                                          const QString &requestId)
{
    QMutexLocker lock(&m_guard);
    if (!alive()) {
        setLastError(QStringLiteral("core_unavailable"));
        return {};
    }
    const QByteArray request = frameEnvelope(type, payload, requestId);
    size_t outLen = 0;
    const QByteArray replies = takeBuffer(
        harbor_core_dispatch(m_core->handle,
                             reinterpret_cast<const uint8_t *>(request.constData()),
                             static_cast<size_t>(request.size()), &outLen),
        outLen);
    if (replies.isEmpty()) {
        setLastError(QStringLiteral("transport_rejected"));
        return {};
    }
    QJsonObject response;
    int offset = 0;
    bool first = true;
    while (offset + 4 <= replies.size()) {
        const quint32 len =
            (static_cast<quint8>(replies[offset]) << 24)
            | (static_cast<quint8>(replies[offset + 1]) << 16)
            | (static_cast<quint8>(replies[offset + 2]) << 8)
            | static_cast<quint8>(replies[offset + 3]);
        if (len == 0 || len > 1024 * 1024 || offset + 4 + static_cast<int>(len) > replies.size())
            break;
        const QJsonDocument doc = QJsonDocument::fromJson(
            QByteArray(replies.constData() + offset + 4, static_cast<int>(len)));
        offset += 4 + static_cast<int>(len);
        if (!doc.isObject())
            continue;
        const QJsonObject envelope = doc.object();
        if (first) {
            first = false;
            const QJsonObject errorObject = envelope.value(QStringLiteral("error")).toObject();
            if (!errorObject.isEmpty()) {
                setLastError(errorObject.value(QStringLiteral("code")).toString());
                return {};
            }
            setLastError(QString());
            response = envelope.value(QStringLiteral("payload")).toObject();
            continue;
        }
        // Follow-up events ride the same router as tick events.
        routeEvent(envelope.value(QStringLiteral("type")).toString(),
                   envelope.value(QStringLiteral("payload")).toObject());
    }
    return response;
}

void HarborCoreAdapter::routeEvent(const QString &type, const QJsonObject &payload)
{
    if (type == QStringLiteral("direct.updated"))
        emit directUpdated(payload);
    else if (type == QStringLiteral("profile.updated"))
        emit profileUpdated(payload);
    else if (type == QStringLiteral("presence.updated"))
        emit presenceUpdated(payload);
    else if (type == QStringLiteral("device.updated"))
        emit deviceUpdated(payload);
    else if (type == QStringLiteral("mobile.updated"))
        emit mobileUpdated(payload);
    else if (type == QStringLiteral("call.state_changed"))
        emit callUpdated(payload);
    else if (type == QStringLiteral("call.share_state_changed"))
        emit callShareUpdated(payload);
    else if (type == QStringLiteral("activity.updated"))
        emit activityUpdated(payload);
    else if (type == QStringLiteral("phone.notification"))
        emit phoneNotification(payload);
}

void HarborCoreAdapter::tick()
{
    QMutexLocker lock(&m_guard);
    if (!alive())
        return;
    size_t outLen = 0;
    const QByteArray frames = takeBuffer(harbor_core_tick(m_core->handle, &outLen), outLen);
    handleReplies(frames);
}

void HarborCoreAdapter::handleReplies(const QByteArray &frames)
{
    int offset = 0;
    while (offset + 4 <= frames.size()) {
        const quint32 len =
            (static_cast<quint8>(frames[offset]) << 24)
            | (static_cast<quint8>(frames[offset + 1]) << 16)
            | (static_cast<quint8>(frames[offset + 2]) << 8)
            | static_cast<quint8>(frames[offset + 3]);
        if (len == 0 || len > 1024 * 1024 || offset + 4 + static_cast<int>(len) > frames.size()) {
            qWarning() << "harbor-core event frame rejected at offset" << offset;
            break;
        }
        const QJsonDocument doc = QJsonDocument::fromJson(
            QByteArray(frames.constData() + offset + 4, static_cast<int>(len)));
        offset += 4 + static_cast<int>(len);
        if (!doc.isObject())
            continue;
        const QJsonObject envelope = doc.object();
        routeEvent(envelope.value(QStringLiteral("type")).toString(),
                   envelope.value(QStringLiteral("payload")).toObject());
    }
}

#include "HarborCoreAdapter.moc"
