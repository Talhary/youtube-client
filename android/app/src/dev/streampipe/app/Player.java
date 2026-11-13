package dev.streampipe.app;

import android.annotation.SuppressLint;
import android.app.Activity;
import android.content.Context;
import android.graphics.Bitmap;
import android.graphics.Color;
import android.net.Uri;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.view.View;
import android.webkit.CookieManager;
import android.webkit.JavascriptInterface;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceRequest;
import android.webkit.WebResourceResponse;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;

import org.json.JSONArray;
import org.json.JSONException;
import org.json.JSONObject;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * The video player, in one web view, with two ways to play:
 *
 * <ul>
 * <li>Direct (the default): the app's own page (assets/player.html) plays the video's stream
 * URLs, resolved by the core, through the device's hardware decoders. No YouTube page is
 * loaded, so there is nothing to block.
 * <li>YouTube page: YouTube's own mobile watch page, signed in with the user's session. Every
 * YouTube document it loads is fetched here first so the page script (uBlock Origin's rules,
 * the ad skipper, the player-only layout) can be put at the very top, before any of
 * YouTube's scripts run.
 * </ul>
 *
 * A video the direct player cannot resolve or play falls back to the YouTube page.
 */
final class Player {
    private static final String TAG = "StreamPipe";
    static final Pattern VIDEO_ID = Pattern.compile("^[A-Za-z0-9_-]{11}$");
    private static final Pattern HEAD_TAG = Pattern.compile("<head[^>]*>", Pattern.CASE_INSENSITIVE);

    interface Listener {
        /** The page moved to another video by itself (e.g. a suggestion tapped in the player). */
        void onVideo(String id);

        void onEnded(String id);

        void onPlaying(boolean playing);

        /** YouTube's fullscreen button was pressed: enter or leave the app's fullscreen. */
        void onFullscreenToggle();

        /** The gear button was pressed; `info` lists qualities, speeds and caption tracks. */
        void onSettings(JSONObject info);
    }

    /** What the app's own controls need from the direct player. */
    interface Progress {
        /** A video starts in the direct player (true) or in YouTube's page (false). */
        void onDirect(boolean on);

        /** Seconds played, total and buffered. */
        void onProgress(double at, double total, double buffered, boolean paused, boolean waiting);

        /** The caption line to show now ("" for none). */
        default void onCaption(String text) {}

        /** A live stream is playing (`behind`: not at the live edge). */
        default void onLive(boolean live, boolean behind) {}
    }

    Progress progress;

    private void tellDirect(boolean on) {
        if (progress != null) {
            progress.onDirect(on);
        }
    }

    /** A web view that can keep playing while the app is not on screen. */
    static final class Web extends WebView {
        boolean keepAlive;

        Web(Context c) {
            super(c);
        }

        @Override
        protected void onWindowVisibilityChanged(int visibility) {
            // Telling the web view its window is hidden makes it stop media. For background
            // play it is simply never told.
            Log.i(TAG, "web view: window visibility " + visibility + (keepAlive ? " (kept alive)" : ""));
            if (visibility == View.VISIBLE || !keepAlive) {
                super.onWindowVisibilityChanged(visibility);
            }
        }
    }

    private final Activity activity;
    private final Listener listener;
    private final Handler main = new Handler(Looper.getMainLooper());
    final Web web;
    private final String userAgent;
    private String scriptSource;

    /** The video the page shows, or null when closed. */
    String currentId;
    boolean playing;
    /** Shown in Settings. */
    volatile String adRulesText = "not loaded yet";

    @SuppressLint("SetJavaScriptEnabled")
    Player(Activity a, Listener l) {
        activity = a;
        listener = l;
        web = new Web(a);
        web.setBackgroundColor(Color.BLACK);
        WebSettings s = web.getSettings();
        s.setJavaScriptEnabled(true);
        s.setDomStorageEnabled(true);
        s.setMediaPlaybackRequiresUserGesture(false);
        s.setSupportMultipleWindows(false);
        s.setTextZoom(100);
        userAgent = chromeUserAgent(s.getUserAgentString());
        s.setUserAgentString(userAgent);
        CookieManager.getInstance().setAcceptCookie(true);
        CookieManager.getInstance().setAcceptThirdPartyCookies(web, true);
        web.setOverScrollMode(View.OVER_SCROLL_NEVER);
        web.setVerticalScrollBarEnabled(false);
        web.setHorizontalScrollBarEnabled(false);
        web.addJavascriptInterface(new Bridge(), "SPP");
        web.setWebViewClient(new Client(WATCH_OPTIONS));
        web.setWebChromeClient(new Chrome());
        // The first streams of a video can turn out to be a preview; the full ones replace
        // them while it plays.
        Core.get().streamsUpdate = (id, streams) -> {
            if (id.equals(currentId) && playingDirect && nativeStreams != null && streams != null) {
                // The app's stream proxy keeps the local addresses and now fetches from the
                // full streams, so the player carries on and its retried requests succeed.
                String before = address(nativeVideo) + " " + address(chooseAudio(nativeStreams));
                long at = Player.this.exo.positionMs();
                boolean play = Player.this.exo.wantsToPlay();
                nativeStreams = streams;
                nativeOptions = options(streams);
                JSONObject same = null;
                for (JSONObject f : nativeOptions) {
                    if (nativeVideo != null && f.optInt("itag") == nativeVideo.optInt("itag")) {
                        same = f;
                    }
                }
                String after = address(same) + " " + address(chooseAudio(streams));
                if (before.equals(after)) {
                    nativeVideo = same;
                    Log.i(TAG, "direct: " + id + " now uses its full streams (no restart)");
                } else {
                    Log.i(TAG, "direct: restarting " + id + " on its full streams");
                    startNative(at, play);
                }
            } else if (id.equals(currentId) && playingDirect && Player.this.exo.isActive()) {
                // A downloaded file is playing: nothing to change.
                Log.i(TAG, "direct: stream update ignored, a file is playing");
            } else if (id.equals(currentId) && playingDirect) {
                // Same files, other addresses: the page keeps its player and its buffer, and
                // the requests it is retrying start to succeed.
                if (updateSources(streams)) {
                    Log.i(TAG, "direct: " + id + " now uses its full streams (no restart)");
                } else {
                    JSONObject data = directData(id, streams);
                    if (data != null) {
                        Log.i(TAG, "direct: restarting " + id + " on its full streams");
                        directPending = null;
                        web.evaluateJavascript("window.__sp&&window.__sp.swap(" + data + ")", null);
                    }
                }
            }
        };
        // Test builds: lets a computer inspect the page over USB.
        WebView.setWebContentsDebuggingEnabled(true);
        exo = new Exo(a, new Exo.Events() {
            @Override
            public void onPlaying(boolean now) {
                if (currentId != null && playingDirect && playing != now) {
                    playing = now;
                    listener.onPlaying(now);
                }
            }

            @Override
            public void onEnded() {
                if (currentId != null && playingDirect) {
                    listener.onEnded(currentId);
                }
            }

            @Override
            public void onProgress(double at, double total, double buffered, boolean paused, boolean waiting) {
                if (progress != null && playingDirect && currentId != null) {
                    progress.onProgress(at, total, buffered, paused, waiting);
                    progress.onCaption(captionAt(at));
                    boolean live = Player.this.exo.isLive();
                    progress.onLive(live, live && Player.this.exo.behindLive() > 15);
                }
                if (playingDirect && currentId != null && !Player.this.exo.isLive()) {
                    Resume.played(currentId, at, total);
                }
                // YouTube's page reports what it plays by itself; the native player has to
                // say so, or the video never reaches the account's history and the home
                // feed keeps recommending the same things.
                if (playingDirect && currentId != null && localFile == null && Core.get().isSignedIn() && !historyPaused()) {
                    boolean started = currentId.equals(reportedWatched);
                    // First after ten seconds, then every minute, and once more at the end,
                    // so YouTube knows how much was really watched.
                    boolean due = !started ? at >= 10
                            : at - reportedAt >= 60 || at < reportedAt - 5 || (total > 0 && at >= total - 3 && at - reportedAt >= 3);
                    if (due) {
                        reportedWatched = currentId;
                        reportedAt = at;
                        Core.get().send("watched", "id", currentId, "seconds", (int) Math.round(at), "total", (int) Math.round(total));
                    }
                }
            }

            @Override
            public void onError(String why) {
                final String id = currentId;
                Log.w(TAG, "native player: " + why);
                if (id == null || !playingDirect) {
                    return;
                }
                if (localFile != null) {
                    // Say so: otherwise the video would quietly start streaming.
                    android.widget.Toast.makeText(activity, "The downloaded file did not play (" + why + "). Streaming it instead.",
                            android.widget.Toast.LENGTH_LONG).show();
                    openDirect(id);
                } else {
                    Log.i(TAG, "native player: using the YouTube page for " + id);
                    openPage(id);
                }
            }
        });
        exo.view().setVisibility(View.GONE);
    }

