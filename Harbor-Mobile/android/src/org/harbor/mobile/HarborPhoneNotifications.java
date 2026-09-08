package org.harbor.mobile;

import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ApplicationInfo;
import android.os.Bundle;
import android.provider.Settings;
import android.service.notification.NotificationListenerService;
import android.service.notification.StatusBarNotification;
import android.text.TextUtils;

/**
 * Mirrors phone notifications to the Harbor PC while the user keeps
 * "Share phone notifications" ON. Display-only: nothing is persisted,
 * system/ongoing notifications are skipped, and revoking access stops
 * the flow immediately.
 */
public class HarborPhoneNotifications extends NotificationListenerService {

    private static volatile boolean sharingEnabled;

    public static void setSharingEnabled(boolean enabled) {
        sharingEnabled = enabled;
    }

    public static boolean hasAccess(Context context) {
        // Framework-only check: the enabled_listeners secure setting lists
        // flattened component names granted by the user.
        String flat = Settings.Secure.getString(
                context.getContentResolver(), "enabled_notification_listeners");
        if (TextUtils.isEmpty(flat)) {
            return false;
        }
        String own = componentName(context).flattenToString();
        for (String entry : flat.split(":")) {
            if (own.equals(entry)
                    || ComponentName.unflattenFromString(entry) != null
                            && own.equals(ComponentName.unflattenFromString(entry)
                                    .flattenToString())) {
                return true;
            }
        }
        return false;
    }

    public static Intent accessSettingsIntent() {
        return new Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS);
    }

    public static ComponentName componentName(Context context) {
        return new ComponentName(context, HarborPhoneNotifications.class);
    }

    @Override
    public void onNotificationPosted(StatusBarNotification notification) {
        if (!sharingEnabled || notification == null
                || !hasAccess(this)) {
            return;
        }
        if (notification.isOngoing()) {
            return;
        }
        android.app.Notification inner = notification.getNotification();
        if (inner == null) {
            return;
        }
        String packageName = notification.getPackageName();
        if (isSystemPackage(packageName)) {
            return;
        }
        Bundle extras = inner.extras;
        if (extras == null) {
            return;
        }
        if (android.app.Notification.CATEGORY_SYSTEM.equals(inner.category)
                || android.app.Notification.CATEGORY_TRANSPORT.equals(inner.category)
                || android.app.Notification.CATEGORY_CALL.equals(inner.category)
                || android.app.Notification.CATEGORY_NAVIGATION.equals(inner.category)) {
            return;
        }
        // Some media apps do not set CATEGORY_TRANSPORT, but still expose a
        // MediaSession token. Treat that as media as well; this is a
        // display-only mirror, not a general notification archive.
        if (extras.containsKey(android.app.Notification.EXTRA_MEDIA_SESSION)) {
            return;
        }
        CharSequence title = extras.getCharSequence(android.app.Notification.EXTRA_TITLE);
        CharSequence text = extras.getCharSequence(android.app.Notification.EXTRA_TEXT);
        String titleText = title == null ? "" : title.toString().trim();
        String bodyText = text == null ? "" : text.toString().trim();
        if (titleText.isEmpty() && bodyText.isEmpty()) {
            return;
        }
        // Never forward likely OTP/verification secrets, even though this
        // surface is display-only. The listener remains intentionally
        // conservative; users can still mirror ordinary notification text.
        String sensitive = (titleText + " " + bodyText).toLowerCase(java.util.Locale.ROOT);
        if (sensitive.matches(".*(otp|one[- ]time|verification|passcode|security code|auth code|\\bcode\\b).*")) {
            return;
        }
        String app = HarborUsage.appLabel(this, packageName);
        if (app == null || app.trim().isEmpty()) {
            app = packageName == null ? "" : packageName;
        }
        if (app.equals(getPackageName())) {
            return;
        }
        try {
            HarborMobileActivity.nativePhoneNotification(app, titleText, bodyText,
                    notification.getPostTime());
        } catch (UnsatisfiedLinkError ignored) {
            // Qt may not have registered the bridge during a very early
            // listener callback; dropping that one display-only event is
            // safer than taking down the listener process.
        }
    }

    private boolean isSystemPackage(String packageName) {
        if (packageName == null || packageName.isEmpty()
                || "android".equals(packageName)
                || "com.android.systemui".equals(packageName)) {
            return true;
        }
        try {
            ApplicationInfo info = getPackageManager().getApplicationInfo(packageName, 0);
            return (info.flags & (ApplicationInfo.FLAG_SYSTEM
                    | ApplicationInfo.FLAG_UPDATED_SYSTEM_APP)) != 0;
        } catch (Exception ignored) {
            // An unresolvable package is not safe to mirror with a raw id.
            return true;
        }
    }

    @Override
    public void onListenerDisconnected() {
        // A revoked listener must stop at the source. The user's intent is
        // retained in core settings, but no further notification is read or
        // forwarded until access is granted again.
        sharingEnabled = false;
        // Refresh the effective toggle immediately when Android tears down
        // the listener. This is best-effort because the service can be
        // restarted after the Qt process has already been reclaimed.
        try {
            HarborMobileActivity.nativePermissionChanged();
        } catch (UnsatisfiedLinkError ignored) {
        }
        super.onListenerDisconnected();
    }
}
