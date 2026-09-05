#include "HarborAppIcon.h"

#include <QBuffer>
#include <QDir>
#include <QFile>
#include <QIcon>
#include <QImage>
#include <QImageReader>
#include <QPixmap>
#include <QStandardPaths>

#ifdef Q_OS_WIN
#include <windows.h>
#include <shellapi.h>
#endif

namespace {

constexpr int LINUX_ICON_PX = 48;
constexpr int WINDOWS_ICON_PX = 32;
constexpr int MAX_CACHE_ENTRIES = 256;

QString dataUrlFromImage(QImage image, int targetPx)
{
    if (image.isNull())
        return {};
    if (image.width() > targetPx || image.height() > targetPx)
        image = image.scaled(targetPx, targetPx, Qt::KeepAspectRatio,
                             Qt::SmoothTransformation);
    QByteArray encoded;
    QBuffer buffer(&encoded);
    if (!buffer.open(QIODevice::WriteOnly))
        return {};
    if (!image.save(&buffer, "PNG"))
        return {};
    // Cap at ~100 KiB PNG so a pathological theme icon cannot bloat QML.
    if (encoded.size() > 100 * 1024)
        return {};
    return QStringLiteral("data:image/png;base64,")
        + QString::fromLatin1(encoded.toBase64());
}

#ifdef Q_OS_WIN
QString dataUrlFromExe(const QString &exePath)
{
    const std::wstring path = exePath.toStdWString();
    HICON largeIcon = nullptr;
    HICON smallIcon = nullptr;
    const UINT extracted = ExtractIconExW(path.c_str(), 0, &largeIcon, &smallIcon, 1);
    HICON handle = largeIcon ? largeIcon : smallIcon;
    if (extracted == 0 || !handle) {
        if (largeIcon)
            DestroyIcon(largeIcon);
        if (smallIcon && smallIcon != largeIcon)
            DestroyIcon(smallIcon);
        return {};
    }
    const QImage image = QImage::fromHICON(handle);
    if (largeIcon)
        DestroyIcon(largeIcon);
    if (smallIcon && smallIcon != largeIcon)
        DestroyIcon(smallIcon);
    if (image.isNull())
        return {};
    return dataUrlFromImage(image, WINDOWS_ICON_PX);
}

QString windowsExecutableForApp(const QString &appId)
{
    if (appId.isEmpty())
        return {};
    // Absolute or relative path passed through (defensive: the core only
    // sends basenames, but never trust the boundary).
    if (appId.contains(QLatin1Char('/')) || appId.contains(QLatin1Char('\\'))) {
        const QFileInfo info(appId);
        if (info.isFile() && info.isReadable())
            return QDir::toNativeSeparators(info.absoluteFilePath());
        return {};
    }
    const QString exeName = appId.endsWith(QStringLiteral(".exe"), Qt::CaseInsensitive)
        ? appId
        : appId + QStringLiteral(".exe");
    // 1. PATH (covers System32, app dirs, scoop/choco shims).
    const QString onPath = QStandardPaths::findExecutable(
        exeName.left(exeName.size() - 4));
    if (!onPath.isEmpty())
        return QDir::toNativeSeparators(onPath);
    const QString onPathExe = QStandardPaths::findExecutable(appId);
    if (!onPathExe.isEmpty())
        return QDir::toNativeSeparators(onPathExe);
    // 2. Well-known install roots (no registry lookup here: the source
    //    contracts keep persistent-settings APIs out of adapters, and env
    //    plus fixed roots cover the common cases).
    const wchar_t *roots[] = {
        L"C:\\Program Files\\",
        L"C:\\Program Files (x86)\\",
        L"C:\\Windows\\System32\\",
        L"C:\\Windows\\",
    };
    for (const wchar_t *root : roots) {
        const QString candidate =
            QString::fromWCharArray(root) + exeName;
        if (QFile::exists(candidate))
            return QDir::toNativeSeparators(candidate);
    }
    return {};
}
#endif

#ifndef Q_OS_WIN
// Best-effort absolute-path fallback for Linux when the theme has no
// entry: re-read the matching `.desktop` `Icon=` absolute path and load
// the file directly. Mirrors the Rust lookup dirs (XDG + fallbacks).
QString desktopAbsoluteIcon(const QString &appId)
{
    QStringList bases;
    const QString xdgHome =
        QString::fromLocal8Bit(qgetenv("XDG_DATA_HOME"));
    if (!xdgHome.isEmpty())
        bases << xdgHome;
    const QString xdgDirs =
        QString::fromLocal8Bit(qgetenv("XDG_DATA_DIRS"));
    for (const QString &part : xdgDirs.split(QLatin1Char(':')))
        if (!part.isEmpty())
            bases << part;
    const QString home =
        QStandardPaths::writableLocation(QStandardPaths::HomeLocation);
    if (!home.isEmpty())
        bases << home + QStringLiteral("/.local/share");
    bases << QStringLiteral("/usr/share") << QStringLiteral("/usr/local/share")
          << QStringLiteral("/var/lib/flatpak/exports/share")
          << QStringLiteral("/var/lib/snapd/desktop");
    const QString wanted = appId.toLower();
    for (const QString &base : bases) {
        const QDir apps(base + QStringLiteral("/applications"));
        if (!apps.exists())
            continue;
        const QStringList files =
            apps.entryList(QStringList() << QStringLiteral("*.desktop"),
                           QDir::Files | QDir::Readable, QDir::Name);
        int scanned = 0;
        for (const QString &file : files) {
            if (++scanned > 400)
                break;
            QFile f(apps.absoluteFilePath(file));
            if (!f.open(QIODevice::ReadOnly | QIODevice::Text))
                continue;
            const QByteArray bytes = f.read(32 * 1024);
            const QString text = QString::fromUtf8(bytes);
            bool inEntry = false;
            bool execMatch = false;
            QString icon;
            for (const QString &rawLine : text.split(QLatin1Char('\n'))) {
                const QString line = rawLine.trimmed();
                if (line.isEmpty() || line.startsWith(QLatin1Char('#')))
                    continue;
                if (line.startsWith(QLatin1Char('['))) {
                    inEntry = (line == QStringLiteral("[Desktop Entry]"));
                    continue;
                }
                if (!inEntry)
                    continue;
                if (line.startsWith(QStringLiteral("Exec="))) {
                    QString exec = line.mid(5).trimmed();
                    // First usable token, skipping env wrappers/args.
                    QString candidate;
                    for (const QString &tok :
                         exec.split(QLatin1Char(' '), Qt::SkipEmptyParts)) {
                        QString t = tok;
                        t.remove(QLatin1Char('"')).remove(QLatin1Char('\''));
                        if (t.isEmpty() || t.contains(QLatin1Char('='))
                            || t.startsWith(QLatin1Char('%'))
                            || t.startsWith(QLatin1Char('-')))
                            continue;
                        candidate = t;
                        break;
                    }
                    if (candidate.isEmpty())
                        continue;
                    QString baseName = candidate;
                    const int slash = qMax(baseName.lastIndexOf(QLatin1Char('/')),
                                           baseName.lastIndexOf(QLatin1Char('\\')));
                    if (slash >= 0)
                        baseName = baseName.mid(slash + 1);
                    QString lower = baseName.toLower();
                    for (const QString &suffix :
                         {QStringLiteral(".exe"), QStringLiteral(".bin")}) {
                        if (lower.endsWith(suffix)) {
                            lower.chop(suffix.size());
                            break;
                        }
                    }
                    if (lower == wanted)
                        execMatch = true;
                } else if (line.startsWith(QStringLiteral("Icon="))) {
                    icon = line.mid(5).trimmed();
                }
                if (execMatch && !icon.isEmpty())
                    break;
            }
            if (execMatch && !icon.isEmpty()
                && (icon.contains(QLatin1Char('/')))) {
                if (QFile::exists(icon))
                    return icon;
            }
        }
    }
    return {};
}
#endif

} // namespace

