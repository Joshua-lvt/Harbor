#include "HarborUpdatePackage.h"
#include "HarborUpdater.h"

#include <QCryptographicHash>
#include <QBuffer>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QTemporaryDir>
#include <QTest>
#include <QNetworkReply>
#include <QStandardPaths>
#include <QTimer>
#include <QUuid>
#include <cstring>
#ifdef Q_OS_UNIX
#include <unistd.h>
#endif

using namespace HarborUpdatePackage;

class HarborUpdaterTest final : public QObject
{
    Q_OBJECT
private slots:
    void versionsOrderNumerically();
    void strictVersionAndChecksumParsing();
    void boundedWriterRejectsOverflowAndWriteFailure();
    void releaseValidation_data();
    void releaseValidation();
    void aliasesAndUnsupportedTargets();
    void packageValidation_data();
    void packageValidation();
    void digestIsRequiredAndMustMatch();
    void updaterFailureAndSuccessPaths();
    void transactionUuidVectors();
};

void HarborUpdaterTest::transactionUuidVectors()
{
    const QString valid = QStringLiteral("00000000-0000-4000-8000-000000000000");
    QCOMPARE(QUuid(valid).toString(QUuid::WithoutBraces), valid);
    QVERIFY(QStringLiteral("00000000-0000-5000-8000-000000000000").at(14) != QLatin1Char('4'));
    QVERIFY(!QStringLiteral("89ab").contains(QStringLiteral("00000000-0000-4000-7000-000000000000").at(19)));
}

namespace {
QByteArray hashFor(const QByteArray &data);
struct ReplyObservations {
    qint64 metadataBuffer = -1;
    qint64 packageBuffer = -1;
    qint64 checksumBuffer = -1;
    qint64 manifestBuffer = -1;
    bool metadataAborted = false;
    bool packageAborted = false;
    bool checksumAborted = false;
    bool manifestAborted = false;
};

class FakeReply final : public QNetworkReply
{
public:
    enum class Kind { Metadata, Package, Checksum, Manifest };
    FakeReply(const QUrl &url, QByteArray body, Kind kind, ReplyObservations *observations,
              QNetworkReply::NetworkError error = NoError, QObject *parent = nullptr)
        : QNetworkReply(parent), data(std::move(body)), result(error), replyKind(kind), observations(observations)
    {
        setUrl(url);
        open(QIODevice::ReadOnly);
        if (result != NoError)
            setError(result, QStringLiteral("fake network failure"));
        QTimer::singleShot(0, this, [this] {
            if (result == NoError && !data.isEmpty())
                emit readyRead();
            if (this->observations) {
                qint64 *buffer = replyKind == Kind::Metadata ? &this->observations->metadataBuffer
                    : replyKind == Kind::Package ? &this->observations->packageBuffer
                    : replyKind == Kind::Checksum ? &this->observations->checksumBuffer
                                                  : &this->observations->manifestBuffer;
                *buffer = readBufferSize();
                bool *aborted = replyKind == Kind::Metadata ? &this->observations->metadataAborted
                    : replyKind == Kind::Package ? &this->observations->packageAborted
                    : replyKind == Kind::Checksum ? &this->observations->checksumAborted
                                                  : &this->observations->manifestAborted;
                *aborted = wasAborted();
            }
            emit finished();
        });
    }
    void abort() override { aborted = true; }
    bool wasAborted() const { return aborted; }
protected:
    qint64 bytesAvailable() const override
    { return data.size() - offset + QNetworkReply::bytesAvailable(); }
    qint64 readData(char *buffer, qint64 maxSize) override
    {
        const qint64 amount = qMin(maxSize, qint64(data.size() - offset));
        if (amount <= 0)
            return -1;
        memcpy(buffer, data.constData() + offset, size_t(amount));
        offset += amount;
        return amount;
    }
    qint64 writeData(const char *, qint64) override { return -1; }
private:
    QByteArray data;
    int offset = 0;
    QNetworkReply::NetworkError result;
    Kind replyKind;
    ReplyObservations *observations;
    bool aborted = false;
};

Platform testPlatform()
{
    const Platform platform = hostPlatform();
    if (platform == Platform::Unsupported || hostArchitecture() == Architecture::Unsupported)
        QTest::qFail("Updater tests require a supported desktop host", __FILE__, __LINE__);
    return platform;
}

QByteArray validPackage(Platform platform)
{
    return platform == Platform::Windows ? QByteArray("PK\x03\x04payload", 11)
                                         : QByteArray("\x1f\x8b\x08payload", 10);
}

QJsonObject updaterRelease(const QByteArray &package, Platform platform, Architecture architecture)
{
    const QString name = expectedAssetName(platform, architecture);
    const QString base = QStringLiteral("https://github.com/Joshua-lvt/Harbor/releases/download/v2.4.0/");
    QJsonObject artifact{{"name", name}, {"browser_download_url", base + name},
                         {"size", package.size()},
                         {"digest", QStringLiteral("sha256:") + QString::fromLatin1(hashFor(package))}};
    QJsonObject checksum{{"name", name + ".sha256"},
                          {"browser_download_url", base + name + ".sha256"}};
    QJsonObject manifest{{"name", name + ".update.json"},
                          {"browser_download_url", base + name + ".update.json"}};
    return {{"tag_name", "v2.4.0"}, {"draft", false}, {"prerelease", false},
            {"assets", QJsonArray{artifact, checksum, manifest}}};
}

class TestPackageFile final : public HarborUpdaterPackageFile
{
public:
    TestPackageFile(bool writeFailure, bool commitFailure)
        : buffer(writeFailure), failCommit(commitFailure) {}
    QIODevice *device() override { return &buffer; }
    bool open() override { return buffer.open(QIODevice::WriteOnly); }
    void cancel() override { buffer.close(); }
    bool commit() override { return !failCommit; }
private:
    class Device final : public QBuffer {
    public:
        explicit Device(bool failure) : fail(failure) {}
    protected:
        qint64 writeData(const char *data, qint64 size) override
        { return fail ? -1 : QBuffer::writeData(data, size); }
    private:
        bool fail;
    } buffer;
    bool failCommit;
};
}

