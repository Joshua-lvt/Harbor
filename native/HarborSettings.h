#pragma once

#include <QJsonObject>
#include <QObject>
#include <QString>

/// Typed, QML-facing mirror of the durable settings owned by the Rust core.
///
/// Every property key matches the control-protocol settings document exactly.
/// Setters only notify; the facade persists patches through the core and
/// applies the authoritative result back, so this object never invents state.
class HarborSettings final : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool loaded READ loaded NOTIFY loadedChanged FINAL)
    Q_PROPERTY(QString locale READ locale WRITE setLocale NOTIFY localeChanged FINAL)
    Q_PROPERTY(QString displayName READ displayName WRITE setDisplayName NOTIFY displayNameChanged FINAL)
    Q_PROPERTY(QString statusMessage READ statusMessage WRITE setStatusMessage NOTIFY statusMessageChanged FINAL)
    Q_PROPERTY(QString avatar READ avatar WRITE setAvatar NOTIFY avatarChanged FINAL)
    Q_PROPERTY(QString avatarType READ avatarType WRITE setAvatarType NOTIFY avatarTypeChanged FINAL)
    Q_PROPERTY(bool higherContrast READ higherContrast WRITE setHigherContrast NOTIFY higherContrastChanged FINAL)
    Q_PROPERTY(bool background READ background WRITE setBackground NOTIFY backgroundChanged FINAL)
    Q_PROPERTY(bool reducedMotion READ reducedMotion WRITE setReducedMotion NOTIFY reducedMotionChanged FINAL)
    Q_PROPERTY(bool startWithSystem READ startWithSystem WRITE setStartWithSystem NOTIFY startWithSystemChanged FINAL)
    Q_PROPERTY(bool minimizeToTray READ minimizeToTray WRITE setMinimizeToTray NOTIFY minimizeToTrayChanged FINAL)
    Q_PROPERTY(bool closeToTray READ closeToTray WRITE setCloseToTray NOTIFY closeToTrayChanged FINAL)
    Q_PROPERTY(bool autoConnect READ autoConnect WRITE setAutoConnect NOTIFY autoConnectChanged FINAL)
    Q_PROPERTY(bool notificationsEnabled READ notificationsEnabled WRITE setNotificationsEnabled NOTIFY notificationsEnabledChanged FINAL)
    Q_PROPERTY(bool gameNotifications READ gameNotifications WRITE setGameNotifications NOTIFY gameNotificationsChanged FINAL)
    Q_PROPERTY(bool appNotifications READ appNotifications WRITE setAppNotifications NOTIFY appNotificationsChanged FINAL)
    Q_PROPERTY(bool connectionNotifications READ connectionNotifications WRITE setConnectionNotifications NOTIFY connectionNotificationsChanged FINAL)
    Q_PROPERTY(bool notificationSound READ notificationSound WRITE setNotificationSound NOTIFY notificationSoundChanged FINAL)
    Q_PROPERTY(bool messagePreviews READ messagePreviews WRITE setMessagePreviews NOTIFY messagePreviewsChanged FINAL)
    Q_PROPERTY(bool notifyPartnerOnline READ notifyPartnerOnline WRITE setNotifyPartnerOnline NOTIFY notifyPartnerOnlineChanged FINAL)
    Q_PROPERTY(bool notifyPartnerAway READ notifyPartnerAway WRITE setNotifyPartnerAway NOTIFY notifyPartnerAwayChanged FINAL)
    Q_PROPERTY(bool notifyPartnerOffline READ notifyPartnerOffline WRITE setNotifyPartnerOffline NOTIFY notifyPartnerOfflineChanged FINAL)
    Q_PROPERTY(bool presenceVisibility READ presenceVisibility WRITE setPresenceVisibility NOTIFY presenceVisibilityChanged FINAL)
    Q_PROPERTY(bool activitySharing READ activitySharing WRITE setActivitySharing NOTIFY activitySharingChanged FINAL)
    Q_PROPERTY(bool gameVisibility READ gameVisibility WRITE setGameVisibility NOTIFY gameVisibilityChanged FINAL)
    Q_PROPERTY(bool deviceVisibility READ deviceVisibility WRITE setDeviceVisibility NOTIFY deviceVisibilityChanged FINAL)
    Q_PROPERTY(bool voiceActivation READ voiceActivation WRITE setVoiceActivation NOTIFY voiceActivationChanged FINAL)
    Q_PROPERTY(bool debugMode READ debugMode WRITE setDebugMode NOTIFY debugModeChanged FINAL)
    Q_PROPERTY(qreal accentIntensity READ accentIntensity WRITE setAccentIntensity NOTIFY accentIntensityChanged FINAL)
    Q_PROPERTY(qreal microphoneVolume READ microphoneVolume WRITE setMicrophoneVolume NOTIFY microphoneVolumeChanged FINAL)
    Q_PROPERTY(qreal outputVolume READ outputVolume WRITE setOutputVolume NOTIFY outputVolumeChanged FINAL)
    Q_PROPERTY(QString inputDevice READ inputDevice WRITE setInputDevice NOTIFY inputDeviceChanged FINAL)
    Q_PROPERTY(QString outputDevice READ outputDevice WRITE setOutputDevice NOTIFY outputDeviceChanged FINAL)
    Q_PROPERTY(QString pushToTalkKey READ pushToTalkKey WRITE setPushToTalkKey NOTIFY pushToTalkKeyChanged FINAL)
    Q_PROPERTY(bool pushToTalkEnabled READ pushToTalkEnabled WRITE setPushToTalkEnabled NOTIFY pushToTalkEnabledChanged FINAL)
    /// Completed file transfers land here; empty means the platform default.
    Q_PROPERTY(QString transferDirectory READ transferDirectory WRITE setTransferDirectory NOTIFY transferDirectoryChanged FINAL)
    Q_PROPERTY(QString appearanceMode READ appearanceMode WRITE setAppearanceMode NOTIFY appearanceModeChanged FINAL)
    Q_PROPERTY(QString accentColor READ accentColor WRITE setAccentColor NOTIFY accentColorChanged FINAL)
    Q_PROPERTY(qreal glassIntensity READ glassIntensity WRITE setGlassIntensity NOTIFY glassIntensityChanged FINAL)
    Q_PROPERTY(qreal animationIntensity READ animationIntensity WRITE setAnimationIntensity NOTIFY animationIntensityChanged FINAL)
    Q_PROPERTY(bool particlesEnabled READ particlesEnabled WRITE setParticlesEnabled NOTIFY particlesEnabledChanged FINAL)
    Q_PROPERTY(QString oceanVariant READ oceanVariant WRITE setOceanVariant NOTIFY oceanVariantChanged FINAL)
    Q_PROPERTY(QString cornerRadius READ cornerRadius WRITE setCornerRadius NOTIFY cornerRadiusChanged FINAL)
    Q_PROPERTY(QString density READ density WRITE setDensity NOTIFY densityChanged FINAL)
    Q_PROPERTY(bool widgetEnabled READ widgetEnabled WRITE setWidgetEnabled NOTIFY widgetEnabledChanged FINAL)
    Q_PROPERTY(QString widgetPosition READ widgetPosition WRITE setWidgetPosition NOTIFY widgetPositionChanged FINAL)
    Q_PROPERTY(bool widgetShowActivity READ widgetShowActivity WRITE setWidgetShowActivity NOTIFY widgetShowActivityChanged FINAL)
    Q_PROPERTY(bool widgetShowAvatar READ widgetShowAvatar WRITE setWidgetShowAvatar NOTIFY widgetShowAvatarChanged FINAL)
    Q_PROPERTY(bool widgetShowCallPresence READ widgetShowCallPresence WRITE setWidgetShowCallPresence NOTIFY widgetShowCallPresenceChanged FINAL)

