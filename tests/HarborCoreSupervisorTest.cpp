#include "HarborCoreSupervisor.h"
#include "HarborFacade.h"

#include <QTemporaryDir>
#include <QTest>
#include <QUuid>

#include <memory>

namespace {
constexpr int CoreReadyTimeoutMs = 5000;
constexpr int IdentityTimeoutMs = 5000;
constexpr int ActivityFirstScanTimeoutMs = 5000;
constexpr int SettingsAcknowledgementTimeoutMs = 5000;
constexpr int ShutdownTimeoutMs = 4000;

class StateDirectory final
{
public:
    StateDirectory()
        : wasSet(qEnvironmentVariableIsSet("HARBOR_STATE_DIR"))
        , previous(qgetenv("HARBOR_STATE_DIR"))
    {
        if (directory.isValid())
            qputenv("HARBOR_STATE_DIR", directory.path().toUtf8());
    }

    ~StateDirectory()
    {
        if (wasSet)
            qputenv("HARBOR_STATE_DIR", previous);
        else
            qunsetenv("HARBOR_STATE_DIR");
    }

    bool isValid() const { return directory.isValid(); }

private:
    QTemporaryDir directory;
    const bool wasSet;
    const QByteArray previous;
};
} // namespace

class HarborCoreSupervisorTest final : public QObject
{
    Q_OBJECT

private slots:
    void init();
    void cleanup();
    void startsAndNegotiatesTheLocalCore();
    void exposesTheRealDeviceIdentity();
    void exposesSanitizedLocalActivity();
    void persistsSettingsAcrossCoreRestarts();
    void validatesHarborIdsWithoutTrimming();

private:
    std::unique_ptr<StateDirectory> m_stateDirectory;
};

void HarborCoreSupervisorTest::init()
{
    m_stateDirectory = std::make_unique<StateDirectory>();
    QVERIFY(m_stateDirectory->isValid());
}

void HarborCoreSupervisorTest::cleanup()
{
    m_stateDirectory.reset();
}

void HarborCoreSupervisorTest::startsAndNegotiatesTheLocalCore()
{
    HarborCoreSupervisor supervisor;
    HarborFacade facade(&supervisor);

    supervisor.start();
    QTRY_VERIFY_WITH_TIMEOUT(facade.coreReady(), CoreReadyTimeoutMs);
    QCOMPARE(facade.coreState(), QStringLiteral("running"));
    QVERIFY(facade.coreErrorKey().isEmpty());

    facade.shutdownCore();
    QTRY_COMPARE_WITH_TIMEOUT(facade.coreState(), QStringLiteral("stopped"), ShutdownTimeoutMs);
}

void HarborCoreSupervisorTest::validatesHarborIdsWithoutTrimming()
{
    QVERIFY(HarborFacade::isValidHarborId(QStringLiteral("harbor-d31846b8")));
    QVERIFY(HarborFacade::isValidHarborId(QStringLiteral("harbor-ABCDEF12")));
    QVERIFY(!HarborFacade::isValidHarborId(QStringLiteral(" harbor-d31846b8")));
    QVERIFY(!HarborFacade::isValidHarborId(QStringLiteral("harbor-d31846b8 ")));
    QVERIFY(!HarborFacade::isValidHarborId(QStringLiteral("harbor-é31846b8")));
}

void HarborCoreSupervisorTest::exposesTheRealDeviceIdentity()
{
    HarborCoreSupervisor supervisor;
    HarborFacade facade(&supervisor);

    supervisor.start();
    QTRY_VERIFY_WITH_TIMEOUT(facade.coreReady(), CoreReadyTimeoutMs);
    QTRY_VERIFY_WITH_TIMEOUT(facade.identityAvailable(), IdentityTimeoutMs);
    QVERIFY2(facade.identityHarborId().startsWith(QLatin1String("harbor-")),
             "the core mints a friendly harbor id");
    QVERIFY(!QUuid(facade.identityDeviceId()).isNull());
    QVERIFY(!facade.identityPublicKey().isEmpty());

    facade.shutdownCore();
    QTRY_COMPARE_WITH_TIMEOUT(facade.identityAvailable(), false, ShutdownTimeoutMs);
}

