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

    /** Told about the first comment once the first page is in (the card under the description). */
    interface Teaser {
        /** `first` is null when there are none; `error` is null on success. */
        void first(JSONObject first, boolean more, String error);
    }

    private Teaser teaser;

    /** Load the first page of `id` now, without showing the panel. */
    void preload(String id, Teaser t) {
        teaser = t;
        if (id != null && !id.equals(videoId)) {
            videoId = id;
            load();
        } else if (!items.isEmpty()) {
            t.first(items.get(0), next >= 0 || items.size() > 1, null);
        }
    }

    CommentsPanel(Context c) {
        super(c);
        setBackgroundColor(Ui.BG);
        setClickable(true);
        setVisibility(GONE);

        LinearLayout col = Ui.column(c);
        // YouTube's sheet: a grab handle, then "Comments" and the close button.
        View handle = new View(c);
        handle.setBackground(Ui.rounded(Ui.light ? 0x33000000 : 0x40FFFFFF, 2));
        LinearLayout.LayoutParams hl = Ui.lin(Ui.dp(40), Ui.dp(4));
        hl.gravity = Gravity.CENTER_HORIZONTAL;
        hl.topMargin = Ui.dp(8);
        col.addView(handle, hl);
        LinearLayout bar = Ui.row(c);
        bar.setPadding(Ui.dp(16), 0, Ui.dp(4), 0);
        TextView title = Ui.text(c, 20, Ui.TEXT, true);
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
        teaser = null;
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
                if (teaser != null) {
                    teaser.first(null, false, err == null ? "Could not load comments." : err);
                }
                // A later open tries again.
                videoId = null;
                return;
            }
            append(e.optJSONObject("ok"));
            if (items.isEmpty()) {
                show("No comments yet, or comments are turned off for this video.");
            }
            if (teaser != null) {
                teaser.first(items.isEmpty() ? null : items.get(0), next >= 0 || items.size() > 1, null);
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
        TextView head, text, likes, replies, pinned;
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
                row.addView(r.avatar, Ui.lin(Ui.dp(28), Ui.dp(28)));
                LinearLayout col = Ui.column(c);
                col.setPadding(Ui.dp(14), 0, 0, 0);
                r.pinned = Ui.lines(Ui.text(c, 12, Ui.MUTE, false), 1);
                r.pinned.setPadding(0, 0, 0, Ui.dp(4));
                col.addView(r.pinned, Ui.lin(Ui.MATCH, Ui.WRAP));
                r.head = Ui.lines(Ui.text(c, 12, Ui.MUTE, false), 1);
                col.addView(r.head, Ui.lin(Ui.MATCH, Ui.WRAP));
                r.text = Ui.text(c, 14, Ui.TEXT, false);
                r.text.setLineSpacing(Ui.dp(3f), 1f);
                r.text.setPadding(0, Ui.dp(4), 0, 0);
                r.text.setAutoLinkMask(android.text.util.Linkify.WEB_URLS);
                r.text.setLinkTextColor(Ui.LINK);
                col.addView(r.text, Ui.lin(Ui.MATCH, Ui.WRAP));
                // Like (with the count), dislike and reply, as small icons.
                LinearLayout acts = Ui.row(c);
                acts.setPadding(0, Ui.dp(10), 0, 0);
                acts.addView(Ui.icon(c, R.drawable.ic_like_outline, Ui.TEXT), Ui.lin(Ui.dp(18), Ui.dp(18)));
                r.likes = Ui.text(c, 12, Ui.MUTE, false);
                r.likes.setPadding(Ui.dp(6), 0, Ui.dp(22), 0);
                acts.addView(r.likes, Ui.lin(Ui.WRAP, Ui.WRAP));
                acts.addView(Ui.icon(c, R.drawable.ic_dislike_outline, Ui.TEXT), Ui.lin(Ui.dp(18), Ui.dp(18)));
                View gap = new View(c);
                acts.addView(gap, Ui.lin(Ui.dp(28), 1));
                acts.addView(Ui.icon(c, R.drawable.ic_comment_outline, Ui.TEXT), Ui.lin(Ui.dp(18), Ui.dp(18)));
                col.addView(acts, Ui.lin(Ui.MATCH, Ui.WRAP));
                r.replies = Ui.text(c, 14, Ui.LINK, true);
                r.replies.setPadding(0, Ui.dp(12), 0, 0);
                col.addView(r.replies, Ui.lin(Ui.WRAP, Ui.WRAP));
                row.addView(col, Ui.weight(Ui.WRAP));
                row.setTag(r);
                view = row;
            }
            Row r = (Row) view.getTag();
            JSONObject o = items.get(position);
            Images.load(r.avatar, Core.url(Core.str(o, "avatar")));
            r.pinned.setText("\uD83D\uDCCC  Pinned");
            r.pinned.setVisibility(o.optBoolean("pinned") ? VISIBLE : GONE);
            String author = Core.str(o, "author");
            r.head.setText((author == null ? "" : author) + "  \u2022  " + Core.Item.shortAgo(o.optString("published"))
                    + (o.optBoolean("hearted") ? "  \u2022  \u2665 by creator" : ""));
            r.text.setText(o.optString("text"));
            long likes = o.isNull("likes") ? -1 : o.optLong("likes", -1);
            r.likes.setText(likes > 0 ? Ui.count(likes) : "");
            int replies = o.optInt("replies", 0);
            r.replies.setText(replies + (replies == 1 ? " reply" : " replies"));
            r.replies.setVisibility(replies > 0 ? VISIBLE : GONE);
            return view;
        }
    }
}
