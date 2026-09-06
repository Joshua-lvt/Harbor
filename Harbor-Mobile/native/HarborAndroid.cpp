// Android facade: JNI in, typed Qt properties out. Every poll tolerates
// a missing class, method, or permission and leaves the honest unknown in
// place — the QML shell renders "unavailable", never a guess.
#include "HarborAndroid.h"

#include <QCoreApplication>
#include <QDateTime>
#include <QDebug>
#include <QFile>
#include <QFileInfo>
#include <QDir>
#include <QCryptographicHash>
#include <QStandardPaths>

#ifndef Q_OS_ANDROID
#include <QClipboard>
#include <QGuiApplication>
#endif

#ifdef Q_OS_ANDROID
#include <jni.h>
#include <QJniObject>
#endif

namespace {

#ifdef Q_OS_ANDROID
HarborAndroid *g_android = nullptr;

QJniObject activity()
{
    return QJniObject(QNativeInterface::QAndroidApplication::context());
}

QString jstringToQString(const QJniObject &value)
{
    return value.isValid() ? value.toString() : QString();
}

#else
int activity()
{
    return 0;
}
#endif

} // namespace

#ifdef Q_OS_ANDROID
// These are deliberately exported JNI entry points rather than methods
// registered from the HarborAndroid constructor.  Qt can invoke an Activity's
// onResume() before main() has constructed the C++ facade on a cold launch;
// dynamic registration in that constructor therefore races the first Java
// callback.  Name-based JNI lookup is safe in either order and also works for
// callbacks originating in the notification-listener process.
extern "C" JNIEXPORT void JNICALL
Java_org_harbor_mobile_HarborMobileActivity_nativePhoneNotification(
    JNIEnv *, jclass, jstring app, jstring title, jstring text, jlong postedAt)
{
    if (g_android) {
        emit g_android->phoneNotification(QJniObject(app).toString(),
                                          QJniObject(title).toString(),
                                          QJniObject(text).toString(), postedAt);
    }
}

extern "C" JNIEXPORT void JNICALL
Java_org_harbor_mobile_HarborMobileActivity_nativePermissionChanged(JNIEnv *, jclass)
{
    if (g_android)
        g_android->refresh();
}
#endif

HarborAndroid::HarborAndroid(QObject *parent)
    : QObject(parent)
{
#ifdef Q_OS_ANDROID
    g_android = this;
#endif
}

void HarborAndroid::refresh()
{
    pollBattery();
    pollPermissions();
    pollUsage();
    pollLocation();
    pollTailscale();
    emit platformChanged();
}

void HarborAndroid::openSystemSettings(const QString &page)
{
#ifdef Q_OS_ANDROID
    QJniObject jPage = QJniObject::fromString(page);
    QJniObject::callStaticMethod<void>(
        "org/harbor/mobile/HarborMobileActivity", "openSystemSettings",
        "(Landroid/content/Context;Ljava/lang/String;)V", activity().object(),
        jPage.object<jstring>());
#else
    Q_UNUSED(page);
#endif
}

void HarborAndroid::setLocationSharing(bool on)
{
#ifdef Q_OS_ANDROID
    pollPermissions();
    if (on && m_locationPermission != QStringLiteral("granted")) {
        QJniObject::callStaticMethod<void>(
            "org/harbor/mobile/HarborMobileActivity", "requestLocationPermission",
            "(Lorg/qtproject/qt/android/bindings/QtActivity;)V", activity().object());
        emit platformChanged();
        return;
    }
    QJniObject::callStaticMethod<void>("org/harbor/mobile/HarborMobileActivity",
                                       on ? "startLocation" : "stopLocation",
                                       "(Landroid/content/Context;)V", activity().object());
    pollLocation();
    emit platformChanged();
#else
    Q_UNUSED(on);
#endif
}

