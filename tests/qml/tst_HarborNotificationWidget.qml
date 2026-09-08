import QtQuick
import QtTest
import Harbor 2.0

// Contract tests for the temporary notification window. The service
// (HarborNotificationsBridge) owns the queue in production; here the test
// plays that role, so these tests pin only what the widget itself promises:
//
//   - the window exists only while cards are queued,
//   - the stack mirrors the queue one delegate per card,
//   - clicking a card activates it first and dismisses it only after the
//     exit fade (so the click navigation never races the removal),
//   - every card dismisses itself after its lifetime,
//   - reduced motion collapses entrance and exit to the plain fade.
TestCase {
    id: root

    name: "HarborNotificationWidget"

    // The window under test. Main.qml mounts it against the shell's screen
    // and routes cardDismissed to the service queue; the test mirrors that
    // routing declaratively (dynamic .connect() here would leak across test
    // functions, since a TestCase never tears down its own resources).
    HarborNotificationWidget {
        id: widget

        property var activatedKinds: []
        property var dismissedIds: []

        onCardActivated: function (kind) { widget.activatedKinds.push(kind) }
        onCardDismissed: function (id) {
            widget.dismissedIds.push(id)
            var next = []
            for (var i = 0; i < widget.cards.length; ++i) {
                if (String(widget.cards[i].id) !== String(id))
                    next.push(widget.cards[i])
            }
            widget.cards = next
        }
    }

    function init() {
        AppState.resetSession()
        widget.cards = []
        widget.activatedKinds = []
        widget.dismissedIds = []
    }

    function cleanup() {
        widget.cards = []
        AppState.resetSession()
    }

    function _card(id, kind) {
        return {
            id: id,
            kind: kind || "message",
            title: "Taylor is away",
            body: "Stepped away from the keyboard.",
            avatar: ""
        }
    }

    function _collect(item, probe, out) {
        var children = item.children
        for (var i = 0; i < children.length; ++i) {
            var child = children[i]
            if (probe(child))
                out.push(child)
            _collect(child, probe, out)
        }
        return out
    }

    // Card delegates are the Rectangle subclasses carrying the two-phase
    // exit API; click areas carry an objectName for the same purpose.
    function _cards() {
        return _collect(widget.contentItem, function (item) {
            return item.leaving !== undefined && item.dismissed !== undefined
        }, [])
    }

    function _clickAreas() {
        return _collect(widget.contentItem, function (item) {
            return item.objectName === "cardClickArea"
        }, [])
    }

    // The area belonging to one specific card — the stack order is an
    // implementation detail the test must not lean on.
    function _clickAreaFor(cardId) {
        var areas = _clickAreas()
        for (var i = 0; i < areas.length; ++i) {
            if (String(areas[i].parent.modelData.id) === String(cardId))
                return areas[i]
        }
        return null
    }

    // ── Window lifecycle ─────────────────────────────────────────────

    // Independent top-level: cards must still pop while the shell is hidden
    // (a transient Tool would hide with its parent).
    function test_widgetIsIndependentTopLevel() {
        verify(widget.transientParent === null,
               "notification window must not be transient to the shell")
    }

    function test_windowLivesOnlyWhileCardsExist() {
        verify(!widget.visible)

        widget.cards = [_card("c1")]
        compare(widget.visible, true)

        // The last dismissal unmounts the stack: no invisible widget stays
        // alive between notifications.
        widget.cards = []
        verify(!widget.visible)
    }

    function test_stackMirrorsTheQueue() {
        widget.cards = [_card("c1"), _card("c2", "presence"), _card("c3", "call")]
        tryVerify(function () { return _cards().length === 3 })
        compare(_clickAreas().length, 3)
    }

    function test_pinnedToTheShellScreenTopRight() {
        widget.cards = [_card("c1")]
        compare(widget.width, 340)
        compare(widget.y, 16)
        // Relative to the pinned screen, never to the primary display.
        compare(widget.x, widget.Screen.width - widget.width - 16)
    }

    // ── Interaction ──────────────────────────────────────────────────

    function test_clickActivatesThenDismissesAfterTheFade() {
        widget.cards = [_card("c1", "message"), _card("c2", "presence")]
        tryVerify(function () { return _clickAreas().length === 2 })

        var area = _clickAreaFor("c2")
        verify(area !== null)
        mouseClick(area, area.width / 2, area.height / 2)

        // Activation is immediate; removal waits for the exit fade.
        compare(widget.activatedKinds.length, 1)
        compare(widget.activatedKinds[0], "presence")
        compare(widget.dismissedIds.length, 0)
        tryVerify(function () { return widget.dismissedIds.length === 1 }, 2000)
        compare(widget.dismissedIds[0], "c2")

        // The untouched card stays up.
        tryVerify(function () { return _cards().length === 1 }, 2000)
        compare(widget.visible, true)
    }

    function test_autoDismissAfterItsLifetime() {
        widget.cards = [_card("c1")]
        tryVerify(function () { return _cards().length === 1 })

        // Not an instant toast: the card survives at least a second.
        wait(1000)
        compare(widget.dismissedIds.length, 0)

        // Lifetime (5s) plus exit fade, with headroom.
        tryVerify(function () { return widget.dismissedIds.length === 1 }, 8000)
        verify(!widget.visible)
    }

    // ── Motion ───────────────────────────────────────────────────────

    // Each card paints its own ocean (same stops as the shell), so every
    // ocean variant visibly recolors it instead of staying blue.
    function test_cardFollowsOceanVariant() {
        widget.cards = [_card("c1")]
        tryVerify(function () { return _cards().length === 1 })
        var card = _cards()[0]
        verify(card.gradient !== null && card.gradient.stops.length === 3)
        AppState.oceanVariant = "lagoon"
        wait(20)
        compare(card.gradient.stops[0].color, Theme.bgTop)
        var lagoonTop = String(card.gradient.stops[0].color)
        AppState.oceanVariant = "ember"
        wait(20)
        compare(card.gradient.stops[0].color, Theme.bgTop)
        verify(String(card.gradient.stops[0].color) !== lagoonTop,
               "ember must recolor the card away from lagoon blue")
        AppState.oceanVariant = "lagoon"
    }

    function test_reducedMotionKeepsOnlyTheFade() {
        AppState.reducedMotion = true
        widget.cards = [_card("c1")]
        tryVerify(function () { return _cards().length === 1 })

        var card = _cards()[0]
        // No scale journey in either phase — the fade carries everything.
        compare(card.scale, 1)
        card.leaving = true
        compare(card.scale, 1)
        tryVerify(function () { return card.opacity === 0 }, 1500)
    }
}
