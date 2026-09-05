#pragma once

#include <QObject>
#include <QString>

class QSystemTrayIcon;
class QMenu;

/// Real system-tray presence for Harbor, kept strictly separate from the
/// in-app tray *preview* flyout (HarborTrayPreview.qml). The preview is a
/// visual mock; this adapter is the actual platform tray icon.
///
/// Availability is honest: when the desktop session exposes no tray
/// (common under pure-Wayland sessions without StatusNotifier), `available`
/// stays false and nothing pretends an icon exists — the QML close policy
/// then treats "close to tray" as a real quit instead of an invisible app.
class HarborTray final : public QObject
{
    Q_OBJECT
    Q_PROPERTY(bool available READ available NOTIFY availabilityChanged FINAL)
    Q_PROPERTY(bool active READ active NOTIFY activeChanged FINAL)

public:
    explicit HarborTray(QObject *parent = nullptr);
    ~HarborTray() override;

    bool available() const;
    bool active() const;

    /// Shows or removes the tray icon. A no-op when the platform exposes
    /// no tray; the state still mirrors the request so callers observe one
    /// truth, and `active` reports what the platform actually renders.
    Q_INVOKABLE void setActive(bool active);

    /// Shows one balloon notification on the tray icon. A no-op while the
    /// icon is not visible; desktop-notification routing (D-Bus on Linux,
    /// this balloon on Windows) is decided by the caller, so nothing here
    /// guesses about policy.
    Q_INVOKABLE void showNotification(const QString &title, const QString &body);

signals:
    void availabilityChanged();
    void activeChanged();
    /// The user asked to restore the main window from the tray.
    void openRequested();
    /// The user chose the explicit quit action in the tray menu.
    void quitRequested();

private:
    QSystemTrayIcon *m_icon = nullptr;
    QMenu *m_menu = nullptr;
    bool m_available = false;
    bool m_active = false;
};
