#include "HarborCoreSupervisor.h"
#include "HarborFacade.h"

#include <QTest>
#include <QUuid>

class HarborCoreSupervisorTest final : public QObject
{
    Q_OBJECT

private slots:
    void startsAndNegotiatesTheLocalCore();
    void exposesTheRealDeviceIdentity();
    void exposesSanitizedLocalActivity();
    void persistsSettingsAcrossCoreRestarts();
};

void HarborCoreSupervisorTest::startsAndNegotiatesTheLocalCore()
{
    HarborCoreSupervisor supervisor;
    HarborFacade facade(&supervisor);

    supervisor.start();
    QTRY_VERIFY_WITH_TIMEOUT(facade.coreReady(), 3000);
    QCOMPARE(facade.coreState(), QStringLiteral("running"));
    QVERIFY(facade.coreErrorKey().isEmpty());

    facade.shutdownCore();
    QTRY_COMPARE_WITH_TIMEOUT(facade.coreState(), QStringLiteral("stopped"), 3000);
}

void HarborCoreSupervisorTest::exposesTheRealDeviceIdentity()
{
    HarborCoreSupervisor supervisor;
    HarborFacade facade(&supervisor);

    supervisor.start();
    QTRY_VERIFY_WITH_TIMEOUT(facade.coreReady(), 3000);
    QTRY_VERIFY_WITH_TIMEOUT(facade.identityAvailable(), 3000);
    QVERIFY2(facade.identityHarborId().startsWith(QLatin1String("harbor-")),
             "the core mints a friendly harbor id");
    QVERIFY(!QUuid(facade.identityDeviceId()).isNull());
    QVERIFY(!facade.identityPublicKey().isEmpty());

    facade.shutdownCore();
    QTRY_COMPARE_WITH_TIMEOUT(facade.identityAvailable(), false, 3000);
}

void HarborCoreSupervisorTest::exposesSanitizedLocalActivity()
{
    HarborCoreSupervisor supervisor;
    HarborFacade facade(&supervisor);

    supervisor.start();
    QTRY_VERIFY_WITH_TIMEOUT(facade.coreReady(), 3000);
    // The Linux monitor scans every two seconds; unlike a fixture, its first
    // real scan emits a running snapshot through the core and typed facade.
    QTRY_COMPARE_WITH_TIMEOUT(facade.activityMonitorState(), QStringLiteral("running"), 5000);
    QTRY_VERIFY_WITH_TIMEOUT(!facade.activityTimeline().isEmpty(), 5000);

    for (const QVariant &value : facade.activityTimeline()) {
        const QVariantMap entry = value.toMap();
        QVERIFY(entry.contains(QStringLiteral("id")));
        QVERIFY(entry.contains(QStringLiteral("titleKey")));
        QVERIFY(!entry.contains(QStringLiteral("pid")));
        QVERIFY(!entry.contains(QStringLiteral("exePath")));
        QVERIFY(!entry.contains(QStringLiteral("commandLine")));
        QVERIFY(!entry.contains(QStringLiteral("path")));
    }

    facade.shutdownCore();
    QTRY_COMPARE_WITH_TIMEOUT(facade.coreState(), QStringLiteral("stopped"), 3000);
}

void HarborCoreSupervisorTest::persistsSettingsAcrossCoreRestarts()
{
    {
        HarborCoreSupervisor supervisor;
        HarborFacade facade(&supervisor);

        supervisor.start();
        QTRY_VERIFY_WITH_TIMEOUT(facade.coreReady(), 3000);
        QTRY_VERIFY_WITH_TIMEOUT(facade.settings()->loaded(), 3000);

        int documentsApplied = 0;
        connect(facade.settings(), &HarborSettings::documentApplied,
                this, [&documentsApplied] { ++documentsApplied; });

        facade.settings()->setAppearanceMode(QStringLiteral("light"));
        facade.settings()->setDisplayName(QStringLiteral("Ari"));
        facade.settings()->setAvatar(QStringLiteral("data:image/png;base64,AA=="));
        facade.settings()->setAvatarType(QStringLiteral("image"));
        QCOMPARE(facade.settings()->appearanceMode(), QStringLiteral("light"));
        QCOMPARE(facade.settings()->displayName(), QStringLiteral("Ari"));
        QCOMPARE(facade.settings()->avatarType(), QStringLiteral("image"));
        // Wait for the settings.update round trip: the core's authoritative
        // echo is applied back onto the mirror.
        QTRY_VERIFY_WITH_TIMEOUT(documentsApplied >= 3, 3000);

        facade.shutdownCore();
        QTRY_COMPARE_WITH_TIMEOUT(facade.coreState(), QStringLiteral("stopped"), 3000);
    }

    HarborCoreSupervisor supervisor;
    HarborFacade facade(&supervisor);
    supervisor.start();
    QTRY_VERIFY_WITH_TIMEOUT(facade.coreReady(), 3000);
    QTRY_VERIFY_WITH_TIMEOUT(facade.settings()->loaded(), 3000);
    QCOMPARE(facade.settings()->appearanceMode(), QStringLiteral("light"));
    QCOMPARE(facade.settings()->displayName(), QStringLiteral("Ari"));
    QCOMPARE(facade.settings()->avatar(), QStringLiteral("data:image/png;base64,AA=="));
    QCOMPARE(facade.settings()->avatarType(), QStringLiteral("image"));

    // Restore the durable default so the sandbox stays predictable.
    bool restored = false;
    connect(facade.settings(), &HarborSettings::documentApplied,
            this, [&restored] { restored = true; });
    facade.settings()->setAppearanceMode(QStringLiteral("dark"));
    facade.settings()->setDisplayName(QString());
    facade.settings()->setAvatar(QString());
    facade.settings()->setAvatarType(QStringLiteral("image"));
    QTRY_VERIFY_WITH_TIMEOUT(restored, 3000);
    facade.shutdownCore();
}

QTEST_MAIN(HarborCoreSupervisorTest)
#include "HarborCoreSupervisorTest.moc"
