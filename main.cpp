#include "native/HarborAutostart.h"
#include "native/HarborCoreSupervisor.h"
#include "native/HarborFacade.h"
#include "native/HarborTray.h"
#include "native/HarborNotifications.h"
#include "native/HarborSounds.h"
#include "native/HarborUpdater.h"

#include <QApplication>
#include <QFile>
#include <QFileInfo>
#include <QMessageAuthenticationCode>
#include <QDebug>
#include <QDir>
#include <QIcon>
#include <QJsonDocument>
#include <QJsonObject>
#include <QRegularExpression>
#include <QSaveFile>
#include <QStandardPaths>
#include <QTimer>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQmlError>
#ifdef Q_OS_UNIX
#include <unistd.h>
#endif

namespace {
bool canonicalTransaction(const QString &value)
{
    static const QRegularExpression pattern(
        QStringLiteral("^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$"));
    return pattern.match(value).hasMatch();
}

bool privateTransactionDirectory(const QString &candidate, const QString &transaction, QString *canonical)
{
    const QString root = QDir::cleanPath(QStandardPaths::writableLocation(QStandardPaths::CacheLocation)
                                         + QStringLiteral("/harbor-updater"));
    const QFileInfo rootInfo(root);
    const QFileInfo txInfo(candidate);
    if (!rootInfo.isDir() || rootInfo.isSymLink() || !txInfo.isDir() || txInfo.isSymLink()) return false;
    const QString rootCanonical = rootInfo.canonicalFilePath();
    const QString txCanonical = txInfo.canonicalFilePath();
    if (rootCanonical.isEmpty() || txCanonical.isEmpty()
        || rootCanonical != QDir::cleanPath(root)
        || txCanonical != QDir::cleanPath(candidate)
        || QFileInfo(txCanonical).absolutePath() != rootCanonical
        || QFileInfo(txCanonical).fileName() != transaction) return false;
#ifdef Q_OS_UNIX
    const auto privatePermissions = [](const QFileInfo &info) {
        const auto permissions = info.permissions();
        return !(permissions & (QFileDevice::ReadGroup | QFileDevice::WriteGroup
                                | QFileDevice::ExeGroup | QFileDevice::ReadOther
                                | QFileDevice::WriteOther | QFileDevice::ExeOther));
    };
    if (!privatePermissions(rootInfo) || !privatePermissions(txInfo)) return false;
    if (rootInfo.ownerId() != uint(::geteuid()) || txInfo.ownerId() != uint(::geteuid())) return false;
#elif defined(Q_OS_WIN)
    if (!harborWindowsPrivatePath(root) || !harborWindowsPrivatePath(candidate)) return false;
#else
    return false;
#endif
    QString current = rootCanonical;
    const QString relative = QDir(rootCanonical).relativeFilePath(txCanonical);
    for (const QString &part : relative.split(QDir::separator(), Qt::SkipEmptyParts)) {
        current += QDir::separator() + part;
        const QFileInfo info(current);
        if (info.isSymLink()) return false;
    }
    if (canonical) *canonical = txCanonical;
    return true;
}
}

