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
#include <QDebug>
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
#include <QRandomGenerator>
#include <QRegularExpression>
#include <QScopeGuard>
#include <QStandardPaths>
#include <QTemporaryFile>
#include <QUuid>
#include <utility>
#ifdef Q_OS_UNIX
#include <unistd.h>
#endif
#ifdef Q_OS_WIN
#include <aclapi.h>
#include <sddl.h>
#include <windows.h>
#endif

namespace {
#ifdef Q_OS_WIN
QByteArray currentUserSid()
{
    HANDLE token = nullptr;
    if (!OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &token))
        return {};
    DWORD size = 0;
    GetTokenInformation(token, TokenUser, nullptr, 0, &size);
    QByteArray tokenData(qsizetype(size), Qt::Uninitialized);
    if (size == 0 || !GetTokenInformation(token, TokenUser, tokenData.data(), size, &size)) {
        CloseHandle(token);
        return {};
    }
    const auto *user = reinterpret_cast<const TOKEN_USER *>(tokenData.constData());
    const DWORD sidSize = GetLengthSid(user->User.Sid);
    QByteArray sid(qsizetype(sidSize), Qt::Uninitialized);
    const bool copied = CopySid(sidSize, sid.data(), user->User.Sid);
    CloseHandle(token);
    return copied ? sid : QByteArray{};
}

bool windowsPrivatePath(const QString &path)
{
    const QByteArray currentSid = currentUserSid();
    if (currentSid.isEmpty())
        return false;
    PSECURITY_DESCRIPTOR descriptor = nullptr;
    PSID owner = nullptr;
    PACL dacl = nullptr;
    const DWORD status = GetNamedSecurityInfoW(
        const_cast<wchar_t *>(reinterpret_cast<const wchar_t *>(path.utf16())), SE_FILE_OBJECT,
        OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION, &owner, nullptr, &dacl, nullptr,
        &descriptor);
    if (status != ERROR_SUCCESS || descriptor == nullptr || owner == nullptr || dacl == nullptr)
        return false;
    const auto release = qScopeGuard([descriptor] { LocalFree(descriptor); });
    if (!EqualSid(owner, const_cast<char *>(currentSid.constData())))
        return false;
    constexpr ACCESS_MASK writable = GENERIC_WRITE | GENERIC_ALL | FILE_WRITE_DATA
                                     | FILE_APPEND_DATA | FILE_WRITE_EA | FILE_WRITE_ATTRIBUTES
                                     | DELETE | WRITE_DAC | WRITE_OWNER;
    // Reject only write access for non-privileged local principals. SYSTEM,
    // Administrators and CREATOR OWNER remain legitimate; GetEffectiveRightsFromAclW
    // honors deny and INHERIT_ONLY ACEs exactly like the object manager.
    for (const wchar_t *threat : {L"S-1-1-0", L"S-1-5-11", L"S-1-5-32-545"}) {
        PSID sid = nullptr;
        if (!ConvertStringSidToSidW(threat, &sid))
            return false;
        const auto freeSid = qScopeGuard([sid] { LocalFree(sid); });
        TRUSTEE_W trustee{};
        BuildTrusteeWithSidW(&trustee, sid);
        DWORD rights = 0;
        if (GetEffectiveRightsFromAclW(dacl, &trustee, &rights) != ERROR_SUCCESS)
            return false;
        if ((rights & writable) != 0)
            return false;
    }
    return true;
}

bool hardenWindowsPrivatePath(const QString &path, bool directory)
{
    const QByteArray sid = currentUserSid();
    if (sid.isEmpty())
        return false;
    EXPLICIT_ACCESSW access{};
    access.grfAccessPermissions = GENERIC_ALL;
    access.grfAccessMode = SET_ACCESS;
    access.grfInheritance = directory ? SUB_CONTAINERS_AND_OBJECTS_INHERIT : NO_INHERITANCE;
    BuildTrusteeWithSidW(&access.Trustee, const_cast<char *>(sid.constData()));
    PACL acl = nullptr;
    if (SetEntriesInAclW(1, &access, nullptr, &acl) != ERROR_SUCCESS || acl == nullptr)
        return false;
    const auto release = qScopeGuard([acl] { LocalFree(acl); });
    const DWORD status = SetNamedSecurityInfoW(
        const_cast<wchar_t *>(reinterpret_cast<const wchar_t *>(path.utf16())), SE_FILE_OBJECT,
        OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION
            | PROTECTED_DACL_SECURITY_INFORMATION,
        const_cast<char *>(sid.constData()), nullptr, acl, nullptr);
    return status == ERROR_SUCCESS && windowsPrivatePath(path);
}

