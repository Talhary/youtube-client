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

import org.json.JSONObject;

import java.util.ArrayList;
import java.util.HashSet;
import java.util.List;
import java.util.Set;

/**
 * A scrolling list of videos, channels and playlists with loading, error and empty states.
 * It loads its first page from a {@link Source} and further pages from the core as the end
 * of the list comes near.
 */
final class FeedList extends FrameLayout {
    /** Delivers the first page. */
    interface Source {
        void first(Result result);
    }

    interface Result {
        /** `error` is null on success. */
        void page(Core.Page page, String error);
    }

    interface Host {
        void open(Core.Item item);
    }

    /** A frame whose height follows its width (16:9). */
    static final class RatioFrame extends FrameLayout {
        RatioFrame(Context c) {
            super(c);
        }

        @Override
        protected void onMeasure(int w, int h) {
            int width = MeasureSpec.getSize(w);
            super.onMeasure(w, MeasureSpec.makeMeasureSpec(Math.round(width * 9f / 16f), MeasureSpec.EXACTLY));
        }
    }

    private static final int BIG = 0, COMPACT = 1, CHANNEL = 2, PLAYLIST = 3, POST = 4, INLINE = 5, PAIR = 6;
    /** Big cards per row: two on wide screens (600dp and up), as YouTube lays them out there. */
    private int columns = 1;
    /** List rows: item indexes ([i], or [i, j] for two cards side by side); [-1] is the inline view. */
    private final List<int[]> rows = new ArrayList<>();

    /** Rebuild the rows from the items and redraw. */
    private void refresh() {
        rows.clear();
        boolean inl = hasInline();
        for (int i = 0; i < items.size(); ) {
            if (inl && i == inlineAt) {
                rows.add(new int[] {-1});
                inl = false;
            }
            int t = cellType(items.get(i));
            boolean pair = t == BIG && columns > 1 && i + 1 < items.size() && cellType(items.get(i + 1)) == BIG
                    && !(inl && i + 1 == inlineAt);
            if (pair) {
                rows.add(new int[] {i, i + 1});
                i += 2;
            } else {
                rows.add(new int[] {i});
                i++;
            }
        }
        adapter.notifyDataSetChanged();
    }

    private int cellType(Core.Item item) {
        switch (item.type) {
            case Core.Item.CHANNEL:
                return CHANNEL;
            case Core.Item.PLAYLIST:
                // Full cards in big lists (Home, Search), as YouTube shows mixes and playlists.
                return compact ? PLAYLIST : BIG;
            case Core.Item.POST:
                return POST;
            default:
                return compact ? COMPACT : BIG;
        }
    }

    @Override
    protected void onSizeChanged(int w, int h, int ow, int oh) {
        super.onSizeChanged(w, h, ow, oh);
        int want = !compact && w / getResources().getDisplayMetrics().density >= 560 ? 2 : 1;
        if (want != columns) {
            columns = want;
            post(this::refresh);
        }
    }
    /** A view placed among the rows (Home's Shorts shelf) and the row it goes before. */
    private View inline;
    private int inlineAt;

    void setInline(View v, int at) {
        inline = v;
        inlineAt = at;
        refresh();
    }

    private boolean hasInline() {
        return inline != null && items.size() > inlineAt;
    }

    /** The (first) item at a list position, or -1 for the inline view. */
    private int itemIndex(int position) {
        return position >= 0 && position < rows.size() ? rows.get(position)[0] : -1;
    }

    /** At the top of the list (nothing scrolled away). */
    boolean atTop() {
        return list.getFirstVisiblePosition() == 0 && (list.getChildCount() == 0 || list.getChildAt(0).getTop() >= list.getPaddingTop());
    }
    /** Set to give video rows a "more" menu (⋮ and long press). */
    private Nav menu;
    /** The menu offers "Not interested" (feeds YouTube recommends from). */
    private boolean dismissable;

    void setMenu(Nav nav, boolean dismissable) {
        this.menu = nav;
        this.dismissable = dismissable;
    }

    /** Take an item out of the list (after "Not interested"). */
    private void remove(Core.Item item) {
        items.remove(item);
        refresh();
    }

    private boolean compact;

