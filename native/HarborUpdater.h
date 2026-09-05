#pragma once

#include <QJsonObject>
#include <QNetworkReply>
#include <QObject>
#include <QTimer>

class QNetworkAccessManager;

/// Mandatory in-app updater for Harbor desktops (Linux, Windows).
///
/// Policy, enforced by the QML shell rather than this adapter:
/// - Updates are mandatory once DISCOVERED (available/ready): the shell
///   shows a blocking surface with no skip path.
/// - A failed check (offline, API error) is NOT discovery: the app keeps
///   running and retries on a slow cadence plus on user request, so a dead
///   network never bricks the app.
/// - A ready update applies automatically once the call is idle, then the
///   app restarts into the new build. An in-progress call defers the
///   restart; it never drops media for an update.
///
/// Source of truth is the GitHub release channel
/// (Joshua-lvt/Harbor/releases/latest). Every downloaded artifact is
/// SHA-256 verified against its sibling .sha256 asset before anything is
/// replaced. Applying over the running install dir goes through a detached
/// self-relaunch (--apply-update), because Windows cannot overwrite its own
/// running executable.
class HarborUpdater final : public QObject
{
    Q_OBJECT

    // idle | checking | available | downloading | ready | applying | error
    Q_PROPERTY(QString status READ status NOTIFY statusChanged FINAL)
    Q_PROPERTY(QString currentVersion READ currentVersion CONSTANT FINAL)
    Q_PROPERTY(QString availableVersion READ availableVersion NOTIFY statusChanged FINAL)
    // 0..1 while downloading.
    Q_PROPERTY(qreal progress READ progress NOTIFY progressChanged FINAL)
    // Stable error key for localized UI ("" when no error).
    Q_PROPERTY(QString errorKey READ errorKey NOTIFY statusChanged FINAL)
    // True while an update is discovered and unpaid-for attention is due:
    // the shell blocks on it. Never true for a mere check failure.
    Q_PROPERTY(bool updateRequired READ updateRequired NOTIFY statusChanged FINAL)
    // True while a call is holding the restart back.
    Q_PROPERTY(bool waitingForCall READ waitingForCall NOTIFY statusChanged FINAL)

public:
    explicit HarborUpdater(QObject *parent = nullptr);

    QString status() const { return m_status; }
    QString currentVersion() const;
    QString availableVersion() const { return m_availableVersion; }
    qreal progress() const { return m_progress; }
    QString errorKey() const { return m_errorKey; }
    bool updateRequired() const;
    bool waitingForCall() const { return m_waitingForCall; }

    /// Compare dotted versions ("2.1.0", leading "v" tolerated).
    /// Returns -1/0/+1. Non-numeric tails compare lower than releases.
    static int compareVersions(const QString &a, const QString &b);
    /// Pick the platform asset from a releases/latest document.
    /// Returns {url, shaUrl} (empty when absent).
    static QJsonObject pickAsset(const QJsonObject &release);

    /// Start one update check (no-op unless idle or error).
    Q_INVOKABLE void checkForUpdates();
    /// Start downloading the discovered update (no-op unless available).
    Q_INVOKABLE void downloadUpdate();
    /// Apply a ready update now if the call is idle, else defer until it is.
    /// The QML shell reports call idleness through setCallActive().
    Q_INVOKABLE void applyUpdate();
    /// The shell mirrors call activity so restarts never drop media.
    Q_INVOKABLE void setCallActive(bool active);
    /// Re-run a failed check (no-op unless in error).
    Q_INVOKABLE void retry();

signals:
    void statusChanged();
    void progressChanged();

private slots:
    void onCheckFinished();
    void onDownloadProgress(qint64 received, qint64 total);
    void onDownloadFinished();
    void onRecheckTimer();

private:
    void setStatus(const QString &status);
    void setError(const QString &errorKey);
    void scheduleRecheck(int msecs);
    QString platformAssetName() const;
    QString downloadDir() const;
    bool verifyChecksum(const QString &filePath, const QString &expectedHex);
    void tryApplyNow();

    QNetworkAccessManager *m_network = nullptr;
    QNetworkReply *m_reply = nullptr;
    QTimer m_recheck;
    QString m_status = QStringLiteral("idle");
    QString m_availableVersion;
    QString m_assetUrl;
    QString m_assetShaUrl;
    QString m_packagePath;
    qreal m_progress = 0;
    QString m_errorKey;
    bool m_callActive = false;
    bool m_waitingForCall = false;
};
