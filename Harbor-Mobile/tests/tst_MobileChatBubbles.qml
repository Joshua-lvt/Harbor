// Chat bubbles size themselves from their text: the bubble rect must
// have a real height and the text must sit inside it with padding.
import QtQuick
import QtQuick.Controls
import QtTest

TestCase {
    name: "MobileChatBubbles"

    ApplicationWindow {
        id: win
        visible: true
        width: 412
        height: 900
    }

    function createChat(messages) {
        var component = Qt.createComponent("../qml/views/MobileChatView.qml")
        compare(component.status, Component.Ready, component.errorString())
        var view = component.createObject(win.contentItem, {
            "width": 412, "height": 700,
            "messages": messages, "connected": true
        })
        verify(view !== null)
        return view
    }

    function collect(item, predicate, out) {
        out = out || []
        if (predicate(item))
            out.push(item)
        for (var i = 0; i < item.children.length; i++)
            collect(item.children[i], predicate, out)
        return out
    }

    function test_bubbles_have_height_and_padding() {
        var view = createChat([
            {"id": "m1", "body": "hello from the desktop side", "outgoing": false},
            {"id": "m2", "body": "a much longer reply that must wrap onto several lines inside the bubble instead of overflowing it", "outgoing": true}
        ])
        tryVerify(function () {
            var bubbles = collect(view, function (c) {
                return c.radius === 14 && c.color !== undefined
            })
            if (bubbles.length < 2)
                return false
            for (var i = 0; i < bubbles.length; i++) {
                if (bubbles[i].height <= 20)
                    return false
                var texts = collect(bubbles[i], function (c) {
                    return c.text !== undefined && String(c.text).length > 0
                })
                if (texts.length === 0)
                    return false
                // Text sits inside the bubble with padding on every side.
                if (texts[0].x < 1 || texts[0].y < 1)
                    return false
                if (texts[0].x + texts[0].width > bubbles[i].width - 1)
                    return false
                if (texts[0].y + texts[0].height > bubbles[i].height - 1)
                    return false
            }
            return true
        })
        view.destroy()
    }
}
