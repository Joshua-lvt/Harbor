package com.harbor.mobile.notify

import android.Manifest
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.content.Context
import android.content.pm.PackageManager
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.content.ContextCompat
import com.harbor.mobile.R
import com.harbor.mobile.core.ConnectionStatus
import com.harbor.mobile.core.HarborStatus
import com.harbor.mobile.net.PresenceState

class HarborNotifier(private val context: Context) {
    init {
        val manager = context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(
                CHANNEL_ID,
                context.getString(R.string.notification_channel_name),
                NotificationManager.IMPORTANCE_LOW,
            ).apply {
                description = context.getString(R.string.notification_channel_description)
                setShowBadge(false)
            },
        )
    }

    fun build(status: HarborStatus): Notification = NotificationCompat.Builder(context, CHANNEL_ID)
        .setSmallIcon(R.drawable.ic_harbor)
        .setContentTitle("Harbor")
        .setContentText(text(status))
        .setStyle(NotificationCompat.BigTextStyle().bigText(text(status)))
        .setOngoing(true)
        .setOnlyAlertOnce(true)
        .setSilent(true)
        .setCategory(NotificationCompat.CATEGORY_SERVICE)
        .build()

    fun update(status: HarborStatus) {
        if (!notificationsAllowed()) return
        NotificationManagerCompat.from(context).notify(NOTIFICATION_ID, build(status))
    }

    fun notificationsAllowed(): Boolean =
        android.os.Build.VERSION.SDK_INT < 33 ||
            ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

    private fun text(status: HarborStatus): String {
        val connection = when {
            status.connection == ConnectionStatus.CONNECTING -> "🟡 Conectando ao PC"
            status.connection == ConnectionStatus.OFFLINE -> "⚫ PC offline"
            status.presence == PresenceState.ONLINE -> "🟢 PC online"
            status.presence == PresenceState.AWAY -> "🟡 PC ausente"
            else -> "⚫ PC offline"
        }
        return if (status.presence != PresenceState.OFFLINE && status.activity != null) {
            "🖥️ Usando ${status.activity} · $connection"
        } else {
            connection
        }
    }

    companion object {
        const val CHANNEL_ID = "harbor_connection"
        const val NOTIFICATION_ID = 4101
    }
}
