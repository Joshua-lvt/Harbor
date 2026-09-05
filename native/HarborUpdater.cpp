// Mandatory in-app updater, desktop side. See HarborUpdater.h for policy.
//
// Release layout (one GitHub release, shared product version):
//   harbor-linux-x86_64.tar.gz   (+ .sha256)   contents: harbor,
//       harbor-core, harbor-media next to each other
//   harbor-windows-x86_64.zip    (+ .sha256)   contents: harbor.exe,
//       harbor-core.exe, harbor-media.exe
// The directory the running application lives in is updated in place, so a
// dev-tree run updates the dev tree and an installed copy updates itself.
#include "HarborUpdater.h"

#include <QCoreApplication>
#include <QCryptographicHash>
#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QNetworkAccessManager>
#include <QNetworkReply>
#include <QNetworkRequest>
#include <QProcess>
#include <QRegularExpression>
#include <QStandardPaths>

namespace {

constexpr char kOwner[] = "Joshua-lvt";
constexpr char kRepo[] = "Harbor";
constexpr int kRecheckIdleHours = 6;

QString currentVersionString()
{
#ifdef HARBOR_VERSION_STRING
    return QStringLiteral(HARBOR_VERSION_STRING);
#else
    return QStringLiteral("0.0.0");
#endif
}

// Split "1.2.3-rc1" into numeric parts plus a pre-release tail.
QList<int> numericParts(const QString &version, QString *tail)
{
    QString clean = version.trimmed();
    if (clean.startsWith(QLatin1Char('v')) || clean.startsWith(QLatin1Char('V')))
        clean = clean.mid(1);
    QString core = clean;
    QString rest;
    const int dash = clean.indexOf(QLatin1Char('-'));
    if (dash >= 0) {
        core = clean.left(dash);
        rest = clean.mid(dash + 1);
    }
    QList<int> parts;
    for (const QString &piece : core.split(QLatin1Char('.'))) {
        bool ok = false;
        const int value = piece.toInt(&ok);
        parts.append(ok ? value : 0);
    }
    if (tail)
        *tail = rest;
    return parts;
}

} // namespace

int HarborUpdater::compareVersions(const QString &a, const QString &b)
{
    QString tailA, tailB;
    const QList<int> partsA = numericParts(a, &tailA);
    const QList<int> partsB = numericParts(b, &tailB);
    const int count = qMax(partsA.size(), partsB.size());
    for (int i = 0; i < count; ++i) {
        const int left = i < partsA.size() ? partsA[i] : 0;
        const int right = i < partsB.size() ? partsB[i] : 0;
        if (left != right)
            return left < right ? -1 : 1;
    }
    // A pre-release tail sorts below the bare release.
    if (tailA == tailB)
        return 0;
    if (tailA.isEmpty())
        return 1;
    if (tailB.isEmpty())
        return -1;
    return tailA < tailB ? -1 : 1;
}

QJsonObject HarborUpdater::pickAsset(const QJsonObject &release)
{
    QJsonObject empty;
#ifdef Q_OS_WIN
    const QString wanted = QStringLiteral("harbor-windows-x86_64.zip");
#else
    const QString wanted = QStringLiteral("harbor-linux-x86_64.tar.gz");
#endif
    const QJsonArray assets = release.value(QStringLiteral("assets")).toArray();
    QString url, shaUrl;
    for (const QJsonValue &entry : assets) {
        const QJsonObject asset = entry.toObject();
        const QString name = asset.value(QStringLiteral("name")).toString();
        const QString download = asset.value(QStringLiteral("browser_download_url")).toString();
        if (name == wanted)
            url = download;
        else if (name == wanted + QStringLiteral(".sha256"))
            shaUrl = download;
    }
    if (url.isEmpty())
        return empty;
    QJsonObject picked;
    picked.insert(QStringLiteral("url"), url);
    picked.insert(QStringLiteral("shaUrl"), shaUrl);
    return picked;
}

HarborUpdater::HarborUpdater(QObject *parent)
    : QObject(parent)
    , m_network(new QNetworkAccessManager(this))
{
    m_recheck.setSingleShot(true);
    connect(&m_recheck, &QTimer::timeout, this, &HarborUpdater::onRecheckTimer);
    scheduleRecheck(kRecheckIdleHours * 3600 * 1000);
}

QString HarborUpdater::currentVersion() const
{
    return currentVersionString();
}

