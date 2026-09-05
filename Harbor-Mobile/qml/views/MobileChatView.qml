// Mobile Chat: the SAME conversation as the desktop. Messages arrive by
// ID; duplicates never render twice (the core dedups, the view just
// renders what it is given). Offline shows a banner, never a fake send.
pragma ComponentBehavior: Bound

import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "../components"

ColumnLayout {
    id: view
    spacing: 0

    // {id, body, outgoing: bool, delivery: string, timestamp: real}
    property var messages: []
    property bool connected: false
    property bool blocked: false
    // Optional shared theme; without one the view keeps its shipped colors.
    property var theme

    signal sendChat(string body)

    Label {
        text: view.blocked
            ? qsTr("Harbor Mobile currently requires a desktop peer.")
            : (view.connected ? "" : qsTr("Offline — messages will send when the session is back."))
        visible: view.blocked || !view.connected
        color: view.theme ? view.theme.textSecondary : "#9db8c4"
        wrapMode: Text.WordWrap
        Layout.fillWidth: true
        Layout.margins: 12
    }

    ListView {
        id: list
        Layout.fillWidth: true
        Layout.fillHeight: true
        model: view.messages
        spacing: 8
        clip: true
        onCountChanged: positionViewAtEnd()

        delegate: RowLayout {
            id: messageRow
            required property var modelData
            width: list.width
            layoutDirection: modelData.outgoing ? Qt.RightToLeft : Qt.LeftToRight

            Rectangle {
                // Sized by the text it holds: without these the bubble
                // collapses and the row renders with no height.
                Layout.preferredWidth: bubbleText.width + 24
                Layout.preferredHeight: bubbleText.height + 20
                Layout.margins: 8
                radius: view.theme ? view.theme.radius : 14
                color: messageRow.modelData.outgoing
                    ? (view.theme ? view.theme.bubbleOut : "#1d5d7a")
                    : (view.theme ? view.theme.bubbleIn : "#16303f")

                Label {
                    id: bubbleText
                    x: 12
                    y: 10
                    width: Math.min(implicitWidth, list.width * 0.75 - 24)
                    text: messageRow.modelData.body
                    color: messageRow.modelData.outgoing
                        ? (view.theme ? view.theme.accentText : "#e6f2f7")
                        : (view.theme ? view.theme.textPrimary : "#e6f2f7")
                    wrapMode: Text.WordWrap
                }
            }
        }
    }

    RowLayout {
        Layout.fillWidth: true
        Layout.margins: 8
        spacing: 8

        TextField {
            id: composer
            Layout.fillWidth: true
            Layout.preferredHeight: 48
            placeholderText: qsTr("Message")
            enabled: !view.blocked
            onAccepted: {
                if (text.trim().length > 0) {
                    view.sendChat(text)
                    text = ""
                }
            }
        }

        MobileButton {
            text: qsTr("Send")
            theme: view.theme
            Layout.preferredHeight: 48
            enabled: !view.blocked && composer.text.trim().length > 0
            onClicked: {
                view.sendChat(composer.text)
                composer.text = ""
            }
        }
    }
}