    // ------------------------------------------------------------------ native player

    /**
     * The native player (ExoPlayer). The direct option and downloaded files play here; the
     * web view is left to the YouTube-page option.
     */
    private final Exo exo;
    /** The streams of the video the native player has, as the core sent them. */
    private JSONObject nativeStreams;
    /** One picture format per height, best first; each has a "hw" flag. */
    private List<JSONObject> nativeOptions = new java.util.ArrayList<>();
    /** The picture format in use; null in audio only. */
    private JSONObject nativeVideo;
    private boolean localAudio;
    /** The video last reported to the account's watch history. */
    private String reportedWatched;
    private double reportedAt;

    View nativeView() {
        return exo.view();
    }

    /** Music layout: the cover fills the player's square. */
    void setMusic(boolean on) {
        exo.setMusic(on, musicCover != null && currentId != null && currentId.equals(musicCoverFor));
    }

    /** Album art for a YouTube Music song (square), used instead of the video's thumbnail. */
    private String musicCover, musicCoverFor;

    void setMusicCover(String id, String url) {
        musicCoverFor = id;
        musicCover = url;
    }

    private String cover(String id) {
        return musicCover != null && id != null && id.equals(musicCoverFor) ? musicCover : thumb(id);
    }

    void release() {
        exo.release();
    }

    /** Show the native player's picture, or the web view. */
    private void showNative(boolean on) {
        exo.view().setVisibility(on ? View.VISIBLE : View.GONE);
        web.setVisibility(on ? View.INVISIBLE : View.VISIBLE);
    }

    private void pageCommand(String what) {
        web.evaluateJavascript("window.__sp&&window.__sp.cmd(" + JSONObject.quote(what) + ")", null);
    }

    private static String thumb(String id) {
        return "https://i.ytimg.com/vi/" + id + "/hqdefault.jpg";
    }

    // Quality names are YouTube's, so one saved choice serves both players.
    private static final int[] HEIGHTS = {144, 240, 360, 480, 720, 1080, 1440, 2160, 4320};
    private static final String[] NAMES = {"tiny", "small", "medium", "large", "hd720", "hd1080", "hd1440", "hd2160", "highres"};

    private static String nameOf(int height) {
        for (int i = 0; i < HEIGHTS.length; i++) {
            if (HEIGHTS[i] == height) {
                return NAMES[i];
            }
        }
        return "h" + height;
    }

    private static int heightOf(String name) {
        for (int i = 0; i < NAMES.length; i++) {
            if (NAMES[i].equals(name)) {
                return HEIGHTS[i];
            }
        }
        try {
            return Integer.parseInt(name.replaceFirst("^h", ""));
        } catch (NumberFormatException e) {
            return 720;
        }
    }

    /** 2 = this phone decodes the codec in hardware, 1 = in software only, 0 = not at all. */
    private static final Map<String, Integer> DECODERS = new HashMap<>();

    private static int decoder(String codec, int height) {
        String mime = codec.equals("h264") ? "video/avc" : codec.equals("vp9") ? "video/x-vnd.on2.vp9"
                : codec.equals("av1") ? "video/av01" : null;
        if (mime == null) {
            return 0;
        }
        String key = mime + "@" + height;
        Integer known = DECODERS.get(key);
        if (known != null) {
            return known;
        }
        int best = 0;
        try {
            for (android.media.MediaCodecInfo info : new android.media.MediaCodecList(android.media.MediaCodecList.REGULAR_CODECS).getCodecInfos()) {
                if (info.isEncoder()) {
                    continue;
                }
                for (String type : info.getSupportedTypes()) {
                    if (!type.equalsIgnoreCase(mime)) {
                        continue;
                    }
                    android.media.MediaCodecInfo.VideoCapabilities video = info.getCapabilitiesForType(type).getVideoCapabilities();
                    int width = Math.round(height * 16f / 9f);
                    if (video != null && !video.isSizeSupported(width, height) && !video.isSizeSupported(height, width)) {
                        continue;
                    }
                    String name = info.getName().toLowerCase(Locale.ROOT);
                    boolean hardware = android.os.Build.VERSION.SDK_INT >= 29 ? info.isHardwareAccelerated()
                            : !(name.startsWith("omx.google.") || name.startsWith("c2.android."));
                    best = Math.max(best, hardware ? 2 : 1);
                }
            }
        } catch (RuntimeException e) {
            best = codec.equals("h264") ? 1 : 0;
        }
        DECODERS.put(key, best);
        return best;
    }

    /**
     * Per height, the format this phone plays best: a hardware-decoded one beats a software
     * one; among hardware ones the more efficient codec wins (AV1, then VP9, then H.264);
     * in software H.264 is the cheapest.
     */
    private static List<JSONObject> options(JSONObject streams) {
        Map<Integer, JSONObject> best = new java.util.TreeMap<>(java.util.Collections.reverseOrder());
        JSONArray list = streams.optJSONArray("video");
        for (int i = 0; list != null && i < list.length(); i++) {
            JSONObject f = list.optJSONObject(i);
            JSONArray urls = f == null ? null : f.optJSONArray("urls");
            if (urls == null || urls.length() == 0) {
                continue;
            }
            String codec = f.optString("codec");
            int height = f.optInt("height");
            int can = decoder(codec, height);
            if (can == 0) {
                continue;
            }
            boolean hw = can == 2;
            int byCodec = codec.equals("av1") ? (hw ? 100 : 0) : codec.equals("vp9") ? (hw ? 75 : 25) : 50;
            int score = (hw ? 1000 : 0) + byCodec + Math.min(49, f.optInt("fps", 30) / 2);
            JSONObject old = best.get(height);
            if (old == null || score > old.optInt("score")) {
                try {
                    f.put("score", score).put("hw", hw);
                } catch (JSONException ignored) {
                }
                best.put(height, f);
            }
        }
        return new java.util.ArrayList<>(best.values());
    }

