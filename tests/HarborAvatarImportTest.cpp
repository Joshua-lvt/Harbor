#include "HarborCoreSupervisor.h"
#include "HarborFacade.h"

#include <QBuffer>
#include <QByteArray>
#include <QFile>
#include <QImage>
#include <QImageReader>
#include <QRandomGenerator>
#include <QTemporaryDir>
#include <QTest>
#include <QUrl>

// The facade's avatar importer is the only path a user-provided image takes
// into the persistent profile. These tests pin its contract: local files
// only, bounded size and dimensions, decoded as a real image, and emitted as
// a data URI no wider than 512px — never a local path or remote URL. Still
// images are normalized to PNG; a valid GIF keeps its original bytes (and
// its animation) as data:image/gif;base64.
class HarborAvatarImportTest final : public QObject
{
    Q_OBJECT

    QTemporaryDir m_dir;

    void writeImage(const QString &name, const QImage &image,
                    const char *format, QString &url) const
    {
        const QString path = m_dir.filePath(name);
        QVERIFY2(image.save(path, format), qPrintable(path));
        url = QUrl::fromLocalFile(path).toString();
    }

    void writeJunk(const QString &name, qint64 bytes, QString &url) const
    {
        const QString path = m_dir.filePath(name);
        QFile file(path);
        QVERIFY(file.open(QIODevice::WriteOnly));
        QByteArray chunk(4096, '\xA7');
        while (bytes > 0) {
            const qint64 take = qMin<qint64>(bytes, chunk.size());
            file.write(chunk.constData(), take);
            bytes -= take;
        }
        file.close();
        url = QUrl::fromLocalFile(path).toString();
    }

    static QImage gradient(int width, int height)
    {
        QImage image(width, height, QImage::Format_RGBA8888);
        for (int y = 0; y < height; ++y)
            for (int x = 0; x < width; ++x)
                image.setPixelColor(x, y, QColor(x % 256, y % 256, (x + y) % 256));
        return image;
    }

    // QVERIFY expands to a bare `return;`, so helpers that use it must be
    // void and hand the result back through an out-parameter.
    static void decodeDataUri(const QString &dataUri, QImage &decoded)
    {
        const QString prefix = QStringLiteral("data:image/png;base64,");
        QVERIFY2(dataUri.startsWith(prefix), qPrintable(dataUri.left(40)));
        const QByteArray encoded = dataUri.mid(prefix.size()).toLatin1();
        QVERIFY(encoded.size() > 0);
        // The Rust validator rejects any whitespace inside the base64 body.
        QVERIFY(!encoded.contains(' '));
        QVERIFY(!encoded.contains('\n'));
        decoded = QImage::fromData(QByteArray::fromBase64(encoded));
        QVERIFY2(!decoded.isNull(), "the payload must decode back to an image");
    }

private slots:
    void initTestCase()
    {
        QVERIFY2(m_dir.isValid(), "a temporary directory is required");
    }

    void rejectsNonLocalUrls();
    void rejectsMissingAndEmptyFiles();
    void rejectsOversizedSourceFile();
    void rejectsHugeDimensions();
    void rejectsUndecodableContent();
    void importsPngAndDownscales();
    void importsJpegAsPng();
    void preservesGifAnimationWhenSupported();
    void rejectsCorruptGif();
};

void HarborAvatarImportTest::rejectsNonLocalUrls()
{
    HarborCoreSupervisor supervisor;
    HarborFacade facade(&supervisor);

    QCOMPARE(facade.importProfileAvatar(QUrl("https://example.com/avatar.png")),
             QString());
    QCOMPARE(facade.importProfileAvatar(QUrl("data:image/png;base64,AA==")),
             QString());
    QCOMPARE(facade.importProfileAvatar(QUrl()), QString());
}

void HarborAvatarImportTest::rejectsMissingAndEmptyFiles()
{
    HarborCoreSupervisor supervisor;
    HarborFacade facade(&supervisor);

    QCOMPARE(facade.importProfileAvatar(
                 QUrl::fromLocalFile(m_dir.filePath("absent.png"))),
             QString());

    const QString empty = m_dir.filePath("empty.png");
    QFile emptyFile(empty);
    QVERIFY(emptyFile.open(QIODevice::WriteOnly));
    emptyFile.close();
    QCOMPARE(facade.importProfileAvatar(QUrl::fromLocalFile(empty)), QString());
}

