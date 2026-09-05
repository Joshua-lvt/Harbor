#include "HarborCoreSupervisor.h"

#include <QCoreApplication>
#include <QDateTime>
#include <QDir>
#include <QtEndian>
#include <QFileInfo>
#include <QJsonDocument>
#include <QJsonObject>
#include <QMetaObject>
#include <QUuid>

namespace {
constexpr int ShutdownDeadlineMs = 3000;
constexpr int InitialBackoffMs = 250;
constexpr int MaximumBackoffMs = 4000;

bool isUuid(const QJsonValue &value)
{
    const QUuid identifier(value.toString());
    return value.isString() && !identifier.isNull();
}

bool isValidEnvelope(const QJsonObject &envelope)
{
    if (envelope.value(QStringLiteral("v")).toInt(-1) != 1
        || !envelope.value(QStringLiteral("type")).isString()
        || envelope.value(QStringLiteral("type")).toString().isEmpty()
        || !envelope.value(QStringLiteral("timestamp")).isString()
        || envelope.value(QStringLiteral("timestamp")).toString().isEmpty()
        || !envelope.value(QStringLiteral("payload")).isObject()) {
        return false;
    }

    const QJsonValue requestId = envelope.value(QStringLiteral("request_id"));
    const QJsonValue eventId = envelope.value(QStringLiteral("event_id"));
    if (requestId.isUndefined() == eventId.isUndefined())
        return false;
    if (!requestId.isUndefined() && !isUuid(requestId))
        return false;
    if (!eventId.isUndefined() && !isUuid(eventId))
        return false;

    const QJsonValue replyTo = envelope.value(QStringLiteral("reply_to"));
    if (!replyTo.isUndefined() && !isUuid(replyTo))
        return false;

    const QJsonValue error = envelope.value(QStringLiteral("error"));
    if (!error.isUndefined()) {
        if (!error.isObject())
            return false;
        const QJsonObject errorObject = error.toObject();
        if (!errorObject.value(QStringLiteral("code")).isString()
            || errorObject.value(QStringLiteral("code")).toString().isEmpty()
            || !errorObject.value(QStringLiteral("ui_key")).isString()
            || errorObject.value(QStringLiteral("ui_key")).toString().isEmpty()
            || !errorObject.value(QStringLiteral("retryable")).isBool()
            || !errorObject.value(QStringLiteral("detail")).isString()) {
            return false;
        }
    }

    return true;
}

QJsonObject shutdownEnvelope()
{
    return {
        {QStringLiteral("v"), 1},
        {QStringLiteral("type"), QStringLiteral("core.shutdown")},
        {QStringLiteral("request_id"), QUuid::createUuid().toString(QUuid::WithoutBraces)},
        {QStringLiteral("timestamp"), QDateTime::currentDateTimeUtc().toString(Qt::ISODateWithMs)},
        {QStringLiteral("payload"), QJsonObject{}},
    };
}
} // namespace

HarborCoreSupervisor::HarborCoreSupervisor(QObject *parent)
    : QObject(parent)
{
    m_process.setProcessChannelMode(QProcess::SeparateChannels);
    m_restartTimer.setSingleShot(true);
    m_frameWorker = new QObject;
    m_frameWorker->moveToThread(&m_frameThread);
    connect(&m_frameThread, &QThread::finished, m_frameWorker, &QObject::deleteLater);
    m_frameThread.start();

    connect(&m_process, &QProcess::started, this, [this] {
        setState(State::Running);
        emit processStarted();
    });
    connect(&m_process, &QProcess::readyReadStandardOutput,
            this, &HarborCoreSupervisor::readStandardOutput);
    connect(&m_process, &QProcess::errorOccurred,
            this, &HarborCoreSupervisor::handleProcessError);
    connect(&m_process,
            qOverload<int, QProcess::ExitStatus>(&QProcess::finished),
            this, &HarborCoreSupervisor::handleProcessFinished);
    connect(&m_restartTimer, &QTimer::timeout,
            this, &HarborCoreSupervisor::restartAfterBackoff);
}

HarborCoreSupervisor::~HarborCoreSupervisor()
{
    m_stopRequested = true;
    m_restartTimer.stop();
    if (m_process.state() != QProcess::NotRunning) {
        m_process.kill();
        m_process.waitForFinished(ShutdownDeadlineMs);
    }
    m_frameThread.quit();
    m_frameThread.wait();
    m_frameWorker = nullptr;
}

HarborCoreSupervisor::State HarborCoreSupervisor::state() const
{
    return m_state;
}

bool HarborCoreSupervisor::isRunning() const
{
    return m_state == State::Starting || m_state == State::Running;
}

void HarborCoreSupervisor::start()
{
    if (m_state == State::Starting || m_state == State::Running || m_state == State::Backoff)
        return;

    m_stopRequested = false;
    m_faultReported = false;
    m_restartAttempts = 0;
    startProcess();
}

void HarborCoreSupervisor::stop()
{
    m_stopRequested = true;
    m_restartTimer.stop();

    if (m_process.state() == QProcess::NotRunning) {
        setState(State::Stopped);
        return;
    }

    sendEnvelope(shutdownEnvelope());
    QTimer::singleShot(ShutdownDeadlineMs, this, [this] {
        if (m_process.state() != QProcess::NotRunning)
            m_process.kill();
    });
}

