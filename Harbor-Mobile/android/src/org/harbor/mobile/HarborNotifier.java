package org.harbor.mobile;

import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.content.Context;
import android.content.Intent;

import androidx.core.app.NotificationCompat;

/**
 * Harbor's own system notifications (new message, partner presence) for
 * moments the app is backgrounded. Foreground delivery stays in-app;
 * nothing here duplicates the in-app widget, and message bodies follow
 * the message-preview setting passed in by the caller.
 */
public final class HarborNotifier {
    private static final String CHANNEL_ID = "harbor_messages";
    private static int nextId = 6200;

    private HarborNotifier() {}

    public static void post(Context context, String title, String text) {
        NotificationManager manager = context.getSystemService(NotificationManager.class);
        if (manager == null) {
            return;
        }
        NotificationChannel channel = new NotificationChannel(
                CHANNEL_ID, "Harbor messages", NotificationManager.IMPORTANCE_DEFAULT);
        manager.createNotificationChannel(channel);
        Intent open = context.getPackageManager().getLaunchIntentForPackage(context.getPackageName());
        PendingIntent tap = open == null ? null : PendingIntent.getActivity(
                context, 0, open, PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        manager.notify(nextId++,
                new NotificationCompat.Builder(context, CHANNEL_ID)
                        .setSmallIcon(android.R.drawable.sym_action_chat)
                        .setContentTitle(title == null || title.isEmpty() ? "Harbor" : title)
                        .setContentText(text == null ? "" : text)
                        .setContentIntent(tap)
                        .setAutoCancel(true)
                        .build());
    }
}
