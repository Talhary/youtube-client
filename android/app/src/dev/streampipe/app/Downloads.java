package dev.streampipe.app;

import android.content.ContentValues;
import android.content.Context;
import android.content.Intent;
import android.content.SharedPreferences;
import android.net.Uri;
import android.os.Build;
import android.os.Handler;
import android.os.Looper;
import android.provider.MediaStore;
import android.util.Log;
import android.view.Gravity;
import android.view.View;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ProgressBar;
import android.widget.ScrollView;
import android.widget.TextView;
import android.widget.Toast;

import org.json.JSONObject;

import java.io.File;
import java.io.FileInputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.TreeMap;

/**
 * The app's downloads. The core fetches the streams (in parallel pieces, resumable) and joins
 * them; this class keeps the list for the UI and moves each finished file into the phone's
 * shared Download/YouTube folder.
 */
final class Downloads {
    private static final String TAG = "StreamPipe";
    static final String FOLDER = "Download/YouTube";

    static final class Job {
        long id;
        String videoId = "";
        String title = "";
        /** 0 = sound only. */
        int height;
        /** The YouTube playlist it was downloaded with, or null. */
        String group;
        /** Queued, Resolving, Downloading, Muxing, Paused, Done, Failed or Cancelled. */
        String state = "Queued";
        /** The file's path (Done) or what went wrong (Failed). */
        String detail = "";
        long done, total, speed;
        /** Where the finished file is in the shared folder. */
        String uri;
        boolean publishing;

        boolean active() {
            return state.equals("Queued") || state.equals("Resolving") || state.equals("Downloading") || state.equals("Muxing");
        }
    }

    interface Listener {
        void onDownloads();
    }

    private static final Downloads INSTANCE = new Downloads();

    static Downloads get() {
        return INSTANCE;
    }

    private final Handler main = new Handler(Looper.getMainLooper());
    /** Newest first. */
    private final TreeMap<Long, Job> jobs = new TreeMap<>(java.util.Collections.reverseOrder());
    private final List<Listener> listeners = new ArrayList<>();
    private Context app;

    private Downloads() {}

    void init(Context c) {
        app = c.getApplicationContext();
    }

    private SharedPreferences prefs() {
        return app.getSharedPreferences("downloads", Context.MODE_PRIVATE);
    }

    List<Job> list() {
        return new ArrayList<>(jobs.values());
    }

    int activeCount() {
        int n = 0;
        for (Job j : jobs.values()) {
            if (j.active()) {
                n++;
            }
        }
        return n;
    }

    void addListener(Listener l) {
        listeners.add(l);
    }

    void removeListener(Listener l) {
        listeners.remove(l);
    }

    private void changed() {
        for (Listener l : new ArrayList<>(listeners)) {
            l.onDownloads();
        }
    }

    // ------------------------------------------------------------------ commands

    /** Ask what to download: sound only, or the picture up to a chosen quality. */
    void choose(Context c, final Core.Item video) {
        final int[] heights = {0, 360, 480, 720, 1080, 1440, 2160};
        String[] labels = {"Audio only (m4a)", "Video 360p", "Video 480p", "Video 720p", "Video 1080p", "Video 1440p", "Video 2160p (4K)"};
        new android.app.AlertDialog.Builder(c)
                .setTitle("Download")
                .setItems(labels, (d, which) -> start(video, heights[which]))
                .setNegativeButton("Cancel", null)
                .show();
    }

    /** Ask how to download every video of a playlist, then queue them all. */
    void choosePlaylist(Context c, final String playlistId, String name) {
        final int[] heights = {0, 360, 480, 720, 1080, 1440, 2160};
        String[] labels = {"Audio only (m4a)", "Video 360p", "Video 480p", "Video 720p", "Video 1080p", "Video 1440p", "Video 2160p (4K)"};
        new android.app.AlertDialog.Builder(c)
                .setTitle("Download all of " + (name == null || name.isEmpty() ? "this playlist" : name))
                .setItems(labels, (d, which) -> {
                    Core.get().send("download_playlist", "id", playlistId, "height", heights[which], "name", name == null ? "" : name);
                    Toast.makeText(app, "Reading the playlist. Its videos will appear in Settings, Downloads.", Toast.LENGTH_LONG).show();
                })
                .setNegativeButton("Cancel", null)
                .show();
    }

    /** Remove a download from the list; a finished one also loses its file. */
    void delete(final Job j) {
        final String uri = j.uri;
        final String path = j.detail;
        if (j.state.equals("Done")) {
            new Thread(() -> {
                try {
                    if (uri != null && uri.startsWith("content:")) {
                        app.getContentResolver().delete(Uri.parse(uri), null, null);
                    }
                    new File(path).delete();
                } catch (Exception e) {
                    Log.w(TAG, "downloads: deleting " + uri + ": " + e);
                }
            }, "streampipe-delete").start();
        }
        // The core forgets a cancelled download, whatever state it was in.
        Core.get().send("cancel_download", "id", j.id);
    }

    /** `height` 0 = sound only; otherwise the best picture up to that height. */
    void start(Core.Item video, int height) {
        Core.get().send("download", "id", video.id, "title", video.title, "height", height);
        Toast.makeText(app, "Download started. See Settings, Downloads.", Toast.LENGTH_SHORT).show();
    }

    void pause(Job j) {
        Core.get().send("pause_download", "id", j.id);
    }

    void resume(Job j) {
        Core.get().send("resume_download", "id", j.id);
    }

    void cancel(Job j) {
        Core.get().send("cancel_download", "id", j.id);
    }

    void clearFinished() {
        Core.get().send("clear_downloads");
    }

