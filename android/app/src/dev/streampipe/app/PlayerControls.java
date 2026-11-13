package dev.streampipe.app;

import android.content.Context;
import android.content.res.ColorStateList;
import android.os.Handler;
import android.os.Looper;
import android.view.GestureDetector;
import android.view.Gravity;
import android.view.MotionEvent;
import android.view.View;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ProgressBar;
import android.widget.SeekBar;
import android.widget.TextView;

/**
 * The app's own controls over the direct player: tap to show or hide, play and pause, ten
 * seconds back and forward (buttons, or a double tap on either side), a seek bar with the
 * buffered part, the settings gear and fullscreen. The YouTube-page player has its own
 * controls, so these stay hidden for it.
 */
final class PlayerControls extends FrameLayout implements Player.Progress {
    interface Host {
        void toggleFullscreen();

        /** Go to the next (+1) or previous (-1) video. False if there is none. */
        boolean step(int by);
    }

    private static final int RED = 0xFFFF0033;
    private static final int WHITE = 0xFFFFFFFF;
    private static final int HIDE_AFTER_MS = 3000;
    private static final int SKIP_S = 10;
    private static final double HOLD_RATE = 2.0;
    /** Also told about progress (the music player's own controls). */
    Player.Progress mirror;
    /** Music layout: the controls are under the artwork, not over it. */
    private boolean music;

    void setMusic(boolean on) {
        if (music != on) {
            music = on;
            if (on && shown) {
                show(false);
            }
            refresh();
        }
    }
    private boolean shownBeforeTap;
    /** A finger is held on the player and the video runs at double speed. */
    private boolean holding;

    private final Player player;
    private final Handler main = new Handler(Looper.getMainLooper());
    private final FrameLayout chrome;
    private final LinearLayout center;
    private final ImageView play, fullscreenButton;
    private final ProgressBar spinner;
    private final SeekBar seek;
    private final TextView time, skipNote;
    private final GestureDetector gestures;

    private boolean direct, compact, shown, dragging, paused = true, waiting;
    private double position, length;
    private final Runnable hide = () -> show(false);
    private boolean fullscreen;
    /** Caption lines over the picture. */
    private final TextView caption;
    /** The picture at the dragged time, above the seek bar. */
    private final LinearLayout preview;
    private final ImageView previewImage;
    private final TextView previewTime;
    /** YouTube's storyboard: base address and one level's layout. */
    private String sbUrl;
    private int sbWidth, sbHeight, sbCount, sbCols, sbRows, sbInterval;
    /** Chapter starts (seconds) and titles. */
    private double[] chapterStart = new double[0];
    private String[] chapterTitle = new String[0];
    /** A live stream is playing, and whether it is behind the live edge. */
    private boolean live, behind;
    /** Swipes on the sides of the fullscreen player: 1 = brightness (left), 2 = volume (right). */
    private int swipe;
    private float swipeStartValue;

