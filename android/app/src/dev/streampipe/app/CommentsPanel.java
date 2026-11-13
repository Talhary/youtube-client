package dev.streampipe.app;

import android.content.Context;
import android.graphics.Color;
import android.graphics.drawable.ColorDrawable;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.widget.AbsListView;
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
 * A video's comments: a titled panel that covers the details under the player, with one row
 * per comment (picture, name and age, the text, likes and replies). More load as the end of
 * the list comes near.
 */
final class CommentsPanel extends FrameLayout {
    private final ListView list;
    private final View footer;
    private final View busy;
    private final TextView message;
    private final List<JSONObject> items = new ArrayList<>();
    private final Adapter adapter = new Adapter();
    private String videoId;
    private long next = -1;
    private boolean loadingMore;
    private boolean showing;
    /** Answers for an earlier video are ignored. */
    private int generation;

    CommentsPanel(Context c) {
        super(c);
        setBackgroundColor(Ui.BG);
        setClickable(true);
        setVisibility(GONE);

        LinearLayout col = Ui.column(c);
        LinearLayout bar = Ui.row(c);
        bar.setPadding(Ui.dp(16), 0, Ui.dp(4), 0);
        TextView title = Ui.text(c, 16, Ui.TEXT, true);
        title.setText("Comments");
        bar.addView(title, Ui.weight(Ui.WRAP));
        bar.addView(Ui.iconButton(c, R.drawable.ic_close, "Close comments", v -> close()));
        col.addView(bar, Ui.lin(Ui.MATCH, Ui.dp(52)));
        col.addView(Ui.divider(c));

        FrameLayout body = new FrameLayout(c);
        list = new ListView(c);
        list.setDivider(null);
        list.setSelector(new ColorDrawable(Color.TRANSPARENT));
        list.setVerticalScrollBarEnabled(false);
        list.setClipToPadding(false);
        list.setPadding(0, Ui.dp(6), 0, Ui.dp(20));
        FrameLayout f = new FrameLayout(c);
        f.addView(Ui.spinner(c), Ui.frame(Ui.dp(28), Ui.dp(28), Gravity.CENTER));
        f.setPadding(0, Ui.dp(14), 0, Ui.dp(14));
        f.setVisibility(GONE);
        FrameLayout wrap = new FrameLayout(c);
        wrap.addView(f, new LayoutParams(Ui.MATCH, Ui.WRAP));
        footer = f;
        list.addFooterView(wrap, null, false);
        list.setAdapter(adapter);
        body.addView(list, new LayoutParams(Ui.MATCH, Ui.MATCH));
        busy = Ui.spinner(c);
        body.addView(busy, Ui.frame(Ui.dp(36), Ui.dp(36), Gravity.CENTER));
        message = Ui.text(c, 14, Ui.MUTE, false);
        message.setGravity(Gravity.CENTER);
        message.setPadding(Ui.dp(32), 0, Ui.dp(32), 0);
        message.setVisibility(GONE);
        body.addView(message, Ui.frame(Ui.MATCH, Ui.WRAP, Gravity.CENTER));
        col.addView(body, new LinearLayout.LayoutParams(Ui.MATCH, 0, 1f));
        addView(col, new LayoutParams(Ui.MATCH, Ui.MATCH));

        list.setOnScrollListener(new AbsListView.OnScrollListener() {
            @Override
            public void onScrollStateChanged(AbsListView view, int state) {}

            @Override
            public void onScroll(AbsListView view, int first, int visible, int total) {
                if (total > 0 && first + visible >= total - 5) {
                    loadMore();
                }
            }
        });
    }

    boolean isShowing() {
        return showing;
    }

    /** Show the comments of `id`, loading them unless they are already here. */
    void open(String id) {
        if (id == null) {
            return;
        }
        if (!id.equals(videoId)) {
            videoId = id;
            load();
        }
        if (showing) {
            return;
        }
        showing = true;
        animate().cancel();
        setAlpha(0f);
        setVisibility(VISIBLE);
        animate().alpha(1f).setDuration(Ui.FAST).start();
    }

    void close() {
        if (!showing) {
            return;
        }
        showing = false;
        animate().cancel();
        animate().alpha(0f).setDuration(Ui.FAST).withEndAction(() -> {
            if (!showing) {
                setVisibility(GONE);
            }
        }).start();
    }

    /** Another video is on: hide at once and forget what was loaded. */
    void reset() {
        generation++;
        showing = false;
        animate().cancel();
        setVisibility(GONE);
        videoId = null;
        dropCursor();
        items.clear();
        adapter.notifyDataSetChanged();
    }

