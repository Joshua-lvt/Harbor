#include "HarborPresenceSource.h"

// Test targets link the source without QtDBus; the whole body is guarded
// below, so the includes must be too.
#ifdef HARBOR_HAVE_DBUS
#include <QDBusConnection>
#include <QDBusInterface>
#include <QDBusReply>
#endif
#include <QDateTime>

#ifdef Q_OS_WIN
#include <windows.h>
#include <wtsapi32.h>
#endif

namespace {

constexpr int POLL_INTERVAL_MS = 2000;
/// D-Bus calls are local and cheap, but a wedged daemon must never stall
/// the pump; every call gets this bounded timeout.
constexpr int DBUS_TIMEOUT_MS = 400;
constexpr QLatin1String MPRIS_NAME_PREFIX("org.mpris.MediaPlayer2");

} // namespace

HarborPresenceSource::HarborPresenceSource(QObject *parent)
    : QObject(parent)
{
    m_timer.setInterval(POLL_INTERVAL_MS);
    connect(&m_timer, &QTimer::timeout, this, &HarborPresenceSource::poll);
}

void HarborPresenceSource::start()
{
    // First facts immediately, then the cadence.
    poll();
    m_timer.start();
}

void HarborPresenceSource::stop()
{
    m_timer.stop();
    m_mediaPositions.clear();
    m_mediaState.clear();
    m_logindSessionPath.clear();
    m_logindResolved = false;
}

void HarborPresenceSource::poll()
{
#ifdef HARBOR_HAVE_DBUS
#ifdef Q_OS_WIN
    QJsonObject snapshot = pollWindows();
#else
    QJsonObject snapshot = pollLinux();
#endif
    if (!snapshot.isEmpty()) {
        emit snapshotReady(snapshot);
    }
#endif
}

#if defined(HARBOR_HAVE_DBUS) && !defined(Q_OS_WIN)

QJsonObject HarborPresenceSource::pollLinux()
{
    QJsonObject snapshot;
    pollLogind(snapshot);
    pollInputIdle(snapshot);
    pollMedia(snapshot);
    return snapshot;
}

void HarborPresenceSource::pollLogind(QJsonObject &snapshot)
{
    auto system = QDBusConnection::systemBus();
    if (!m_logindResolved) {
        // Resolve this user's graphical session once; the path is stable
        // for the login's lifetime. Re-resolving happens only after stop().
        QDBusInterface manager(QStringLiteral("org.freedesktop.login1"),
            QStringLiteral("/org/freedesktop/login1"),
            QStringLiteral("org.freedesktop.DBus.Properties"), system);
        QDBusReply<QDBusVariant> sessions = manager.call(
            QStringLiteral("Get"),
            QStringLiteral("org.freedesktop.login1.Manager"),
            QStringLiteral("Sessions"));
        // ListSessions returns the session list as a variant of (a(so)).
        if (sessions.isValid()) {
            const auto argument = sessions.value().variant().value<QDBusArgument>();
            QDBusArgument entries = argument;
            entries.beginArray();
            while (!entries.atEnd()) {
                entries.beginStructure();
                QString id;
                QDBusObjectPath path;
                entries >> id >> path;
                entries.endStructure();
                // The first listed session of this login is ours on every
                // single-seat desktop; multi-seat uses the first active one.
                m_logindSessionPath = path.path();
                break;
            }
            entries.endArray();
        }
        m_logindResolved = true;
    }
    if (m_logindSessionPath.isEmpty()) {
        return;
    }
    QDBusInterface session(QStringLiteral("org.freedesktop.login1"),
        m_logindSessionPath, QStringLiteral("org.freedesktop.DBus.Properties"),
        system);
    QDBusReply<QDBusVariant> active = session.call(QStringLiteral("Get"),
        QStringLiteral("org.freedesktop.login1.Session"),
        QStringLiteral("Active"));
    if (active.isValid()) {
        snapshot.insert(QStringLiteral("sessionActive"),
            active.value().variant().toBool());
    }
    QDBusReply<QDBusVariant> locked = session.call(QStringLiteral("Get"),
        QStringLiteral("org.freedesktop.login1.Session"),
        QStringLiteral("LockedHint"));
    if (locked.isValid()) {
        snapshot.insert(QStringLiteral("screenLocked"),
            locked.value().variant().toBool());
    } else {
        // GNOME and KDE both answer the freedesktop screensaver name; the
        // proxied GetActive is the fallback when LockedHint is missing.
        QDBusInterface screensaver(QStringLiteral("org.freedesktop.ScreenSaver"),
            QStringLiteral("/org/freedesktop/ScreenSaver"),
            QStringLiteral("org.freedesktop.DBus.Properties"),
            QDBusConnection::sessionBus());
        QDBusReply<QDBusVariant> lockState = screensaver.call(QStringLiteral("Get"),
            QStringLiteral("org.freedesktop.ScreenSaver"),
            QStringLiteral("GetActive"));
        if (lockState.isValid()) {
            snapshot.insert(QStringLiteral("screenLocked"),
                lockState.value().variant().toBool());
        }
    }
}

