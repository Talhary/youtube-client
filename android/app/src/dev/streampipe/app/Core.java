package dev.streampipe.app;

import android.content.Context;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.io.File;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * The app's side of the core: sends commands, hands every answer back on the main thread,
 * and remembers the latest sign-in state and settings.
 */
final class Core implements Native.Listener {
    private static final String TAG = "StreamPipe";

    /** The answer to one request: `error` is null on success. */
    interface Reply {
        void done(JSONObject event, String error);
    }

    interface Listener {
        /** Sign-in state, settings or status changed. */
        void onCoreChanged();

        void onToast(String text);
    }

    /** One list entry. */
    static final class Item {
        static final int VIDEO = 0, CHANNEL = 1, PLAYLIST = 2, POST = 3;
        int type;
        String id = "";
        String title = "";
        String channelId;
        String channelName;
        String channelAvatar;
        String thumb;
        String published;
        long views = -1;
        long subscribers = -1;
        long videoCount = -1;
        int durationS = -1;
        /** Percent of the video already watched, 0 when none or unknown. */
        int watched;
        boolean live;
        /** A downloaded file to play instead of streaming (and whether it has sound only). */
        String localUri;
        String localMime;
        boolean localAudio;
        /**
         * A YouTube Music entry: `kind` is "song", "video", "album", "single", "playlist",
         * "artist", "podcast", "episode", "mood" or "page"; `id` is the video id (songs,
         * videos) or the page to open.
         */
        boolean music;
        String kind;
        String params;
        String albumId;
        String albumName;
        String duration;
        int color;

        /** Plays (a song, video or episode) rather than opening a page. */
        boolean playable() {
            return !music || "song".equals(kind) || "video".equals(kind) || "episode".equals(kind);
        }

        static Item music(JSONObject o) {
            Item i = new Item();
            i.music = true;
            i.kind = o.optString("kind");
            i.id = o.optString("id");
            i.params = str(o, "params");
            i.title = o.optString("title");
            i.published = o.optString("subtitle", null);
            if (i.published != null && i.published.isEmpty()) {
                i.published = null;
            }
            i.thumb = url(str(o, "thumbnail"));
            i.duration = str(o, "duration");
            i.durationS = parseDuration(i.duration);
            i.color = (int) o.optLong("color", 0);
            JSONArray artists = o.optJSONArray("artists");
            StringBuilder names = new StringBuilder();
            for (int k = 0; artists != null && k < artists.length(); k++) {
                JSONObject a = artists.optJSONObject(k);
                if (a == null) {
                    continue;
                }
                if (names.length() > 0) {
                    names.append(", ");
                }
                names.append(a.optString("name"));
                if (i.channelId == null && !a.isNull("id")) {
                    i.channelId = a.optString("id", null);
                }
            }
            i.channelName = names.length() > 0 ? names.toString() : null;
            JSONObject album = o.optJSONObject("album");
            if (album != null) {
                i.albumName = album.optString("name");
                i.albumId = str(album, "id");
            }
            i.type = i.playable() ? VIDEO : "artist".equals(i.kind) ? CHANNEL : PLAYLIST;
            return i;
        }

        static int parseDuration(String d) {
            if (d == null || d.isEmpty()) {
                return -1;
            }
            int s = 0;
            try {
                for (String p : d.split(":")) {
                    s = s * 60 + Integer.parseInt(p.trim());
                }
            } catch (NumberFormatException e) {
                return -1;
            }
            return s;
        }

        /** A post: its text, likes, pictures, and the video attached to it. */
        String text;
        String likes;
        List<String> images;
        Item attached;

        /** "Channel · 1.2M views · 3 days ago", leaving out what is unknown. */
        /** YouTube's short card line: "Fireship  ▷2.4M  2y ago". */
        String shortMeta() {
            StringBuilder sb = new StringBuilder();
            if (channelName != null && !channelName.isEmpty()) {
                sb.append(channelName);
            }
            if (views >= 0) {
                sb.append(sb.length() > 0 ? "  " : "").append(live ? Ui.count(views) + " watching" : "▷" + Ui.count(views));
            }
            if (published != null && !published.isEmpty()) {
                sb.append(sb.length() > 0 ? "  " : "").append(shortAgo(published));
            }
            return sb.toString();
        }