    // ------------------------------------------------------------------ events from the core

    /** One download event, on the main thread. */
    void onEvent(JSONObject e) {
        if (e == null || app == null) {
            return;
        }
        JSONObject o;
        if ((o = e.optJSONObject("Added")) != null) {
            Job j = new Job();
            j.id = o.optLong("id");
            JSONObject req = o.optJSONObject("req");
            if (req != null) {
                j.videoId = req.optString("video_id");
                j.title = req.optString("title");
                JSONObject kind = req.optJSONObject("kind");
                JSONObject video = kind == null ? null : kind.optJSONObject("Video");
                j.height = video == null ? 0 : video.optInt("max_height", 0);
                j.group = req.isNull("group") || req.optString("group").isEmpty() ? null : req.optString("group");
            }
            jobs.put(j.id, j);
            status(j, o.opt("status"));
            DownloadService.ensure(app);
        } else if ((o = e.optJSONObject("Status")) != null) {
            Job j = jobs.get(o.optLong("id"));
            if (j != null) {
                status(j, o.opt("status"));
                if (j.state.equals("Cancelled")) {
                    // Cancelled or deleted: it leaves the list.
                    jobs.remove(j.id);
                    prefs().edit().remove("uri_" + j.id).apply();
                }
                DownloadService.ensure(app);
            }
        } else if ((o = e.optJSONObject("Progress")) != null) {
            Job j = jobs.get(o.optLong("id"));
            if (j != null) {
                j.done = o.optLong("done");
                j.total = o.optLong("total");
                j.speed = o.optLong("bytes_per_s");
            }
        } else if ((o = e.optJSONObject("Removed")) != null) {
            long id = o.optLong("id");
            jobs.remove(id);
            prefs().edit().remove("uri_" + id).apply();
        }
        changed();
    }

    private void status(Job j, Object status) {
        if (status instanceof String) {
            j.state = (String) status;
            j.detail = "";
        } else if (status instanceof JSONObject) {
            JSONObject s = (JSONObject) status;
            if (s.has("Done")) {
                j.state = "Done";
                j.detail = s.optString("Done");
                j.uri = prefs().getString("uri_" + j.id, null);
                if (j.uri == null) {
                    publish(j);
                }
            } else if (s.has("Failed")) {
                j.state = "Failed";
                j.detail = s.optString("Failed");
            }
        }
    }

    // ------------------------------------------------------------------ the shared folder

    private static String mime(String name) {
        String n = name.toLowerCase(Locale.ROOT);
        return n.endsWith(".mp4") ? "video/mp4" : n.endsWith(".webm") ? "video/webm" : n.endsWith(".m4a") ? "audio/mp4"
                : n.endsWith(".opus") ? "audio/ogg" : "application/octet-stream";
    }

    /** Move a finished file from the app's own storage into Download/YouTube. */
    private void publish(final Job j) {
        final File file = new File(j.detail);
        if (j.publishing || !file.isFile()) {
            return;
        }
        if (Build.VERSION.SDK_INT < 29) {
            // Older Android: the file stays in the app's folder, which file managers can open.
            j.uri = Uri.fromFile(file).toString();
            return;
        }
        j.publishing = true;
        new Thread(() -> {
            String result = null;
            Uri target = null;
            try {
                ContentValues v = new ContentValues();
                v.put(MediaStore.MediaColumns.DISPLAY_NAME, file.getName());
                v.put(MediaStore.MediaColumns.MIME_TYPE, mime(file.getName()));
                v.put(MediaStore.MediaColumns.RELATIVE_PATH, FOLDER);
                v.put(MediaStore.MediaColumns.IS_PENDING, 1);
                target = app.getContentResolver().insert(MediaStore.Downloads.EXTERNAL_CONTENT_URI, v);
                if (target == null) {
                    throw new java.io.IOException("the Download folder refused the file");
                }
                try (InputStream in = new FileInputStream(file);
                     OutputStream out = app.getContentResolver().openOutputStream(target)) {
                    if (out == null) {
                        throw new java.io.IOException("the Download folder refused the file");
                    }
                    byte[] buf = new byte[1 << 16];
                    int n;
                    while ((n = in.read(buf)) > 0) {
                        out.write(buf, 0, n);
                    }
                }
                ContentValues ready = new ContentValues();
                ready.put(MediaStore.MediaColumns.IS_PENDING, 0);
                app.getContentResolver().update(target, ready, null, null);
                result = target.toString();
                if (!file.delete()) {
                    Log.w(TAG, "downloads: could not remove " + file);
                }
            } catch (Exception e) {
                Log.w(TAG, "downloads: moving " + file + " to " + FOLDER + ": " + e);
                if (target != null) {
                    try {
                        app.getContentResolver().delete(target, null, null);
                    } catch (Exception ignored) {
                    }
                }
            }
            final String uri = result;
            main.post(() -> {
                j.publishing = false;
                // If the move failed the file is still in the app's folder.
                j.uri = uri != null ? uri : Uri.fromFile(file).toString();
                if (uri != null) {
                    prefs().edit().putString("uri_" + j.id, uri).apply();
                    Toast.makeText(app, "Saved to " + FOLDER + ": " + j.title, Toast.LENGTH_SHORT).show();
                }
                DownloadService.finished(app, j);
                changed();
            });
        }, "streampipe-publish").start();
    }

    /** The best finished download of a video (the tallest picture; sound only if nothing else), or null. */
    Job finished(String videoId) {
        Job best = null;
        for (Job j : jobs.values()) {
            if (j.state.equals("Done") && j.uri != null && j.videoId.equals(videoId) && (best == null || j.height > best.height)) {
                best = j;
            }
        }
        return best;
    }

