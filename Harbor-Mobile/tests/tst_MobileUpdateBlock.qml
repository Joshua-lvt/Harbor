// The mandatory-update block only ever appears for a discovered update:
// idle and error stay out of the way, downloading shows progress, and the
// ready state offers exactly the platform install.
import QtQuick
import QtQuick.Controls
import QtTest

TestCase {
    name: "MobileUpdateBlock"

    ApplicationWindow {
        id: win
        visible: true
        width: 412
        height: 900
    }

    function createBlock() {
        var component = Qt.createComponent("../qml/components/MobileUpdateBlock.qml")
        compare(component.status, Component.Ready, component.errorString())
        var block = component.createObject(win.contentItem, {
            "width": 412, "height": 900
        })
        verify(block !== null)
        return block
    }

    function collectTexts(item, out) {
        out = out || []
        if (item.text !== undefined && String(item.text).length > 0)
            out.push(String(item.text))
        for (var i = 0; i < item.children.length; i++)
            collectTexts(item.children[i], out)
        return out
    }

    function hasText(block, needle) {
        var texts = collectTexts(block)
        for (var i = 0; i < texts.length; i++) {
            if (texts[i].indexOf(needle) >= 0)
                return true
        }
        return false
    }

    function test_idleAndErrorStayOutOfTheWay() {
        var block = createBlock()
        verify(!block.blocked)
        verify(!block.visible)
        block.updateStatus = "error"
        block.updateError = "update.error.network"
        verify(!block.blocked)
        verify(!block.visible)
        block.destroy()
    }

    function test_discoveredUpdateBlocks() {
        var block = createBlock()
        block.updateStatus = "available"
        block.updateVersion = "2.2.0"
        verify(block.blocked)
        verify(block.visible)
        verify(hasText(block, "2.2.0"))
        verify(hasText(block, "cannot be skipped"))
        block.destroy()
    }

    function test_downloadingShowsProgress() {
        var block = createBlock()
        block.updateStatus = "downloading"
        block.updateProgress = 0.5
        verify(block.visible)
        verify(hasText(block, "50%"))
        block.destroy()
    }

    function test_readyOffersInstall() {
        var block = createBlock()
        var fired = 0
        block.installUpdate.connect(function () { fired++ })
        block.updateStatus = "ready"
        verify(block.visible)
        var install = null
        function findButton(item) {
            if (item.text !== undefined && String(item.text) === "Install now") {
                if (typeof item.clicked === "function") {
                    install = item
                    return
                }
                var owner = item.parent
                while (owner !== null && typeof owner.clicked !== "function")
                    owner = owner.parent
                install = owner
                return
            }
            for (var i = 0; i < item.children.length && install === null; i++)
                findButton(item.children[i])
        }
        findButton(block)
        verify(install !== null, "install button exists")
        mouseClick(install, install.width / 2, install.height / 2)
        compare(fired, 1)
        block.destroy()
    }
}
