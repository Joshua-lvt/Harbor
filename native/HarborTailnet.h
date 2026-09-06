// HarborTailnet: zero-config join of the Harbor Tailnet on desktop.
//
// Fresh installs must pair with no manual network setup, but the control
// plane lives behind Tailscale (CGNAT makes inbound IPv4 impossible). This
// adapter detects the local Tailscale client and, when it is logged out,
// logs it in with the build-time pre-auth key — once per process launch.
//
// Threat model (read before touching this file):
// - The embedded key is extractable from the public binary (`strings`).
//   Containment, not secrecy, is the defense: the key is reusable +
//   ephemeral + tagged (`tag:harbor-client`), and the Tailnet ACL allows
//   that tag ONLY to the K11+ on 9091/tcp. Pairing itself still needs the
//   six-digit code plus explicit accept, so a leaked key buys probing, not
//   access. Rotation = new key + release; the mandatory updater retires old
//   builds when the key expires.
// - The key NEVER appears in argv (visible via `ps`), logs, QML, or IPC.
//   It is staged in a 0600 temp file passed as --auth-key=file:PATH and
//   removed immediately after use, and every captured output is scrubbed
//   for `tskey-` tokens before logging.
// - An already-connected client is NEVER touched: a personal Tailnet login
//   keeps its stable identity; only a logged-out client is joined (and only
//   then does the node become ephemeral).
#ifndef HARBORTAILNET_H
#define HARBORTAILNET_H

#include <QObject>
#include <QString>
#include <QStringList>

class HarborTailnet final : public QObject
{
    Q_OBJECT
    /// One of: "disabled" (no key embedded — dev builds), "connected",
    /// "joining" (transient), "installing" (one-time privileged client
    /// setup, Linux only), "missing" (no client binary and no known way to
    /// install it), "needs-admin" (one-time operator grant required, see
    /// runbook), "unavailable".
    Q_PROPERTY(QString status READ status NOTIFY statusChanged FINAL)

public:
    explicit HarborTailnet(QObject *parent = nullptr);

    QString status() const;

    /// Detects the local client and joins when logged out. One shot per
    /// process; safe to call on every launch. Synchronous with bounded
    /// timeouts — call after first paint (queued) on slow machines.
    Q_INVOKABLE void ensureJoined();

    /// Test seam: directory prepended to PATH when locating `tailscale`.
    void setPathOverride(const QString &directory);

    /// Default install locations searched after PATH (Windows only: the MSI
    /// does not always extend PATH, so `%ProgramFiles%\Tailscale` must be
    /// probed explicitly). Empty everywhere else. Public as a test seam.
    static QStringList defaultClientLocations();

    /// Builds the one-time privileged setup script for this distro from an
    /// /etc/os-release content snapshot, or empty when unknown. Pure and
    /// unit-tested: no process runs here. The script installs the Tailscale
    /// client, starts its daemon, and performs the first join plus the
    /// operator grant in one elevation; afterwards every launch is
    /// unprivileged. `$1` is the 0600 file holding the pre-auth key.
    static QString installScriptForOsRelease(const QString &osRelease);

signals:
    void statusChanged();

private:
    void setStatus(const QString &status);
#ifndef Q_OS_WIN
    /// One-time privileged client setup via the distro installer (pkexec):
    /// installs Tailscale, starts its daemon, joins once and grants the
    /// operator bit, all in a single elevation. True when the setup ran
    /// (the join itself is verified afterwards by the normal flow).
    bool installClient(const QString &keyFilePath);
#endif

    QString m_status = QStringLiteral("unknown");
    QString m_pathOverride;
    bool m_ensured = false;
};

#endif // HARBORTAILNET_H