public:
    explicit HarborSettings(QObject *parent = nullptr);

    bool loaded() const;

    QString locale() const;
    QString displayName() const;
    QString statusMessage() const;
    QString avatar() const;
    QString avatarType() const;
    bool higherContrast() const;
    bool background() const;
    bool reducedMotion() const;
    bool startWithSystem() const;
    bool minimizeToTray() const;
    bool closeToTray() const;
    bool autoConnect() const;
    bool notificationsEnabled() const;
    bool gameNotifications() const;
    bool appNotifications() const;
    bool connectionNotifications() const;
    bool notificationSound() const;
    bool messagePreviews() const;
    bool notifyPartnerOnline() const;
    bool notifyPartnerAway() const;
    bool notifyPartnerOffline() const;
    bool presenceVisibility() const;
    bool activitySharing() const;
    bool gameVisibility() const;
    bool deviceVisibility() const;
    bool voiceActivation() const;
    bool debugMode() const;
    qreal accentIntensity() const;
    qreal microphoneVolume() const;
    qreal outputVolume() const;
    QString inputDevice() const;
    QString outputDevice() const;
    QString pushToTalkKey() const;
    bool pushToTalkEnabled() const;
    QString transferDirectory() const;
    QString appearanceMode() const;
    QString accentColor() const;
    qreal glassIntensity() const;
    qreal animationIntensity() const;
    bool particlesEnabled() const;
    QString oceanVariant() const;
    QString cornerRadius() const;
    QString density() const;
    bool widgetEnabled() const;
    QString widgetPosition() const;
    bool widgetShowActivity() const;
    bool widgetShowAvatar() const;
    bool widgetShowCallPresence() const;

    void setLocale(const QString &locale);
    void setDisplayName(const QString &displayName);
    void setStatusMessage(const QString &statusMessage);
    void setAvatar(const QString &avatar);
    void setAvatarType(const QString &avatarType);
    void setHigherContrast(bool higherContrast);
    void setBackground(bool background);
    void setReducedMotion(bool reducedMotion);
    void setStartWithSystem(bool startWithSystem);
    void setMinimizeToTray(bool minimizeToTray);
    void setCloseToTray(bool closeToTray);
    void setAutoConnect(bool autoConnect);
    void setNotificationsEnabled(bool notificationsEnabled);
    void setGameNotifications(bool gameNotifications);
    void setAppNotifications(bool appNotifications);
    void setConnectionNotifications(bool connectionNotifications);
    void setNotificationSound(bool notificationSound);
    void setMessagePreviews(bool messagePreviews);
    void setNotifyPartnerOnline(bool notifyPartnerOnline);
    void setNotifyPartnerAway(bool notifyPartnerAway);
    void setNotifyPartnerOffline(bool notifyPartnerOffline);
    void setPresenceVisibility(bool presenceVisibility);
    void setActivitySharing(bool activitySharing);
    void setGameVisibility(bool gameVisibility);
    void setDeviceVisibility(bool deviceVisibility);
    void setVoiceActivation(bool voiceActivation);
    void setDebugMode(bool debugMode);
    void setAccentIntensity(qreal accentIntensity);
    void setMicrophoneVolume(qreal microphoneVolume);
    void setOutputVolume(qreal outputVolume);
    void setInputDevice(const QString &inputDevice);
    void setOutputDevice(const QString &outputDevice);
    void setPushToTalkKey(const QString &pushToTalkKey);
    void setPushToTalkEnabled(bool pushToTalkEnabled);
    void setTransferDirectory(const QString &transferDirectory);
    void setAppearanceMode(const QString &appearanceMode);
    void setAccentColor(const QString &accentColor);
    void setGlassIntensity(qreal glassIntensity);
    void setAnimationIntensity(qreal animationIntensity);
    void setParticlesEnabled(bool particlesEnabled);
    void setOceanVariant(const QString &oceanVariant);
    void setCornerRadius(const QString &cornerRadius);
    void setDensity(const QString &density);
    void setWidgetEnabled(bool widgetEnabled);
    void setWidgetPosition(const QString &widgetPosition);
    void setWidgetShowActivity(bool widgetShowActivity);
    void setWidgetShowAvatar(bool widgetShowAvatar);
    void setWidgetShowCallPresence(bool widgetShowCallPresence);

    /// Replaces every value from an authoritative settings document
    /// (a `settings.get` result or a confirmed `settings.update` echo).
    void applyDocument(const QJsonObject &document);

