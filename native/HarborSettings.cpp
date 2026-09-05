#include "HarborSettings.h"

namespace {

/// Emits `settingChanged` only when `stored` actually differs from `value`
/// and the write did not come from an authoritative document, keeping the
/// core round-trip free of echo storms and redundant load-time patches.
#define HARBOR_ASSIGN(member, value, key, signal)        \
    do {                                                 \
        const auto next = (value);                       \
        if (member == next)                              \
            return;                                      \
        member = next;                                   \
        if (!m_applying)                                 \
            emit settingChanged(QStringLiteral(key), next); \
        emit signal();                                   \
    } while (false)

} // namespace

HarborSettings::HarborSettings(QObject *parent)
    : QObject(parent)
{
}

bool HarborSettings::loaded() const
{
    return m_loaded;
}

void HarborSettings::setLoaded(bool loaded)
{
    if (m_loaded == loaded)
        return;

    m_loaded = loaded;
    emit loadedChanged();
}

QString HarborSettings::locale() const
{
    return m_locale;
}

QString HarborSettings::displayName() const
{
    return m_displayName;
}

QString HarborSettings::statusMessage() const
{
    return m_statusMessage;
}

QString HarborSettings::avatar() const
{
    return m_avatar;
}

QString HarborSettings::avatarType() const
{
    return m_avatarType;
}

bool HarborSettings::higherContrast() const
{
    return m_higherContrast;
}

bool HarborSettings::background() const
{
    return m_background;
}

bool HarborSettings::reducedMotion() const
{
    return m_reducedMotion;
}

bool HarborSettings::startWithSystem() const
{
    return m_startWithSystem;
}

bool HarborSettings::minimizeToTray() const
{
    return m_minimizeToTray;
}

bool HarborSettings::closeToTray() const
{
    return m_closeToTray;
}

bool HarborSettings::autoConnect() const
{
    return m_autoConnect;
}

bool HarborSettings::notificationsEnabled() const
{
    return m_notificationsEnabled;
}

bool HarborSettings::gameNotifications() const
{
    return m_gameNotifications;
}

bool HarborSettings::appNotifications() const
{
    return m_appNotifications;
}

bool HarborSettings::connectionNotifications() const
{
    return m_connectionNotifications;
}

bool HarborSettings::notificationSound() const
{
    return m_notificationSound;
}

bool HarborSettings::messagePreviews() const
{
    return m_messagePreviews;
}

bool HarborSettings::notifyPartnerOnline() const
{
    return m_notifyPartnerOnline;
}

bool HarborSettings::notifyPartnerAway() const
{
    return m_notifyPartnerAway;
}

bool HarborSettings::notifyPartnerOffline() const
{
    return m_notifyPartnerOffline;
}

bool HarborSettings::presenceVisibility() const
{
    return m_presenceVisibility;
}

bool HarborSettings::activitySharing() const
{
    return m_activitySharing;
}

bool HarborSettings::gameVisibility() const
{
    return m_gameVisibility;
}

bool HarborSettings::deviceVisibility() const
{
    return m_deviceVisibility;
}

bool HarborSettings::voiceActivation() const
{
    return m_voiceActivation;
}

bool HarborSettings::debugMode() const
{
    return m_debugMode;
}

qreal HarborSettings::accentIntensity() const
{
    return m_accentIntensity;
}

qreal HarborSettings::microphoneVolume() const
{
    return m_microphoneVolume;
}

qreal HarborSettings::outputVolume() const
{
    return m_outputVolume;
}

QString HarborSettings::inputDevice() const
{
    return m_inputDevice;
}

QString HarborSettings::outputDevice() const
{
    return m_outputDevice;
}

QString HarborSettings::pushToTalkKey() const
{
    return m_pushToTalkKey;
}

bool HarborSettings::pushToTalkEnabled() const
{
    return m_pushToTalkEnabled;
}

QString HarborSettings::transferDirectory() const
{
    return m_transferDirectory;
}

QString HarborSettings::appearanceMode() const
{
    return m_appearanceMode;
}

QString HarborSettings::accentColor() const
{
    return m_accentColor;
}

qreal HarborSettings::glassIntensity() const
{
    return m_glassIntensity;
}

qreal HarborSettings::animationIntensity() const
{
    return m_animationIntensity;
}

bool HarborSettings::particlesEnabled() const
{
    return m_particlesEnabled;
}

QString HarborSettings::oceanVariant() const
{
    return m_oceanVariant;
}

QString HarborSettings::cornerRadius() const
{
    return m_cornerRadius;
}

QString HarborSettings::density() const
{
    return m_density;
}

