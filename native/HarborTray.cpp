#include "HarborTray.h"

#include <QApplication>
#include <QMenu>
#include <QSystemTrayIcon>

namespace {
QIcon trayIcon()
{
    // Named icon from the installed hicolor theme; the CMake install rules
    // ship the same name, so packaged and development builds agree.
    // Windows has no theme lookup: fall back to the PNG bundled in the
    // Harbor QML module resources.
    QIcon themed = QIcon::fromTheme(QStringLiteral("harbor"));
    if (!themed.isNull())
        return themed;
    return QIcon(QStringLiteral(":/qt/qml/Harbor/images/harbor.png"));
}
} // namespace

HarborTray::HarborTray(QObject *parent)
    : QObject(parent)
{
    m_available = QSystemTrayIcon::isSystemTrayAvailable();
    if (!m_available)
        return;

    m_menu = new QMenu();
    QAction *openAction = m_menu->addAction(QApplication::translate("HarborTray", "Open Harbor"));
    QAction *quitAction = m_menu->addAction(QApplication::translate("HarborTray", "Quit Harbor"));
    connect(openAction, &QAction::triggered, this, &HarborTray::openRequested);
    connect(quitAction, &QAction::triggered, this, &HarborTray::quitRequested);

    m_icon = new QSystemTrayIcon(trayIcon(), this);
    m_icon->setToolTip(QStringLiteral("Harbor"));
    m_icon->setContextMenu(m_menu);
    connect(m_icon, &QSystemTrayIcon::activated, this, [this](QSystemTrayIcon::ActivationReason reason) {
        if (reason == QSystemTrayIcon::Trigger || reason == QSystemTrayIcon::DoubleClick)
            emit openRequested();
    });
}

HarborTray::~HarborTray()
{
    delete m_menu;
}

bool HarborTray::available() const
{
    return m_available;
}

bool HarborTray::active() const
{
    return m_active;
}

void HarborTray::setActive(bool active)
{
    const bool rendered = active && m_available;
    if (m_icon)
        m_icon->setVisible(rendered);
    if (m_active == rendered)
        return;
    m_active = rendered;
    emit activeChanged();
}

void HarborTray::showNotification(const QString &title, const QString &body)
{
    // Balloons need a visible icon; without one the in-app widget the QML
    // bridge always shows is the notification surface.
    if (!m_icon || !m_active)
        return;
    m_icon->showMessage(title, body, QSystemTrayIcon::Information, 5000);
}
