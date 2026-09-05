#pragma once

#include <QByteArray>
#include <QJsonObject>
#include <QMutex>
#include <QSet>
#include <QObject>
#include <QThread>
#include <QTimer>

/// In-process Harbor core for Android: the same Rust state the desktop
/// binary drives over stdio, called through the C ABI (Android cannot
/// spawn the core as a child process). Framing is identical — 4-byte
/// big-endian length plus v1 JSON envelopes — so the QML host speaks one
/// protocol on both platforms.
///
/// Threading: asynchronous user requests, signaling polls, and the link pump
/// all run on one dedicated core-owner thread. The legacy synchronous entry
/// point exists for native compatibility tests only; QML uses sendAsync so
/// control-plane I/O never runs on the UI thread. Events cross threads via
/// queued signals.
class HarborCoreAdapter final : public QObject
{
    Q_OBJECT

public:
    explicit HarborCoreAdapter(const QString &stateDir, QObject *parent = nullptr);
    ~HarborCoreAdapter() override;

    bool alive() const;
    bool setMediaWorkerPath(const QString &path);

    /// Sends one typed request; returns the correlated response payload.
    /// An empty object means the core rejected or never answered: read
    /// `lastError` for the structured code and render it — the caller
    /// renders the honest empty state, never invented data.
    /// Legacy synchronous entry point retained for native tests only. QML
    /// must use sendAsync so network/control-plane work never runs on the UI
    /// thread.
    Q_INVOKABLE QJsonObject send(const QString &type, const QJsonObject &payload);
    Q_INVOKABLE QString sendAsync(const QString &type, const QJsonObject &payload);

    /// Structured code of the most recent rejection (`""` when the last
    /// request succeeded). Never a secret: safe to display.
    Q_PROPERTY(QString lastError READ lastError NOTIFY lastErrorChanged)
    QString lastError() const { return m_lastError; }

signals:
    void lastErrorChanged();
    // Unsolicited core events, payloads verbatim.
    void directUpdated(const QJsonObject &snapshot);
    void profileUpdated(const QJsonObject &snapshot);
    void presenceUpdated(const QJsonObject &sides);
    void deviceUpdated(const QJsonObject &snapshot);
    void mobileUpdated(const QJsonObject &snapshot);
    void callUpdated(const QJsonObject &snapshot);
    void callShareUpdated(const QJsonObject &snapshot);
    void activityUpdated(const QJsonObject &snapshot);
    void phoneNotification(const QJsonObject &notification);
    void requestFinished(const QString &requestId, const QString &type,
                         const QJsonObject &payload, const QString &errorCode);

private slots:
    void tick();

private:
    /// Pump worker: lives on its own thread, calls tick() on cadence.
    class Ticker;
    Ticker *m_ticker = nullptr;
    struct CoreState;
    void handleReplies(const QByteArray &frames);
    void routeEvent(const QString &type, const QJsonObject &payload);
    void finishRequest(const QString &requestId, const QString &type,
                       const QJsonObject &payload, const QString &errorCode);
    bool claimRequest(const QString &requestId);

    // The Rust core is synchronous and single-owner: every entry point
    // takes this guard, whichever thread calls.
    friend class Ticker;
    CoreState *m_core = nullptr;
    QThread m_thread;
    QMutex m_guard;
    QString m_lastError;
    QSet<QString> m_pendingRequestIds;
    void setLastError(const QString &code);
    QJsonObject sendWithId(const QString &type, const QJsonObject &payload,
                           const QString &requestId);
};