void HarborUpdaterTest::versionsOrderNumerically()
{
    QCOMPARE(HarborUpdater::compareVersions("2.1.0", "2.1.0"), 0);
    QVERIFY(HarborUpdater::compareVersions("2.1.0", "2.0.9") > 0);
    QVERIFY(HarborUpdater::compareVersions("2.1.0", "2.10.0") < 0);
    QCOMPARE(HarborUpdater::compareVersions("v2.1.0", "2.1.0"), 0);
    QVERIFY(HarborUpdater::compareVersions("999999999999999999999999.0.0", "2.0.0") > 0);
    QCOMPARE(HarborUpdater::compareVersions("2.01.0", "2.1.0"), 0);
}

void HarborUpdaterTest::boundedWriterRejectsOverflowAndWriteFailure()
{
    QBuffer buffer;
    QVERIFY(buffer.open(QIODevice::WriteOnly));
    BoundedPackageWriter writer(&buffer, 4);
    QVERIFY(writer.append("ab"));
    QVERIFY(!writer.append("cdef"));
    QVERIFY(!writer.finish());
    QVERIFY(writer.append("cd"));
    QVERIFY(writer.finish());
    QCOMPARE(buffer.data(), QByteArray("abcd"));

    class FailingDevice final : public QIODevice {
    public:
        FailingDevice() { open(QIODevice::WriteOnly); }
    protected:
        qint64 readData(char *, qint64) override { return -1; }
        qint64 writeData(const char *, qint64) override { return -1; }
    } failing;
    BoundedPackageWriter failed(&failing, 2);
    QVERIFY(!failed.append("ab"));
    QVERIFY(!failed.finish());
}

void HarborUpdaterTest::strictVersionAndChecksumParsing()
{
    QString checksum;
    const QByteArray hex(64, 'a');
    QVERIFY(parseChecksumResponse(hex, "asset.zip", &checksum));
    QCOMPARE(checksum, QString::fromLatin1(hex));
    QVERIFY(parseChecksumResponse(hex + "  asset.zip\n", "asset.zip", &checksum));
    QVERIFY(!parseChecksumResponse(hex + "  other.zip", "asset.zip", &checksum));
    QVERIFY(!parseChecksumResponse(hex + " extra", "asset.zip", &checksum));
    QVERIFY(!parseChecksumResponse(hex + "\n" + hex, "asset.zip", &checksum));
    QVERIFY(!parseChecksumResponse(" ", "asset.zip", &checksum));
    QVERIFY(!parseChecksumResponse(QByteArray(63, 'a'), "asset.zip", &checksum));
}