QString quoteWindowsArgument(const QString &argument)
{
    if (!argument.isEmpty() && !argument.contains(QRegularExpression(QStringLiteral("[\\s\"]"))))
        return argument;
    QString quoted = QStringLiteral("\"");
    qsizetype backslashes = 0;
    for (const QChar character : argument) {
        if (character == QLatin1Char('\\')) {
            ++backslashes;
        } else if (character == QLatin1Char('"')) {
            quoted += QString(backslashes * 2 + 1, QLatin1Char('\\')) + character;
            backslashes = 0;
        } else {
            quoted += QString(backslashes, QLatin1Char('\\')) + character;
            backslashes = 0;
        }
    }
    quoted += QString(backslashes * 2, QLatin1Char('\\')) + QLatin1Char('"');
    return quoted;
}

bool launchElevated(const QString &program, const QStringList &arguments)
{
    const QString readyEventName = QStringLiteral("Local\\HarborUpdate-")
        + QUuid::createUuid().toString(QUuid::WithoutBraces).toLower();
    HANDLE readyEvent = CreateEventW(nullptr, TRUE, FALSE,
        reinterpret_cast<const wchar_t *>(readyEventName.utf16()));
    if (!readyEvent)
        return false;
    const auto closeReadyEvent = qScopeGuard([readyEvent] { CloseHandle(readyEvent); });
    QStringList brokerArguments = arguments;
    brokerArguments.append({QStringLiteral("--ready-event"), readyEventName});
    const QString parameters = [&] {
        QStringList quoted;
        for (const QString &argument : brokerArguments)
            quoted.append(quoteWindowsArgument(argument));
        return quoted.join(QLatin1Char(' '));
    }();
    SHELLEXECUTEINFOW info{};
    info.cbSize = sizeof(info);
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_FLAG_NO_UI;
    info.lpVerb = L"runas";
    info.lpFile = reinterpret_cast<const wchar_t *>(program.utf16());
    info.lpParameters = reinterpret_cast<const wchar_t *>(parameters.utf16());
    const QString directory = QFileInfo(program).absolutePath();
    info.lpDirectory = reinterpret_cast<const wchar_t *>(directory.utf16());
    info.nShow = SW_HIDE;
    if (!ShellExecuteExW(&info))
        return false;
    if (!info.hProcess)
        return false;
    const auto closeProcess = qScopeGuard([process = info.hProcess] { CloseHandle(process); });
    HANDLE handles[] = {readyEvent, info.hProcess};
    const DWORD wait = WaitForMultipleObjects(2, handles, FALSE, 30 * 1000);
    if (wait == WAIT_OBJECT_0)
        return true;
    if (wait == WAIT_TIMEOUT) {
        TerminateProcess(info.hProcess, ERROR_TIMEOUT);
        WaitForSingleObject(info.hProcess, 5000);
    }
    return false;
}

QString registryString(const wchar_t *name)
{
    DWORD bytes = 0;
    if (RegGetValueW(HKEY_LOCAL_MACHINE, L"Software\\Harbor", name, RRF_RT_REG_SZ,
                     nullptr, nullptr, &bytes) != ERROR_SUCCESS || bytes < sizeof(wchar_t))
        return {};
    QByteArray buffer(qsizetype(bytes), Qt::Uninitialized);
    if (RegGetValueW(HKEY_LOCAL_MACHINE, L"Software\\Harbor", name, RRF_RT_REG_SZ,
                     nullptr, buffer.data(), &bytes) != ERROR_SUCCESS)
        return {};
    return QString::fromWCharArray(reinterpret_cast<const wchar_t *>(buffer.constData()));
}

QString protectedBrokerPath()
{
    const QString registeredInstall = QDir::cleanPath(registryString(L"InstallDir"));
    const QString applicationInstall = QDir::cleanPath(QCoreApplication::applicationDirPath());
    const QString broker = QDir::cleanPath(registryString(L"BrokerPath"));
    const QFileInfo info(broker);
    if (registeredInstall.compare(applicationInstall, Qt::CaseInsensitive) != 0
        || !info.isFile() || info.isSymLink())
        return {};
    return info.absoluteFilePath();
}
#endif

bool installParentWritable()
{
    QTemporaryFile probe(QFileInfo(QCoreApplication::applicationDirPath()).absolutePath()
                         + QStringLiteral("/.harbor-update-probe-XXXXXX"));
    return probe.open();
}