void HarborCoreSupervisorTest::exposesSanitizedLocalActivity()
{
    HarborCoreSupervisor supervisor;
    HarborFacade facade(&supervisor);

    supervisor.start();
    QTRY_VERIFY_WITH_TIMEOUT(facade.coreReady(), CoreReadyTimeoutMs);
    // The Linux monitor scans every two seconds; unlike a fixture, its first
    // real scan emits a running snapshot through the core and typed facade.
    QTRY_COMPARE_WITH_TIMEOUT(facade.activityMonitorState(), QStringLiteral("running"),
                              ActivityFirstScanTimeoutMs);
    QTRY_VERIFY_WITH_TIMEOUT(!facade.activityTimeline().isEmpty(), ActivityFirstScanTimeoutMs);

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
    QTRY_COMPARE_WITH_TIMEOUT(facade.coreState(), QStringLiteral("stopped"), ShutdownTimeoutMs);
}

void HarborCoreSupervisorTest::persistsSettingsAcrossCoreRestarts()
{
    {
        HarborCoreSupervisor supervisor;
        HarborFacade facade(&supervisor);

        supervisor.start();
        QTRY_VERIFY_WITH_TIMEOUT(facade.coreReady(), CoreReadyTimeoutMs);
        QTRY_VERIFY_WITH_TIMEOUT(facade.settings()->loaded(), CoreReadyTimeoutMs);

        bool settingsAcknowledged = false;
        connect(facade.settings(), &HarborSettings::documentApplied,
                this, [&facade, &settingsAcknowledged] {
                    settingsAcknowledged = facade.settings()->appearanceMode() == QStringLiteral("light")
                        && facade.settings()->displayName() == QStringLiteral("Ari")
                        && facade.settings()->avatar() == QStringLiteral("data:image/png;base64,AA==")
                        && facade.settings()->avatarType() == QStringLiteral("image");
                });

        facade.settings()->setAppearanceMode(QStringLiteral("light"));
        facade.settings()->setDisplayName(QStringLiteral("Ari"));
        facade.settings()->setAvatar(QStringLiteral("data:image/png;base64,AA=="));
        facade.settings()->setAvatarType(QStringLiteral("image"));
        QCOMPARE(facade.settings()->appearanceMode(), QStringLiteral("light"));
        QCOMPARE(facade.settings()->displayName(), QStringLiteral("Ari"));
        QCOMPARE(facade.settings()->avatarType(), QStringLiteral("image"));
        // Wait for an authoritative echo containing every requested value,
        // rather than assuming how many update documents the core emits.
        QTRY_VERIFY_WITH_TIMEOUT(settingsAcknowledged, SettingsAcknowledgementTimeoutMs);

        facade.shutdownCore();
        QTRY_COMPARE_WITH_TIMEOUT(facade.coreState(), QStringLiteral("stopped"), ShutdownTimeoutMs);
    }

    HarborCoreSupervisor supervisor;
    HarborFacade facade(&supervisor);
    supervisor.start();
    QTRY_VERIFY_WITH_TIMEOUT(facade.coreReady(), CoreReadyTimeoutMs);
    QTRY_VERIFY_WITH_TIMEOUT(facade.settings()->loaded(), CoreReadyTimeoutMs);
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
    QTRY_VERIFY_WITH_TIMEOUT(restored, SettingsAcknowledgementTimeoutMs);
    facade.shutdownCore();
    QTRY_COMPARE_WITH_TIMEOUT(facade.coreState(), QStringLiteral("stopped"), ShutdownTimeoutMs);
}

QTEST_MAIN(HarborCoreSupervisorTest)
#include "HarborCoreSupervisorTest.moc"
