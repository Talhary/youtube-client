package dev.streampipe.app;

import android.app.Dialog;
import android.content.Context;
import android.graphics.drawable.ColorDrawable;
import android.view.Gravity;
import android.view.View;
import android.view.Window;
import android.view.WindowManager;
import android.view.inputmethod.InputMethodManager;
import android.widget.EditText;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import java.util.ArrayList;
import java.util.List;

/**
 * A menu that slides up from the bottom, in the app's colours: the "more" menu of a video,
 * the bell choices of a channel, search filters, playlist choices.
 */
final class Sheet {
    interface Pick {
        void picked(int index);
    }

    interface Text {
        void entered(String text);
    }

    private final Dialog dialog;
    private final LinearLayout body;
    private final Context context;

    Sheet(Context c, String title) {
        context = c;
        dialog = new Dialog(c);
        dialog.requestWindowFeature(Window.FEATURE_NO_TITLE);
        LinearLayout panel = Ui.column(c);
        android.graphics.drawable.GradientDrawable bg = new android.graphics.drawable.GradientDrawable();
        bg.setColor(Ui.SHEET);
        float r = Ui.dp(18);
        bg.setCornerRadii(new float[] {r, r, r, r, 0, 0, 0, 0});
        panel.setBackground(bg);
        panel.setPadding(0, Ui.dp(8), 0, Ui.dp(12));
        View handle = new View(c);
        handle.setBackground(Ui.rounded(Ui.LINE, 2));
        LinearLayout.LayoutParams hl = Ui.lin(Ui.dp(36), Ui.dp(4));
        hl.gravity = Gravity.CENTER_HORIZONTAL;
        hl.bottomMargin = Ui.dp(6);
        panel.addView(handle, hl);
        if (title != null) {
            TextView t = Ui.lines(Ui.text(c, 15, Ui.TEXT, true), 2);
            t.setText(title);
            t.setPadding(Ui.dp(20), Ui.dp(8), Ui.dp(20), Ui.dp(8));
            panel.addView(t, Ui.lin(Ui.MATCH, Ui.WRAP));
        }
        body = Ui.column(c);
        ScrollView scroll = new ScrollView(c);
        scroll.setVerticalScrollBarEnabled(false);
        scroll.addView(body);
        panel.addView(scroll, Ui.lin(Ui.MATCH, Ui.WRAP));
        dialog.setContentView(panel);
        Window w = dialog.getWindow();
        if (w != null) {
            w.setBackgroundDrawable(new ColorDrawable(0));
            w.setGravity(Gravity.BOTTOM);
            w.setLayout(Math.min(c.getResources().getDisplayMetrics().widthPixels, Ui.dp(480)), WindowManager.LayoutParams.WRAP_CONTENT);
            w.setWindowAnimations(android.R.style.Animation_InputMethod);
            w.setDimAmount(0.5f);
        }
    }

    /** A row with an icon (0 for none) and a label; tapping it closes the sheet first. */
    /** Rows drawn as outlined cards, so each choice reads as its own button. */
    private boolean boxed;

    Sheet boxed() {
        boxed = true;
        return this;
    }

    Sheet add(int icon, String label, Runnable onTap) {
        return add(icon, label, null, onTap);
    }

    Sheet add(int icon, String label, String hint, final Runnable onTap) {
        LinearLayout row = Ui.row(context);
        row.setMinimumHeight(Ui.dp(52));
        row.setPadding(Ui.dp(12), Ui.dp(4), Ui.dp(20), Ui.dp(4));
        row.setForeground(Ui.ripple());
        if (icon != 0) {
            row.addView(Ui.icon(context, icon, Ui.TEXT), Ui.lin(Ui.dp(44), Ui.dp(40)));
        } else {
            row.addView(new View(context), Ui.lin(Ui.dp(8), 1));
        }
        LinearLayout col = Ui.column(context);
        col.setPadding(Ui.dp(10), 0, 0, 0);
        TextView t = Ui.text(context, 15, Ui.TEXT, false);
        t.setText(label);
        col.addView(t);
        if (hint != null) {
            TextView h = Ui.text(context, 12, Ui.MUTE, false);
            h.setText(hint);
            h.setPadding(0, Ui.dp(3), 0, 0);
            col.addView(h);
        }
        row.addView(col, Ui.weight(Ui.WRAP));
        row.setOnClickListener(v -> {
            dialog.dismiss();
            onTap.run();
        });
        LinearLayout.LayoutParams lp = Ui.lin(Ui.MATCH, Ui.WRAP);
        if (boxed) {
            android.graphics.drawable.GradientDrawable card = new android.graphics.drawable.GradientDrawable();
            card.setCornerRadius(Ui.dp(12));
            card.setColor(Ui.light ? 0xFFF7F7F7 : 0x14FFFFFF);
            card.setStroke(Math.max(1, Ui.dp(1)), Ui.light ? 0x26000000 : 0x33FFFFFF);
            row.setForeground(null);
            row.setBackground(new android.graphics.drawable.RippleDrawable(
                    android.content.res.ColorStateList.valueOf(Ui.RIPPLE), card, null));
            row.setPadding(Ui.dp(8), Ui.dp(10), Ui.dp(16), Ui.dp(10));
            row.setElevation(Ui.dp(1));
            lp.setMargins(Ui.dp(14), Ui.dp(5), Ui.dp(14), Ui.dp(5));
        }
        body.addView(row, lp);
        return this;
    }