signals:
    void loadedChanged();
    /// Emitted after a setter changes a value; `key` is the protocol key.
    void settingChanged(const QString &key, const QJsonValue &value);
    /// Emitted once per applied authoritative document (core echo).
    void documentApplied();
    void localeChanged();
    void displayNameChanged();
    void statusMessageChanged();
    void avatarChanged();
    void avatarTypeChanged();
    void higherContrastChanged();
    void backgroundChanged();
    void reducedMotionChanged();
    void startWithSystemChanged();
    void minimizeToTrayChanged();
    void closeToTrayChanged();
    void autoConnectChanged();
    void notificationsEnabledChanged();
    void gameNotificationsChanged();
    void appNotificationsChanged();
    void connectionNotificationsChanged();
    void notificationSoundChanged();
    void messagePreviewsChanged();
    void notifyPartnerOnlineChanged();
    void notifyPartnerAwayChanged();
    void notifyPartnerOfflineChanged();
    void presenceVisibilityChanged();
    void activitySharingChanged();
    void gameVisibilityChanged();
    void deviceVisibilityChanged();
    void voiceActivationChanged();
    void debugModeChanged();
    void accentIntensityChanged();
    void microphoneVolumeChanged();
    void outputVolumeChanged();
    void inputDeviceChanged();
    void outputDeviceChanged();
    void pushToTalkKeyChanged();
    void pushToTalkEnabledChanged();
    void transferDirectoryChanged();
    void appearanceModeChanged();
    void accentColorChanged();
    void glassIntensityChanged();
    void animationIntensityChanged();
    void particlesEnabledChanged();
    void oceanVariantChanged();
    void cornerRadiusChanged();
    void densityChanged();
    void widgetEnabledChanged();
    void widgetPositionChanged();
    void widgetShowActivityChanged();
    void widgetShowAvatarChanged();
    void widgetShowCallPresenceChanged();