namespace {
QJsonObject release(const QString &tag, Platform platform = Platform::Linux,
                   bool draft = false, bool prerelease = false,
                   const QString &packageUrl = {})
{
    const QString name = expectedAssetName(platform, Architecture::X86_64);
    const QString base = QStringLiteral("https://github.com/Joshua-lvt/Harbor/releases/download/") + tag + QLatin1Char('/');
    QJsonObject package{{QStringLiteral("name"), name},
                        {QStringLiteral("browser_download_url"), packageUrl.isEmpty() ? base + name : packageUrl},
                        {QStringLiteral("size"), 32},
                        {QStringLiteral("digest"), QStringLiteral("sha256:") + QString(64, QLatin1Char('a'))}};
    QJsonObject checksum{{QStringLiteral("name"), name + QStringLiteral(".sha256")},
                          {QStringLiteral("browser_download_url"), base + name + QStringLiteral(".sha256")}};
    QJsonObject manifest{{QStringLiteral("name"), name + QStringLiteral(".update.json")},
                          {QStringLiteral("browser_download_url"), base + name + QStringLiteral(".update.json")}};
    return {{QStringLiteral("tag_name"), tag}, {QStringLiteral("draft"), draft},
            {QStringLiteral("prerelease"), prerelease}, {QStringLiteral("assets"), QJsonArray{package, checksum, manifest}}};
}

QByteArray hashFor(const QByteArray &data)
{
    return QCryptographicHash::hash(data, QCryptographicHash::Sha256).toHex();
}
}

void HarborUpdaterTest::releaseValidation_data()
{
    QTest::addColumn<QString>("tag");
    QTest::addColumn<bool>("draft");
    QTest::addColumn<bool>("prerelease");
    QTest::addColumn<bool>("valid");
    QTest::newRow("newer") << "v2.3.0" << false << false << true;
    QTest::newRow("current") << "v2.1.0" << false << false << false;
    QTest::newRow("malformed") << "v2.3" << false << false << false;
    QTest::newRow("leading-major-zero") << "v02.3.0" << false << false << false;
    QTest::newRow("leading-minor-zero") << "v2.03.0" << false << false << false;
    QTest::newRow("leading-patch-zero") << "v2.3.00" << false << false << false;
    QTest::newRow("huge-component") << "v999999999999999999999999.0.0" << false << false << true;
    QTest::newRow("draft") << "v2.3.0" << true << false << false;
    QTest::newRow("prerelease") << "v2.3.0-rc1" << false << true << false;
}

void HarborUpdaterTest::releaseValidation()
{
    QFETCH(QString, tag);
    QFETCH(bool, draft);
    QFETCH(bool, prerelease);
    QFETCH(bool, valid);
    const auto result = selectRelease(release(tag, Platform::Linux, draft, prerelease), "2.1.0",
                                      Platform::Linux, Architecture::X86_64);
    QCOMPARE(result.valid, valid);
}

void HarborUpdaterTest::aliasesAndUnsupportedTargets()
{
    QCOMPARE(architectureFromString("amd64"), Architecture::X86_64);
    QCOMPARE(architectureFromString("arm64"), Architecture::Unsupported);
    QCOMPARE(platformFromString("macos"), Platform::Unsupported);
    QVERIFY(selectRelease(release("v2.3.0", Platform::Windows), "2.1.0", Platform::Windows,
                          Architecture::X86_64).valid);
    QVERIFY(!selectRelease(release("v2.3.0"), "2.1.0", Platform::Linux,
                           Architecture::Unsupported).valid);
    QVERIFY(expectedAssetName(Platform::Unsupported, Architecture::X86_64).isEmpty());
    QVERIFY(!selectRelease(release("v2.3.0"), "2.1.0", Platform::Unsupported,
                           Architecture::X86_64).valid);
#ifdef Q_OS_WIN
    QCOMPARE(hostPlatform(), Platform::Windows);
#elif defined(Q_OS_LINUX)
    QCOMPARE(hostPlatform(), Platform::Linux);
#else
    QCOMPARE(hostPlatform(), Platform::Unsupported);
#endif
    const auto wrong = selectRelease(release("v2.3.0", Platform::Linux, false, false,
                                             QStringLiteral("https://evil.example/file")),
                                      "2.1.0", Platform::Linux, Architecture::X86_64);
    QVERIFY(!wrong.valid);
    QVERIFY(!selectRelease(QJsonObject(), "2.1.0", Platform::Linux, Architecture::X86_64).valid);
}

