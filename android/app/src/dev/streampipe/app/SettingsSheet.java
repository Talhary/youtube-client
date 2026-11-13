package dev.streampipe.app;

import android.content.Context;
import android.view.Gravity;
import android.view.MotionEvent;
import android.view.View;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.Locale;

/**
 * The player's settings as a sheet that slides up from the bottom: quality, playback speed
 * and captions. It replaces YouTube's own small menu; the choices are sent back to the page.
 */
final class SettingsSheet extends FrameLayout {
    interface Host {
        /** `what` is "quality", "rate" or "captions"; `value` is what the page expects. */
        void onPlayerSetting(String what, Object value);
    }

    private final Host host;
    private final View scrim;
    private final LinearLayout panel;
    private final LinearLayout body;
    private final TextView heading;
    private final ImageView back;
    private JSONObject info = new JSONObject();
    private boolean showing;
    private float downY;
    /** Set by the host before the sheet opens. */
    boolean audioOnly;

    SettingsSheet(Context c, Host host) {
        super(c);
        this.host = host;
        setVisibility(GONE);

        scrim = new View(c);
        scrim.setBackgroundColor(0x99000000);
        scrim.setOnClickListener(v -> dismiss());
        addView(scrim, new LayoutParams(Ui.MATCH, Ui.MATCH));

        panel = Ui.column(c);
        panel.setBackground(topRounded());
        panel.setClickable(true);
        panel.setPadding(0, Ui.dp(6), 0, Ui.dp(10));

        // A grab handle: dragging the sheet down closes it.
        View handle = new View(c);
        handle.setBackground(Ui.rounded(Ui.LINE, 2));
        LinearLayout.LayoutParams hl = Ui.lin(Ui.dp(36), Ui.dp(4));
        hl.gravity = Gravity.CENTER_HORIZONTAL;
        hl.topMargin = Ui.dp(4);
        hl.bottomMargin = Ui.dp(4);
        panel.addView(handle, hl);

        LinearLayout top = Ui.row(c);
        top.setPadding(Ui.dp(4), 0, Ui.dp(16), 0);
        back = Ui.iconButton(c, R.drawable.ic_back, "Back", v -> showMain());
        top.addView(back);
        heading = Ui.text(c, 16, Ui.TEXT, true);
        top.addView(heading, Ui.weight(Ui.WRAP));
        panel.addView(top, Ui.lin(Ui.MATCH, Ui.dp(48)));

        body = Ui.column(c);
        ScrollView scroll = new ScrollView(c);
        scroll.setVerticalScrollBarEnabled(false);
        scroll.setOverScrollMode(OVER_SCROLL_NEVER);
        scroll.addView(body);
        panel.addView(scroll, Ui.lin(Ui.MATCH, Ui.WRAP));

        LayoutParams pl = Ui.frame(Ui.MATCH, Ui.WRAP, Gravity.BOTTOM | Gravity.CENTER_HORIZONTAL);
        addView(panel, pl);
    }

    private static android.graphics.drawable.Drawable topRounded() {
        android.graphics.drawable.GradientDrawable g = new android.graphics.drawable.GradientDrawable();
        g.setColor(Ui.SHEET);
        float r = Ui.dp(18);
        g.setCornerRadii(new float[] {r, r, r, r, 0, 0, 0, 0});
        return g;
    }

    boolean isShowing() {
        return showing;
    }

    /** Open with what the page reported about the current video. */
    void show(JSONObject pageInfo) {
        info = pageInfo;
        showMain();
        // In landscape the sheet is a centred column, not the full width.
        LayoutParams pl = (LayoutParams) panel.getLayoutParams();
        pl.width = Math.min(getWidth() > 0 ? getWidth() : Ui.dp(360), Ui.dp(460));
        panel.setLayoutParams(pl);
        // The list must never be taller than the screen allows.
        View scroll = (View) body.getParent();
        LinearLayout.LayoutParams sl = (LinearLayout.LayoutParams) scroll.getLayoutParams();
        sl.height = Ui.WRAP;
        scroll.setLayoutParams(sl);
        if (showing) {
            return;
        }
        showing = true;
        setVisibility(VISIBLE);
        scrim.setAlpha(0f);
        scrim.animate().alpha(1f).setDuration(Ui.FAST).start();
        panel.setTranslationY(Ui.dp(420));
        panel.animate().translationY(0).setDuration(Ui.MEDIUM).setInterpolator(Ui.EASE_OUT).start();
    }

    void dismiss() {
        if (!showing) {
            return;
        }
        showing = false;
        scrim.animate().alpha(0f).setDuration(Ui.FAST).start();
        panel.animate().translationY(panel.getHeight() + Ui.dp(20)).setDuration(Ui.FAST).setInterpolator(Ui.EASE)
                .withEndAction(() -> {
                    if (!showing) {
                        setVisibility(GONE);
                    }
                }).start();
    }