bool HarborSettings::widgetEnabled() const
{
    return m_widgetEnabled;
}

QString HarborSettings::widgetPosition() const
{
    return m_widgetPosition;
}

bool HarborSettings::widgetShowActivity() const
{
    return m_widgetShowActivity;
}

bool HarborSettings::widgetShowAvatar() const
{
    return m_widgetShowAvatar;
}

bool HarborSettings::widgetShowCallPresence() const
{
    return m_widgetShowCallPresence;
}

void HarborSettings::setLocale(const QString &locale)
{
    HARBOR_ASSIGN(m_locale, locale, "locale", localeChanged);
}

void HarborSettings::setDisplayName(const QString &displayName)
{
    HARBOR_ASSIGN(m_displayName, displayName, "displayName", displayNameChanged);
}

void HarborSettings::setStatusMessage(const QString &statusMessage)
{
    HARBOR_ASSIGN(m_statusMessage, statusMessage, "statusMessage", statusMessageChanged);
}

void HarborSettings::setAvatar(const QString &avatar)
{
    HARBOR_ASSIGN(m_avatar, avatar, "avatar", avatarChanged);
}

void HarborSettings::setAvatarType(const QString &avatarType)
{
    HARBOR_ASSIGN(m_avatarType, avatarType, "avatarType", avatarTypeChanged);
}

void HarborSettings::setHigherContrast(bool higherContrast)
{
    HARBOR_ASSIGN(m_higherContrast, higherContrast, "higherContrast", higherContrastChanged);
}

void HarborSettings::setBackground(bool background)
{
    HARBOR_ASSIGN(m_background, background, "background", backgroundChanged);
}

void HarborSettings::setReducedMotion(bool reducedMotion)
{
    HARBOR_ASSIGN(m_reducedMotion, reducedMotion, "reducedMotion", reducedMotionChanged);
}

void HarborSettings::setStartWithSystem(bool startWithSystem)
{
    HARBOR_ASSIGN(m_startWithSystem, startWithSystem, "startWithSystem", startWithSystemChanged);
}

void HarborSettings::setMinimizeToTray(bool minimizeToTray)
{
    HARBOR_ASSIGN(m_minimizeToTray, minimizeToTray, "minimizeToTray", minimizeToTrayChanged);
}

void HarborSettings::setCloseToTray(bool closeToTray)
{
    HARBOR_ASSIGN(m_closeToTray, closeToTray, "closeToTray", closeToTrayChanged);
}

void HarborSettings::setAutoConnect(bool autoConnect)
{
    HARBOR_ASSIGN(m_autoConnect, autoConnect, "autoConnect", autoConnectChanged);
}

void HarborSettings::setNotificationsEnabled(bool notificationsEnabled)
{
    HARBOR_ASSIGN(m_notificationsEnabled, notificationsEnabled, "notificationsEnabled", notificationsEnabledChanged);
}

void HarborSettings::setGameNotifications(bool gameNotifications)
{
    HARBOR_ASSIGN(m_gameNotifications, gameNotifications, "gameNotifications", gameNotificationsChanged);
}

void HarborSettings::setAppNotifications(bool appNotifications)
{
    HARBOR_ASSIGN(m_appNotifications, appNotifications, "appNotifications", appNotificationsChanged);
}

void HarborSettings::setConnectionNotifications(bool connectionNotifications)
{
    HARBOR_ASSIGN(m_connectionNotifications, connectionNotifications, "connectionNotifications", connectionNotificationsChanged);
}

void HarborSettings::setNotificationSound(bool notificationSound)
{
    HARBOR_ASSIGN(m_notificationSound, notificationSound, "notificationSound", notificationSoundChanged);
}

void HarborSettings::setMessagePreviews(bool messagePreviews)
{
    HARBOR_ASSIGN(m_messagePreviews, messagePreviews, "messagePreviews", messagePreviewsChanged);
}

void HarborSettings::setNotifyPartnerOnline(bool notifyPartnerOnline)
{
    HARBOR_ASSIGN(m_notifyPartnerOnline, notifyPartnerOnline, "notifyPartnerOnline", notifyPartnerOnlineChanged);
}

void HarborSettings::setNotifyPartnerAway(bool notifyPartnerAway)
{
    HARBOR_ASSIGN(m_notifyPartnerAway, notifyPartnerAway, "notifyPartnerAway", notifyPartnerAwayChanged);
}

void HarborSettings::setNotifyPartnerOffline(bool notifyPartnerOffline)
{
    HARBOR_ASSIGN(m_notifyPartnerOffline, notifyPartnerOffline, "notifyPartnerOffline", notifyPartnerOfflineChanged);
}

