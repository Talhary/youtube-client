package dev.streampipe.app;

import android.content.Context;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.os.Handler;
import android.os.Looper;
import android.util.LruCache;
import android.widget.ImageView;

import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.security.MessageDigest;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/** Thumbnails: memory cache, disk cache, then the network. Images fade in when they arrive. */
final class Images {
    /** Pictures are decoded down to about the screen's width (set in init). */
    private static int MAX_SIDE = 720;
    private static final long DISK_LIMIT = 60L * 1024 * 1024;

    private static final Handler MAIN = new Handler(Looper.getMainLooper());
    private static final ExecutorService POOL = Executors.newFixedThreadPool(4);
    private static LruCache<String, Bitmap> memory;
    private static File dir;

    private Images() {}

    static void init(Context c) {
        if (memory != null) {
            return;
        }
        android.util.DisplayMetrics m = c.getResources().getDisplayMetrics();
        MAX_SIDE = Math.max(720, Math.min(1280, Math.max(m.widthPixels, m.heightPixels) / 2 + Math.min(m.widthPixels, m.heightPixels) / 2));
        int kb = (int) (Runtime.getRuntime().maxMemory() / 1024 / 5);
        memory = new LruCache<String, Bitmap>(kb) {
            @Override
            protected int sizeOf(String key, Bitmap b) {
                return b.getByteCount() / 1024;
            }
        };
        dir = new File(c.getCacheDir(), "img");
        //noinspection ResultOfMethodCallIgnored
        dir.mkdirs();
        POOL.execute(Images::trimDisk);
    }

    static Bitmap cached(String url) {
        return url == null || memory == null ? null : memory.get(url);
    }

    /** Show `url` in `view`. A view reused for another image ignores the stale result. */
    static void load(final ImageView view, final String url) {
        view.setTag(url);
        if (url == null) {
            view.setImageDrawable(null);
            return;
        }
        Bitmap hit = memory.get(url);
        if (hit != null) {
            view.animate().cancel();
            view.setAlpha(1f);
            view.setImageBitmap(hit);
            return;
        }
        view.setImageDrawable(null);
        POOL.execute(() -> {
            final Bitmap b = fetch(url);
            if (b == null) {
                return;
            }
            memory.put(url, b);
            MAIN.post(() -> {
                if (url.equals(view.getTag())) {
                    view.setImageBitmap(b);
                    view.setAlpha(0f);
                    view.animate().alpha(1f).setDuration(Ui.FAST).start();
                }
            });
        });
    }

    private static final java.util.regex.Pattern VIDEO_THUMB =
            java.util.regex.Pattern.compile("^https?://i\\d?\\.ytimg\\.com/(?:vi|vi_webp)/([A-Za-z0-9_-]{11})/");

    /** A sharp version of a video thumbnail: 1280x720 ("hq720"), or null if `url` is not one. */
    static String sharp(String url) {
        if (url == null) {
            return null;
        }
        java.util.regex.Matcher m = VIDEO_THUMB.matcher(url);
        return m.find() && !url.contains("hq720") && !url.contains("maxres") && !url.contains("oar")
                ? "https://i.ytimg.com/vi/" + m.group(1) + "/hq720.jpg" : null;
    }

    /**
     * Two passes, as YouTube does: the small picture first (fast, often cached), then the
     * sharp one replaces it when it arrives. Falls back to "sddefault" for videos without hq720.
     */
    static void loadSharp(final ImageView view, final String url) {
        final String hi = sharp(url);
        if (hi == null) {
            load(view, url);
            return;
        }
        Bitmap ready = memory.get(hi);
        if (ready != null) {
            view.setTag(url);
            view.animate().cancel();
            view.setAlpha(1f);
            view.setImageBitmap(ready);
            return;
        }
        load(view, url);
        POOL.execute(() -> {
            Bitmap b = fetch(hi);
            String used = hi;
            if (b == null) {
                used = hi.replace("hq720.jpg", "sddefault.jpg");
                b = fetch(used);
            }
            if (b == null) {
                return;
            }
            final Bitmap got = b;
            memory.put(hi, got);
            MAIN.post(() -> {
                if (url.equals(view.getTag())) {
                    view.animate().cancel();
                    view.setAlpha(1f);
                    view.setImageBitmap(got);
                }
            });
        });
    }

    interface Got {
        void got(Bitmap b);
    }

