pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Control {
    id: root

    property alias model: combo.model
    property alias currentIndex: combo.currentIndex
    readonly property alias currentText: combo.currentText
    readonly property alias currentValue: combo.currentValue
    property alias textRole: combo.textRole
    property alias valueRole: combo.valueRole
    // Optional roles for descriptor models whose user-visible label is a catalog
    // key. Plain string and ordinary textRole models keep their current behavior.
    property string translationKeyRole: ""
    property string translationParamsRole: ""
    property string label: ""
    property string description: ""
    property string placeholderText: I18n.t("component.select.placeholder")
    property string accessibleName: label
    readonly property alias comboBox: combo

    signal activated(int index)

    function forceSelectFocus() {
        combo.forceActiveFocus()
    }

    function _modelItem(index) {
        if (!combo.model || index < 0 || index >= combo.count)
            return null
        return combo.model[index]
    }

    function optionText(index) {
        var item = _modelItem(index)
        if (translationKeyRole.length > 0 && item !== null
                && typeof item === "object") {
            var key = item[translationKeyRole]
            if (typeof key === "string" && key.length > 0) {
                var params = translationParamsRole.length > 0
                    ? item[translationParamsRole] || {} : {}
                return I18n.t(key, params)
            }
        }
        return combo.textAt(index)
    }

    implicitWidth: 280
    implicitHeight: selectLayout.implicitHeight
    padding: 0
    focusPolicy: Qt.NoFocus
    Accessible.role: Accessible.Pane
    Accessible.name: accessibleName
    Accessible.description: description

    background: Item {
    }

    contentItem: ColumnLayout {
        id: selectLayout

        spacing: Theme.sp1

        Text {
            visible: root.label.length > 0
            text: root.label
            color: root.enabled ? Theme.textPrimary : Theme.textDisabled
            font.family: Theme.fontFamily
            font.pixelSize: Theme.fontSmall
            font.weight: Font.DemiBold
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }

        ComboBox {
            id: combo

            Layout.fillWidth: true
            implicitHeight: Theme.hitTarget
            enabled: root.enabled
            hoverEnabled: true
            focusPolicy: Qt.StrongFocus
            leftPadding: Theme.sp3
            rightPadding: Theme.hitTarget
            displayText: currentIndex >= 0 ? root.optionText(currentIndex) : root.placeholderText
            Accessible.name: root.accessibleName.length > 0 ? root.accessibleName : root.placeholderText
            Accessible.description: root.description
            onActivated: (index) => {
                return root.activated(index);
            }

            contentItem: Text {
                leftPadding: 0
                rightPadding: 0
                text: combo.displayText
                color: !combo.enabled ? Theme.textDisabled : combo.currentIndex < 0 ? Theme.textMuted : Theme.textPrimary
                font.family: Theme.fontFamily
                font.pixelSize: Theme.fontBody
                verticalAlignment: Text.AlignVCenter
                elide: Text.ElideRight
            }

            indicator: HarborIcon {
                x: combo.width - combo.rightPadding + (combo.rightPadding - width) / 2
                y: (combo.height - height) / 2
                name: "chevron-down"
                color: combo.enabled ? Theme.iconSecondary : Theme.iconDisabled
                implicitWidth: 18
                implicitHeight: 18
                rotation: combo.popup.visible ? 180 : 0

                Behavior on rotation {
                    NumberAnimation {
                        duration: Theme.duration(Theme.motionFast)
                        easing.type: Theme.animEasing
                    }

                }

            }

            background: Rectangle {
                color: !combo.enabled ? Theme.surfaceSunken : combo.down ? Theme.surfacePressed : combo.hovered ? Theme.surfaceHover : Theme.surfaceInteractive
                radius: Theme.radiusSmall
                border.width: combo.visualFocus ? Theme.focusWidth : 1
                border.color: combo.visualFocus ? Theme.focusRing : Theme.borderSubtle
                opacity: combo.enabled ? 1 : Theme.opacityDisabled

                Behavior on color {
                    ColorAnimation {
                        duration: Theme.duration(Theme.motionFast)
                    }

                }

            }

            delegate: ItemDelegate {
                id: optionDelegate

                required property int index

                width: ListView.view ? ListView.view.width : combo.width
                implicitHeight: Math.max(Theme.hitTarget, optionText.implicitHeight + topPadding + bottomPadding)
                topPadding: Theme.sp2
                bottomPadding: Theme.sp2
                leftPadding: Theme.sp3
                rightPadding: Theme.sp3
                highlighted: combo.highlightedIndex === index
                focusPolicy: Qt.StrongFocus
                Accessible.name: root.optionText(optionDelegate.index)
                Accessible.selected: combo.currentIndex === index

                contentItem: Text {
                    id: optionText

                    text: root.optionText(optionDelegate.index)
                    color: optionDelegate.enabled ? Theme.textPrimary : Theme.textDisabled
                    font.family: Theme.fontFamily
                    font.pixelSize: Theme.fontBody
                    wrapMode: Text.WrapAnywhere
                    verticalAlignment: Text.AlignVCenter
                }

                background: Rectangle {
                    color: optionDelegate.highlighted ? Theme.surfaceHover : "transparent"
                    radius: Theme.radiusSmall
                }

            }

            popup: Popup {
                y: combo.height + Theme.sp1
                width: combo.width
                implicitHeight: Math.min(contentItem.implicitHeight + topPadding + bottomPadding, 320)
                padding: Theme.sp1

                contentItem: ListView {
                    clip: true
                    implicitHeight: contentHeight
                    model: combo.popup.visible ? combo.delegateModel : null
                    currentIndex: combo.highlightedIndex
                    highlightMoveDuration: Theme.duration(Theme.motionFast)

                    ScrollIndicator.vertical: ScrollIndicator {
                    }

                }

                background: Rectangle {
                    color: Theme.surfaceOverlay
                    radius: Theme.radiusSmall
                    border.width: 1
                    border.color: Theme.borderStrong
                }

            }

        }

        Text {
            visible: root.description.length > 0
            text: root.description
            color: root.enabled ? Theme.textSecondary : Theme.textDisabled
            font.family: Theme.fontFamily
            font.pixelSize: Theme.fontTiny
            lineHeight: Theme.lineHeightTiny
            lineHeightMode: Text.ProportionalHeight
            wrapMode: Text.Wrap
            Layout.fillWidth: true
        }

    }

}
