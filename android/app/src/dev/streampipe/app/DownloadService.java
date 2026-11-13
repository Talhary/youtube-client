package dev.streampipe.app;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Build;
import android.os.IBinder;
import android.os.PowerManager;
import android.os.SystemClock;

import java.util.Locale;

/**
 * Runs while something is downloading: a foreground service with a progress notification,
 * so Android keeps the app alive in the background, and a wake lock so the phone does not
 * sleep in the middle of a file. It stops itself when nothing is left to download.
 */
public final class DownloadService extends Service implements Downloads.Listener {
    private static final String CHANNEL = "downloads";
    private static final int PROGRESS = 2;
    private static final int FINISHED_BASE = 1000;

    private static volatile boolean running;
    private PowerManager.WakeLock wakeLock;
    private long lastShown;

    /** Start the service if a download is active and it is not running yet. */
    static void ensure(Context c) {
        if (running || Downloads.get().activeCount() == 0) {
            return;
        }
        Intent i = new Intent(c, DownloadService.class);
        try {
            if (Build.VERSION.SDK_INT >= 26) {
                c.startForegroundService(i);
            } else {
                c.startService(i);
            }
        } catch (RuntimeException e) {
            // Android refuses a start from the background; the download still runs while the app does.
        }
    }

    private static void channel(Context c) {
        if (Build.VERSION.SDK_INT >= 26) {
            NotificationChannel ch = new NotificationChannel(CHANNEL, "Downloads", NotificationManager.IMPORTANCE_LOW);
            ch.setDescription("Download progress and finished downloads");
            ch.setShowBadge(false);
            c.getSystemService(NotificationManager.class).createNotificationChannel(ch);
        }
    }

    private static Notification.Builder builder(Context c) {
        Notification.Builder b = Build.VERSION.SDK_INT >= 26 ? new Notification.Builder(c, CHANNEL) : new Notification.Builder(c);
        Intent open = new Intent(c, MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP);
        b.setContentIntent(PendingIntent.getActivity(c, 0, open, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT));
        b.setSmallIcon(R.drawable.ic_download);
        return b;
    }

    /** A lasting note that one download is done. */
    static void finished(Context c, Downloads.Job j) {
        try {
            channel(c);
            Notification n = builder(c)
                    .setContentTitle("Download finished")
                    .setContentText(j.title.isEmpty() ? j.videoId : j.title)
                    .setAutoCancel(true)
                    .build();
            c.getSystemService(NotificationManager.class).notify(FINISHED_BASE + (int) (j.id % 100000), n);
        } catch (RuntimeException ignored) {
            // Notifications are switched off for the app.
        }
    }

    private static String size(long bytes) {
        return String.format(Locale.US, "%.1f MB", bytes / (double) (1 << 20));
    }

    private Notification progress() {
        long done = 0, total = 0, speed = 0;
        int active = 0;
        String title = "";
        boolean joining = false;
        for (Downloads.Job j : Downloads.get().list()) {
            if (!j.active()) {
                continue;
            }
            active++;
            title = j.title.isEmpty() ? j.videoId : j.title;
            done += j.done;
            total += j.total;
            speed += j.state.equals("Downloading") ? j.speed : 0;
            joining |= j.state.equals("Muxing");
        }
        int percent = total > 0 ? (int) (100 * done / total) : 0;
        String text = total > 0
                ? percent + "% · " + size(done) + " of " + size(total) + (speed > 0 ? " · " + size(speed) + "/s" : "")
                : joining ? "Joining picture and sound" : "Starting";
        return builder(this)
                .setContentTitle(active > 1 ? "Downloading " + active + " videos" : title.isEmpty() ? "Downloading" : title)
                .setContentText(text)
                .setProgress(100, percent, total <= 0)
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .build();
    }

    @Override
    public void onCreate() {
        super.onCreate();
        running = true;
        channel(this);
        PowerManager pm = (PowerManager) getSystemService(POWER_SERVICE);
        wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "StreamPipe:downloads");
        wakeLock.setReferenceCounted(false);
        wakeLock.acquire(6 * 60 * 60 * 1000L);
        Downloads.get().addListener(this);
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        Notification n = progress();
        if (Build.VERSION.SDK_INT >= 29) {
            startForeground(PROGRESS, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC);
        } else {
            startForeground(PROGRESS, n);
        }
        if (Downloads.get().activeCount() == 0) {
            stopSelf();
        }
        return START_NOT_STICKY;
    }

    @Override
    public void onDownloads() {
        if (Downloads.get().activeCount() == 0) {
            stopForeground(true);
            stopSelf();
            return;
        }
        // Progress arrives twice a second per download; once a second is enough to show.
        long now = SystemClock.elapsedRealtime();
        if (now - lastShown >= 900) {
            lastShown = now;
            getSystemService(NotificationManager.class).notify(PROGRESS, progress());
        }
    }

    @Override
    public void onDestroy() {
        running = false;
        Downloads.get().removeListener(this);
        if (wakeLock.isHeld()) {
            wakeLock.release();
        }
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