void HarborAndroid::syncLocationService(bool on)
{
#ifdef Q_OS_ANDROID
    QJniObject::callStaticMethod<void>(
        "org/harbor/mobile/HarborMobileActivity", on ? "startLocation" : "stopLocation",
        "(Landroid/content/Context;)V", activity().object());
#else
    Q_UNUSED(on);
#endif
}

void HarborAndroid::setNotificationMirroring(bool on)
{
#ifdef Q_OS_ANDROID
    QJniObject::callStaticMethod<void>("org/harbor/mobile/HarborPhoneNotifications",
                                       "setSharingEnabled", "(Z)V", on);
#else
    Q_UNUSED(on);
#endif
}

bool HarborAndroid::ensureCallAudio()
{
#ifdef Q_OS_ANDROID
    QJniObject context = activity();
    const bool granted = QJniObject::callStaticMethod<jboolean>(
        "org/harbor/mobile/HarborMobileActivity", "hasMicrophonePermission",
        "(Landroid/content/Context;)Z", context.object());
    if (!granted) {
        QJniObject::callStaticMethod<void>(
            "org/harbor/mobile/HarborMobileActivity", "requestMicrophonePermission",
            "(Lorg/qtproject/qt/android/bindings/QtActivity;)V", context.object());
        m_microphonePermission = QStringLiteral("denied");
        emit platformChanged();
        return false;
    }
    QJniObject::callStaticMethod<void>(
        "org/harbor/mobile/HarborCallService", "start",
        "(Landroid/content/Context;)V", context.object());
    m_microphonePermission = QStringLiteral("granted");
    emit platformChanged();
    return true;
#else
    return false;
#endif
}

void HarborAndroid::stopCallAudio()
{
#ifdef Q_OS_ANDROID
    QJniObject::callStaticMethod<void>(
        "org/harbor/mobile/HarborCallService", "stop",
        "(Landroid/content/Context;)V", activity().object());
#endif
}

void HarborAndroid::requestOwnNotificationPermission()
{
#ifdef Q_OS_ANDROID
    QJniObject::callStaticMethod<void>(
        "org/harbor/mobile/HarborMobileActivity", "requestPostNotifications",
        "(Lorg/qtproject/qt/android/bindings/QtActivity;)V", activity().object());
#endif
}

void HarborAndroid::checkForUpdates()
{
#ifdef Q_OS_ANDROID
    QJniObject::callStaticMethod<void>(
        "org/harbor/mobile/HarborUpdater", "checkForUpdates",
        "(Landroid/content/Context;)V", activity().object());
#endif
}

QString HarborAndroid::updateState()
{
#ifdef Q_OS_ANDROID
    QJniObject state = QJniObject::callStaticObjectMethod(
        "org/harbor/mobile/HarborUpdater", "updateState",
        "(Landroid/content/Context;)Ljava/lang/String;", activity().object());
    return state.toString();
#else
    return QStringLiteral("{\"status\":\"idle\"}");
#endif
}

void HarborAndroid::downloadUpdate(const QString &url, const QString &sha)
{
#ifdef Q_OS_ANDROID
    QJniObject jUrl = QJniObject::fromString(url);
    QJniObject jSha = QJniObject::fromString(sha);
    QJniObject::callStaticMethod<void>(
        "org/harbor/mobile/HarborUpdater", "downloadUpdate",
        "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;)V", activity().object(),
        jUrl.object<jstring>(), jSha.object<jstring>());
#else
    Q_UNUSED(url);
    Q_UNUSED(sha);
#endif
}

void HarborAndroid::installUpdate()
{
#ifdef Q_OS_ANDROID
    QJniObject::callStaticMethod<void>(
        "org/harbor/mobile/HarborUpdater", "installUpdate",
        "(Landroid/content/Context;)V", activity().object());
#endif
}

