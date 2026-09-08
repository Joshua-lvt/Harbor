// Qt Quick Test runner with a keyboard injection helper.
//
// Qt 6.11 dropped TestCase's item-targeted key functions, and the remaining
// keyPress/keyClick variants only reach the TestCase's own window. The shell
// and its views live in separate windows, so tests drive real key delivery
// through KeyTest, which posts the event into the target's window exactly the
// way QTest::keyEvent does.
#include <QtQuickTest/quicktest.h>

#include <QCoreApplication>
#include <QEvent>
#include <QKeyEvent>
#include <QQmlContext>
#include <QQmlEngine>
#include <QQuickItem>
#include <QQuickWindow>

namespace {

class KeyInjector : public QObject {
    Q_OBJECT

public:
    explicit KeyInjector(QObject *parent = nullptr) : QObject(parent) {}

    // target: a QQuickItem (its window receives the event) or a QWindow.
    Q_INVOKABLE void press(const QVariant &target, int key, int modifiers = 0)
    {
        keyEvent(QEvent::KeyPress, target, key, modifiers);
    }

    Q_INVOKABLE void release(const QVariant &target, int key, int modifiers = 0)
    {
        keyEvent(QEvent::KeyRelease, target, key, modifiers);
    }

    Q_INVOKABLE void click(const QVariant &target, int key, int modifiers = 0)
    {
        keyEvent(QEvent::KeyPress, target, key, modifiers);
        keyEvent(QEvent::KeyRelease, target, key, modifiers);
    }

private:
    static QWindow *windowFor(const QVariant &target)
    {
        QObject *object = target.value<QObject *>();
        if (auto *item = qobject_cast<QQuickItem *>(object))
            return item->window();
        return qobject_cast<QWindow *>(object);
    }

    static void keyEvent(QEvent::Type type, const QVariant &target, int key, int modifiers)
    {
        QWindow *window = windowFor(target);
        if (!window)
            return;
        QKeyEvent event(type, key, static_cast<Qt::KeyboardModifiers>(modifiers));
        QCoreApplication::sendEvent(window, &event);
    }
};

class TestSetup : public QObject {
    Q_OBJECT

public slots:
    void qmlEngineAvailable(QQmlEngine *engine)
    {
        static KeyInjector injector;
        engine->rootContext()->setContextProperty("KeyTest", &injector);
    }
};

} // namespace

QUICK_TEST_MAIN_WITH_SETUP(harbor, TestSetup)

#include "quicktest_main.moc"
