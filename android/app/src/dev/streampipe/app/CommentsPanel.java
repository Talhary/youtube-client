package dev.streampipe.app;

import android.content.Context;
import android.graphics.Color;
import android.graphics.drawable.ColorDrawable;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.widget.AbsListView;
import android.widget.BaseAdapter;
import android.widget.EditText;
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
    /** Posts a new comment on this video (signed in; from the first page). */
    private String createParams;
    /** The thread a reply goes to (null: a new comment). */
    private JSONObject replyTo;
    private final LinearLayout replying;
    private final TextView replyingText;
    private final ImageView myAvatar;
    private final EditText input;
    private final ImageView send;
    private final View inputBox;

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

        // YouTube's comment box: who you reply to (when replying), then the field and Send.
        LinearLayout box = Ui.column(c);
        box.addView(Ui.divider(c));
        replying = Ui.row(c);
        replying.setPadding(Ui.dp(16), Ui.dp(8), Ui.dp(4), 0);
        replyingText = Ui.lines(Ui.text(c, 12, Ui.MUTE, false), 1);
        replying.addView(replyingText, Ui.weight(Ui.WRAP));
        ImageView cancel = Ui.iconButton(c, R.drawable.ic_close, "Cancel reply", v -> setReplyTo(null));
        replying.addView(cancel, Ui.lin(Ui.dp(32), Ui.dp(28)));
        replying.setVisibility(GONE);
        box.addView(replying);
        LinearLayout field = Ui.row(c);
        field.setPadding(Ui.dp(12), Ui.dp(8), Ui.dp(6), Ui.dp(8));
        myAvatar = new ImageView(c);
        myAvatar.setScaleType(ImageView.ScaleType.CENTER_CROP);
        myAvatar.setBackgroundColor(Ui.PANEL);
        Ui.clipCircle(myAvatar);
        field.addView(myAvatar, Ui.lin(Ui.dp(28), Ui.dp(28)));
        input = new EditText(c);
        input.setTextSize(14);
        input.setTextColor(Ui.TEXT);
        input.setHintTextColor(Ui.MUTE);
        input.setHint("Add a comment\u2026");
        input.setBackground(Ui.rounded(Ui.PANEL, 20));
        input.setPadding(Ui.dp(14), Ui.dp(8), Ui.dp(14), Ui.dp(8));
        input.setMaxLines(4);
        LinearLayout.LayoutParams il = Ui.weight(Ui.WRAP);
        il.leftMargin = Ui.dp(10);
        field.addView(input, il);
        send = Ui.iconButton(c, R.drawable.ic_send, "Post", v -> post());
        field.addView(send, Ui.lin(Ui.dp(44), Ui.dp(44)));
        box.addView(field);
        inputBox = box;
        col.addView(box, Ui.lin(Ui.MATCH, Ui.WRAP));
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
            JSONObject first = e.optJSONObject("ok");
            createParams = first == null || first.isNull("create_params") ? null : first.optString("create_params", null);
            paintBox();
            append(first);
            if (items.isEmpty()) {
                show(createParams != null ? "No comments yet. Be the first to comment."
                        : "No comments yet, or comments are turned off for this video.");
            }
            if (teaser != null) {
                teaser.first(items.isEmpty() ? null : items.get(0), next >= 0 || items.size() > 1, null);
            }
        }, "comments", "id", videoId);
    }

    /** The comment box: signed in only; the account's picture beside it. */
    private void paintBox() {
        boolean in = Core.get().isSignedIn();
        inputBox.setVisibility(in ? VISIBLE : GONE);
        if (YouScreen.account != null) {
            Images.load(myAvatar, YouScreen.account.optString("avatar"));
        }
    }

    /** Reply to `comment`'s thread (a reply row replies to its thread, naming its author). */
    private void setReplyTo(JSONObject comment) {
        if (comment == null) {
            replyTo = null;
            replying.setVisibility(GONE);
            input.setHint("Add a comment\u2026");
            return;
        }
        JSONObject thread = comment.optBoolean("_reply") ? find(comment.optString("_parent")) : comment;
        if (thread == null) {
            thread = comment;
        }
        replyTo = thread;
        String who = Core.str(comment, "author");
        replyingText.setText("Replying to " + (who == null ? "this comment" : who));
        replying.setVisibility(VISIBLE);
        input.setHint("Add a reply\u2026");
        if (comment.optBoolean("_reply") && who != null && input.length() == 0) {
            input.setText(who + " ");
            input.setSelection(input.length());
        }
        input.requestFocus();
        android.view.inputmethod.InputMethodManager im = (android.view.inputmethod.InputMethodManager)
                getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
        if (im != null) {
            im.showSoftInput(input, 0);
        }
    }

    private JSONObject find(String id) {
        for (JSONObject o : items) {
            if (!o.optBoolean("_more") && id.equals(o.optString("id"))) {
                return o;
            }
        }
        return null;
    }

    private void post() {
        final String text = input.getText().toString().trim();
        if (text.isEmpty()) {
            return;
        }
        if (!Core.get().isSignedIn()) {
            android.widget.Toast.makeText(getContext(), "Sign in to comment.", android.widget.Toast.LENGTH_SHORT).show();
            return;
        }
        final JSONObject thread = replyTo;
        String params = thread != null ? Core.str(thread, "reply_params") : createParams;
        if (params == null) {
            android.widget.Toast.makeText(getContext(), thread != null ? "YouTube does not allow replies to this comment."
                    : "YouTube does not allow comments on this video.", android.widget.Toast.LENGTH_LONG).show();
            return;
        }
        send.setEnabled(false);
        send.setAlpha(0.4f);
        final int mine = generation;
        Core.get().call((e, err) -> {
            send.setEnabled(true);
            send.setAlpha(1f);
            if (err != null) {
                android.widget.Toast.makeText(getContext(), "YouTube did not take the comment: " + err, android.widget.Toast.LENGTH_LONG).show();
                return;
            }
            if (mine != generation) {
                return;
            }
            // Show it at once, as YouTube does; the next load reads it from YouTube.
            JSONObject c = new JSONObject();
            try {
                JSONObject acc = YouScreen.account;
                c.put("id", "local-" + System.currentTimeMillis());
                c.put("author", acc == null ? "You" : acc.optString("handle", acc.optString("name", "You")));
                c.put("avatar", acc == null ? JSONObject.NULL : acc.optString("avatar"));
                c.put("text", text);
                c.put("published", "Just now");
                if (thread != null) {
                    c.put("_reply", true);
                    c.put("_parent", thread.optString("id"));
                    items.add(blockEnd(thread), c);
                } else {
                    items.add(0, c);
                    list.setSelection(0);
                }
            } catch (org.json.JSONException ignored) {
            }
            message.setVisibility(GONE);
            adapter.notifyDataSetChanged();
            input.setText("");
            setReplyTo(null);
            android.view.inputmethod.InputMethodManager im = (android.view.inputmethod.InputMethodManager)
                    getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
            if (im != null) {
                im.hideSoftInputFromWindow(input.getWindowToken(), 0);
            }
            android.widget.Toast.makeText(getContext(), thread != null ? "Reply posted" : "Comment posted", android.widget.Toast.LENGTH_SHORT).show();
        }, "post_comment", "params", params, "text", text, "reply", thread != null);
    }

    /** The position after a thread and the replies shown under it. */
    private int blockEnd(JSONObject thread) {
        int at = items.indexOf(thread) + 1;
        String id = thread.optString("id");
        while (at < items.size() && id.equals(items.get(at).optString("_parent"))) {
            at++;
        }
        return at;
    }

    /** Show or hide a comment's replies under it. */
    private void toggleReplies(final JSONObject thread) {
        final String id = thread.optString("id");
        if (thread.optBoolean("_open")) {
            for (int i = items.size() - 1; i >= 0; i--) {
                if (id.equals(items.get(i).optString("_parent"))) {
                    items.remove(i);
                }
            }
            put(thread, "_open", false);
            adapter.notifyDataSetChanged();
            return;
        }
        String token = Core.str(thread, "replies_token");
        if (token == null) {
            android.widget.Toast.makeText(getContext(), "These replies cannot be loaded here.", android.widget.Toast.LENGTH_SHORT).show();
            return;
        }
        put(thread, "_open", true);
        put(thread, "_loading", true);
        adapter.notifyDataSetChanged();
        final int mine = generation;
        Core.get().call((e, err) -> {
            if (mine != generation) {
                return;
            }
            put(thread, "_loading", false);
            JSONObject ok = e == null ? null : e.optJSONObject("ok");
            if (ok == null) {
                put(thread, "_open", false);
                adapter.notifyDataSetChanged();
                android.widget.Toast.makeText(getContext(), err == null ? "Replies did not load." : err, android.widget.Toast.LENGTH_SHORT).show();
                return;
            }
            insertReplies(thread, ok);
        }, "comment_replies", "token", token);
    }

    /** Put a page of replies at the end of a thread's block, with "Show more replies" if there are more. */
    private void insertReplies(JSONObject thread, JSONObject page) {
        String id = thread.optString("id");
        int at = blockEnd(thread);
        // The old "Show more replies" row goes; a new one follows if there are still more.
        if (at > 0 && items.get(at - 1).optBoolean("_more") && id.equals(items.get(at - 1).optString("_parent"))) {
            items.remove(--at);
        }
        JSONArray a = page.optJSONArray("items");
        for (int i = 0; a != null && i < a.length(); i++) {
            JSONObject r = a.optJSONObject(i);
            if (r == null) {
                continue;
            }
            put(r, "_reply", true);
            put(r, "_parent", id);
            items.add(at++, r);
        }
        long more = page.isNull("next") ? -1 : page.optLong("next", -1);
        if (more >= 0) {
            JSONObject row = new JSONObject();
            put(row, "_more", true);
            put(row, "_parent", id);
            put(row, "_cursor", more);
            items.add(at, row);
        }
        adapter.notifyDataSetChanged();
    }

    private void moreReplies(final JSONObject row) {
        if (row.optBoolean("_loading")) {
            return;
        }
        put(row, "_loading", true);
        adapter.notifyDataSetChanged();
        final JSONObject thread = find(row.optString("_parent"));
        final int mine = generation;
        Core.get().call((e, err) -> {
            put(row, "_loading", false);
            JSONObject ok = e == null ? null : e.optJSONObject("ok");
            if (mine != generation || thread == null) {
                return;
            }
            if (ok == null) {
                adapter.notifyDataSetChanged();
                return;
            }
            insertReplies(thread, ok);
        }, "more_comments", "cursor", row.optLong("_cursor"));
    }

    private static void put(JSONObject o, String k, Object v) {
        try {
            o.put(k, v);
        } catch (org.json.JSONException ignored) {
        }
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
        View replyButton;
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
        public int getViewTypeCount() {
            return 2;
        }

        @Override
        public int getItemViewType(int position) {
            return items.get(position).optBoolean("_more") ? 1 : 0;
        }

        @Override
        public View getView(int position, View view, ViewGroup parent) {
            Context c = getContext();
            final JSONObject item = items.get(position);
            if (item.optBoolean("_more")) {
                TextView more = (TextView) view;
                if (more == null) {
                    more = Ui.text(c, 14, Ui.LINK, true);
                    more.setPadding(Ui.dp(96), Ui.dp(4), Ui.dp(16), Ui.dp(14));
                }
                more.setText(item.optBoolean("_loading") ? "Loading\u2026" : "\u21B3  Show more replies");
                more.setOnClickListener(v -> moreReplies(item));
                return more;
            }
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
                ImageView replyIcon = Ui.icon(c, R.drawable.ic_comment_outline, Ui.TEXT);
                replyIcon.setBackground(Ui.rippleRound());
                replyIcon.setContentDescription("Reply");
                acts.addView(replyIcon, Ui.lin(Ui.dp(30), Ui.dp(30)));
                r.replyButton = replyIcon;
                col.addView(acts, Ui.lin(Ui.MATCH, Ui.WRAP));
                r.replies = Ui.text(c, 14, Ui.LINK, true);
                r.replies.setPadding(0, Ui.dp(12), 0, 0);
                col.addView(r.replies, Ui.lin(Ui.WRAP, Ui.WRAP));
                row.addView(col, Ui.weight(Ui.WRAP));
                row.setTag(r);
                view = row;
            }
            Row r = (Row) view.getTag();
            final JSONObject o = item;
            boolean reply = o.optBoolean("_reply");
            // Replies sit indented under their comment, with smaller pictures.
            view.setPadding(Ui.dp(reply ? 58 : 16), Ui.dp(reply ? 6 : 10), Ui.dp(16), Ui.dp(reply ? 6 : 10));
            ViewGroup.LayoutParams al = r.avatar.getLayoutParams();
            al.width = al.height = Ui.dp(reply ? 22 : 28);
            r.avatar.setLayoutParams(al);
            r.replyButton.setOnClickListener(v -> setReplyTo(o));
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
            boolean open = o.optBoolean("_open");
            r.replies.setText(o.optBoolean("_loading") ? "Loading replies\u2026"
                    : open ? "\u25B4  Hide replies" : "\u25BE  " + replies + (replies == 1 ? " reply" : " replies"));
            r.replies.setVisibility(!reply && replies > 0 ? VISIBLE : GONE);
            r.replies.setOnClickListener(v -> toggleReplies(o));
            return view;
        }
    }
}