QString HarborAndroid::prepareMediaWorker() const
{
#ifdef Q_OS_ANDROID
    // The resource is generated by qt_add_resources and is deliberately
    // copied into filesDir rather than executed from the APK zip.
    QFile source(QStringLiteral(":/harbor-media/harbor-media"));
    if (!source.open(QIODevice::ReadOnly))
        return {};
    const QByteArray bytes = source.readAll();
    if (bytes.isEmpty())
        return {};
    const QString dir = QStandardPaths::writableLocation(QStandardPaths::AppDataLocation);
    if (dir.isEmpty() || !QDir().mkpath(dir))
        return {};
    const QString path = QDir(dir).filePath(QStringLiteral("harbor-media"));
    QFileInfo info(path);
    // Size alone is not an adequate version check: two app updates can
    // produce workers with the same length, leaving the old executable in
    // app-private storage.  Hash the existing asset before deciding that it
    // is safe to reuse it; this also makes an interrupted staged copy
    // self-healing on the next launch.
    bool current = false;
    if (info.isFile() && info.size() == bytes.size()) {
        QFile installed(path);
        if (installed.open(QIODevice::ReadOnly)) {
            current = QCryptographicHash::hash(installed.readAll(),
                                               QCryptographicHash::Sha256)
                      == QCryptographicHash::hash(bytes, QCryptographicHash::Sha256);
        }
    }
    if (!current) {
        const QString temporary = path + QStringLiteral(".new");
        QFile staged(temporary);
        if (!staged.open(QIODevice::WriteOnly | QIODevice::Truncate)
            || staged.write(bytes) != bytes.size()) {
            staged.close();
            QFile::remove(temporary);
            return {};
        }
        staged.close();
        QFile::remove(path);
        if (!QFile::rename(temporary, path))
            return {};
    }
    QFile::setPermissions(path, QFileDevice::ReadOwner | QFileDevice::WriteOwner
                                  | QFileDevice::ExeOwner);
    return QFileInfo(path).isExecutable() ? path : QString();
#else
    return {};
#endif
}

void HarborAndroid::updatePresenceBar(const QString &name, const QString &state,
                                      const QString &detail)
{
#ifdef Q_OS_ANDROID
    QJniObject jName = QJniObject::fromString(name);
    QJniObject jState = QJniObject::fromString(state);
    QJniObject jDetail = QJniObject::fromString(detail);
    QJniObject::callStaticMethod<void>(
        "org/harbor/mobile/HarborPresenceBar", "update",
        "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V",
        activity().object(), jName.object<jstring>(), jState.object<jstring>(),
        jDetail.object<jstring>());
#else
    Q_UNUSED(name);
    Q_UNUSED(state);
    Q_UNUSED(detail);
#endif
}

void HarborAndroid::postHarborNotification(const QString &title, const QString &text)
{
#ifdef Q_OS_ANDROID
    QJniObject jTitle = QJniObject::fromString(title);
    QJniObject jText = QJniObject::fromString(text);
    QJniObject::callStaticMethod<void>(
        "org/harbor/mobile/HarborNotifier", "post",
        "(Landroid/content/Context;Ljava/lang/String;Ljava/lang/String;)V", activity().object(),
        jTitle.object<jstring>(), jText.object<jstring>());
#else
    Q_UNUSED(title);
    Q_UNUSED(text);
#endif
}

void HarborAndroid::hidePresenceBar()
{
#ifdef Q_OS_ANDROID
    QJniObject::callStaticMethod<void>(
        "org/harbor/mobile/HarborPresenceBar", "stop",
        "(Landroid/content/Context;)V", activity().object());
#endif
}

void HarborAndroid::pollBattery()
{
#ifdef Q_OS_ANDROID
    QJniObject context = activity();
    const int percent = QJniObject::callStaticMethod<jint>(
        "org/harbor/mobile/HarborBattery", "capacityPercent",
        "(Landroid/content/Context;)I", context.object());
    const bool charging = QJniObject::callStaticMethod<jboolean>(
        "org/harbor/mobile/HarborBattery", "charging", "(Landroid/content/Context;)Z",
        context.object());
    m_batteryAvailable = percent >= 0 && percent <= 100;
    m_batteryPercent = m_batteryAvailable ? percent : 0;
    m_batteryCharging = m_batteryAvailable && charging;
#else
    m_batteryAvailable = false;
#endif
}

