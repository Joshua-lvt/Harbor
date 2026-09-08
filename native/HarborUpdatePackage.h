#pragma once

#include <QJsonObject>
#include <QString>
#include <QStringList>
#include <QByteArray>
#include <QCryptographicHash>
#include <QIODevice>

namespace HarborUpdatePackage {

class BoundedPackageWriter final
{
public:
    BoundedPackageWriter(QIODevice *sink, qint64 expectedSize);
    bool append(const QByteArray &chunk);
    bool finish() const;
    qint64 bytesWritten() const { return m_bytesWritten; }
    QString streamedChecksum() const;

private:
    QIODevice *m_sink = nullptr;
    qint64 m_expectedSize = 0;
    qint64 m_bytesWritten = 0;
    QCryptographicHash m_hash{QCryptographicHash::Sha256};
};

enum class Platform { Windows, Linux, Unsupported };
enum class Architecture { X86_64, Unsupported };

struct Manifest {
    Platform platform = Platform::Linux;
    Architecture architecture = Architecture::X86_64;
    QString version;
    QString tag;
    QString assetName;
    QString assetUrl;
    qint64 declaredSize = 0;
    QString githubDigest;
    QString checksumUrl;
    QString signedManifestUrl;
};

struct ValidationFailure {
    QString code;
    QString detail;
};

struct ValidationResult {
    bool valid = false;
    Manifest manifest;
    QList<ValidationFailure> failures;

    QString errorKey() const;
};

QString platformName(Platform platform);
QString architectureName(Architecture architecture);
QString expectedAssetName(Platform platform, Architecture architecture);
Platform platformFromString(const QString &platform);
Architecture architectureFromString(const QString &architecture);
int compareVersions(const QString &left, const QString &right);
bool parseChecksumResponse(const QByteArray &response, const QString &assetName, QString *checksum);

// Validates and selects one release without performing network I/O.
ValidationResult selectRelease(const QJsonObject &release,
                               const QString &currentVersion,
                               Platform platform,
                               Architecture architecture);

// Validates a downloaded archive and its sibling checksum contents locally.
ValidationResult validateDownloadedPackage(const QString &filePath,
                                           const Manifest &manifest,
                                           const QString &expectedChecksum,
                                           const QString &computedChecksum = {});

Platform hostPlatform();
Architecture hostArchitecture();

} // namespace HarborUpdatePackage