int main(int argc, char *argv[])
{
    // QApplication rather than QGuiApplication: the real system-tray adapter
    // owns menus, which live in QtWidgets. The QML surface is unchanged.
    QApplication app(argc, argv);
    app.setApplicationName("Harbor");
    app.setOrganizationName("Harbor");
    // Window/taskbar icon. The exe file icon itself is embedded via
    // packaging/harbor.rc on Windows; this covers the taskbar button,
    // Alt+Tab, and the (custom-drawn) title bar on every platform.
    // Same bundled PNG the tray adapter falls back to.
    app.setWindowIcon(QIcon(QStringLiteral(":/qt/qml/Harbor/images/harbor.png")));

    HarborCoreSupervisor coreSupervisor;
    HarborFacade harborCore(&coreSupervisor);
    HarborTray systemTray;
    HarborAutostart autostart;
    HarborNotifications notifications;
    HarborSounds sounds;
    HarborUpdater updater;
    const QString transaction = qEnvironmentVariable("HARBOR_HEALTH_TRANSACTION");
    const QString expectedVersion = qEnvironmentVariable("HARBOR_HEALTH_EXPECTED_VERSION");
    const QString transactionDir = qEnvironmentVariable("HARBOR_HEALTH_TRANSACTION_DIR");

    // The stored preference is the single source of truth for start-with-
    // system; every change (core load or UI edit) re-applies the OS fact.
    QObject::connect(harborCore.settings(), &HarborSettings::startWithSystemChanged,
                     &autostart, [&harborCore, &autostart]() {
                        autostart.setEnabled(harborCore.settings()->startWithSystem());
                    });
    QObject::connect(&harborCore, &HarborFacade::coreReadyChanged, &autostart,
                     [&harborCore, &autostart]() {
                        if (harborCore.coreReady())
                            autostart.setEnabled(harborCore.settings()->startWithSystem());
                    });

    // An explicit quit — tray menu, preview flyout, or shortcut — ends the
    // whole tree: aboutToQuit shuts the core down, which tears down the call,
    // the share and its worker, and marks the session offline.
    QObject::connect(&systemTray, &HarborTray::quitRequested, &app,
                     &QCoreApplication::quit);
    // Windows has no D-Bus: desktop notifications arrive here from the
    // adapter and ride the tray balloon, which no-ops while hidden.
    QObject::connect(&notifications,
                     &HarborNotifications::fallbackRequested, &systemTray,
                     &HarborTray::showNotification);
    QObject::connect(&app, &QCoreApplication::aboutToQuit,
                     &harborCore, &HarborFacade::shutdownCore);

    QQmlApplicationEngine engine;
    engine.rootContext()->setContextProperty("HarborCore", &harborCore);
    engine.rootContext()->setContextProperty("HarborTray", &systemTray);
    engine.rootContext()->setContextProperty("HarborAutostart", &autostart);
    engine.rootContext()->setContextProperty("HarborNotifications", &notifications);
    engine.rootContext()->setContextProperty("HarborSounds", &sounds);
    engine.rootContext()->setContextProperty("HarborUpdater", &updater);
    QObject::connect(&engine, &QQmlApplicationEngine::warnings,
                     &app, [](const QList<QQmlError> &warnings) {
        for (const auto &warning : warnings)
            qCritical().noquote() << warning.toString();
    });
    QObject::connect(&engine, &QQmlApplicationEngine::objectCreationFailed,
                     &app, []() {
        qCritical() << "Harbor QML root object could not be created.";
        QCoreApplication::exit(-1);
    }, Qt::QueuedConnection);
    coreSupervisor.start();
    engine.loadFromModule("Harbor", "Main");

    // The helper accepts a marker only after both the QML root and the real
    // core are ready. The token is never logged and QSaveFile prevents a
    // partially written marker from being mistaken for health.
    QString canonicalTransactionDir;
    const QString secretPath = transactionDir + QStringLiteral("/health.secret");
    const QString markerPath = transactionDir + QStringLiteral("/health.json");
    if (!transaction.isEmpty() && !expectedVersion.isEmpty()
        && privateTransactionDirectory(transactionDir, transaction, &canonicalTransactionDir)
        && canonicalTransaction(transaction)) {
        if (canonicalTransactionDir == QDir::cleanPath(transactionDir)) {
            auto publishHealth = [&]() {
                if (!harborCore.coreReady() || engine.rootObjects().isEmpty()) return;
                if (QStringLiteral(HARBOR_VERSION_STRING) != expectedVersion) return;
                const QFileInfo secretInfo(secretPath);
                if (!secretInfo.isFile() || secretInfo.isSymLink()) return;
#ifdef Q_OS_UNIX
                if (secretInfo.permissions() & (QFileDevice::ReadGroup | QFileDevice::WriteGroup
                                                | QFileDevice::ExeGroup | QFileDevice::ReadOther
                                                | QFileDevice::WriteOther | QFileDevice::ExeOther)) return;
#elif defined(Q_OS_WIN)
                if (!harborWindowsPrivatePath(secretPath)) return;
#else
                return;
#endif
                QFile secret(secretPath);
                if (!secret.open(QIODevice::ReadOnly)) return;
                const QByteArray key = secret.read(64);
                if (key.size() != 32) return;
                QMessageAuthenticationCode hmac(QCryptographicHash::Sha256, key);
                hmac.addData("harbor-health-v1\n");
                hmac.addData(expectedVersion.toUtf8());
                hmac.addData("\n");
                hmac.addData(transaction.toUtf8());
                QSaveFile marker(markerPath);
                if (!marker.open(QIODevice::WriteOnly)) return;
                const QByteArray body = QJsonDocument(QJsonObject{{QStringLiteral("transaction"), transaction},
                                                        {QStringLiteral("version"), expectedVersion},
                                                        {QStringLiteral("hmac"), QString::fromLatin1(hmac.result().toHex())},
                                                        {QStringLiteral("ready"), true}}).toJson(QJsonDocument::Compact);
                if (marker.write(body) != body.size() || !marker.commit()) return;
            };
            auto *healthTimer = new QTimer(&app);
            healthTimer->setInterval(50);
            QObject::connect(healthTimer, &QTimer::timeout, &app, publishHealth);
            healthTimer->start();
        }
    }

    return app.exec();
}