    private JSONObject pickVideo() {
        if (nativeOptions.isEmpty()) {
            return null;
        }
        if (!"auto".equals(preferredQuality)) {
            int want = heightOf(preferredQuality);
            // The chosen height, or the nearest one below it that this video has.
            for (JSONObject f : nativeOptions) {
                if (f.optInt("height") <= want) {
                    return f;
                }
            }
            return nativeOptions.get(nativeOptions.size() - 1);
        }
        // Auto: enough for this screen (at least 720p), hardware-decoded when possible.
        android.util.DisplayMetrics m = activity.getResources().getDisplayMetrics();
        int cap = Math.max(720, Math.min(m.widthPixels, m.heightPixels));
        JSONObject first = null;
        for (JSONObject f : nativeOptions) {
            if (f.optInt("height") > cap) {
                continue;
            }
            if (first == null) {
                first = f;
            }
            if (f.optBoolean("hw")) {
                return f;
            }
        }
        return first != null ? first : nativeOptions.get(nativeOptions.size() - 1);
    }

    private static String address(JSONObject format) {
        JSONArray urls = format == null ? null : format.optJSONArray("urls");
        return urls == null || urls.length() == 0 ? null : urls.optString(0);
    }

    /** (Re)start the native player on the current streams at a position. */
    private void startNative(long positionMs, boolean play) {
        final String id = currentId;
        if (id == null || nativeStreams == null) {
            return;
        }
        String hls = nativeStreams.isNull("hls") ? null : nativeStreams.optString("hls", null);
        if (nativeStreams.optBoolean("live") && hls != null) {
            Log.i(TAG, "native player: " + id + " live (HLS)");
            JSONArray v = nativeStreams.optJSONArray("video");
            JSONObject any = v != null && v.length() > 0 ? v.optJSONObject(0) : nativeStreams.optJSONObject("audio");
            exo.setFill(fill);
            exo.setRate(1);
            exo.playLive(hls, any == null || any.isNull("agent") ? null : any.optString("agent"), play);
            return;
        }
        nativeOptions = options(nativeStreams);
        nativeVideo = audioOnly ? null : pickVideo();
        String audio = address(chooseAudio(nativeStreams));
        String video = address(nativeVideo);
        if (audio == null || (video == null && !audioOnly)) {
            Log.i(TAG, "native player: no stream this phone can play for " + id + ", using the YouTube page");
            openPage(id);
            return;
        }
        Log.i(TAG, "native player: " + id + " " + (nativeVideo == null ? "audio only"
                : nativeVideo.optInt("height") + "p " + nativeVideo.optString("codec") + (nativeVideo.optBoolean("hw") ? " (hardware)" : " (software)")));
        exo.setFill(fill);
        exo.setRate(preferredRate);
        exo.play(video, audio, positionMs, play, cover(id));
        applyLeveller();
    }

    // ------------------------------------------------------------------ sound tracks and captions

    /** The sound track chosen by name for dubbed videos ("" = the original). */
    String audioTrack = "";
    /** Stable volume: YouTube's loudness-levelled sound, or a compressor where there is none. */
    boolean stableVolume;
    /** YouTube history is paused: the app does not report what it plays either. */
    private boolean historyPaused() {
        return activity.getSharedPreferences("shell", Context.MODE_PRIVATE).getBoolean("historyPaused", false);
    }
    private android.media.audiofx.DynamicsProcessing leveller;
    /** The sound stream to play from a set of streams. */
    private JSONObject chooseAudio(JSONObject streams) {
        if (streams == null) {
            return null;
        }
        JSONArray tracks = streams.optJSONArray("audio_tracks");
        if (!audioTrack.isEmpty() && tracks != null) {
            for (int i = 0; i < tracks.length(); i++) {
                JSONObject t = tracks.optJSONObject(i);
                if (t != null && audioTrack.equals(t.optString("name"))) {
                    return t;
                }
            }
        }
        JSONObject drc = streams.optJSONObject("audio_drc");
        if (stableVolume && drc != null && audioTrack.isEmpty()) {
            return drc;
        }
        return streams.optJSONObject("audio");
    }

    /** With stable volume on and no levelled stream from YouTube, even out loud and quiet parts here. */
    private void applyLeveller() {
        boolean youtubeLevelled = nativeStreams != null && nativeStreams.optJSONObject("audio_drc") != null && audioTrack.isEmpty();
        boolean want = stableVolume && !youtubeLevelled;
        if (!want) {
            if (leveller != null) {
                leveller.release();
                leveller = null;
            }
            return;
        }
        if (leveller != null || android.os.Build.VERSION.SDK_INT < 28) {
            return;
        }
        try {
            android.media.audiofx.DynamicsProcessing.Config config = new android.media.audiofx.DynamicsProcessing.Config.Builder(
                    android.media.audiofx.DynamicsProcessing.VARIANT_FAVOR_FREQUENCY_RESOLUTION, 2, false, 0, true, 1, false, 0, true)
                    .build();
            android.media.audiofx.DynamicsProcessing.MbcBand band = config.getMbcBandByChannelIndex(0, 0);
            band.setThreshold(-30f);
            band.setRatio(4f);
            band.setAttackTime(15f);
            band.setReleaseTime(250f);
            band.setPostGain(9f);
            config.setMbcBandAllChannelsTo(0, band);
            android.media.audiofx.DynamicsProcessing.Limiter limiter = config.getLimiterByChannelIndex(0);
            limiter.setThreshold(-2f);
            config.setLimiterAllChannelsTo(limiter);
            leveller = new android.media.audiofx.DynamicsProcessing(0, exo.audioSession(), config);
            leveller.setEnabled(true);
        } catch (RuntimeException e) {
            Log.w(TAG, "stable volume: " + e);
            leveller = null;
        }
    }

    /** Caption tracks, preview pictures and chat of the current video (from the core's "extras"). */
    private JSONObject extras;
    private String extrasFor;
    /** The caption track in use (its index in the extras), or -1. */
    private int captionTrack = -1;
    /** The language the user last chose captions in ("" = off). */
    String preferredCaption = "";
    private double[] cueStart = new double[0], cueEnd = new double[0];
    private String[] cueText = new String[0];
    private int captionLoads;

    void setExtras(String id, JSONObject x) {
        if (id == null || !id.equals(currentId)) {
            return;
        }
        extras = x;
        extrasFor = id;
        if (!preferredCaption.isEmpty()) {
            JSONArray tracks = x.optJSONArray("captions");
            int pick = -1;
            for (int i = 0; tracks != null && i < tracks.length(); i++) {
                JSONObject t = tracks.optJSONObject(i);
                if (t == null || !preferredCaption.equals(t.optString("lang"))) {
                    continue;
                }
                // A written track beats the automatic one.
                if (pick < 0 || !t.optBoolean("auto")) {
                    pick = i;
                }
            }
            if (pick >= 0) {
                loadCaptions(pick);
            }
        }
    }

    private void clearCaptions() {
        captionTrack = -1;
        cueStart = new double[0];
        cueEnd = new double[0];
        cueText = new String[0];
        captionLoads++;
        if (progress != null) {
            progress.onCaption("");
        }
    }

    private void loadCaptions(final int index) {
        JSONArray tracks = extras == null ? null : extras.optJSONArray("captions");
        final JSONObject t = tracks == null ? null : tracks.optJSONObject(index);
        if (t == null) {
            clearCaptions();
            return;
        }
        final int mine = ++captionLoads;
        final String id = currentId;
        captionTrack = index;
        Core.get().call((e, err) -> {
            if (mine != captionLoads || id == null || !id.equals(currentId)) {
                return;
            }
            JSONObject ok = e == null ? null : e.optJSONObject("ok");
            JSONArray cues = ok == null ? null : ok.optJSONArray("cues");
            if (cues == null) {
                android.widget.Toast.makeText(activity, "Captions did not load" + (err == null ? "" : ": " + err), android.widget.Toast.LENGTH_SHORT).show();
                captionTrack = -1;
                return;
            }
            int n = cues.length();
            cueStart = new double[n];
            cueEnd = new double[n];
            cueText = new String[n];
            for (int i = 0; i < n; i++) {
                JSONObject c = cues.optJSONObject(i);
                cueStart[i] = c.optDouble("s");
                cueEnd[i] = c.optDouble("e");
                cueText[i] = c.optString("t");
            }
        }, "captions", "url", t.optString("url"), "agent", t.optString("agent"));
    }