    /** The other downloaded videos, newest first, as a list that plays from their files. */
    Core.Page others(String exceptVideoId) {
        Core.Page page = new Core.Page();
        java.util.Set<String> seen = new java.util.HashSet<>();
        seen.add(exceptVideoId);
        for (Job j : jobs.values()) {
            if (!j.state.equals("Done") || j.uri == null || !seen.add(j.videoId)) {
                continue;
            }
            Job best = finished(j.videoId);
            Core.Item item = new Core.Item();
            item.type = Core.Item.VIDEO;
            item.id = best.videoId;
            item.title = best.title;
            item.channelName = best.height == 0 ? "Downloaded audio" : "Downloaded video";
            item.thumb = "https://i.ytimg.com/vi/" + best.videoId + "/mqdefault.jpg";
            item.localUri = best.uri;
            item.localMime = mime(best.detail);
            item.localAudio = best.height == 0;
            page.items.add(item);
        }
        return page;
    }

    // ------------------------------------------------------------------ the play queue

    /** What plays one after another: set when the user plays a list, a playlist or a selection. */
    private List<Core.Item> queue = new ArrayList<>();
    /** 0 = stop at the end, 1 = start the queue again, 2 = play the same one again. */
    int repeat;

    /** Shuffle what comes after `videoId` in the queue. False if it is not playing from a queue. */
    boolean shuffleRest(String videoId) {
        int at = videoId == null ? -1 : queueIndex(videoId);
        if (at < 0 || at + 2 > queue.size()) {
            return false;
        }
        java.util.Collections.shuffle(queue.subList(at + 1, queue.size()));
        changed();
        return true;
    }

    /** The first item of the queue `videoId` belongs to (for "repeat all"), or null. */
    Core.Item restart(String videoId) {
        return queueIndex(videoId) < 0 || queue.isEmpty() ? null : queue.get(0);
    }

    /** Play these in this order, or shuffled. Returns the one to start with, or null if empty. */
    Core.Item setQueue(List<Core.Item> items, Core.Item first, boolean shuffle) {
        queue = new ArrayList<>(items);
        if (shuffle) {
            java.util.Collections.shuffle(queue);
            if (first != null && queue.remove(first)) {
                queue.add(0, first);
            }
        } else if (first != null) {
            // Start at the tapped one and carry on from there, wrapping round to the ones before it.
            int at = queue.indexOf(first);
            if (at > 0) {
                java.util.Collections.rotate(queue, -at);
            }
        }
        return queue.isEmpty() ? null : queue.get(0);
    }

    /**
     * "Play next" (`next`) or "Add to queue": put `item` right after the playing video, or at
     * the end. A video played on its own starts a queue of its own.
     */
    void enqueue(Core.Item playing, Core.Item item, boolean next) {
        int at = queueIndex(playing.id);
        if (at < 0) {
            queue = new ArrayList<>();
            queue.add(playing);
            at = 0;
        }
        // Never twice in the rest of the queue.
        for (int i = queue.size() - 1; i > at; i--) {
            if (queue.get(i).id.equals(item.id)) {
                queue.remove(i);
            }
        }
        if (next) {
            queue.add(at + 1, item);
        } else {
            queue.add(item);
        }
    }

    /** A YouTube Music radio's "more" handle (-1 when the queue is not a radio). */
    long radioCursor = -1;

    /** Add songs at the end of the queue `playingId` is in (a radio growing). */
    void appendToQueue(String playingId, List<Core.Item> items) {
        if (playingId != null && queueIndex(playingId) < 0) {
            return;
        }
        java.util.Set<String> have = new java.util.HashSet<>();
        for (Core.Item i : queue) {
            have.add(i.id);
        }
        for (Core.Item i : items) {
            if (have.add(i.id)) {
                queue.add(i);
            }
        }
    }

    /** What is queued after `videoId` (empty when it is not playing from a queue). */
    List<Core.Item> queuedAfter(String videoId) {
        int at = queueIndex(videoId);
        return at < 0 ? new ArrayList<>() : new ArrayList<>(queue.subList(at + 1, queue.size()));
    }

    private int queueIndex(String videoId) {
        for (int i = 0; i < queue.size(); i++) {
            if (queue.get(i).id.equals(videoId)) {
                return i;
            }
        }
        return -1;
    }

    /** The next (+1) or previous (-1) item of the queue after `videoId`, or null at its ends. */
    Core.Item step(String videoId, int by) {
        int at = queueIndex(videoId);
        int to = at + by;
        return at < 0 || to < 0 || to >= queue.size() ? null : queue.get(to);
    }

    /** What to list under a downloaded video: the rest of the queue, or else the other downloads. */
    Core.Page upNext(String videoId) {
        int at = queueIndex(videoId);
        if (at < 0) {
            return others(videoId);
        }
        Core.Page page = new Core.Page();
        page.items.addAll(queue.subList(at + 1, queue.size()));
        return page;
    }

    // ------------------------------------------------------------------ the user's own playlists

    /** Name → the downloads in it, in the order they were added. */
    Map<String, List<Long>> playlists() {
        Map<String, List<Long>> out = new java.util.LinkedHashMap<>();
        try {
            JSONObject all = new JSONObject(prefs().getString("playlists", "{}"));
            for (java.util.Iterator<String> it = all.keys(); it.hasNext(); ) {
                String name = it.next();
                org.json.JSONArray ids = all.optJSONArray(name);
                List<Long> list = new ArrayList<>();
                for (int i = 0; ids != null && i < ids.length(); i++) {
                    // Downloads that were deleted since drop out.
                    if (jobs.containsKey(ids.optLong(i))) {
                        list.add(ids.optLong(i));
                    }
                }
                out.put(name, list);
            }
        } catch (org.json.JSONException ignored) {
        }
        return out;
    }

