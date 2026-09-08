package org.harbor.mobile;

import android.app.AppOpsManager;
import android.app.usage.UsageEvents;
import android.app.usage.UsageStatsManager;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.os.Build;
import android.os.Process;
import android.provider.Settings;

/**
 * Phone-activity facts from UsageStatsManager. Foreground-app labels only
 * from recent MOVE_TO_FOREGROUND events; otherwise the caller must report
 * a coarse "phone active", never a guessed package.
 */
public final class HarborUsage {
    private HarborUsage() {}

    public static boolean hasAccess(Context context) {
        AppOpsManager ops = context.getSystemService(AppOpsManager.class);
        if (ops == null) {
            return false;
        }
        int mode = ops.checkOpNoThrow(
                AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), context.getPackageName());
        return mode == AppOpsManager.MODE_ALLOWED;
    }

    public static Intent accessSettingsIntent() {
        return new Intent(Settings.ACTION_USAGE_ACCESS_SETTINGS);
    }

    /** Foreground package within the last minute, or null when unknown. */
    @SuppressWarnings("deprecation")
    public static String foregroundPackage(Context context) {
        UsageStatsManager stats = context.getSystemService(UsageStatsManager.class);
        if (stats == null || !hasAccess(context)) {
            return null;
        }
        long now = System.currentTimeMillis();
        UsageEvents events = stats.queryEvents(now - 60_000L, now);
        String current = null;
        UsageEvents.Event event = new UsageEvents.Event();
        while (events.hasNextEvent()) {
            events.getNextEvent(event);
            boolean foreground = event.getEventType() == UsageEvents.Event.MOVE_TO_FOREGROUND;
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                foreground = foreground
                        || event.getEventType() == UsageEvents.Event.ACTIVITY_RESUMED;
            }
            if (foreground) {
                current = event.getPackageName();
            }
        }
        return current;
    }

    /** Display label for a package, or null when it cannot be resolved. */
    public static String appLabel(Context context, String packageName) {
        if (packageName == null) {
            return null;
        }
        try {
            PackageManager pm = context.getPackageManager();
            CharSequence label = pm.getApplicationLabel(
                    pm.getApplicationInfo(packageName, 0));
            String text = label == null ? null : label.toString().trim();
            return (text == null || text.isEmpty()) ? null : text;
        } catch (PackageManager.NameNotFoundException e) {
            return null;
        }
    }

    public static long lastUsedMillis(Context context, String packageName) {
        UsageStatsManager stats = context.getSystemService(UsageStatsManager.class);
        if (stats == null || !hasAccess(context) || packageName == null) {
            return -1L;
        }
        long now = System.currentTimeMillis();
        return stats.queryUsageStats(
                        UsageStatsManager.INTERVAL_DAILY, now - 24 * 3_600_000L, now)
                .stream()
                .filter(s -> packageName.equals(s.getPackageName()))
                .mapToLong(s -> {
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                        return Math.max(s.getLastTimeUsed(), s.getLastTimeVisible());
                    }
                    return s.getLastTimeUsed();
                })
                .max()
                .orElse(-1L);
    }
}
