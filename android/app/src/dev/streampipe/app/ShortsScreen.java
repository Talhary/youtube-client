package dev.streampipe.app;

import android.animation.ValueAnimator;
import android.content.Context;
import android.graphics.drawable.GradientDrawable;
import android.view.Gravity;
import android.view.MotionEvent;
import android.view.VelocityTracker;
import android.view.View;
import android.view.ViewConfiguration;
import android.view.ViewGroup;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.TextView;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * The Shorts tab, native: YouTube's Shorts feed (read by the core from the same page API the
 * site uses), one Short per screen, swiped up and down, played by the app's own player from
 * the streams the resolver finds. No web view. The overlay follows YouTube's app: actions down
 * the right edge, channel, Subscribe and title at the bottom, a thin progress line.
 */
final class ShortsScreen extends Screen implements Exo.Events {
    private static final class Short {
        String id, params, thumb;
        JSONObject info;
        JSONObject streams;
        boolean infoAsked;
    }

    private final Context c;
    private final Nav nav;
    private final Exo exo;
    private final Stage stage;
    private final CommentsPanel comments;
    private final View busy;
    private final TextView message;
    private final List<Short> items = new ArrayList<>();
    private final Map<Integer, Page> pages = new HashMap<>();
    private String next;
    private boolean loadingMore;
    private int index;
    private boolean visible;
    private boolean started;
    /** Answers for an older feed are ignored. */
    private int generation;
    private String seed;

    ShortsScreen(Context c, Nav nav) {
        this.c = c;
        this.nav = nav;
        exo = new Exo(c, this);
        exo.setFill(true);
        FrameLayout root = new FrameLayout(c);
        root.setBackgroundColor(0xFF000000);
        stage = new Stage(c);
        root.addView(stage, new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));

        // Search and more, over every Short, as in YouTube's app.
        LinearLayout top = Ui.row(c);
        top.setGravity(Gravity.END | Gravity.CENTER_VERTICAL);
        top.setPadding(0, Ui.dp(4), Ui.dp(4), 0);
        top.addView(white(Ui.iconButton(c, R.drawable.ic_search_outline, "Search", v -> nav.push(new SearchScreen(c, nav)))));
        top.addView(white(Ui.iconButton(c, R.drawable.ic_more_h, "More", v -> {
            Short s = current();
            if (s != null) {
                ItemMenu.show(c, nav, asItem(s), null);
            }
        })));
        root.addView(top, Ui.frame(Ui.MATCH, Ui.WRAP, Gravity.TOP));

        busy = Ui.spinner(c);
        root.addView(busy, Ui.frame(Ui.dp(36), Ui.dp(36), Gravity.CENTER));
        message = Ui.text(c, 14, 0xFFDDDDDD, false);
        message.setGravity(Gravity.CENTER);
        message.setPadding(Ui.dp(32), 0, Ui.dp(32), 0);
        message.setOnClickListener(v -> load());
        message.setVisibility(View.GONE);
        root.addView(message, Ui.frame(Ui.MATCH, Ui.WRAP, Gravity.CENTER));

