// See HarborTailnet.h for the threat model. The rules restated as code:
// no argv key, no logged key, no touched personal logins.
#include "HarborTailnet.h"

#include <QDebug>
#include <QDir>
#include <QFileInfo>
#include <QProcess>
#include <QProcessEnvironment>
#include <QRegularExpression>
#include <QStandardPaths>
#include <QTemporaryFile>

#include <algorithm>
#include <cstring>

namespace {

// `tailscale up` honors TS_AUTHKEY from its environment (the container
// convention), which keeps the credential out of /proc argv. The define
// below exists only when HARBOR_TAILSCALE_AUTHKEY was set at configure
// time (CI secret, never committed); dev builds stay inert.
#ifdef HARBOR_TAILSCALE_AUTHKEY
constexpr const char *embeddedAuthKey()
{
    return HARBOR_TAILSCALE_AUTHKEY;
}
#else
constexpr const char *embeddedAuthKey()
{
    return nullptr;
}
#endif

// Defense in depth: anything captured from the child that looks like a key
// token is redacted before it can reach logs. Matches tskey-auth-… (and any
// future tskey- flavor) plus a generous trailing run.
QString scrubKeyTokens(QString text)
{
    static const QRegularExpression token(QStringLiteral("tskey-[A-Za-z0-9_-]{1,128}"));
    return text.replace(token, QStringLiteral("tskey-<redacted>"));
}

struct CommandResult {
    int exitCode = -1;
    QString standardOutput;
    QString standardError;
    bool timedOut = false;
};

CommandResult runCommand(const QString &program, const QStringList &arguments,
                         const QProcessEnvironment &environment, int timeoutMs)
{
    QProcess child;
    child.setProcessEnvironment(environment);
    child.setProgram(program);
    child.setArguments(arguments);
    child.start();
    CommandResult result;
    if (!child.waitForStarted(timeoutMs)) {
        result.timedOut = true;
        child.kill();
        child.waitForFinished(5000);
        return result;
    }
    if (!child.waitForFinished(timeoutMs)) {
        result.timedOut = true;
        child.kill();
        child.waitForFinished(5000);
    }
    result.exitCode = child.exitCode();
    result.standardOutput = QString::fromUtf8(child.readAllStandardOutput());
    result.standardError = QString::fromUtf8(child.readAllStandardError());
    return result;
}

bool outputShowsTailnetAddress(const QString &output)
{
    // `tailscale status` prints the node's 100.x (and IPv6) addresses when
    // logged in; a logged-out client prints an error instead.
    static const QRegularExpression tailnetIp(QStringLiteral("\\b100\\.\\d{1,3}\\.\\d{1,3}\\.\\d{1,3}\\b"));
    return output.contains(tailnetIp);
}

bool outputShowsPermissionRefusal(const QString &output)
{
    const QString lowered = output.toLower();
    return lowered.contains(QStringLiteral("permission denied"))
        || lowered.contains(QStringLiteral("operator"))
        || lowered.contains(QStringLiteral("access denied"))
        || lowered.contains(QStringLiteral("must be root"));
}

QString osReleaseField(const QString &content, const QString &key)
{
    for (const QString &line : content.split(QLatin1Char('\n'))) {
        const QString trimmed = line.trimmed();
        if (!trimmed.startsWith(key + QLatin1Char('=')))
            continue;
        QString value = trimmed.mid(key.length() + 1).trimmed();
        if (value.length() >= 2 && value.startsWith(QLatin1Char('"'))
            && value.endsWith(QLatin1Char('"')))
            value = value.mid(1, value.length() - 2);
        return value.toLower();
    }
    return {};
}

} // namespace

HarborTailnet::HarborTailnet(QObject *parent)
    : QObject(parent)
{
}

QString HarborTailnet::status() const
{
    return m_status;
}

void HarborTailnet::setPathOverride(const QString &directory)
{
    m_pathOverride = directory;
}

QStringList HarborTailnet::defaultClientLocations()
{
#ifdef Q_OS_WIN
    // The MSI does not reliably extend PATH: probe the standard install
    // roots explicitly so a fresh install is found on first launch.
    QStringList locations;
    const QString programFiles = QProcessEnvironment::systemEnvironment().value(
        QStringLiteral("ProgramFiles"), QStringLiteral("C:\\Program Files"));
    const QString programFilesX86 = QProcessEnvironment::systemEnvironment().value(
        QStringLiteral("ProgramFiles(x86)"), QStringLiteral("C:\\Program Files (x86)"));
    for (const QString &root : {programFiles, programFilesX86}) {
        const QString candidate = QDir(root).filePath(QStringLiteral("Tailscale"));
        if (!locations.contains(candidate))
            locations.append(candidate);
    }
    return locations;
#else
    return {};
#endif
}