void HarborAvatarImportTest::rejectsOversizedSourceFile()
{
    HarborCoreSupervisor supervisor;
    HarborFacade facade(&supervisor);

    // A valid image whose encoded file exceeds the 8 MiB source bound.
    // Real entropy: a formula-based pattern is periodic and PNG's filters
    // would compress it far under the bound.
    QImage noise(2000, 2000, QImage::Format_RGBA8888);
    QRandomGenerator rng(20260902);
    for (int y = 0; y < noise.height(); ++y) {
        uchar *line = noise.scanLine(y);
        for (int x = 0; x < noise.width() * 4; x += 4) {
            const quint32 v = rng.generate();
            line[x] = uchar(v);
            line[x + 1] = uchar(v >> 8);
            line[x + 2] = uchar(v >> 16);
            line[x + 3] = 0xFF;
        }
    }
    QString url;
    writeImage("oversized-source.png", noise, "PNG", url);
    QFileInfo info(QUrl(url).toLocalFile());
    QVERIFY2(info.size() > 8 * 1024 * 1024,
             "fixture must actually exceed the source bound");

    QCOMPARE(facade.importProfileAvatar(QUrl(url)), QString());
}

void HarborAvatarImportTest::rejectsHugeDimensions()
{
    HarborCoreSupervisor supervisor;
    HarborFacade facade(&supervisor);

    // Valid content, small file, but beyond the 4096px dimension bound.
    QString url;
    writeImage("huge-dimensions.png", gradient(4200, 10), "PNG", url);
    QCOMPARE(facade.importProfileAvatar(QUrl(url)), QString());
}

void HarborAvatarImportTest::rejectsUndecodableContent()
{
    HarborCoreSupervisor supervisor;
    HarborFacade facade(&supervisor);

    QString junkUrl;
    writeJunk("junk.png", 100 * 1024, junkUrl);
    QCOMPARE(facade.importProfileAvatar(QUrl(junkUrl)), QString());
}

void HarborAvatarImportTest::importsPngAndDownscales()
{
    HarborCoreSupervisor supervisor;
    HarborFacade facade(&supervisor);

    QString url;
    writeImage("big.png", gradient(2000, 1000), "PNG", url);
    const QString dataUri = facade.importProfileAvatar(QUrl(url));
    QVERIFY(!dataUri.isEmpty());

    QImage decoded;
    decodeDataUri(dataUri, decoded);
    QVERIFY(decoded.width() <= 512);
    QVERIFY(decoded.height() <= 512);
    // KeepAspectRatio: a 2:1 source keeps its 2:1 shape inside the box.
    QCOMPARE(decoded.width(), 512);
    QCOMPARE(decoded.height(), 256);
}

void HarborAvatarImportTest::importsJpegAsPng()
{
    HarborCoreSupervisor supervisor;
    HarborFacade facade(&supervisor);

    QString url;
    writeImage("small.jpg", gradient(300, 300), "JPEG", url);
    QImage decoded;
    decodeDataUri(facade.importProfileAvatar(QUrl(url)), decoded);
    QCOMPARE(decoded.width(), 300);
    QCOMPARE(decoded.height(), 300);
}

void HarborAvatarImportTest::preservesGifAnimationWhenSupported()
{
    if (!QImageReader::supportedMimeTypes().contains("image/gif"))
        QSKIP("the gif plugin is not available in this environment");

    HarborCoreSupervisor supervisor;
    HarborFacade facade(&supervisor);

    // Qt's gif plugin reads but does not write, so the fixture is a known
    // 1x1 GIF embedded as base64.
    const QByteArray gifBytes = QByteArray::fromBase64(
        "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7");
    const QString path = m_dir.filePath("frame.gif");
    QFile gifFile(path);
    QVERIFY(gifFile.open(QIODevice::WriteOnly));
    QCOMPARE(gifFile.write(gifBytes), gifBytes.size());
    gifFile.close();

    // The original bytes (and their animation) survive the import: the data
    // URI carries the GIF itself, never a re-encoded still.
    const QString dataUri =
        facade.importProfileAvatar(QUrl::fromLocalFile(path));
    const QString prefix = QStringLiteral("data:image/gif;base64,");
    QVERIFY2(dataUri.startsWith(prefix), qPrintable(dataUri.left(40)));
    const QByteArray roundTripped =
        QByteArray::fromBase64(dataUri.mid(prefix.size()).toLatin1());
    QCOMPARE(roundTripped, gifBytes);
    QImageReader probe(path);
    QCOMPARE(probe.imageCount(), 1);
}

void HarborAvatarImportTest::rejectsCorruptGif()
{
    if (!QImageReader::supportedMimeTypes().contains("image/gif"))
        QSKIP("the gif plugin is not available in this environment");

    HarborCoreSupervisor supervisor;
    HarborFacade facade(&supervisor);

    const QString path = m_dir.filePath("broken.gif");
    QFile gifFile(path);
    QVERIFY(gifFile.open(QIODevice::WriteOnly));
    gifFile.write("GIF89a-not-an-image");
    gifFile.close();

    QCOMPARE(facade.importProfileAvatar(QUrl::fromLocalFile(path)),
             QString());
}

QTEST_MAIN(HarborAvatarImportTest)
#include "HarborAvatarImportTest.moc"
