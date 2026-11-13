package dev.streampipe.app;

import android.content.Context;
import android.graphics.Color;
import android.graphics.drawable.ColorDrawable;
import android.os.Handler;
import android.os.Looper;
import android.text.SpannableStringBuilder;
import android.text.Spanned;
import android.text.style.ForegroundColorSpan;
import android.text.style.StyleSpan;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.widget.BaseAdapter;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ListView;
import android.widget.TextView;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.List;

/**
 * A live stream's chat, covering the details under the player like the comments do. New
 * messages are asked for as often as YouTube says, while the panel is open.
 */
final class LiveChatPanel extends FrameLayout {
    private static final int KEEP = 250;
    private final Handler main = new Handler(Looper.getMainLooper());
    private final ListView list;
    private final TextView message;
    private final List<JSONObject> items = new ArrayList<>();
    private final java.util.Set<String> seen = new java.util.HashSet<>();
    private final Adapter adapter = new Adapter();
    private String token;
    private boolean showing;
    private int generation;
    private final Runnable poll = this::fetch;

    LiveChatPanel(Context c) {
        super(c);
        setBackgroundColor(Ui.BG);
        setClickable(true);
        setVisibility(GONE);
        LinearLayout col = Ui.column(c);
        LinearLayout bar = Ui.row(c);
        bar.setPadding(Ui.dp(16), 0, Ui.dp(4), 0);
        TextView title = Ui.text(c, 16, Ui.TEXT, true);
        title.setText("Live chat");
        bar.addView(title, Ui.weight(Ui.WRAP));
        bar.addView(Ui.iconButton(c, R.drawable.ic_close, "Close chat", v -> close()));
        col.addView(bar, Ui.lin(Ui.MATCH, Ui.dp(52)));
        col.addView(Ui.divider(c));
        FrameLayout body = new FrameLayout(c);
        list = new ListView(c);
        list.setDivider(null);
        list.setSelector(new ColorDrawable(Color.TRANSPARENT));
        list.setStackFromBottom(true);
        list.setTranscriptMode(ListView.TRANSCRIPT_MODE_NORMAL);
        list.setAdapter(adapter);
        body.addView(list, new LayoutParams(Ui.MATCH, Ui.MATCH));
        message = Ui.text(c, 14, Ui.MUTE, false);
        message.setGravity(Gravity.CENTER);
        message.setText("Connecting to chat…");
        body.addView(message, Ui.frame(Ui.MATCH, Ui.WRAP, Gravity.CENTER));
        col.addView(body, new LinearLayout.LayoutParams(Ui.MATCH, 0, 1f));
        addView(col, new LayoutParams(Ui.MATCH, Ui.MATCH));
    }

    boolean isShowing() {
        return showing;
    }

    /** Start following the chat that `chatToken` opens. */
    void open(String chatToken) {
        if (chatToken == null) {
            return;
        }
        if (token == null) {
            token = chatToken;
        }
        showing = true;
        setAlpha(0f);
        setVisibility(VISIBLE);
        animate().alpha(1f).setDuration(Ui.FAST).start();
        main.removeCallbacks(poll);
        fetch();
    }

    void close() {
        showing = false;
        main.removeCallbacks(poll);
        animate().alpha(0f).setDuration(Ui.FAST).withEndAction(() -> {
            if (!showing) {
                setVisibility(GONE);
            }
        }).start();
    }

    /** Another video: forget this chat. */
    void reset() {
        generation++;
        main.removeCallbacks(poll);
        token = null;
        items.clear();
        seen.clear();
        adapter.notifyDataSetChanged();
        message.setText("Connecting to chat…");
        message.setVisibility(VISIBLE);
        if (showing) {
            showing = false;
            setVisibility(GONE);
        }
    }

    private void fetch() {
        if (!showing || token == null) {
            return;
        }
        final int mine = generation;
        Core.get().call((e, err) -> {
            if (mine != generation || !showing) {
                return;
            }
            JSONObject ok = e == null ? null : e.optJSONObject("ok");
            if (ok == null) {
                message.setText(err == null ? "The chat is not available." : err);
                message.setVisibility(items.isEmpty() ? VISIBLE : GONE);
                main.postDelayed(poll, 10000);
                return;
            }
            JSONArray msgs = ok.optJSONArray("messages");
            for (int i = 0; msgs != null && i < msgs.length(); i++) {
                JSONObject m = msgs.optJSONObject(i);
                if (m != null && seen.add(m.optString("id") + m.optString("text"))) {
                    items.add(m);
                }
            }
            while (items.size() > KEEP) {
                items.remove(0);
            }
            adapter.notifyDataSetChanged();
            message.setVisibility(items.isEmpty() ? VISIBLE : GONE);
            if (items.isEmpty()) {
                message.setText("No messages yet.");
            }
            String next = ok.isNull("next") ? null : ok.optString("next", null);
            if (next == null) {
                message.setText("The chat has ended.");
                message.setVisibility(items.isEmpty() ? VISIBLE : GONE);
                return;
            }
            token = next;
            main.postDelayed(poll, Math.max(1500, ok.optLong("wait_ms", 5000)));
        }, "live_chat", "token", token);
    }

    private final class Adapter extends BaseAdapter {
        @Override
        public int getCount() {
            return items.size();
        }

        @Override
        public Object getItem(int position) {
            return items.get(position);
        }

        @Override
        public long getItemId(int position) {
            return position;
        }

        @Override
        public View getView(int position, View view, ViewGroup parent) {
            Context c = getContext();
            if (view == null) {
                LinearLayout row = Ui.row(c);
                row.setGravity(Gravity.TOP);
                row.setPadding(Ui.dp(14), Ui.dp(5), Ui.dp(14), Ui.dp(5));
                ImageView avatar = new ImageView(c);
                avatar.setScaleType(ImageView.ScaleType.CENTER_CROP);
                avatar.setBackgroundColor(Ui.PANEL);
                Ui.clipCircle(avatar);
                row.addView(avatar, Ui.lin(Ui.dp(24), Ui.dp(24)));
                TextView t = Ui.text(c, 13, Ui.TEXT, false);
                t.setPadding(Ui.dp(10), Ui.dp(2), 0, 0);
                t.setLineSpacing(Ui.dp(2), 1f);
                row.addView(t, Ui.weight(Ui.WRAP));
                view = row;
            }
            JSONObject m = items.get(position);
            LinearLayout row = (LinearLayout) view;
            Images.load((ImageView) row.getChildAt(0), Core.url(Core.str(m, "avatar")));
            SpannableStringBuilder sb = new SpannableStringBuilder();
            String author = m.optString("author");
            sb.append(author);
            sb.setSpan(new ForegroundColorSpan(Ui.MUTE), 0, author.length(), Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
            String amount = m.isNull("amount") ? null : m.optString("amount", null);
            if (amount != null) {
                int at = sb.length();
                sb.append("  ").append(amount);
                sb.setSpan(new ForegroundColorSpan(Ui.ACCENT), at, sb.length(), Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
                sb.setSpan(new StyleSpan(android.graphics.Typeface.BOLD), at, sb.length(), Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
            }
            sb.append("  ").append(m.optString("text"));
            ((TextView) row.getChildAt(1)).setText(sb);
            return view;
        }
    }
}