bool HarborCoreSupervisor::sendEnvelope(const QJsonObject &envelope)
{
    if (m_process.state() != QProcess::Running || !isValidEnvelope(envelope))
        return false;

    const QByteArray payload = QJsonDocument(envelope).toJson(QJsonDocument::Compact);
    if (payload.isEmpty() || payload.size() > MaxFrameBytes)
        return false;

    const quint32 length = qToBigEndian<quint32>(payload.size());
    QByteArray frame(reinterpret_cast<const char *>(&length), sizeof(length));
    frame.append(payload);
    return m_process.write(frame) == frame.size();
}

void HarborCoreSupervisor::readStandardOutput()
{
    const QByteArray output = m_process.readAllStandardOutput();
    QMetaObject::invokeMethod(m_frameWorker, [this, output] {
        m_pending.append(output);
        processFrames();
    }, Qt::QueuedConnection);
}

void HarborCoreSupervisor::handleProcessError(QProcess::ProcessError error)
{
    if (m_stopRequested)
        return;

    if (error == QProcess::FailedToStart) {
        fail(QStringLiteral("error.core.unavailable"));
        return;
    }

    scheduleRestart();
}

void HarborCoreSupervisor::handleProcessFinished(int exitCode, QProcess::ExitStatus exitStatus)
{
    if (m_stopRequested) {
        clearPendingFrames();
        setState(State::Stopped);
        return;
    }

    if (exitStatus == QProcess::CrashExit || exitCode != 0) {
        scheduleRestart();
        return;
    }

    fail(QStringLiteral("error.core.stoppedUnexpectedly"));
}

void HarborCoreSupervisor::restartAfterBackoff()
{
    if (!m_stopRequested)
        startProcess();
}

void HarborCoreSupervisor::startProcess()
{
    const QString program = coreProgram();
    if (program.isEmpty() || !QFileInfo::exists(program) || !QFileInfo(program).isExecutable()) {
        fail(QStringLiteral("error.core.unavailable"));
        return;
    }

    clearPendingFrames();
    setState(State::Starting);
    m_process.setProgram(program);
    m_process.setArguments({});
    m_process.start();
}

void HarborCoreSupervisor::setState(State state)
{
    if (m_state == state)
        return;

    m_state = state;
    emit stateChanged(m_state);
}

void HarborCoreSupervisor::fail(const QString &errorKey)
{
    m_restartTimer.stop();
    if (m_process.state() != QProcess::NotRunning)
        m_process.kill();
    setState(State::Failed);
    if (!m_faultReported) {
        m_faultReported = true;
        emit faulted(errorKey);
    }
}

void HarborCoreSupervisor::scheduleRestart()
{
    if (m_stopRequested || m_state == State::Backoff || m_state == State::Failed)
        return;

    if (m_restartAttempts >= MaxRestartAttempts) {
        fail(QStringLiteral("error.core.restartExhausted"));
        return;
    }

    const int delay = qMin(InitialBackoffMs * (1 << m_restartAttempts), MaximumBackoffMs);
    ++m_restartAttempts;
    setState(State::Backoff);
    m_restartTimer.start(delay);
}

QString HarborCoreSupervisor::coreProgram() const
{
#ifdef HARBOR_TEST_CORE_EXECUTABLE
    return QString::fromUtf8(HARBOR_TEST_CORE_EXECUTABLE);
#else
#ifdef Q_OS_WIN
    const QString executableName = QStringLiteral("harbor-core.exe");
#else
    const QString executableName = QStringLiteral("harbor-core");
#endif
    return QDir(QCoreApplication::applicationDirPath()).filePath(executableName);
#endif
}

void HarborCoreSupervisor::clearPendingFrames()
{
    QMetaObject::invokeMethod(m_frameWorker, [this] {
        m_pending.clear();
    }, Qt::QueuedConnection);
}

bool HarborCoreSupervisor::processFrames()
{
    while (m_pending.size() >= 4) {
        const quint32 length = qFromBigEndian<quint32>(
            reinterpret_cast<const uchar *>(m_pending.constData()));
        if (length == 0 || length > MaxFrameBytes) {
            QMetaObject::invokeMethod(this, [this] {
                fail(QStringLiteral("error.core.invalidFrame"));
            }, Qt::QueuedConnection);
            return false;
        }
        if (m_pending.size() < static_cast<qsizetype>(4 + length))
            return true;

        const QByteArray payload = m_pending.mid(4, length);
        m_pending.remove(0, 4 + length);
        QJsonParseError error;
        const QJsonDocument document = QJsonDocument::fromJson(payload, &error);
        if (error.error != QJsonParseError::NoError || !document.isObject()
            || !isValidEnvelope(document.object())) {
            QMetaObject::invokeMethod(this, [this] {
                fail(QStringLiteral("error.core.invalidFrame"));
            }, Qt::QueuedConnection);
            return false;
        }
        const QJsonObject envelope = document.object();
        QMetaObject::invokeMethod(this, [this, envelope] {
            emit envelopeReceived(envelope);
        }, Qt::QueuedConnection);
    }
    return true;
}
