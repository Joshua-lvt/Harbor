import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Control {
    id: control

    property alias text: field.text
    property alias placeholderText: field.placeholderText
    property alias echoMode: field.echoMode
    property alias readOnly: field.readOnly
    property alias validator: field.validator
    property alias inputMethodHints: field.inputMethodHints
    property alias acceptableInput: field.acceptableInput
    property string label: ""
    property string helperText: ""
    property string errorText: ""
    property string leadingIcon: ""
    property string trailingIcon: ""
    property bool clearable: false
    signal accepted()
    signal textEdited(string nextText)
    signal trailingClicked()

    implicitWidth: 280
    implicitHeight: layout.implicitHeight
    focusPolicy: Qt.NoFocus

    function forceInputFocus() {
        field.forceActiveFocus()
    }

    Accessible.name: label.length > 0 ? label : placeholderText
    Accessible.description: errorText.length > 0 ? errorText : helperText

    contentItem: ColumnLayout {
        id: layout
        spacing: Theme.sp1

        Text {
            visible: control.label.length > 0
            text: control.label
            color: Theme.text
            font.pixelSize: Theme.fontSmall
            font.weight: Font.DemiBold
            Layout.fillWidth: true
        }

        TextField {
            id: field
            Layout.fillWidth: true
            implicitHeight: 44
            leftPadding: control.leadingIcon.length > 0 ? 42 : Theme.sp3
            rightPadding: trailingButton.visible ? 42 : Theme.sp3
            color: Theme.text
            placeholderTextColor: Theme.textFaint
            selectionColor: Theme.accentDeep
            selectedTextColor: "white"
            font.pixelSize: Theme.fontBody
            focusPolicy: Qt.StrongFocus
            onAccepted: control.accepted()
            onTextEdited: control.textEdited(field.text)

            background: Rectangle {
                radius: Theme.radiusSmall
                color: Theme.surface
                border.width: field.activeFocus ? 2 : 1
                border.color: control.errorText.length > 0 ? Theme.danger
                              : field.activeFocus ? Theme.accent : Theme.surfaceBorder
            }

            HarborIcon {
                visible: control.leadingIcon.length > 0
                anchors.left: parent.left
                anchors.leftMargin: Theme.sp3
                anchors.verticalCenter: parent.verticalCenter
                name: control.leadingIcon
                color: Theme.textDim
                implicitWidth: 20
                implicitHeight: 20
            }

            HarborIconButton {
                id: trailingButton
                visible: control.trailingIcon.length > 0 || (control.clearable && field.text.length > 0)
                anchors.right: parent.right
                anchors.rightMargin: 4
                anchors.verticalCenter: parent.verticalCenter
                buttonSize: 36
                iconName: control.clearable && field.text.length > 0 ? "close" : ""
                iconText: control.clearable && field.text.length > 0 ? "" : control.trailingIcon
                toolTip: control.clearable && field.text.length > 0
                         ? I18n.t("common.actions.clear")
                         : I18n.t("component.input.action")
                onClicked: {
                    if (control.clearable && field.text.length > 0) field.clear()
                    else control.trailingClicked()
                }
            }
        }

        Text {
            visible: control.errorText.length > 0 || control.helperText.length > 0
            text: control.errorText.length > 0 ? control.errorText : control.helperText
            color: control.errorText.length > 0 ? Theme.danger : Theme.textDim
            font.pixelSize: Theme.fontTiny
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }
    }
}