    PlayerControls(Context c, Player player, final Host host) {
        super(c);
        this.player = player;

        chrome = new FrameLayout(c);
        chrome.setBackgroundColor(0x73000000);
        addView(chrome, new LayoutParams(Ui.MATCH, Ui.MATCH));

        ImageView gear = button(c, R.drawable.ic_settings, "Playback settings", 48, v -> {
            player.command("settings");
            keep();
        });
        chrome.addView(gear, Ui.frame(Ui.dp(48), Ui.dp(48), Gravity.TOP | Gravity.END));

        center = new LinearLayout(c);
        center.setGravity(Gravity.CENTER);
        ImageView previous = button(c, R.drawable.ic_previous, "Previous", 44, v -> {
            // Like a music player: back to the start first, to the previous one from there.
            if (position > 3 || !host.step(-1)) {
                PlayerControls.this.player.command("seek", 0);
            }
            keep();
        });
        center.addView(previous, new LinearLayout.LayoutParams(Ui.dp(44), Ui.dp(44)));
        center.addView(skipButton(c, R.drawable.ic_replay, "Back 10 seconds", -SKIP_S));
        play = button(c, R.drawable.ic_pause, "Pause", 72, v -> {
            togglePlay();
            keep();
        });
        play.setScaleX(1.7f);
        play.setScaleY(1.7f);
        LinearLayout.LayoutParams pl = new LinearLayout.LayoutParams(Ui.dp(72), Ui.dp(72));
        pl.leftMargin = pl.rightMargin = Ui.dp(14);
        center.addView(play, pl);
        center.addView(skipButton(c, R.drawable.ic_forward, "Forward 10 seconds", SKIP_S));
        ImageView next = button(c, R.drawable.ic_next, "Next", 44, v -> {
            host.step(1);
            keep();
        });
        center.addView(next, new LinearLayout.LayoutParams(Ui.dp(44), Ui.dp(44)));
        chrome.addView(center, Ui.frame(Ui.WRAP, Ui.WRAP, Gravity.CENTER));

        LinearLayout bottom = Ui.column(c);
        LinearLayout line = Ui.row(c);
        line.setGravity(Gravity.CENTER_VERTICAL);
        line.setPadding(Ui.dp(14), 0, Ui.dp(2), 0);
        time = Ui.text(c, 13, WHITE, false);
        line.addView(time, Ui.weight(Ui.WRAP));
        fullscreenButton = button(c, R.drawable.ic_fullscreen, "Fullscreen", 44, v -> host.toggleFullscreen());
        line.addView(fullscreenButton, new LinearLayout.LayoutParams(Ui.dp(44), Ui.dp(44)));
        bottom.addView(line, Ui.lin(Ui.MATCH, Ui.WRAP));

        seek = new SeekBar(c);
        seek.setMax(1000);
        seek.setPadding(Ui.dp(14), Ui.dp(6), Ui.dp(14), Ui.dp(10));
        seek.setProgressTintList(ColorStateList.valueOf(RED));
        seek.setThumbTintList(ColorStateList.valueOf(RED));
        seek.setSecondaryProgressTintList(ColorStateList.valueOf(0xCCFFFFFF));
        seek.setProgressBackgroundTintList(ColorStateList.valueOf(0x66FFFFFF));
        seek.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
            @Override
            public void onProgressChanged(SeekBar bar, int value, boolean fromUser) {
                if (fromUser) {
                    showTime(length * value / 1000.0);
                    showPreview(length * value / 1000.0, value / 1000f);
                }
            }

            @Override
            public void onStartTrackingTouch(SeekBar bar) {
                dragging = true;
                main.removeCallbacks(hide);
                showPreview(length * bar.getProgress() / 1000.0, bar.getProgress() / 1000f);
            }

            @Override
            public void onStopTrackingTouch(SeekBar bar) {
                dragging = false;
                preview.setVisibility(GONE);
                if (length > 0) {
                    position = length * bar.getProgress() / 1000.0;
                    PlayerControls.this.player.command("seek", position);
                }
                keep();
            }
        });
        bottom.addView(seek, Ui.lin(Ui.MATCH, Ui.WRAP));
        chrome.addView(bottom, Ui.frame(Ui.MATCH, Ui.WRAP, Gravity.BOTTOM));

        caption = Ui.text(c, 15, WHITE, false);
        caption.setGravity(Gravity.CENTER);
        caption.setBackground(Ui.rounded(0xB0000000, 4));
        caption.setPadding(Ui.dp(8), Ui.dp(3), Ui.dp(8), Ui.dp(3));
        caption.setShadowLayer(2, 0, 0, 0xFF000000);
        caption.setVisibility(GONE);
        LayoutParams capl = Ui.frame(Ui.WRAP, Ui.WRAP, Gravity.BOTTOM | Gravity.CENTER_HORIZONTAL);
        capl.bottomMargin = Ui.dp(14);
        capl.leftMargin = capl.rightMargin = Ui.dp(24);
        addView(caption, capl);

        preview = Ui.column(c);
        preview.setGravity(Gravity.CENTER_HORIZONTAL);
        previewImage = new ImageView(c);
        previewImage.setScaleType(ImageView.ScaleType.FIT_XY);
        previewImage.setBackground(Ui.rounded(0xFF000000, 6));
        preview.addView(previewImage, Ui.lin(Ui.dp(144), Ui.dp(81)));
        previewTime = Ui.text(c, 13, WHITE, true);
        previewTime.setShadowLayer(3, 0, 0, 0xFF000000);
        previewTime.setPadding(0, Ui.dp(4), 0, 0);
        preview.addView(previewTime, Ui.lin(Ui.WRAP, Ui.WRAP));
        preview.setVisibility(GONE);
        LayoutParams prl = Ui.frame(Ui.WRAP, Ui.WRAP, Gravity.BOTTOM | Gravity.START);
        prl.bottomMargin = Ui.dp(56);
        addView(preview, prl);

        time.setOnClickListener(v -> {
            // On a live stream the time is the "LIVE" button: back to the live edge.
            if (live) {
                PlayerControls.this.player.command("golive");
                keep();
            }
        });

        spinner = new ProgressBar(c);
        spinner.setIndeterminateTintList(ColorStateList.valueOf(WHITE));
        spinner.setVisibility(GONE);
        addView(spinner, Ui.frame(Ui.dp(48), Ui.dp(48), Gravity.CENTER));

        // "+10 s" after a double tap.
        skipNote = Ui.text(c, 14, WHITE, true);
        skipNote.setBackground(Ui.rounded(0x99000000, 18));
        skipNote.setPadding(Ui.dp(14), Ui.dp(8), Ui.dp(14), Ui.dp(8));
        skipNote.setAlpha(0f);
        addView(skipNote, Ui.frame(Ui.WRAP, Ui.WRAP, Gravity.CENTER_VERTICAL));

        gestures = new GestureDetector(c, new GestureDetector.SimpleOnGestureListener() {
            @Override
            public boolean onDown(MotionEvent e) {
                swipe = 0;
                return true;
            }

            @Override
            public boolean onScroll(MotionEvent e1, MotionEvent e2, float dx, float dy) {
                if (!fullscreen || e1 == null) {
                    return false;
                }
                if (swipe == 0) {
                    float tx = e2.getX() - e1.getX(), ty = e2.getY() - e1.getY();
                    if (Math.abs(ty) < Ui.dp(12) || Math.abs(ty) < Math.abs(tx) * 1.5f) {
                        return false;
                    }
                    float side = e1.getX() / Math.max(1, getWidth());
                    swipe = side < 0.35f ? 1 : side > 0.65f ? 2 : -1;
                    swipeStartValue = swipe == 1 ? brightness() : swipe == 2 ? volume() : 0;
                }
                if (swipe <= 0) {
                    return false;
                }
                // A full sweep over 80% of the height goes from nothing to everything.
                float change = (e1.getY() - e2.getY()) / (getHeight() * 0.8f);
                float value = Math.max(0f, Math.min(1f, swipeStartValue + change));
                if (swipe == 1) {
                    setBrightness(value);
                    note("Brightness " + Math.round(value * 100) + "%");
                } else {
                    setVolume(value);
                    note("Volume " + Math.round(value * 100) + "%");
                }
                return true;
            }

            @Override
            public boolean onSingleTapUp(MotionEvent e) {
                // Act on the first tap at once. If a second tap follows, the double tap
                // below puts the controls back as they were.
                shownBeforeTap = shown;
                show(!shown);
                return true;
            }

            @Override
            public void onLongPress(MotionEvent e) {
                // Hold to play at double speed; it ends when the finger lifts.
                if (!paused && direct && !compact) {
                    holding = true;
                    PlayerControls.this.player.command("rate", HOLD_RATE);
                    skipNote.animate().cancel();
                    skipNote.setText("2× speed");
                    LayoutParams lp = (LayoutParams) skipNote.getLayoutParams();
                    lp.gravity = Gravity.TOP | Gravity.CENTER_HORIZONTAL;
                    lp.topMargin = Ui.dp(14);
                    lp.leftMargin = lp.rightMargin = 0;
                    skipNote.setLayoutParams(lp);
                    skipNote.setAlpha(1f);
                    if (shown) {
                        show(false);
                    }
                }
            }

            @Override
            public boolean onDoubleTap(MotionEvent e) {
                if (shown != shownBeforeTap) {
                    show(shownBeforeTap);
                }
                float third = getWidth() / 3f;
                if (e.getX() < third) {
                    skip(-SKIP_S);
                } else if (e.getX() > 2 * third) {
                    skip(SKIP_S);
                } else {
                    togglePlay();
                }
                return true;
            }
        });

        chrome.setVisibility(GONE);
        chrome.setAlpha(0f);
        setVisibility(GONE);
        showTime(0);
    }

    private static ImageView button(Context c, int icon, String label, int sizeDp, OnClickListener onClick) {
        ImageView v = Ui.icon(c, icon, WHITE);
        v.setBackground(Ui.rippleRound());
        v.setContentDescription(label);
        v.setOnClickListener(onClick);
        v.setLayoutParams(new LayoutParams(Ui.dp(sizeDp), Ui.dp(sizeDp)));
        return v;
    }

    /** A round arrow with "10" inside. */
    private View skipButton(Context c, int icon, String label, final int seconds) {
        FrameLayout f = new FrameLayout(c);
        ImageView arrow = Ui.icon(c, icon, WHITE);
        arrow.setScaleX(1.35f);
        arrow.setScaleY(1.35f);
        f.addView(arrow, new LayoutParams(Ui.MATCH, Ui.MATCH));
        TextView ten = Ui.text(c, 9, WHITE, true);
        ten.setText(String.valueOf(SKIP_S));
        LayoutParams tl = Ui.frame(Ui.WRAP, Ui.WRAP, Gravity.CENTER);
        tl.topMargin = Ui.dp(2);
        f.addView(ten, tl);
        f.setBackground(Ui.rippleRound());
        f.setContentDescription(label);
        f.setOnClickListener(v -> {
            skip(seconds);
            keep();
        });
        f.setLayoutParams(new LinearLayout.LayoutParams(Ui.dp(56), Ui.dp(56)));
        return f;
    }

    /** Play or pause, and show it on the button at once (the player confirms a moment later). */
    private void togglePlay() {
        paused = !paused;
        refresh();
        player.command("toggle");
    }

    private void skip(int seconds) {
        // Move the bar and the time at once; the player follows.
        if (length > 0) {
            position = Math.max(0, Math.min(length, position + seconds));
            seek.setProgress((int) Math.round(1000 * position / length));
            showTime(position);
        }
        player.command("seekby", seconds);
        skipNote.setText((seconds > 0 ? "+" : "−") + Math.abs(seconds) + " s");
        LayoutParams lp = (LayoutParams) skipNote.getLayoutParams();
        lp.gravity = Gravity.CENTER_VERTICAL | (seconds > 0 ? Gravity.END : Gravity.START);
        lp.topMargin = 0;
        lp.leftMargin = lp.rightMargin = Ui.dp(36);
        skipNote.setLayoutParams(lp);
        skipNote.animate().cancel();
        skipNote.setAlpha(1f);
        skipNote.animate().alpha(0f).setStartDelay(500).setDuration(250).start();
    }

    @Override
    public boolean onTouchEvent(MotionEvent e) {
        gestures.onTouchEvent(e);
        int action = e.getActionMasked();
        if (holding && (action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_CANCEL)) {
            // The finger lifted (or the gesture went to the watch screen): back to the chosen speed.
            holding = false;
            player.command("rate", player.preferredRate);
            skipNote.animate().alpha(0f).setStartDelay(0).setDuration(200).start();
        }
        return true;
    }

    private void note(String text) {
        skipNote.animate().cancel();
        skipNote.setText(text);
        LayoutParams lp = (LayoutParams) skipNote.getLayoutParams();
        lp.gravity = Gravity.TOP | Gravity.CENTER_HORIZONTAL;
        lp.topMargin = Ui.dp(24);
        lp.leftMargin = lp.rightMargin = 0;
        skipNote.setLayoutParams(lp);
        skipNote.setAlpha(1f);
        skipNote.animate().alpha(0f).setStartDelay(700).setDuration(250).start();
    }

    private float brightness() {
        android.app.Activity a = (android.app.Activity) getContext();
        float b = a.getWindow().getAttributes().screenBrightness;
        if (b >= 0) {
            return b;
        }
        try {
            return android.provider.Settings.System.getInt(a.getContentResolver(), android.provider.Settings.System.SCREEN_BRIGHTNESS) / 255f;
        } catch (Exception e) {
            return 0.5f;
        }
    }

    private void setBrightness(float value) {
        android.app.Activity a = (android.app.Activity) getContext();
        android.view.WindowManager.LayoutParams lp = a.getWindow().getAttributes();
        lp.screenBrightness = Math.max(0.01f, value);
        a.getWindow().setAttributes(lp);
    }

    private float volume() {
        android.media.AudioManager am = (android.media.AudioManager) getContext().getSystemService(Context.AUDIO_SERVICE);
        return am.getStreamVolume(android.media.AudioManager.STREAM_MUSIC) / (float) Math.max(1, am.getStreamMaxVolume(android.media.AudioManager.STREAM_MUSIC));
    }

    private void setVolume(float value) {
        android.media.AudioManager am = (android.media.AudioManager) getContext().getSystemService(Context.AUDIO_SERVICE);
        int max = am.getStreamMaxVolume(android.media.AudioManager.STREAM_MUSIC);
        am.setStreamVolume(android.media.AudioManager.STREAM_MUSIC, Math.round(value * max), 0);
    }

    /** Is a sideways-of-centre vertical swipe in fullscreen ours (brightness, volume)? */
    boolean ownsSwipeAt(float x) {
        if (!fullscreen || getWidth() == 0) {
            return false;
        }
        float side = x / getWidth();
        return side < 0.35f || side > 0.65f;
    }

    // ------------------------------------------------------------------ captions, chapters, preview

    @Override
    public void onCaption(String text) {
        boolean on = text != null && !text.isEmpty();
        if (on && !text.contentEquals(caption.getText())) {
            caption.setText(text);
        }
        caption.setVisibility(on ? VISIBLE : GONE);
    }

    @Override
    public void onLive(boolean isLive, boolean isBehind) {
        if (live != isLive || behind != isBehind) {
            live = isLive;
            behind = isBehind;
            showTime(position);
        }
    }

    /** The video's chapters (null or empty for none). */
    void setChapters(double[] starts, String[] titles) {
        chapterStart = starts == null ? new double[0] : starts;
        chapterTitle = titles == null ? new String[0] : titles;
        showTime(position);
    }

    private String chapterAt(double at) {
        String found = null;
        for (int i = 0; i < chapterStart.length; i++) {
            if (chapterStart[i] <= at) {
                found = chapterTitle[i];
            }
        }
        return found;
    }

    /**
     * YouTube's storyboard spec: "base|w#h#count#cols#rows#interval#name#sigh|…", one part per
     * level. The largest level is used.
     */
    void setStoryboard(String spec) {
        sbUrl = null;
        if (spec == null || spec.isEmpty()) {
            return;
        }
        try {
            String[] parts = spec.split("\\|");
            if (parts.length < 2) {
                return;
            }
            int level = parts.length - 2;
            String[] f = parts[parts.length - 1].split("#");
            if (f.length < 8) {
                return;
            }
            sbWidth = Integer.parseInt(f[0]);
            sbHeight = Integer.parseInt(f[1]);
            sbCount = Integer.parseInt(f[2]);
            sbCols = Integer.parseInt(f[3]);
            sbRows = Integer.parseInt(f[4]);
            sbInterval = Integer.parseInt(f[5]);
            sbUrl = parts[0].replace("$L", String.valueOf(level)).replace("$N", f[6]) + "&sigh=" + f[7];
        } catch (RuntimeException e) {
            sbUrl = null;
        }
    }

    private void showPreview(double at, float fraction) {
        String chapter = chapterAt(at);
        previewTime.setText(chapter == null ? Ui.duration((int) Math.round(at)) : Ui.duration((int) Math.round(at)) + "\n" + chapter);
        previewTime.setGravity(Gravity.CENTER);
        boolean picture = sbUrl != null && length > 0 && sbCount > 0;
        previewImage.setVisibility(picture ? VISIBLE : GONE);
        preview.setVisibility(VISIBLE);
        int w = Ui.dp(144);
        float x = seek.getLeft() + seek.getPaddingLeft() + fraction * (seek.getWidth() - seek.getPaddingLeft() - seek.getPaddingRight()) - w / 2f;
        preview.setTranslationX(Math.max(Ui.dp(6), Math.min(getWidth() - w - Ui.dp(6), x)));
        if (!picture) {
            return;
        }
        int interval = sbInterval > 0 ? sbInterval : (int) Math.max(1, length * 1000 / sbCount);
        int index = (int) Math.min(sbCount - 1, Math.max(0, at * 1000 / interval));
        int perPage = Math.max(1, sbCols * sbRows);
        final int pageNo = index / perPage, within = index % perPage;
        final String url = sbUrl.replace("$M", String.valueOf(pageNo));
        Images.bitmap(url, b -> {
            if (b == null || !dragging) {
                return;
            }
            // The sheet may have been scaled down when it was read.
            float scale = b.getWidth() / (float) (sbWidth * sbCols);
            int fx = Math.round((within % sbCols) * sbWidth * scale), fy = Math.round((within / sbCols) * sbHeight * scale);
            int fw = Math.round(sbWidth * scale), fh = Math.round(sbHeight * scale);
            if (fx + fw <= b.getWidth() && fy + fh <= b.getHeight() && fw > 0 && fh > 0) {
                previewImage.setImageBitmap(android.graphics.Bitmap.createBitmap(b, fx, fy, fw, fh));
            }
        });
    }

    /** The seek bar is being dragged, so the gesture is not the watch screen's. */
    boolean seeking() {
        return dragging;
    }

    /** Keep the controls up a little longer after a press. */
    private void keep() {
        main.removeCallbacks(hide);
        if (shown && !paused && !dragging) {
            main.postDelayed(hide, HIDE_AFTER_MS);
        }
    }

    private void show(boolean on) {
        shown = on;
        // Captions move up out of the way of the seek bar.
        caption.animate().translationY(on ? -Ui.dp(52) : 0).setDuration(150).start();
        main.removeCallbacks(hide);
        chrome.animate().cancel();
        if (on) {
            chrome.setVisibility(VISIBLE);
            chrome.animate().alpha(1f).setDuration(80).withEndAction(null).start();
            keep();
        } else {
            chrome.animate().alpha(0f).setDuration(110).withEndAction(() -> chrome.setVisibility(GONE)).start();
        }
    }

    private void showTime(double at) {
        if (live) {
            android.text.SpannableString s = new android.text.SpannableString("●  LIVE");
            s.setSpan(new android.text.style.ForegroundColorSpan(behind ? 0xFFAAAAAA : RED), 0, 1, android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
            time.setText(s);
            time.setContentDescription(behind ? "Go to live" : "Live");
            return;
        }
        String chapter = chapterAt(at);
        time.setText(Ui.duration((int) Math.round(at)) + " / " + Ui.duration((int) Math.round(length)) + (chapter == null ? "" : "  ·  " + chapter));
    }

    private void refresh() {
        setVisibility(direct && !compact && !music ? VISIBLE : GONE);
        spinner.setVisibility(waiting ? VISIBLE : GONE);
        play.setVisibility(waiting ? INVISIBLE : VISIBLE);
        play.setImageResource(paused ? R.drawable.ic_play : R.drawable.ic_pause);
        play.setContentDescription(paused ? "Play" : "Pause");
    }

    /** Mini bar or picture-in-picture: the picture only. */
    void setCompact(boolean on) {
        if (compact != on) {
            compact = on;
            if (on && shown) {
                show(false);
            }
            refresh();
        }
    }

    void setFullscreen(boolean on) {
        fullscreen = on;
        LayoutParams capl = (LayoutParams) caption.getLayoutParams();
        capl.bottomMargin = Ui.dp(on ? 40 : 14);
        caption.setLayoutParams(capl);
        caption.setTextSize(on ? 19 : 15);
        fullscreenButton.setImageResource(on ? R.drawable.ic_fullscreen_exit : R.drawable.ic_fullscreen);
        fullscreenButton.setContentDescription(on ? "Leave fullscreen" : "Fullscreen");
    }

    // ------------------------------------------------------------------ from the player

    @Override
    public void onDirect(boolean on) {
        direct = on;
        if (mirror != null) {
            mirror.onDirect(on);
        }
        position = 0;
        length = 0;
        paused = false;
        waiting = on;
        live = false;
        behind = false;
        sbUrl = null;
        chapterStart = new double[0];
        chapterTitle = new String[0];
        caption.setVisibility(GONE);
        seek.setProgress(0);
        seek.setSecondaryProgress(0);
        showTime(0);
        if (shown) {
            show(false);
        }
        refresh();
    }

    @Override
    public void onProgress(double at, double total, double buffered, boolean isPaused, boolean isWaiting) {
        if (mirror != null) {
            mirror.onProgress(at, total, buffered, isPaused, isWaiting);
        }
        boolean wasPaused = paused;
        length = total;
        paused = isPaused;
        waiting = isWaiting;
        if (!dragging) {
            position = at;
            seek.setProgress(total > 0 ? (int) Math.round(1000 * at / total) : 0);
            showTime(at);
        }
        seek.setSecondaryProgress(total > 0 ? (int) Math.round(1000 * buffered / total) : 0);
        refresh();
        if (wasPaused != paused) {
            // Paused: the controls stay. Playing again: they leave after a moment.
            if (paused && direct && !compact) {
                show(true);
            } else {
                keep();
            }
        }
    }
}
