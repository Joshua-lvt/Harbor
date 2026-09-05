#pragma once

#include <QJsonObject>
#include <QObject>

/// Android platform facts for the mobile shell. Every property is either a
/// live reading or an honest unknown: permission states are explicit, and
/// unavailable APIs surface as unavailable, never as invented data.
///
/// The Java side (HarborMobileActivity plus the service adapters under
/// android/src/org/harbor/mobile) owns the platform calls; this facade
/// only ferries typed facts and user intents across JNI.
class HarborAndroid final : public QObject
{
    Q_OBJECT

    // Battery (BatteryManager, no permission needed).
    Q_PROPERTY(bool batteryAvailable READ batteryAvailable NOTIFY platformChanged)
    Q_PROPERTY(int batteryPercent READ batteryPercent NOTIFY platformChanged)
    Q_PROPERTY(bool batteryCharging READ batteryCharging NOTIFY platformChanged)
    // Phone activity (UsageStatsManager, special access grant).
    Q_PROPERTY(QString phoneActivity READ phoneActivity NOTIFY platformChanged)
    Q_PROPERTY(QString currentApp READ currentApp NOTIFY platformChanged)
    Q_PROPERTY(QString lastActiveText READ lastActiveText NOTIFY platformChanged)
    Q_PROPERTY(qint64 lastActiveAt READ lastActiveAt NOTIFY platformChanged)
    // Location (Fused provider via the location foreground service).
    Q_PROPERTY(bool locationAvailable READ locationAvailable NOTIFY platformChanged)
    Q_PROPERTY(QString locationText READ locationText NOTIFY platformChanged)
    Q_PROPERTY(QString locationUpdatedText READ locationUpdatedText NOTIFY platformChanged)
    Q_PROPERTY(double locationLatitude READ locationLatitude NOTIFY platformChanged)
    Q_PROPERTY(double locationLongitude READ locationLongitude NOTIFY platformChanged)
    Q_PROPERTY(double locationAccuracyMeters READ locationAccuracyMeters NOTIFY platformChanged)
    Q_PROPERTY(qint64 locationUpdatedAt READ locationUpdatedAt NOTIFY platformChanged)
    // Permission states: "granted" | "denied" | "unknown".
    Q_PROPERTY(QString usagePermission READ usagePermission NOTIFY platformChanged)
    Q_PROPERTY(QString locationPermission READ locationPermission NOTIFY platformChanged)
    Q_PROPERTY(QString backgroundLocationPermission READ backgroundLocationPermission NOTIFY platformChanged)
    Q_PROPERTY(QString notificationPermission READ notificationPermission NOTIFY platformChanged)
    Q_PROPERTY(QString microphonePermission READ microphonePermission NOTIFY platformChanged)
    Q_PROPERTY(QString ownNotificationPermission READ ownNotificationPermission NOTIFY platformChanged)
    Q_PROPERTY(QString batteryOptimizationPermission READ batteryOptimizationPermission NOTIFY platformChanged)

public:
    explicit HarborAndroid(QObject *parent = nullptr);

    bool batteryAvailable() const { return m_batteryAvailable; }
    int batteryPercent() const { return m_batteryPercent; }
    bool batteryCharging() const { return m_batteryCharging; }
    QString phoneActivity() const { return m_phoneActivity; }
    QString currentApp() const { return m_currentApp; }
    QString lastActiveText() const { return m_lastActiveText; }
    qint64 lastActiveAt() const { return m_lastActiveAt; }
    bool locationAvailable() const { return m_locationAvailable; }
    QString locationText() const { return m_locationText; }
    QString locationUpdatedText() const { return m_locationUpdatedText; }
    double locationLatitude() const { return m_locationLatitude; }
    double locationLongitude() const { return m_locationLongitude; }
    double locationAccuracyMeters() const { return m_locationAccuracyMeters; }
    qint64 locationUpdatedAt() const { return m_locationUpdatedAt; }
    QString usagePermission() const { return m_usagePermission; }
    QString locationPermission() const { return m_locationPermission; }
    QString backgroundLocationPermission() const { return m_backgroundLocationPermission; }
    QString notificationPermission() const { return m_notificationPermission; }
    QString microphonePermission() const { return m_microphonePermission; }
    QString ownNotificationPermission() const { return m_ownNotificationPermission; }
    QString batteryOptimizationPermission() const { return m_batteryOptimizationPermission; }