bool privateUpdaterRoot(const QString &root)
{
    QFileInfo info(root);
    if (info.exists()) {
        if (!info.isDir() || info.isSymLink()
            || info.canonicalFilePath() != QDir::cleanPath(root))
            return false;
#ifdef Q_OS_UNIX
        return info.ownerId() == uint(::geteuid())
               && !(info.permissions() & (QFileDevice::ReadGroup | QFileDevice::WriteGroup
                                           | QFileDevice::ExeGroup | QFileDevice::ReadOther
                                           | QFileDevice::WriteOther | QFileDevice::ExeOther));
#elif defined(Q_OS_WIN)
        return windowsPrivatePath(root);
#else
        return false;
#endif
    }
    QFileInfo parent(QDir::cleanPath(QFileInfo(root).absolutePath()));
    return parent.isDir() && !parent.isSymLink() && !parent.canonicalFilePath().isEmpty()
#ifdef Q_OS_UNIX
           && parent.ownerId() == uint(::geteuid())
#endif
        ;
}
bool canonicalTransaction(const QString &value)
{
    const QUuid uuid(value);
    return !uuid.isNull() && uuid.toString(QUuid::WithoutBraces) == value
           && value.at(14) == QLatin1Char('4')
           && QStringLiteral("89ab").contains(value.at(19));
}
}

#ifdef Q_OS_WIN
bool harborWindowsPrivatePath(const QString &path)
{
    return windowsPrivatePath(path);
}
#endif

namespace {

constexpr char kOwner[] = "Joshua-lvt";
constexpr char kRepo[] = "Harbor";
constexpr int kRecheckIdleHours = 6;
constexpr qint64 kReleaseMetadataLimit = 1024 * 1024;
constexpr qint64 kPackageReadBuffer = 64 * 1024;

class SaveFile final : public HarborUpdaterPackageFile
{
public:
    explicit SaveFile(const QString &path) : file(path) {}
    QIODevice *device() override { return &file; }
    bool open() override { return file.open(QIODevice::WriteOnly); }
    void cancel() override { file.cancelWriting(); }
    bool commit() override { return file.commit(); }
private:
    QSaveFile file;
};

QString currentVersionString()
{
#ifdef HARBOR_VERSION_STRING
    return QStringLiteral(HARBOR_VERSION_STRING);
#else
    return QStringLiteral("0.0.0");
#endif
}

} // namespace

int HarborUpdater::compareVersions(const QString &a, const QString &b)
{
    return HarborUpdatePackage::compareVersions(a, b);
}

QJsonObject HarborUpdater::pickAsset(const QJsonObject &release)
{
    const auto selected = HarborUpdatePackage::selectRelease(
        release, QStringLiteral("0.0.0"), HarborUpdatePackage::hostPlatform(),
        HarborUpdatePackage::hostArchitecture());
    if (!selected.valid)
        return {};
    QJsonObject picked;
    picked.insert(QStringLiteral("url"), selected.manifest.assetUrl);
    picked.insert(QStringLiteral("shaUrl"), selected.manifest.checksumUrl);
    picked.insert(QStringLiteral("name"), selected.manifest.assetName);
    picked.insert(QStringLiteral("size"), selected.manifest.declaredSize);
    picked.insert(QStringLiteral("digest"), selected.manifest.githubDigest);
    return picked;
}

HarborUpdater::HarborUpdater(QObject *parent, Dependencies dependencies)
    : QObject(parent)
    , m_network(new QNetworkAccessManager(this))
    , m_dependencies(std::move(dependencies))
{
    if (!m_dependencies.get)
        m_dependencies.get = [this](const QNetworkRequest &request) { return m_network->get(request); };
    if (!m_dependencies.launch)
        m_dependencies.launch = [](const QString &program, const QStringList &args, qint64) {
            return QProcess::startDetached(program, args);
        };
#ifdef Q_OS_WIN
    if (!m_dependencies.launchElevated)
        m_dependencies.launchElevated = [](const QString &program, const QStringList &args, qint64) {
            return launchElevated(program, args);
        };
#endif
    if (!m_dependencies.packageFile)
        m_dependencies.packageFile = [](const QString &path) {
            return std::make_unique<SaveFile>(path);
        };
    m_recheck.setSingleShot(true);
    connect(&m_recheck, &QTimer::timeout, this, &HarborUpdater::onRecheckTimer);
    loadLastUpdateResult();
    scheduleRecheck(kRecheckIdleHours * 3600 * 1000);
}

