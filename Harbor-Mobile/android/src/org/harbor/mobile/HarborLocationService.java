package org.harbor.mobile;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.location.Location;
import android.location.LocationListener;
import android.location.LocationManager;
import android.os.Build;
import android.os.IBinder;
import android.os.Looper;

import androidx.core.app.NotificationCompat;

/**
 * Foreground location service (type {@code location} on API 29+): the only
 * component allowed to keep receiving fixes for sharing. Runs solely while
 * "Share location" is ON with a grant; stopping clears the cached fix so a
 * stale position is never reported as fresh.
 */
public class HarborLocationService extends Service {

    public static final String ACTION_START = "org.harbor.mobile.LOCATION_START";
    public static final String ACTION_STOP = "org.harbor.mobile.LOCATION_STOP";
    private static final String CHANNEL_ID = "harbor_location";
    private static final int NOTIFICATION_ID = 6103;

    private static final long MIN_INTERVAL_MILLIS = 60_000L;
    private static final float MIN_DISTANCE_METERS = 50.0f;

    private LocationListener listener;

    @Override
    public void onCreate() {
        super.onCreate();
        NotificationManager manager = getSystemService(NotificationManager.class);
        if (manager != null) {
            NotificationChannel channel = new NotificationChannel(
                    CHANNEL_ID, "Location sharing", NotificationManager.IMPORTANCE_LOW);
            channel.setShowBadge(false);
            manager.createNotificationChannel(channel);
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent != null && ACTION_STOP.equals(intent.getAction())) {
            stopTracking();
            stopSelf();
            return START_NOT_STICKY;
        }
        if (!HarborMobileActivity.hasLocationPermission(this)) {
            stopSelf();
            return START_NOT_STICKY;
        }
        if (!getSharedPreferences("harbor_mobile", MODE_PRIVATE)
                .getBoolean("location_intent", false)) {
            stopSelf();
            return START_NOT_STICKY;
        }
        Notification notification = new NotificationCompat.Builder(this, CHANNEL_ID)
                .setSmallIcon(android.R.drawable.ic_menu_mylocation)
                .setContentTitle("Harbor is sharing your location")
                .setContentText("Turn off Share location to stop")
                .setOngoing(true)
                .setShowWhen(false)
                .build();
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(NOTIFICATION_ID, notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_LOCATION);
        } else {
            startForeground(NOTIFICATION_ID, notification);
        }
        startTracking();
        return START_STICKY;
    }

    private void startTracking() {
        if (listener != null) {
            return;
        }
        LocationManager manager = getSystemService(LocationManager.class);
        if (manager == null) {
            return;
        }
        // The Google fused provider is not part of the base Qt APK. Use the
        // platform LocationManager as an explicit fallback: network first
        // for balanced power, GPS when that is the only enabled provider.
        final String provider = manager.isProviderEnabled(LocationManager.NETWORK_PROVIDER)
                ? LocationManager.NETWORK_PROVIDER
                : manager.isProviderEnabled(LocationManager.GPS_PROVIDER)
                        ? LocationManager.GPS_PROVIDER : null;
        if (provider == null) {
            return;
        }
        try {
            listener = new LocationListener() {
                @Override
                public void onLocationChanged(Location location) {
                    HarborMobileActivity.noteFix(location);
                }

                @Override
                public void onProviderDisabled(String provider) {}

                @Override
                public void onProviderEnabled(String provider) {}

            };
            manager.requestLocationUpdates(provider,
                    MIN_INTERVAL_MILLIS, MIN_DISTANCE_METERS, listener, Looper.getMainLooper());
        } catch (SecurityException e) {
            listener = null;
            stopSelf();
        }
    }

    private void stopTracking() {
        LocationManager manager = getSystemService(LocationManager.class);
        if (manager != null && listener != null) {
            manager.removeUpdates(listener);
        }
        listener = null;
        HarborMobileActivity.noteFix(null);
    }

    @Override
    public void onDestroy() {
        stopTracking();
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