    /** The picture at `url` itself (on the main thread; null if it failed). */
    static void bitmap(final String url, final Got got) {
        Bitmap hit = cached(url);
        if (hit != null) {
            got.got(hit);
            return;
        }
        POOL.execute(() -> {
            final Bitmap b = fetch(url);
            if (b != null) {
                memory.put(url, b);
            }
            MAIN.post(() -> got.got(b));
        });
    }

    private static Bitmap fetch(String url) {
        File f = new File(dir, name(url));
        try {
            byte[] bytes;
            if (f.isFile() && f.length() > 0) {
                bytes = readAll(new FileInputStream(f));
                //noinspection ResultOfMethodCallIgnored
                f.setLastModified(System.currentTimeMillis());
            } else {
                HttpURLConnection c = open(url);
                try {
                    // Picture hosts redirect now and then (also between http and https,
                    // which HttpURLConnection does not follow by itself).
                    for (int hops = 0; hops < 4; hops++) {
                        int code = c.getResponseCode();
                        String next = code >= 300 && code < 400 ? c.getHeaderField("Location") : null;
                        if (next == null) {
                            break;
                        }
                        c.disconnect();
                        c = open(new URL(new URL(url), next).toString());
                    }
                    int code = c.getResponseCode();
                    if (code < 200 || code >= 300) {
                        // Not every Short has a portrait cover ("oardefault"): use the regular one.
                        if (code == 404 && url.contains("/oardefault.jpg")) {
                            c.disconnect();
                            return fetch(url.replace("/oardefault.jpg", "/hqdefault.jpg"));
                        }
                        android.util.Log.w("StreamPipe", "image " + code + ": " + url);
                        return null;
                    }
                    bytes = readAll(c.getInputStream());
                } finally {
                    c.disconnect();
                }
                try (FileOutputStream out = new FileOutputStream(f)) {
                    out.write(bytes);
                }
            }
            BitmapFactory.Options o = new BitmapFactory.Options();
            o.inJustDecodeBounds = true;
            BitmapFactory.decodeByteArray(bytes, 0, bytes.length, o);
            int sample = 1;
            while (Math.max(o.outWidth, o.outHeight) / (sample * 2) >= MAX_SIDE) {
                sample *= 2;
            }
            o = new BitmapFactory.Options();
            o.inSampleSize = sample;
            // Full colour: 16-bit pictures show bands in skies and dark gradients.
            o.inPreferredConfig = Bitmap.Config.ARGB_8888;
            return BitmapFactory.decodeByteArray(bytes, 0, bytes.length, o);
        } catch (Exception | OutOfMemoryError e) {
            return null;
        }
    }

    private static HttpURLConnection open(String url) throws java.io.IOException {
        HttpURLConnection c = (HttpURLConnection) new URL(url).openConnection();
        c.setConnectTimeout(10000);
        c.setReadTimeout(15000);
        c.setInstanceFollowRedirects(true);
        // Some picture hosts answer an unknown client differently from a browser.
        c.setRequestProperty("User-Agent", "Mozilla/5.0 (Linux; Android 10) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Mobile Safari/537.36");
        return c;
    }

    private static byte[] readAll(InputStream in) throws java.io.IOException {
        try (InputStream i = in) {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            byte[] buf = new byte[16384];
            int n;
            while ((n = i.read(buf)) > 0) {
                out.write(buf, 0, n);
            }
            return out.toByteArray();
        }
    }

    private static String name(String url) {
        try {
            byte[] d = MessageDigest.getInstance("SHA-1").digest(url.getBytes("UTF-8"));
            StringBuilder sb = new StringBuilder();
            for (byte b : d) {
                sb.append(String.format(java.util.Locale.ROOT, "%02x", b));
            }
            return sb.toString();
        } catch (Exception e) {
            return String.valueOf(url.hashCode());
        }
    }

    /** Keep the disk cache under its limit by deleting the least recently used files. */
    private static void trimDisk() {
        File[] files = dir.listFiles();
        if (files == null) {
            return;
        }
        long total = 0;
        for (File f : files) {
            total += f.length();
        }
        if (total <= DISK_LIMIT) {
            return;
        }
        java.util.Arrays.sort(files, (a, b) -> Long.compare(a.lastModified(), b.lastModified()));
        for (File f : files) {
            if (total <= DISK_LIMIT * 3 / 4) {
                break;
            }
            total -= f.length();
            //noinspection ResultOfMethodCallIgnored
            f.delete();
        }
    }
}