    /// Polls every platform source and pushes one `mobile.update` intent's
    /// worth of facts to QML through `platformChanged`. Called on a slow
    /// cadence and after every grant/settings activity returns.
    Q_INVOKABLE void refresh();
    /// Opens the Android settings page for one access grant:
    /// "location" | "usage" | "notifications" | "battery".
    Q_INVOKABLE void openSystemSettings(const QString &page);
    /// Starts/stops the location foreground service. Starting without a
    /// grant is a no-op that refreshes the permission state instead.
    Q_INVOKABLE void setLocationSharing(bool on);
    /// Applies an already-resolved effective state without opening a
    /// permission dialog or emitting another refresh signal.
    Q_INVOKABLE void syncLocationService(bool on);
    /// Enables/disables phone-notification mirroring at the listener.
    Q_INVOKABLE void setNotificationMirroring(bool on);
    /// Renders the persistent partner presence bar (foreground-service
    /// notification) while paired: name, ONLINE/AWAY/OFFLINE, and the
    /// shared current app. Tapping opens Harbor.
    Q_INVOKABLE void updatePresenceBar(const QString &name, const QString &state,
                                       const QString &detail);
    /// Stops the persistent presence bar (unpaired: no stale offline row).
    Q_INVOKABLE void hidePresenceBar();
    /// Posts a Harbor notification (message or presence) to the system tray
    /// when the app is backgrounded. Foreground delivery stays in-app.
    Q_INVOKABLE void postHarborNotification(const QString &title, const QString &text);
    /// Starts the call foreground service only after RECORD_AUDIO is granted.
    /// A false return means the caller must render an unavailable state.
    Q_INVOKABLE bool ensureCallAudio();
    Q_INVOKABLE void stopCallAudio();
    /// Requests the Android 13+ permission used only for Harbor's own
    /// notifications. It is deliberately separate from listener access.
    Q_INVOKABLE void requestOwnNotificationPermission();
    /// Mandatory update channel: check the release feed (worker thread),
    /// read the latest state JSON, fetch the verified package, and hand it
    /// to the platform installer UI. State shape:
    /// {"status","version","progress","error","url","sha"}.
    Q_INVOKABLE void checkForUpdates();
    Q_INVOKABLE QString updateState();
    Q_INVOKABLE void downloadUpdate(const QString &url, const QString &sha);
    Q_INVOKABLE void installUpdate();
    /// Extracts the ABI-specific worker from the APK once and returns its
    /// absolute private path. Empty means media is unavailable.
    QString prepareMediaWorker() const;

signals:
    void platformChanged();
    /// Ephemeral phone notification facts. The host forwards them to the
    /// core immediately; neither this facade nor the core persists them.
    void phoneNotification(const QString &appLabel, const QString &title,
                           const QString &text, qint64 postedAt);

private:
    void pollBattery();
    void pollPermissions();
    void pollUsage();
    void pollLocation();

    bool m_batteryAvailable = false;
    int m_batteryPercent = 0;
    bool m_batteryCharging = false;
    QString m_phoneActivity = QStringLiteral("offline");
    QString m_currentApp;
    QString m_lastActiveText;
    qint64 m_lastActiveAt = 0;
    bool m_locationAvailable = false;
    QString m_locationText;
    QString m_locationUpdatedText;
    double m_locationLatitude = 0.0;
    double m_locationLongitude = 0.0;
    double m_locationAccuracyMeters = -1.0;
    qint64 m_locationUpdatedAt = 0;
    QString m_usagePermission = QStringLiteral("unknown");
    QString m_locationPermission = QStringLiteral("unknown");
    QString m_backgroundLocationPermission = QStringLiteral("unknown");
    QString m_notificationPermission = QStringLiteral("unknown");
    QString m_microphonePermission = QStringLiteral("unknown");
    QString m_ownNotificationPermission = QStringLiteral("unknown");
    QString m_batteryOptimizationPermission = QStringLiteral("unknown");
};