void HarborSettings::setPresenceVisibility(bool presenceVisibility)
{
    HARBOR_ASSIGN(m_presenceVisibility, presenceVisibility, "presenceVisibility", presenceVisibilityChanged);
}

void HarborSettings::setActivitySharing(bool activitySharing)
{
    HARBOR_ASSIGN(m_activitySharing, activitySharing, "activitySharing", activitySharingChanged);
}

void HarborSettings::setGameVisibility(bool gameVisibility)
{
    HARBOR_ASSIGN(m_gameVisibility, gameVisibility, "gameVisibility", gameVisibilityChanged);
}

void HarborSettings::setDeviceVisibility(bool deviceVisibility)
{
    HARBOR_ASSIGN(m_deviceVisibility, deviceVisibility, "deviceVisibility", deviceVisibilityChanged);
}

void HarborSettings::setVoiceActivation(bool voiceActivation)
{
    HARBOR_ASSIGN(m_voiceActivation, voiceActivation, "voiceActivation", voiceActivationChanged);
}

void HarborSettings::setDebugMode(bool debugMode)
{
    HARBOR_ASSIGN(m_debugMode, debugMode, "debugMode", debugModeChanged);
}

void HarborSettings::setAccentIntensity(qreal accentIntensity)
{
    if (qFuzzyCompare(m_accentIntensity, accentIntensity))
        return;
    m_accentIntensity = accentIntensity;
    if (!m_applying)
        emit settingChanged(QStringLiteral("accentIntensity"), accentIntensity);
    emit accentIntensityChanged();
}

void HarborSettings::setMicrophoneVolume(qreal microphoneVolume)
{
    if (qFuzzyCompare(m_microphoneVolume, microphoneVolume))
        return;
    m_microphoneVolume = microphoneVolume;
    if (!m_applying)
        emit settingChanged(QStringLiteral("microphoneVolume"), microphoneVolume);
    emit microphoneVolumeChanged();
}

void HarborSettings::setOutputVolume(qreal outputVolume)
{
    if (qFuzzyCompare(m_outputVolume, outputVolume))
        return;
    m_outputVolume = outputVolume;
    if (!m_applying)
        emit settingChanged(QStringLiteral("outputVolume"), outputVolume);
    emit outputVolumeChanged();
}

void HarborSettings::setInputDevice(const QString &inputDevice)
{
    HARBOR_ASSIGN(m_inputDevice, inputDevice, "inputDevice", inputDeviceChanged);
}

void HarborSettings::setOutputDevice(const QString &outputDevice)
{
    HARBOR_ASSIGN(m_outputDevice, outputDevice, "outputDevice", outputDeviceChanged);
}

void HarborSettings::setPushToTalkKey(const QString &pushToTalkKey)
{
    HARBOR_ASSIGN(m_pushToTalkKey, pushToTalkKey, "pushToTalkKey", pushToTalkKeyChanged);
}

void HarborSettings::setPushToTalkEnabled(bool pushToTalkEnabled)
{
    HARBOR_ASSIGN(m_pushToTalkEnabled, pushToTalkEnabled, "pushToTalkEnabled", pushToTalkEnabledChanged);
}

void HarborSettings::setTransferDirectory(const QString &transferDirectory)
{
    HARBOR_ASSIGN(m_transferDirectory, transferDirectory, "transferDirectory", transferDirectoryChanged);
}

void HarborSettings::setAppearanceMode(const QString &appearanceMode)
{
    HARBOR_ASSIGN(m_appearanceMode, appearanceMode, "appearanceMode", appearanceModeChanged);
}

void HarborSettings::setAccentColor(const QString &accentColor)
{
    HARBOR_ASSIGN(m_accentColor, accentColor, "accentColor", accentColorChanged);
}

void HarborSettings::setGlassIntensity(qreal glassIntensity)
{
    if (qFuzzyCompare(m_glassIntensity, glassIntensity))
        return;
    m_glassIntensity = glassIntensity;
    if (!m_applying)
        emit settingChanged(QStringLiteral("glassIntensity"), glassIntensity);
    emit glassIntensityChanged();
}

void HarborSettings::setAnimationIntensity(qreal animationIntensity)
{
    if (qFuzzyCompare(m_animationIntensity, animationIntensity))
        return;
    m_animationIntensity = animationIntensity;
    if (!m_applying)
        emit settingChanged(QStringLiteral("animationIntensity"), animationIntensity);
    emit animationIntensityChanged();
}

void HarborSettings::setParticlesEnabled(bool particlesEnabled)
{
    HARBOR_ASSIGN(m_particlesEnabled, particlesEnabled, "particlesEnabled", particlesEnabledChanged);
}

