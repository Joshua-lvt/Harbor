#include "HarborNotifications.h"

#ifdef HARBOR_HAVE_DBUS
#include <QDBusConnection>
#include <QDBusConnectionInterface>
#include <QDBusMessage>
#include <QDBusPendingCall>
#include <QVariantMap>
#endif

HarborNotifications::HarborNotifications(QObject *parent)
    : QObject(parent)
{
#ifdef Q_OS_WIN
    // No D-Bus on Windows: the tray balloon (wired in main()) is the
    // desktop path, and it exists wherever a tray does.
    m_available = true;
#else
#ifdef HARBOR_HAVE_DBUS
    // A session bus without a notification daemon is still "unavailable":
    // only a registered org.freedesktop.Notifications counts.
    QDBusConnection bus = QDBusConnection::sessionBus();
    m_available = bus.isConnected()
        && bus.interface()->isServiceRegistered(
               QStringLiteral("org.freedesktop.Notifications"));
#endif
#endif
}

bool HarborNotifications::available() const
{
    return m_available;
}

void HarborNotifications::notify(const QString &title, const QString &body,
                                 const QString &category)
{
    if (!m_available)
        return;

#ifdef Q_OS_WIN
    Q_UNUSED(category);
    // The tray owns the only icon balloons can anchor to; main() forwards
    // this to HarborTray::showNotification, which no-ops while the icon
    // is hidden. Bodies stay generic by QML contract (no message text).
    emit fallbackRequested(title, body);
#else
#ifdef HARBOR_HAVE_DBUS
    QDBusMessage message = QDBusMessage::createMethodCall(
        QStringLiteral("org.freedesktop.Notifications"),
        QStringLiteral("/org/freedesktop/Notifications"),
        QStringLiteral("org.freedesktop.Notifications"),
        QStringLiteral("Notify"));
    message.setArguments({
        QStringLiteral("Harbor"),          // app_name
        uint32_t(0),                       // replaces_id
        QString(),                         // app_icon (desktop file supplies it)
        title,                             // summary
        body,                              // body
        QStringList(),                     // actions
        QVariantMap{
            {QStringLiteral("category"), category},
            {QStringLiteral("desktop-entry"), QStringLiteral("harbor")},
        },
        int32_t(-1),                       // expire_timeout: daemon default
    });
    // Fire and forget: a failed notification must never disturb the call or
    // transfer that produced it. The daemon's own errors surface there.
    QDBusConnection::sessionBus().asyncCall(message);
#else
    Q_UNUSED(title);
    Q_UNUSED(body);
    Q_UNUSED(category);
#endif
#endif
}