HarborAppIconProvider::HarborAppIconProvider(QObject *parent)
    : QObject(parent)
{
}

QString HarborAppIconProvider::sanitizedKey(const QString &raw)
{
    QString out;
    out.reserve(raw.size());
    for (const QChar c : raw.trimmed()) {
        const ushort u = c.unicode();
        const bool ok = (u >= 'a' && u <= 'z') || (u >= 'A' && u <= 'Z')
            || (u >= '0' && u <= '9') || c == QLatin1Char('-')
            || c == QLatin1Char('_') || c == QLatin1Char('.')
            || c == QLatin1Char('+');
        if (ok)
            out.append(c);
    }
    // Strip a raster/vector suffix when a full filename slipped through.
    QString lower = out.toLower();
    for (const QString &suffix :
         {QStringLiteral(".png"), QStringLiteral(".svg"),
          QStringLiteral(".xpm"), QStringLiteral(".ico")}) {
        if (lower.endsWith(suffix)) {
            out.chop(suffix.size());
            break;
        }
    }
    out = out.trimmed();
    while (out.startsWith(QLatin1Char('.')))
        out.remove(0, 1);
    if (out.isEmpty() || out.size() > 64)
        return {};
    return out;
}

QString HarborAppIconProvider::pixmapToDataUrl(const QPixmap &pixmap)
{
    if (pixmap.isNull())
        return {};
    QImage image = pixmap.toImage();
    return dataUrlFromImage(image, LINUX_ICON_PX);
}