QString HarborTailnet::installScriptForOsRelease(const QString &osRelease)
{
    const QString id = osReleaseField(osRelease, QStringLiteral("ID"));
    const QStringList like = osReleaseField(osRelease, QStringLiteral("ID_LIKE"))
                                 .split(QLatin1Char(' '), Qt::SkipEmptyParts);
    const auto isFamily = [&](const QStringList &names) {
        return names.contains(id) || std::any_of(like.begin(), like.end(),
                                                  [&](const QString &token) {
                                                      return names.contains(token);
                                                  });
    };
    // The first join and the operator grant ride the same elevation: later
    // launches find a logged-in client and never elevate again. `$1` is the
    // 0600 key file staged by the caller; join failures never fail the
    // install itself (the unprivileged flow retries on every launch).
    const QString joinStep =
        QStringLiteral("tailscale up --auth-key=\"file:$1\" "
                       "--operator=\"$(id -un \"${PKEXEC_UID:-0}\")\" || true");
    if (isFamily({QStringLiteral("arch")})) {
        // tailscale ships in [extra]; the daemon is socket-activated by the
        // official unit, enabled here so reboots keep working.
        return QStringLiteral("pacman -Sy --noconfirm tailscale && "
                              "systemctl enable --now tailscaled && ")
            + joinStep;
    }
    if (isFamily({QStringLiteral("debian"), QStringLiteral("ubuntu"),
                  QStringLiteral("fedora"), QStringLiteral("rhel"),
                  QStringLiteral("centos"), QStringLiteral("rocky"),
                  QStringLiteral("alma"), QStringLiteral("suse"),
                  QStringLiteral("opensuse")})) {
        // Heterogeneous repo layouts: the vendor script detects them all.
        // curl is bootstrapped natively so Harbor itself downloads nothing.
        return QStringLiteral("command -v curl >/dev/null || "
                              "(command -v apt-get >/dev/null && "
                              "(apt-get update && apt-get install -y curl)) || "
                              "(command -v dnf >/dev/null && dnf install -y curl); "
                              "curl -fsSL https://tailscale.com/install.sh | sh && ")
            + joinStep;
    }
    return {};
}

void HarborTailnet::setStatus(const QString &status)
{
    if (m_status == status)
        return;
    m_status = status;
    emit statusChanged();
}

#ifndef Q_OS_WIN
bool HarborTailnet::installClient(const QString &keyFilePath)
{
    QFile osRelease(QStringLiteral("/etc/os-release"));
    if (!osRelease.open(QIODevice::ReadOnly))
        return false;
    const QString script =
        installScriptForOsRelease(QString::fromUtf8(osRelease.readAll()));
    if (script.isEmpty())
        return false;
    // One elevation for install + daemon start + first join + operator
    // grant; the credential travels as a file path argument, never inline.
    setStatus(QStringLiteral("installing"));
    const CommandResult setup =
        runCommand(QStringLiteral("pkexec"),
                   {QStringLiteral("sh"), QStringLiteral("-c"), script,
                    QStringLiteral("harbor-tailnet-setup"), keyFilePath},
                   QProcessEnvironment::systemEnvironment(), 180000);
    if (setup.exitCode != 0) {
        qWarning() << "Harbor Tailnet: one-time setup did not complete:"
                   << scrubKeyTokens(
                          (setup.standardError + QLatin1Char('\n')
                           + setup.standardOutput)
                              .trimmed());
        return false;
    }
    return true;
}
#endif