void HarborUpdaterTest::digestIsRequiredAndMustMatch()
{
    QJsonObject missing = release("v2.3.0");
    QJsonArray assets = missing.value("assets").toArray();
    QJsonObject package = assets.at(0).toObject();
    package.remove("digest");
    assets[0] = package;
    missing["assets"] = assets;
    QVERIFY(!selectRelease(missing, "2.1.0", Platform::Linux, Architecture::X86_64).valid);

    QJsonObject malformed = release("v2.3.0");
    assets = malformed.value("assets").toArray();
    package = assets.at(0).toObject();
    package["digest"] = "sha256:xyz";
    assets[0] = package;
    malformed["assets"] = assets;
    QVERIFY(!selectRelease(malformed, "2.1.0", Platform::Linux, Architecture::X86_64).valid);

    const QByteArray data("PK\x03\x04payload", 11);
    QTemporaryDir dir;
    const QString path = dir.filePath("package.zip");
    QFile file(path);
    QVERIFY(file.open(QIODevice::WriteOnly));
    QVERIFY(file.write(data) == data.size());
    file.close();
    Manifest manifest;
    manifest.assetName = "harbor-windows-x86_64.zip";
    manifest.declaredSize = data.size();
    manifest.githubDigest = "sha256:" + QString::fromLatin1(hashFor(data));
    auto bad = validateDownloadedPackage(path, manifest, QString::fromLatin1(hashFor(data)),
                                         QString::fromLatin1(hashFor(data)));
    QVERIFY(bad.valid);
    manifest.githubDigest = "sha256:" + QString(64, 'b');
    QVERIFY(!validateDownloadedPackage(path, manifest, QString::fromLatin1(hashFor(data)),
                                       QString::fromLatin1(hashFor(data))).valid);

    QJsonObject zeroSize = release("v2.3.0");
    assets = zeroSize.value("assets").toArray();
    package = assets.at(0).toObject();
    package["size"] = 0;
    assets[0] = package;
    zeroSize["assets"] = assets;
    QVERIFY(!selectRelease(zeroSize, "2.1.0", Platform::Linux, Architecture::X86_64).valid);
    QJsonObject hugeSize = release("v2.3.0");
    assets = hugeSize.value("assets").toArray();
    package = assets.at(0).toObject();
    package["size"] = 3LL * 1024 * 1024 * 1024;
    assets[0] = package;
    hugeSize["assets"] = assets;
    QVERIFY(!selectRelease(hugeSize, "2.1.0", Platform::Linux, Architecture::X86_64).valid);
    QJsonObject fractionalSize = release("v2.3.0");
    assets = fractionalSize.value("assets").toArray();
    package = assets.at(0).toObject();
    package["size"] = 1.5;
    assets[0] = package;
    fractionalSize["assets"] = assets;
    QVERIFY(!selectRelease(fractionalSize, "2.1.0", Platform::Linux, Architecture::X86_64).valid);

#ifdef Q_OS_UNIX
    const QString linkPath = dir.filePath("package-link.zip");
    QVERIFY(QFile::link(path, linkPath));
    QVERIFY(!validateDownloadedPackage(linkPath, manifest, QString::fromLatin1(hashFor(data)),
                                        QString::fromLatin1(hashFor(data))).valid);
#endif
}

void HarborUpdaterTest::packageValidation_data()
{
    QTest::addColumn<bool>("zip");
    QTest::addColumn<bool>("exists");
    QTest::addColumn<bool>("valid");
    QTest::newRow("zip") << true << true << true;
    QTest::newRow("gzip") << false << true << true;
    QTest::newRow("empty") << true << false << false;
    QTest::newRow("wrong-magic") << true << true << false;
}