    /** The caption lines showing at `at` seconds (automatic captions overlap; the latest two are kept). */
    private String captionAt(double at) {
        if (captionTrack < 0 || cueStart.length == 0) {
            return "";
        }
        StringBuilder sb = new StringBuilder();
        int shown = 0;
        int lo = 0, hi = cueStart.length - 1, last = -1;
        while (lo <= hi) {
            int mid = (lo + hi) >>> 1;
            if (cueStart[mid] <= at) {
                last = mid;
                lo = mid + 1;
            } else {
                hi = mid - 1;
            }
        }
        for (int i = last; i >= 0 && i > last - 6 && shown < 2; i--) {
            if (cueStart[i] <= at && at < cueEnd[i]) {
                sb.insert(0, sb.length() > 0 ? cueText[i] + "\n" : cueText[i]);
                shown++;
            }
        }
        return sb.toString();
    }

    /** What the settings sheet lists for the native player. */
    private JSONObject nativeSettings() {
        JSONObject o = new JSONObject();
        try {
            JSONArray qualities = new JSONArray();
            for (JSONObject f : nativeOptions) {
                int fps = f.optInt("fps", 30);
                qualities.put(new JSONObject().put("id", nameOf(f.optInt("height")))
                        .put("label", f.optInt("height") + "p" + (fps > 30 ? String.valueOf(fps) : "") + (f.optBoolean("hw") ? "" : " (software)")));
            }
            JSONArray rates = new JSONArray();
            for (double r : new double[] {0.25, 0.5, 0.75, 1, 1.25, 1.5, 1.75, 2}) {
                rates.put(r);
            }
            JSONArray captions = new JSONArray();
            JSONArray tracks = extras == null || !currentIdEquals(extrasFor) ? null : extras.optJSONArray("captions");
            for (int i = 0; tracks != null && i < tracks.length(); i++) {
                JSONObject t = tracks.optJSONObject(i);
                if (t != null) {
                    captions.put(new JSONObject().put("id", String.valueOf(i)).put("label", t.optString("name")));
                }
            }
            JSONArray sounds = new JSONArray();
            JSONArray dubbed = nativeStreams == null ? null : nativeStreams.optJSONArray("audio_tracks");
            for (int i = 0; dubbed != null && i < dubbed.length(); i++) {
                JSONObject t = dubbed.optJSONObject(i);
                if (t != null) {
                    sounds.put(new JSONObject().put("id", t.optBoolean("original") ? "" : t.optString("name")).put("label", t.optString("name")));
                }
            }
            o.put("type", "settings").put("qualities", qualities)
                    .put("quality", nativeVideo == null ? "" : nameOf(nativeVideo.optInt("height")))
                    .put("preferred", preferredQuality).put("rates", rates).put("rate", exo.rate())
                    .put("captions", captions).put("caption", captionTrack < 0 ? "" : String.valueOf(captionTrack))
                    .put("audio_tracks", sounds).put("audio_track", audioTrack)
                    .put("stable", stableVolume).put("live", exo.isLive());
        } catch (JSONException ignored) {
        }
        return o;
    }

    /** A command for the native player. False if it is not one of its commands. */
    private boolean nativeCommand(String what, Object arg) {
        double number = arg instanceof Number ? ((Number) arg).doubleValue() : 0;
        switch (what) {
            case "toggle":
                exo.setPlaying(!exo.wantsToPlay());
                return true;
            case "play":
                exo.setPlaying(true);
                return true;
            case "pause":
                exo.setPlaying(false);
                return true;
            case "stop":
                exo.stop();
                return true;
            case "seek":
                exo.seekTo(number);
                return true;
            case "seekby":
                exo.seekBy(number);
                return true;
            case "rate":
                exo.setRate(number > 0 ? number : 1);
                return true;
            case "fill":
                exo.setFill(number != 0);
                return true;
            case "settings":
                listener.onSettings(nativeSettings());
                return true;
            case "quality": {
                preferredQuality = String.valueOf(arg);
                JSONObject next = audioOnly || nativeStreams == null ? nativeVideo : pickVideo();
                if (next != nativeVideo) {
                    startNative(exo.positionMs(), exo.wantsToPlay());
                }
                return true;
            }
            case "audio":
                if (localFile != null) {
                    // The same file keeps playing; only the picture is covered or shown.
                    exo.showCover(audioOnly || localAudio ? cover(currentId) : null);
                } else if (nativeStreams != null) {
                    startNative(exo.positionMs(), exo.wantsToPlay());
                }
                return true;
            case "captions": {
                String which = String.valueOf(arg);
                if (which.isEmpty()) {
                    preferredCaption = "";
                    clearCaptions();
                } else {
                    try {
                        int index = Integer.parseInt(which);
                        JSONObject t = extras.optJSONArray("captions").optJSONObject(index);
                        preferredCaption = t.optString("lang");
                        loadCaptions(index);
                    } catch (RuntimeException e) {
                        clearCaptions();
                    }
                }
                return true;
            }
            case "audiotrack":
                audioTrack = arg == null ? "" : String.valueOf(arg);
                if (nativeStreams != null && localFile == null) {
                    startNative(exo.positionMs(), exo.wantsToPlay());
                }
                return true;
            case "stable":
                stableVolume = number != 0;
                if (nativeStreams != null && localFile == null && nativeStreams.optJSONObject("audio_drc") != null) {
                    startNative(exo.positionMs(), exo.wantsToPlay());
                } else {
                    applyLeveller();
                }
                return true;
            case "golive":
                exo.goLive();
                return true;
            case "compact":
            case "shape":
                return true;
            default:
                return false;
        }
    }

    /** Google refuses to sign in inside an app's web view; present a plain Chrome instead. */
    static String chromeUserAgent(String webViewAgent) {
        return webViewAgent.replace("; wv", "").replaceAll("Version/\\d+\\.\\d+ ", "");
    }

    /** A YouTube page with the page script is loaded and can switch videos by itself. */
    private boolean pageReady;
    /** The video the page itself last reported. */
    private String pageVideoId;

    // ------------------------------------------------------------------ direct playback

    static final String DIRECT_HOST = "app.streampipe";
    private static final String DIRECT_PAGE = "https://" + DIRECT_HOST + "/player.html";

    /** Play stream URLs with the app's own page; otherwise YouTube's watch page. */
    boolean direct = true;
    /** The app's own player page is loaded. */
    private boolean directReady;
    /** What to hand the page once it is loaded. */
    private JSONObject directPending;
    /** The video now playing came through the direct player. */
    boolean playingDirect;

    /** One stream the page may fetch: its addresses and the browser name they expect. */
    private static final class Source {
        final String[] urls;
        final String agent;
        /** The address that last worked, tried first. */
        volatile int good;

        Source(String[] urls, String agent) {
            this.urls = urls;
            this.agent = agent;
        }
    }

    private final Map<String, Source> sources = new java.util.concurrent.ConcurrentHashMap<>();

    void setDirect(boolean on) {
        direct = on;
    }

    /** Where the native player is, in whole seconds (0 for YouTube's page). */
    int positionSeconds() {
        return playingDirect ? (int) (exo.positionMs() / 1000) : 0;
    }

    /** The picture's average colour, for the glow under the player (0 when unknown). */
    int frameColor() {
        return playingDirect && !audioOnly ? exo.frameColor() : 0;
    }