    /** A row with a check mark that stays open when tapped (e.g. playlists to save to). */
    Sheet check(String label, String hint, boolean on, final java.util.function.Predicate<Boolean> onToggle) {
        final LinearLayout row = Ui.row(context);
        row.setMinimumHeight(Ui.dp(52));
        row.setPadding(Ui.dp(20), Ui.dp(4), Ui.dp(20), Ui.dp(4));
        row.setForeground(Ui.ripple());
        final ImageView box = Ui.icon(context, R.drawable.ic_check, Ui.ON_ACCENT);
        final boolean[] state = {on};
        final Runnable paint = () -> {
            box.setBackground(state[0] ? Ui.rounded(Ui.ACCENT, 5) : outline());
            box.setImageAlpha(state[0] ? 255 : 0);
        };
        paint.run();
        row.addView(box, Ui.lin(Ui.dp(22), Ui.dp(22)));
        LinearLayout col = Ui.column(context);
        col.setPadding(Ui.dp(16), 0, 0, 0);
        TextView t = Ui.text(context, 15, Ui.TEXT, false);
        t.setText(label);
        col.addView(t);
        if (hint != null && !hint.isEmpty()) {
            TextView h = Ui.text(context, 12, Ui.MUTE, false);
            h.setText(hint);
            col.addView(h);
        }
        row.addView(col, Ui.weight(Ui.WRAP));
        row.setOnClickListener(v -> {
            boolean wanted = !state[0];
            // The handler says whether to show the new state now.
            if (onToggle.test(wanted)) {
                state[0] = wanted;
                paint.run();
            }
        });
        body.addView(row, Ui.lin(Ui.MATCH, Ui.WRAP));
        return this;
    }

    private static android.graphics.drawable.Drawable outline() {
        android.graphics.drawable.GradientDrawable g = new android.graphics.drawable.GradientDrawable();
        g.setCornerRadius(Ui.dp(5));
        g.setStroke(Ui.dp(2), Ui.MUTE);
        return g;
    }

    /** A line of plain text (a note, or "Loading…"). */
    TextView note(String text) {
        TextView t = Ui.text(context, 13, Ui.MUTE, false);
        t.setText(text);
        t.setLineSpacing(Ui.dp(3), 1f);
        t.setPadding(Ui.dp(20), Ui.dp(8), Ui.dp(20), Ui.dp(8));
        body.addView(t, Ui.lin(Ui.MATCH, Ui.WRAP));
        return t;
    }

    void clear() {
        body.removeAllViews();
    }

    Sheet show() {
        dialog.show();
        return this;
    }

    void dismiss() {
        dialog.dismiss();
    }

    boolean isShowing() {
        return dialog.isShowing();
    }

    /** One choice out of several; the current one is marked. */
    static void choose(Context c, String title, String[] labels, int selected, final Pick pick) {
        Sheet s = new Sheet(c, title);
        for (int i = 0; i < labels.length; i++) {
            final int index = i;
            s.add(i == selected ? R.drawable.ic_check : 0, labels[i], () -> pick.picked(index));
        }
        s.show();
    }

    /** Ask for a line of text. */
    static void ask(Context c, String title, String hint, String okLabel, final Text done) {
        final Sheet s = new Sheet(c, title);
        final EditText field = new EditText(c);
        field.setHint(hint);
        field.setHintTextColor(Ui.MUTE);
        field.setTextColor(Ui.TEXT);
        field.setSingleLine(true);
        field.setBackground(Ui.rounded(Ui.PANEL, 12));
        field.setPadding(Ui.dp(14), Ui.dp(12), Ui.dp(14), Ui.dp(12));
        LinearLayout.LayoutParams fl = Ui.lin(Ui.MATCH, Ui.WRAP);
        fl.setMargins(Ui.dp(20), Ui.dp(8), Ui.dp(20), Ui.dp(12));
        s.body.addView(field, fl);
        TextView ok = Ui.button(c, okLabel, v -> {
            String text = field.getText().toString().trim();
            if (!text.isEmpty()) {
                s.dismiss();
                done.entered(text);
            }
        });
        LinearLayout.LayoutParams ol = Ui.lin(Ui.WRAP, Ui.WRAP);
        ol.gravity = Gravity.END;
        ol.rightMargin = Ui.dp(20);
        s.body.addView(ok, ol);
        s.show();
        field.requestFocus();
        field.postDelayed(() -> {
            InputMethodManager imm = (InputMethodManager) c.getSystemService(Context.INPUT_METHOD_SERVICE);
            if (imm != null) {
                imm.showSoftInput(field, InputMethodManager.SHOW_IMPLICIT);
            }
        }, 150);
    }

    static List<String> list(String... items) {
        List<String> out = new ArrayList<>();
        java.util.Collections.addAll(out, items);
        return out;
    }
}
