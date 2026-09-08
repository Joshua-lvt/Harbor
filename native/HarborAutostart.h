#pragma once

#include <QObject>

/// Start-with-system support through the platform's real mechanism.
///
/// Linux (X11 and Wayland alike): an XDG autostart entry at
/// `$XDG_CONFIG_HOME/autostart/harbor.desktop`, written and removed
/// atomically. Windows: a `Harbor.lnk` shortcut in the per-user Startup
/// folder (no registry writes, no admin rights). Other platforms:
/// `supported` is false and enabling is an honest no-op — the setting may
/// be stored, but the adapter never claims a startup registration it did
/// not make.
class HarborAutostart final : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool supported READ supported CONSTANT FINAL)
    Q_PROPERTY(bool enabled READ enabled NOTIFY enabledChanged FINAL)

public:
    explicit HarborAutostart(QObject *parent = nullptr);

    static bool supported();
    bool enabled() const;

    /// Installs or removes the autostart entry. Returns what the platform
    /// now reports; on unsupported platforms the answer stays false.
    Q_INVOKABLE bool setEnabled(bool enabled);

signals:
    void enabledChanged();

private:
    bool entryExists() const;
};