void HarborAndroid::pollPermissions()
{
#ifdef Q_OS_ANDROID
    QJniObject context = activity();
    auto triState = [](bool granted, bool known) {
        return known ? (granted ? QStringLiteral("granted") : QStringLiteral("denied"))
                     : QStringLiteral("unknown");
    };
    // Usage access: checkOpNoThrow answers; a missing manager is unknown.
    const int usageMode = QJniObject::callStaticMethod<jint>(
        "org/harbor/mobile/HarborMobileActivity", "usageAccessMode",
        "(Landroid/content/Context;)I", context.object());
    m_usagePermission = usageMode < 0 ? QStringLiteral("unknown")
                                      : triState(usageMode == 0, true);
    const bool locationGranted = QJniObject::callStaticMethod<jboolean>(
        "org/harbor/mobile/HarborMobileActivity", "hasLocationPermission",
        "(Landroid/content/Context;)Z", context.object());
    m_locationPermission = triState(locationGranted, true);
    const bool backgroundLocationGranted = QJniObject::callStaticMethod<jboolean>(
        "org/harbor/mobile/HarborMobileActivity", "hasBackgroundLocation",
        "(Landroid/content/Context;)Z", context.object());
    m_backgroundLocationPermission = triState(backgroundLocationGranted, true);
    const bool listenerGranted = QJniObject::callStaticMethod<jboolean>(
        "org/harbor/mobile/HarborPhoneNotifications", "hasAccess",
        "(Landroid/content/Context;)Z", context.object());
    m_notificationPermission = triState(listenerGranted, true);
    const bool microphoneGranted = QJniObject::callStaticMethod<jboolean>(
        "org/harbor/mobile/HarborMobileActivity", "hasMicrophonePermission",
        "(Landroid/content/Context;)Z", context.object());
    m_microphonePermission = triState(microphoneGranted, true);
    const bool ownNotificationsGranted = QJniObject::callStaticMethod<jboolean>(
        "org/harbor/mobile/HarborMobileActivity", "hasPostNotificationsPermission",
        "(Landroid/content/Context;)Z", context.object());
    m_ownNotificationPermission = triState(ownNotificationsGranted, true);
    const bool batteryExempt = QJniObject::callStaticMethod<jboolean>(
        "org/harbor/mobile/HarborMobileActivity", "ignoresBatteryOptimizations",
        "(Landroid/content/Context;)Z", context.object());
    m_batteryOptimizationPermission = triState(batteryExempt, true);
#else
    m_usagePermission = m_locationPermission = m_notificationPermission =
        QStringLiteral("unknown");
    m_microphonePermission = m_ownNotificationPermission = QStringLiteral("unknown");
    m_backgroundLocationPermission = m_batteryOptimizationPermission = QStringLiteral("unknown");
#endif
}

void HarborAndroid::pollTailscale()
{
#ifdef Q_OS_ANDROID
    QJniObject context = activity();
    m_tailscaleInstalled = QJniObject::callStaticMethod<jboolean>(
        "org/harbor/mobile/HarborMobileActivity", "isTailscaleInstalled",
        "(Landroid/content/Context;)Z", context.object());
#else
    // Desktop preview: no gate.
    m_tailscaleInstalled = true;
#endif
}

void HarborAndroid::copyText(const QString &text)
{
#ifdef Q_OS_ANDROID
    QJniObject context = activity();
    QJniObject jText = QJniObject::fromString(text);
    QJniObject::callStaticMethod<void>(
        "org/harbor/mobile/HarborMobileActivity", "copyText",
        "(Landroid/content/Context;Ljava/lang/String;)V", context.object(),
        jText.object<jstring>());
#else
    if (QGuiApplication::instance() != nullptr)
        QGuiApplication::clipboard()->setText(text);
#endif
}