    private boolean currentIdEquals(String id) {
        return id != null && id.equals(currentId);
    }

    private void openDirect(final String id) {
        localFile = null;
        // Captions and sound choices belong to one video; the preferred language carries on.
        clearCaptions();
        extras = null;
        playingDirect = true;
        tellDirect(true);
        directPending = null;
        nativeStreams = null;
        nativeVideo = null;
        exo.stop();
        // Whatever the web view was playing stops; the native player takes the screen.
        pageCommand("stop");
        showNative(true);
        final long started = System.currentTimeMillis();
        Core.get().call((e, err) -> {
            if (!id.equals(currentId) || !playingDirect || localFile != null) {
                return;
            }
            JSONObject streams = err == null && e != null ? e.optJSONObject("ok") : null;
            if (streams == null || streams.optJSONObject("audio") == null) {
                Log.i(TAG, "direct: no streams for " + id + " (" + err + "), using the YouTube page");
                openPage(id);
                return;
            }
            Log.i(TAG, "direct: streams for " + id + " in " + (System.currentTimeMillis() - started) + " ms");
            nativeStreams = streams;
            // Carry on where it was left, as YouTube does.
            startNative(Math.round(Resume.startAt(id) * 1000), true);
        }, "streams", "id", id);
    }

    /**
     * Replace the addresses of the streams in use. False if the new set lacks one of them
     * (then the player has to start again on the new set).
     */
    private boolean updateSources(JSONObject streams) {
        if (streams == null || sources.isEmpty()) {
            return false;
        }
        Map<String, Source> fresh = new HashMap<>();
        try {
            JSONArray list = streams.optJSONArray("video");
            for (int i = 0; list != null && i <= list.length(); i++) {
                JSONObject f = i < list.length() ? list.getJSONObject(i) : streams.optJSONObject("audio");
                JSONArray urls = f == null ? null : f.optJSONArray("urls");
                if (urls == null || urls.length() == 0) {
                    continue;
                }
                String[] all = new String[urls.length()];
                for (int u = 0; u < all.length; u++) {
                    all[u] = urls.getString(u);
                }
                fresh.put(f.optString("kind") + f.optInt("itag"), new Source(all, f.isNull("agent") ? userAgent : f.optString("agent")));
            }
        } catch (JSONException e) {
            return false;
        }
        if (!fresh.keySet().containsAll(sources.keySet())) {
            return false;
        }
        sources.putAll(fresh);
        return true;
    }

    /** What the page needs to play: formats named by key, with the addresses kept here. */
    private JSONObject directData(String id, JSONObject streams) {
        if (streams == null || streams.optJSONObject("audio") == null) {
            return null;
        }
        try {
            sources.clear();
            JSONArray video = new JSONArray();
            JSONArray list = streams.optJSONArray("video");
            for (int i = 0; list != null && i < list.length(); i++) {
                JSONObject f = keyed(list.getJSONObject(i));
                if (f != null) {
                    video.put(f);
                }
            }
            JSONObject audio = keyed(streams.getJSONObject("audio"));
            if (audio == null) {
                return null;
            }
            return new JSONObject().put("id", id).put("duration", streams.optInt("duration"))
                    .put("video", video).put("audio", audio)
                    .put("quality", preferredQuality).put("rate", preferredRate).put("audioOnly", audioOnly);
        } catch (JSONException e) {
            return null;
        }
    }

    private JSONObject keyed(JSONObject f) throws JSONException {
        JSONArray urls = f.optJSONArray("urls");
        if (urls == null || urls.length() == 0) {
            return null;
        }
        String[] list = new String[urls.length()];
        for (int i = 0; i < list.length; i++) {
            list[i] = urls.getString(i);
        }
        String key = f.optString("kind") + f.optInt("itag");
        sources.put(key, new Source(list, f.isNull("agent") ? userAgent : f.optString("agent")));
        f.remove("urls");
        f.remove("agent");
        return f.put("key", key);
    }

    private void loadDirect(JSONObject data) {
        web.evaluateJavascript("window.__sp&&window.__sp.load(" + data + ")", null);
    }

    /** One byte range of a stream, for the player page. */
    private WebResourceResponse stream(Uri uri) {
        String key = uri.getLastPathSegment();
        String range = uri.getQueryParameter("range");
        Source src = key == null ? null : sources.get(key);
        if (src == null || range == null || !range.matches("\\d+-\\d+")) {
            return plain(404, "Not Found");
        }
        int first = src.good;
        for (int n = 0; n < src.urls.length; n++) {
            int i = (first + n) % src.urls.length;
            HttpURLConnection c = null;
            try {
                boolean local = src.urls[i].startsWith("http://127.0.0.1:");
                c = (HttpURLConnection) new URL(local ? src.urls[i] : src.urls[i] + "&range=" + range).openConnection();
                if (local) {
                    c.setRequestProperty("Range", "bytes=" + range);
                }
                c.setConnectTimeout(8000);
                c.setReadTimeout(20000);
                c.setRequestProperty("User-Agent", src.agent);
                int status = c.getResponseCode();
                if (status == 200 || status == 206) {
                    src.good = i;
                    final HttpURLConnection open = c;
                    InputStream body = new java.io.FilterInputStream(c.getInputStream()) {
                        @Override
                        public void close() throws IOException {
                            super.close();
                            open.disconnect();
                        }
                    };
                    return new WebResourceResponse("application/octet-stream", null, 200, "OK", new HashMap<>(), body);
                }
                Log.w(TAG, "direct: stream " + key + " answered " + status + " on address " + i);
                c.disconnect();
                if (status == 403) {
                    // The address expired or was refused: another host will say the same.
                    return plain(403, "Forbidden");
                }
            } catch (IOException e) {
                Log.w(TAG, "direct: stream " + key + " address " + i + ": " + e);
                if (c != null) {
                    c.disconnect();
                }
            }
        }
        return plain(502, "Bad Gateway");
    }

    private static WebResourceResponse plain(int status, String reason) {
        return new WebResourceResponse("text/plain", "utf-8", status, reason, new HashMap<>(),
                new ByteArrayInputStream(new byte[0]));
    }

    /** The downloaded file being played, or null. */
    private volatile Uri localFile;
    private volatile String localMime = "video/mp4";
    private volatile int localSerial;

    /** Play a downloaded file in the app's player. `audio`: the file has sound only. */
    void openLocal(String id, Uri file, String mime, boolean audio) {
        if (!VIDEO_ID.matcher(id).matches()) {
            return;
        }
        currentId = id;
        playing = false;
        playingDirect = true;
        tellDirect(true);
        sources.clear();
        localFile = file;
        localMime = mime;
        localAudio = audio;
        nativeStreams = null;
        nativeVideo = null;
        nativeOptions = new java.util.ArrayList<>();
        pageCommand("stop");
        showNative(true);
        Log.i(TAG, "native player: playing the downloaded file of " + id);
        exo.setFill(fill);
        exo.setRate(preferredRate);
        exo.playFile(file, Math.round(Resume.startAt(id) * 1000), true, audio || audioOnly ? cover(id) : null);
        if (exo.isActive()) {
            return;
        }
        JSONObject data;
        try {
            if (fileServer == null) {
                fileServer = new FileServer();
            }
            // A new address for every file, so the web view never reuses another file's bytes.
            data = new JSONObject().put("id", id).put("local", true).put("audioFile", audio)
                    .put("url", fileServer.address(++localSerial))
                    .put("quality", preferredQuality).put("rate", preferredRate).put("audioOnly", audioOnly);
        } catch (JSONException | IOException e) {
            Log.w(TAG, "direct: downloaded file: " + e);
            return;
        }
        Log.i(TAG, "direct: playing the downloaded file of " + id + " from " + data.optString("url").replaceAll("/f/.*", "/f/…"));
        if (directReady) {
            directPending = null;
            loadDirect(data);
        } else {
            directPending = data;
            web.loadUrl(DIRECT_PAGE);
        }
    }