void HarborUpdater::loadLastUpdateResult()
{
    const QString root = QStandardPaths::writableLocation(QStandardPaths::CacheLocation)
        + QStringLiteral("/harbor-updater");
    const QFileInfoList results = QDir(root).entryInfoList(
        {QStringLiteral("*.result.json")}, QDir::Files | QDir::NoSymLinks, QDir::Time);
    for (const QFileInfo &info : results) {
        if (info.size() <= 0 || info.size() > 16 * 1024)
            continue;
        QFile file(info.absoluteFilePath());
        if (!file.open(QIODevice::ReadOnly))
            continue;
        QJsonParseError error;
        const QJsonDocument document = QJsonDocument::fromJson(file.readAll(), &error);
        if (error.error != QJsonParseError::NoError || !document.isObject())
            continue;
        const QJsonObject object = document.object();
        const QString result = object.value(QStringLiteral("result")).toString();
        if (result != QStringLiteral("UPDATED") && result != QStringLiteral("FAILED")
            && result != QStringLiteral("RECOVERY_REQUIRED"))
            continue;
        m_lastUpdateResult = result.toLower();
        m_lastUpdateVersion = object.value(QStringLiteral("version")).toString();
        m_lastUpdateError = object.value(QStringLiteral("error")).toString();
        if (result == QStringLiteral("RECOVERY_REQUIRED")) {
            m_recoveryRequired = true;
            m_errorKey = QStringLiteral("update.error.recoveryRequired");
            m_status = QStringLiteral("error");
        }
        return;
    }
    // The sibling transaction journal is the durable fallback when the cache
    // result could not be persisted. It names the recovery case the same way.
    const QString installParent = QFileInfo(QCoreApplication::applicationDirPath()).absolutePath();
    for (const QString &directory : QDir(installParent).entryList(
             {QStringLiteral(".harbor-tx-*")}, QDir::Dirs | QDir::NoDotAndDotDot, QDir::Name)) {
        if (!directory.startsWith(QStringLiteral(".harbor-tx-"))
            || !canonicalTransaction(directory.mid(QStringLiteral(".harbor-tx-").size())))
            continue;
        const QFileInfo state(installParent + QLatin1Char('/') + directory
                               + QStringLiteral("/state.json"));
        if (!state.isFile() || state.isSymLink())
            continue;
        QFile file(state.absoluteFilePath());
        if (!file.open(QIODevice::ReadOnly))
            continue;
        QJsonParseError error;
        const QJsonDocument document = QJsonDocument::fromJson(file.readAll(), &error);
        if (error.error != QJsonParseError::NoError || !document.isObject())
            continue;
        const QJsonObject object = document.object();
        if (object.value(QStringLiteral("transaction_state")).toString()
            != QStringLiteral("RECOVERY_REQUIRED"))
            continue;
        m_lastUpdateResult = QStringLiteral("recovery_required");
        m_lastUpdateVersion = object.value(QStringLiteral("version")).toString();
        m_lastUpdateError = object.value(QStringLiteral("error")).toString();
        m_recoveryRequired = true;
        m_errorKey = QStringLiteral("update.error.recoveryRequired");
        m_status = QStringLiteral("error");
        return;
    }
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
    if (m_recoveryRequired)
        return;
    if (m_status != QStringLiteral("idle") && m_status != QStringLiteral("error"))
        return;
    m_errorKey.clear();
    m_progress = 0;
    emit progressChanged();
    setStatus(QStringLiteral("checking"));
    qInfo().noquote() << "[Updater] checking latest release";
    QNetworkRequest request(QUrl(QStringLiteral("https://api.github.com/repos/") + QLatin1String(kOwner)
                                 + QLatin1Char('/') + QLatin1String(kRepo)
                                 + QStringLiteral("/releases/latest")));
    request.setRawHeader("Accept", "application/vnd.github+json");
    request.setRawHeader("User-Agent", "Harbor-Updater");
    m_releaseBody.clear();
    m_reply = m_dependencies.get(request);
    m_reply->setReadBufferSize(kReleaseMetadataLimit);
    connect(m_reply, &QIODevice::readyRead, this, &HarborUpdater::onCheckReadyRead);
    connect(m_reply, &QNetworkReply::finished, this, &HarborUpdater::onCheckFinished);
}

void HarborUpdater::onCheckReadyRead()
{
    if (!m_reply)
        return;
    const QByteArray chunk = m_reply->read(kReleaseMetadataLimit + 1);
    if (m_releaseBody.size() + chunk.size() > kReleaseMetadataLimit) {
        QNetworkReply *reply = m_reply;
        m_reply = nullptr;
        reply->disconnect(this);
        reply->abort();
        reply->deleteLater();
        m_releaseBody.clear();
        setError(QStringLiteral("update.error.invalidMetadata"));
        return;
    }
    m_releaseBody.append(chunk);
}

