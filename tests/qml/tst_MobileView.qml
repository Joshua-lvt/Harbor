import QtQuick
import QtTest
import Harbor 2.0

// Partner's-phone tab: battery, activity, schematic map and mirrored
// notifications render from the transient AppState slots, and every
// no-share combination degrades to its honest empty copy — never a fake.
TestCase {
    id: testCase
    name: "MobileView"
    width: 1280
    height: 960
    visible: true

    function init() {
        MockController.resetSession()
    }

    function cleanup() {
        MockController.resetSession()
    }

    function _createView() {
        var component = Qt.createComponent("qrc:/qt/qml/Harbor/MobileView.qml")
        verify(component.status === Component.Ready,
               "MobileView should be ready: " + component.errorString())
        var view = component.createObject(testCase, { width: 1200, height: 900 })
        verify(view !== null)
        return view
    }

    function _texts(item, out) {
        out = out || []
        if (item && item.text !== undefined && String(item.text).length > 0)
            out.push(String(item.text))
        var children = item ? item.children : []
        for (var i = 0; i < children.length; ++i)
            _texts(children[i], out)
        return out
    }

    function _hasText(view, needle) {
        var texts = _texts(view)
        for (var i = 0; i < texts.length; ++i) {
            if (texts[i].indexOf(needle) >= 0)
                return true
        }
        return false
    }

    function _findButton(view, objectName) {
        return _findFirst(view, objectName)
    }

    function _findFirst(item, name) {
        if (!item)
            return null
        if (item.objectName === name)
            return item
        if (!item.children)
            return null
        for (var i = 0; i < item.children.length; ++i) {
            var found = _findFirst(item.children[i], name)
            if (found !== null)
                return found
        }
        return null
    }

    function _pair() {
        AppState.setPairedPeers([{ deviceId: "d-1", harborId: "HBR-1" }])
    }

    function _phone(overrides) {
        var base = {
            batteryPercent: 73, charging: false, phoneActivity: "ACTIVE",
            lastActiveAt: Date.now() / 1000 - 90, currentApp: "Minecraft",
            locationSharingEnabled: false, location: null,
            notificationSharingEnabled: false, deviceType: "mobile"
        }
        overrides = overrides || {}
        for (var key in overrides)
            base[key] = overrides[key]
        AppState.setPeerPhone(base)
    }

    // Without a pair there is no phone anywhere on screen.
    function test_unpairedShowsPairEmpty() {
        AppState.setPairedPeers([])
        AppState.setPeerPhone(null)
        var view = _createView()
        try {
            verify(_hasText(view, I18n.t("mobile.empty.title")))
            verify(!_hasText(view, "73%"))
        } finally {
            view.destroy()
        }
    }

    // Paired but silent: the connect-phone card leads, and every other
    // card says what is missing, nothing invented.
    function test_pairedWithoutSharingShowsHonestEmpty() {
        _pair()
        AppState.setPeerPhone(null)
        var view = _createView()
        try {
            verify(_hasText(view, I18n.t("mobile.connect.title")))
            verify(_hasText(view, I18n.t("mobile.location.off")))
            verify(_hasText(view, I18n.t("mobile.notifications.off")))
        } finally {
            view.destroy()
        }
    }

    function test_batteryCard() {
        _pair()
        _phone({ batteryPercent: 73, charging: true })
        var view = _createView()
        try {
            verify(_hasText(view, "73%"))
            verify(_hasText(view, I18n.t("mobile.battery.charging")))
            _phone({ batteryPercent: null })
            wait(30)
            verify(_hasText(view, I18n.t("mobile.battery.unavailable")))
        } finally {
            view.destroy()
        }
    }

    function test_activityCard() {
        _pair()
        _phone({})
        var view = _createView()
        try {
            verify(_hasText(view, I18n.t("mobile.activity.active")))
            verify(_hasText(view, "Minecraft"))
            _phone({ phoneActivity: "OFFLINE", currentApp: "", lastActiveAt: 0 })
            wait(30)
            verify(_hasText(view, I18n.t("mobile.activity.unknown")))
        } finally {
            view.destroy()
        }
    }

    function test_locationStates() {
        _pair()
        _phone({ locationSharingEnabled: true, location: null })
        var view = _createView()
        try {
            verify(_hasText(view, I18n.t("mobile.location.waiting")))
            _phone({
                locationSharingEnabled: true,
                location: { latitude: -23.55052, longitude: -46.63331,
                            accuracyMeters: 25, updatedAt: Date.now() / 1000 - 60 }
            })
            wait(30)
            verify(_hasText(view, "-23.55052, -46.63331"))
        } finally {
            view.destroy()
        }
    }

    function test_copyCoordinatesUsesMockClipboard() {
        _pair()
        _phone({
            locationSharingEnabled: true,
            location: { latitude: -23.55052, longitude: -46.63331,
                        accuracyMeters: 25, updatedAt: Date.now() / 1000 - 60 }
        })
        var view = _createView()
        try {
            var button = _findButton(view, "copyCoordinatesButton")
            verify(button !== null)
            mouseClick(button, button.width / 2, button.height / 2)
            compare(MockController.mockCopyTarget, "coordinates")
            verify(_hasText(view, I18n.t("mobile.location.copied")))
        } finally {
            view.destroy()
        }
    }

    function test_notificationsListAndNote() {
        _pair()
        _phone({ notificationSharingEnabled: true })
        AppState.pushPhoneNotice({ appLabel: "Chat", title: "Taylor",
                                   text: "hey", at: Date.now() / 1000 })
        var view = _createView()
        try {
            verify(_hasText(view, "Chat"))
            verify(_hasText(view, "hey"))
            verify(_hasText(view, I18n.t("mobile.notifications.note")))
        } finally {
            view.destroy()
        }
    }

    // The schematic map only ever shows a shared fix; otherwise it labels
    // itself honestly for assistive tech.
    function test_phoneMapHonesty() {
        var component = Qt.createComponent("qrc:/qt/qml/Harbor/HarborPhoneMap.qml")
        verify(component.status === Component.Ready,
               "HarborPhoneMap should be ready: " + component.errorString())
        var map = component.createObject(testCase, { width: 480, height: 300 })
        try {
            verify(!map.hasFix)
            map.latitude = -23.55052
            map.longitude = -46.63331
            map.accuracyMeters = 25
            verify(map.hasFix)
            verify(String(map.Accessible.description).indexOf("-23.55052") >= 0)
            map.accuracyMeters = -1
            verify(!map.hasFix)
        } finally {
            map.destroy()
        }
    }

    function test_navigateReachesMobile() {
        AppState.navigate("mobile")
        compare(AppState.currentView, "mobile")
    }
}