bool HarborUpdater::updateRequired() const
{
    return m_status == QStringLiteral("available") || m_status == QStringLiteral("downloading")
        || m_status == QStringLiteral("ready") || m_status == QStringLiteral("applying");
}

void HarborUpdater::setStatus(const QString &status)
{
    if (m_status == status)
        return;
    m_status = status;
    emit statusChanged();
}

void HarborUpdater::setError(const QString &errorKey)
{
    m_errorKey = errorKey;
    setStatus(QStringLiteral("error"));
    // A failed check is not a discovery: stay usable, retry later.
    scheduleRecheck(kRecheckIdleHours * 3600 * 1000);
}

void HarborUpdater::scheduleRecheck(int msecs)
{
    m_recheck.stop();
    m_recheck.setInterval(msecs);
    m_recheck.start();
}

QString HarborUpdater::downloadDir() const
{
    const QString base = QStandardPaths::writableLocation(QStandardPaths::CacheLocation);
    QDir().mkpath(base + QStringLiteral("/harbor-updates"));
    return base + QStringLiteral("/harbor-updates");
}

void HarborUpdater::checkForUpdates()
{
    if (m_status != QStringLiteral("idle") && m_status != QStringLiteral("error"))
        return;
    m_errorKey.clear();
    m_progress = 0;
    emit progressChanged();
    setStatus(QStringLiteral("checking"));
    QNetworkRequest request(QUrl(QStringLiteral("https://api.github.com/repos/") + QLatin1String(kOwner)
                                 + QLatin1Char('/') + QLatin1String(kRepo)
                                 + QStringLiteral("/releases/latest")));
    request.setRawHeader("Accept", "application/vnd.github+json");
    request.setRawHeader("User-Agent", "Harbor-Updater");
    m_reply = m_network->get(request);
    connect(m_reply, &QNetworkReply::finished, this, &HarborUpdater::onCheckFinished);
}

void HarborUpdater::onCheckFinished()
{
    QNetworkReply *reply = m_reply;
    m_reply = nullptr;
    if (!reply)
        return;
    reply->deleteLater();
    if (reply->error() != QNetworkReply::NoError) {
        setError(QStringLiteral("update.error.network"));
        return;
    }
    const QJsonObject release =
        QJsonDocument::fromJson(reply->readAll()).object();
    if (release.value(QStringLiteral("prerelease")).toBool(false)) {
        // Pre-releases never auto-apply; treat as "nothing new" and look again later.
        setStatus(QStringLiteral("idle"));
        scheduleRecheck(kRecheckIdleHours * 3600 * 1000);
        return;
    }
    const QString tag = release.value(QStringLiteral("tag_name")).toString();
    if (tag.isEmpty() || compareVersions(tag, currentVersionString()) <= 0) {
        setStatus(QStringLiteral("idle"));
        scheduleRecheck(kRecheckIdleHours * 3600 * 1000);
        return;
    }
    const QJsonObject picked = pickAsset(release);
    if (picked.isEmpty()) {
        setError(QStringLiteral("update.error.noArtifact"));
        return;
    }
    m_availableVersion = tag.startsWith(QLatin1Char('v')) ? tag.mid(1) : tag;
    m_assetUrl = picked.value(QStringLiteral("url")).toString();
    m_assetShaUrl = picked.value(QStringLiteral("shaUrl")).toString();
    setStatus(QStringLiteral("available"));
    // Mandatory once discovered: start fetching immediately.
    downloadUpdate();
}

void HarborUpdater::downloadUpdate()
{
    if (m_status != QStringLiteral("available") || m_assetUrl.isEmpty())
        return;
    setStatus(QStringLiteral("downloading"));
    m_progress = 0;
    emit progressChanged();
#ifdef Q_OS_WIN
    const QString fileName = QStringLiteral("harbor-update.zip");
#else
    const QString fileName = QStringLiteral("harbor-update.tar.gz");
#endif
    m_packagePath = downloadDir() + QLatin1Char('/') + fileName;
    QFile::remove(m_packagePath);
    QNetworkRequest request{QUrl(m_assetUrl)};
    request.setRawHeader("User-Agent", "Harbor-Updater");
    // GitHub release assets answer 302 to a same-host object URL; follow it.
    request.setAttribute(QNetworkRequest::RedirectPolicyAttribute,
                         QNetworkRequest::NoLessSafeRedirectPolicy);
    m_reply = m_network->get(request);
    connect(m_reply, &QNetworkReply::downloadProgress, this, &HarborUpdater::onDownloadProgress);
    connect(m_reply, &QNetworkReply::finished, this, &HarborUpdater::onDownloadFinished);
}

