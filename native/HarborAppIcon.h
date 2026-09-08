#pragma once

#include <QHash>
#include <QObject>
#include <QString>

/// Resolves a real program icon for an activity entry without new
/// dependencies (QtCore + QtGui only, so unit tests keep linking).
///
/// The Rust core only sends theme-safe keys (`app_id`, `icon`): never
/// absolute paths, pids, or command lines. This provider turns those keys
/// into a small PNG data URL the QML `Image` can render:
/// - Linux: `QIcon::fromTheme(iconKey)` → `QIcon::fromTheme(appId)` →
///   `.desktop` absolute-path fallback, scaled to 48px.
/// - Windows: locates `<appId>.exe` via `PATH` + well-known folders and
///   extracts its first icon via the system icon API, scaled to 32px.
///
/// Empty string means "unknown": QML falls back to the category glyph.
class HarborAppIconProvider final : public QObject
{
    Q_OBJECT
public:
    explicit HarborAppIconProvider(QObject *parent = nullptr);

    Q_INVOKABLE QString iconUrl(const QString &appId, const QString &iconKey) const;

private:
    QString resolveLinux(const QString &appId, const QString &iconKey) const;
    QString resolveWindows(const QString &appId) const;
    static QString pixmapToDataUrl(const QPixmap &pixmap);
    static QString sanitizedKey(const QString &raw);

    mutable QHash<QString, QString> m_cache;
};
