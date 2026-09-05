// Unit tests for the mandatory updater's pure policy: version ordering and
// release-asset selection. No network is touched; the GitHub channel shape
// is replayed from literals.
#include "HarborUpdater.h"

#include <QJsonArray>
#include <QJsonObject>
#include <QTest>

class HarborUpdaterTest final : public QObject
{
    Q_OBJECT

private slots:
    void versionsOrderNumerically();
    void versionsTolerateLeadingVAndTails();
    void assetPickSelectsThisPlatform();
    void assetPickRefusesUnknownReleases();
};

void HarborUpdaterTest::versionsOrderNumerically()
{
    QCOMPARE(HarborUpdater::compareVersions("2.1.0", "2.1.0"), 0);
    QVERIFY(HarborUpdater::compareVersions("2.1.0", "2.0.9") > 0);
    QVERIFY(HarborUpdater::compareVersions("2.1.0", "2.10.0") < 0);
    QVERIFY(HarborUpdater::compareVersions("2.1", "2.1.0") == 0);
    QVERIFY(HarborUpdater::compareVersions("10.0.0", "9.9.9") > 0);
}

void HarborUpdaterTest::versionsTolerateLeadingVAndTails()
{
    QCOMPARE(HarborUpdater::compareVersions("v2.1.0", "2.1.0"), 0);
    // A pre-release sorts below its bare release, never above it.
    QVERIFY(HarborUpdater::compareVersions("2.1.0-rc1", "2.1.0") < 0);
    QVERIFY(HarborUpdater::compareVersions("2.1.0", "2.1.0-rc1") > 0);
    QVERIFY(HarborUpdater::compareVersions("2.1.0-a", "2.1.0-b") < 0);
}

namespace {

QJsonObject releaseWith(const QStringList &names)
{
    QJsonArray assets;
    for (const QString &name : names) {
        QJsonObject asset;
        asset.insert(QStringLiteral("name"), name);
        asset.insert(QStringLiteral("browser_download_url"),
                     QStringLiteral("https://example.invalid/") + name);
        assets.append(asset);
    }
    QJsonObject release;
    release.insert(QStringLiteral("tag_name"), QStringLiteral("v9.9.9"));
    release.insert(QStringLiteral("assets"), assets);
    return release;
}

} // namespace

void HarborUpdaterTest::assetPickSelectsThisPlatform()
{
#ifdef Q_OS_WIN
    const QString wanted = QStringLiteral("harbor-windows-x86_64.zip");
#else
    const QString wanted = QStringLiteral("harbor-linux-x86_64.tar.gz");
#endif
    const QJsonObject picked = HarborUpdater::pickAsset(
        releaseWith({wanted, wanted + QStringLiteral(".sha256"),
                     QStringLiteral("harbor-android-arm64-debug.apk")}));
    QCOMPARE(picked.value(QStringLiteral("url")).toString(),
             QStringLiteral("https://example.invalid/") + wanted);
    QVERIFY(picked.value(QStringLiteral("shaUrl")).toString().endsWith(QStringLiteral(".sha256")));
    // A missing checksum degrades to unsigned, never to a wrong file.
    const QJsonObject unsignedPick =
        HarborUpdater::pickAsset(releaseWith({wanted}));
    QCOMPARE(unsignedPick.value(QStringLiteral("url")).toString(),
             QStringLiteral("https://example.invalid/") + wanted);
    QVERIFY(unsignedPick.value(QStringLiteral("shaUrl")).toString().isEmpty());
}

void HarborUpdaterTest::assetPickRefusesUnknownReleases()
{
    QVERIFY(HarborUpdater::pickAsset(releaseWith({})).isEmpty());
    QVERIFY(HarborUpdater::pickAsset(releaseWith({QStringLiteral("harbor-android-arm64-debug.apk")})).isEmpty());
    QVERIFY(HarborUpdater::pickAsset(QJsonObject()).isEmpty());
}

QTEST_MAIN(HarborUpdaterTest)
#include "HarborUpdaterTest.moc"
