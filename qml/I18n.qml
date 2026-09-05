pragma Singleton
import QtQuick
import "i18n/en.js" as English
import "i18n/pt_BR.js" as BrazilianPortuguese

QtObject {
    id: root

    property string locale: "en"
    readonly property var supportedLocales: ["en", "pt-BR"]
    readonly property string languageName: locale === "pt-BR" ? "Português (Brasil)" : "English"

    onLocaleChanged: {
        var normalized = normalizeLocale(locale)
        if (locale !== normalized)
            locale = normalized
    }

    function normalizeLocale(value) {
        var candidate = String(value || "en").replace(/_/g, "-").toLowerCase()
        if (candidate === "pt" || candidate.indexOf("pt-") === 0)
            return "pt-BR"
        return "en"
    }

    function setLocale(value) {
        locale = normalizeLocale(value)
    }

    function _catalog(localeName) {
        return localeName === "pt-BR" ? BrazilianPortuguese.catalog : English.catalog
    }

    function _entry(key) {
        var selected = _catalog(locale)
        var value = selected[key]
        if (typeof value === "undefined")
            value = English.catalog[key]
        return value
    }

    function _pluralForm(count) {
        return Number(count) === 1 ? "one" : "other"
    }

    function _interpolate(message, params) {
        if (!params)
            return String(message)
        return String(message).replace(/\{([A-Za-z0-9_]+)\}/g, function(match, name) {
            var value = params[name]
            return value === undefined || value === null ? match : String(value)
        })
    }

    function t(key, params) {
        var values = params
        if (typeof params === "number")
            values = { count: params }

        var value = _entry(String(key))
        if (typeof value === "undefined")
            return String(key)

        if (value !== null && typeof value === "object") {
            var forms = value
            var form = _pluralForm(values && values.count)
            value = forms[form]
            if (typeof value === "undefined")
                value = forms.other
        }
        return _interpolate(value, values)
    }

    function _repeat(character, count) {
        var result = ""
        for (var i = 0; i < count; ++i)
            result += character
        return result
    }

    function number(value, options) {
        var numeric = Number(value)
        if (!isFinite(numeric))
            return t("common.notAvailable")

        var config = options || {}
        var minimum = config.minimumFractionDigits === undefined ? 0 : Math.max(0, Math.floor(config.minimumFractionDigits))
        var maximum = config.maximumFractionDigits === undefined ? Math.max(minimum, 2) : Math.max(minimum, Math.floor(config.maximumFractionDigits))
        if (config.decimals !== undefined) {
            minimum = Math.max(0, Math.floor(config.decimals))
            maximum = minimum
        }
        maximum = Math.min(maximum, 12)

        var negative = numeric < 0
        var absolute = Math.abs(numeric)
        var fixed = absolute.toFixed(maximum)
        var pieces = fixed.split(".")
        var whole = pieces[0]
        var fraction = pieces.length > 1 ? pieces[1] : ""
        while (fraction.length > minimum && fraction.charAt(fraction.length - 1) === "0")
            fraction = fraction.slice(0, -1)

        if (config.useGrouping !== false) {
            var grouped = ""
            while (whole.length > 3) {
                grouped = (locale === "pt-BR" ? "." : ",") + whole.slice(-3) + grouped
                whole = whole.slice(0, -3)
            }
            whole += grouped
        }

        var result = whole
        if (fraction.length > 0)
            result += (locale === "pt-BR" ? "," : ".") + fraction
        if (negative && absolute !== 0)
            result = "−" + result
        return result
    }

    function percent(value, options) {
        var config = options || {}
        var numeric = Number(value)
        if (!isFinite(numeric))
            return t("common.notAvailable")
        if (config.isRatio !== false)
            numeric *= 100
        var numberOptions = {
            minimumFractionDigits: config.minimumFractionDigits === undefined ? 0 : config.minimumFractionDigits,
            maximumFractionDigits: config.maximumFractionDigits === undefined ? 0 : config.maximumFractionDigits,
            useGrouping: config.useGrouping
        }
        if (config.decimals !== undefined)
            numberOptions.decimals = config.decimals
        return number(numeric, numberOptions) + "%"
    }

    function _pad2(value) {
        return value < 10 ? "0" + value : String(value)
    }

    function duration(totalSeconds, options) {
        var numeric = Number(totalSeconds)
        if (!isFinite(numeric))
            return t("common.notAvailable")

        var config = options || {}
        var seconds = Math.max(0, Math.round(numeric))
        var hours = Math.floor(seconds / 3600)
        var minutes = Math.floor((seconds % 3600) / 60)
        var remainder = seconds % 60

        if (config.style === "clock")
            return hours > 0 ? hours + ":" + _pad2(minutes) + ":" + _pad2(remainder) : minutes + ":" + _pad2(remainder)

        var parts = []
        if (hours > 0)
            parts.push(t("unit.duration.hour", { count: hours, value: number(hours, { maximumFractionDigits: 0 }) }))
        if (minutes > 0)
            parts.push(t("unit.duration.minute", { count: minutes, value: number(minutes, { maximumFractionDigits: 0 }) }))
        if (remainder > 0 || parts.length === 0)
            parts.push(t("unit.duration.second", { count: remainder, value: number(remainder, { maximumFractionDigits: 0 }) }))

        var maxParts = config.maxParts === undefined ? 2 : Math.max(1, Math.floor(config.maxParts))
        return parts.slice(0, maxParts).join(" ")
    }

    function unit(value, unitName, options) {
        var config = options || {}
        var aliases = {
            ms: "millisecond",
            s: "second",
            sec: "second",
            min: "minute",
            h: "hour",
            hr: "hour",
            d: "day",
            B: "byte",
            KB: "kilobyte",
            MB: "megabyte",
            GB: "gigabyte",
            TB: "terabyte",
            bps: "bitPerSecond",
            kbps: "kilobitPerSecond",
            Mbps: "megabitPerSecond",
            Gbps: "gigabitPerSecond"
        }
        var canonical = aliases[unitName] || String(unitName)
        var style = config.style === "long" ? "long" : "short"
        var key = "unit." + canonical + "." + style
        var entry = _entry(key)
        var formatted = number(value, config)
        if (typeof entry === "undefined")
            return formatted + " " + String(unitName)
        return t(key, { count: Number(value), value: formatted })
    }
}
