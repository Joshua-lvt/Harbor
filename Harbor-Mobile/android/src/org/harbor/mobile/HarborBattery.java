package org.harbor.mobile;

import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.os.BatteryManager;

/** Battery facts, no permission needed. Absent readings stay absent. */
public final class HarborBattery {
    private HarborBattery() {}

    /** 0..100, or -1 when the platform answers nothing usable. */
    public static int capacityPercent(Context context) {
        BatteryManager manager = context.getSystemService(BatteryManager.class);
        if (manager == null) {
            return -1;
        }
        int percent = manager.getIntProperty(BatteryManager.BATTERY_PROPERTY_CAPACITY);
        return (percent >= 0 && percent <= 100) ? percent : -1;
    }

    public static boolean charging(Context context) {
        Intent sticky = context.registerReceiver(
                null, new IntentFilter(Intent.ACTION_BATTERY_CHANGED));
        if (sticky == null) {
            return false;
        }
        int status = sticky.getIntExtra(BatteryManager.EXTRA_STATUS, -1);
        return status == BatteryManager.BATTERY_STATUS_CHARGING
                || status == BatteryManager.BATTERY_STATUS_FULL;
    }
}
