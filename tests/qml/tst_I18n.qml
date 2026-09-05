import QtQuick
import QtTest
import Harbor 2.0

TestCase {
    name: "I18n"

    function init() {
        AppState.locale = "en"
    }

    function cleanup() {
        AppState.locale = "en"
    }

    function test_localeNormalization() {
        compare(I18n.normalizeLocale("pt_BR"), "pt-BR")
        compare(I18n.normalizeLocale("pt-PT"), "pt-BR")
        compare(I18n.normalizeLocale("unknown"), "en")
    }

    function test_missingKeyFallsBackToKey() {
        compare(I18n.t("test.key.that.does.not.exist"), "test.key.that.does.not.exist")
    }

    function test_interpolationAndPlural() {
        compare(I18n.t("common.time.minutesAgo", { count: 1 }), "1 minute ago")
        compare(I18n.t("common.time.minutesAgo", { count: 2 }), "2 minutes ago")
    }

    function test_percentContracts() {
        compare(I18n.percent(0.92), "92%")
        compare(I18n.percent(92, { isRatio: false }), "92%")
    }

    function test_runtimeLocale() {
        AppState.locale = "pt-BR"
        tryCompare(I18n, "locale", "pt-BR")
        compare(I18n.number(1234.5), "1.234,5")
        AppState.locale = "en"
        tryCompare(I18n, "locale", "en")
        compare(I18n.number(1234.5), "1,234.5")
    }

    function _placeholders(message) {
        var tokens = []
        var matcher = /\{([A-Za-z0-9_]+)\}/g
        var match
        while ((match = matcher.exec(String(message))) !== null) {
            if (tokens.indexOf(match[1]) < 0)
                tokens.push(match[1])
        }
        return tokens.sort().join("|")
    }

    function test_catalogsKeepKeyParity() {
        var english = I18n._catalog("en")
        var portuguese = I18n._catalog("pt-BR")
        compare(Object.keys(portuguese).sort().join("|"),
                Object.keys(english).sort().join("|"))
    }

    function test_pluralEntriesHaveBothForms() {
        var catalogs = [I18n._catalog("en"), I18n._catalog("pt-BR")]
        for (var ci = 0; ci < catalogs.length; ++ci) {
            var catalog = catalogs[ci]
            var keys = Object.keys(catalog)
            for (var ki = 0; ki < keys.length; ++ki) {
                var value = catalog[keys[ki]]
                if (value !== null && typeof value === "object") {
                    verify(typeof value.one === "string",
                           keys[ki] + " should define plural form 'one'")
                    verify(typeof value.other === "string",
                           keys[ki] + " should define plural form 'other'")
                }
            }
        }
    }

    function test_placeholdersMatchAcrossCatalogs() {
        var english = I18n._catalog("en")
        var portuguese = I18n._catalog("pt-BR")
        var keys = Object.keys(english)
        for (var ki = 0; ki < keys.length; ++ki) {
            var key = keys[ki]
            var englishValue = english[key]
            var portugueseValue = portuguese[key]
            if (englishValue !== null && typeof englishValue === "object") {
                compare(_placeholders(portugueseValue.one), _placeholders(englishValue.one),
                        key + ".one placeholders must match")
                compare(_placeholders(portugueseValue.other), _placeholders(englishValue.other),
                        key + ".other placeholders must match")
            } else {
                compare(_placeholders(portugueseValue), _placeholders(englishValue),
                        key + " placeholders must match")
            }
        }
    }
}