    /**
     * Serves the downloaded file being played to the player page, on this phone only
     * (127.0.0.1), with proper byte ranges: the player seeks by asking for ranges, and a
     * finished MP4 keeps its index at the end of the file.
     */
    private final class FileServer implements Runnable {
        private final java.net.ServerSocket socket;
        /** Part of every address, so no other app on the phone can ask for the file. */
        final String token = Long.toHexString(new java.security.SecureRandom().nextLong());

        FileServer() throws IOException {
            socket = new java.net.ServerSocket(0, 16, java.net.InetAddress.getByAddress(new byte[] {127, 0, 0, 1}));
            Thread t = new Thread(this, "streampipe-files");
            t.setDaemon(true);
            t.start();
        }

        String address(int serial) {
            return "http://127.0.0.1:" + socket.getLocalPort() + "/f/" + token + "/" + serial;
        }

        @Override
        public void run() {
            while (!socket.isClosed()) {
                try {
                    final java.net.Socket client = socket.accept();
                    Thread t = new Thread(() -> serve(client), "streampipe-file");
                    t.setDaemon(true);
                    t.start();
                } catch (IOException e) {
                    return;
                }
            }
        }

        private void serve(java.net.Socket client) {
            try (java.net.Socket c = client) {
                c.setSoTimeout(30000);
                java.io.BufferedReader in = new java.io.BufferedReader(
                        new java.io.InputStreamReader(c.getInputStream(), StandardCharsets.ISO_8859_1));
                String first = in.readLine();
                if (first == null) {
                    return;
                }
                String[] parts = first.split(" ");
                String range = null;
                for (String line; (line = in.readLine()) != null && !line.isEmpty(); ) {
                    if (line.regionMatches(true, 0, "range:", 0, 6)) {
                        range = line.substring(6).trim();
                    }
                }
                java.io.OutputStream out = c.getOutputStream();
                Uri source = localFile;
                boolean head = parts.length > 0 && parts[0].equals("HEAD");
                if (parts.length < 2 || !(head || parts[0].equals("GET")) || source == null
                        || !parts[1].equals("/f/" + token + "/" + localSerial)) {
                    out.write("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".getBytes(StandardCharsets.ISO_8859_1));
                    return;
                }
                android.os.ParcelFileDescriptor fd = activity.getContentResolver().openFileDescriptor(source, "r");
                if (fd == null) {
                    out.write("HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".getBytes(StandardCharsets.ISO_8859_1));
                    return;
                }
                try (FileInputStream file = new android.os.ParcelFileDescriptor.AutoCloseInputStream(fd)) {
                    long size = file.getChannel().size();
                    long from = 0, to = size - 1;
                    Matcher m = range == null ? null : Pattern.compile("bytes=(\\d*)-(\\d*)").matcher(range);
                    boolean partial = m != null && m.find() && !(m.group(1).isEmpty() && m.group(2).isEmpty());
                    if (partial) {
                        if (m.group(1).isEmpty()) {
                            from = Math.max(0, size - Long.parseLong(m.group(2)));
                        } else {
                            from = Long.parseLong(m.group(1));
                            if (!m.group(2).isEmpty()) {
                                to = Math.min(size - 1, Long.parseLong(m.group(2)));
                            }
                        }
                    }
                    if (from > to || from >= size) {
                        out.write(("HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */" + size
                                + "\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").getBytes(StandardCharsets.ISO_8859_1));
                        return;
                    }
                    long length = to - from + 1;
                    Log.i(TAG, "direct: downloaded file " + from + "-" + to + " of " + size);
                    String headers = (partial ? "HTTP/1.1 206 Partial Content\r\n" : "HTTP/1.1 200 OK\r\n")
                            + "Content-Type: " + localMime + "\r\n"
                            + "Accept-Ranges: bytes\r\n"
                            + (partial ? "Content-Range: bytes " + from + "-" + to + "/" + size + "\r\n" : "")
                            + "Content-Length: " + length + "\r\n"
                            + "Access-Control-Allow-Origin: *\r\n"
                            + "Connection: close\r\n\r\n";
                    out.write(headers.getBytes(StandardCharsets.ISO_8859_1));
                    if (head) {
                        return;
                    }
                    file.getChannel().position(from);
                    byte[] buf = new byte[1 << 16];
                    while (length > 0) {
                        int n = file.read(buf, 0, (int) Math.min(buf.length, length));
                        if (n <= 0) {
                            break;
                        }
                        out.write(buf, 0, n);
                        length -= n;
                    }
                    out.flush();
                }
            } catch (Exception e) {
                // The player closes connections whenever it seeks; that is not an error.
            }
        }
    }

    private FileServer fileServer;

    /** The app's own pages: the player, stream bytes, and downloaded files. */
    private WebResourceResponse own(WebResourceRequest request) {
        Uri uri = request.getUrl();
        String path = uri.getPath();
        if (path != null && path.startsWith("/s/")) {
            return stream(uri);
        }

        if ("/player.html".equals(path)) {
            try {
                return new WebResourceResponse("text/html", "utf-8", activity.getAssets().open("player.html"));
            } catch (IOException e) {
                return plain(500, "Missing");
            }
        }
        return plain(404, "Not Found");
    }

    // ------------------------------------------------------------------ opening

    void open(final String id) {
        if (!VIDEO_ID.matcher(id).matches()) {
            return;
        }
        currentId = id;
        playing = false;
        web.onResume();
        if (direct) {
            openDirect(id);
        } else {
            openPage(id);
        }
    }

    /** Play through YouTube's watch page. */
    private void openPage(final String id) {
        playingDirect = false;
        tellDirect(false);
        directPending = null;
        localFile = null;
        nativeStreams = null;
        exo.stop();
        showNative(false);
        web.onResume();
        if (!pageReady) {
            web.loadUrl("https://m.youtube.com/watch?v=" + id);
            return;
        }
        // Fast path: the loaded page switches videos without reloading.
        command("open", id);
        main.postDelayed(() -> {
            // If the page did not follow (it was discarded, or its router changed), load it.
            if (id.equals(currentId) && !id.equals(pageVideoId)) {
                Log.i(TAG, "app: in-page switch did not happen, loading the page");
                pageReady = false;
                web.loadUrl("https://m.youtube.com/watch?v=" + id);
            }
        }, 4000);
    }

    /** Stop playback but keep the page, so the next video starts without a page load. */
    void close() {
        currentId = null;
        playing = false;
        localFile = null;
        directPending = null;
        tellDirect(false);
        sources.clear();
        nativeStreams = null;
        exo.stop();
        if (pageReady || directReady) {
            command("stop");
        } else {
            web.loadUrl("about:blank");
        }
    }

    /** Load YouTube once in the background so the first video can use the fast path too. */
    void warmUp() {
        if (!direct && !pageReady && currentId == null) {
            web.loadUrl("https://m.youtube.com/");
        }
    }

    private boolean compact;

    /** Small player (mini bar or picture-in-picture): hide YouTube's controls and overlays. */
    void setCompact(boolean on) {
        if (compact != on) {
            compact = on;
            applyCompact();
        }
    }

    private void applyCompact() {
        web.evaluateJavascript("window.__sp&&window.__sp.cmd('compact'," + compact + ")", null);
    }

    /** The user's standing choices, applied to every video when it starts. */
    String preferredQuality = "auto";
    double preferredRate = 1.0;
    private String preferencesAppliedTo;

