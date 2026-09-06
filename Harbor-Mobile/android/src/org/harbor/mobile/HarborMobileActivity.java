package org.harbor.mobile;

import android.Manifest;
import android.app.AppOpsManager;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.location.Location;
import android.location.LocationManager;
import android.os.Build;
import android.os.Process;
import android.provider.Settings;
import android.net.Uri;
import android.os.PowerManager;

import androidx.core.app.ActivityCompat;
import androidx.core.content.ContextCompat;

import org.qtproject.qt.android.bindings.QtActivity;

/**
 * Harbor's Android entry point. Owns the runtime-permission flows and the
 * small cached facts the C++ facade polls (location fix, service state);
 * continuous work lives in the foreground services, never here.
 */
public class HarborMobileActivity extends QtActivity {

    public static final int LOCATION_REQUEST = 6102;
    public static final int MICROPHONE_REQUEST = 6104;

    private static volatile Location lastFix;
    private static volatile boolean locationActive;

    /** Registered by the Qt native facade during application startup. */
    public static native void nativePhoneNotification(
            String appLabel, String title, String text, long postedAt);
    public static native void nativePermissionChanged();

    // ---- settings pages (just-in-time, per feature) ----

    public static void openSystemSettings(Context context, String page) {
        Intent intent;
        if ("usage".equals(page)) {
            intent = HarborUsage.accessSettingsIntent();
        } else if ("notifications".equals(page)) {
            intent = HarborPhoneNotifications.accessSettingsIntent();
        } else if ("location".equals(page)) {
            intent = new Intent(Settings.ACTION_LOCATION_SOURCE_SETTINGS);
        } else if ("background_location".equals(page)) {
            intent = new Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS,
                    Uri.parse("package:" + context.getPackageName()));
        } else if ("battery".equals(page)) {
            intent = new Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS,
                    Uri.parse("package:" + context.getPackageName()));
        } else {
            intent = new Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS);
        }
        intent.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
        try {
            context.startActivity(intent);
        } catch (Exception ignored) {
        }
    }

    // ---- clipboard (pairing codes copy verbatim, never reformatted) ----

    public static void copyText(Context context, String text) {
        try {
            android.content.ClipboardManager clipboard =
                    (android.content.ClipboardManager)
                            context.getSystemService(Context.CLIPBOARD_SERVICE);
            if (clipboard == null) {
                return;
            }
            clipboard.setPrimaryClip(android.content.ClipData.newPlainText(
                    "Harbor", text == null ? "" : text));
        } catch (Exception ignored) {
        }
    }

    // ---- Tailnet gate (Harbor pairs through Tailscale) ----

    /** True when the Tailscale client is installed (any login state). */
    public static boolean isTailscaleInstalled(Context context) {
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                context.getPackageManager().getPackageInfo(
                        "com.tailscale.ipn",
                        PackageManager.PackageInfoFlags.of(0));
            } else {
                context.getPackageManager().getPackageInfo("com.tailscale.ipn", 0);
            }
            return true;
        } catch (Exception ignored) {
            return false;
        }
    }

    /** Deep-link to the Tailscale store page (Play app, HTTPS fallback). */
    public static void openTailscaleStore(Context context) {
        try {
            Intent market = new Intent(Intent.ACTION_VIEW,
                    Uri.parse("market://details?id=com.tailscale.ipn"));
            market.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            context.startActivity(market);
            return;
        } catch (Exception ignored) {
        }
        try {
            Intent web = new Intent(Intent.ACTION_VIEW, Uri.parse(
                    "https://play.google.com/store/apps/details?id=com.tailscale.ipn"));
            web.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK);
            context.startActivity(web);
        } catch (Exception ignored) {
        }
    }

    // ---- usage access (special grant, no runtime dialog) ----

    /** 0 allowed, 1 denied/unknown-checked, -1 manager missing. */
    public static int usageAccessMode(Context context) {
        AppOpsManager ops = context.getSystemService(AppOpsManager.class);
        if (ops == null) {
            return -1;
        }
        int mode = ops.checkOpNoThrow(
                AppOpsManager.OPSTR_GET_USAGE_STATS, Process.myUid(), context.getPackageName());
        return mode == AppOpsManager.MODE_ALLOWED ? 0 : 1;
    }

    // ---- location permission + foreground service ----

    public static boolean hasLocationPermission(Context context) {
        return ContextCompat.checkSelfPermission(context, Manifest.permission.ACCESS_FINE_LOCATION)
                == PackageManager.PERMISSION_GRANTED
                || ContextCompat.checkSelfPermission(context, Manifest.permission.ACCESS_COARSE_LOCATION)
                == PackageManager.PERMISSION_GRANTED;
    }

    public static boolean hasMicrophonePermission(Context context) {
        return ContextCompat.checkSelfPermission(context, Manifest.permission.RECORD_AUDIO)
                == PackageManager.PERMISSION_GRANTED;
    }

    public static boolean hasPostNotificationsPermission(Context context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            return true;
        }
        return ContextCompat.checkSelfPermission(
                context, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED;
    }

    public static void requestMicrophonePermission(QtActivity activity) {
        ActivityCompat.requestPermissions(
                activity, new String[]{Manifest.permission.RECORD_AUDIO}, MICROPHONE_REQUEST);
    }

    /** POST_NOTIFICATIONS is unrelated to notification-listener access. */
    public static void requestPostNotifications(QtActivity activity) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            ActivityCompat.requestPermissions(
                    activity, new String[]{Manifest.permission.POST_NOTIFICATIONS}, 6106);
        }
    }

    public static boolean ignoresBatteryOptimizations(Context context) {
        PowerManager power = context.getSystemService(PowerManager.class);
        return power == null || Build.VERSION.SDK_INT < Build.VERSION_CODES.M
                || power.isIgnoringBatteryOptimizations(context.getPackageName());
    }

    @Override
    public void onRequestPermissionsResult(int requestCode, String[] permissions, int[] results) {
        super.onRequestPermissionsResult(requestCode, permissions, results);
        if (requestCode == LOCATION_REQUEST || requestCode == MICROPHONE_REQUEST
                || requestCode == 6106) {
            notifyNativePermissionChanged();
        }
    }

    @Override
    protected void onResume() {
        super.onResume();
        // Settings grants (usage access and notification listener) do not
        // have an Activity permission callback.  Refresh the Qt facade every
        // time the app becomes visible, including after process recreation.
        notifyNativePermissionChanged();
    }

    private static void notifyNativePermissionChanged() {
        // Qt registers this native method during application startup. The
        // Activity can resume once before that registration on a cold launch.
        try {
            nativePermissionChanged();
        } catch (UnsatisfiedLinkError ignored) {
        }
    }

    public static boolean hasBackgroundLocation(Context context) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
            return true;
        }
        return ContextCompat.checkSelfPermission(context, Manifest.permission.ACCESS_BACKGROUND_LOCATION)
                == PackageManager.PERMISSION_GRANTED;
    }

    public static void requestLocationPermission(QtActivity activity) {
        ActivityCompat.requestPermissions(
                activity,
                new String[]{
                        Manifest.permission.ACCESS_FINE_LOCATION,
                        Manifest.permission.ACCESS_COARSE_LOCATION},
                LOCATION_REQUEST);
    }

    public static void startLocation(Context context) {
        if (!hasLocationPermission(context)) {
            context.getSharedPreferences("harbor_mobile", Context.MODE_PRIVATE)
                    .edit().putBoolean("location_intent", false).apply();
            return;
        }
        context.getSharedPreferences("harbor_mobile", Context.MODE_PRIVATE)
                .edit().putBoolean("location_intent", true).apply();
        Intent intent = new Intent(context, HarborLocationService.class);
        intent.setAction(HarborLocationService.ACTION_START);
        ContextCompat.startForegroundService(context, intent);
    }

    public static void stopLocation(Context context) {
        context.getSharedPreferences("harbor_mobile", Context.MODE_PRIVATE)
                .edit().putBoolean("location_intent", false).apply();
        Intent intent = new Intent(context, HarborLocationService.class);
        intent.setAction(HarborLocationService.ACTION_STOP);
        context.startService(intent);
        locationActive = false;
        lastFix = null;
    }

    static void noteFix(Location fix) {
        lastFix = fix;
        locationActive = fix != null;
    }

    public static boolean isLocationActive() {
        return locationActive && lastFix != null;
    }

    public static double lastLatitude() {
        Location fix = lastFix;
        return fix == null ? 0.0 : fix.getLatitude();
    }

    public static double lastLongitude() {
        Location fix = lastFix;
        return fix == null ? 0.0 : fix.getLongitude();
    }

    /** Seconds since the cached fix, or -1 without one. */
    public static long lastFixAgeSeconds() {
        Location fix = lastFix;
        if (fix == null) {
            return -1;
        }
        return Math.max(0, (System.currentTimeMillis() - fix.getTime()) / 1000);
    }

    public static float lastAccuracyMeters() {
        Location fix = lastFix;
        return fix == null || !fix.hasAccuracy() ? -1.0f : fix.getAccuracy();
    }

    public static boolean hasFix() {
        return lastFix != null;
    }
}