QString HarborAppIconProvider::iconUrl(const QString &appId,
                                       const QString &iconKey) const
{
    const QString cleanApp = sanitizedKey(appId);
    const QString cleanIcon = sanitizedKey(iconKey);
    if (cleanApp.isEmpty() && cleanIcon.isEmpty())
        return {};
    const QString cacheKey = cleanIcon.isEmpty()
        ? cleanApp
        : cleanIcon + QLatin1Char('|') + cleanApp;
    auto cached = m_cache.constFind(cacheKey);
    if (cached != m_cache.constEnd())
        return cached.value();
    QString resolved;
#ifdef Q_OS_WIN
    Q_UNUSED(iconKey);
    resolved = resolveWindows(cleanApp);
#else
    resolved = resolveLinux(cleanApp, cleanIcon);
#endif
    if (m_cache.size() >= MAX_CACHE_ENTRIES)
        m_cache.clear();
    m_cache.insert(cacheKey, resolved);
    return resolved;
}

QString HarborAppIconProvider::resolveLinux(const QString &appId,
                                            const QString &iconKey) const
{
    // 1–2. Theme lookup: explicit icon key first, then the app id itself
    // (`firefox` resolves without any .desktop parse on most systems).
    for (const QString &name :
         {iconKey, appId}) {
        if (name.isEmpty())
            continue;
        if (!QIcon::hasThemeIcon(name))
            continue;
        const QIcon icon = QIcon::fromTheme(name);
        const QPixmap pixmap = icon.pixmap(LINUX_ICON_PX, LINUX_ICON_PX);
        const QString url = pixmapToDataUrl(pixmap);
        if (!url.isEmpty())
            return url;
    }
    // 3. Absolute-path fallback from the matching .desktop entry.
    if (!appId.isEmpty()) {
        const QString abs = desktopAbsoluteIcon(appId);
        if (!abs.isEmpty()) {
            QImageReader reader(abs);
            reader.setAutoTransform(true);
            QImage image = reader.read();
            const QString url = dataUrlFromImage(image, LINUX_ICON_PX);
            if (!url.isEmpty())
                return url;
        }
    }
    return {};
}

QString HarborAppIconProvider::resolveWindows(const QString &appId) const
{
#ifdef Q_OS_WIN
    if (appId.isEmpty())
        return {};
    const QString exe = windowsExecutableForApp(appId);
    if (exe.isEmpty())
        return {};
    return dataUrlFromExe(exe);
#else
    Q_UNUSED(appId);
    return {};
#endif
}
