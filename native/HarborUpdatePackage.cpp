#include "HarborUpdatePackage.h"

#include <QCryptographicHash>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QRegularExpression>
#include <QUrl>

#include <cmath>
#include <limits>

namespace HarborUpdatePackage {
namespace {

constexpr qint64 kMaxPackageSize = 2LL * 1024 * 1024 * 1024;
const QRegularExpression kVersionPattern(QStringLiteral("^([0-9]+)\\.([0-9]+)\\.([0-9]+)$"));
const QRegularExpression kDigestPattern(QStringLiteral("^sha256:[0-9A-Fa-f]{64}$"));
const QRegularExpression kHexPattern(QStringLiteral("^[0-9A-Fa-f]{64}$"));

void fail(ValidationResult &result, const QString &code, const QString &detail)
{
    result.failures.append({code, detail});
}

bool validHttpsUrl(const QString &value)
{
    const QUrl url(value);
    return url.isValid() && url.scheme() == QStringLiteral("https") && !url.host().isEmpty();
}

bool parseVersion(const QString &value, QString *normalized)
{
    QString clean = value;
    if (clean.startsWith(QLatin1Char('v')) || clean.startsWith(QLatin1Char('V')))
        clean = clean.mid(1);
    const auto match = kVersionPattern.match(clean);
    if (!match.hasMatch())
        return false;
    for (int i = 1; i <= 3; ++i) {
        const QString component = match.captured(i);
        if (component.size() > 1 && component.startsWith(QLatin1Char('0')))
            return false;
    }
    if (normalized)
        *normalized = clean;
    return true;
}

int compareCanonicalVersions(const QString &left, const QString &right)
{
    const QStringList a = left.split(QLatin1Char('.'));
    const QStringList b = right.split(QLatin1Char('.'));
    for (int i = 0; i < 3; ++i) {
        if (a.at(i).size() != b.at(i).size())
            return a.at(i).size() < b.at(i).size() ? -1 : 1;
        if (a.at(i) != b.at(i))
            return a.at(i) < b.at(i) ? -1 : 1;
    }
    return 0;
}

QString expectedUrl(const QString &tag, const QString &name)
{
    return QStringLiteral("https://github.com/Joshua-lvt/Harbor/releases/download/")
        + tag + QLatin1Char('/') + name;
}

bool validHex(const QString &value)
{
    return kHexPattern.match(value).hasMatch();
}

} // namespace

BoundedPackageWriter::BoundedPackageWriter(QIODevice *sink, qint64 expectedSize)
    : m_sink(sink), m_expectedSize(expectedSize)
{
}

bool BoundedPackageWriter::append(const QByteArray &chunk)
{
    if (!m_sink || chunk.isEmpty() || chunk.size() > m_expectedSize - m_bytesWritten)
        return chunk.isEmpty();
    qint64 offset = 0;
    while (offset < chunk.size()) {
        const qint64 written = m_sink->write(chunk.constData() + offset, chunk.size() - offset);
        if (written <= 0)
            return false;
        offset += written;
    }
    m_hash.addData(chunk);
    m_bytesWritten += chunk.size();
    return true;
}

bool BoundedPackageWriter::finish() const
{
    return m_bytesWritten == m_expectedSize;
}

QString BoundedPackageWriter::streamedChecksum() const
{
    return QString::fromLatin1(m_hash.result().toHex());
}

QString ValidationResult::errorKey() const
{
    if (failures.isEmpty())
        return {};
    const QString code = failures.first().code;
    if (code == QStringLiteral("version") || code == QStringLiteral("not_newer"))
        return QStringLiteral("update.error.invalidRelease");
    if (code == QStringLiteral("asset") || code == QStringLiteral("url"))
        return QStringLiteral("update.error.noArtifact");
    if (code == QStringLiteral("checksum"))
        return QStringLiteral("update.error.noChecksum");
    if (code == QStringLiteral("size") || code == QStringLiteral("digest"))
        return QStringLiteral("update.error.invalidMetadata");
    return QStringLiteral("update.error.invalidPackage");
}

QString platformName(Platform platform)
{
    if (platform == Platform::Windows)
        return QStringLiteral("windows");
    if (platform == Platform::Linux)
        return QStringLiteral("linux");
    return {};
}

QString architectureName(Architecture architecture)
{
    return architecture == Architecture::X86_64 ? QStringLiteral("x86_64") : QString();
}

Platform platformFromString(const QString &platform)
{
    if (platform.compare(QStringLiteral("windows"), Qt::CaseInsensitive) == 0)
        return Platform::Windows;
    if (platform.compare(QStringLiteral("linux"), Qt::CaseInsensitive) == 0)
        return Platform::Linux;
    return Platform::Unsupported;
}

Architecture architectureFromString(const QString &architecture)
{
    if (architecture.compare(QStringLiteral("x86_64"), Qt::CaseInsensitive) == 0
        || architecture.compare(QStringLiteral("amd64"), Qt::CaseInsensitive) == 0)
        return Architecture::X86_64;
    return Architecture::Unsupported;
}

QString expectedAssetName(Platform platform, Architecture architecture)
{
    if ((platform != Platform::Windows && platform != Platform::Linux)
        || architecture != Architecture::X86_64)
        return {};
    return QStringLiteral("harbor-") + platformName(platform)
        + QStringLiteral("-x86_64")
        + (platform == Platform::Windows ? QStringLiteral(".zip") : QStringLiteral(".tar.gz"));
}

int compareVersions(const QString &left, const QString &right)
{
    QString a, b;
    if (!parseVersion(left, &a) || !parseVersion(right, &b))
        return 0;
    return compareCanonicalVersions(a, b);
}

bool parseChecksumResponse(const QByteArray &response, const QString &assetName, QString *checksum)
{
    const QStringList tokens = QString::fromLatin1(response).trimmed()
                                   .split(QRegularExpression(QStringLiteral("\\s+")), Qt::SkipEmptyParts);
    if (tokens.size() != 1 && tokens.size() != 2)
        return false;
    if (!validHex(tokens.at(0)) || (tokens.size() == 2 && tokens.at(1) != assetName))
        return false;
    if (checksum)
        *checksum = tokens.at(0);
    return true;
}

ValidationResult selectRelease(const QJsonObject &release, const QString &currentVersion,
                               Platform platform, Architecture architecture)
{
    ValidationResult result;
    const QString wanted = expectedAssetName(platform, architecture);
    if (wanted.isEmpty()) {
        fail(result, QStringLiteral("platform"), QStringLiteral("unsupported platform or architecture"));
        return result;
    }
    const QString tag = release.value(QStringLiteral("tag_name")).toString();
    QString version;
    if (!parseVersion(tag, &version))
        fail(result, QStringLiteral("version"), QStringLiteral("release tag is not stable semantic version"));
    const QJsonValue draftValue = release.value(QStringLiteral("draft"));
    const QJsonValue prereleaseValue = release.value(QStringLiteral("prerelease"));
    if (!draftValue.isBool() || !prereleaseValue.isBool())
        fail(result, QStringLiteral("metadata"), QStringLiteral("release stability flags are missing"));
    if (draftValue.toBool(false))
        fail(result, QStringLiteral("draft"), QStringLiteral("draft releases are not supported"));
    if (prereleaseValue.toBool(false))
        fail(result, QStringLiteral("prerelease"), QStringLiteral("pre-releases are not supported"));

    QString current;
    if (!parseVersion(currentVersion, &current))
        fail(result, QStringLiteral("version"), QStringLiteral("current version is invalid"));
    else if (!version.isEmpty() && compareCanonicalVersions(version, current) <= 0)
        fail(result, QStringLiteral("not_newer"), QStringLiteral("release is not newer than current version"));

    const QJsonArray assets = release.value(QStringLiteral("assets")).toArray();
    QJsonObject packageAsset;
    QJsonObject checksumAsset;
    QJsonObject signedManifestAsset;
    for (const QJsonValue &value : assets) {
        const QJsonObject asset = value.toObject();
        const QString name = asset.value(QStringLiteral("name")).toString();
        if (name == wanted)
            packageAsset = asset;
        else if (name == wanted + QStringLiteral(".sha256"))
            checksumAsset = asset;
        else if (name == wanted + QStringLiteral(".update.json"))
            signedManifestAsset = asset;
    }
    if (packageAsset.isEmpty())
        fail(result, QStringLiteral("asset"), QStringLiteral("expected desktop asset is missing"));
    if (checksumAsset.isEmpty())
        fail(result, QStringLiteral("checksum"), QStringLiteral("sibling checksum asset is missing"));
    if (platform == Platform::Windows && signedManifestAsset.isEmpty())
        fail(result, QStringLiteral("checksum"), QStringLiteral("signed update manifest is missing"));

    const QString packageUrl = packageAsset.value(QStringLiteral("browser_download_url")).toString();
    const QString checksumUrl = checksumAsset.value(QStringLiteral("browser_download_url")).toString();
    const QString signedManifestUrl = signedManifestAsset.value(QStringLiteral("browser_download_url")).toString();
    if (!validHttpsUrl(packageUrl) || packageUrl != expectedUrl(tag, wanted))
        fail(result, QStringLiteral("url"), QStringLiteral("package URL is not the expected GitHub release URL"));
    if (!validHttpsUrl(checksumUrl) || checksumUrl != expectedUrl(tag, wanted + QStringLiteral(".sha256")))
        fail(result, QStringLiteral("url"), QStringLiteral("checksum URL is not the expected sibling URL"));
    if (platform == Platform::Windows
        && (!validHttpsUrl(signedManifestUrl)
            || signedManifestUrl != expectedUrl(tag, wanted + QStringLiteral(".update.json"))))
        fail(result, QStringLiteral("url"), QStringLiteral("signed manifest URL is not the expected sibling URL"));

    const QJsonValue sizeValue = packageAsset.value(QStringLiteral("size"));
    const double rawSize = sizeValue.isDouble() ? sizeValue.toDouble() : 0.0;
    const bool validSize = sizeValue.isDouble() && std::isfinite(rawSize) && rawSize > 0.0
        && rawSize <= static_cast<double>(kMaxPackageSize)
        && std::floor(rawSize) == rawSize
        && rawSize <= static_cast<double>(std::numeric_limits<qint64>::max());
    const qint64 size = validSize ? static_cast<qint64>(rawSize) : 0;
    if (!validSize)
        fail(result, QStringLiteral("size"), QStringLiteral("package size is missing or unreasonable"));
    const QString digest = packageAsset.value(QStringLiteral("digest")).toString();
    if (!kDigestPattern.match(digest).hasMatch())
        fail(result, QStringLiteral("digest"), QStringLiteral("GitHub digest is not sha256 plus 64 hex characters"));

    if (!result.failures.isEmpty())
        return result;
    result.valid = true;
    result.manifest = {platform, architecture, version, tag, wanted, packageUrl, size, digest,
                       checksumUrl, signedManifestUrl};
    return result;
}

ValidationResult validateDownloadedPackage(const QString &filePath, const Manifest &manifest,
                                           const QString &expectedChecksum,
                                           const QString &computedChecksum)
{
    ValidationResult result;
    const QFileInfo info(filePath);
    if (!info.exists() || !info.isFile() || info.isSymLink()) {
        fail(result, QStringLiteral("file"), QStringLiteral("downloaded package does not exist or is not a regular file"));
        return result;
    }
    if (info.size() <= 0 || info.size() > kMaxPackageSize ||
        (manifest.declaredSize > 0 && info.size() != manifest.declaredSize)) {
        fail(result, QStringLiteral("size"), QStringLiteral("downloaded package size is invalid"));
        return result;
    }
    QFile file(filePath);
    if (!file.open(QIODevice::ReadOnly)) {
        fail(result, QStringLiteral("file"), QStringLiteral("downloaded package cannot be read"));
        return result;
    }
    const QByteArray prefix = file.read(4);
    const bool zip = manifest.assetName.endsWith(QStringLiteral(".zip"));
    if ((zip && !(prefix.startsWith("PK\x03\x04") || prefix.startsWith("PK\x05\x06") || prefix.startsWith("PK\x07\x08")))
        || (!zip && !(prefix.size() >= 2 && prefix.at(0) == char(0x1f) && prefix.at(1) == char(0x8b)))) {
        fail(result, QStringLiteral("magic"), QStringLiteral("downloaded package has the wrong archive format"));
        return result;
    }
    if (!kHexPattern.match(expectedChecksum.trimmed()).hasMatch()) {
        fail(result, QStringLiteral("checksum"), QStringLiteral("checksum is not exactly 64 hexadecimal characters"));
        return result;
    }
    // The committed file is the trust boundary. The streamed digest is only a
    // diagnostic optimization; always hash this regular file after commit.
    Q_UNUSED(computedChecksum);
    if (!file.seek(0)) {
        fail(result, QStringLiteral("file"), QStringLiteral("committed package could not be rewound"));
        return result;
    }
    QCryptographicHash committedHash(QCryptographicHash::Sha256);
    QByteArray chunk;
    while (!(chunk = file.read(64 * 1024)).isEmpty())
        committedHash.addData(chunk);
    if (file.error() != QFile::NoError) {
        fail(result, QStringLiteral("file"), QStringLiteral("committed package could not be hashed"));
        return result;
    }
    const QString actualChecksum = QString::fromLatin1(committedHash.result().toHex());
    if (actualChecksum.compare(expectedChecksum.trimmed(), Qt::CaseInsensitive) != 0) {
        fail(result, QStringLiteral("checksum"), QStringLiteral("downloaded package checksum does not match"));
        return result;
    }
    if (!kDigestPattern.match(manifest.githubDigest).hasMatch()
        || manifest.githubDigest.mid(7).compare(actualChecksum, Qt::CaseInsensitive) != 0) {
        fail(result, QStringLiteral("digest"), QStringLiteral("checksum disagrees with GitHub API digest"));
        return result;
    }
    result.valid = true;
    result.manifest = manifest;
    return result;
}

Platform hostPlatform()
{
#ifdef Q_OS_WIN
    return Platform::Windows;
#elif defined(Q_OS_LINUX)
    return Platform::Linux;
#else
    return Platform::Unsupported;
#endif
}

Architecture hostArchitecture()
{
#if defined(Q_PROCESSOR_X86_64) || defined(Q_PROCESSOR_AMD64)
    return Architecture::X86_64;
#else
    return Architecture::Unsupported;
#endif
}

} // namespace HarborUpdatePackage