void HarborUpdaterTest::packageValidation()
{
    QFETCH(bool, zip);
    QFETCH(bool, exists);
    QFETCH(bool, valid);
    QTemporaryDir dir;
    const QByteArray data = !valid && exists
        ? QByteArray("not-an-archive")
        : (zip ? QByteArray("PK\x03\x04payload", 11)
               : QByteArray("\x1f\x8b\x08payload", 10));
    const QString path = dir.filePath(zip ? QStringLiteral("package.zip") : QStringLiteral("package.tar.gz"));
    if (exists) {
        QFile file(path);
        QVERIFY(file.open(QIODevice::WriteOnly));
        file.write(data);
    }
    Manifest manifest;
    manifest.assetName = zip ? QStringLiteral("harbor-windows-x86_64.zip")
                             : QStringLiteral("harbor-linux-x86_64.tar.gz");
    manifest.declaredSize = exists ? data.size() : 1;
    manifest.githubDigest = QStringLiteral("sha256:") + QString::fromLatin1(hashFor(data));
    const auto result = validateDownloadedPackage(path, manifest,
                                                  exists ? QString::fromLatin1(hashFor(data)) : QString(64, QLatin1Char('a')),
                                                  exists ? QString::fromLatin1(hashFor(data)) : QString());
    QCOMPARE(result.valid, valid);
}

