#pragma once

#include <QObject>
#include <QString>

/// Desktop notification adapter: the freedesktop session bus on Linux, the
/// system-tray balloon (through HarborTray, which owns the tray icon) on
/// Windows.
///
/// Pure adapter, no policy: which events deserve a notification, the user's
/// settings, and the localized texts are decided in the UI layer, which
/// calls notify() through the typed context property. QML never touches
/// D-Bus itself. Without a reachable service (no session bus on Linux)
/// available() is false and notify() is an honest no-op — the in-app
/// surfaces keep working, nothing pretends to have notified.
class HarborNotifications : public QObject
{
    Q_OBJECT

    /// Whether a desktop notification service is actually reachable.
    Q_PROPERTY(bool available READ available CONSTANT FINAL)

public:
    explicit HarborNotifications(QObject *parent = nullptr);

    bool available() const;

    /// Posts one desktop notification. category follows the freedesktop
    /// conventions ("im.call", "im.received", "transfer.complete") so the
    /// desktop daemon may rate-limit or group; it carries no private data.
    /// On Windows the call is forwarded as fallbackRequested, which main()
    /// wires to the tray balloon — QSystemTrayIcon stays confined to
    /// HarborTray, this adapter never touches it.
    Q_INVOKABLE void notify(const QString &title, const QString &body,
                            const QString &category);

signals:
    /// Emitted instead of a native post where the only path is the tray
    /// balloon (Windows). main() connects it to HarborTray::showNotification.
    void fallbackRequested(const QString &title, const QString &body);

private:
    bool m_available = false;
};
