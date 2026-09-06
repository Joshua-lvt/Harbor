#include "native/HarborAutostart.h"
#include "native/HarborCoreSupervisor.h"
#include "native/HarborFacade.h"
#include "native/HarborTray.h"
#include "native/HarborNotifications.h"
#include "native/HarborSounds.h"
#include "native/HarborTailnet.h"
#include "native/HarborUpdater.h">

#include <QApplication>
#include <QDebug>
#include <QDir>
#include <QFileInfo>
#include <QIcon>
#include <QProcess>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QQmlError>
#include <QThread>
#include <QTimer>

namespace {

// Detached self-relaunch used by the mandatory updater: the old process is
// already quitting when this runs. Wait briefly for it to exit, swap the
// verified payload over the install dir (with backups), and start the new
// build. Returns the process exit code; anything nonzero leaves the old
// tree restored and logs the reason.
int applyUpdatePackage(const QString &packagePath)
{
    const QString appDir = QCoreApplication::applicationDirPath();
#ifdef Q_OS_WIN
    const QStringList binaries{QStringLiteral("harbor.exe"), QStringLiteral("harbor-core.exe"),
                               QStringLiteral("harbor-media.exe")};
    const QStringList extractArgs{QStringLiteral("-xf"), packagePath, QStringLiteral("-C")};
#else
    const QStringList binaries{QStringLiteral("harbor"), QStringLiteral("harbor-core"),
                               QStringLiteral("harbor-media")};
    const QStringList extractArgs{QStringLiteral("-xzf"), packagePath, QStringLiteral("-C")};
#endif
    const QString pendingDir = appDir + QStringLiteral("/.harbor-update-pending");
    const QString backupDir = appDir + QStringLiteral("/.harbor-update-backup");
    QDir().mkpath(pendingDir);
    QDir().mkpath(backupDir);

    // The parent quits the moment it spawns us; still, give file locks a
    // moment to release (Windows holds the running image open).
    QThread::sleep(6);

    auto fail = [&](const QString &reason) {
        qWarning() << "Harbor update failed:" << reason;
        return 1;
    };

    // bsdtar ships with Windows 10+ and handles both .zip and .tar.gz.
    QProcess tar;
#ifdef Q_OS_WIN
    tar.setProgram(QStringLiteral("tar.exe"));
#else
    tar.setProgram(QStringLiteral("tar"));
#endif
    tar.setArguments(extractArgs + QStringList{pendingDir});
    tar.start();
    if (!tar.waitForFinished(120000) || tar.exitCode() != 0)
        return fail(QStringLiteral("extract: ") + QString::fromUtf8(tar.readAllStandardError()));

    for (const QString &binary : binaries) {
        const QFileInfo staged(pendingDir + QLatin1Char('/') + binary);
        if (!staged.exists() || !staged.isFile())
            return fail(QStringLiteral("package missing ") + binary);
    }
    for (const QString &binary : binaries) {
        const QString live = appDir + QLatin1Char('/') + binary;
        const QString staged = pendingDir + QLatin1Char('/') + binary;
        const QString backup = backupDir + QLatin1Char('/') + binary;
        QFile::remove(backup);
        if (QFileInfo::exists(live) && !QFile::rename(live, backup))
            return fail(QStringLiteral("backup ") + binary);
        if (!QFile::copy(staged, live)) {
            QFile::rename(backup, live); // best-effort restore
            return fail(QStringLiteral("install ") + binary);
        }
        QFile::setPermissions(live, QFileDevice::ReadOwner | QFileDevice::WriteOwner
                                        | QFileDevice::ExeOwner | QFileDevice::ReadGroup
                                        | QFileDevice::ExeGroup | QFileDevice::ReadOther
                                        | QFileDevice::ExeOther);
    }
    QDir(pendingDir).removeRecursively();

    const QString program =
#ifdef Q_OS_WIN
        appDir + QStringLiteral("/harbor.exe");
#else
        appDir + QStringLiteral("/harbor");
#endif
    if (!QProcess::startDetached(program, {}))
        return fail(QStringLiteral("relaunch"));
    return 0;
}

} // namespace

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

    const QStringList args = QCoreApplication::arguments();
    const int applyIndex = args.indexOf(QStringLiteral("--apply-update"));
    if (applyIndex >= 0 && applyIndex + 1 < args.size())
        return applyUpdatePackage(args.at(applyIndex + 1));

    HarborCoreSupervisor coreSupervisor;
    HarborFacade harborCore(&coreSupervisor);
    HarborTailnet tailnet;
    HarborTray systemTray;
    HarborAutostart autostart;
    HarborNotifications notifications;
    HarborSounds sounds;
    HarborUpdater updater;

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
    engine.rootContext()->setContextProperty("HarborTailnet", &tailnet);
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
    // Join the Harbor Tailnet after first paint: the first join on a fresh
    // machine can take seconds, and pairing (the only consumer) happens
    // strictly after the user interacts.
    QTimer::singleShot(0, &tailnet, &HarborTailnet::ensureJoined);

    return app.exec();
}
