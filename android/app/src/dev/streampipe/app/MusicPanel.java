package dev.streampipe.app;

import android.content.Context;
import android.content.res.ColorStateList;
import android.view.Gravity;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.SeekBar;
import android.widget.TextView;
import android.widget.Toast;

/**
 * The music player's controls, shown under the artwork when sound is what is playing (a
 * downloaded song, or audio-only mode): a seek bar with the times, and shuffle, previous,
 * play or pause, next and repeat.
 */
final class MusicPanel extends LinearLayout implements Player.Progress {
    interface Host {
        /** Go to the next (+1) or previous (-1) item. False if there is none. */
        boolean step(int by);

        /** The player changed between the native player and YouTube's page. */
        void onEngineChanged();
    }

    private final Player player;
    private final Host host;
    private final SeekBar seek;
    private final TextView elapsed, total;
    private final ImageView play, shuffle, repeat;
    private boolean dragging, paused = true;
    private double position, length;

    MusicPanel(Context c, Player player, final Host host) {
        super(c);
        this.player = player;
        this.host = host;
        setOrientation(VERTICAL);
        setPadding(Ui.dp(8), Ui.dp(6), Ui.dp(8), Ui.dp(4));

        seek = new SeekBar(c);
        seek.setMax(1000);
        seek.setPadding(Ui.dp(14), Ui.dp(8), Ui.dp(14), Ui.dp(8));
        seek.setProgressTintList(ColorStateList.valueOf(Ui.ACCENT));
        seek.setThumbTintList(ColorStateList.valueOf(Ui.ACCENT));
        seek.setSecondaryProgressTintList(ColorStateList.valueOf(Ui.MUTE));
        seek.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
            @Override
            public void onProgressChanged(SeekBar bar, int value, boolean fromUser) {
                if (fromUser) {
                    elapsed.setText(Ui.duration((int) Math.round(length * value / 1000.0)));
                }
            }

            @Override
            public void onStartTrackingTouch(SeekBar bar) {
                dragging = true;
            }

            @Override
            public void onStopTrackingTouch(SeekBar bar) {
                dragging = false;
                if (length > 0) {
                    position = length * bar.getProgress() / 1000.0;
                    MusicPanel.this.player.command("seek", position);
                }
            }
        });
        addView(seek, Ui.lin(Ui.MATCH, Ui.WRAP));

        LinearLayout times = Ui.row(c);
        times.setPadding(Ui.dp(14), 0, Ui.dp(14), 0);
        elapsed = Ui.text(c, 12, Ui.MUTE, false);
        total = Ui.text(c, 12, Ui.MUTE, false);
        total.setGravity(Gravity.END);
        times.addView(elapsed, Ui.weight(Ui.WRAP));
        times.addView(total, Ui.weight(Ui.WRAP));
        addView(times, Ui.lin(Ui.MATCH, Ui.WRAP));

        LinearLayout row = Ui.row(c);
        row.setGravity(Gravity.CENTER);
        row.setPadding(0, Ui.dp(6), 0, Ui.dp(6));
        shuffle = button(c, R.drawable.ic_shuffle, "Shuffle", 48, 1f);
        shuffle.setOnClickListener(v -> {
            if (!Downloads.get().shuffleRest(player.currentId)) {
                Toast.makeText(c, "Shuffle works on a list you started from Downloads.", Toast.LENGTH_SHORT).show();
            } else {
                Toast.makeText(c, "Shuffled what comes next.", Toast.LENGTH_SHORT).show();
            }
        });
        row.addView(shuffle);
        ImageView previous = button(c, R.drawable.ic_previous, "Previous", 56, 1.3f);
        previous.setOnClickListener(v -> {
            // Back to the start first; to the previous one from there.
            if (position > 3 || !host.step(-1)) {
                MusicPanel.this.player.command("seek", 0);
            }
        });
        row.addView(previous);
        // Not scaled: a scaled view scales its background too, and the circle then grew past
        // the row and was cut into a flat blob. The icon is sized by padding instead.
        play = button(c, R.drawable.ic_pause, "Pause", 68, 1f);
        android.graphics.drawable.GradientDrawable disc = new android.graphics.drawable.GradientDrawable();
        disc.setShape(android.graphics.drawable.GradientDrawable.OVAL);
        disc.setColor(Ui.ACCENT);
        play.setBackground(new android.graphics.drawable.RippleDrawable(ColorStateList.valueOf(Ui.RIPPLE), disc, null));
        play.setScaleType(ImageView.ScaleType.FIT_CENTER);
        play.setPadding(Ui.dp(16), Ui.dp(16), Ui.dp(16), Ui.dp(16));
        play.setImageTintList(ColorStateList.valueOf(Ui.ON_ACCENT));
        play.setOnClickListener(v -> {
            // Show it at once; the player confirms a moment later.
            paused = !paused;
            paint();
            MusicPanel.this.player.command("toggle");
        });
        LinearLayout.LayoutParams pl = new LinearLayout.LayoutParams(Ui.dp(68), Ui.dp(68));
        pl.leftMargin = pl.rightMargin = Ui.dp(14);
        row.addView(play, pl);
        ImageView next = button(c, R.drawable.ic_next, "Next", 56, 1.3f);
        next.setOnClickListener(v -> host.step(1));
        row.addView(next);
        repeat = button(c, R.drawable.ic_repeat, "Repeat", 48, 1f);
        repeat.setOnClickListener(v -> {
            Downloads.get().repeat = (Downloads.get().repeat + 1) % 3;
            int mode = Downloads.get().repeat;
            Toast.makeText(c, mode == 0 ? "Repeat off" : mode == 1 ? "Repeat all" : "Repeat this one", Toast.LENGTH_SHORT).show();
            paint();
        });
        row.addView(repeat);
        addView(row, Ui.lin(Ui.MATCH, Ui.WRAP));
        paint();
        showTimes();
    }

    private static ImageView button(Context c, int icon, String label, int sizeDp, float scale) {
        ImageView v = Ui.icon(c, icon, Ui.TEXT);
        v.setBackground(Ui.rippleRound());
        v.setContentDescription(label);
        v.setScaleX(scale);
        v.setScaleY(scale);
        v.setLayoutParams(new LinearLayout.LayoutParams(Ui.dp(sizeDp), Ui.dp(sizeDp)));
        return v;
    }

    private void paint() {
        play.setImageResource(paused ? R.drawable.ic_play : R.drawable.ic_pause);
        play.setContentDescription(paused ? "Play" : "Pause");
        int mode = Downloads.get().repeat;
        repeat.setImageTintList(ColorStateList.valueOf(mode == 0 ? Ui.MUTE : Ui.ACCENT));
        repeat.setContentDescription(mode == 0 ? "Repeat, off" : mode == 1 ? "Repeat all" : "Repeat this one");
        repeat.setAlpha(mode == 2 ? 0.7f : 1f);
    }

    private void showTimes() {
        elapsed.setText(Ui.duration((int) Math.round(position)));
        total.setText(Ui.duration((int) Math.round(length)));
    }

    @Override
    public void onDirect(boolean on) {
        position = 0;
        length = 0;
        paused = false;
        seek.setProgress(0);
        seek.setSecondaryProgress(0);
        showTimes();
        paint();
        host.onEngineChanged();
    }

    @Override
    public void onProgress(double at, double totalSeconds, double buffered, boolean isPaused, boolean waiting) {
        length = totalSeconds;
        paused = isPaused;
        if (!dragging) {
            position = at;
            seek.setProgress(totalSeconds > 0 ? (int) Math.round(1000 * at / totalSeconds) : 0);
            showTimes();
        }
        seek.setSecondaryProgress(totalSeconds > 0 ? (int) Math.round(1000 * buffered / totalSeconds) : 0);
        paint();
    }
}