    /** A page command with one argument: "quality", "rate" or "captions". */
    void command(String what, Object arg) {
        if (playingDirect && nativeCommand(what, arg)) {
            return;
        }
        String a = arg instanceof Number ? String.valueOf(arg) : JSONObject.quote(String.valueOf(arg));
        web.evaluateJavascript("window.__sp&&window.__sp.cmd(" + JSONObject.quote(what) + "," + a + ")", null);
    }

    /** Zoom to fill: the picture covers the view, cropped at the edges (fullscreen only). */
    private boolean fill;

    void setFill(boolean on) {
        if (fill != on) {
            fill = on;
            command("fill", on ? 1 : 0);
        }
    }

    /** Audio only: lowest picture quality, hidden behind the thumbnail. */
    boolean audioOnly;

    void setAudioOnly(boolean on) {
        audioOnly = on;
        command("audio", on ? 1 : 0);
    }

    private void applyPreferences() {
        if (currentId == null || currentId.equals(preferencesAppliedTo)) {
            return;
        }
        preferencesAppliedTo = currentId;
        if (audioOnly) {
            command("audio", 1);
        }
        if (!"auto".equals(preferredQuality)) {
            command("quality", preferredQuality);
        }
        if (Math.abs(preferredRate - 1.0) > 0.001) {
            command("rate", preferredRate);
        }
    }

    /** "play", "pause" or "toggle". */
    void command(String what) {
        Log.i(TAG, "app: command " + what);
        if (playingDirect && nativeCommand(what, null)) {
            return;
        }
        web.evaluateJavascript("window.__sp&&window.__sp.cmd(" + JSONObject.quote(what) + ")", null);
    }

    /** Tell the page its size is about to change, so a pause caused by that is undone. */
    void shapeChanging() {
        web.evaluateJavascript("window.__sp&&window.__sp.cmd('shape')", null);
    }

    /** Refresh the text shown in Settings from the rules file. */
    void readRulesInfo() {
        new Thread(() -> script(WATCH_OPTIONS), "streampipe-rules").start();
    }

    // ------------------------------------------------------------------ page script

    private static byte[] readAll(InputStream in) throws IOException {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        byte[] buf = new byte[16384];
        int n;
        while ((n = in.read(buf)) > 0) {
            out.write(buf, 0, n);
        }
        return out.toByteArray();
    }

    /** The page script with the current rules filled in. */
    private static final String WATCH_OPTIONS = "{\"watchOnly\":true,\"background\":true}";

    /** A page client that injects the page script with other options (the Shorts feed). */
    WebViewClient newPageClient(String optionsJson) {
        return new Client(optionsJson);
    }

    String userAgent() {
        return userAgent;
    }

    private synchronized String script(String options) {
        if (scriptSource == null) {
            try (InputStream in = activity.getAssets().open("player-inject.js")) {
                scriptSource = new String(readAll(in), StandardCharsets.UTF_8);
            } catch (IOException e) {
                scriptSource = "";
            }
        }
        String rules = "[]";
        try {
            File f = new File(Native.rulesFile(Core.get().cacheDir));
            if (f.isFile()) {
                try (InputStream in = new FileInputStream(f)) {
                    JSONObject set = new JSONObject(new String(readAll(in), StandardCharsets.UTF_8));
                    JSONArray list = set.optJSONArray("rules");
                    if (list != null) {
                        rules = list.toString();
                        long at = set.optLong("fetched_at", 0);
                        adRulesText = list.length() + " rules from uBlock Origin, "
                                + (at > 0 ? "updated " + android.text.format.DateFormat.format("d MMM HH:mm", at * 1000L) : "built-in set");
                    }
                }
            }
        } catch (Exception e) {
            Log.w(TAG, "ad rules: " + e);
        }
        // The script sits inside a <script> element: nothing in it may close that element.
        rules = rules.replace("</", "<\\/");
        return scriptSource.replace("__RULES__", rules).replace("__OPTS__", options);
    }

    private static boolean isYouTube(String host) {
        return host != null && (host.equals("youtube.com") || host.endsWith(".youtube.com"));
    }

    private final class Client extends WebViewClient {
        private final String options;
        /** The watch page shows only the player, so what the rest of the page fetches is waste. */
        private final boolean playerOnly;
        private int blocked;

        Client(String options) {
            this.options = options;
            this.playerOnly = options.contains("watchOnly");
        }

        /**
         * Requests the player-only page does not need: thumbnails of recommended videos,
         * channel avatars, and advertising and ad-tracking hosts. Storyboards (seek previews)
         * and the player's own files are not touched, nor are watch-history reports.
         */
        private boolean isWaste(Uri uri) {
            String host = uri.getHost();
            String path = uri.getPath();
            if (host == null || path == null) {
                return false;
            }
            if (host.endsWith("ytimg.com")) {
                // Video thumbnails. The audio-only background is marked by the page script.
                return path.startsWith("/vi/") || path.startsWith("/vi_webp/") || path.startsWith("/an_webp/")
                        ? !"1".equals(uri.getQueryParameter("sp")) : false;
            }
            return host.endsWith("ggpht.com") || host.endsWith("doubleclick.net")
                    || host.endsWith("googlesyndication.com") || host.endsWith("googleadservices.com")
                    || path.startsWith("/pagead/") || path.startsWith("/api/stats/ads");
        }

        /** Fetch YouTube documents here and put the page script first in `<head>`. */
        @Override
        public WebResourceResponse shouldInterceptRequest(WebView view, WebResourceRequest request) {
            if (DIRECT_HOST.equals(request.getUrl().getHost())) {
                return own(request);
            }
            if (playerOnly && !request.isForMainFrame() && isWaste(request.getUrl())) {
                if (++blocked % 50 == 1) {
                    Log.i(TAG, "player page: " + blocked + " unneeded requests blocked so far");
                }
                return new WebResourceResponse("text/plain", "utf-8", 204, "No Content", new HashMap<>(),
                        new ByteArrayInputStream(new byte[0]));
            }
            return intercept(request, 0);
        }

