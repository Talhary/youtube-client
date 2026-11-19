package dev.streampipe.app;

import android.content.Context;
import android.content.res.ColorStateList;
import android.graphics.Outline;
import android.graphics.Typeface;
import android.graphics.drawable.ColorDrawable;
import android.graphics.drawable.Drawable;
import android.graphics.drawable.GradientDrawable;
import android.graphics.drawable.RippleDrawable;
import android.text.TextUtils;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.view.ViewOutlineProvider;
import android.view.animation.Interpolator;
import android.view.animation.PathInterpolator;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ProgressBar;
import android.widget.TextView;

/** Colours, sizes and small view builders shared by every screen. */
final class Ui {
    // The palette. These are set once per activity by init() for the dark or light theme.
    static int BG, PANEL, LINE, MUTE, TEXT, ACCENT, DANGER;
    /** Links and "Show more" text (YouTube's blue). */
    static int LINK;
    /** YouTube red: the logo, progress lines, LIVE badges. */
    static final int RED = 0xFFFF0033;
    /** Text and icons drawn on top of the accent colour. */
    static int ON_ACCENT;
    /** Touch feedback tint. */
    static int RIPPLE;
    /** Background of sheets that slide over the content. */
    static int SHEET;
    static boolean light;

    /** Standard motion curves (Material "emphasized" and "standard"). */
    static final Interpolator EASE_OUT = new PathInterpolator(0.05f, 0.7f, 0.1f, 1f);
    static final Interpolator EASE = new PathInterpolator(0.2f, 0f, 0f, 1f);
    static final int FAST = 160;
    static final int MEDIUM = 280;

    static final int MATCH = ViewGroup.LayoutParams.MATCH_PARENT;
    static final int WRAP = ViewGroup.LayoutParams.WRAP_CONTENT;

    private static float density = 2f;

    private Ui() {}

    static void init(Context c, boolean lightTheme) {
        density = c.getResources().getDisplayMetrics().density;
        light = lightTheme;
        // YouTube's own app colours.
        if (lightTheme) {
            BG = 0xFFFFFFFF;
            PANEL = 0xFFF2F2F2;
            LINE = 0xFFE5E5E5;
            MUTE = 0xFF606060;
            TEXT = 0xFF0F0F0F;
            ACCENT = 0xFF0F0F0F;
            LINK = 0xFF065FD4;
            DANGER = 0xFFCC0000;
            ON_ACCENT = 0xFFFFFFFF;
            RIPPLE = 0x1F000000;
            SHEET = 0xFFFFFFFF;
        } else {
            BG = 0xFF0F0F0F;
            PANEL = 0xFF272727;
            LINE = 0xFF333333;
            MUTE = 0xFFAAAAAA;
            TEXT = 0xFFF1F1F1;
            ACCENT = 0xFFF1F1F1;
            LINK = 0xFF3EA6FF;
            DANGER = 0xFFFF4E45;
            ON_ACCENT = 0xFF0F0F0F;
            RIPPLE = 0x33FFFFFF;
            SHEET = 0xFF212121;
        }
    }

    static int dp(float v) {
        return Math.round(v * density);
    }

    static TextView text(Context c, float sp, int color, boolean bold) {
        TextView t = new TextView(c);
        t.setTextSize(TypedValue.COMPLEX_UNIT_SP, sp);
        t.setTextColor(color);
        t.setIncludeFontPadding(false);
        if (bold) {
            t.setTypeface(Typeface.create("sans-serif-medium", Typeface.NORMAL));
        }
        return t;
    }

    static TextView lines(TextView t, int max) {
        t.setMaxLines(max);
        t.setEllipsize(TextUtils.TruncateAt.END);
        return t;
    }

    static Drawable rounded(int color, float radiusDp) {
        GradientDrawable g = new GradientDrawable();
        g.setColor(color);
        g.setCornerRadius(dp(radiusDp));
        return g;
    }

    /** Touch feedback that stays inside the view's bounds. */
    static Drawable ripple() {
        return new RippleDrawable(ColorStateList.valueOf(RIPPLE), null, new ColorDrawable(0xFFFFFFFF));
    }

    /** Touch feedback for round icon buttons. */
    static Drawable rippleRound() {
        return new RippleDrawable(ColorStateList.valueOf(RIPPLE), null, null);
    }