    private void dropCursor() {
        if (next >= 0) {
            Core.get().send("drop_cursor", "cursor", next);
            next = -1;
        }
    }

    private void load() {
        final int mine = ++generation;
        dropCursor();
        items.clear();
        adapter.notifyDataSetChanged();
        loadingMore = false;
        footer.setVisibility(GONE);
        message.setVisibility(GONE);
        busy.setVisibility(VISIBLE);
        Core.get().call((e, err) -> {
            if (mine != generation) {
                return;
            }
            busy.setVisibility(GONE);
            if (err != null || e == null) {
                show(err == null ? "Could not load comments." : err);
                return;
            }
            append(e.optJSONObject("ok"));
            if (items.isEmpty()) {
                show("No comments yet, or comments are turned off for this video.");
            }
        }, "comments", "id", videoId);
    }

    private void show(String text) {
        message.setText(text);
        message.setVisibility(VISIBLE);
    }

    private void append(JSONObject page) {
        if (page == null) {
            return;
        }
        JSONArray a = page.optJSONArray("items");
        for (int i = 0; a != null && i < a.length(); i++) {
            JSONObject o = a.optJSONObject(i);
            if (o != null) {
                items.add(o);
            }
        }
        next = page.isNull("next") ? -1 : page.optLong("next", -1);
        adapter.notifyDataSetChanged();
    }

    private void loadMore() {
        if (next < 0 || loadingMore) {
            return;
        }
        final int mine = generation;
        final long cursor = next;
        next = -1;
        loadingMore = true;
        footer.setVisibility(VISIBLE);
        Core.get().call((e, err) -> {
            if (mine != generation) {
                return;
            }
            loadingMore = false;
            footer.setVisibility(GONE);
            if (err == null && e != null) {
                append(e.optJSONObject("ok"));
            }
        }, "more_comments", "cursor", cursor);
    }

    private static final class Row {
        ImageView avatar;
        TextView head, text, foot;
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
        public boolean isEnabled(int position) {
            return false;
        }

        @Override
        public View getView(int position, View view, ViewGroup parent) {
            Context c = getContext();
            if (view == null) {
                Row r = new Row();
                LinearLayout row = Ui.row(c);
                row.setGravity(Gravity.TOP);
                row.setPadding(Ui.dp(16), Ui.dp(10), Ui.dp(16), Ui.dp(10));
                r.avatar = new ImageView(c);
                r.avatar.setScaleType(ImageView.ScaleType.CENTER_CROP);
                r.avatar.setBackgroundColor(Ui.PANEL);
                Ui.clipCircle(r.avatar);
                row.addView(r.avatar, Ui.lin(Ui.dp(36), Ui.dp(36)));
                LinearLayout col = Ui.column(c);
                col.setPadding(Ui.dp(12), 0, 0, 0);
                r.head = Ui.lines(Ui.text(c, 12, Ui.MUTE, false), 1);
                col.addView(r.head, Ui.lin(Ui.MATCH, Ui.WRAP));
                r.text = Ui.text(c, 14, Ui.TEXT, false);
                r.text.setLineSpacing(Ui.dp(3.5f), 1f);
                r.text.setPadding(0, Ui.dp(5), 0, 0);
                col.addView(r.text, Ui.lin(Ui.MATCH, Ui.WRAP));
                r.foot = Ui.lines(Ui.text(c, 12, Ui.MUTE, false), 1);
                r.foot.setPadding(0, Ui.dp(7), 0, 0);
                col.addView(r.foot, Ui.lin(Ui.MATCH, Ui.WRAP));
                row.addView(col, Ui.weight(Ui.WRAP));
                row.setTag(r);
                view = row;
            }
            Row r = (Row) view.getTag();
            JSONObject o = items.get(position);
            Images.load(r.avatar, Core.url(Core.str(o, "avatar")));
            r.head.setText(Ui.join(o.optBoolean("pinned") ? "Pinned" : null, Core.str(o, "author"),
                    o.optString("published"), o.optBoolean("by_owner") ? "Creator" : null));
            r.text.setText(o.optString("text"));
            long likes = o.isNull("likes") ? -1 : o.optLong("likes", -1);
            int replies = o.optInt("replies", 0);
            String foot = Ui.join(likes > 0 ? Ui.count(likes) + (likes == 1 ? " like" : " likes") : null,
                    replies > 0 ? replies + (replies == 1 ? " reply" : " replies") : null,
                    o.optBoolean("hearted") ? "Liked by the creator" : null);
            r.foot.setText(foot);
            r.foot.setVisibility(foot.isEmpty() ? GONE : VISIBLE);
            return view;
        }
    }
}
