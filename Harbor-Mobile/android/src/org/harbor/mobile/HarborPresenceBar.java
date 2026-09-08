package org.harbor.mobile;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.os.IBinder;

/**
 * Persistent partner presence bar: an ongoing low-importance notification
 * rendering the committed aggregate (name, ONLINE/AWAY/OFFLINE, shared
 * current app). Updated on presence.changed; tapping opens Harbor Chat.
 * Foreground-service type is declared in the manifest per target API
 * enforcement (location/microphone types attach to their own services).
 */
public class HarborPresenceBar extends Service {

    public static final String CHANNEL_ID = "harbor_presence";
    public static final int NOTIFICATION_ID = 6101;

    public static final String EXTRA_NAME = "org.harbor.mobile.NAME";
    public static final String EXTRA_STATE = "org.harbor.mobile.STATE";
    public static final String EXTRA_DETAIL = "org.harbor.mobile.DETAIL";
    public static final String ACTION_UPDATE =
            "org.harbor.mobile.ACTION_UPDATE_PRESENCE";
    public static final String ACTION_STOP =
            "org.harbor.mobile.ACTION_STOP_PRESENCE";

    @Override
    public void onCreate() {
        super.onCreate();
        NotificationManager manager = getSystemService(NotificationManager.class);
        if (manager != null) {
            NotificationChannel channel = new NotificationChannel(
                    CHANNEL_ID,
                    "Partner presence",
                    NotificationManager.IMPORTANCE_LOW);
            channel.setShowBadge(false);
            manager.createNotificationChannel(channel);
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent != null && ACTION_STOP.equals(intent.getAction())) {
            stopForeground(STOP_FOREGROUND_REMOVE);
            stopSelf();
            return START_NOT_STICKY;
        }
        if (intent != null && ACTION_UPDATE.equals(intent.getAction())) {
            show(intent.getStringExtra(EXTRA_NAME),
                    intent.getStringExtra(EXTRA_STATE),
                    intent.getStringExtra(EXTRA_DETAIL));
        }
        // The host republishes the bar while paired. Do not resurrect a
        // stale partner name after process death before the core has
        // re-established a session.
        return START_NOT_STICKY;
    }

    @Override
    public void onDestroy() {
        stopForeground(STOP_FOREGROUND_REMOVE);
        super.onDestroy();
    }

    private void show(String name, String state, String detail) {
        String title = name == null || name.isEmpty() ? "Harbor" : name;
        String line = state == null ? "" : state;
        if (detail != null && !detail.isEmpty()) {
            line = line.isEmpty() ? detail : line + " · " + detail;
        }
        int icon = android.R.drawable.presence_offline;
        if ("Online".equals(state)) {
            icon = android.R.drawable.presence_online;
        } else if ("Away".equals(state)) {
            icon = android.R.drawable.presence_away;
        }
        Intent open = getPackageManager().getLaunchIntentForPackage(getPackageName());
        PendingIntent tap = open == null ? null : PendingIntent.getActivity(
                this, 0, open,
                PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        Notification notification = new Notification.Builder(this, CHANNEL_ID)
                .setSmallIcon(icon)
                .setContentTitle(title)
                .setContentText(line)
                .setOngoing(true)
                .setShowWhen(false)
                .setContentIntent(tap)
                .build();
        try {
            startForeground(NOTIFICATION_ID, notification);
        } catch (Exception e) {
            // A presence change can arrive while the app is backgrounded,
            // where starting a foreground service is refused. Keep the same
            // bar as a plain ongoing notification instead of crashing; the
            // next foreground update re-establishes the service form.
            NotificationManager manager = getSystemService(NotificationManager.class);
            if (manager != null) {
                manager.notify(NOTIFICATION_ID, notification);
            }
        }
    }

    public static void update(Context context, String name, String state, String detail) {
        Intent intent = new Intent(context, HarborPresenceBar.class);
        intent.setAction(ACTION_UPDATE);
        intent.putExtra(EXTRA_NAME, name);
        intent.putExtra(EXTRA_STATE, state);
        intent.putExtra(EXTRA_DETAIL, detail);
        try {
            context.startForegroundService(intent);
        } catch (Exception e) {
            try {
                context.startService(intent);
            } catch (Exception ignored) {
                // Background start refused while the app is fully stopped:
                // nothing to show until the next live update.
            }
        }
    }

    public static void stop(Context context) {
        try {
            Intent intent = new Intent(context, HarborPresenceBar.class);
            intent.setAction(ACTION_STOP);
            context.startService(intent);
        } catch (Exception ignored) {
            // Service already gone with a dead process; nothing to stop.
        }
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