    private void savePlaylists(Map<String, List<Long>> all) {
        JSONObject o = new JSONObject();
        try {
            for (Map.Entry<String, List<Long>> e : all.entrySet()) {
                o.put(e.getKey(), new org.json.JSONArray(e.getValue()));
            }
        } catch (org.json.JSONException ignored) {
        }
        prefs().edit().putString("playlists", o.toString()).apply();
        changed();
    }

    void addToPlaylist(String name, java.util.Collection<Long> ids) {
        Map<String, List<Long>> all = playlists();
        List<Long> list = all.get(name);
        if (list == null) {
            list = new ArrayList<>();
            all.put(name, list);
        }
        for (Long id : ids) {
            if (!list.contains(id)) {
                list.add(id);
            }
        }
        savePlaylists(all);
    }

    void removeFromPlaylist(String name, java.util.Collection<Long> ids) {
        Map<String, List<Long>> all = playlists();
        List<Long> list = all.get(name);
        if (list != null) {
            list.removeAll(ids);
            savePlaylists(all);
        }
    }

    void deletePlaylist(String name) {
        Map<String, List<Long>> all = playlists();
        all.remove(name);
        savePlaylists(all);
    }

    /** Point a list item at its downloaded file, if the video has one. */
    void useFile(Core.Item video) {
        Job j = video.localUri == null ? finished(video.id) : null;
        if (j != null) {
            video.localUri = j.uri;
            video.localMime = mime(j.detail);
            video.localAudio = j.height == 0;
        }
    }

    /** A finished download as a list item that plays the file in the app's own player. */
    Core.Item playable(Context c, Job j) {
        if (j.uri == null) {
            Toast.makeText(c, "The file is not ready yet.", Toast.LENGTH_SHORT).show();
            return null;
        }
        Core.Item item = new Core.Item();
        item.type = Core.Item.VIDEO;
        item.id = j.videoId;
        item.title = j.title;
        item.thumb = "https://i.ytimg.com/vi/" + j.videoId + "/mqdefault.jpg";
        item.localUri = j.uri;
        item.localMime = mime(j.detail);
        item.localAudio = j.height == 0;
        return item;
    }

