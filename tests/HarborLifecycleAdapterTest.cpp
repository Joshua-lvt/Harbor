#include "HarborAutostart.h"
#include "HarborTray.h"

#include <QDir>
#include <QFile>
#include <QTest>
#include <QUuid>

class HarborLifecycleAdapterTest final : public QObject
{
    Q_OBJECT

private slots:
    void autostartEntryFollowsTheStoredPreference();
    void trayNeverClaimsAnUnavailableSession();
};

void HarborLifecycleAdapterTest::autostartEntryFollowsTheStoredPreference()
{
#ifdef Q_OS_UNIX
    const QString configHome = QDir::tempPath() + QStringLiteral("/harbor-lifecycle-%1")
                                                  .arg(QUuid::createUuid().toString(QUuid::WithoutBraces));
    QVERIFY(QDir().mkpath(configHome));
    qputenv("XDG_CONFIG_HOME", configHome.toUtf8());

    HarborAutostart autostart;
    QVERIFY(HarborAutostart::supported());
    QVERIFY(!autostart.enabled());

    QVERIFY(autostart.setEnabled(true));
    QVERIFY(autostart.enabled());
    const QString entry = configHome + QStringLiteral("/autostart/harbor.desktop");
    QVERIFY(QFile::exists(entry));
    QFile written(entry);
    QVERIFY(written.open(QIODevice::ReadOnly));
    const QString contents = QString::fromUtf8(written.readAll());
    QVERIFY(contents.contains(QLatin1String("[Desktop Entry]")));
    QVERIFY(contents.contains(QLatin1String("Exec=")));

    QVERIFY(autostart.setEnabled(false));
    QVERIFY(!autostart.enabled());
    QVERIFY(!QFile::exists(entry));

    QDir(configHome).removeRecursively();
#else
    // Windows writes a Startup-folder shortcut instead of a desktop entry;
    // APPDATA is redirected so the test never touches the real profile.
    const QString appData = QDir::tempPath() + QStringLiteral("/harbor-lifecycle-%1")
                                                  .arg(QUuid::createUuid().toString(QUuid::WithoutBraces));
    QVERIFY(QDir().mkpath(appData));
    qputenv("APPDATA", appData.toUtf8());

    HarborAutostart autostart;
    QVERIFY(HarborAutostart::supported());
    QVERIFY(!autostart.enabled());

    QVERIFY(autostart.setEnabled(true));
    QVERIFY(autostart.enabled());
    QVERIFY(QFile::exists(appData + QStringLiteral("/Microsoft/Windows/Start Menu/Programs/Startup/Harbor.lnk")));

    QVERIFY(autostart.setEnabled(false));
    QVERIFY(!autostart.enabled());

    QDir(appData).removeRecursively();
#endif
}

void HarborLifecycleAdapterTest::trayNeverClaimsAnUnavailableSession()
{
    HarborTray tray;
    // Under the offscreen test platform no system tray exists. The adapter
    // must report that honestly and never flip `active` to true, so the QML
    // close policy treats close-to-tray as a real quit.
    if (!tray.available()) {
        tray.setActive(true);
        QVERIFY(!tray.active());
    } else {
        tray.setActive(true);
        QVERIFY(tray.active());
        tray.setActive(false);
        QVERIFY(!tray.active());
    }
}

QTEST_MAIN(HarborLifecycleAdapterTest)
#include "HarborLifecycleAdapterTest.moc"