        comments = new CommentsPanel(c);
        Ui.clipRound(comments, 14);
        FrameLayout.LayoutParams cl = new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH);
        cl.topMargin = Math.round(c.getResources().getDisplayMetrics().heightPixels * 0.32f);
        root.addView(comments, cl);
        view = root;
    }

    private static ImageView white(ImageView v) {
        v.setImageTintList(android.content.res.ColorStateList.valueOf(0xFFFFFFFF));
        return v;
    }

    // ------------------------------------------------------------------ the feed

    /** Start the feed at `id` (a Short tapped in a list), then carry on with YouTube's feed. */
    void openAt(String id) {
        seed = id;
        if (started) {
            load();
        }
    }

    private void load() {
        final int mine = ++generation;
        started = true;
        exo.stop();
        items.clear();
        next = null;
        index = 0;
        stage.clearPages();
        message.setVisibility(View.GONE);
        busy.setVisibility(View.VISIBLE);
        Object[] args = seed == null ? new Object[] {"op", "feed"} : new Object[] {"op", "feed", "id", seed};
        seed = null;
        Core.get().call((e, err) -> {
            if (mine != generation) {
                return;
            }
            busy.setVisibility(View.GONE);
            JSONObject ok = e == null ? null : e.optJSONObject("ok");
            if (err != null || ok == null) {
                message.setText((err == null ? "Shorts could not be loaded." : err) + "\n\nTap to try again.");
                message.setVisibility(View.VISIBLE);
                return;
            }
            add(ok);
            if (items.isEmpty()) {
                message.setText("No Shorts right now.\n\nTap to try again.");
                message.setVisibility(View.VISIBLE);
                return;
            }
            stage.showAt(0);
            settle();
        }, "shorts", args);
    }

    private void add(JSONObject page) {
        JSONArray list = page.optJSONArray("items");
        for (int i = 0; list != null && i < list.length(); i++) {
            JSONObject o = list.optJSONObject(i);
            if (o == null) {
                continue;
            }
            String id = o.optString("id");
            boolean dup = false;
            for (Short s : items) {
                dup |= s.id.equals(id);
            }
            if (dup || id.isEmpty()) {
                continue;
            }
            Short s = new Short();
            s.id = id;
            s.params = o.optString("params");
            s.thumb = o.optString("thumb");
            items.add(s);
        }
        next = page.isNull("next") ? null : page.optString("next", null);
    }

    private void loadMore() {
        if (next == null || loadingMore) {
            return;
        }
        final int mine = generation;
        loadingMore = true;
        String token = next;
        Core.get().call((e, err) -> {
            if (mine != generation) {
                return;
            }
            loadingMore = false;
            JSONObject ok = e == null ? null : e.optJSONObject("ok");
            if (err == null && ok != null) {
                add(ok);
                stage.layoutPages();
            }
        }, "shorts", "op", "more", "token", token);
    }

    private Short current() {
        return index >= 0 && index < items.size() ? items.get(index) : null;
    }

    private Core.Item asItem(Short s) {
        Core.Item it = new Core.Item();
        it.type = Core.Item.VIDEO;
        it.id = s.id;
        it.title = s.info == null ? "" : s.info.optString("title");
        it.thumb = s.thumb;
        it.channelName = s.info == null ? null : s.info.optString("channel");
        it.channelId = s.info == null ? null : s.info.optString("channel_id");
        return it;
    }

    /** The Short on screen changed (or the feed arrived): play it, read ahead. */
    private void settle() {
        final Short s = current();
        if (s == null) {
            return;
        }
        if (index >= items.size() - 3) {
            loadMore();
        }
        Page p = pages.get(index);
        if (p != null) {
            p.attach();
        }
        info(s);
        for (int k = 1; k <= 2 && index + k < items.size(); k++) {
            info(items.get(index + k));
        }
        // The next two are resolved now so swiping to them starts at once.
        JSONArray ahead = new JSONArray();
        for (int k = 1; k <= 2 && index + k < items.size(); k++) {
            ahead.put(items.get(index + k).id);
        }
        if (ahead.length() > 0 && Core.get().prefetch) {
            Core.get().send("prefetch_streams", "ids", ahead);
        }
        play(s);
    }

    private void play(final Short s) {
        exo.stop();
        if (s.streams != null) {
            start(s);
            return;
        }
        final int mine = generation;
        Core.get().call((e, err) -> {
            JSONObject ok = e == null ? null : e.optJSONObject("ok");
            if (ok != null) {
                s.streams = ok;
            }
            if (mine != generation || current() != s) {
                return;
            }
            if (ok == null) {
                Page p = pages.get(index);
                if (p != null) {
                    p.error(err == null ? "This Short could not be played." : err);
                }
                return;
            }
            start(s);
        }, "streams", "id", s.id);
    }

    private void start(Short s) {
        JSONObject audio = s.streams.optJSONObject("audio");
        JSONObject video = pick(s.streams);
        String a = first(audio), v = first(video);
        if (a == null) {
            return;
        }
        exo.play(v, a, 0, visible, null);
    }

    /** A picture stream for a phone screen: the short side at most 720, hardware first. */
    private static JSONObject pick(JSONObject streams) {
        List<JSONObject> opts = Player.options(streams);
        JSONObject first = null;
        for (JSONObject f : opts) {
            int w = f.optInt("width"), h = f.optInt("height");
            int side = w > 0 ? Math.min(w, h) : h;
            if (side > 720) {
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

    private void info(final Short s) {
        if (s.infoAsked) {
            return;
        }
        s.infoAsked = true;
        Core.get().call((e, err) -> {
            JSONObject ok = e == null ? null : e.optJSONObject("ok");
            if (ok == null) {
                s.infoAsked = false;
                return;
            }
            s.info = ok;
            for (Page p : pages.values()) {
                if (p.item == s) {
                    p.bind();
                }
            }
        }, "shorts", "op", "info", "id", s.id, "params", s.params);
    }

    // ------------------------------------------------------------------ player events

    @Override
    public void onPlaying(boolean playing) {
        Page p = pages.get(index);
        if (p != null) {
            if (playing) {
                p.thumb.animate().alpha(0f).setDuration(Ui.FAST).start();
            }
            if (playing) {
                p.paused.setVisibility(View.GONE);
                p.failure.setVisibility(View.GONE);
            }
        }
    }

    @Override
    public void onEnded() {
        // Shorts loop, as on YouTube.
        exo.setPlaying(true);
    }

    @Override
    public void onProgress(double at, double total, double buffered, boolean paused, boolean waiting) {
        Page p = pages.get(index);
        if (p != null && total > 0) {
            p.progress.setScaleX((float) Math.max(0, Math.min(1, at / total)));
        }
    }

    @Override
    public void onError(String why) {
        android.util.Log.w("StreamPipe", "shorts: " + why);
        Page p = pages.get(index);
        if (p != null) {
            p.error("This Short could not be played.");
        }
    }

    // ------------------------------------------------------------------ screen hooks

    @Override
    void onShown() {
        visible = true;
        if (!started || seed != null) {
            load();
        } else {
            exo.setPlaying(true);
        }
    }

    @Override
    void onHidden() {
        visible = false;
        exo.setPlaying(false);
    }

    /** Tapping the tab again starts the feed over. */
    @Override
    void onReselected() {
        load();
    }

    /** Back closes the comments first. */
    boolean goBack() {
        if (comments.isShowing()) {
            comments.close();
            return true;
        }
        return false;
    }

    void destroy() {
        exo.release();
    }

    // ------------------------------------------------------------------ one Short on screen

    private final class Page extends FrameLayout {
        Short item;
        final ImageView thumb;
        final FrameLayout video;
        final ImageView avatar, paused;
        final TextView channel, subscribe, title, likes, commentCount, failure;
        final ImageView likeIcon, dislikeIcon;
        final View progress;
        String rating = "none";

        Page(Context c) {
            super(c);
            setBackgroundColor(0xFF000000);
            video = new FrameLayout(c);
            addView(video, new LayoutParams(Ui.MATCH, Ui.MATCH));
            thumb = new ImageView(c);
            thumb.setScaleType(ImageView.ScaleType.CENTER_CROP);
            addView(thumb, new LayoutParams(Ui.MATCH, Ui.MATCH));

            View shade = new View(c);
            shade.setBackground(new GradientDrawable(GradientDrawable.Orientation.BOTTOM_TOP, new int[] {0x99000000, 0x00000000}));
            addView(shade, Ui.frame(Ui.MATCH, Ui.dp(220), Gravity.BOTTOM));

            paused = Ui.icon(c, R.drawable.ic_play, 0xFFFFFFFF);
            paused.setBackground(Ui.rounded(0x66000000, 36));
            paused.setVisibility(GONE);
            addView(paused, Ui.frame(Ui.dp(72), Ui.dp(72), Gravity.CENTER));

            failure = Ui.text(c, 14, 0xFFFFFFFF, false);
            failure.setGravity(Gravity.CENTER);
            failure.setVisibility(GONE);
            addView(failure, Ui.frame(Ui.MATCH, Ui.WRAP, Gravity.CENTER));

            // Right edge: like, dislike, comments, save, share.
            LinearLayout actions = Ui.column(c);
            actions.setGravity(Gravity.CENTER_HORIZONTAL);
            View like = action(c, R.drawable.ic_like_outline, "Like");
            likeIcon = (ImageView) ((ViewGroup) like).getChildAt(0);
            likes = (TextView) ((ViewGroup) like).getChildAt(1);
            like.setOnClickListener(v -> rate("like".equals(rating) ? "none" : "like"));
            actions.addView(like);
            View dislike = action(c, R.drawable.ic_dislike_outline, "Dislike");
            dislikeIcon = (ImageView) ((ViewGroup) dislike).getChildAt(0);
            dislike.setOnClickListener(v -> rate("dislike".equals(rating) ? "none" : "dislike"));
            actions.addView(dislike);
            View comment = action(c, R.drawable.ic_comment_outline, "Comments");
            commentCount = (TextView) ((ViewGroup) comment).getChildAt(1);
            comment.setOnClickListener(v -> comments.open(item.id));
            actions.addView(comment);
            View save = action(c, R.drawable.ic_save_outline, "Save");
            save.setOnClickListener(v -> ItemMenu.saveToPlaylist(c, item.id));
            actions.addView(save);
            View share = action(c, R.drawable.ic_share_outline, "Share");
            share.setOnClickListener(v -> ItemMenu.share(c, asItem(item), 0));
            actions.addView(share);
            FrameLayout.LayoutParams al = Ui.frame(Ui.WRAP, Ui.WRAP, Gravity.BOTTOM | Gravity.END);
            al.setMargins(0, 0, Ui.dp(6), Ui.dp(20));
            addView(actions, al);

            // Bottom: channel, Subscribe, title.
            LinearLayout info = Ui.column(c);
            info.setPadding(Ui.dp(12), 0, Ui.dp(72), Ui.dp(18));
            LinearLayout who = Ui.row(c);
            avatar = new ImageView(c);
            avatar.setScaleType(ImageView.ScaleType.CENTER_CROP);
            avatar.setBackgroundColor(0x33FFFFFF);
            Ui.clipCircle(avatar);
            who.addView(avatar, Ui.lin(Ui.dp(32), Ui.dp(32)));
            channel = Ui.lines(Ui.text(c, 14, 0xFFFFFFFF, true), 1);
            channel.setPadding(Ui.dp(8), 0, Ui.dp(10), 0);
            who.addView(channel, Ui.lin(Ui.WRAP, Ui.WRAP));
            subscribe = Ui.text(c, 13, 0xFF0F0F0F, true);
            subscribe.setText("Subscribe");
            subscribe.setPadding(Ui.dp(12), Ui.dp(7), Ui.dp(12), Ui.dp(7));
            subscribe.setBackground(Ui.rounded(0xFFFFFFFF, 16));
            subscribe.setOnClickListener(v -> subscribe());
            who.addView(subscribe, Ui.lin(Ui.WRAP, Ui.WRAP));
            View.OnClickListener toChannel = v -> {
                if (item.info != null && !item.info.optString("channel_id").isEmpty()) {
                    nav.push(new ChannelScreen(c, nav, item.info.optString("channel_id"), item.info.optString("channel")));
                }
            };
            avatar.setOnClickListener(toChannel);
            channel.setOnClickListener(toChannel);
            info.addView(who, Ui.lin(Ui.WRAP, Ui.WRAP));
            title = Ui.lines(Ui.text(c, 14, 0xFFFFFFFF, false), 2);
            title.setPadding(0, Ui.dp(10), 0, 0);
            title.setLineSpacing(Ui.dp(2), 1f);
            info.addView(title, Ui.lin(Ui.MATCH, Ui.WRAP));
            addView(info, Ui.frame(Ui.MATCH, Ui.WRAP, Gravity.BOTTOM));

            FrameLayout line = new FrameLayout(c);
            line.setBackgroundColor(0x40FFFFFF);
            progress = new View(c);
            progress.setBackgroundColor(Ui.RED);
            progress.setPivotX(0);
            progress.setScaleX(0);
            line.addView(progress, new LayoutParams(Ui.MATCH, Ui.MATCH));
            addView(line, Ui.frame(Ui.MATCH, Ui.dp(2), Gravity.BOTTOM));

            setOnClickListener(v -> {
                if (pages.get(index) != this) {
                    return;
                }
                boolean play = !exo.wantsToPlay();
                exo.setPlaying(play);
                paused.setVisibility(play ? GONE : VISIBLE);
            });
        }

        private View action(Context c, int icon, String label) {
            LinearLayout col = Ui.column(c);
            col.setGravity(Gravity.CENTER_HORIZONTAL);
            col.setPadding(Ui.dp(4), Ui.dp(8), Ui.dp(4), Ui.dp(8));
            col.setBackground(Ui.rippleRound());
            col.setContentDescription(label);
            ImageView i = Ui.icon(c, icon, 0xFFFFFFFF);
            i.setScaleX(1.25f);
            i.setScaleY(1.25f);
            col.addView(i, Ui.lin(Ui.dp(44), Ui.dp(36)));
            TextView t = Ui.text(c, 12, 0xFFFFFFFF, true);
            t.setText(label);
            t.setShadowLayer(4, 0, 1, 0x80000000);
            col.addView(t, Ui.lin(Ui.WRAP, Ui.WRAP));
            return col;
        }

        void set(Short s) {
            item = s;
            thumb.animate().cancel();
            thumb.setAlpha(1f);
            Images.load(thumb, s.thumb);
            progress.setScaleX(0);
            paused.setVisibility(GONE);
            failure.setVisibility(GONE);
            bind();
        }

        void bind() {
            JSONObject i = item.info;
            if (i == null) {
                channel.setText("");
                title.setText("");
                likes.setText("Like");
                commentCount.setText("Comments");
                subscribe.setVisibility(INVISIBLE);
                avatar.setImageDrawable(null);
                return;
            }
            channel.setText(i.optString("channel"));
            title.setText(i.optString("title"));
            String l = i.optString("likes");
            likes.setText(l.isEmpty() ? "Like" : l);
            String cm = i.optString("comments");
            commentCount.setText(cm.isEmpty() ? "Comments" : cm);
            Images.load(avatar, i.optString("avatar"));
            String liked = i.optString("liked");
            rating = "LIKE".equals(liked) ? "like" : "DISLIKE".equals(liked) ? "dislike" : "none";
            paintRating();
            paintSubscribed(i.isNull("subscribed") ? null : i.optBoolean("subscribed"));
        }

        private void paintRating() {
            likeIcon.setImageResource("like".equals(rating) ? R.drawable.ic_like_filled : R.drawable.ic_like_outline);
            dislikeIcon.setImageResource("dislike".equals(rating) ? R.drawable.ic_dislike_filled : R.drawable.ic_dislike_outline);
        }

        private void paintSubscribed(Boolean on) {
            subscribe.setVisibility(VISIBLE);
            boolean yes = Boolean.TRUE.equals(on);
            subscribe.setText(yes ? "Subscribed" : "Subscribe");
            subscribe.setTextColor(yes ? 0xFFFFFFFF : 0xFF0F0F0F);
            subscribe.setBackground(Ui.rounded(yes ? 0x40FFFFFF : 0xFFFFFFFF, 16));
        }

        private void rate(final String want) {
            if (!Core.get().isSignedIn()) {
                nav.toast("Sign in to rate videos");
                return;
            }
            final String before = rating;
            rating = want;
            paintRating();
            Core.get().call((e, err) -> {
                if (err != null) {
                    rating = before;
                    paintRating();
                    nav.toast(err);
                }
            }, "rate", "id", item.id, "rating", want);
        }

        private void subscribe() {
            if (item.info == null) {
                return;
            }
            if (!Core.get().isSignedIn()) {
                nav.toast("Sign in to subscribe");
                return;
            }
            final boolean want = !"Subscribed".contentEquals(subscribe.getText());
            paintSubscribed(want);
            try {
                item.info.put("subscribed", want);
            } catch (org.json.JSONException ignored) {
            }
            Core.get().call((e, err) -> {
                if (err != null) {
                    paintSubscribed(!want);
                    nav.toast(err);
                }
            }, "subscribe", "id", item.info.optString("channel_id"), "on", want);
        }

        void error(String text) {
            failure.setText(text);
            failure.setVisibility(VISIBLE);
        }

        /** This page is the one on screen: the player draws in it. */
        void attach() {
            View v = exo.view();
            if (v.getParent() != video) {
                if (v.getParent() != null) {
                    ((ViewGroup) v.getParent()).removeView(v);
                }
                video.addView(v, new LayoutParams(Ui.MATCH, Ui.MATCH));
            }
        }
    }

    // ------------------------------------------------------------------ vertical pager

    /** Holds the Short on screen and its neighbours, and moves between them with a swipe. */
    private final class Stage extends FrameLayout {
        private final int slop;
        private final List<Page> spare = new ArrayList<>();
        private float downX, downY, offset;
        private boolean dragging;
        private VelocityTracker velocity;
        private ValueAnimator settle;

        Stage(Context c) {
            super(c);
            slop = ViewConfiguration.get(c).getScaledTouchSlop();
        }

        void clearPages() {
            for (Page p : pages.values()) {
                removeView(p);
                spare.add(p);
            }
            pages.clear();
            offset = 0;
        }

        void showAt(int i) {
            index = i;
            layoutPages();
        }

        /** Keep pages for index-1..index+1 and place them. */
        void layoutPages() {
            for (Integer k : new ArrayList<>(pages.keySet())) {
                if (Math.abs(k - index) > 1) {
                    Page p = pages.remove(k);
                    removeView(p);
                    spare.add(p);
                }
            }
            for (int k = index - 1; k <= index + 1; k++) {
                if (k < 0 || k >= items.size() || pages.containsKey(k)) {
                    continue;
                }
                Page p = spare.isEmpty() ? new Page(getContext()) : spare.remove(spare.size() - 1);
                p.set(items.get(k));
                addView(p, new LayoutParams(Ui.MATCH, Ui.MATCH));
                pages.put(k, p);
            }
            place();
        }

        private void place() {
            int h = Math.max(getHeight(), 1);
            for (Map.Entry<Integer, Page> e : pages.entrySet()) {
                e.getValue().setTranslationY((e.getKey() - index) * h + offset);
            }
        }

        @Override
        protected void onSizeChanged(int w, int h, int ow, int oh) {
            super.onSizeChanged(w, h, ow, oh);
            place();
        }

        @Override
        public boolean onInterceptTouchEvent(MotionEvent e) {
            if (comments.isShowing()) {
                return false;
            }
            switch (e.getActionMasked()) {
                case MotionEvent.ACTION_DOWN:
                    downX = e.getX();
                    downY = e.getY();
                    dragging = false;
                    track(e, true);
                    return false;
                case MotionEvent.ACTION_MOVE:
                    track(e, false);
                    float dy = e.getY() - downY, dx = e.getX() - downX;
                    if (Math.abs(dy) > slop && Math.abs(dy) > Math.abs(dx)) {
                        dragging = true;
                        downY = e.getY();
                        if (settle != null) {
                            settle.cancel();
                        }
                        return true;
                    }
                    return false;
                default:
                    return false;
            }
        }

        @Override
        public boolean onTouchEvent(MotionEvent e) {
            track(e, e.getActionMasked() == MotionEvent.ACTION_DOWN);
            switch (e.getActionMasked()) {
                case MotionEvent.ACTION_DOWN:
                    downY = e.getY();
                    return true;
                case MotionEvent.ACTION_MOVE: {
                    float dy = e.getY() - downY;
                    // No pulling past the first Short, or past the last one loaded.
                    if ((index == 0 && dy > 0) || (index >= items.size() - 1 && dy < 0)) {
                        dy *= 0.25f;
                    }
                    offset = dy;
                    place();
                    return true;
                }
                case MotionEvent.ACTION_UP:
                case MotionEvent.ACTION_CANCEL: {
                    velocity.computeCurrentVelocity(1000);
                    float vy = velocity.getYVelocity();
                    int h = getHeight();
                    int go = 0;
                    if ((offset < -h / 4f || vy < -1200) && index < items.size() - 1) {
                        go = 1;
                    } else if ((offset > h / 4f || vy > 1200) && index > 0) {
                        go = -1;
                    }
                    finish(go);
                    velocity.recycle();
                    velocity = null;
                    dragging = false;
                    return true;
                }
                default:
                    return true;
            }
        }

        private void track(MotionEvent e, boolean reset) {
            if (velocity == null) {
                velocity = VelocityTracker.obtain();
            } else if (reset) {
                velocity.clear();
            }
            velocity.addMovement(e);
        }

        /** Animate to the next (1), previous (-1) or same (0) Short. */
        private void finish(final int go) {
            final float from = offset, to = -go * getHeight();
            settle = ValueAnimator.ofFloat(0f, 1f);
            settle.setDuration(Ui.MEDIUM);
            settle.setInterpolator(Ui.EASE_OUT);
            settle.addUpdateListener(a -> {
                offset = from + (to - from) * (float) a.getAnimatedValue();
                place();
            });
            settle.addListener(new android.animation.AnimatorListenerAdapter() {
                boolean cancelled;

                @Override
                public void onAnimationCancel(android.animation.Animator a) {
                    cancelled = true;
                }

                @Override
                public void onAnimationEnd(android.animation.Animator a) {
                    if (cancelled) {
                        return;
                    }
                    offset = 0;
                    if (go != 0) {
                        index += go;
                        layoutPages();
                        settle();
                    } else {
                        place();
                    }
                }
            });
            settle.start();
        }
    }
}