    /** Small rows (true) or full cards; changes how the rows are drawn from now on. */
    void setCompact(boolean on) {
        if (on != compact) {
            compact = on;
            columns = !on && getWidth() / getResources().getDisplayMetrics().density >= 560 ? 2 : 1;
            refresh();
        }
    }
    private final Host host;
    private final ListView list;
    private final View footer;
    private final View busy;
    private final LinearLayout message;
    private final TextView messageText;
    private final TextView retry;
    private final List<Core.Item> items = new ArrayList<>();
    private final Set<String> seen = new HashSet<>();
    private final Adapter adapter = new Adapter();
    private Source source;
    private String emptyText = "Nothing here yet.";
    private long next = -1;
    private boolean loadingMore;
    /** Answers from an older load are ignored. */
    private int generation;

    FeedList(Context c, boolean compact, Host host) {
        super(c);
        this.compact = compact;
        this.host = host;

        list = new ListView(c);
        list.setDivider(null);
        list.setSelector(new ColorDrawable(Color.TRANSPARENT));
        list.setVerticalScrollBarEnabled(false);
        list.setOverScrollMode(OVER_SCROLL_IF_CONTENT_SCROLLS);
        list.setClipToPadding(false);
        list.setPadding(0, compact ? Ui.dp(6) : 0, 0, Ui.dp(16));
        addView(list, new LayoutParams(Ui.MATCH, Ui.MATCH));

        FrameLayout f = new FrameLayout(c);
        f.addView(Ui.spinner(c), Ui.frame(Ui.dp(28), Ui.dp(28), Gravity.CENTER));
        f.setPadding(0, Ui.dp(14), 0, Ui.dp(14));
        f.setVisibility(GONE);
        // ListView needs the footer wrapped so it can be hidden without leaving a gap.
        FrameLayout wrap = new FrameLayout(c);
        wrap.addView(f, new LayoutParams(Ui.MATCH, Ui.WRAP));
        footer = f;
        list.addFooterView(wrap, null, false);

        busy = Ui.spinner(c);
        addView(busy, Ui.frame(Ui.dp(36), Ui.dp(36), Gravity.CENTER));

        message = Ui.column(c);
        message.setGravity(Gravity.CENTER_HORIZONTAL);
        message.setPadding(Ui.dp(32), 0, Ui.dp(32), 0);
        messageText = Ui.text(c, 14, Ui.MUTE, false);
        messageText.setGravity(Gravity.CENTER);
        messageText.setLineSpacing(Ui.dp(3), 1f);
        message.addView(messageText, Ui.lin(Ui.WRAP, Ui.WRAP));
        retry = Ui.button(c, "Try again", v -> load());
        LinearLayout.LayoutParams rl = Ui.lin(Ui.WRAP, Ui.WRAP);
        rl.topMargin = Ui.dp(16);
        message.addView(retry, rl);
        message.setVisibility(GONE);
        addView(message, Ui.frame(Ui.MATCH, Ui.WRAP, Gravity.CENTER));

        list.setOnScrollListener(new AbsListView.OnScrollListener() {
            @Override
            public void onScrollStateChanged(AbsListView view, int state) {
                if (state == SCROLL_STATE_IDLE) {
                    prefetchVisible();
                }
            }

            @Override
            public void onScroll(AbsListView view, int first, int visible, int total) {
                if (total > 0 && first + visible >= total - 6) {
                    loadMore();
                }
            }
        });
    }

    /** A view shown above the first row (set before {@link #setSource}). */
    void setHeader(View header) {
        list.addHeaderView(header, null, false);
    }

    void setEmptyText(String text) {
        emptyText = text;
    }

    void setSource(Source s) {
        source = s;
        list.setAdapter(adapter);
    }

    /** Scroll back to the top, e.g. when the tab is tapped again. */
    void scrollToTop() {
        list.smoothScrollToPosition(0);
    }

    /** (Re)load from the first page. */
    void load() {
        if (source == null) {
            return;
        }
        final int mine = ++generation;
        dropCursor();
        items.clear();
        seen.clear();
        inline = null;
        refresh();
        loadingMore = false;
        footer.setVisibility(GONE);
        message.setVisibility(GONE);
        busy.setVisibility(VISIBLE);
        source.first((page, error) -> {
            if (mine != generation) {
                return;
            }
            busy.setVisibility(GONE);
            if (error != null) {
                showMessage(error, true);
                return;
            }
            append(page);
            if (items.isEmpty()) {
                showMessage(emptyText, false);
            } else {
                // What is on screen once the first page is in.
                list.postDelayed(FeedList.this::prefetchVisible, 800);
                list.setAlpha(0f);
                list.setTranslationY(Ui.dp(14));
                list.animate().alpha(1f).translationY(0).setDuration(Ui.MEDIUM).setInterpolator(Ui.EASE_OUT).start();
            }
        });
    }