        /**
         * One failed fetch is retried: if the page loads directly instead, the script only
         * arrives late (see {@link #inject}), after YouTube's own code has started.
         */
        private WebResourceResponse intercept(WebResourceRequest request, int attempt) {
            Uri uri = request.getUrl();
            if (!request.isForMainFrame() || !"GET".equals(request.getMethod())
                    || !"https".equals(uri.getScheme()) || !isYouTube(uri.getHost())) {
                return null;
            }
            String url = uri.toString();
            HttpURLConnection c = null;
            try {
                c = (HttpURLConnection) new URL(url).openConnection();
                c.setInstanceFollowRedirects(false);
                c.setConnectTimeout(15000);
                c.setReadTimeout(30000);
                for (Map.Entry<String, String> h : request.getRequestHeaders().entrySet()) {
                    String k = h.getKey().toLowerCase(Locale.ROOT);
                    // Let the connection negotiate (and undo) compression itself.
                    if (!k.equals("accept-encoding") && !k.equals("cookie") && !k.equals("user-agent")) {
                        c.setRequestProperty(h.getKey(), h.getValue());
                    }
                }
                c.setRequestProperty("User-Agent", userAgent);
                String cookie = CookieManager.getInstance().getCookie(url);
                if (cookie != null) {
                    c.setRequestProperty("Cookie", cookie);
                }
                int status = c.getResponseCode();
                Map<String, String> headers = new HashMap<>();
                for (Map.Entry<String, List<String>> h : c.getHeaderFields().entrySet()) {
                    if (h.getKey() == null || h.getValue() == null) {
                        continue;
                    }
                    String k = h.getKey().toLowerCase(Locale.ROOT);
                    if (k.equals("set-cookie")) {
                        for (String v : h.getValue()) {
                            CookieManager.getInstance().setCookie(url, v);
                        }
                    } else if (!k.equals("content-encoding") && !k.equals("content-length") && !k.equals("transfer-encoding")
                            // The page's script policy would reject the injected script.
                            && !k.startsWith("content-security-policy")) {
                        headers.put(h.getKey(), h.getValue().get(h.getValue().size() - 1));
                    }
                }
                if (status >= 300 && status < 400) {
                    String to = c.getHeaderField("Location");
                    if (to == null) {
                        return null;
                    }
                    String target = new URL(new URL(url), to).toString();
                    String html = "<!doctype html><script>location.replace(" + JSONObject.quote(target) + ")</script>";
                    return new WebResourceResponse("text/html", "utf-8", 200, "OK", new HashMap<>(),
                            new ByteArrayInputStream(html.getBytes(StandardCharsets.UTF_8)));
                }
                String type = c.getContentType();
                InputStream body = status >= 400 ? c.getErrorStream() : c.getInputStream();
                byte[] bytes = body == null ? new byte[0] : readAll(body);
                if (type != null && type.toLowerCase(Locale.ROOT).contains("text/html")) {
                    String html = new String(bytes, StandardCharsets.UTF_8);
                    String tag = "<script>" + script(options) + "</script>";
                    Matcher m = HEAD_TAG.matcher(html);
                    html = m.find() ? html.substring(0, m.end()) + tag + html.substring(m.end()) : tag + html;
                    bytes = html.getBytes(StandardCharsets.UTF_8);
                    type = "text/html";
                } else if (type != null && type.contains(";")) {
                    type = type.substring(0, type.indexOf(';')).trim();
                }
                String reason = c.getResponseMessage();
                return new WebResourceResponse(type == null ? "text/html" : type, "utf-8", status,
                        reason == null || reason.isEmpty() ? "OK" : reason, headers, new ByteArrayInputStream(bytes));
            } catch (Exception e) {
                if (attempt == 0) {
                    Log.w(TAG, "page fetch failed, trying once more: " + e);
                    if (c != null) {
                        c.disconnect();
                        c = null;
                    }
                    return intercept(request, 1);
                }
                Log.w(TAG, "page fetch failed twice, loading directly: " + e);
                return null;
            } finally {
                if (c != null) {
                    c.disconnect();
                }
            }
        }

        /**
         * Late injection for a page that did not come through the fetch above. The script
         * ignores a second run, so calling this on every page is harmless.
         */
        private void inject(WebView view, String url) {
            if (url != null && isYouTube(Uri.parse(url).getHost())) {
                view.evaluateJavascript(script(options), null);
            }
        }

        @Override
        public void onPageCommitVisible(WebView view, String url) {
            inject(view, url);
        }

        @Override
        public void onPageFinished(WebView view, String url) {
            inject(view, url);
        }

        @Override
        public boolean shouldOverrideUrlLoading(WebView view, WebResourceRequest request) {
            String scheme = request.getUrl().getScheme();
            // No "open in the YouTube app" hand-offs and no other app links.
            return !"https".equals(scheme) && !"http".equals(scheme) && !"about".equals(scheme);
        }

        @Override
        public void onPageStarted(WebView view, String url, Bitmap favicon) {
            // A new document: until its script reports in, the fast path is not available.
            main.post(() -> {
                pageReady = false;
                directReady = false;
                pageVideoId = null;
            });
            inject(view, url);
        }
    }

    private final class Chrome extends WebChromeClient {
        private Bitmap blank;

        /**
         * The page asks for its own fullscreen. That would move the video into a separate
         * view (and pause it on the way back), so it is declined and the app makes the
         * player fill the screen itself. Every press of the button therefore toggles.
         */
        @Override
        public void onShowCustomView(View view, final CustomViewCallback callback) {
            main.post(() -> {
                callback.onCustomViewHidden();
                listener.onFullscreenToggle();
            });
        }

        /** Without this the web view draws a large grey play icon before a video starts. */
        @Override
        public Bitmap getDefaultVideoPoster() {
            if (blank == null) {
                blank = Bitmap.createBitmap(1, 1, Bitmap.Config.ARGB_8888);
            }
            return blank;
        }
    }

    private final class Bridge {
        private int stateLogs;

        /** Called by the page script inside YouTube's page: treat everything as untrusted. */
        @JavascriptInterface
        public void post(String json) {
            final JSONObject o;
            try {
                o = new JSONObject(json);
            } catch (JSONException e) {
                return;
            }
            final String id = o.optString("id");
            final boolean validId = VIDEO_ID.matcher(id).matches();
            switch (o.optString("type")) {
                case "loaded":
                    Log.i(TAG, "player page: " + abbreviate(json));
                    // A new page starts without the compact and audio-only flags.
                    final boolean own = o.optBoolean("direct");
                    main.post(() -> {
                        if (own) {
                            directReady = true;
                            if (directPending != null) {
                                loadDirect(directPending);
                                directPending = null;
                            }
                        } else {
                            pageReady = true;
                        }
                        applyCompact();
                        if (fill) {
                            command("fill", 1);
                        }
                        if (audioOnly) {
                            command("audio", 1);
                        }
                    });
                    break;
                case "video":
                    if (validId) {
                        main.post(() -> {
                            pageVideoId = id;
                            if (currentId != null && !id.equals(currentId)) {
                                currentId = id;
                                listener.onVideo(id);
                            }
                        });
                    }
                    break;
                case "ended":
                    if (validId) {
                        main.post(() -> {
                            if (id.equals(currentId)) {
                                listener.onEnded(id);
                            }
                        });
                    }
                    break;
                case "playing": {
                    final boolean now = o.optBoolean("playing");
                    main.post(() -> {
                        if (currentId != null && playing != now) {
                            playing = now;
                            if (now) {
                                applyPreferences();
                            }
                            listener.onPlaying(now);
                        }
                    });
                    break;
                }
                case "time":
                    main.post(() -> {
                        if (progress != null && playingDirect && currentId != null) {
                            progress.onProgress(o.optDouble("t", 0), o.optDouble("d", 0), o.optDouble("b", 0),
                                    o.optBoolean("paused"), o.optBoolean("waiting"));
                        }
                    });
                    break;
                case "error":
                    // The direct player gave up on this video: YouTube's page plays it.
                    Log.w(TAG, "direct: " + abbreviate(json));
                    if (validId) {
                        main.post(() -> {
                            if (id.equals(currentId) && playingDirect) {
                                if (localFile != null) {
                                    // Say so: otherwise the video would quietly start streaming.
                                    android.widget.Toast.makeText(activity, "The downloaded file did not play ("
                                            + o.optString("text") + "). Streaming it instead.", android.widget.Toast.LENGTH_LONG).show();
                                    openDirect(id);
                                } else {
                                    openPage(id);
                                }
                            }
                        });
                    }
                    break;
                case "settings":
                    main.post(() -> {
                        if (currentId != null) {
                            listener.onSettings(o);
                        }
                    });
                    break;
                case "log":
                    Log.i(TAG, "page: " + abbreviate(o.optString("text")));
                    break;
                case "state":
                    // A line every 10 s is enough to diagnose playback and ads.
                    if (stateLogs++ % 5 == 0 || o.optBoolean("ad")) {
                        Log.i(TAG, "player state: " + abbreviate(json));
                    }
                    break;
                default:
            }
        }
    }

    private static String abbreviate(String s) {
        return s.length() > 600 ? s.substring(0, 600) + "…" : s;
    }
}
