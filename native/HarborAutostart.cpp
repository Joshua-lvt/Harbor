#include "HarborAutostart.h"

#include <QCoreApplication>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QStandardPaths>

#ifdef Q_OS_UNIX
#include <QSaveFile>
#endif

#ifdef Q_OS_WIN
#include <windows.h>
#include <shlguid.h>
#include <shobjidl.h>
#endif

namespace {
QString autostartDirectory()
{
    QString configHome = qEnvironmentVariable("XDG_CONFIG_HOME");
    if (configHome.isEmpty())
        configHome = QDir::homePath() + QStringLiteral("/.config");
    return configHome + QStringLiteral("/autostart");
}

QString entryPath()
{
    return autostartDirectory() + QStringLiteral("/harbor.desktop");
}

QString desktopEntry()
{
    // The path is resolved from the install layout at runtime: the deployed
    // layout puts every binary in <prefix>/bin. Development runs fall back
    // to the applicationDirPath the same way.
    const QString binDir = QCoreApplication::applicationDirPath();
    return QStringLiteral("[Desktop Entry]\n"
                          "Type=Application\n"
                          "Name=Harbor\n"
                          "Comment=Harbor\n"
                          "Exec=%1/harbor\n"
                          "Icon=harbor\n"
                          "Terminal=false\n"
                          "Categories=Network;InstantMessaging;\n")
        .arg(binDir);
}

#ifdef Q_OS_WIN
namespace {
// Start-with-system on Windows is a shortcut in the per-user Startup
// folder (no registry writes, no admin rights): the shell launches it at
// logon exactly like the XDG entry on Linux.
QString windowsStartupFolder()
{
    QString appData = qEnvironmentVariable("APPDATA");
    if (appData.isEmpty())
        appData = QDir::homePath() + QStringLiteral("/AppData/Roaming");
    return appData
        + QStringLiteral("/Microsoft/Windows/Start Menu/Programs/Startup");
}

QString windowsLinkPath()
{
    return windowsStartupFolder() + QStringLiteral("/Harbor.lnk");
}

bool writeWindowsShortcut()
{
    const QString target = QDir::toNativeSeparators(
        QCoreApplication::applicationFilePath());
    const QString workDir = QDir::toNativeSeparators(
        QCoreApplication::applicationDirPath());
    const QString link = QDir::toNativeSeparators(windowsLinkPath());
    if (target.isEmpty() || !QDir().mkpath(QFileInfo(link).absolutePath()))
        return false;
    const HRESULT init = CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED);
    const bool uninit = SUCCEEDED(init);
    IShellLinkW *shellLink = nullptr;
    HRESULT result = CoCreateInstance(CLSID_ShellLink, nullptr,
                                      CLSCTX_INPROC_SERVER, IID_IShellLinkW,
                                      reinterpret_cast<void **>(&shellLink));
    if (FAILED(result) || shellLink == nullptr) {
        if (uninit)
            CoUninitialize();
        return false;
    }
    shellLink->SetPath(reinterpret_cast<LPCWSTR>(target.utf16()));
    shellLink->SetWorkingDirectory(reinterpret_cast<LPCWSTR>(workDir.utf16()));
    shellLink->SetDescription(L"Harbor");
    IPersistFile *persist = nullptr;
    result = shellLink->QueryInterface(
        IID_IPersistFile, reinterpret_cast<void **>(&persist));
    shellLink->Release();
    if (FAILED(result) || persist == nullptr) {
        if (uninit)
            CoUninitialize();
        return false;
    }
    result = persist->Save(reinterpret_cast<LPCOLESTR>(link.utf16()), TRUE);
    persist->Release();
    if (uninit)
        CoUninitialize();
    return SUCCEEDED(result);
}
} // namespace
#endif
} // namespace

HarborAutostart::HarborAutostart(QObject *parent)
    : QObject(parent)
{
}

bool HarborAutostart::supported()
{
#if defined(Q_OS_UNIX) || defined(Q_OS_WIN)
    return true;
#else
    return false;
#endif
}

bool HarborAutostart::enabled() const
{
    return entryExists();
}

bool HarborAutostart::entryExists() const
{
#ifdef Q_OS_UNIX
    return QFile::exists(entryPath());
#else
#ifdef Q_OS_WIN
    return QFile::exists(windowsLinkPath());
#else
    return false;
#endif
#endif
}

bool HarborAutostart::setEnabled(bool enabled)
{
#ifdef Q_OS_UNIX
    if (enabled) {
        const QString directory = autostartDirectory();
        if (!QDir().mkpath(directory))
            return false;
        // One atomic replace: a crash mid-write can never leave a truncated
        // desktop entry behind.
        QSaveFile file(entryPath());
        if (!file.open(QIODevice::WriteOnly))
            return false;
        file.write(desktopEntry().toUtf8());
        if (!file.commit())
            return false;
    } else {
        if (!QFile::remove(entryPath()) && QFile::exists(entryPath()))
            return false;
    }
    emit enabledChanged();
    return true;
#else
#ifdef Q_OS_WIN
    if (enabled) {
        if (!writeWindowsShortcut())
            return false;
    } else {
        if (!QFile::remove(windowsLinkPath())
            && QFile::exists(windowsLinkPath()))
            return false;
    }
    emit enabledChanged();
    return true;
#else
    Q_UNUSED(enabled);
    return false;
#endif
#endif
}
