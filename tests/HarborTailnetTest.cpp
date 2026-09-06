// Unit tests for the Tailnet auto-join adapter. A stub `tailscale`
// executable replays client states (logged in/out, permission refusal,
// daemon down); the adapter is pointed at the stub directory, so no real
// daemon is ever touched. One test pins the core security property: the
// pre-auth key travels in a 0600 file referenced as --auth-key=file:PATH
// and never appears in argv.
#include "HarborTailnet.h"

#include <QDir>
#include <QFile>
#include <QTemporaryDir>
#include <QTest>

namespace {

const char kStubScript[] =
    "#!/bin/sh\n"
    "echo \"$1 argv:$*\" >> \"$STUB_LOG\"\n"
    "case \"$1\" in\n"
    "  status)\n"
    "    if grep -q logged-in \"$STUB_STATE\" 2>/dev/null; then\n"
    "      echo \"100.99.99.99  stub-node  tester  linux  -\"\n"
    "      exit 0\n"
    "    else\n"
    "      echo \"Logged out.\" >&2\n"
    "      exit 1\n"
    "    fi\n"
    "    ;;\n"
    "  up)\n"
    "    if [ \"$STUB_DENY\" = \"permission\" ]; then\n"
    "      echo \"Error: tailscaled needs root or operator\" >&2\n"
    "      exit 1\n"
    "    fi\n"
    "    if [ \"$STUB_DENY\" = \"down\" ]; then\n"
    "      echo \"Error: failed to connect to local tailscaled\" >&2\n"
    "      exit 1\n"
    "    fi\n"
    "    keyfile=\"\"\n"
    "    for a in \"$@\"; do\n"
    "      case \"$a\" in\n"
    "        *file:*) keyfile=\"${a##*file:}\" ;;\n"
    "      esac\n"
    "    done\n"
    "    if [ -z \"$keyfile\" ] || [ ! -f \"$keyfile\" ]; then\n"
    "      echo \"Error: no auth key\" >&2\n"
    "      exit 1\n"
    "    fi\n"
    "    echo \"up KEYFILE_CONTENT:$(cat \"$keyfile\")\" >> \"$STUB_LOG\"\n"
    "    echo logged-in > \"$STUB_STATE\"\n"
    "    exit 0\n"
    "    ;;\n"
    "esac\n";

QString readLog(const QString &path)
{
    QFile log(path);
    if (!log.open(QIODevice::ReadOnly))
        return {};
    return QString::fromUtf8(log.readAll());
}

} // namespace

class HarborTailnetTest final : public QObject
{
    Q_OBJECT

private slots:
    void leavesConnectedClientAlone();
    void joinsLoggedOutClientWithoutLeakingTheKey();
    void missingClientReportsMissing();
    void permissionRefusalReportsNeedsAdmin();
    void failedJoinIsOneShot();
    void installScriptPicksNativePackageOnArch();
    void installScriptUsesVendorScriptOnDebianAndFedora();
    void installScriptRefusesUnknownDistros();
};

void HarborTailnetTest::leavesConnectedClientAlone()
{
    QTemporaryDir stubDir;
    QVERIFY(stubDir.isValid());
    QFile stub(stubDir.filePath(QStringLiteral("tailscale")));
    QVERIFY(stub.open(QIODevice::WriteOnly));
    QVERIFY(stub.write(kStubScript) > 0);
    stub.close();
    QVERIFY(QFile::setPermissions(
        stub.fileName(), QFileDevice::ReadOwner | QFileDevice::WriteOwner
                             | QFileDevice::ExeOwner | QFileDevice::ReadGroup
                             | QFileDevice::ExeGroup | QFileDevice::ReadOther
                             | QFileDevice::ExeOther));
    QFile state(stubDir.filePath(QStringLiteral("state")));
    QVERIFY(state.open(QIODevice::WriteOnly));
    QVERIFY(state.write("logged-in") > 0);
    state.close();

    qputenv("STUB_LOG", stubDir.filePath(QStringLiteral("log")).toUtf8());
    qputenv("STUB_STATE", state.fileName().toUtf8());
    qputenv("STUB_DENY", "");

    HarborTailnet tailnet;
    tailnet.setPathOverride(stubDir.path());
    tailnet.ensureJoined();
    QCOMPARE(tailnet.status(), QStringLiteral("connected"));
    // A personal, already-connected login must never see an `up`.
    QVERIFY(!readLog(stubDir.filePath(QStringLiteral("log"))).contains(QStringLiteral("up ")));

    qunsetenv("STUB_LOG");
    qunsetenv("STUB_STATE");
    qunsetenv("STUB_DENY");
}

void HarborTailnetTest::joinsLoggedOutClientWithoutLeakingTheKey()
{
    QTemporaryDir stubDir;
    QVERIFY(stubDir.isValid());
    QFile stub(stubDir.filePath(QStringLiteral("tailscale")));
    QVERIFY(stub.open(QIODevice::WriteOnly));
    QVERIFY(stub.write(kStubScript) > 0);
    stub.close();
    QVERIFY(QFile::setPermissions(
        stub.fileName(), QFileDevice::ReadOwner | QFileDevice::WriteOwner
                             | QFileDevice::ExeOwner | QFileDevice::ReadGroup
                             | QFileDevice::ExeGroup | QFileDevice::ReadOther
                             | QFileDevice::ExeOther));

    qputenv("STUB_LOG", stubDir.filePath(QStringLiteral("log")).toUtf8());
    qputenv("STUB_STATE", stubDir.filePath(QStringLiteral("state")).toUtf8());
    qputenv("STUB_DENY", "");

    HarborTailnet tailnet;
    tailnet.setPathOverride(stubDir.path());
    tailnet.ensureJoined();
    QCOMPARE(tailnet.status(), QStringLiteral("connected"));

    const QString log = readLog(stubDir.filePath(QStringLiteral("log")));
    // The join happened with the key staged in a file …
    QVERIFY(log.contains(QStringLiteral("up KEYFILE_CONTENT:tskey-auth-test-only-0000")));
    // … while no argv line ever carried the credential (`ps`-safe).
    const QStringList lines = log.split(QLatin1Char('\n'));
    for (const QString &line : lines) {
        if (line.contains(QStringLiteral("argv:")))
            QVERIFY(!line.contains(QStringLiteral("tskey-auth-test-only-0000")));
    }

    qunsetenv("STUB_LOG");
    qunsetenv("STUB_STATE");
    qunsetenv("STUB_DENY");
}