    static ImageView icon(Context c, int res, int tint) {
        ImageView v = new ImageView(c);
        v.setImageResource(res);
        v.setImageTintList(ColorStateList.valueOf(tint));
        v.setScaleType(ImageView.ScaleType.CENTER);
        return v;
    }

    /** A 48dp round icon button with a spoken label. */
    static ImageView iconButton(Context c, int res, String label, View.OnClickListener onClick) {
        ImageView v = icon(c, res, TEXT);
        v.setBackground(rippleRound());
        v.setContentDescription(label);
        v.setOnClickListener(onClick);
        v.setLayoutParams(new LinearLayout.LayoutParams(dp(48), dp(48)));
        return v;
    }

    static void clipRound(View v, final float radiusDp) {
        v.setClipToOutline(true);
        v.setOutlineProvider(new ViewOutlineProvider() {
            @Override
            public void getOutline(View view, Outline outline) {
                outline.setRoundRect(0, 0, view.getWidth(), view.getHeight(), dp(radiusDp));
            }
        });
    }

    static void clipCircle(View v) {
        v.setClipToOutline(true);
        v.setOutlineProvider(new ViewOutlineProvider() {
            @Override
            public void getOutline(View view, Outline outline) {
                outline.setOval(0, 0, view.getWidth(), view.getHeight());
            }
        });
    }

    static ProgressBar spinner(Context c) {
        ProgressBar p = new ProgressBar(c);
        p.setIndeterminateTintList(ColorStateList.valueOf(MUTE));
        return p;
    }

    static View divider(Context c) {
        View v = new View(c);
        v.setBackgroundColor(LINE);
        v.setLayoutParams(new LinearLayout.LayoutParams(MATCH, Math.max(1, dp(0.75f))));
        return v;
    }

    /** A filled pill button. */
    static TextView button(Context c, String label, View.OnClickListener onClick) {
        TextView b = text(c, 14, ON_ACCENT, true);
        b.setText(label);
        b.setGravity(Gravity.CENTER);
        b.setPadding(dp(16), dp(9), dp(16), dp(9));
        b.setBackground(new RippleDrawable(ColorStateList.valueOf(0x33000000), rounded(ACCENT, 18), null));
        b.setOnClickListener(onClick);
        return b;
    }

    static LinearLayout row(Context c) {
        LinearLayout l = new LinearLayout(c);
        l.setOrientation(LinearLayout.HORIZONTAL);
        l.setGravity(Gravity.CENTER_VERTICAL);
        return l;
    }

    static LinearLayout column(Context c) {
        LinearLayout l = new LinearLayout(c);
        l.setOrientation(LinearLayout.VERTICAL);
        return l;
    }

    static LinearLayout.LayoutParams lin(int w, int h) {
        return new LinearLayout.LayoutParams(w, h);
    }

    static LinearLayout.LayoutParams weight(int h) {
        return new LinearLayout.LayoutParams(0, h, 1f);
    }

    static FrameLayout.LayoutParams frame(int w, int h, int gravity) {
        return new FrameLayout.LayoutParams(w, h, gravity);
    }

    /** "1:02:03" / "4:05". */
    static String duration(int s) {
        if (s < 0) {
            return "";
        }
        int h = s / 3600, m = (s / 60) % 60, sec = s % 60;
        return h > 0 ? String.format(java.util.Locale.ROOT, "%d:%02d:%02d", h, m, sec)
                : String.format(java.util.Locale.ROOT, "%d:%02d", m, sec);
    }

    /** "1.2M". */
    static String count(long n) {
        if (n < 1000) {
            return String.valueOf(n);
        }
        double v;
        String unit;
        if (n < 1_000_000) {
            v = n / 1e3;
            unit = "K";
        } else if (n < 1_000_000_000L) {
            v = n / 1e6;
            unit = "M";
        } else {
            v = n / 1e9;
            unit = "B";
        }
        String s = v >= 100 ? String.format(java.util.Locale.ROOT, "%.0f", v) : String.format(java.util.Locale.ROOT, "%.1f", v);
        if (s.endsWith(".0")) {
            s = s.substring(0, s.length() - 2);
        }
        return s + unit;
    }

    static String join(String... parts) {
        StringBuilder sb = new StringBuilder();
        for (String p : parts) {
            if (p != null && !p.isEmpty()) {
                if (sb.length() > 0) {
                    sb.append(" · ");
                }
                sb.append(p);
            }
        }
        return sb.toString();
    }
}