        /** "7 years ago" → "7y ago", as YouTube's app writes it. */
        static String shortAgo(String s) {
            java.util.regex.Matcher m = java.util.regex.Pattern.compile("(\\d+) (second|minute|hour|day|week|month|year)s? ago").matcher(s);
            if (!m.find()) {
                return s;
            }
            String unit;
            switch (m.group(2)) {
                case "second": unit = "s"; break;
                case "minute": unit = "m"; break;
                case "hour": unit = "h"; break;
                case "day": unit = "d"; break;
                case "week": unit = "w"; break;
                case "month": unit = "mo"; break;
                default: unit = "y";
            }
            return s.substring(0, m.start()) + m.group(1) + unit + " ago" + s.substring(m.end());
        }

        String meta(boolean withChannel) {
            return Ui.join(withChannel ? channelName : null, views >= 0 ? Ui.count(views) + " views" : null, published);
        }

        static Item video(JSONObject v) {
            Item i = new Item();
            i.type = VIDEO;
            i.id = v.optString("id");
            i.title = v.optString("title");
            i.channelId = str(v, "channel_id");
            i.channelName = str(v, "channel_name");
            i.channelAvatar = url(str(v, "channel_avatar"));
            i.thumb = url(str(v, "thumbnail"));
            i.published = str(v, "published");
            i.views = v.isNull("views") ? -1 : v.optLong("views", -1);
            i.durationS = v.isNull("duration_s") ? -1 : v.optInt("duration_s", -1);
            i.live = v.optBoolean("is_live");
            i.watched = v.isNull("watched") ? 0 : v.optInt("watched", 0);
            return i;
        }

        static Item parse(JSONObject o) {
            JSONObject v = o.optJSONObject("Video");
            if (v != null) {
                return video(v);
            }
            Item i = new Item();
            JSONObject c = o.optJSONObject("Channel");
            if (c != null) {
                i.type = CHANNEL;
                i.id = c.optString("id");
                i.title = c.optString("name");
                i.thumb = url(str(c, "avatar"));
                i.subscribers = c.isNull("subscribers") ? -1 : c.optLong("subscribers", -1);
                return i;
            }
            JSONObject post = o.optJSONObject("Post");
            if (post != null) {
                i.type = POST;
                i.id = post.optString("id");
                i.title = Ui.join(str(post, "author"));
                i.channelName = str(post, "author");
                i.channelId = str(post, "author_id");
                i.channelAvatar = url(str(post, "author_avatar"));
                i.text = post.optString("text");
                i.published = str(post, "published");
                i.likes = str(post, "likes");
                i.images = new ArrayList<>();
                JSONArray imgs = post.optJSONArray("images");
                for (int k = 0; imgs != null && k < imgs.length(); k++) {
                    i.images.add(url(imgs.optString(k)));
                }
                i.thumb = i.images.isEmpty() ? null : i.images.get(0);
                JSONObject pv = post.optJSONObject("video");
                i.attached = pv == null ? null : video(pv);
                return i;
            }
            JSONObject p = o.optJSONObject("Playlist");
            if (p != null) {
                i.type = PLAYLIST;
                i.id = p.optString("id");
                i.title = p.optString("name");
                i.thumb = url(str(p, "thumbnail"));
                i.channelName = str(p, "channel_name");
                i.videoCount = p.isNull("video_count") ? -1 : p.optLong("video_count", -1);
                return i;
            }
            return null;
        }
    }

    /** One page of a list plus the handle for the next page (-1 when there is none). */
    static final class Page {
        final List<Item> items = new ArrayList<>();
        long next = -1;

        static Page parse(JSONObject page) {
            Page p = new Page();
            if (page == null) {
                return p;
            }
            JSONArray a = page.optJSONArray("items");
            for (int i = 0; a != null && i < a.length(); i++) {
                JSONObject o = a.optJSONObject(i);
                Item item = o == null ? null : Item.parse(o);
                if (item != null && !item.id.isEmpty()) {
                    p.items.add(item);
                }
            }
            p.next = page.isNull("next") ? -1 : page.optLong("next", -1);
            return p;
        }
    }

    static String str(JSONObject o, String key) {
        return o.isNull(key) ? null : o.optString(key, null);
    }

    static String url(String u) {
        if (u == null || u.isEmpty()) {
            return null;
        }
        return u.startsWith("//") ? "https:" + u : u;
    }

    private static final Core INSTANCE = new Core();

    static Core get() {
        return INSTANCE;
    }

    private final Handler main = new Handler(Looper.getMainLooper());
    private final Map<Long, Reply> pending = new HashMap<>();
    private final List<Listener> listeners = new ArrayList<>();
    private long nextReq = 1;
    private boolean started;

    /** Better streams arrived for a video that is already playing. */
    interface StreamsUpdate {
        void onStreams(String id, JSONObject streams);
    }