void HarborTailnetTest::missingClientReportsMissing()
{
    QTemporaryDir emptyDir;
    QVERIFY(emptyDir.isValid());

    HarborTailnet tailnet;
    tailnet.setPathOverride(emptyDir.path());
    tailnet.ensureJoined();
    QCOMPARE(tailnet.status(), QStringLiteral("missing"));
}

void HarborTailnetTest::permissionRefusalReportsNeedsAdmin()
{
    QTemporaryDir stubDir;
    QVERIFY(stubDir.isValid());
    QFile stub(stubDir.filePath(QStringLiteral("tailscale")));
    QVERIFY(stub.open(QIODevice::WriteOnly));
    QVERIFY(stub.write(kStubScript) > 0);
    stub.close();
    QVERIFY(QFile::setPermissions(
        stub.fileName(), QFileDevice::ReadOwner | QFileDevice::WriteOwner
                             | QFileDevice::ExeOwner | QFileDevice::ReadGroup
                             | QFileDevice::ExeGroup | QFileDevice::ReadOther
                             | QFileDevice::ExeOther));

    qputenv("STUB_LOG", stubDir.filePath(QStringLiteral("log")).toUtf8());
    qputenv("STUB_STATE", stubDir.filePath(QStringLiteral("state")).toUtf8());
    qputenv("STUB_DENY", "permission");

    HarborTailnet tailnet;
    tailnet.setPathOverride(stubDir.path());
    tailnet.ensureJoined();
    QCOMPARE(tailnet.status(), QStringLiteral("needs-admin"));

    qunsetenv("STUB_LOG");
    qunsetenv("STUB_STATE");
    qunsetenv("STUB_DENY");
}

void HarborTailnetTest::failedJoinIsOneShot()
{
    QTemporaryDir stubDir;
    QVERIFY(stubDir.isValid());
    QFile stub(stubDir.filePath(QStringLiteral("tailscale")));
    QVERIFY(stub.open(QIODevice::WriteOnly));
    QVERIFY(stub.write(kStubScript) > 0);
    stub.close();
    QVERIFY(QFile::setPermissions(
        stub.fileName(), QFileDevice::ReadOwner | QFileDevice::WriteOwner
                             | QFileDevice::ExeOwner | QFileDevice::ReadGroup
                             | QFileDevice::ExeGroup | QFileDevice::ReadOther
                             | QFileDevice::ExeOther));

    qputenv("STUB_LOG", stubDir.filePath(QStringLiteral("log")).toUtf8());
    qputenv("STUB_STATE", stubDir.filePath(QStringLiteral("state")).toUtf8());
    qputenv("STUB_DENY", "down");

    HarborTailnet tailnet;
    tailnet.setPathOverride(stubDir.path());
    tailnet.ensureJoined();
    tailnet.ensureJoined();
    QCOMPARE(tailnet.status(), QStringLiteral("unavailable"));
    // Exactly one `up` attempt per process, never a retry loop.
    QCOMPARE(readLog(stubDir.filePath(QStringLiteral("log")))
                 .count(QStringLiteral("up argv:up")),
             1);

    qunsetenv("STUB_LOG");
    qunsetenv("STUB_STATE");
    qunsetenv("STUB_DENY");
}

void HarborTailnetTest::installScriptPicksNativePackageOnArch()
{
    const QString script = HarborTailnet::installScriptForOsRelease(
        QStringLiteral("NAME=\"Arch Linux\"\nID=arch\nID_LIKE=\"archlinux\"\n"));
    QVERIFY(!script.isEmpty());
    QVERIFY(script.contains(QStringLiteral("pacman")));
    QVERIFY(script.contains(QStringLiteral("tailscale up")));
    // The credential travels as a staged-file argument, never inline.
    QVERIFY(script.contains(QStringLiteral("$1")));
    QVERIFY(!script.contains(QStringLiteral("tskey")));
}

void HarborTailnetTest::installScriptUsesVendorScriptOnDebianAndFedora()
{
    const QString ubuntu = HarborTailnet::installScriptForOsRelease(
        QStringLiteral(
            "NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=\"debian\"\n"));
    QVERIFY(ubuntu.contains(QStringLiteral("tailscale.com/install.sh")));
    QVERIFY(ubuntu.contains(QStringLiteral("tailscale up")));

    const QString fedora = HarborTailnet::installScriptForOsRelease(
        QStringLiteral("NAME=\"Fedora Linux\"\nID=fedora\n"));
    QVERIFY(fedora.contains(QStringLiteral("tailscale.com/install.sh")));
}

void HarborTailnetTest::installScriptRefusesUnknownDistros()
{
    QVERIFY(HarborTailnet::installScriptForOsRelease({}).isEmpty());
    QVERIFY(HarborTailnet::installScriptForOsRelease(
                QStringLiteral("NAME=\"MysteryOS\"\nID=mystery\n"))
                .isEmpty());
}

QTEST_MAIN(HarborTailnetTest)
#include "HarborTailnetTest.moc"
