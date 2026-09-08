package org.harbor.mobile;

import android.Manifest;
import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.media.AudioAttributes;
import android.media.AudioFocusRequest;
import android.media.AudioManager;
import android.os.Build;
import android.os.IBinder;

import androidx.core.content.ContextCompat;

/**
 * Owns the Android foreground-service lifetime for a live Harbor call. The
 * Go worker owns the miniaudio/OpenSL ES capture and playback devices; this
 * service keeps the process alive and makes microphone use visible to the user
 * while the call is backgrounded.
 */
public final class HarborCallService extends Service {
    private static final String CHANNEL_ID = "harbor_call";
    private static final int NOTIFICATION_ID = 6105;
    public static final String ACTION_START = "org.harbor.mobile.CALL_START";
    public static final String ACTION_STOP = "org.harbor.mobile.CALL_STOP";

    private AudioManager audioManager;
    private AudioFocusRequest audioFocusRequest;
    private final AudioManager.OnAudioFocusChangeListener audioFocusListener =
            focusChange -> { };

    @Override
    public void onCreate() {
        super.onCreate();
        NotificationManager manager = getSystemService(NotificationManager.class);
        if (manager != null) {
            manager.createNotificationChannel(new NotificationChannel(
                    CHANNEL_ID, "Harbor calls", NotificationManager.IMPORTANCE_LOW));
        }
        audioManager = getSystemService(AudioManager.class);
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent != null && ACTION_STOP.equals(intent.getAction())) {
            abandonAudioFocus();
            stopForeground(STOP_FOREGROUND_REMOVE);
            stopSelf();
            return START_NOT_STICKY;
        }
        if (!HarborMobileActivity.hasMicrophonePermission(this)) {
            stopSelf();
            return START_NOT_STICKY;
        }
        // Audio focus is a courtesy to other apps; a focus denial must not
        // leave the call without its required foreground-service lifetime.
        requestAudioFocus();
        Notification notification = new Notification.Builder(this, CHANNEL_ID)
                .setSmallIcon(android.R.drawable.ic_btn_speak_now)
                .setContentTitle("Harbor call")
                .setContentText("Microphone is in use — Harbor joins calls muted")
                .setOngoing(true)
                .setShowWhen(false)
                .build();
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(NOTIFICATION_ID, notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
                            | ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK);
        } else {
            startForeground(NOTIFICATION_ID, notification);
        }
        return START_NOT_STICKY;
    }

    public static void start(Context context) {
        if (!HarborMobileActivity.hasMicrophonePermission(context)) {
            return;
        }
        Intent intent = new Intent(context, HarborCallService.class);
        intent.setAction(ACTION_START);
        ContextCompat.startForegroundService(context, intent);
    }

    public static void stop(Context context) {
        Intent intent = new Intent(context, HarborCallService.class);
        intent.setAction(ACTION_STOP);
        context.startService(intent);
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }

    @SuppressWarnings("deprecation")
    private void requestAudioFocus() {
        if (audioManager == null) {
            return;
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            if (audioFocusRequest == null) {
                AudioAttributes attributes = new AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
                        .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
                        .build();
                audioFocusRequest = new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                        .setAudioAttributes(attributes)
                        .setAcceptsDelayedFocusGain(false)
                        .setOnAudioFocusChangeListener(audioFocusListener)
                        .build();
            }
            audioManager.requestAudioFocus(audioFocusRequest);
            return;
        }
        audioManager.requestAudioFocus(audioFocusListener, AudioManager.STREAM_VOICE_CALL,
                AudioManager.AUDIOFOCUS_GAIN);
        // Android 9+ takes the API 26 path above; keep the legacy branch for
        // completeness if the minimum SDK is ever lowered.
    }

    private void abandonAudioFocus() {
        if (audioManager == null) {
            return;
        }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && audioFocusRequest != null) {
            audioManager.abandonAudioFocusRequest(audioFocusRequest);
            return;
        }
        @SuppressWarnings("deprecation")
        int ignored = audioManager.abandonAudioFocus(audioFocusListener);
    }

    @Override
    public void onDestroy() {
        abandonAudioFocus();
        super.onDestroy();
    }
}
