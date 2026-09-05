pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts

// Blocking surface for mandatory updates. It shows exactly while the
// updater reports a discovered update (available, downloading, ready,
// applying) and absorbs every click, so there is no path around it —
// updates are mandatory once discovered. A mere check failure never shows
// this: offline stays usable, the Settings card carries the error with a
// retry action, and the updater looks again later on its own.
//
// The updater is a C++ context property, so this file is deliberately
// dynamically typed; qmllint cannot know its members.
// qmllint disable missing-property
Item {
    id: root

    // The HarborUpdater context object (or a stub with the same surface).
    property QtObject updater: null

    readonly property bool required: updater !== null && updater.updateRequired === true
    readonly property string status: updater !== null ? String(updater.status || "idle") : "idle"

    // Visibility follows the requirement outright; opacity only fades the
    // entrance. Deriving visibility from the animated opacity leaves the
    // layer stuck on when the fade is interrupted.
    visible: root.required
    opacity: root.required ? 1 : 0
    Behavior on opacity {
        NumberAnimation { duration: Theme.duration(Theme.motionFast) }
    }

    Accessible.role: Accessible.AlertMessage
    Accessible.name: I18n.t("update.title")

    Rectangle {
        anchors.fill: parent
        color: Theme.surfaceScrim
        opacity: root.required ? 1 : 0

        // Absorbs every stray click: nothing behind a mandatory update reacts.
        MouseArea { anchors.fill: parent }
    }

    ColumnLayout {
        anchors.centerIn: parent
        width: Math.max(0, Math.min(460, parent.width - Theme.sp8))
        spacing: Theme.sp4
        visible: root.required

        Rectangle {
            Layout.fillWidth: true
            Layout.preferredHeight: content.implicitHeight + Theme.sp5 * 2
            radius: Theme.radiusLarge
            color: Theme.surfaceOverlay
            border.width: 1
            border.color: Theme.borderStrong

            Accessible.role: Accessible.Dialog
            Accessible.name: I18n.t("update.title")

            ColumnLayout {
                id: content

                anchors.fill: parent
                anchors.margins: Theme.sp5
                spacing: Theme.sp3

                RowLayout {
                    Layout.fillWidth: true
                    spacing: Theme.sp3

                    HarborIcon {
                        name: "refresh"
                        color: Theme.accent
                        implicitWidth: 30
                        implicitHeight: 30
                        Layout.alignment: Qt.AlignVCenter
                        Accessible.ignored: true
                    }

                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: Theme.sp1

                        Text {
                            text: I18n.t("update.title")
                            color: Theme.textPrimary
                            font.family: Theme.fontFamilyDisplay
                            font.pixelSize: Theme.fontTitle
                            font.weight: Font.Bold
                            wrapMode: Text.Wrap
                            Layout.fillWidth: true
                        }

                        Text {
                            text: root.updater !== null && String(root.updater.availableVersion || "").length > 0
                                ? I18n.t("update.available", { version: root.updater.availableVersion })
                                : I18n.t("update.checking")
                            color: Theme.textSecondary
                            font.family: Theme.fontFamily
                            font.pixelSize: Theme.fontBody
                            wrapMode: Text.Wrap
                            Layout.fillWidth: true
                        }
                    }
                }

                // Determinate progress while fetching the package.
                ColumnLayout {
                    visible: root.status === "downloading"
                    Layout.fillWidth: true
                    spacing: Theme.sp2

                    Rectangle {
                        Layout.fillWidth: true
                        Layout.preferredHeight: 8
                        radius: 4
                        color: Theme.surfaceSunken
                        Accessible.ignored: true

                        Rectangle {
                            anchors.top: parent.top
                            anchors.bottom: parent.bottom
                            anchors.left: parent.left
                            width: parent.width * (root.updater !== null ? Number(root.updater.progress || 0) : 0)
                            radius: 4
                            color: Theme.accent
                        }
                    }

                    Text {
                        text: I18n.t("update.downloading", {
                            percent: Math.round((root.updater !== null ? Number(root.updater.progress || 0) : 0) * 100)
                        })
                        color: Theme.textSecondary
                        font.family: Theme.fontFamily
                        font.pixelSize: Theme.fontSmall
                        Layout.fillWidth: true
                    }
                }

                Text {
                    visible: root.status === "ready" || root.status === "applying"
                    text: root.updater !== null && root.updater.waitingForCall === true
                        ? I18n.t("update.waitingForCall")
                        : I18n.t("update.applying")
                    color: Theme.textSecondary
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontBody
                    wrapMode: Text.Wrap
                    Layout.fillWidth: true
                }

                Text {
                    text: I18n.t("update.mandatoryNote")
                    color: Theme.textFaint
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontSmall
                    wrapMode: Text.Wrap
                    horizontalAlignment: Text.AlignHCenter
                    Layout.fillWidth: true
                }
            }
        }
    }
}
