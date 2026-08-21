package com.harbor.mobile.net

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.doubleOrNull
import kotlinx.serialization.json.jsonPrimitive

/** The small receive-side subset understood by the V0.1 observer. */
sealed interface IncomingMessage {
    val deviceId: String
    val timestampSeconds: Double?

    data class Presence(
        override val deviceId: String,
        val state: PresenceState,
        override val timestampSeconds: Double?,
        val lastSeenSeconds: Double?,
    ) : IncomingMessage

    data class Activity(
        override val deviceId: String,
        val app: String?,
        override val timestampSeconds: Double?,
    ) : IncomingMessage
}

enum class PresenceState { ONLINE, AWAY, OFFLINE }

object Protocol {
    const val HEARTBEAT_INTERVAL_MS = 60_000L
    private val json = Json { ignoreUnknownKeys = true; isLenient = true }

    /**
     * Parses only presence/activity. Unknown, malformed, and command messages are
     * deliberately discarded so a future peer protocol addition cannot crash the app.
     */
    fun decode(raw: String): IncomingMessage? {
        val root = runCatching { json.parseToJsonElement(raw) as? JsonObject }.getOrNull() ?: return null
        val type = root.string("type") ?: return null
        val deviceId = root.string("device_id") ?: return null
        val ts = root.number("ts")
        return when (type) {
            "presence" -> {
                val state = when (root.string("state")) {
                    "online" -> PresenceState.ONLINE
                    "away" -> PresenceState.AWAY
                    "offline" -> PresenceState.OFFLINE
                    else -> return null
                }
                IncomingMessage.Presence(deviceId, state, ts, root.number("last_seen"))
            }
            "activity" -> IncomingMessage.Activity(deviceId, root.stringOrNull("app"), ts)
            else -> null
        }
    }

    private fun JsonObject.string(key: String): String? =
        (this[key] as? JsonPrimitive)?.contentOrNull?.takeIf { it.isNotEmpty() }

    private fun JsonObject.stringOrNull(key: String): String? =
        (this[key] as? JsonPrimitive)?.contentOrNull

    private fun JsonObject.number(key: String): Double? =
        (this[key] as? JsonPrimitive)?.doubleOrNull
}