void HarborUpdaterTest::updaterFailureAndSuccessPaths()
{
    const Platform platform = testPlatform();
    const Architecture architecture = hostArchitecture();
    const QByteArray package = validPackage(platform);
    const QString name = expectedAssetName(platform, architecture);
    const QString path = QStandardPaths::writableLocation(QStandardPaths::CacheLocation)
        + QStringLiteral("/harbor-updates/") + name;
    enum class Mode { MetadataOverflow, PackageNetwork, PackageOverflow, Short, Long, Write, Commit,
                     ChecksumNetwork, ChecksumOverflow, Malformed, Success };
    const QList<Mode> modes{Mode::MetadataOverflow, Mode::PackageNetwork, Mode::PackageOverflow, Mode::Short,
                            Mode::Long, Mode::Write, Mode::Commit, Mode::ChecksumNetwork,
                            Mode::ChecksumOverflow, Mode::Malformed, Mode::Success};
    for (const Mode mode : modes) {
        const char *modeName = mode == Mode::MetadataOverflow ? "metadata-overflow"
            : mode == Mode::PackageNetwork ? "package-network"
            : mode == Mode::PackageOverflow ? "package-overflow"
            : mode == Mode::Short ? "declared-short"
            : mode == Mode::Long ? "declared-long"
            : mode == Mode::Write ? "package-write"
            : mode == Mode::Commit ? "savefile-commit"
            : mode == Mode::ChecksumNetwork ? "checksum-network"
            : mode == Mode::ChecksumOverflow ? "checksum-overflow"
            : mode == Mode::Malformed ? "malformed-checksum" : "successful-validation";
        qInfo().noquote() << "[Updater test]" << modeName;
        QFile::remove(path);
        int applyCalls = 0;
        QString launchedProgram;
        QStringList launchedArgs;
        HarborUpdater::Dependencies deps;
        deps.launch = [&applyCalls, &launchedProgram, &launchedArgs](const QString &program,
                                                                       const QStringList &args, qint64) {
            ++applyCalls;
            launchedProgram = program;
            launchedArgs = args;
            return false;
        };
        if (mode == Mode::Write || mode == Mode::Commit) {
            deps.packageFile = [mode](const QString &) -> std::unique_ptr<HarborUpdaterPackageFile> {
                return std::make_unique<TestPackageFile>(mode == Mode::Write, mode == Mode::Commit);
            };
        }
        ReplyObservations observations;
        deps.get = [mode, package, name, platform, architecture, &observations](const QNetworkRequest &request) {
            const QString url = request.url().toString();
            if (url.contains(QStringLiteral("api.github.com"))) {
                QByteArray body = QJsonDocument(updaterRelease(package, platform, architecture))
                    .toJson(QJsonDocument::Compact);
                if (mode == Mode::MetadataOverflow)
                    body.append(QByteArray(1024 * 1024 + 1, 'x'));
                return static_cast<QNetworkReply *>(new FakeReply(request.url(), body,
                    FakeReply::Kind::Metadata, &observations));
            }
            if (url.endsWith(QStringLiteral(".sha256"))) {
                if (mode == Mode::ChecksumNetwork)
                    return static_cast<QNetworkReply *>(new FakeReply(request.url(), {}, FakeReply::Kind::Checksum,
                        &observations, QNetworkReply::RemoteHostClosedError));
                if (mode == Mode::ChecksumOverflow)
                    return static_cast<QNetworkReply *>(new FakeReply(request.url(), QByteArray(4097, 'x'),
                        FakeReply::Kind::Checksum, &observations));
                if (mode == Mode::Malformed)
                    return static_cast<QNetworkReply *>(new FakeReply(request.url(), "not-a-checksum",
                        FakeReply::Kind::Checksum, &observations));
                return static_cast<QNetworkReply *>(new FakeReply(request.url(),
                    hashFor(package) + "  " + name.toLatin1() + "\n", FakeReply::Kind::Checksum,
                    &observations));
            }
            if (url.endsWith(QStringLiteral(".update.json")))
                return static_cast<QNetworkReply *>(new FakeReply(request.url(), "{}",
                    FakeReply::Kind::Manifest, &observations));
            QByteArray body = package;
            if (mode == Mode::PackageOverflow || mode == Mode::Long)
                body.append('x');
            if (mode == Mode::Short)
                body.chop(1);
            const auto error = mode == Mode::PackageNetwork
                ? QNetworkReply::RemoteHostClosedError : QNetworkReply::NoError;
            return static_cast<QNetworkReply *>(new FakeReply(request.url(), body, FakeReply::Kind::Package,
                &observations, error));
        };
        HarborUpdater updater(nullptr, std::move(deps));
        updater.setCallActive(true);
        updater.checkForUpdates();
        QTRY_VERIFY_WITH_TIMEOUT(updater.status() == QStringLiteral("error")
                                     || updater.status() == QStringLiteral("ready"), 1000);
        QCOMPARE(observations.metadataBuffer, qint64(1024 * 1024));
        if (mode == Mode::MetadataOverflow) {
            QCOMPARE(updater.status(), QStringLiteral("error"));
            QCOMPARE(updater.errorKey(), QStringLiteral("update.error.invalidMetadata"));
            QVERIFY(observations.metadataAborted);
            QVERIFY(!QFile::exists(path));
            QCOMPARE(applyCalls, 0);
        } else if (mode == Mode::Success) {
            QCOMPARE(observations.packageBuffer, qint64(64 * 1024));
            QCOMPARE(observations.checksumBuffer, qint64(4097));
            QCOMPARE(updater.status(), QStringLiteral("ready"));
            QVERIFY(QFile::exists(path));
            QCOMPARE(applyCalls, 0);
            QVERIFY(updater.waitingForCall());
            updater.setCallActive(false);
            QCOMPARE(applyCalls, 1);
            QVERIFY(launchedProgram.endsWith(QStringLiteral("harbor-update-helper"))
                    || launchedProgram.endsWith(QStringLiteral("harbor-update-helper.exe")));
            QVERIFY(!launchedArgs.contains(QStringLiteral("--apply-update")));
            QVERIFY(launchedArgs.contains(QStringLiteral("--transaction-dir")));
            QVERIFY(!launchedArgs.contains(QStringLiteral("--secret")));
        } else {
            if (mode == Mode::PackageNetwork || mode == Mode::PackageOverflow
                || mode == Mode::Short || mode == Mode::Long || mode == Mode::Write || mode == Mode::Commit) {
                QCOMPARE(observations.packageBuffer, qint64(64 * 1024));
                if (mode == Mode::PackageOverflow)
                    QVERIFY(observations.packageAborted);
            }
            if (mode == Mode::ChecksumNetwork || mode == Mode::ChecksumOverflow || mode == Mode::Malformed) {
                QCOMPARE(observations.packageBuffer, qint64(64 * 1024));
                QCOMPARE(observations.checksumBuffer, qint64(4097));
                if (mode == Mode::ChecksumOverflow)
                    QVERIFY(observations.checksumAborted);
            }
            QCOMPARE(updater.status(), QStringLiteral("error"));
            QVERIFY(!QFile::exists(path));
            QCOMPARE(applyCalls, 0);
        }
        QFile::remove(path);
    }
}

QTEST_MAIN(HarborUpdaterTest)
#include "HarborUpdaterTest.moc"