    StreamsUpdate streamsUpdate;

    /** Resolve videos on screen ahead of time (the direct player is on). */
    boolean prefetch = true;

    /** null until the core has checked the saved session. */
    Boolean signedIn;
    JSONObject settings;
    String version = "";
    String cacheDir = "";

    private Core() {}

    void start(Context c, String versionName) {
        if (started) {
            return;
        }
        started = true;
        version = versionName;
        cacheDir = c.getCacheDir().getAbsolutePath();
        Downloads.get().init(c);
        Native.listener = this;
        File downloads = c.getExternalFilesDir(android.os.Environment.DIRECTORY_DOWNLOADS);
        if (downloads == null) {
            downloads = new File(c.getFilesDir(), "downloads");
        }
        String err = Native.start(c.getFilesDir().getAbsolutePath(), cacheDir, downloads.getAbsolutePath(), versionName);
        if (err != null) {
            Log.e(TAG, "core did not start: " + err);
        }
    }

    void addListener(Listener l) {
        listeners.add(l);
    }

    void removeListener(Listener l) {
        listeners.remove(l);
    }

    boolean isSignedIn() {
        return Boolean.TRUE.equals(signedIn);
    }

    boolean autoplay() {
        return settings == null || settings.optBoolean("autoplay", true);
    }

    void setAutoplay(boolean on) {
        if (settings == null) {
            return;
        }
        try {
            settings.put("autoplay", on);
            send(new JSONObject().put("cmd", "update_settings").put("settings", settings));
        } catch (JSONException ignored) {
        }
    }

    /** Send a command that has no answer. */
    void send(JSONObject cmd) {
        String err = Native.send(cmd.toString());
        if (err != null) {
            Log.w(TAG, "command " + cmd.optString("cmd") + ": " + err);
        }
    }

    void send(String cmd, Object... pairs) {
        send(build(cmd, pairs));
    }

    private static JSONObject build(String cmd, Object... pairs) {
        JSONObject o = new JSONObject();
        try {
            o.put("cmd", cmd);
            for (int i = 0; i + 1 < pairs.length; i += 2) {
                o.put((String) pairs[i], pairs[i + 1]);
            }
        } catch (JSONException ignored) {
        }
        return o;
    }

    /** Send a request; `reply` runs on the main thread. Main thread only. */
    void call(Reply reply, String cmd, Object... pairs) {
        final long req = nextReq++;
        JSONObject o = build(cmd, pairs);
        try {
            o.put("req", req);
        } catch (JSONException ignored) {
        }
        pending.put(req, reply);
        String err = Native.send(o.toString());
        if (err != null) {
            pending.remove(req);
            reply.done(null, err);
            return;
        }
        // A request the core never answers must not leave a screen loading forever.
        main.postDelayed(() -> {
            Reply r = pending.remove(req);
            if (r != null) {
                r.done(null, "No answer. Check your connection and try again.");
            }
        }, 60_000);
    }

    @Override
    public void onCoreEvent(final String json) {
        main.post(() -> dispatch(json));
    }

    private void dispatch(String json) {
        final JSONObject e;
        try {
            e = new JSONObject(json);
        } catch (JSONException ex) {
            return;
        }
        if (e.has("req")) {
            Reply r = pending.remove(e.optLong("req"));
            if (r != null) {
                r.done(e, e.has("err") ? e.optString("err") : null);
            }
            return;
        }
        switch (e.optString("ev")) {
            case "settings":
                settings = e.optJSONObject("settings");
                changed();
                break;
            case "auth": {
                Object a = e.opt("auth");
                if ("SignedOut".equals(a) || "Denied".equals(a)) {
                    signedIn = false;
                } else if (a instanceof JSONObject) {
                    JSONObject o = (JSONObject) a;
                    if (o.has("SignedIn")) {
                        signedIn = true;
                    } else if (o.has("Failed")) {
                        signedIn = false;
                        toast(o.optString("Failed"));
                    }
                }
                changed();
                break;
            }
            case "toast":
                toast(e.optString("text"));
                break;
            case "download":
                Downloads.get().onEvent(e.optJSONObject("download"));
                break;
            case "streams_update":
                if (streamsUpdate != null) {
                    streamsUpdate.onStreams(e.optString("id"), e.optJSONObject("streams"));
                }
                break;
            default:
        }
    }

    private void changed() {
        for (Listener l : new ArrayList<>(listeners)) {
            l.onCoreChanged();
        }
    }

    private void toast(String text) {
        for (Listener l : new ArrayList<>(listeners)) {
            l.onToast(text);
        }
    }
}