void HarborAndroid::openTailscaleStore()
{
#ifdef Q_OS_ANDROID
    QJniObject context = activity();
    QJniObject::callStaticMethod<void>(
        "org/harbor/mobile/HarborMobileActivity", "openTailscaleStore",
        "(Landroid/content/Context;)V", context.object());
#endif
}

void HarborAndroid::pollUsage()
{
    if (m_usagePermission != QStringLiteral("granted")) {
        m_phoneActivity = QStringLiteral("offline");
        m_currentApp.clear();
        m_lastActiveText.clear();
        m_lastActiveAt = 0;
        return;
    }
#ifdef Q_OS_ANDROID
    QJniObject context = activity();
    QJniObject package = QJniObject::callStaticObjectMethod(
        "org/harbor/mobile/HarborUsage", "foregroundPackage",
        "(Landroid/content/Context;)Ljava/lang/String;", context.object());
    QJniObject label;
    if (package.isValid()) {
        label = QJniObject::callStaticObjectMethod(
            "org/harbor/mobile/HarborUsage", "appLabel",
            "(Landroid/content/Context;Ljava/lang/String;)Ljava/lang/String;", context.object(),
            package.object<jstring>());
    }
    const QString app = jstringToQString(label);
    if (!app.isEmpty()) {
        m_phoneActivity = QStringLiteral("active");
        m_currentApp = app;
        m_lastActiveText = tr("Last active: just now");
        const jlong lastUsedMillis = QJniObject::callStaticMethod<jlong>(
            "org/harbor/mobile/HarborUsage", "lastUsedMillis",
            "(Landroid/content/Context;Ljava/lang/String;)J", context.object(),
            package.object<jstring>());
        m_lastActiveAt = lastUsedMillis > 0 ? lastUsedMillis / 1000 : 0;
    } else {
        m_phoneActivity = QStringLiteral("idle");
        m_currentApp.clear();
        m_lastActiveAt = 0;
    }
#endif
}

void HarborAndroid::pollLocation()
{
#ifdef Q_OS_ANDROID
    QJniObject context = activity();
    const bool tracking = QJniObject::callStaticMethod<jboolean>(
        "org/harbor/mobile/HarborMobileActivity", "isLocationActive", "()Z");
    m_locationAvailable = tracking && m_locationPermission == QStringLiteral("granted");
    if (m_locationAvailable) {
        const double lat = QJniObject::callStaticMethod<jdouble>(
            "org/harbor/mobile/HarborMobileActivity", "lastLatitude", "()D");
        const double lon = QJniObject::callStaticMethod<jdouble>(
            "org/harbor/mobile/HarborMobileActivity", "lastLongitude", "()D");
        const jlong updated = QJniObject::callStaticMethod<jlong>(
            "org/harbor/mobile/HarborMobileActivity", "lastFixAgeSeconds", "()J");
        const jfloat accuracy = QJniObject::callStaticMethod<jfloat>(
            "org/harbor/mobile/HarborMobileActivity", "lastAccuracyMeters", "()F");
        m_locationLatitude = lat;
        m_locationLongitude = lon;
        m_locationAccuracyMeters = accuracy;
        m_locationUpdatedAt = updated >= 0
            ? QDateTime::currentSecsSinceEpoch() - updated : 0;
        m_locationText = QStringLiteral("%1, %2").arg(lat, 0, 'f', 5).arg(lon, 0, 'f', 5);
        m_locationUpdatedText =
            updated < 15 ? tr("Updated just now") : tr("Updated %1s ago").arg(updated);
    } else {
        m_locationText.clear();
        m_locationUpdatedText.clear();
        m_locationLatitude = 0.0;
        m_locationLongitude = 0.0;
        m_locationAccuracyMeters = -1.0;
        m_locationUpdatedAt = 0;
    }
#else
    m_locationAvailable = false;
#endif
}