void HarborUpdater::onCheckFinished()
{
    QNetworkReply *reply = m_reply;
    if (!reply)
        return;
    reply->deleteLater();
    if (reply->error() != QNetworkReply::NoError) {
        setError(QStringLiteral("update.error.network"));
        return;
    }
    onCheckReadyRead();
    if (m_status == QStringLiteral("error")) {
        return;
    }
    m_reply = nullptr;
    const QJsonObject release = QJsonDocument::fromJson(m_releaseBody).object();
    m_releaseBody.clear();
    const auto selected = HarborUpdatePackage::selectRelease(
        release, currentVersionString(), HarborUpdatePackage::hostPlatform(),
        HarborUpdatePackage::hostArchitecture());
    qInfo().noquote() << "[Updater] release validation:" << selected.valid;
    if (!selected.valid && !selected.failures.isEmpty()
        && selected.failures.first().code == QStringLiteral("not_newer")) {
        setStatus(QStringLiteral("idle"));
        scheduleRecheck(kRecheckIdleHours * 3600 * 1000);
        return;
    }
    if (!selected.valid) {
        qWarning().noquote() << "[Updater] release rejected:" << selected.failures.first().code
                             << selected.failures.first().detail;
        setError(selected.errorKey());
        return;
    }
    m_manifest = selected.manifest;
    m_availableVersion = m_manifest.version;
    m_assetUrl = m_manifest.assetUrl;
    m_assetShaUrl = m_manifest.checksumUrl;
    qInfo().noquote() << "[Updater] selected" << m_availableVersion << m_manifest.assetName;
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
    const QString fileName = m_manifest.assetName;
    m_packagePath = downloadDir() + QLatin1Char('/') + fileName;
    QFile::remove(m_packagePath);
    m_packageFile = m_dependencies.packageFile(m_packagePath);
    if (!m_packageFile || !m_packageFile->open()) {
        m_packageFile.reset();
        setError(QStringLiteral("update.error.write"));
        return;
    }
    delete m_packageWriter;
    m_packageWriter = new HarborUpdatePackage::BoundedPackageWriter(
        m_packageFile->device(), m_manifest.declaredSize);
    m_downloadedBytes = 0;
    m_downloadPrefix.clear();
    QNetworkRequest request{QUrl(m_assetUrl)};
    request.setRawHeader("User-Agent", "Harbor-Updater");
    // GitHub release assets answer 302 to a same-host object URL; follow it.
    request.setAttribute(QNetworkRequest::RedirectPolicyAttribute,
                         QNetworkRequest::NoLessSafeRedirectPolicy);
    m_reply = m_dependencies.get(request);
    m_reply->setReadBufferSize(kPackageReadBuffer);
    connect(m_reply, &QNetworkReply::downloadProgress, this, &HarborUpdater::onDownloadProgress);
    connect(m_reply, &QIODevice::readyRead, this, &HarborUpdater::onDownloadReadyRead);
    connect(m_reply, &QNetworkReply::finished, this, &HarborUpdater::onDownloadFinished);
}

void HarborUpdater::onDownloadProgress(qint64 received, qint64 total)
{
    if (total <= 0)
        return;
    m_progress = qBound(0.0, double(received) / double(total), 1.0);
    emit progressChanged();
}

void HarborUpdater::abortDownload(const QString &errorKey)
{
    if (m_packageFile) {
        m_packageFile->cancel();
        delete m_packageWriter;
        m_packageWriter = nullptr;
        m_packageFile.reset();
    }
    QFile::remove(m_packagePath);
    QNetworkReply *reply = m_reply;
    m_reply = nullptr;
    if (reply) {
        reply->disconnect(this);
        reply->abort();
        reply->deleteLater();
    }
    setError(errorKey);
}

void HarborUpdater::onDownloadReadyRead()
{
    if (!m_reply || !m_packageFile || !m_packageWriter)
        return;
    while (m_reply->bytesAvailable() > 0) {
        const qint64 remaining = m_manifest.declaredSize - m_downloadedBytes;
        const qint64 chunkSize = qMin<qint64>(64 * 1024, remaining + 1);
        const QByteArray chunk = m_reply->read(chunkSize);
        if (chunk.isEmpty())
            break;
        if (m_downloadedBytes + chunk.size() > m_manifest.declaredSize
            || m_downloadedBytes + chunk.size() > 2LL * 1024 * 1024 * 1024) {
            abortDownload(QStringLiteral("update.error.invalidMetadata"));
            return;
        }
        if (!m_packageWriter->append(chunk)) {
            abortDownload(QStringLiteral("update.error.write"));
            return;
        }
        m_downloadedBytes += chunk.size();
        if (m_downloadPrefix.size() < 4)
            m_downloadPrefix.append(chunk.left(4 - m_downloadPrefix.size()));
    }
}