void HarborUpdater::onDownloadProgress(qint64 received, qint64 total)
{
    if (total <= 0)
        return;
    m_progress = qBound(0.0, double(received) / double(total), 1.0);
    emit progressChanged();
}

bool HarborUpdater::verifyChecksum(const QString &filePath, const QString &expectedHex)
{
    QFile file(filePath);
    if (!file.open(QIODevice::ReadOnly))
        return false;
    QCryptographicHash hash(QCryptographicHash::Sha256);
    if (!hash.addData(&file))
        return false;
    return hash.result().toHex().compare(expectedHex.trimmed().toLatin1(), Qt::CaseInsensitive) == 0;
}

void HarborUpdater::onDownloadFinished()
{
    QNetworkReply *reply = m_reply;
    m_reply = nullptr;
    if (!reply)
        return;
    reply->deleteLater();
    if (reply->error() != QNetworkReply::NoError) {
        setError(QStringLiteral("update.error.network"));
        return;
    }
    QFile package(m_packagePath);
    if (!package.open(QIODevice::WriteOnly | QIODevice::Truncate)
        || package.write(reply->readAll()) < 0) {
        setError(QStringLiteral("update.error.write"));
        return;
    }
    package.close();
    // Fetch the sibling checksum, then trust nothing but the hash.
    if (m_assetShaUrl.isEmpty()) {
        setError(QStringLiteral("update.error.noChecksum"));
        return;
    }
    QNetworkRequest request{QUrl(m_assetShaUrl)};
    request.setRawHeader("User-Agent", "Harbor-Updater");
    request.setAttribute(QNetworkRequest::RedirectPolicyAttribute,
                         QNetworkRequest::NoLessSafeRedirectPolicy);
    QNetworkReply *shaReply = m_network->get(request);
    connect(shaReply, &QNetworkReply::finished, this, [this, shaReply]() {
        shaReply->deleteLater();
        if (shaReply->error() != QNetworkReply::NoError) {
            setError(QStringLiteral("update.error.network"));
            return;
        }
        // "<hex>  <filename>" or bare hex.
        const QString expected =
            QString::fromLatin1(shaReply->readAll()).split(QRegularExpression(QStringLiteral("\\s+"))).value(0);
        if (expected.isEmpty() || !verifyChecksum(m_packagePath, expected)) {
            QFile::remove(m_packagePath);
            setError(QStringLiteral("update.error.checksum"));
            return;
        }
        m_progress = 1;
        emit progressChanged();
        setStatus(QStringLiteral("ready"));
        // Mandatory: go as soon as media allows.
        tryApplyNow();
    });
}

void HarborUpdater::setCallActive(bool active)
{
    if (m_callActive == active)
        return;
    m_callActive = active;
    if (!active && m_status == QStringLiteral("ready"))
        tryApplyNow();
}

void HarborUpdater::applyUpdate()
{
    if (m_status != QStringLiteral("ready"))
        return;
    tryApplyNow();
}

void HarborUpdater::tryApplyNow()
{
    if (m_status != QStringLiteral("ready") || m_packagePath.isEmpty()
        || !QFileInfo::exists(m_packagePath))
        return;
    if (m_callActive) {
        // Never drop media for an update: park here until the call ends.
        if (!m_waitingForCall) {
            m_waitingForCall = true;
            emit statusChanged();
        }
        return;
    }
    m_waitingForCall = false;
    setStatus(QStringLiteral("applying"));
    // The running executable cannot replace itself on Windows, and a single
    // code path beats two: relaunch detached into --apply-update, which waits
    // for this process to exit, swaps the payload over the install dir, and
    // starts the new build. Linux takes the same path.
    const QString program = QCoreApplication::applicationFilePath();
    const QStringList args{QStringLiteral("--apply-update"), m_packagePath,
                           QString::number(QCoreApplication::applicationPid())};
    if (!QProcess::startDetached(program, args)) {
        setError(QStringLiteral("update.error.apply"));
        return;
    }
    QCoreApplication::quit();
}

void HarborUpdater::retry()
{
    if (m_status != QStringLiteral("error"))
        return;
    m_recheck.stop();
    checkForUpdates();
}

void HarborUpdater::onRecheckTimer()
{
    if (m_status == QStringLiteral("idle") || m_status == QStringLiteral("error"))
        checkForUpdates();
}
