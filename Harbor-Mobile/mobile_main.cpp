// Harbor Mobile entry: one window, one in-process core, one Android
// facade. No developer panels, no mock providers — production only.
#include <QDir>
#include <QGuiApplication>
#include <QQmlApplicationEngine>
#include <QQmlContext>
#include <QStandardPaths>
#include <QVariantMap>

#include <cstring>

#include "HarborAndroid.h"
#include "HarborCoreAdapter.h"

#ifdef Q_OS_ANDROID
#include <sys/system_properties.h>

static QString androidProperty(const char *name)
{
    char value[PROP_VALUE_MAX] = {};
    const int length = __system_property_get(name, value);
    return length > 0 ? QString::fromLatin1(value, length) : QString();
}
#endif

static QString diagnosticValue(const char *name)
{
    QString value = qEnvironmentVariable(name).trimmed();
#ifdef Q_OS_ANDROID
    // Android app processes are forked from Zygote, so shell environment
    // assignments before `am start` do not reach the app. Debug system
    // properties make the same switches usable on a device/emulator without
    // changing the production default. Both the verbatim env-style name
    // (debug.harbor_render_backend) and the dotted alias documented in the
    // README (debug.harbor.render_backend) are accepted.
    if (value.isEmpty()) {
        const QByteArray base = QByteArray(name).toLower();
        value = androidProperty(("debug." + base).constData());
        if (value.isEmpty() && base.startsWith("harbor_")) {
            const QByteArray dotted =
                QByteArray("debug.harbor.") + base.mid(int(strlen("harbor_")));
            value = androidProperty(dotted.constData());
        }
    }
#endif
    return value.toLower();
}

static bool diagnosticFlag(const char *name)
{
    const QString value = diagnosticValue(name);
    return value == QLatin1String("1") || value == QLatin1String("true")
        || value == QLatin1String("yes") || value == QLatin1String("on");
}

int main(int argc, char *argv[])
{
    const bool disableEffects = diagnosticFlag("HARBOR_DISABLE_EFFECTS");
    QString renderBackend = diagnosticValue("HARBOR_RENDER_BACKEND");

    // The Android Material style layers button/toolbar backgrounds through
    // Ripple/ElevationEffect. That pipeline produced invalid triangles and
    // torn controls on the Android emulator. Harbor's mobile controls draw
    // explicit flat surfaces, so diagnostics can safely use Basic (no
    // style-level effects) while leaving the normal production style alone.
    if (disableEffects) {
        qputenv("QT_QUICK_CONTROLS_STYLE", "Basic");
        qputenv("QSG_NO_DEPTH", "1");
        qputenv("QSG_NO_STENCIL", "1");
        qunsetenv("QSG_RHI_BACKEND");
    } else {
#ifdef Q_OS_ANDROID
        // The mobile shell uses a dark palette. Qt Quick Controls otherwise
        // chooses the Android Material light theme, whose #fffbfe surface
        // leaks through the shell and can look split between two themes.
        qputenv("QT_QUICK_CONTROLS_STYLE", "Material");
        qputenv("QT_QUICK_CONTROLS_MATERIAL_THEME", "Dark");
#endif
    }

    // HARBOR_RENDER_BACKEND is only a diagnostic switch. Production remains
    // on Qt's Android default so we do not hide a driver/scene-graph defect.
    if (renderBackend == QLatin1String("software")) {
        qputenv("QT_QUICK_BACKEND", "software");
        qunsetenv("QSG_RHI_BACKEND");
    } else if (renderBackend == QLatin1String("opengl")
               || renderBackend == QLatin1String("gles")) {
        qputenv("QSG_RHI_BACKEND", "opengl");
        qunsetenv("QT_QUICK_BACKEND");
    } else if (renderBackend == QLatin1String("vulkan")) {
        qputenv("QSG_RHI_BACKEND", "vulkan");
        qunsetenv("QT_QUICK_BACKEND");
    } else {
        renderBackend = QStringLiteral("default");
        qunsetenv("QT_QUICK_BACKEND");
        qunsetenv("QSG_RHI_BACKEND");
    }

    if (disableEffects || renderBackend != QLatin1String("default"))
        qputenv("QSG_INFO", "1");

    QGuiApplication app(argc, argv);
    app.setApplicationName(QStringLiteral("Harbor"));
    app.setOrganizationName(QStringLiteral("Harbor"));

    const QString stateDir =
        QStandardPaths::writableLocation(QStandardPaths::AppDataLocation);
    QDir().mkpath(stateDir);

    HarborCoreAdapter core(stateDir);
    HarborAndroid platform;
    const QString mediaWorker = platform.prepareMediaWorker();
    if (!mediaWorker.isEmpty()) {
        if (!core.setMediaWorkerPath(mediaWorker))
            qWarning() << "could not configure the private Android media worker";
    }

    QQmlApplicationEngine engine;
    engine.rootContext()->setContextProperty(QStringLiteral("coreAdapter"), &core);
    engine.rootContext()->setContextProperty(QStringLiteral("androidBridge"), &platform);
    engine.rootContext()->setContextProperty(QStringLiteral("harborRender"), QVariantMap{
        {"effectsDisabled", disableEffects},
        {"advancedTransforms", !disableEffects},
        {"backend", renderBackend}
    });
    engine.loadFromModule("HarborMobile", "HarborMobileHost");
    if (engine.rootObjects().isEmpty())
        return 1;
    return app.exec();
}