private:
    void setLoaded(bool loaded);

    bool m_loaded = false;
    /// True while applyDocument() writes an authoritative document, so the
    /// mirror does not echo values it just received back to the core.
    bool m_applying = false;
    QString m_locale = QStringLiteral("en");
    QString m_displayName;
    QString m_statusMessage;
    QString m_avatar;
    QString m_avatarType = QStringLiteral("image");
    bool m_higherContrast = false;
    bool m_background = true;
    bool m_reducedMotion = false;
    bool m_startWithSystem = true;
    bool m_minimizeToTray = true;
    bool m_closeToTray = true;
    bool m_autoConnect = true;
    bool m_notificationsEnabled = true;
    bool m_gameNotifications = true;
    bool m_appNotifications = true;
    bool m_connectionNotifications = true;
    bool m_notificationSound = true;
    bool m_messagePreviews = true;
    bool m_notifyPartnerOnline = true;
    bool m_notifyPartnerAway = true;
    bool m_notifyPartnerOffline = true;
    bool m_presenceVisibility = true;
    bool m_activitySharing = true;
    bool m_gameVisibility = true;
    bool m_deviceVisibility = true;
    bool m_voiceActivation = true;
    bool m_debugMode = false;
    qreal m_accentIntensity = 0.75;
    qreal m_microphoneVolume = 0.72;
    qreal m_outputVolume = 0.64;
    QString m_inputDevice = QStringLiteral("default-microphone");
    QString m_outputDevice = QStringLiteral("harbor-headphones");
    QString m_pushToTalkKey = QStringLiteral("Space");
    bool m_pushToTalkEnabled = true;
    QString m_transferDirectory;
    QString m_appearanceMode = QStringLiteral("dark");
    QString m_accentColor = QStringLiteral("ocean");
    qreal m_glassIntensity = 1.0;
    qreal m_animationIntensity = 1.0;
    bool m_particlesEnabled = true;
    QString m_oceanVariant = QStringLiteral("lagoon");
    QString m_cornerRadius = QStringLiteral("soft");
    QString m_density = QStringLiteral("comfortable");
    bool m_widgetEnabled = true;
    QString m_widgetPosition = QStringLiteral("bottomRight");
    bool m_widgetShowActivity = true;
    bool m_widgetShowAvatar = true;
    bool m_widgetShowCallPresence = true;
};
