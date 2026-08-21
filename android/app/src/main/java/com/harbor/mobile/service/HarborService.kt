package com.harbor.mobile.service

import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.ServiceCompat
import com.harbor.mobile.core.HarborState
import com.harbor.mobile.net.RelayClient
import com.harbor.mobile.notify.HarborNotifier
import com.harbor.mobile.storage.CredentialsStore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch

class HarborService : Service() {
    private val serviceJob = SupervisorJob()
    private val serviceScope = CoroutineScope(Dispatchers.IO + serviceJob)
    private var relayClient: RelayClient? = null
    private lateinit var notifier: HarborNotifier

    override fun onCreate() {
        super.onCreate()
        notifier = HarborNotifier(this)
        val initial = com.harbor.mobile.core.HarborStatus()
        ServiceCompat.startForeground(
            this,
            HarborNotifier.NOTIFICATION_ID,
            notifier.build(initial),
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                ServiceInfo.FOREGROUND_SERVICE_TYPE_CONNECTED_DEVICE
            } else {
                0
            },
        )

        val credentials = CredentialsStore(this).load()
        if (credentials == null) {
            stopSelf()
            return
        }

        val harborState = HarborState(credentials.targetId)
        serviceScope.launch {
            harborState.status.collectLatest { status ->
                HarborRuntime.publish(status)
                notifier.update(status)
            }
        }
        relayClient = RelayClient(
            credentials = credentials,
            scope = serviceScope,
            listener = object : RelayClient.Listener {
                override fun onConnectionChanged(status: RelayClient.ConnectionStatus) {
                    when (status) {
                        RelayClient.ConnectionStatus.CONNECTING -> harborState.markConnecting()
                        RelayClient.ConnectionStatus.CONNECTED -> harborState.markConnected()
                        RelayClient.ConnectionStatus.DISCONNECTED -> harborState.markDisconnected()
                    }
                }

                override fun onMessage(message: com.harbor.mobile.net.IncomingMessage) {
                    harborState.apply(message)
                }
            },
        ).also { it.start() }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int = START_STICKY

    override fun onDestroy() {
        relayClient?.stop()
        serviceScope.cancel()
        super.onDestroy()
    }

    override fun onBind(intent: Intent?): IBinder? = null

    companion object {
        fun start(context: Context) {
            val intent = Intent(context, HarborService::class.java)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) context.startForegroundService(intent)
            else context.startService(intent)
        }
    }
}
