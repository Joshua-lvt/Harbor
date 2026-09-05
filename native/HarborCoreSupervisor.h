#pragma once

#include <QByteArray>
#include <QObject>
#include <QProcess>
#include <QThread>
#include <QTimer>

class QJsonObject;

class HarborCoreSupervisor final : public QObject
{
    Q_OBJECT

public:
    enum class State {
        Stopped,
        Starting,
        Running,
        Backoff,
        Failed,
    };
    Q_ENUM(State)

    explicit HarborCoreSupervisor(QObject *parent = nullptr);
    ~HarborCoreSupervisor() override;

    State state() const;
    bool isRunning() const;

    void start();
    void stop();
    bool sendEnvelope(const QJsonObject &envelope);

signals:
    void stateChanged(HarborCoreSupervisor::State state);
    void processStarted();
    void envelopeReceived(const QJsonObject &envelope);
    void faulted(const QString &errorKey);

private slots:
    void readStandardOutput();
    void handleProcessError(QProcess::ProcessError error);
    void handleProcessFinished(int exitCode, QProcess::ExitStatus exitStatus);
    void restartAfterBackoff();

private:
    static constexpr qsizetype MaxFrameBytes = 1024 * 1024;
    static constexpr int MaxRestartAttempts = 5;

    void startProcess();
    void setState(State state);
    void fail(const QString &errorKey);
    void scheduleRestart();
    QString coreProgram() const;
    void clearPendingFrames();
    bool processFrames();

    QProcess m_process;
    QTimer m_restartTimer;
    QThread m_frameThread;
    QObject *m_frameWorker = nullptr;
    QByteArray m_pending;
    State m_state = State::Stopped;
    int m_restartAttempts = 0;
    bool m_stopRequested = false;
    bool m_faultReported = false;
};
