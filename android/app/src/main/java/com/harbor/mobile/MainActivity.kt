package com.harbor.mobile

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.core.app.NotificationManagerCompat
import com.harbor.mobile.service.HarborService
import com.harbor.mobile.storage.CredentialsStore
import com.harbor.mobile.ui.HomeScreen
import com.harbor.mobile.ui.PairingScreen

class MainActivity : ComponentActivity() {
    private lateinit var store: CredentialsStore
    private val notificationPermissionState = mutableStateOf(false)
    private var serviceStartPending = false

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        store = CredentialsStore(this)
        notificationPermissionState.value = notificationsAllowed()
        serviceStartPending = store.load() != null
        requestNotificationPermissionIfNeeded()
        startServiceIfReady()

        setContent {
            MaterialTheme {
                var connected by remember { mutableStateOf(store.load() != null) }
                val notificationsEnabled by notificationPermissionState
                if (connected) {
                    HomeScreen(notificationEnabled = notificationsEnabled)
                } else {
                    PairingScreen(
                        store = store,
                        onConnected = {
                            connected = true
                            serviceStartPending = true
                            startServiceIfReady()
                        },
                    )
                }
            }
        }
    }

    override fun onResume() {
        super.onResume()
        notificationPermissionState.value = notificationsAllowed()
        startServiceIfReady()
    }

    override fun onRequestPermissionsResult(requestCode: Int, permissions: Array<String>, grantResults: IntArray) {
        super.onRequestPermissionsResult(requestCode, permissions, grantResults)
        if (requestCode == NOTIFICATION_PERMISSION_REQUEST) {
            notificationPermissionState.value = notificationsAllowed()
            startServiceIfReady()
        }
    }

    private fun requestNotificationPermissionIfNeeded() {
        if (Build.VERSION.SDK_INT >= 33 &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), NOTIFICATION_PERMISSION_REQUEST)
        }
    }

    private fun notificationsAllowed(): Boolean =
        (Build.VERSION.SDK_INT < 33 ||
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) &&
            NotificationManagerCompat.from(this).areNotificationsEnabled()

    private fun startServiceIfReady() {
        if (serviceStartPending && notificationPermissionState.value && store.load() != null) {
            serviceStartPending = false
            HarborService.start(this)
        }
    }

    companion object {
        private const val NOTIFICATION_PERMISSION_REQUEST = 1001
    }
}
