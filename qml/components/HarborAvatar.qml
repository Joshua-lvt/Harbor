import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Item {
    id: root

    property string initials: "?"
    property url source: ""
    // "image" | "gif": a gif source plays animated everywhere this avatar
    // is used. Anything else renders as a still image.
    property string avatarType: "image"
    property string status: "offline" // online, idle, offline
    property int avatarSize: 56
    property bool showStatus: true
    property bool speaking: false
    property bool muted: false
    property color fillColor: Theme.accentDeep
    property color textColor: "white"
    property string accessibleName: initials
    // Only sanitized embedded image data may reach the image provider. A
    // settings or QML mistake must never turn an avatar into a file URL.
    readonly property url safeSource: {
        var value = String(root.source || "")
        return /^(data:image\/(png|jpeg|jpg|webp|gif);base64,[A-Za-z0-9+/]+={0,2})$/.test(value)
            ? value : ""
    }

    implicitWidth: avatarSize
    implicitHeight: avatarSize

    Accessible.role: Accessible.Graphic
    Accessible.name: accessibleName
    Accessible.description: showStatus ? I18n.t("a11y.status", { status: status }) : ""

    readonly property bool isAnimated: root.avatarType === "gif"
        && String(root.source).indexOf("data:image/gif") === 0
    // Initials are the fallback only: they show while no avatar is set or
    // while the image is still loading — never over a configured avatar.
    readonly property bool imageReady: root.isAnimated ? gifImage.status === AnimatedImage.Ready
                                                      : avatarImage.status === Image.Ready

    Rectangle {
        anchors.fill: parent
        anchors.margins: -Math.max(2, Math.round(root.avatarSize * 0.05))
        radius: width / 2
        color: "transparent"
        border.width: root.speaking ? Math.max(2, Math.round(root.avatarSize * 0.045)) : 0
        border.color: Theme.success
        opacity: root.speaking ? 0.85 : 0
        scale: root.speaking ? 1.04 : 1
        Behavior on opacity { NumberAnimation { duration: Theme.duration(Theme.motionFast) } }
        Behavior on scale { NumberAnimation { duration: Theme.duration(Theme.motionFast) } }
        Accessible.ignored: true
    }

    Rectangle {
        anchors.fill: parent
        radius: width / 2
        color: root.fillColor
        border.width: 1
        border.color: Theme.surfaceHighlight
        clip: true

        Text {
            anchors.centerIn: parent
            visible: !root.imageReady
            text: root.initials.slice(0, 2).toUpperCase()
            color: root.textColor
            font.pixelSize: Math.round(root.avatarSize * 0.34)
            font.weight: Font.DemiBold
        }

        Image {
            id: avatarImage
            anchors.fill: parent
            anchors.margins: 2
            source: !root.isAnimated ? root.safeSource : ""
            fillMode: Image.PreserveAspectCrop
            asynchronous: true
            visible: !root.isAnimated && status === Image.Ready
        }

        AnimatedImage {
            id: gifImage
            anchors.fill: parent
            anchors.margins: 2
            source: root.isAnimated ? root.safeSource : ""
            fillMode: Image.PreserveAspectCrop
            asynchronous: true
            // Reduced motion freezes the gif on its first frame; the avatar
            // itself stays. Functional states are unaffected.
            playing: root.isAnimated && !AppState.reducedMotion
            visible: root.isAnimated && status === AnimatedImage.Ready
        }
    }

    Rectangle {
        visible: root.showStatus
        width: Math.max(12, Math.round(root.avatarSize * 0.25))
        height: width
        radius: width / 2
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        color: root.status === "online" ? Theme.online : root.status === "idle" ? Theme.idle : Theme.offline
        border.width: Math.max(2, Math.round(root.avatarSize * 0.05))
        border.color: Theme.bgMid
    }
}
