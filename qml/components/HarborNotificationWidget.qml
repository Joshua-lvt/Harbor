pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import Harbor 2.0

// The temporary notification widget: a frameless, always-on-top glass stack
// pinned to the top-right of the shell's screen. It exists only while cards
// are queued — the window hides (and its delegates unmount) when the last
// card is dismissed, so no invisible widget ever stays alive.
//
// Each card lives 4–6 seconds, then fades on its own; clicking a card opens
// Harbor on the chat (messages) or simply dismisses it. Reduced motion
// collapses entrances and exits to the plain fade, like every other
// surface. The queue is owned by the notification service
// (HarborNotificationsBridge); this window is purely a view.
Window {
    id: widget

    // Cards from the service: {id, kind, title, body, avatar}.
    property var cards: []
    readonly property bool reducedMotion: AppState.reducedMotion
    readonly property int cardWidth: 340
    readonly property int screenMargin: 16
    readonly property int cardSpacing: 10
    readonly property int autoDismissMs: 5000

    // Emitted after a card's exit fade finished; the service removes it.
    signal cardDismissed(string id)
    // Emitted on click before the dismissal fade; the shell navigates.
    signal cardActivated(string kind)

    flags: Qt.Tool | Qt.FramelessWindowHint | Qt.WindowStaysOnTopHint
    // Independent top-level: cards must still pop while the shell is hidden
    // to the tray/widget (a transient Tool would hide with its parent).
    transientParent: null
    color: "transparent"
    visible: cards.length > 0
    width: cardWidth
    height: stack.implicitHeight

    // Top-right of whichever screen the shell lives on (multi-monitor safe:
    // Main.qml pins `screen` to the shell's screen; these coordinates are
    // relative to it, never to the primary display).
    x: Screen.width - width - screenMargin
    y: screenMargin

    Column {
        id: stack

        width: parent.width
        spacing: widget.cardSpacing

        Repeater {
            model: widget.cards

            delegate: NotificationCard {
                required property var modelData

                width: stack.width
                onDismissed: widget.cardDismissed(modelData.id)
                onActivated: {
                    widget.cardActivated(modelData.kind)
                    leaving = true
                }
            }
        }
    }

    component NotificationCard: Rectangle {
        id: card

        objectName: "notificationCard"
        property bool shown: false
        // Two-phase exit: `leaving` fades the card first; the removal from
        // the queue happens only after the fade, so nothing pops.
        property bool leaving: false

        signal dismissed()
        signal activated()

        radius: Theme.radiusLarge
        // Standalone window: paints its own ocean like the shell surface.
        // A fixed glass color would stay blue for every ocean variant.
        gradient: Gradient {
            GradientStop { position: 0.0; color: Theme.bgTop }
            GradientStop { position: 0.48; color: Theme.bgMid }
            GradientStop { position: 1.0; color: Theme.bgBottom }
        }
        border.width: 1
        border.color: Theme.surfaceBorder
        implicitHeight: contentRow.implicitHeight + 2 * Theme.sp3

        // Entrance: fade plus a gentle scale toward the top-right corner;
        // reduced motion keeps only the fade.
        opacity: shown && !leaving ? 1 : 0
        scale: widget.reducedMotion ? 1 : (shown && !leaving ? 1 : 0.96)
        transformOrigin: Item.TopRight
        Behavior on opacity {
            NumberAnimation { duration: Theme.duration(Theme.motionNormal) }
        }
        Behavior on scale {
            enabled: !widget.reducedMotion
            NumberAnimation { duration: Theme.duration(Theme.motionNormal) }
        }
        Component.onCompleted: shown = true

        Timer {
            interval: widget.autoDismissMs
            running: true
            repeat: false
            onTriggered: card.leaving = true
        }

        onLeavingChanged: {
            if (leaving)
                exitTimer.restart()
        }

        Timer {
            id: exitTimer

            interval: Theme.duration(Theme.motionNormal) + 50
            onTriggered: card.dismissed()
        }

        MouseArea {
            objectName: "cardClickArea"
            anchors.fill: parent
            cursorShape: Qt.PointingHandCursor
            onClicked: card.activated()
            Accessible.role: Accessible.Button
            Accessible.name: cardTitle.text
            Accessible.onPressAction: card.activated()
        }

        Accessible.role: Accessible.Notification
        Accessible.name: cardTitle.text + ". " + cardBody.text

        Row {
            id: contentRow

            anchors.fill: parent
            anchors.margins: Theme.sp3
            spacing: Theme.sp3

            HarborAvatar {
                anchors.verticalCenter: parent.verticalCenter
                avatarSize: 40
                source: String(card.modelData.avatar || "")
                avatarType: "image"
                showStatus: false
                accessibleName: String(card.modelData.title || "")
            }

            Column {
                width: parent.width - parent.spacing - 40 - 30
                spacing: 2

                Text {
                    id: cardTitle

                    width: parent.width
                    text: String(card.modelData.title || "")
                    color: Theme.textPrimary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontBody
                    font.weight: Font.DemiBold
                    elide: Text.ElideRight
                }

                Text {
                    id: cardBody

                    width: parent.width
                    text: String(card.modelData.body || "")
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontSmall
                    lineHeight: Theme.lineHeightSmall
                    lineHeightMode: Text.ProportionalHeight
                    elide: Text.ElideRight
                    maximumLineCount: 2
                }
            }

            HarborIconButton {
                anchors.verticalCenter: parent.verticalCenter
                buttonSize: 30
                iconName: "close"
                iconColor: hovered ? Theme.textPrimary : Theme.iconSecondary
                accessibleName: I18n.t("common.actions.dismiss")
                onClicked: card.leaving = true
            }
        }
    }
}
