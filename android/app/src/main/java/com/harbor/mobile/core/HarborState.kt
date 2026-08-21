package com.harbor.mobile.core

import com.harbor.mobile.net.IncomingMessage
import com.harbor.mobile.net.PresenceState
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

enum class ConnectionStatus { CONNECTING, ONLINE, OFFLINE }

data class HarborStatus(
    val connection: ConnectionStatus = ConnectionStatus.OFFLINE,
    val presence: PresenceState = PresenceState.OFFLINE,
    val activity: String? = null,
    val lastActivityTsMs: Long? = null,
    val lastEventTsMs: Long? = null,
)

/** Pure receive-side state machine; target and timestamp checks happen here too. */
class HarborState(private val targetId: String) {
    private val _status = MutableStateFlow(HarborStatus())
    private val lock = Any()
    val status: StateFlow<HarborStatus> = _status.asStateFlow()

    fun markConnecting() = synchronized(lock) {
        _status.value = _status.value.copy(connection = ConnectionStatus.CONNECTING)
    }

    fun markConnected() = synchronized(lock) {
        _status.value = _status.value.copy(connection = ConnectionStatus.ONLINE)
    }

    fun markDisconnected() = synchronized(lock) {
        _status.value = _status.value.copy(connection = ConnectionStatus.OFFLINE)
    }

    /** Returns false when the event is for another device or is older than state. */
    fun apply(message: IncomingMessage, nowMs: Long = System.currentTimeMillis()): Boolean {
        if (message.deviceId != targetId) return false
        val eventMs = message.timestampSeconds?.toEpochMillis() ?: nowMs
        return synchronized(lock) {
            val current = _status.value
            if (current.lastEventTsMs != null && eventMs < current.lastEventTsMs) {
                false
            } else {
                _status.value = when (message) {
                    is IncomingMessage.Presence -> current.copy(
                        connection = if (message.state == PresenceState.OFFLINE) {
                            ConnectionStatus.OFFLINE
                        } else {
                            ConnectionStatus.ONLINE
                        },
                        presence = message.state,
                        activity = if (message.state == PresenceState.OFFLINE) null else current.activity,
                        lastActivityTsMs = if (message.state == PresenceState.OFFLINE) null else current.lastActivityTsMs,
                        lastEventTsMs = eventMs,
                    )
                    is IncomingMessage.Activity -> current.copy(
                        activity = message.app,
                        lastActivityTsMs = eventMs,
                        lastEventTsMs = eventMs,
                    )
                }
                true
            }
        }
    }

    private fun Double.toEpochMillis(): Long = (this * 1000.0).toLong()
}
