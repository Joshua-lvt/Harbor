#pragma once

#include <QHash>
#include <QJsonObject>
#include <QObject>
#include <QTimer>

/// Native presence detector: the only place in Harbor that may look at
/// platform activity APIs (logind, desktop idle monitors, MPRIS, the OS
/// input stack). On each cadence tick it assembles one private
/// UserActivitySnapshot and hands it to the facade, which pushes it into
/// the Rust core's presence machine. The snapshot is internal forever —
/// only the committed ONLINE/AWAY/OFFLINE aggregate is ever displayed or
/// published, and raw evidence (idle seconds, media titles, lock timing)
/// never leaves this process.
///
/// Every signal is optional: an API that answers nothing is omitted from
/// the snapshot, and the core treats absence as no evidence — state is
/// never invented here.
class HarborPresenceSource final : public QObject
{
    Q_OBJECT

public:
    explicit HarborPresenceSource(QObject *parent = nullptr);

    void start();
    void stop();

signals:
    /// One private multi-signal observation, camelCase per the local
    /// protocol: inputIdleSeconds (seconds), screenLocked (bool),
    /// sessionActive (bool), media ("playing" | "paused" | "stopped").
    /// Fields the platform could not observe are absent, not null.
    void snapshotReady(const QJsonObject &snapshot);

private slots:
    void poll();

private:
    QJsonObject pollLinux();
    QJsonObject pollWindows();

    /// logind session state for this user. Fills whichever of
    /// sessionActive / screenLocked the daemon answers.
    void pollLogind(QJsonObject &snapshot);
    /// Desktop idle monitor: Mutter on GNOME, the freedesktop screensaver
    /// interface elsewhere. Seconds since the last input event.
    void pollInputIdle(QJsonObject &snapshot);
    /// MPRIS2 observation: any player whose playback position actually
    /// advances reads "playing"; a player claiming Playing with a frozen
    /// position is a playlist somebody left behind, so it reads "paused".
    void pollMedia(QJsonObject &snapshot);

    QTimer m_timer;
    /// Cached logind session object path for this user.
    QString m_logindSessionPath;
    bool m_logindResolved = false;
    /// MPRIS position bookkeeping per player bus name, for the advancing
    /// check. Absence of a previous position means "unknown, assume real".
    QHash<QString, qlonglong> m_mediaPositions;
    QString m_mediaState;
};