void HarborPresenceSource::pollInputIdle(QJsonObject &snapshot)
{
    // GNOME Wayland: Mutter's idle monitor, milliseconds since last input.
    QDBusInterface mutter(QStringLiteral("org.gnome.Mutter.IdleMonitor"),
        QStringLiteral("/org/gnome/Mutter/IdleMonitor/Core"),
        QStringLiteral("org.gnome.Mutter.IdleMonitor"),
        QDBusConnection::sessionBus());
    QDBusReply<uint> mutterIdle = mutter.call(QStringLiteral("GetIdletime"));
    if (mutterIdle.isValid()) {
        snapshot.insert(QStringLiteral("inputIdleSeconds"),
            static_cast<qint64>(mutterIdle.value() / 1000));
        return;
    }
    // KDE (X11 sessions only — the implementation refuses on Wayland):
    // seconds since last input through the screensaver interface.
    QDBusInterface screensaver(QStringLiteral("org.freedesktop.ScreenSaver"),
        QStringLiteral("/org/freedesktop/ScreenSaver"),
        QStringLiteral("org.freedesktop.ScreenSaver"),
        QDBusConnection::sessionBus());
    QDBusReply<uint> idle = screensaver.call(QStringLiteral("GetSessionIdleTime"));
    if (idle.isValid()) {
        snapshot.insert(QStringLiteral("inputIdleSeconds"),
            static_cast<qint64>(idle.value()));
    }
}

