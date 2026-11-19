package dev.streampipe.app;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.graphics.Bitmap;
import android.graphics.drawable.Icon;
import android.media.MediaMetadata;
import android.media.session.MediaSession;
import android.media.session.PlaybackState;
import android.os.Build;
import android.os.IBinder;
import android.os.PowerManager;
import android.os.SystemClock;

/**
 * Keeps playback alive when the app is not on screen: a foreground service with a media
 * notification (title, play/pause, close) and a wake lock while sound is playing. The video
 * itself keeps running in the activity's web view; this service only tells Android the app
 * is playing media and relays the notification's buttons.
 */
public final class PlaybackService extends Service {
    /** Implemented by the activity. */
    interface Controller {
        void togglePlayback();

        void stopPlayback();
    }

    private static final String CHANNEL = "playback";
    private static final int NOTIFICATION = 1;
    private static final String ACTION_UPDATE = "dev.streampipe.app.UPDATE";
    private static final String ACTION_TOGGLE = "dev.streampipe.app.TOGGLE";
    private static final String ACTION_STOP = "dev.streampipe.app.STOP";

    static volatile Controller controller;
    private static volatile String title = "";
    private static volatile String artist = "";
    private static volatile boolean playing;
    private static volatile Bitmap art;
    private static volatile boolean running;

    private MediaSession session;
    private PowerManager.WakeLock wakeLock;

    /** Show or refresh the notification for what is playing. */
    static void update(Context c, String newTitle, String newArtist, boolean isPlaying, Bitmap artwork) {
        title = newTitle == null ? "" : newTitle;
        artist = newArtist == null ? "" : newArtist;
        playing = isPlaying;
        art = artwork;
        Intent i = new Intent(c, PlaybackService.class).setAction(ACTION_UPDATE);
        try {
            if (Build.VERSION.SDK_INT >= 26) {
                c.startForegroundService(i);
            } else {
                c.startService(i);
            }
        } catch (RuntimeException e) {
            // Android refuses to start it from the background; the next foreground update will.
        }
    }

    static void stop(Context c) {
        if (running) {
            c.stopService(new Intent(c, PlaybackService.class));
        }
    }

    @Override
    public void onCreate() {
        super.onCreate();
        running = true;
        if (Build.VERSION.SDK_INT >= 26) {
            NotificationChannel ch = new NotificationChannel(CHANNEL, "Playback", NotificationManager.IMPORTANCE_LOW);
            ch.setDescription("Shows what is playing, with play and pause");
            ch.setShowBadge(false);
            getSystemService(NotificationManager.class).createNotificationChannel(ch);
        }
        session = new MediaSession(this, "StreamPipe");
        session.setCallback(new MediaSession.Callback() {
            @Override
            public void onPlay() {
                toggle(true);
            }

            @Override
            public void onPause() {
                toggle(false);
            }

            @Override
            public void onStop() {
                Controller c = controller;
                if (c != null) {
                    c.stopPlayback();
                }
            }
        });
        session.setActive(true);
        PowerManager pm = (PowerManager) getSystemService(POWER_SERVICE);
        wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "StreamPipe:playback");
        wakeLock.setReferenceCounted(false);
    }

    /** Headset and lock-screen buttons say what they want; the page only has a toggle. */
    private void toggle(boolean wantPlaying) {
        Controller c = controller;
        if (c != null && playing != wantPlaying) {
            c.togglePlayback();
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        String action = intent == null ? null : intent.getAction();
        Controller c = controller;
        if (ACTION_TOGGLE.equals(action) && c != null) {
            c.togglePlayback();
        } else if (ACTION_STOP.equals(action)) {
            if (c != null) {
                c.stopPlayback();
            }
            // startForegroundService must be answered with startForeground, even when stopping.
            show();
            stopForeground(true);
            stopSelf();
            return START_NOT_STICKY;
        }
        show();
        return START_NOT_STICKY;
    }

    private PendingIntent service(String action) {
        Intent i = new Intent(this, PlaybackService.class).setAction(action);
        int flags = PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE;
        return Build.VERSION.SDK_INT >= 26 ? PendingIntent.getForegroundService(this, action.hashCode(), i, flags)
                : PendingIntent.getService(this, action.hashCode(), i, flags);
    }

    private void show() {
        session.setMetadata(new MediaMetadata.Builder()
                .putString(MediaMetadata.METADATA_KEY_TITLE, title)
                .putString(MediaMetadata.METADATA_KEY_ARTIST, artist)
                .putBitmap(MediaMetadata.METADATA_KEY_ALBUM_ART, art)
                .build());
        session.setPlaybackState(new PlaybackState.Builder()
                .setActions(PlaybackState.ACTION_PLAY | PlaybackState.ACTION_PAUSE | PlaybackState.ACTION_PLAY_PAUSE | PlaybackState.ACTION_STOP)
                .setState(playing ? PlaybackState.STATE_PLAYING : PlaybackState.STATE_PAUSED,
                        PlaybackState.PLAYBACK_POSITION_UNKNOWN, playing ? 1f : 0f, SystemClock.elapsedRealtime())
                .build());

        Intent open = new Intent(this, MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP);
        PendingIntent content = PendingIntent.getActivity(this, 0, open, PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        Notification.Builder b = Build.VERSION.SDK_INT >= 26 ? new Notification.Builder(this, CHANNEL) : new Notification.Builder(this);
        b.setSmallIcon(R.drawable.ic_play)
                .setContentTitle(title.isEmpty() ? "PlayTube" : title)
                .setContentText(artist)
                .setLargeIcon(art)
                .setContentIntent(content)
                .setVisibility(Notification.VISIBILITY_PUBLIC)
                .setOnlyAlertOnce(true)
                .setShowWhen(false)
                .setOngoing(playing)
                .addAction(new Notification.Action.Builder(
                        Icon.createWithResource(this, playing ? R.drawable.ic_pause : R.drawable.ic_play),
                        playing ? "Pause" : "Play", service(ACTION_TOGGLE)).build())
                .addAction(new Notification.Action.Builder(
                        Icon.createWithResource(this, R.drawable.ic_close), "Close", service(ACTION_STOP)).build())
                .setStyle(new Notification.MediaStyle().setMediaSession(session.getSessionToken()).setShowActionsInCompactView(0, 1));
        Notification n = b.build();
        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(NOTIFICATION, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK);
        } else {
            startForeground(NOTIFICATION, n);
        }
        // Keep the processor awake only while sound is actually playing.
        if (playing) {
            wakeLock.acquire(4 * 60 * 60 * 1000L);
        } else if (wakeLock.isHeld()) {
            wakeLock.release();
        }
    }

    @Override
    public void onTaskRemoved(Intent rootIntent) {
        // Swiped away from recents: the player is gone with the activity.
        stopForeground(true);
        stopSelf();
    }

    @Override
    public void onDestroy() {
        running = false;
        if (wakeLock.isHeld()) {
            wakeLock.release();
        }
        session.setActive(false);
        session.release();
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