    /** Hand a finished download to another app (a music player, a gallery). */
    void openElsewhere(Context c, Job j) {
        if (j.uri == null || !j.uri.startsWith("content:")) {
            Toast.makeText(c, "The file is in the app's own folder: " + j.detail, Toast.LENGTH_LONG).show();
            return;
        }
        try {
            Intent i = new Intent(Intent.ACTION_VIEW);
            i.setDataAndType(Uri.parse(j.uri), mime(j.detail));
            i.addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_ACTIVITY_NEW_TASK);
            c.startActivity(i);
        } catch (Exception e) {
            Toast.makeText(c, "No app on this phone can open the file.", Toast.LENGTH_LONG).show();
        }
    }

    // ------------------------------------------------------------------ the screen

    private static String size(long bytes) {
        return bytes >= (1L << 30) ? String.format(Locale.US, "%.2f GB", bytes / (double) (1L << 30))
                : String.format(Locale.US, "%.1f MB", bytes / (double) (1 << 20));
    }

    /** Settings, Downloads: every download with its progress and controls. */
    /**
     * Settings, Downloads: the library of downloaded videos and music. Tabs for everything,
     * videos, music and playlists; long-press starts a selection that can be played,
     * shuffled, added to a playlist or deleted.
     */
    static final class Page extends Screen implements Listener {
        private static final String ALL = "all", VIDEOS = "videos", MUSIC = "music", LISTS = "lists";

        private final Context c;
        private final Nav nav;
        private final LinearLayout col, chips, actions;
        private final TextView title;
        private final Map<Long, Row> rows = new HashMap<>();
        /** What is laid out now, so the views are rebuilt only when it changes. */
        private String shown = "";
        private String tab = ALL;
        /** A playlist being looked at: "y:" + a YouTube playlist's name, or "u:" + one of the user's. */
        private String open;
        private final java.util.LinkedHashSet<Long> selected = new java.util.LinkedHashSet<>();
        private boolean selecting;

        private static final class Row {
            LinearLayout view;
            TextView title, status;
            ProgressBar bar;
            ImageView toggle, remove, mark;
        }

        Page(Context c, Nav nav) {
            this.c = c;
            this.nav = nav;
            title = barTitle(c, "Downloads");
            chips = Ui.row(c);
            chips.setPadding(Ui.dp(12), Ui.dp(8), Ui.dp(12), Ui.dp(4));
            actions = Ui.row(c);
            actions.setPadding(Ui.dp(12), Ui.dp(6), Ui.dp(12), Ui.dp(6));
            android.widget.HorizontalScrollView actionScroll = new android.widget.HorizontalScrollView(c);
            actionScroll.setHorizontalScrollBarEnabled(false);
            actionScroll.addView(actions);
            col = Ui.column(c);
            col.setPadding(0, Ui.dp(4), 0, Ui.dp(24));
            ScrollView scroll = new ScrollView(c);
            scroll.setVerticalScrollBarEnabled(false);
            scroll.addView(col);
            LinearLayout body = Ui.column(c);
            body.addView(chips, Ui.lin(Ui.MATCH, Ui.WRAP));
            body.addView(actionScroll, Ui.lin(Ui.MATCH, Ui.WRAP));
            body.addView(scroll, new LinearLayout.LayoutParams(Ui.MATCH, 0, 1f));
            // Back leaves a selection first, then an opened playlist, then the screen.
            LinearLayout top = Ui.row(c);
            top.setBackgroundColor(Ui.BG);
            top.setPadding(Ui.dp(4), 0, Ui.dp(4), 0);
            top.addView(Ui.iconButton(c, R.drawable.ic_back, "Back", v -> {
                if (selecting) {
                    endSelection();
                } else if (open != null) {
                    open = null;
                    render();
                } else {
                    nav.pop();
                }
            }));
            top.addView(title, Ui.weight(Ui.WRAP));
            top.setLayoutParams(Ui.lin(Ui.MATCH, Ui.dp(56)));
            view = page(c, top, body);
            // The list is live only while the screen is attached.
            view.addOnAttachStateChangeListener(new View.OnAttachStateChangeListener() {
                @Override
                public void onViewAttachedToWindow(View v) {
                    Downloads.get().addListener(Page.this);
                    render();
                }

                @Override
                public void onViewDetachedFromWindow(View v) {
                    Downloads.get().removeListener(Page.this);
                }
            });
        }

        @Override
        public void onDownloads() {
            render();
        }

        // ---------------------------------------------------------------- what is listed

        private Job job(long id) {
            for (Job j : Downloads.get().list()) {
                if (j.id == id) {
                    return j;
                }
            }
            return null;
        }

        /** The downloads of the tab or of the opened playlist, in display order. */
        private List<Job> visible() {
            List<Job> all = Downloads.get().list();
            List<Job> out = new ArrayList<>();
            if (open != null && open.startsWith("u:")) {
                List<Long> ids = Downloads.get().playlists().get(open.substring(2));
                for (int i = 0; ids != null && i < ids.size(); i++) {
                    Job j = job(ids.get(i));
                    if (j != null) {
                        out.add(j);
                    }
                }
                return out;
            }
            for (Job j : all) {
                if (open != null) {
                    if (open.substring(2).equals(j.group)) {
                        out.add(j);
                    }
                } else if (tab.equals(ALL) || (tab.equals(VIDEOS) && j.height > 0) || (tab.equals(MUSIC) && j.height == 0)) {
                    out.add(j);
                }
            }
            if (open != null) {
                // A YouTube playlist reads top to bottom in the order it was queued.
                java.util.Collections.reverse(out);
            }
            return out;
        }

        private List<Core.Item> playable(List<Job> jobs) {
            List<Core.Item> out = new ArrayList<>();
            for (Job j : jobs) {
                if (j.state.equals("Done") && j.uri != null) {
                    Core.Item item = Downloads.get().playable(c, j);
                    if (item != null) {
                        out.add(item);
                    }
                }
            }
            return out;
        }

        private void play(List<Job> jobs, Job first, boolean shuffle) {
            List<Core.Item> items = playable(jobs);
            Core.Item start = null;
            for (Core.Item i : items) {
                if (first != null && i.id.equals(first.videoId)) {
                    start = i;
                }
            }
            Core.Item begin = Downloads.get().setQueue(items, start, shuffle);
            if (begin == null) {
                Toast.makeText(c, "Nothing here has finished downloading yet.", Toast.LENGTH_SHORT).show();
            } else {
                nav.open(begin);
            }
        }

        // ---------------------------------------------------------------- selection

        private void endSelection() {
            selecting = false;
            selected.clear();
            render();
        }

        private List<Job> selectedJobs() {
            List<Job> out = new ArrayList<>();
            for (Job j : visible()) {
                if (selected.contains(j.id)) {
                    out.add(j);
                }
            }
            return out;
        }

        private void askPlaylist() {
            final List<Long> ids = new ArrayList<>(selected);
            final List<String> names = new ArrayList<>(Downloads.get().playlists().keySet());
            String[] labels = new String[names.size() + 1];
            labels[0] = "New playlist…";
            for (int i = 0; i < names.size(); i++) {
                labels[i + 1] = names.get(i);
            }
            new android.app.AlertDialog.Builder(c).setTitle("Add to playlist").setItems(labels, (d, which) -> {
                if (which > 0) {
                    Downloads.get().addToPlaylist(names.get(which - 1), ids);
                    Toast.makeText(c, "Added to " + names.get(which - 1), Toast.LENGTH_SHORT).show();
                    endSelection();
                    return;
                }
                final android.widget.EditText name = new android.widget.EditText(c);
                name.setHint("Playlist name");
                name.setSingleLine(true);
                android.widget.FrameLayout box = new android.widget.FrameLayout(c);
                box.setPadding(Ui.dp(20), Ui.dp(8), Ui.dp(20), 0);
                box.addView(name);
                new android.app.AlertDialog.Builder(c).setTitle("New playlist").setView(box)
                        .setPositiveButton("Create", (d2, w) -> {
                            String n = name.getText().toString().trim();
                            if (!n.isEmpty()) {
                                Downloads.get().addToPlaylist(n, ids);
                                Toast.makeText(c, "Added to " + n, Toast.LENGTH_SHORT).show();
                                endSelection();
                            }
                        })
                        .setNegativeButton("Cancel", null).show();
            }).setNegativeButton("Cancel", null).show();
        }

        private void deleteSelected() {
            final List<Job> jobs = selectedJobs();
            if (jobs.isEmpty()) {
                return;
            }
            final boolean fromOwn = open != null && open.startsWith("u:");
            android.app.AlertDialog.Builder ask = new android.app.AlertDialog.Builder(c)
                    .setMessage("Delete " + jobs.size() + (jobs.size() == 1 ? " download" : " downloads") + " and their files from " + FOLDER + "?")
                    .setPositiveButton("Delete files", (d, w) -> {
                        for (Job j : jobs) {
                            Downloads.get().delete(j);
                        }
                        endSelection();
                    })
                    .setNegativeButton("Cancel", null);
            if (fromOwn) {
                // In one of the user's playlists there is a gentler choice too.
                ask.setNeutralButton("Only remove from playlist", (d, w) -> {
                    List<Long> ids = new ArrayList<>();
                    for (Job j : jobs) {
                        ids.add(j.id);
                    }
                    Downloads.get().removeFromPlaylist(open.substring(2), ids);
                    endSelection();
                });
            }
            ask.show();
        }

        // ---------------------------------------------------------------- drawing

        private TextView chip(String label, boolean on, View.OnClickListener onClick) {
            TextView t = Ui.text(c, 13, on ? Ui.ON_ACCENT : Ui.TEXT, true);
            t.setText(label);
            t.setPadding(Ui.dp(14), Ui.dp(8), Ui.dp(14), Ui.dp(8));
            t.setBackground(new android.graphics.drawable.RippleDrawable(
                    android.content.res.ColorStateList.valueOf(Ui.RIPPLE), Ui.rounded(on ? Ui.ACCENT : Ui.PANEL, 18), null));
            t.setOnClickListener(onClick);
            LinearLayout.LayoutParams lp = Ui.lin(Ui.WRAP, Ui.WRAP);
            lp.rightMargin = Ui.dp(8);
            t.setLayoutParams(lp);
            return t;
        }

        private void tabChip(String label, final String value) {
            chips.addView(chip(label, tab.equals(value) && open == null, v -> {
                tab = value;
                open = null;
                selecting = false;
                selected.clear();
                render();
            }));
        }

        private TextView header(String text) {
            TextView h = Ui.text(c, 12, Ui.MUTE, true);
            h.setText(text);
            h.setPadding(Ui.dp(16), Ui.dp(14), Ui.dp(16), Ui.dp(4));
            return h;
        }

        /** One line of the Playlists tab: a name, how many downloads, and a menu for the user's own. */
        private View listCard(final String key, final String name, int count, int done) {
            LinearLayout row = Ui.row(c);
            row.setGravity(Gravity.CENTER_VERTICAL);
            row.setPadding(Ui.dp(8), Ui.dp(6), Ui.dp(4), Ui.dp(6));
            row.setForeground(Ui.ripple());
            row.addView(Ui.icon(c, R.drawable.ic_playlist, Ui.MUTE), Ui.lin(Ui.dp(48), Ui.dp(40)));
            LinearLayout texts = Ui.column(c);
            TextView n = Ui.lines(Ui.text(c, 15, Ui.TEXT, true), 1);
            n.setText(name);
            texts.addView(n);
            TextView s = Ui.text(c, 12, Ui.MUTE, false);
            s.setText((key.startsWith("u:") ? "Your playlist" : "From YouTube") + " · " + count + (count == 1 ? " item" : " items")
                    + (done < count ? " · " + done + " downloaded" : ""));
            s.setPadding(0, Ui.dp(4), 0, 0);
            texts.addView(s);
            row.addView(texts, Ui.weight(Ui.WRAP));
            row.setOnClickListener(v -> {
                open = key;
                render();
            });
            if (key.startsWith("u:")) {
                row.addView(Ui.iconButton(c, R.drawable.ic_more, "More", v -> {
                    android.widget.PopupMenu menu = new android.widget.PopupMenu(c, v);
                    menu.getMenu().add(0, 1, 0, "Delete playlist (keeps the files)");
                    menu.setOnMenuItemClickListener(item -> {
                        Downloads.get().deletePlaylist(name);
                        return true;
                    });
                    menu.show();
                }));
            }
            return row;
        }

        private void render() {
            List<Job> list = visible();
            boolean lists = tab.equals(LISTS) && open == null;
            Map<String, List<Long>> own = Downloads.get().playlists();
            // YouTube playlists that downloads came from, newest first.
            Map<String, int[]> groups = new java.util.LinkedHashMap<>();
            for (Job j : Downloads.get().list()) {
                if (j.group != null) {
                    int[] n = groups.get(j.group);
                    if (n == null) {
                        groups.put(j.group, n = new int[2]);
                    }
                    n[0]++;
                    n[1] += j.state.equals("Done") ? 1 : 0;
                }
            }

            title.setText(selecting ? selected.size() + " selected" : open != null ? open.substring(2) : "Downloads");

            chips.removeAllViews();
            tabChip("All", ALL);
            tabChip("Videos", VIDEOS);
            tabChip("Music", MUSIC);
            tabChip("Playlists", LISTS);

            actions.removeAllViews();
            if (selecting) {
                actions.addView(chip("Select all", false, v -> {
                    for (Job j : visible()) {
                        selected.add(j.id);
                    }
                    render();
                }));
                actions.addView(chip("Play", false, v -> {
                    List<Job> jobs = selectedJobs();
                    endSelection();
                    play(jobs, null, false);
                }));
                actions.addView(chip("Shuffle", false, v -> {
                    List<Job> jobs = selectedJobs();
                    endSelection();
                    play(jobs, null, true);
                }));
                actions.addView(chip("Add to playlist", false, v -> {
                    if (!selected.isEmpty()) {
                        askPlaylist();
                    }
                }));
                actions.addView(chip("Delete", false, v -> deleteSelected()));
                actions.addView(chip("Cancel", false, v -> endSelection()));
            } else if (!lists) {
                actions.addView(chip("Play all", true, v -> play(visible(), null, false)));
                actions.addView(chip("Shuffle", false, v -> play(visible(), null, true)));
                actions.addView(chip("Select", false, v -> {
                    selecting = true;
                    render();
                }));
                if (open == null) {
                    actions.addView(chip("Clear finished from list", false, v -> new android.app.AlertDialog.Builder(c)
                            .setMessage("Remove every finished, failed and cancelled download from this list? The files stay in " + FOLDER + ", but the app will no longer list or play them.")
                            .setPositiveButton("Clear", (d, w) -> Downloads.get().clearFinished())
                            .setNegativeButton("Cancel", null).show()));
                }
            }
            ((View) actions.getParent()).setVisibility(actions.getChildCount() == 0 ? View.GONE : View.VISIBLE);

            // The rows: rebuilt when the set or its grouping changes, otherwise only refreshed.
            StringBuilder sig = new StringBuilder(tab).append('|').append(open).append('|').append(selecting);
            if (lists) {
                sig.append(own.toString());
                for (Map.Entry<String, int[]> g : groups.entrySet()) {
                    sig.append(g.getKey()).append(g.getValue()[0]).append('/').append(g.getValue()[1]);
                }
            } else {
                for (Job j : list) {
                    sig.append(j.id).append(open == null ? String.valueOf(j.group) : "").append(',');
                }
            }
            if (!sig.toString().equals(shown)) {
                shown = sig.toString();
                col.removeAllViews();
                if (lists) {
                    for (Map.Entry<String, List<Long>> e : own.entrySet()) {
                        int done = 0;
                        for (Long id : e.getValue()) {
                            Job j = job(id);
                            done += j != null && j.state.equals("Done") ? 1 : 0;
                        }
                        col.addView(listCard("u:" + e.getKey(), e.getKey(), e.getValue().size(), done), Ui.lin(Ui.MATCH, Ui.WRAP));
                    }
                    for (Map.Entry<String, int[]> g : groups.entrySet()) {
                        col.addView(listCard("y:" + g.getKey(), g.getKey(), g.getValue()[0], g.getValue()[1]), Ui.lin(Ui.MATCH, Ui.WRAP));
                    }
                    if (col.getChildCount() == 0) {
                        col.addView(note("No playlists yet.\nDownload a YouTube playlist with \"Download all\" on its page, or select downloads here and choose \"Add to playlist\"."));
                    }
                } else {
                    if (list.isEmpty()) {
                        col.addView(note(tab.equals(MUSIC) && open == null
                                ? "No music yet. Choose \"Audio only\" when you download."
                                : "Nothing here yet. Use the Download button under a video.\nFinished files go to " + FOLDER + "."));
                    }
                    // Outside a playlist, downloads that came from one sit together under its name.
                    String under = "\u0000";
                    boolean grouped = open == null && !groups.isEmpty();
                    List<Job> ordered = new ArrayList<>(list);
                    if (grouped) {
                        final List<String> order = new ArrayList<>(groups.keySet());
                        java.util.Collections.sort(ordered, (a, b) -> {
                            int ga = a.group == null ? -1 : order.indexOf(a.group), gb = b.group == null ? -1 : order.indexOf(b.group);
                            return ga != gb ? Integer.compare(ga, gb) : a.group == null ? Long.compare(b.id, a.id) : Long.compare(a.id, b.id);
                        });
                    }
                    for (Job j : ordered) {
                        String g = j.group == null ? "" : j.group;
                        if (grouped && !g.equals(under)) {
                            under = g;
                            col.addView(header(g.isEmpty() ? "Singles" : g));
                        }
                        Row r = rows.get(j.id);
                        if (r == null) {
                            r = row(j.id);
                            rows.put(j.id, r);
                        }
                        if (r.view.getParent() != null) {
                            ((android.view.ViewGroup) r.view.getParent()).removeView(r.view);
                        }
                        col.addView(r.view, Ui.lin(Ui.MATCH, Ui.WRAP));
                    }
                }
            }
            if (!lists) {
                for (Job j : list) {
                    bind(rows.get(j.id), j);
                }
            }
        }

        private TextView note(String text) {
            TextView t = Ui.text(c, 14, Ui.MUTE, false);
            t.setText(text);
            t.setLineSpacing(Ui.dp(3), 1f);
            t.setPadding(Ui.dp(20), Ui.dp(28), Ui.dp(20), Ui.dp(20));
            return t;
        }

        private Row row(final long id) {
            Row r = new Row();
            r.view = Ui.row(c);
            r.view.setGravity(Gravity.CENTER_VERTICAL);
            r.view.setPadding(Ui.dp(8), Ui.dp(10), Ui.dp(4), Ui.dp(10));
            r.view.setForeground(Ui.ripple());
            r.view.setOnClickListener(v -> {
                Job j = job(id);
                if (j == null) {
                    return;
                }
                if (selecting) {
                    if (!selected.remove(id)) {
                        selected.add(id);
                    }
                    render();
                } else if (j.state.equals("Done")) {
                    // Play this one and carry on through the rest of what is listed.
                    play(visible(), j, false);
                }
            });
            // A long press starts a selection with this row.
            r.view.setOnLongClickListener(v -> {
                selecting = true;
                selected.add(id);
                render();
                return true;
            });
            // The mark on the left: a tick while selecting, otherwise what kind of file it is.
            r.mark = Ui.icon(c, R.drawable.ic_play, Ui.MUTE);
            r.view.addView(r.mark, Ui.lin(Ui.dp(40), Ui.dp(40)));
            LinearLayout texts = Ui.column(c);
            r.title = Ui.lines(Ui.text(c, 14, Ui.TEXT, true), 2);
            texts.addView(r.title);
            r.status = Ui.lines(Ui.text(c, 12, Ui.MUTE, false), 2);
            r.status.setPadding(0, Ui.dp(4), 0, 0);
            texts.addView(r.status);
            r.bar = new ProgressBar(c, null, android.R.attr.progressBarStyleHorizontal);
            r.bar.setMax(1000);
            r.bar.setProgressTintList(android.content.res.ColorStateList.valueOf(Ui.ACCENT));
            LinearLayout.LayoutParams bl = Ui.lin(Ui.MATCH, Ui.dp(4));
            bl.topMargin = Ui.dp(8);
            texts.addView(r.bar, bl);
            r.view.addView(texts, Ui.weight(Ui.WRAP));
            r.toggle = Ui.iconButton(c, R.drawable.ic_pause, "Pause", v -> {
                Job j = job(id);
                if (j == null) {
                    return;
                }
                if (j.active()) {
                    Downloads.get().pause(j);
                } else {
                    Downloads.get().resume(j);
                }
            });
            r.view.addView(r.toggle);
            r.remove = Ui.iconButton(c, R.drawable.ic_more, "More", v -> {
                final Job j = job(id);
                if (j == null) {
                    return;
                }
                final boolean done = j.state.equals("Done");
                android.widget.PopupMenu menu = new android.widget.PopupMenu(c, v);
                if (done) {
                    menu.getMenu().add(0, 1, 0, "Play");
                    menu.getMenu().add(0, 5, 1, "Add to playlist");
                    menu.getMenu().add(0, 2, 2, "Open in another app");
                }
                menu.getMenu().add(0, 3, 3, "Download in another quality");
                menu.getMenu().add(0, 4, 4, done ? "Delete file" : j.active() || j.state.equals("Paused") ? "Cancel download" : "Remove from list");
                menu.setOnMenuItemClickListener(item -> {
                    switch (item.getItemId()) {
                        case 1:
                            play(visible(), j, false);
                            break;
                        case 2:
                            Downloads.get().openElsewhere(c, j);
                            break;
                        case 3: {
                            Core.Item video = new Core.Item();
                            video.type = Core.Item.VIDEO;
                            video.id = j.videoId;
                            video.title = j.title;
                            Downloads.get().choose(c, video);
                            break;
                        }
                        case 5:
                            selected.clear();
                            selected.add(id);
                            askPlaylist();
                            break;
                        default:
                            if (done) {
                                new android.app.AlertDialog.Builder(c)
                                        .setMessage("Delete this file from " + FOLDER + "?\n\n" + j.title)
                                        .setPositiveButton("Delete", (d, w) -> Downloads.get().delete(j))
                                        .setNegativeButton("Cancel", null)
                                        .show();
                            } else {
                                Downloads.get().delete(j);
                            }
                    }
                    return true;
                });
                menu.show();
            });
            r.view.addView(r.remove);
            return r;
        }

        private void bind(Row r, Job j) {
            if (r == null) {
                return;
            }
            boolean picked = selecting && selected.contains(j.id);
            r.view.setBackgroundColor(picked ? Ui.PANEL : 0);
            r.mark.setImageResource(selecting ? (picked ? R.drawable.ic_check : R.drawable.ic_circle)
                    : j.height == 0 ? R.drawable.ic_headset : R.drawable.ic_play);
            r.mark.setImageTintList(android.content.res.ColorStateList.valueOf(picked ? Ui.ACCENT : Ui.MUTE));
            r.title.setText(j.title.isEmpty() ? j.videoId : j.title);
            String what = j.height == 0 ? "Audio" : "Video up to " + j.height + "p";
            String text;
            switch (j.state) {
                case "Downloading":
                    int pct = j.total > 0 ? (int) (100 * j.done / j.total) : 0;
                    text = pct + "% · " + size(j.done) + " of " + size(j.total) + (j.speed > 0 ? " · " + size(j.speed) + "/s" : "");
                    break;
                case "Queued":
                    text = "Waiting";
                    break;
                case "Resolving":
                    text = "Getting the streams";
                    break;
                case "Muxing":
                    text = "Joining picture and sound";
                    break;
                case "Paused":
                    text = "Paused" + (j.total > 0 ? " at " + (int) (100 * j.done / j.total) + "%" : "");
                    break;
                case "Done":
                    text = j.publishing ? "Moving to " + FOLDER : "Downloaded";
                    break;
                case "Failed":
                    text = "Failed: " + j.detail;
                    break;
                default:
                    text = j.state;
            }
            r.status.setText(what + " · " + text);
            boolean moving = j.state.equals("Downloading") || j.state.equals("Paused");
            r.bar.setVisibility(moving || j.state.equals("Muxing") || j.state.equals("Resolving") ? View.VISIBLE : View.GONE);
            r.bar.setIndeterminate(!moving);
            r.bar.setProgress(j.total > 0 ? (int) (1000 * j.done / j.total) : 0);
            boolean canToggle = j.active() || j.state.equals("Paused") || j.state.equals("Failed");
            r.toggle.setVisibility(!selecting && canToggle && !j.state.equals("Muxing") ? View.VISIBLE : View.GONE);
            r.toggle.setImageResource(j.active() ? R.drawable.ic_pause : R.drawable.ic_play);
            r.toggle.setContentDescription(j.active() ? "Pause" : j.state.equals("Failed") ? "Try again" : "Resume");
            r.remove.setVisibility(selecting ? View.GONE : View.VISIBLE);
        }
    }
}