void HarborSettings::setOceanVariant(const QString &oceanVariant)
{
    HARBOR_ASSIGN(m_oceanVariant, oceanVariant, "oceanVariant", oceanVariantChanged);
}

void HarborSettings::setCornerRadius(const QString &cornerRadius)
{
    HARBOR_ASSIGN(m_cornerRadius, cornerRadius, "cornerRadius", cornerRadiusChanged);
}

void HarborSettings::setDensity(const QString &density)
{
    HARBOR_ASSIGN(m_density, density, "density", densityChanged);
}

void HarborSettings::setWidgetEnabled(bool widgetEnabled)
{
    HARBOR_ASSIGN(m_widgetEnabled, widgetEnabled, "widgetEnabled", widgetEnabledChanged);
}

void HarborSettings::setWidgetPosition(const QString &widgetPosition)
{
    HARBOR_ASSIGN(m_widgetPosition, widgetPosition, "widgetPosition", widgetPositionChanged);
}

void HarborSettings::setWidgetShowActivity(bool widgetShowActivity)
{
    HARBOR_ASSIGN(m_widgetShowActivity, widgetShowActivity, "widgetShowActivity", widgetShowActivityChanged);
}

void HarborSettings::setWidgetShowAvatar(bool widgetShowAvatar)
{
    HARBOR_ASSIGN(m_widgetShowAvatar, widgetShowAvatar, "widgetShowAvatar", widgetShowAvatarChanged);
}

void HarborSettings::setWidgetShowCallPresence(bool widgetShowCallPresence)
{
    HARBOR_ASSIGN(m_widgetShowCallPresence, widgetShowCallPresence, "widgetShowCallPresence", widgetShowCallPresenceChanged);
}