    /**
     * Resolve the videos now on screen ahead of time (streams and details), so the one the
     * user taps starts at once. Only when the list has stopped moving, at most a few.
     */
    private void prefetchVisible() {
        if (!Core.get().prefetch) {
            return;
        }
        int first = list.getFirstVisiblePosition() - list.getHeaderViewsCount();
        int last = list.getLastVisiblePosition() - list.getHeaderViewsCount();
        org.json.JSONArray ids = new org.json.JSONArray();
        List<Integer> visible = new ArrayList<>();
        for (int p = Math.max(0, first); p <= last && p < rows.size(); p++) {
            for (int i : rows.get(p)) {
                visible.add(i);
            }
        }
        for (int i : visible) {
            if (i < 0 || i >= items.size() || ids.length() >= 4) {
                continue;
            }
            Core.Item it = items.get(i);
            if (it.type == Core.Item.VIDEO && !it.live && it.localUri == null) {
                ids.put(it.id);
            }
        }
        if (ids.length() > 0) {
            Core.get().send("prefetch_streams", "ids", ids);
        }
    }

    /** Show a fixed message instead of a list (e.g. "sign in to see this"). */
    void showMessage(String text, boolean canRetry) {
        generation++;
        busy.setVisibility(GONE);
        items.clear();
        seen.clear();
        refresh();
        messageText.setText(text);
        retry.setVisibility(canRetry ? VISIBLE : GONE);
        message.setVisibility(VISIBLE);
    }

