package com.harbor.mobile.net

import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrl

/** OkHttp parses only HTTP schemes; accept the same ws/wss URL users use for the relay. */
internal fun relayHttpUrl(raw: String): HttpUrl {
    val value = raw.trim()
    val normalized = when {
        value.startsWith("ws://", ignoreCase = true) -> "http://${value.substring(5)}"
        value.startsWith("wss://", ignoreCase = true) -> "https://${value.substring(6)}"
        else -> value
    }
    val url = normalized.toHttpUrl()
    require(url.scheme == "https" || url.host == "localhost" || url.host == "127.0.0.1" || url.host == "::1") {
        "The relay must use HTTPS/WSS except for loopback development"
    }
    return url
}
