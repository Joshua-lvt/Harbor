import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Control {
    id: control

    property string category: "system"
    property string title: ""
    property string description: ""
    property string time: ""
    property bool unread: false
    // Real program identity for icons. `iconUrl` is a small PNG data URL
    // resolved by the native layer (`HarborCore.appIconUrl`); empty means
    // "unknown" and the category glyph below stays visible.
    property string appId: ""
    property string iconKey: ""
    property string iconUrl: ""
    property color categoryColor: Theme.categoryColor(category)
    signal clicked()

    // Control has no `down` of its own; the pointer area supplies it.
    readonly property bool pressed: pressArea.containsPress

    implicitWidth: 360
    implicitHeight: Math.max(68, contentItem.implicitHeight + topPadding + bottomPadding)
    padding: Theme.sp3
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus

    readonly property string unreadPrefix: unread
        ? I18n.t("a11y.unreadNotification", { title: title }) + " — " : ""

    Accessible.role: Accessible.Button
    Accessible.name: unreadPrefix + title
    Accessible.description: description + (time.length > 0 ? ", " + time : "")
    Accessible.onPressAction: control.clicked()

    Keys.onSpacePressed: event => { control.clicked(); event.accepted = true }
    Keys.onReturnPressed: event => { control.clicked(); event.accepted = true }

    contentItem: RowLayout {
        spacing: Theme.sp3

        Rectangle {
            implicitWidth: 40
            implicitHeight: 40
            radius: 20
            color: Qt.rgba(control.categoryColor.r, control.categoryColor.g, control.categoryColor.b, 0.16)
            Accessible.ignored: true

            HarborIcon {
                anchors.centerIn: parent
                name: Theme.categoryIconName(control.category)
                color: control.categoryColor
                implicitWidth: 20
                implicitHeight: 20
                // A real app icon replaces the generic glyph; both never
                // show at once so the layout never shifts.
                visible: control.iconUrl.length === 0
            }

            Image {
                anchors.centerIn: parent
                width: 24
                height: 24
                source: control.iconUrl
                visible: control.iconUrl.length > 0
                fillMode: Image.PreserveAspectFit
                smooth: true
                cache: true
                asynchronous: true
                Accessible.ignored: true
            }
        }

        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2
            Text {
                text: control.title
                color: Theme.text
                font.pixelSize: Theme.fontBody
                font.weight: control.unread ? Font.Bold : Font.DemiBold
                elide: Text.ElideRight
                Layout.fillWidth: true
            }
            Text {
                visible: control.description.length > 0
                text: control.description
                color: Theme.textDim
                font.pixelSize: Theme.fontSmall
                elide: Text.ElideRight
                Layout.fillWidth: true
            }
        }

        ColumnLayout {
            spacing: Theme.sp1
            Layout.alignment: Qt.AlignTop
            Text {
                text: control.time
                color: Theme.textFaint
                font.pixelSize: Theme.fontTiny
                Layout.alignment: Qt.AlignRight
            }
            Rectangle {
                visible: control.unread
                implicitWidth: 8
                implicitHeight: 8
                radius: 4
                color: Theme.accent
                Layout.alignment: Qt.AlignRight
            }
        }
    }

    background: Rectangle {
        radius: Theme.radiusSmall
        color: control.pressed ? Theme.surfaceHighlight : control.hovered ? Theme.surface : "transparent"
        border.width: control.visualFocus ? 2 : 0
        border.color: Theme.accent
    }

    MouseArea {
        id: pressArea

        anchors.fill: parent
        cursorShape: Qt.PointingHandCursor
        onClicked: {
            control.forceActiveFocus()
            control.clicked()
        }
    }
}