void HarborUpdater::onDownloadFinished()
{
    QNetworkReply *reply = m_reply;
    if (!reply)
        return;
    onDownloadReadyRead();
    if (!m_packageFile) {
        reply->deleteLater();
        return;
    }
    m_reply = nullptr;
    reply->deleteLater();
    if (reply->error() != QNetworkReply::NoError) {
        m_packageFile->cancel();
        delete m_packageWriter;
        m_packageWriter = nullptr;
        m_packageFile.reset();
        QFile::remove(m_packagePath);
        setError(QStringLiteral("update.error.network"));
        return;
    }
    if (m_downloadedBytes != m_manifest.declaredSize) {
        abortDownload(QStringLiteral("update.error.invalidMetadata"));
        return;
    }
    if (!m_packageWriter->finish() || !m_packageFile->commit()) {
        m_packageFile->cancel();
        delete m_packageWriter;
        m_packageWriter = nullptr;
        m_packageFile.reset();
        QFile::remove(m_packagePath);
        setError(QStringLiteral("update.error.write"));
        return;
    }
    m_packageFile.reset();
    const QString streamedChecksum = m_packageWriter->streamedChecksum();
    Q_UNUSED(streamedChecksum);
    delete m_packageWriter;
    m_packageWriter = nullptr;
    qInfo().noquote() << "[Updater] download completed, bytes:" << m_downloadedBytes;
    // Fetch the sibling checksum, then trust nothing but the hash.
    if (m_assetShaUrl.isEmpty()) {
        QFile::remove(m_packagePath);
        setError(QStringLiteral("update.error.noChecksum"));
        return;
    }
    QNetworkRequest request{QUrl(m_assetShaUrl)};
    request.setRawHeader("User-Agent", "Harbor-Updater");
    request.setAttribute(QNetworkRequest::RedirectPolicyAttribute,
                         QNetworkRequest::NoLessSafeRedirectPolicy);
    m_checksumBody.clear();
    m_checksumReply = m_dependencies.get(request);
    m_checksumReply->setReadBufferSize(4097);
    connect(m_checksumReply, &QIODevice::readyRead, this, &HarborUpdater::onChecksumReadyRead);
    connect(m_checksumReply, &QNetworkReply::finished, this, &HarborUpdater::onChecksumFinished);
}

void HarborUpdater::onChecksumReadyRead()
{
    if (!m_checksumReply)
        return;
    const QByteArray chunk = m_checksumReply->read(4097);
    if (m_checksumBody.size() + chunk.size() > 4096) {
        QNetworkReply *reply = m_checksumReply;
        m_checksumReply = nullptr;
        reply->disconnect(this);
        reply->abort();
        reply->deleteLater();
        m_checksumBody.clear();
        QFile::remove(m_packagePath);
        setError(QStringLiteral("update.error.checksum"));
        return;
    }
    m_checksumBody.append(chunk);
}

void HarborUpdater::onChecksumFinished()
{
    QNetworkReply *reply = m_checksumReply;
    if (!reply)
        return;
    onChecksumReadyRead();
    if (!m_checksumReply)
        return;
    m_checksumReply = nullptr;
    reply->deleteLater();
    if (reply->error() != QNetworkReply::NoError) {
        QFile::remove(m_packagePath);
        setError(QStringLiteral("update.error.network"));
        return;
    }
    QString expected;
    if (!HarborUpdatePackage::parseChecksumResponse(
            m_checksumBody, m_manifest.assetName, &expected)) {
        m_checksumBody.clear();
        QFile::remove(m_packagePath);
        setError(QStringLiteral("update.error.checksum"));
        return;
    }
    m_checksumBody.clear();
    const auto validation = HarborUpdatePackage::validateDownloadedPackage(
        m_packagePath, m_manifest, expected);
        qInfo().noquote() << "[Updater] package validation:" << validation.valid;
        if (!validation.valid) {
            qWarning().noquote() << "[Updater] package rejected:" << validation.failures.first().code
                                 << validation.failures.first().detail;
            QFile::remove(m_packagePath);
            setError(validation.errorKey());
            return;
        }
#ifdef Q_OS_WIN
        QNetworkRequest request{QUrl(m_manifest.signedManifestUrl)};
        request.setRawHeader("User-Agent", "Harbor-Updater");
        request.setAttribute(QNetworkRequest::RedirectPolicyAttribute,
                             QNetworkRequest::NoLessSafeRedirectPolicy);
        m_signedManifestBody.clear();
        m_signedManifestReply = m_dependencies.get(request);
        m_signedManifestReply->setReadBufferSize(16 * 1024 + 1);
        connect(m_signedManifestReply, &QIODevice::readyRead,
                this, &HarborUpdater::onSignedManifestReadyRead);
        connect(m_signedManifestReply, &QNetworkReply::finished,
                this, &HarborUpdater::onSignedManifestFinished);
#else
        markReady();
#endif
}

