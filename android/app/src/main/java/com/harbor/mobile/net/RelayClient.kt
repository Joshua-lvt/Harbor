package com.harbor.mobile.net

import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.InternalCoroutinesApi
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import org.json.JSONObject

class RelayClient(
    private val credentials: com.harbor.mobile.storage.Credentials,
    private val scope: CoroutineScope,
    private val listener: Listener,
    private val httpClient: OkHttpClient = OkHttpClient(),
) {
    interface Listener {
        fun onConnectionChanged(status: ConnectionStatus)
        fun onMessage(message: IncomingMessage)
    }

    enum class ConnectionStatus { CONNECTING, CONNECTED, DISCONNECTED }

    private var loop: Job? = null
    @Volatile private var socket: WebSocket? = null

    fun start() {
        if (loop?.isActive == true) return
        loop = scope.launch {
            var failures = 0
            while (isActive) {
                listener.onConnectionChanged(ConnectionStatus.CONNECTING)
                val stayedOpen = try {
                    connectOnce()
                } catch (_: CancellationException) {
                    return@launch
                } catch (_: Throwable) {
                    false
                }
                if (!isActive) break
                val delayIndex = failures.coerceIn(0, RECONNECT_BACKOFF_MS.lastIndex)
                if (stayedOpen) failures = 0 else failures++
                delay(RECONNECT_BACKOFF_MS[delayIndex])
            }
        }
    }

    fun stop() {
        loop?.cancel()
        loop = null
        socket?.close(1000, "client stopped")
        socket = null
        listener.onConnectionChanged(ConnectionStatus.DISCONNECTED)
    }

    @OptIn(InternalCoroutinesApi::class)
    private suspend fun connectOnce(): Boolean = suspendCancellableCoroutine { continuation ->
        val callbackLock = Any()
        var completed = false
        var heartbeat: Job? = null
        var currentSocket: WebSocket? = null

        fun finish(stayedOpen: Boolean) {
            val shouldNotify = synchronized(callbackLock) {
                if (completed) {
                    false
                } else {
                    completed = true
                    heartbeat?.cancel()
                    heartbeat = null
                    socket = null
                    val token = continuation.tryResume(stayedOpen)
                    if (token != null) continuation.completeResume(token)
                    true
                }
            }
            if (shouldNotify) listener.onConnectionChanged(ConnectionStatus.DISCONNECTED)
        }

        val base = relayHttpUrl(credentials.relayUrl)
        // OkHttp's HttpUrl.Builder only accepts http/https schemes. Its
        // WebSocket implementation upgrades this HTTP(S) request internally.
        val wsUrl = base.newBuilder()
            .addPathSegments("ws")
            .addQueryParameter("device_id", credentials.deviceId)
            .addQueryParameter("secret", credentials.deviceSecret)
            .build()
        val request = Request.Builder().url(wsUrl).build()
        val openedSocket = httpClient.newWebSocket(request, object : WebSocketListener() {
            override fun onOpen(webSocket: WebSocket, response: Response) {
                val shouldRun = synchronized(callbackLock) {
                    if (completed) {
                        false
                    } else {
                        socket = webSocket
                        heartbeat = scope.launch {
                            while (isActive) {
                                webSocket.send(JSONObject().put("type", "heartbeat").toString())
                                delay(Protocol.HEARTBEAT_INTERVAL_MS)
                            }
                        }
                        true
                    }
                }
                if (!shouldRun) {
                    webSocket.cancel()
                    return
                }
                listener.onConnectionChanged(ConnectionStatus.CONNECTED)
            }

            override fun onMessage(webSocket: WebSocket, text: String) {
                Protocol.decode(text)?.let(listener::onMessage)
            }

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                webSocket.close(code, reason)
                finish(true)
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                finish(true)
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                finish(false)
            }
        })
        synchronized(callbackLock) {
            currentSocket = openedSocket
            if (completed) openedSocket.cancel() else socket = openedSocket
        }
        continuation.invokeOnCancellation {
            synchronized(callbackLock) {
                completed = true
                heartbeat?.cancel()
                heartbeat = null
                currentSocket?.cancel()
                socket = null
            }
        }
    }

    companion object {
        val RECONNECT_BACKOFF_MS = longArrayOf(1_000L, 2_000L, 4_000L, 8_000L, 15_000L)
    }
}