void HarborTailnet::ensureJoined()
{
    if (m_ensured)
        return;
    m_ensured = true;

    const char *authKey = embeddedAuthKey();
    if (authKey == nullptr || *authKey == '\0') {
        // Dev/CI builds without the secret: stay out of the way. Pairing
        // then needs a manually joined Tailscale client, as before.
        setStatus(QStringLiteral("disabled"));
        return;
    }

    // Stage the credential once for every path below (privileged install,
    // unprivileged join): a 0600 temp file referenced as --auth-key=file:.
    // argv carries only the path and logs carry nothing — `ps`- and
    // log-safe. authKey points at read-only storage and is never formatted
    // into a loggable string anywhere in this file.
    const size_t keyLength = std::strlen(authKey);
    QTemporaryFile keyFile(
        QDir::temp().filePath(QStringLiteral("harbor-tskey-XXXXXX")));
    keyFile.setAutoRemove(true);
    if (!keyFile.open()
        || keyFile.write(authKey, static_cast<qint64>(keyLength))
            != static_cast<qint64>(keyLength)) {
        qWarning() << "Harbor Tailnet: cannot stage the join credential.";
        setStatus(QStringLiteral("unavailable"));
        return;
    }
    keyFile.flush();
    const QString keyReference = QStringLiteral("file:") + keyFile.fileName();

    QProcessEnvironment baseEnvironment = QProcessEnvironment::systemEnvironment();
    if (!m_pathOverride.isEmpty()) {
        const QChar separator =
#ifdef Q_OS_WIN
            QLatin1Char(';');
#else
            QLatin1Char(':');
#endif
        baseEnvironment.insert(
            QStringLiteral("PATH"),
            m_pathOverride + separator + baseEnvironment.value(QStringLiteral("PATH")));
    }
    // Resolve the client binary: an explicit override (tests) is
    // exclusive — nothing outside it is executed. Otherwise search PATH
    // (.exe is appended on Windows automatically).
    auto resolveProgram = [&]() {
        if (!m_pathOverride.isEmpty()) {
            const QString candidate = QDir(m_pathOverride).filePath(
#ifdef Q_OS_WIN
                QStringLiteral("tailscale.exe")
#else
                QStringLiteral("tailscale")
#endif
            );
            if (QFileInfo(candidate).isFile() && QFileInfo(candidate).isExecutable())
                return candidate;
            return QString();
        }
        QString found = QStandardPaths::findExecutable(QStringLiteral("tailscale"));
        if (!found.isEmpty())
            return found;
        // Not on PATH (fresh MSI install): probe the standard roots.
        for (const QString &location : defaultClientLocations()) {
            const QString candidate = QDir(location).filePath(
#ifdef Q_OS_WIN
                QStringLiteral("tailscale.exe")
#else
                QStringLiteral("tailscale")
#endif
            );
            if (QFileInfo(candidate).isFile() && QFileInfo(candidate).isExecutable())
                return candidate;
        }
        return QString();
    };
    QString program = resolveProgram();
    bool installAttempted = false;
    if (program.isEmpty()) {
#ifndef Q_OS_WIN
        // No client at all: one-time privileged setup when the distro is
        // known (a system password dialog appears once), honest guidance
        // otherwise. Never under a test override.
        if (m_pathOverride.isEmpty()) {
            installAttempted = installClient(keyFile.fileName());
            if (installAttempted)
                program = resolveProgram();
        }
#endif
    }
    if (program.isEmpty()) {
        if (installAttempted) {
            qWarning() << "Harbor Tailnet: setup ran but no client appeared.";
            setStatus(QStringLiteral("unavailable"));
        } else {
            qWarning() << "Harbor Tailnet: no Tailscale client found;"
                          " install it once from https://tailscale.com/download";
            setStatus(QStringLiteral("missing"));
        }
        return;
    }

    setStatus(QStringLiteral("joining"));

    const CommandResult probe = runCommand(program, {QStringLiteral("status")},
                                           baseEnvironment, 10000);
    if (probe.exitCode == 0 && outputShowsTailnetAddress(probe.standardOutput)) {
        // Already logged in — possibly the user's personal, stable node.
        // Never touch it: no `up`, no re-auth, no identity change.
        setStatus(QStringLiteral("connected"));
        return;
    }

    // Logged out (or daemon warming up): join with the staged credential.
    const CommandResult join = runCommand(program,
                                          {QStringLiteral("up"),
                                           QStringLiteral("--auth-key"), keyReference},
                                          baseEnvironment, 45000);
    keyFile.remove();
    if (join.exitCode == 0) {
        const CommandResult verify = runCommand(program, {QStringLiteral("status")},
                                                baseEnvironment, 10000);
        if (verify.exitCode == 0 && outputShowsTailnetAddress(verify.standardOutput)) {
            setStatus(QStringLiteral("connected"));
            return;
        }
        qWarning() << "Harbor Tailnet: join reported success but no Tailnet"
                      " address is visible:"
                   << scrubKeyTokens(verify.standardError.trimmed());
        setStatus(QStringLiteral("unavailable"));
        return;
    }

    const QString detail = scrubKeyTokens(
        (join.standardError + QLatin1Char('\n') + join.standardOutput).trimmed());
    if (join.timedOut) {
        qWarning() << "Harbor Tailnet: join timed out; will retry next launch.";
        setStatus(QStringLiteral("unavailable"));
    } else if (outputShowsPermissionRefusal(detail)) {
        // Stock distro layout: the system daemon needs a one-time operator
        // grant (see runbook). Nothing to retry until the user runs it.
        qWarning() << "Harbor Tailnet: the system daemon needs a one-time"
                      " operator grant; run once: sudo tailscale up"
                      " --operator=$USER (then relaunch Harbor).";
        setStatus(QStringLiteral("needs-admin"));
    } else {
        qWarning() << "Harbor Tailnet: automatic join failed:" << detail;
        setStatus(QStringLiteral("unavailable"));
    }
}