void HarborUpdater::onSignedManifestReadyRead()
{
    if (!m_signedManifestReply)
        return;
    const QByteArray chunk = m_signedManifestReply->read(16 * 1024 + 1);
    if (m_signedManifestBody.size() + chunk.size() > 16 * 1024) {
        QNetworkReply *reply = m_signedManifestReply;
        m_signedManifestReply = nullptr;
        reply->disconnect(this);
        reply->abort();
        reply->deleteLater();
        m_signedManifestBody.clear();
        QFile::remove(m_packagePath);
        setError(QStringLiteral("update.error.invalidMetadata"));
        return;
    }
    m_signedManifestBody.append(chunk);
}

void HarborUpdater::onSignedManifestFinished()
{
    QNetworkReply *reply = m_signedManifestReply;
    if (!reply)
        return;
    onSignedManifestReadyRead();
    if (!m_signedManifestReply)
        return;
    m_signedManifestReply = nullptr;
    reply->deleteLater();
    if (reply->error() != QNetworkReply::NoError || m_signedManifestBody.isEmpty()) {
        m_signedManifestBody.clear();
        QFile::remove(m_packagePath);
        setError(QStringLiteral("update.error.network"));
        return;
    }
    m_signedManifestPath = m_packagePath + QStringLiteral(".update.json");
    QSaveFile file(m_signedManifestPath);
    if (!file.open(QIODevice::WriteOnly)
        || file.write(m_signedManifestBody) != m_signedManifestBody.size()
        || !file.commit()) {
        file.cancelWriting();
        m_signedManifestBody.clear();
        QFile::remove(m_packagePath);
        QFile::remove(m_signedManifestPath);
        setError(QStringLiteral("update.error.write"));
        return;
    }
    m_signedManifestBody.clear();
    markReady();
}

