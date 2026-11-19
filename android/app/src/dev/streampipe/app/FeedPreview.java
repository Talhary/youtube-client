package dev.streampipe.app;

import android.content.Context;
import android.os.Handler;
import android.os.Looper;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.TextView;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.List;

/**
 * "Playback in feeds", as in YouTube's app: the video card most in view (once scrolling
 * stops), or the one under the finger, plays inside its thumbnail, with sound unless muted.
 * One shared native player for every list. Settings: "Playback in feeds" (always, Wi-Fi only,
 * off) and whether it has sound.
 */
final class FeedPreview implements Exo.Events {
    private static FeedPreview instance;

    static FeedPreview get(Context c) {
        if (instance == null) {
            instance = new FeedPreview(c.getApplicationContext());
        }
        return instance;
    }

    /** Previews may play now (false while a video is open in the watch player). */
    static java.util.function.BooleanSupplier allowed = () -> true;

    private final Context app;
    private final Handler main = new Handler(Looper.getMainLooper());
    private Exo exo;
    private FrameLayout holder;
    private FrameLayout host;
    private String playing;
    private int generation;
    private TextView time;
    private ImageView sound;

    private FeedPreview(Context app) {
        this.app = app;
    }

    private android.content.SharedPreferences prefs() {
        return app.getSharedPreferences("shell", Context.MODE_PRIVATE);
    }

    /** "always", "wifi" or "off". */
    static String mode(Context c) {
        return c.getSharedPreferences("shell", Context.MODE_PRIVATE).getString("feedPreview", "always");
    }

    boolean muted() {
        return prefs().getBoolean("feedPreviewMuted", false);
    }

    private boolean enabled() {
        String m = mode(app);
        if ("off".equals(m) || !allowed.getAsBoolean()) {
            return false;
        }
        if ("wifi".equals(m)) {
            android.net.ConnectivityManager cm = (android.net.ConnectivityManager) app.getSystemService(Context.CONNECTIVITY_SERVICE);
            return cm != null && !cm.isActiveNetworkMetered();
        }
        return true;
    }

    /** Play `video` inside `thumb` (the card's 16:9 frame). Ignored if it is already playing. */
    void play(final FrameLayout thumb, final Core.Item video) {
        if (video == null || video.type != Core.Item.VIDEO || video.live || video.localUri != null || !enabled()) {
            return;
        }
        if (video.id.equals(playing) && host == thumb) {
            return;
        }
        stop();
        if (exo == null) {
            exo = new Exo(app, this);
            exo.setFill(true);
            holder = new FrameLayout(app);
            holder.addView(exo.view(), new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));
            time = Ui.text(app, 11, 0xFFFFFFFF, true);
            time.setPadding(Ui.dp(5), Ui.dp(2), Ui.dp(5), Ui.dp(2));
            time.setBackground(Ui.rounded(0xCC000000, 4));
            FrameLayout.LayoutParams tl = Ui.frame(Ui.WRAP, Ui.WRAP, Gravity.BOTTOM | Gravity.END);
            tl.setMargins(0, 0, Ui.dp(8), Ui.dp(8));
            holder.addView(time, tl);
            sound = new ImageView(app);
            sound.setScaleType(ImageView.ScaleType.CENTER);
            sound.setBackground(Ui.rounded(0x99000000, 16));
            sound.setOnClickListener(v -> {
                boolean m = !muted();
                prefs().edit().putBoolean("feedPreviewMuted", m).apply();
                exo.setVolume(m ? 0f : 1f);
                paintSound();
            });
            FrameLayout.LayoutParams sl = Ui.frame(Ui.dp(32), Ui.dp(32), Gravity.TOP | Gravity.END);
            sl.setMargins(0, Ui.dp(8), Ui.dp(8), 0);
            holder.addView(sound, sl);
        }
        playing = video.id;
        host = thumb;
        holder.setAlpha(0f);
        thumb.addView(holder, new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));
        paintSound();
        final int mine = ++generation;
        Core.get().call((e, err) -> {
            JSONObject st = e == null ? null : e.optJSONObject("ok");
            if (mine != generation || st == null) {
                return;
            }
            JSONObject v = pick(st);
            JSONObject a = st.optJSONObject("audio");
            String vu = first(v), au = first(a);
            if (vu == null || au == null) {
                stop();
                return;
            }
            exo.setVolume(muted() ? 0f : 1f);
            exo.play(vu, au, Math.round(Resume.startAt(video.id) * 1000), true, null);
        }, "streams", "id", video.id);
    }

    private void paintSound() {
        boolean m = muted();
        sound.setImageResource(m ? R.drawable.ic_volume_off : R.drawable.ic_volume);
        sound.setImageTintList(android.content.res.ColorStateList.valueOf(0xFFFFFFFF));
        sound.setContentDescription(m ? "Turn sound on" : "Mute");
    }

    /** A small picture is enough inside a card: at most 480p, hardware-decoded first. */
    private static JSONObject pick(JSONObject st) {
        List<JSONObject> opts = Player.options(st);
        JSONObject first = null;
        for (JSONObject f : opts) {
            if (f.optInt("height") > 480) {
                continue;
            }
            if (first == null) {
                first = f;
            }
            if (f.optBoolean("hw")) {
                return f;
            }
        }
        return first != null ? first : opts.isEmpty() ? null : opts.get(opts.size() - 1);
    }

    private static String first(JSONObject f) {
        JSONArray urls = f == null ? null : f.optJSONArray("urls");
        return urls == null || urls.length() == 0 ? null : urls.optString(0);
    }

    /** Is the preview inside this frame? */
    boolean isIn(View frame) {
        return host != null && host == frame;
    }

    /** Is this card's video the one playing? */
    boolean isPlaying(Core.Item video) {
        return video != null && video.id.equals(playing);
    }

    /** Where the preview got to, in seconds (the watch player starts there). */
    double position() {
        return exo == null ? 0 : exo.positionMs() / 1000.0;
    }

    void stop() {
        generation++;
        playing = null;
        if (exo != null) {
            exo.stop();
        }
        if (holder != null && holder.getParent() instanceof ViewGroup) {
            ((ViewGroup) holder.getParent()).removeView(holder);
        }
        host = null;
    }

    // ------------------------------------------------------------------ player events

    @Override
    public void onPlaying(boolean on) {
        if (on && holder != null) {
            holder.animate().alpha(1f).setDuration(Ui.FAST).start();
        }
    }

    @Override
    public void onEnded() {
        stop();
    }

    @Override
    public void onProgress(double at, double total, double buffered, boolean paused, boolean waiting) {
        if (time != null && total > 0) {
            time.setText(Ui.duration((int) Math.max(0, total - at)));
        }
        if (playing != null && at > 0) {
            Resume.played(playing, at, total);
        }
    }

    @Override
    public void onError(String why) {
        android.util.Log.w("StreamPipe", "feed preview: " + why);
        stop();
    }
}