    @Override
    protected void onMeasure(int w, int h) {
        // Keep the option list within 70% of the screen; it scrolls beyond that.
        int max = Math.round(MeasureSpec.getSize(h) * 0.7f);
        View scroll = (View) body.getParent();
        scroll.measure(MeasureSpec.makeMeasureSpec(MeasureSpec.getSize(w), MeasureSpec.AT_MOST),
                MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED));
        LinearLayout.LayoutParams sl = (LinearLayout.LayoutParams) scroll.getLayoutParams();
        sl.height = scroll.getMeasuredHeight() > max ? max : Ui.WRAP;
        super.onMeasure(w, h);
    }

    // ------------------------------------------------------------------ content

    private static String rateLabel(double r) {
        if (Math.abs(r - 1.0) < 0.001) {
            return "Normal";
        }
        String s = String.format(Locale.ROOT, "%.2f", r).replaceAll("0+$", "").replaceAll("\\.$", "");
        return s + "x";
    }

    private String qualityLabel(String id) {
        if (id == null || id.isEmpty() || id.equals("auto")) {
            String now = label(info.optJSONArray("qualities"), info.optString("quality"));
            return now.isEmpty() ? "Auto" : "Auto (" + now + ")";
        }
        return label(info.optJSONArray("qualities"), id);
    }

    private static String label(JSONArray options, String id) {
        for (int i = 0; options != null && i < options.length(); i++) {
            JSONObject o = options.optJSONObject(i);
            if (o != null && o.optString("id").equals(id)) {
                return o.optString("label");
            }
        }
        return "";
    }

    private void showMain() {
        heading.setText("Settings");
        back.setVisibility(GONE);
        ((LinearLayout) heading.getParent()).setPadding(Ui.dp(20), 0, Ui.dp(16), 0);
        body.removeAllViews();
        JSONArray qualities = info.optJSONArray("qualities");
        // In audio-only mode the picture quality is fixed at the lowest.
        if (qualities != null && qualities.length() > 0 && !audioOnly) {
            body.addView(menuRow("Quality", qualityLabel(info.optString("preferred")), v -> showQualities()));
        }
        body.addView(menuRow("Playback speed", rateLabel(info.optDouble("rate", 1.0)), v -> showRates()));
        JSONArray captions = info.optJSONArray("captions");
        if (captions != null && captions.length() > 0) {
            String now = label(captions, info.optString("caption"));
            body.addView(menuRow("Captions", now.isEmpty() ? "Off" : now, v -> showCaptions()));
        }
        final JSONArray tracks = info.optJSONArray("audio_tracks");
        if (tracks != null && tracks.length() > 1) {
            String now = label(tracks, info.optString("audio_track"));
            body.addView(menuRow("Audio track", now.isEmpty() ? "Original" : now, v -> {
                sub("Audio track");
                String current = info.optString("audio_track");
                for (int i = 0; i < tracks.length(); i++) {
                    final JSONObject t = tracks.optJSONObject(i);
                    if (t != null) {
                        body.addView(option(t.optString("label"), t.optString("id").equals(current), x -> choose("audiotrack", t.optString("id"))));
                    }
                }
            }));
        }
        if (info.has("stable")) {
            final boolean stable = info.optBoolean("stable");
            body.addView(menuRow("Stable volume", stable ? "On" : "Off", v -> choose("stable", !stable)));
        }
        if (info.has("loop")) {
            final boolean loop = info.optBoolean("loop");
            body.addView(menuRow("Loop video", loop ? "On" : "Off", v -> choose("loop", !loop)));
        }
        if (info.has("sleep")) {
            body.addView(menuRow("Sleep timer", info.optString("sleep"), v -> {
                sub("Sleep timer");
                int[] minutes = {0, 10, 15, 30, 45, 60, 90, -1};
                String[] names = {"Off", "10 minutes", "15 minutes", "30 minutes", "45 minutes", "1 hour", "1.5 hours", "End of video"};
                int now = info.optInt("sleep_minutes", 0);
                for (int i = 0; i < minutes.length; i++) {
                    final int m = minutes[i];
                    body.addView(option(names[i], m == now, x -> choose("sleep", m)));
                }
            }));
        }
    }

    private void sub(String title) {
        heading.setText(title);
        back.setVisibility(VISIBLE);
        ((LinearLayout) heading.getParent()).setPadding(Ui.dp(4), 0, Ui.dp(16), 0);
        body.removeAllViews();
    }

    private void showQualities() {
        sub("Quality");
        String preferred = info.optString("preferred", "auto");
        boolean auto = preferred.isEmpty() || preferred.equals("auto");
        body.addView(option(qualityLabel("auto"), auto, v -> choose("quality", "auto")));
        JSONArray a = info.optJSONArray("qualities");
        for (int i = 0; a != null && i < a.length(); i++) {
            final JSONObject q = a.optJSONObject(i);
            if (q == null) {
                continue;
            }
            body.addView(option(q.optString("label"), !auto && q.optString("id").equals(preferred),
                    v -> choose("quality", q.optString("id"))));
        }
    }

    private void showRates() {
        sub("Playback speed");
        double now = info.optDouble("rate", 1.0);
        JSONArray a = info.optJSONArray("rates");
        for (int i = 0; a != null && i < a.length(); i++) {
            final double r = a.optDouble(i, 1.0);
            body.addView(option(rateLabel(r), Math.abs(r - now) < 0.001, v -> choose("rate", r)));
        }
    }

    private void showCaptions() {
        sub("Captions");
        String now = info.optString("caption");
        body.addView(option("Off", now.isEmpty(), v -> choose("captions", "")));
        JSONArray a = info.optJSONArray("captions");
        for (int i = 0; a != null && i < a.length(); i++) {
            final JSONObject t = a.optJSONObject(i);
            if (t == null) {
                continue;
            }
            body.addView(option(t.optString("label"), t.optString("id").equals(now), v -> choose("captions", t.optString("id"))));
        }
    }

    private void choose(String what, Object value) {
        host.onPlayerSetting(what, value);
        dismiss();
    }

    private View menuRow(String name, String value, OnClickListener onClick) {
        Context c = getContext();
        LinearLayout row = Ui.row(c);
        row.setMinimumHeight(Ui.dp(54));
        row.setPadding(Ui.dp(20), 0, Ui.dp(8), 0);
        row.setForeground(Ui.ripple());
        TextView n = Ui.text(c, 15, Ui.TEXT, false);
        n.setText(name);
        row.addView(n, Ui.weight(Ui.WRAP));
        TextView v = Ui.lines(Ui.text(c, 14, Ui.MUTE, false), 1);
        v.setText(value);
        row.addView(v, Ui.lin(Ui.WRAP, Ui.WRAP));
        row.addView(Ui.icon(c, R.drawable.ic_chevron, Ui.MUTE), Ui.lin(Ui.dp(36), Ui.dp(40)));
        row.setOnClickListener(onClick);
        return row;
    }

    private View option(String label, boolean selected, OnClickListener onClick) {
        Context c = getContext();
        LinearLayout row = Ui.row(c);
        row.setMinimumHeight(Ui.dp(50));
        row.setPadding(Ui.dp(20), 0, Ui.dp(20), 0);
        row.setForeground(Ui.ripple());
        // A filled dot marks the current choice.
        View dot = new View(c);
        dot.setBackground(Ui.rounded(selected ? Ui.ACCENT : 0x00000000, 5));
        row.addView(dot, Ui.lin(Ui.dp(10), Ui.dp(10)));
        TextView t = Ui.text(c, 15, selected ? Ui.ACCENT : Ui.TEXT, selected);
        t.setText(label);
        t.setPadding(Ui.dp(16), 0, 0, 0);
        row.addView(t, Ui.weight(Ui.WRAP));
        row.setContentDescription(label + (selected ? ", selected" : ""));
        row.setOnClickListener(onClick);
        return row;
    }

    // ------------------------------------------------------------------ drag down to close

    @Override
    public boolean onInterceptTouchEvent(MotionEvent e) {
        if (e.getActionMasked() == MotionEvent.ACTION_DOWN) {
            downY = e.getRawY();
        } else if (e.getActionMasked() == MotionEvent.ACTION_MOVE && showing) {
            // Only from the top of the sheet (handle and title), so lists keep scrolling.
            float topOfPanel = panel.getTop() + panel.getTranslationY();
            boolean onHeader = e.getY() - (e.getRawY() - downY) < topOfPanel + Ui.dp(60);
            if (onHeader && e.getRawY() - downY > Ui.dp(10)) {
                return true;
            }
        }
        return false;
    }

    @Override
    public boolean onTouchEvent(MotionEvent e) {
        float dy = Math.max(0, e.getRawY() - downY);
        switch (e.getActionMasked()) {
            case MotionEvent.ACTION_MOVE:
                panel.setTranslationY(dy);
                return true;
            case MotionEvent.ACTION_UP:
            case MotionEvent.ACTION_CANCEL:
                if (dy > panel.getHeight() / 3f) {
                    dismiss();
                } else {
                    panel.animate().translationY(0).setDuration(Ui.FAST).start();
                }
                return true;
            default:
                return true;
        }
    }
}