void HarborUpdater::markReady()
{
    m_progress = 1;
    emit progressChanged();
    setStatus(QStringLiteral("ready"));
    // Mandatory: go as soon as media allows.
    tryApplyNow();
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
    const QString helperDirectory = m_dependencies.helperPath.isEmpty()
        ? QCoreApplication::applicationDirPath()
        : QFileInfo(m_dependencies.helperPath).absolutePath();
    const QFileInfo installedHelper(helperDirectory + QStringLiteral("/harbor-update-helper")
#ifdef Q_OS_WIN
                                    + QStringLiteral(".exe")
#endif
                                    );
    if (!installedHelper.isFile() || installedHelper.isSymLink()) {
        setError(QStringLiteral("update.error.apply"));
        return;
    }
    const QString stateRoot = QStandardPaths::writableLocation(QStandardPaths::CacheLocation)
                              + QStringLiteral("/harbor-updater");
    if (!QDir().mkpath(stateRoot)) {
        setError(QStringLiteral("update.error.apply"));
        return;
    }
    if (!QFile::setPermissions(stateRoot, QFileDevice::ReadOwner | QFileDevice::WriteOwner
                                            | QFileDevice::ExeOwner)) {
        setError(QStringLiteral("update.error.apply"));
        return;
    }
#ifdef Q_OS_WIN
    if (!hardenWindowsPrivatePath(stateRoot, true)) {
        setError(QStringLiteral("update.error.apply"));
        return;
    }
#endif
    if (!privateUpdaterRoot(stateRoot)) {
        setError(QStringLiteral("update.error.apply"));
        return;
    }
    const QString transaction = QUuid::createUuid().toString(QUuid::WithoutBraces).toLower();
    if (!canonicalTransaction(transaction)) {
        setError(QStringLiteral("update.error.apply"));
        return;
    }
    const QString transactionDir = stateRoot + QStringLiteral("/") + transaction;
    if (QFileInfo::exists(transactionDir)) {
        setError(QStringLiteral("update.error.apply"));
        return;
    }
    if (!QDir().mkpath(transactionDir)) {
        setError(QStringLiteral("update.error.apply"));
        return;
    }
    if (QFileInfo(transactionDir).absolutePath() != QFileInfo(stateRoot).absoluteFilePath()
        || QFileInfo(transactionDir).fileName() != transaction
        || QFileInfo(transactionDir).isSymLink()) {
        setError(QStringLiteral("update.error.apply"));
        return;
    }
    if (!QFile::setPermissions(transactionDir, QFileDevice::ReadOwner | QFileDevice::WriteOwner
                                              | QFileDevice::ExeOwner)) {
        setError(QStringLiteral("update.error.apply"));
        return;
    }
#ifdef Q_OS_WIN
    if (!hardenWindowsPrivatePath(transactionDir, true)) {
        setError(QStringLiteral("update.error.apply"));
        return;
    }
#endif
    if (!privateUpdaterRoot(transactionDir)
        || QFileInfo(transactionDir).canonicalFilePath()
               != QFileInfo(QDir(stateRoot).filePath(transaction)).canonicalFilePath()) {
        setError(QStringLiteral("update.error.apply"));
        return;
    }
    const QString helper = transactionDir + QStringLiteral("/harbor-update-helper")
#ifdef Q_OS_WIN
                           + QStringLiteral(".exe")
#endif
        ;
    if (!QFile::copy(installedHelper.absoluteFilePath(), helper)) {
        setError(QStringLiteral("update.error.apply"));
        return;
    }
    const QFileInfo copiedHelper(helper);
    if (!copiedHelper.isFile() || copiedHelper.isSymLink()
        || !QFile::setPermissions(helper, QFileDevice::ReadOwner | QFileDevice::WriteOwner
                                             | QFileDevice::ExeOwner)) {
        setError(QStringLiteral("update.error.apply"));
        return;
    }
#ifdef Q_OS_WIN
    if (!hardenWindowsPrivatePath(helper, false)) {
        setError(QStringLiteral("update.error.apply"));
        return;
    }
#endif
    const QString secret = transactionDir + QStringLiteral("/health.secret");
    QFile secretFile(secret);
    if (!secretFile.open(QIODevice::WriteOnly | QIODevice::NewOnly)) {
        setError(QStringLiteral("update.error.apply"));
        return;
    }
    QByteArray secretBytes(32, '\0');
    for (char &byte : secretBytes)
        byte = char(QRandomGenerator::system()->generate() & 0xff);
    if (secretFile.write(secretBytes) != secretBytes.size() || !secretFile.flush()) {
        setError(QStringLiteral("update.error.apply"));
        return;
    }
    secretFile.close();
    if (!QFile::setPermissions(secret, QFileDevice::ReadOwner | QFileDevice::WriteOwner)) {
        setError(QStringLiteral("update.error.apply"));
        return;
    }
#ifdef Q_OS_WIN
    if (!hardenWindowsPrivatePath(secret, false)
        || !windowsPrivatePath(stateRoot) || !windowsPrivatePath(transactionDir)
        || !windowsPrivatePath(helper)) {
        setError(QStringLiteral("update.error.apply"));
        return;
    }
#endif
    QStringList args{QStringLiteral("--package"), m_packagePath,
#ifdef Q_OS_WIN
                           QStringLiteral("--manifest"), m_signedManifestPath,
#endif
                           QStringLiteral("--install-dir"), QCoreApplication::applicationDirPath(),
                           QStringLiteral("--expected-version"), m_availableVersion,
                           QStringLiteral("--parent-pid"), QString::number(QCoreApplication::applicationPid()),
                           QStringLiteral("--updater-root"), stateRoot,
                           QStringLiteral("--transaction-dir"), transactionDir,
                           QStringLiteral("--transaction"), transaction,
#ifdef Q_OS_WIN
                           QStringLiteral("--format"), QStringLiteral("zip")};
#else
                           QStringLiteral("--format"), QStringLiteral("targz")};
#endif
    bool launched = false;
#ifdef Q_OS_WIN
    if (!installParentWritable()) {
        const QString broker = protectedBrokerPath();
        args.removeAt(args.indexOf(QStringLiteral("--install-dir")) + 1);
        args.removeAt(args.indexOf(QStringLiteral("--install-dir")));
        args.removeAt(args.indexOf(QStringLiteral("--format")) + 1);
        args.removeAt(args.indexOf(QStringLiteral("--format")));
        launched = !broker.isEmpty() && m_dependencies.launchElevated
            && m_dependencies.launchElevated(broker, args, QCoreApplication::applicationPid());
    } else
#endif
    {
        launched = m_dependencies.launch(helper, args, QCoreApplication::applicationPid());
    }
    if (!launched) {
        setError(QStringLiteral("update.error.apply"));
        return;
    }
    QCoreApplication::quit();
}

void HarborUpdater::retry()
{
    if (m_status != QStringLiteral("error") || m_recoveryRequired)
        return;
    m_recheck.stop();
    checkForUpdates();
}

void HarborUpdater::onRecheckTimer()
{
    if (m_status == QStringLiteral("idle") || m_status == QStringLiteral("error"))
        checkForUpdates();
}
