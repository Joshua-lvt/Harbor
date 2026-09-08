#pragma once

#include <QJsonObject>
#include <QNetworkReply>
#include <QNetworkRequest>
#include <QIODevice>
#include <QObject>
#include <QCryptographicHash>
#include <QSaveFile>
#include <QTimer>
#include <functional>
#include <memory>

#include "HarborUpdatePackage.h"

class QNetworkAccessManager;

#ifdef Q_OS_WIN
bool harborWindowsPrivatePath(const QString &path);
#endif

class HarborUpdaterPackageFile
{
public:
    virtual ~HarborUpdaterPackageFile() = default;
    virtual QIODevice *device() = 0;
    virtual bool open() = 0;
    virtual void cancel() = 0;
    virtual bool commit() = 0;
};

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
/// replaced. Applying over the running install dir goes through a detached,
/// copied helper, because Windows cannot overwrite its own running executable.
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
    Q_PROPERTY(QString lastUpdateResult READ lastUpdateResult CONSTANT FINAL)
    Q_PROPERTY(QString lastUpdateVersion READ lastUpdateVersion CONSTANT FINAL)
    Q_PROPERTY(QString lastUpdateError READ lastUpdateError CONSTANT FINAL)

public:
    struct Dependencies {
        std::function<QNetworkReply *(const QNetworkRequest &)> get;
        std::function<bool(const QString &, const QStringList &, qint64)> launch;
        std::function<bool(const QString &, const QStringList &, qint64)> launchElevated;
        std::function<std::unique_ptr<HarborUpdaterPackageFile>(const QString &)> packageFile;
        QString helperPath;
    };

    explicit HarborUpdater(QObject *parent = nullptr, Dependencies dependencies = {});

    QString status() const { return m_status; }
    QString currentVersion() const;
    QString availableVersion() const { return m_availableVersion; }
    qreal progress() const { return m_progress; }
    QString errorKey() const { return m_errorKey; }
    bool updateRequired() const;
    bool waitingForCall() const { return m_waitingForCall; }
    QString lastUpdateResult() const { return m_lastUpdateResult; }
    QString lastUpdateVersion() const { return m_lastUpdateVersion; }
    QString lastUpdateError() const { return m_lastUpdateError; }

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
    void onCheckReadyRead();
    void onDownloadProgress(qint64 received, qint64 total);
    void onDownloadReadyRead();
    void onDownloadFinished();
    void onChecksumReadyRead();
    void onChecksumFinished();
    void onSignedManifestReadyRead();
    void onSignedManifestFinished();
    void onRecheckTimer();

private:
    void setStatus(const QString &status);
    void setError(const QString &errorKey);
    void scheduleRecheck(int msecs);
    QString platformAssetName() const;
    QString downloadDir() const;
    void abortDownload(const QString &errorKey);
    void markReady();
    void tryApplyNow();
    void loadLastUpdateResult();

    QNetworkAccessManager *m_network = nullptr;
    Dependencies m_dependencies;
    QNetworkReply *m_reply = nullptr;
    QNetworkReply *m_checksumReply = nullptr;
    QNetworkReply *m_signedManifestReply = nullptr;
    QTimer m_recheck;
    QString m_status = QStringLiteral("idle");
    QString m_availableVersion;
    QString m_assetUrl;
    QString m_assetShaUrl;
    QString m_packagePath;
    QString m_signedManifestPath;
    HarborUpdatePackage::Manifest m_manifest;
    std::unique_ptr<HarborUpdaterPackageFile> m_packageFile;
    HarborUpdatePackage::BoundedPackageWriter *m_packageWriter = nullptr;
    qint64 m_downloadedBytes = 0;
    QByteArray m_downloadPrefix;
    QByteArray m_checksumBody;
    QByteArray m_signedManifestBody;
    QByteArray m_releaseBody;
    qreal m_progress = 0;
    QString m_errorKey;
    bool m_callActive = false;
    bool m_waitingForCall = false;
    QString m_lastUpdateResult;
    QString m_lastUpdateVersion;
    QString m_lastUpdateError;
    bool m_recoveryRequired = false;
};