    private void append(Core.Page page) {
        for (Core.Item i : page.items) {
            if (seen.add(i.type + ":" + i.id)) {
                items.add(i);
            }
        }
        next = page.next;
        refresh();
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
            // The handle is single-use: a failed page simply ends the list.
            if (err == null && e != null) {
                append(Core.Page.parse(e.optJSONObject("ok")));
            }
        }, "more", "cursor", cursor);
    }

    /** Let the core forget a continuation that will not be used. */
    private void dropCursor() {
        if (next >= 0) {
            Core.get().send("drop_cursor", "cursor", next);
            next = -1;
        }
    }

    @Override
    protected void onDetachedFromWindow() {
        super.onDetachedFromWindow();
        dropCursor();
    }

    /** A page out of an answer whose `ok` is the page itself. */
    static void deliver(JSONObject e, String err, Result r) {
        if (err != null || e == null) {
            r.page(null, err == null ? "No answer." : err);
        } else {
            r.page(Core.Page.parse(e.optJSONObject("ok")), null);
        }
    }

    // ------------------------------------------------------------------ rows

    private static final class Row {
        ImageView avatar;
        ImageView image;
        TextView badge, title, line1, line2;
        WatchLine watch;
        ImageView more;
        LinearLayout images;
    }

    /** The line along a thumbnail's bottom edge showing how much was watched, as on YouTube. */
    static final class WatchLine extends View {
        private final android.graphics.Paint paint = new android.graphics.Paint();
        private int percent;

        WatchLine(Context c) {
            super(c);
        }

        void set(int percent) {
            this.percent = Math.max(0, Math.min(100, percent));
            setVisibility(this.percent > 0 ? VISIBLE : GONE);
            invalidate();
        }

        @Override
        protected void onDraw(android.graphics.Canvas canvas) {
            int w = getWidth(), h = getHeight();
            paint.setColor(0x99FFFFFF);
            canvas.drawRect(0, 0, w, h, paint);
            paint.setColor(0xFFFF0033);
            canvas.drawRect(0, 0, w * percent / 100f, h, paint);
        }

        static WatchLine in(FrameLayout thumb) {
            WatchLine line = new WatchLine(thumb.getContext());
            line.setVisibility(GONE);
            thumb.addView(line, Ui.frame(Ui.MATCH, Ui.dp(4), Gravity.BOTTOM));
            return line;
        }
    }

    private final class Adapter extends BaseAdapter {
        @Override
        public int getCount() {
            return rows.size();
        }

        @Override
        public Core.Item getItem(int position) {
            int i = itemIndex(position);
            return i < 0 ? null : items.get(i);
        }

        @Override
        public long getItemId(int position) {
            return position;
        }

        @Override
        public int getViewTypeCount() {
            return 7;
        }

        @Override
        public int getItemViewType(int position) {
            int[] row = rows.get(position);
            if (row[0] < 0) {
                return INLINE;
            }
            return row.length == 2 ? PAIR : cellType(items.get(row[0]));
        }

        @Override
        public View getView(int position, View view, ViewGroup parent) {
            int type = getItemViewType(position);
            if (type == INLINE) {
                if (inline.getParent() instanceof ViewGroup && inline.getParent() != parent) {
                    ((ViewGroup) inline.getParent()).removeView(inline);
                }
                return inline;
            }
            int[] row = rows.get(position);
            if (type == PAIR) {
                LinearLayout pair = (LinearLayout) view;
                if (pair == null) {
                    pair = Ui.row(getContext());
                    pair.setGravity(Gravity.TOP);
                    pair.setPadding(Ui.dp(10), 0, Ui.dp(10), 0);
                    for (int k = 0; k < 2; k++) {
                        View cell = bigRow();
                        cell.setPadding(Ui.dp(6), 0, Ui.dp(6), Ui.dp(20));
                        Ui.clipRound(((LinearLayout) cell).getChildAt(0), 12);
                        pair.addView(cell, Ui.weight(Ui.WRAP));
                    }
                }
                bindCell(pair.getChildAt(0), items.get(row[0]), BIG);
                bindCell(pair.getChildAt(1), items.get(row[1]), BIG);
                return pair;
            }
            final Core.Item item = items.get(row[0]);
            if (view == null) {
                view = type == BIG ? bigRow() : type == CHANNEL ? channelRow() : type == POST ? postRow() : sideRow(type == PLAYLIST);
            }
            bindCell(view, item, type);
            return view;
        }

        private void bindCell(View view, final Core.Item item, int type) {
            Row r = (Row) view.getTag();
            if (type == POST) {
                bindPost(r, item);
                view.setOnClickListener(item.attached == null ? null : v -> host.open(item.attached));
                return;
            }
            Images.load(r.image, item.thumb);
            r.title.setText(item.title);
            if (r.watch != null) {
                // What YouTube says, or how far it got in this app, whichever is further.
                r.watch.set(type == BIG || type == COMPACT ? Math.max(item.watched, Resume.percent(item.id)) : 0);
            }
            if (r.more != null) {
                boolean on = menu != null && item.type == Core.Item.VIDEO;
                if (type == BIG && item.type == Core.Item.PLAYLIST) {
                    on = false;
                    r.more.setVisibility(GONE);
                }
                r.more.setVisibility(on ? VISIBLE : GONE);
                r.more.setOnClickListener(on ? v -> openMenu(item) : null);
            }
            if (menu != null && item.type == Core.Item.VIDEO) {
                view.setOnLongClickListener(v -> {
                    openMenu(item);
                    return true;
                });
            } else {
                view.setOnLongClickListener(null);
            }
            switch (type) {
                case BIG:
                    if (item.type == Core.Item.PLAYLIST) {
                        r.line1.setText(Ui.join(item.channelName, "Playlist"));
                        r.badge.setText(item.videoCount >= 0 ? "\u2261  " + item.videoCount + " videos" : "Mix");
                        r.badge.setBackground(Ui.rounded(0xCC000000, 4));
                        r.badge.setVisibility(VISIBLE);
                        r.avatar.setVisibility(GONE);
                        break;
                    }
                    r.line1.setText(item.shortMeta());
                    badge(r.badge, item);
                    // The channel's picture sits beside the title, as on YouTube.
                    r.avatar.setVisibility(item.channelAvatar == null ? GONE : VISIBLE);
                    Images.load(r.avatar, item.channelAvatar);
                    break;
                case COMPACT:
                    set(r.line1, item.channelName);
                    set(r.line2, item.meta(false));
                    badge(r.badge, item);
                    break;
                case PLAYLIST:
                    set(r.line1, Ui.join(item.channelName, "Playlist"));
                    set(r.line2, null);
                    r.badge.setText(item.videoCount >= 0 ? item.videoCount + " videos" : "Playlist");
                    r.badge.setVisibility(VISIBLE);
                    break;
                default:
                    set(r.line1, item.subscribers >= 0 ? Ui.count(item.subscribers) + " subscribers" : null);
            }
            view.setContentDescription(item.title);
            view.setOnClickListener(v -> host.open(item));
        }

        private void openMenu(final Core.Item item) {
            ItemMenu.show(getContext(), menu, item, dismissable ? () -> remove(item) : null);
        }

        private void bindPost(Row r, Core.Item item) {
            Images.load(r.avatar, item.channelAvatar);
            r.title.setText(item.channelName == null ? "" : item.channelName);
            set(r.line1, item.published);
            r.line2.setText(item.text);
            r.line2.setVisibility(item.text == null || item.text.isEmpty() ? GONE : VISIBLE);
            r.badge.setText(item.likes == null ? "" : item.likes + " likes");
            r.badge.setVisibility(item.likes == null ? GONE : VISIBLE);
            r.images.removeAllViews();
            java.util.List<String> pictures = new ArrayList<>();
            if (item.attached != null && item.attached.thumb != null) {
                pictures.add(item.attached.thumb);
            } else if (item.images != null) {
                pictures.addAll(item.images);
            }
            Context c = getContext();
            for (int i = 0; i < Math.min(pictures.size(), 4); i++) {
                RatioFrame f = new RatioFrame(c);
                f.setBackgroundColor(Ui.PANEL);
                Ui.clipRound(f, 10);
                ImageView img = new ImageView(c);
                img.setScaleType(ImageView.ScaleType.CENTER_CROP);
                f.addView(img, new LayoutParams(Ui.MATCH, Ui.MATCH));
                Images.load(img, pictures.get(i));
                if (item.attached != null) {
                    TextView play = Ui.text(c, 12, Ui.TEXT, true);
                    play.setText("▶  " + item.attached.title);
                    play.setMaxLines(1);
                    play.setEllipsize(android.text.TextUtils.TruncateAt.END);
                    play.setBackgroundColor(0xB0000000);
                    play.setPadding(Ui.dp(10), Ui.dp(6), Ui.dp(10), Ui.dp(6));
                    f.addView(play, Ui.frame(Ui.MATCH, Ui.WRAP, Gravity.BOTTOM));
                }
                LinearLayout.LayoutParams lp = Ui.lin(Ui.MATCH, Ui.WRAP);
                lp.topMargin = Ui.dp(8);
                r.images.addView(f, lp);
            }
            r.images.setVisibility(pictures.isEmpty() ? GONE : VISIBLE);
        }

        /** A channel post: author, time, text, pictures or the attached video. */
        private View postRow() {
            Context c = getContext();
            Row r = new Row();
            LinearLayout root = Ui.column(c);
            root.setPadding(Ui.dp(16), Ui.dp(14), Ui.dp(16), Ui.dp(14));
            LinearLayout top = Ui.row(c);
            r.avatar = new ImageView(c);
            r.avatar.setScaleType(ImageView.ScaleType.CENTER_CROP);
            r.avatar.setBackgroundColor(Ui.PANEL);
            Ui.clipCircle(r.avatar);
            top.addView(r.avatar, Ui.lin(Ui.dp(36), Ui.dp(36)));
            LinearLayout who = Ui.column(c);
            who.setPadding(Ui.dp(12), 0, 0, 0);
            r.title = Ui.lines(Ui.text(c, 14, Ui.TEXT, true), 1);
            who.addView(r.title);
            r.line1 = Ui.lines(Ui.text(c, 12, Ui.MUTE, false), 1);
            who.addView(r.line1);
            top.addView(who, Ui.weight(Ui.WRAP));
            root.addView(top, Ui.lin(Ui.MATCH, Ui.WRAP));
            r.line2 = Ui.text(c, 14, Ui.TEXT, false);
            r.line2.setLineSpacing(Ui.dp(3), 1f);
            r.line2.setPadding(0, Ui.dp(10), 0, 0);
            r.line2.setAutoLinkMask(android.text.util.Linkify.WEB_URLS);
            r.line2.setLinkTextColor(Ui.LINK);
            Ui.lines(r.line2, 8);
            r.line2.setOnClickListener(v -> ((TextView) v).setMaxLines(((TextView) v).getMaxLines() == 8 ? Integer.MAX_VALUE : 8));
            root.addView(r.line2, Ui.lin(Ui.MATCH, Ui.WRAP));
            r.images = Ui.column(c);
            root.addView(r.images, Ui.lin(Ui.MATCH, Ui.WRAP));
            r.badge = Ui.text(c, 12, Ui.MUTE, false);
            r.badge.setPadding(0, Ui.dp(10), 0, 0);
            root.addView(r.badge, Ui.lin(Ui.WRAP, Ui.WRAP));
            View line = Ui.divider(c);
            LinearLayout.LayoutParams dl = Ui.lin(Ui.MATCH, Math.max(1, Ui.dp(0.75f)));
            dl.topMargin = Ui.dp(14);
            root.addView(line, dl);
            root.setTag(r);
            return root;
        }

        private ImageView moreButton(Context c) {
            ImageView more = Ui.icon(c, R.drawable.ic_more, Ui.MUTE);
            more.setBackground(Ui.rippleRound());
            more.setContentDescription("More");
            more.setVisibility(GONE);
            return more;
        }

        private void set(TextView t, String s) {
            t.setText(s == null ? "" : s);
            t.setVisibility(s == null || s.isEmpty() ? GONE : VISIBLE);
        }

        private void badge(TextView b, Core.Item item) {
            if (item.live) {
                b.setText("LIVE");
                b.setBackground(Ui.rounded(0xFFCC0000, 4));
                b.setVisibility(VISIBLE);
            } else if (item.durationS >= 0) {
                b.setText(Ui.duration(item.durationS));
                b.setBackground(Ui.rounded(0xCC000000, 4));
                b.setVisibility(VISIBLE);
            } else {
                b.setVisibility(GONE);
            }
        }

        private TextView newBadge(Context c) {
            TextView b = Ui.text(c, 11, Ui.TEXT, true);
            b.setPadding(Ui.dp(5), Ui.dp(2), Ui.dp(5), Ui.dp(2));
            b.setBackground(Ui.rounded(0xCC000000, 4));
            return b;
        }

        /** Full-width thumbnail with the title under it. */
        private View bigRow() {
            Context c = getContext();
            Row r = new Row();
            LinearLayout root = Ui.column(c);
            root.setForeground(Ui.ripple());
            root.setPadding(0, 0, 0, Ui.dp(20));
            RatioFrame thumb = new RatioFrame(c);
            thumb.setBackgroundColor(Ui.PANEL);
            r.image = new ImageView(c);
            r.image.setScaleType(ImageView.ScaleType.CENTER_CROP);
            thumb.addView(r.image, new LayoutParams(Ui.MATCH, Ui.MATCH));
            r.badge = newBadge(c);
            LayoutParams bl = Ui.frame(Ui.WRAP, Ui.WRAP, Gravity.BOTTOM | Gravity.END);
            bl.setMargins(0, 0, Ui.dp(8), Ui.dp(8));
            thumb.addView(r.badge, bl);
            r.watch = WatchLine.in(thumb);
            root.addView(thumb, Ui.lin(Ui.MATCH, Ui.WRAP));
            LinearLayout info = Ui.row(c);
            info.setGravity(Gravity.TOP);
            info.setPadding(Ui.dp(12), Ui.dp(12), Ui.dp(4), 0);
            r.avatar = new ImageView(c);
            r.avatar.setScaleType(ImageView.ScaleType.CENTER_CROP);
            r.avatar.setBackgroundColor(Ui.PANEL);
            Ui.clipCircle(r.avatar);
            LinearLayout.LayoutParams al = Ui.lin(Ui.dp(36), Ui.dp(36));
            al.rightMargin = Ui.dp(12);
            info.addView(r.avatar, al);
            LinearLayout texts = Ui.column(c);
            r.title = Ui.lines(Ui.text(c, 15, Ui.TEXT, false), 2);
            r.title.setLineSpacing(Ui.dp(2), 1f);
            texts.addView(r.title, Ui.lin(Ui.MATCH, Ui.WRAP));
            r.line1 = Ui.lines(Ui.text(c, 12, Ui.MUTE, false), 2);
            r.line1.setPadding(0, Ui.dp(5), 0, 0);
            texts.addView(r.line1, Ui.lin(Ui.MATCH, Ui.WRAP));
            info.addView(texts, Ui.weight(Ui.WRAP));
            r.more = moreButton(c);
            r.more.setImageResource(R.drawable.ic_more_h);
            r.more.setImageTintList(android.content.res.ColorStateList.valueOf(Ui.TEXT));
            LinearLayout.LayoutParams ml = Ui.lin(Ui.dp(40), Ui.dp(32));
            ml.leftMargin = Ui.dp(4);
            ml.topMargin = -Ui.dp(6);
            info.addView(r.more, ml);
            root.addView(info, Ui.lin(Ui.MATCH, Ui.WRAP));
            r.line2 = r.line1;
            root.setTag(r);
            return root;
        }

        /** Small thumbnail with the text beside it (also used for playlists). */
        private View sideRow(boolean playlist) {
            Context c = getContext();
            Row r = new Row();
            LinearLayout root = Ui.row(c);
            root.setGravity(Gravity.TOP);
            root.setForeground(Ui.ripple());
            root.setPadding(Ui.dp(12), Ui.dp(7), Ui.dp(12), Ui.dp(7));
            RatioFrame thumb = new RatioFrame(c);
            thumb.setBackgroundColor(Ui.PANEL);
            Ui.clipRound(thumb, 8);
            r.image = new ImageView(c);
            r.image.setScaleType(ImageView.ScaleType.CENTER_CROP);
            thumb.addView(r.image, new LayoutParams(Ui.MATCH, Ui.MATCH));
            r.badge = newBadge(c);
            LayoutParams bl = Ui.frame(Ui.WRAP, Ui.WRAP, Gravity.BOTTOM | Gravity.END);
            bl.setMargins(0, 0, Ui.dp(5), Ui.dp(5));
            thumb.addView(r.badge, bl);
            r.watch = WatchLine.in(thumb);
            root.addView(thumb, Ui.lin(Ui.dp(160), Ui.WRAP));
            LinearLayout col = Ui.column(c);
            col.setPadding(Ui.dp(12), 0, 0, 0);
            r.title = Ui.lines(Ui.text(c, 14, Ui.TEXT, true), playlist ? 2 : 3);
            r.title.setLineSpacing(Ui.dp(2.5f), 1f);
            col.addView(r.title, Ui.lin(Ui.MATCH, Ui.WRAP));
            r.line1 = Ui.lines(Ui.text(c, 12, Ui.MUTE, false), 1);
            r.line1.setPadding(0, Ui.dp(6), 0, 0);
            col.addView(r.line1, Ui.lin(Ui.MATCH, Ui.WRAP));
            r.line2 = Ui.lines(Ui.text(c, 12, Ui.MUTE, false), 1);
            r.line2.setPadding(0, Ui.dp(3), 0, 0);
            col.addView(r.line2, Ui.lin(Ui.MATCH, Ui.WRAP));
            root.addView(col, Ui.weight(Ui.WRAP));
            if (!playlist) {
                r.more = moreButton(c);
                LinearLayout.LayoutParams ml = Ui.lin(Ui.dp(32), Ui.dp(36));
                ml.rightMargin = -Ui.dp(8);
                root.addView(r.more, ml);
            }
            root.setTag(r);
            return root;
        }

        private View channelRow() {
            Context c = getContext();
            Row r = new Row();
            LinearLayout root = Ui.row(c);
            root.setForeground(Ui.ripple());
            root.setPadding(Ui.dp(16), Ui.dp(10), Ui.dp(8), Ui.dp(10));
            r.image = new ImageView(c);
            r.image.setScaleType(ImageView.ScaleType.CENTER_CROP);
            r.image.setBackgroundColor(Ui.PANEL);
            Ui.clipCircle(r.image);
            root.addView(r.image, Ui.lin(Ui.dp(48), Ui.dp(48)));
            LinearLayout col = Ui.column(c);
            col.setPadding(Ui.dp(14), 0, 0, 0);
            r.title = Ui.lines(Ui.text(c, 15, Ui.TEXT, true), 1);
            col.addView(r.title, Ui.lin(Ui.MATCH, Ui.WRAP));
            r.line1 = Ui.lines(Ui.text(c, 12, Ui.MUTE, false), 1);
            r.line1.setPadding(0, Ui.dp(4), 0, 0);
            col.addView(r.line1, Ui.lin(Ui.MATCH, Ui.WRAP));
            r.line2 = r.line1;
            r.badge = r.line1;
            root.addView(col, Ui.weight(Ui.WRAP));
            root.addView(Ui.icon(c, R.drawable.ic_chevron, Ui.MUTE), Ui.lin(Ui.dp(40), Ui.dp(40)));
            root.setTag(r);
            return root;
        }
    }
}