void HarborPresenceSource::pollMedia(QJsonObject &snapshot)
{
    QDBusInterface dbusDaemon(QStringLiteral("org.freedesktop.DBus"),
        QStringLiteral("/org/freedesktop/DBus"),
        QStringLiteral("org.freedesktop.DBus"),
        QDBusConnection::sessionBus());
    QDBusReply<QStringList> names = dbusDaemon.call(QStringLiteral("ListNames"));
    if (!names.isValid()) {
        return;
    }
    // One materialized copy: every `names.value()` call hands back a fresh
    // temporary, and mixing iterators from two detach-happy QLists iterates
    // straight off the end of the buffer.
    const QStringList serviceNames = names.value();

    QString best;
    // Rank: an advancing player outranks everything; then paused; then a
    // merely-present stopped player.
    static const int kMissing = 0, kStopped = 1, kPaused = 2, kPlaying = 3;
    int rank = kMissing;
    for (const QString &name : serviceNames) {
        if (!name.startsWith(MPRIS_NAME_PREFIX)) {
            continue;
        }
        QDBusInterface player(name, QStringLiteral("/org/mpris/MediaPlayer2"),
            QStringLiteral("org.freedesktop.DBus.Properties"),
            QDBusConnection::sessionBus());
        QDBusReply<QDBusVariant> status = player.call(QStringLiteral("Get"),
            QStringLiteral("org.mpris.MediaPlayer2.Player"),
            QStringLiteral("PlaybackStatus"));
        if (!status.isValid()) {
            continue;
        }
        const QString playback = status.value().variant().toString();
        if (playback == QLatin1String("Playing")) {
            // Playing is believed only while the position advances between
            // polls; a frozen player is a paused video somebody forgot.
            QDBusReply<QDBusVariant> position = player.call(QStringLiteral("Get"),
                QStringLiteral("org.mpris.MediaPlayer2.Player"),
                QStringLiteral("Position"));
            const qlonglong now = position.isValid()
                ? position.value().variant().toLongLong()
                : -1;
            const qlonglong previous = m_mediaPositions.value(name, -1);
            m_mediaPositions[name] = now;
            const bool advancing = now > previous || previous < 0;
            if (advancing && rank < kPlaying) {
                rank = kPlaying;
                best = QStringLiteral("playing");
            } else if (!advancing && rank < kPaused) {
                rank = kPaused;
                best = QStringLiteral("paused");
            }
        } else if (playback == QLatin1String("Paused")) {
            m_mediaPositions.remove(name);
            if (rank < kPaused) {
                rank = kPaused;
                best = QStringLiteral("paused");
            }
        } else if (playback == QLatin1String("Stopped")) {
            m_mediaPositions.remove(name);
            if (rank < kStopped) {
                rank = kStopped;
                best = QStringLiteral("stopped");
            }
        }
    }
    // Forget players that vanished since the last poll.
    QSet<QString> live(serviceNames.cbegin(), serviceNames.cend());
    for (auto it = m_mediaPositions.begin(); it != m_mediaPositions.end();) {
        if (!live.contains(it.key())) {
            it = m_mediaPositions.erase(it);
        } else {
            ++it;
        }
    }
    if (!best.isEmpty()) {
        snapshot.insert(QStringLiteral("media"), best);
        m_mediaState = best;
    } else {
        m_mediaState.clear();
    }
}

#endif // HARBOR_HAVE_DBUS && !Q_OS_WIN

#ifdef Q_OS_WIN

QJsonObject HarborPresenceSource::pollWindows()
{
    QJsonObject snapshot;

    // Session-specific idle: the documented wraparound-safe 32-bit delta.
    LASTINPUTINFO input{};
    input.cbSize = sizeof(input);
    if (GetLastInputInfo(&input)) {
        const DWORD elapsed = GetTickCount() - input.dwTime;
        snapshot.insert(QStringLiteral("inputIdleSeconds"),
            static_cast<qint64>(elapsed / 1000));
    }

    // Lock state for this session; Win10+ report the documented flags.
    WTS_SESSION_INFO_1W *sessions = nullptr;
    DWORD count = 0, level = 1;
    if (WTSEnumerateSessionsExW(WTS_CURRENT_SERVER_HANDLE, &level, 0,
            &sessions, &count)) {
        for (DWORD i = 0; i < count; ++i) {
            const auto &info = sessions[i];
            if (info.SessionId != WTSGetActiveConsoleSessionId()) {
                continue;
            }
            snapshot.insert(QStringLiteral("sessionActive"),
                info.State == WTSActive);
            WTSINFOEXW *details = nullptr;
            DWORD bytes = 0;
            if (WTSQuerySessionInformationW(WTS_CURRENT_SERVER_HANDLE,
                    info.SessionId, WTSSessionInfoEx,
                    reinterpret_cast<LPWSTR *>(&details), &bytes)
                && details != nullptr) {
                snapshot.insert(QStringLiteral("screenLocked"),
                    details->Data.WTSSessionInfo.SessionFlags
                        == WTS_SESSIONSTATE_LOCK);
                WTSFreeMemory(details);
            }
        }
        WTSFreeMemory(sessions);
    }
    return snapshot;
}

#endif // Q_OS_WIN