void HarborSettings::applyDocument(const QJsonObject &document)
{
    m_applying = true;
    const auto apply = [&document](auto setter, const QLatin1String key) {
        const QJsonValue value = document.value(key);
        if (!value.isUndefined())
            setter(value);
    };

    apply([this](const QJsonValue &v) { setLocale(v.toString(m_locale)); }, QLatin1String("locale"));
    apply([this](const QJsonValue &v) { setDisplayName(v.toString(m_displayName)); }, QLatin1String("displayName"));
    apply([this](const QJsonValue &v) { setStatusMessage(v.toString(m_statusMessage)); }, QLatin1String("statusMessage"));
    apply([this](const QJsonValue &v) { setAvatar(v.toString(m_avatar)); }, QLatin1String("avatar"));
    apply([this](const QJsonValue &v) { setAvatarType(v.toString(m_avatarType)); }, QLatin1String("avatarType"));
    apply([this](const QJsonValue &v) { setHigherContrast(v.toBool(m_higherContrast)); }, QLatin1String("higherContrast"));
    apply([this](const QJsonValue &v) { setBackground(v.toBool(m_background)); }, QLatin1String("background"));
    apply([this](const QJsonValue &v) { setReducedMotion(v.toBool(m_reducedMotion)); }, QLatin1String("reducedMotion"));
    apply([this](const QJsonValue &v) { setStartWithSystem(v.toBool(m_startWithSystem)); }, QLatin1String("startWithSystem"));
    apply([this](const QJsonValue &v) { setMinimizeToTray(v.toBool(m_minimizeToTray)); }, QLatin1String("minimizeToTray"));
    apply([this](const QJsonValue &v) { setCloseToTray(v.toBool(m_closeToTray)); }, QLatin1String("closeToTray"));
    apply([this](const QJsonValue &v) { setAutoConnect(v.toBool(m_autoConnect)); }, QLatin1String("autoConnect"));
    apply([this](const QJsonValue &v) { setNotificationsEnabled(v.toBool(m_notificationsEnabled)); }, QLatin1String("notificationsEnabled"));
    apply([this](const QJsonValue &v) { setGameNotifications(v.toBool(m_gameNotifications)); }, QLatin1String("gameNotifications"));
    apply([this](const QJsonValue &v) { setAppNotifications(v.toBool(m_appNotifications)); }, QLatin1String("appNotifications"));
    apply([this](const QJsonValue &v) { setConnectionNotifications(v.toBool(m_connectionNotifications)); }, QLatin1String("connectionNotifications"));
    apply([this](const QJsonValue &v) { setNotificationSound(v.toBool(m_notificationSound)); }, QLatin1String("notificationSound"));
    apply([this](const QJsonValue &v) { setMessagePreviews(v.toBool(m_messagePreviews)); }, QLatin1String("messagePreviews"));
    apply([this](const QJsonValue &v) { setNotifyPartnerOnline(v.toBool(m_notifyPartnerOnline)); }, QLatin1String("notifyPartnerOnline"));
    apply([this](const QJsonValue &v) { setNotifyPartnerAway(v.toBool(m_notifyPartnerAway)); }, QLatin1String("notifyPartnerAway"));
    apply([this](const QJsonValue &v) { setNotifyPartnerOffline(v.toBool(m_notifyPartnerOffline)); }, QLatin1String("notifyPartnerOffline"));
    apply([this](const QJsonValue &v) { setPresenceVisibility(v.toBool(m_presenceVisibility)); }, QLatin1String("presenceVisibility"));
    apply([this](const QJsonValue &v) { setActivitySharing(v.toBool(m_activitySharing)); }, QLatin1String("activitySharing"));
    apply([this](const QJsonValue &v) { setGameVisibility(v.toBool(m_gameVisibility)); }, QLatin1String("gameVisibility"));
    apply([this](const QJsonValue &v) { setDeviceVisibility(v.toBool(m_deviceVisibility)); }, QLatin1String("deviceVisibility"));
    apply([this](const QJsonValue &v) { setVoiceActivation(v.toBool(m_voiceActivation)); }, QLatin1String("voiceActivation"));
    apply([this](const QJsonValue &v) { setDebugMode(v.toBool(m_debugMode)); }, QLatin1String("debugMode"));
    apply([this](const QJsonValue &v) { setAccentIntensity(v.toDouble(m_accentIntensity)); }, QLatin1String("accentIntensity"));
    apply([this](const QJsonValue &v) { setMicrophoneVolume(v.toDouble(m_microphoneVolume)); }, QLatin1String("microphoneVolume"));
    apply([this](const QJsonValue &v) { setOutputVolume(v.toDouble(m_outputVolume)); }, QLatin1String("outputVolume"));
    apply([this](const QJsonValue &v) { setInputDevice(v.toString(m_inputDevice)); }, QLatin1String("inputDevice"));
    apply([this](const QJsonValue &v) { setOutputDevice(v.toString(m_outputDevice)); }, QLatin1String("outputDevice"));
    apply([this](const QJsonValue &v) { setPushToTalkKey(v.toString(m_pushToTalkKey)); }, QLatin1String("pushToTalkKey"));
    apply([this](const QJsonValue &v) { setPushToTalkEnabled(v.toBool(m_pushToTalkEnabled)); }, QLatin1String("pushToTalkEnabled"));
    apply([this](const QJsonValue &v) { setTransferDirectory(v.toString(m_transferDirectory)); }, QLatin1String("transferDirectory"));
    apply([this](const QJsonValue &v) { setAppearanceMode(v.toString(m_appearanceMode)); }, QLatin1String("appearanceMode"));
    apply([this](const QJsonValue &v) { setAccentColor(v.toString(m_accentColor)); }, QLatin1String("accentColor"));
    apply([this](const QJsonValue &v) { setGlassIntensity(v.toDouble(m_glassIntensity)); }, QLatin1String("glassIntensity"));
    apply([this](const QJsonValue &v) { setAnimationIntensity(v.toDouble(m_animationIntensity)); }, QLatin1String("animationIntensity"));
    apply([this](const QJsonValue &v) { setParticlesEnabled(v.toBool(m_particlesEnabled)); }, QLatin1String("particlesEnabled"));
    apply([this](const QJsonValue &v) { setOceanVariant(v.toString(m_oceanVariant)); }, QLatin1String("oceanVariant"));
    apply([this](const QJsonValue &v) { setCornerRadius(v.toString(m_cornerRadius)); }, QLatin1String("cornerRadius"));
    apply([this](const QJsonValue &v) { setDensity(v.toString(m_density)); }, QLatin1String("density"));
    apply([this](const QJsonValue &v) { setWidgetEnabled(v.toBool(m_widgetEnabled)); }, QLatin1String("widgetEnabled"));
    apply([this](const QJsonValue &v) { setWidgetPosition(v.toString(m_widgetPosition)); }, QLatin1String("widgetPosition"));
    apply([this](const QJsonValue &v) { setWidgetShowActivity(v.toBool(m_widgetShowActivity)); }, QLatin1String("widgetShowActivity"));
    apply([this](const QJsonValue &v) { setWidgetShowAvatar(v.toBool(m_widgetShowAvatar)); }, QLatin1String("widgetShowAvatar"));
    apply([this](const QJsonValue &v) { setWidgetShowCallPresence(v.toBool(m_widgetShowCallPresence)); }, QLatin1String("widgetShowCallPresence"));
    m_applying = false;

    setLoaded(true);
    emit documentApplied();
}
