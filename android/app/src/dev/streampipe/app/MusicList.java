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
import android.widget.HorizontalScrollView;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ListView;
import android.widget.TextView;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.List;

/**
 * A YouTube Music page: shelves of cards side by side, song rows, Quick picks (song rows in
 * pages of four that scroll sideways), grids, mood and genre buttons and text, as the core
 * sends them (see core/src/music.rs). More shelves or rows load as the end comes near.
 */
final class MusicList extends FrameLayout {
    interface Host {
        /** An entry was tapped; `list` is the shelf it is in (for playing on from it). */
        void open(Core.Item item, List<Core.Item> list, int index, boolean listIsAlbum);

        /** The ⋮ button or a long press. */
        void menu(Core.Item item);
    }

    interface Source {
        void first(Done done);
    }

    interface Done {
        /** `page` is the core's page JSON; `error` is null on success. */
        void page(JSONObject page, String error);
    }

    /** Called with each first page, so a screen can build its header. */
    interface PageListener {
        void onPage(JSONObject page);
    }

    static final class Shelf {
        String title = "";
        String style = "list";
        String text;
        Core.Item more;
        final List<Core.Item> items = new ArrayList<>();

        static Shelf parse(JSONObject o) {
            Shelf s = new Shelf();
            s.title = o.optString("title");
            s.style = o.optString("style", "list");
            s.text = o.isNull("text") ? null : o.optString("text", null);
            JSONObject more = o.optJSONObject("more");
            s.more = more == null ? null : Core.Item.music(more);
            JSONArray a = o.optJSONArray("items");
            for (int i = 0; a != null && i < a.length(); i++) {
                JSONObject it = a.optJSONObject(i);
                if (it != null && !it.optString("id").isEmpty()) {
                    s.items.add(Core.Item.music(it));
                }
            }
            return s;
        }
    }

    private static final int TITLE = 0, CAROUSEL = 1, SONG = 2, GRID = 3, BUTTONS = 4, TEXT = 5, PAGES = 6;

    private static final class Row {
        final int type;
        final Shelf shelf;
        final int index;

        Row(int type, Shelf shelf, int index) {
            this.type = type;
            this.shelf = shelf;
            this.index = index;
        }
    }

    private final Host host;
    private final ListView list;
    private final View busy;
    private final LinearLayout message;
    private final TextView messageText;
    private final View footer;
    private final List<Shelf> shelves = new ArrayList<>();
    private final List<Row> rows = new ArrayList<>();
    private final Adapter adapter = new Adapter();
    private Source source;
    private PageListener pageListener;
    private String emptyText = "Nothing here yet.";
    private long next = -1;
    private boolean loadingMore;
    private int generation;
    /** The page is an album or playlist: its song list plays as one. */
    boolean albumPage;

    MusicList(Context c, Host host) {
        super(c);
        this.host = host;
        list = new ListView(c);
        list.setDivider(null);
        list.setSelector(new ColorDrawable(Color.TRANSPARENT));
        list.setVerticalScrollBarEnabled(false);
        list.setClipToPadding(false);
        list.setPadding(0, 0, 0, Ui.dp(16));
        addView(list, new LayoutParams(Ui.MATCH, Ui.MATCH));
        FrameLayout f = new FrameLayout(c);
        f.addView(Ui.spinner(c), Ui.frame(Ui.dp(28), Ui.dp(28), Gravity.CENTER));
        f.setPadding(0, Ui.dp(14), 0, Ui.dp(14));
        f.setVisibility(GONE);
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
        message.addView(messageText, Ui.lin(Ui.WRAP, Ui.WRAP));
        TextView retry = Ui.button(c, "Try again", v -> load());
        LinearLayout.LayoutParams rl = Ui.lin(Ui.WRAP, Ui.WRAP);
        rl.topMargin = Ui.dp(16);
        message.addView(retry, rl);
        message.setVisibility(GONE);
        addView(message, Ui.frame(Ui.MATCH, Ui.WRAP, Gravity.CENTER));
        list.setOnScrollListener(new AbsListView.OnScrollListener() {
            @Override
            public void onScrollStateChanged(AbsListView view, int state) {}

            @Override
            public void onScroll(AbsListView view, int first, int visible, int total) {
                if (total > 0 && first + visible >= total - 4) {
                    loadMore();
                }
            }
        });
    }

    void setHeader(View header) {
        list.addHeaderView(header, null, false);
    }

    void setSource(Source s) {
        source = s;
        list.setAdapter(adapter);
    }

    void setPageListener(PageListener l) {
        pageListener = l;
    }

    void setEmptyText(String text) {
        emptyText = text;
    }

    void scrollToTop() {
        list.smoothScrollToPosition(0);
    }

    /** The songs and videos of the first list shelf (an album's or playlist's tracks). */
    List<Core.Item> tracks() {
        for (Shelf s : shelves) {
            if ("list".equals(s.style)) {
                return new ArrayList<>(s.items);
            }
        }
        return new ArrayList<>();
    }

    void load() {
        if (source == null) {
            return;
        }
        final int mine = ++generation;
        dropCursor();
        shelves.clear();
        rebuild();
        message.setVisibility(GONE);
        busy.setVisibility(VISIBLE);
        source.first((page, error) -> {
            if (mine != generation) {
                return;
            }
            busy.setVisibility(GONE);
            if (error != null || page == null) {
                showMessage(error == null ? "No answer." : error, true);
                return;
            }
            if (pageListener != null) {
                pageListener.onPage(page);
            }
            add(page);
            prefetchFirst();
            if (shelves.isEmpty() && emptyText != null && !emptyText.isEmpty()) {
                showMessage(emptyText, false);
            }
        });
    }

    /** Get the first songs of the page ready (the ones most likely tapped), as feeds do. */
    private void prefetchFirst() {
        if (!Core.get().prefetch) {
            return;
        }
        org.json.JSONArray ids = new org.json.JSONArray();
        for (Shelf s : shelves) {
            for (Core.Item i : s.items) {
                if (ids.length() >= 3) {
                    break;
                }
                if ("song".equals(i.kind) || "video".equals(i.kind)) {
                    ids.put(i.id);
                }
            }
        }
        if (ids.length() > 0) {
            final org.json.JSONArray send = ids;
            postDelayed(() -> Core.get().send("prefetch_streams", "ids", send), 800);
        }
    }

    void showMessage(String text, boolean retry) {
        busy.setVisibility(GONE);
        messageText.setText(text);
        message.getChildAt(1).setVisibility(retry ? VISIBLE : GONE);
        message.setVisibility(VISIBLE);
    }

    private void add(JSONObject page) {
        JSONArray a = page.optJSONArray("shelves");
        boolean append = page.optBoolean("append");
        for (int i = 0; a != null && i < a.length(); i++) {
            Shelf s = Shelf.parse(a.optJSONObject(i));
            Shelf last = shelves.isEmpty() ? null : shelves.get(shelves.size() - 1);
            if (append && last != null) {
                last.items.addAll(s.items);
            } else if (!s.items.isEmpty() || s.text != null) {
                shelves.add(s);
            }
        }
        next = page.isNull("next") ? -1 : page.optLong("next", -1);
        rebuild();
    }

    private void rebuild() {
        rows.clear();
        for (Shelf s : shelves) {
            if (!s.title.isEmpty() || s.more != null) {
                rows.add(new Row(TITLE, s, 0));
            }
            switch (s.style) {
                case "carousel":
                    rows.add(new Row(CAROUSEL, s, 0));
                    break;
                case "pages":
                    rows.add(new Row(PAGES, s, 0));
                    break;
                case "grid":
                    for (int i = 0; i < s.items.size(); i += 2) {
                        rows.add(new Row(GRID, s, i));
                    }
                    break;
                case "buttons":
                    for (int i = 0; i < s.items.size(); i += 2) {
                        rows.add(new Row(BUTTONS, s, i));
                    }
                    break;
                case "text":
                    rows.add(new Row(TEXT, s, 0));
                    break;
                default:
                    for (int i = 0; i < s.items.size(); i++) {
                        rows.add(new Row(SONG, s, i));
                    }
            }
        }
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
            JSONObject ok = e == null ? null : e.optJSONObject("ok");
            if (ok != null) {
                add(ok);
            }
        }, "music_more", "cursor", cursor);
    }

    private void dropCursor() {
        next = -1;
    }

    /** The answer of a Music command, as a page. */
    static void deliver(JSONObject e, String err, Done done) {
        JSONObject ok = e == null ? null : e.optJSONObject("ok");
        done.page(ok, ok == null ? (err == null ? "No answer." : err) : null);
    }

    // ------------------------------------------------------------------ views

    private void tap(Shelf s, int index) {
        Core.Item item = s.items.get(index);
        host.open(item, s.items, index, albumPage && "list".equals(s.style));
    }

    /** A card: picture (round for artists, wide for videos), title and line under it. */
    private View card(final Shelf s, final int index, int widthPx) {
        Context c = getContext();
        final Core.Item item = s.items.get(index);
        LinearLayout col = Ui.column(c);
        boolean wide = "video".equals(item.kind) || "episode".equals(item.kind);
        boolean round = "artist".equals(item.kind);
        FrameLayout frame = new FrameLayout(c);
        frame.setBackgroundColor(Ui.PANEL);
        if (round) {
            Ui.clipCircle(frame);
        } else {
            Ui.clipRound(frame, 6);
        }
        ImageView img = new ImageView(c);
        img.setScaleType(ImageView.ScaleType.CENTER_CROP);
        frame.addView(img, new LayoutParams(Ui.MATCH, Ui.MATCH));
        Images.load(img, item.thumb);
        int h = wide ? Math.round(widthPx * 9f / 16f) : widthPx;
        col.addView(frame, Ui.lin(widthPx, h));
        TextView t = Ui.lines(Ui.text(c, 13, Ui.TEXT, true), 2);
        t.setText(item.title);
        t.setPadding(0, Ui.dp(7), 0, 0);
        if (round) {
            t.setGravity(Gravity.CENTER_HORIZONTAL);
        }
        col.addView(t, Ui.lin(widthPx, Ui.WRAP));
        if (item.published != null) {
            TextView sub = Ui.lines(Ui.text(c, 12, Ui.MUTE, false), 2);
            sub.setText(item.published);
            sub.setPadding(0, Ui.dp(3), 0, 0);
            if (round) {
                sub.setGravity(Gravity.CENTER_HORIZONTAL);
            }
            col.addView(sub, Ui.lin(widthPx, Ui.WRAP));
        }
        col.setForeground(Ui.ripple());
        col.setOnClickListener(v -> tap(s, index));
        col.setOnLongClickListener(v -> {
            host.menu(item);
            return true;
        });
        return col;
    }

    /** A song row: square picture, title, artists and the rest, length, ⋮. */
    private View song(final Shelf s, final int index) {
        Context c = getContext();
        final Core.Item item = s.items.get(index);
        LinearLayout row = Ui.row(c);
        row.setPadding(Ui.dp(16), Ui.dp(7), Ui.dp(4), Ui.dp(7));
        row.setForeground(Ui.ripple());
        boolean wide = "video".equals(item.kind) && !albumPage;
        FrameLayout frame = new FrameLayout(c);
        frame.setBackgroundColor(Ui.PANEL);
        Ui.clipRound(frame, "artist".equals(item.kind) ? 26 : 4);
        ImageView img = new ImageView(c);
        img.setScaleType(ImageView.ScaleType.CENTER_CROP);
        frame.addView(img, new LayoutParams(Ui.MATCH, Ui.MATCH));
        Images.load(img, item.thumb);
        row.addView(frame, Ui.lin(Ui.dp(wide ? 84 : 50), Ui.dp(wide ? 47 : 50)));
        LinearLayout texts = Ui.column(c);
        texts.setPadding(Ui.dp(14), 0, Ui.dp(4), 0);
        TextView t = Ui.lines(Ui.text(c, 15, Ui.TEXT, false), 1);
        t.setText(item.title);
        texts.addView(t);
        String sub = item.published;
        if (sub != null && !sub.isEmpty()) {
            TextView st = Ui.lines(Ui.text(c, 12, Ui.MUTE, false), 1);
            st.setText(("song".equals(item.kind) || "video".equals(item.kind) || "episode".equals(item.kind)) ? sub : kindLabel(item) + sub);
            st.setPadding(0, Ui.dp(3), 0, 0);
            texts.addView(st);
        }
        row.addView(texts, Ui.weight(Ui.WRAP));
        if (item.playable()) {
            ImageView more = Ui.icon(c, R.drawable.ic_more, Ui.MUTE);
            more.setBackground(Ui.rippleRound());
            more.setContentDescription("More");
            more.setOnClickListener(v -> host.menu(item));
            row.addView(more, Ui.lin(Ui.dp(40), Ui.dp(44)));
        } else {
            row.addView(Ui.icon(c, R.drawable.ic_chevron, Ui.MUTE), Ui.lin(Ui.dp(40), Ui.dp(44)));
        }
        row.setOnClickListener(v -> tap(s, index));
        row.setOnLongClickListener(v -> {
            host.menu(item);
            return true;
        });
        return row;
    }

    private static String kindLabel(Core.Item item) {
        if (item.published != null && (item.published.startsWith("Album") || item.published.startsWith("Single")
                || item.published.startsWith("Playlist") || item.published.startsWith("Artist") || item.published.startsWith("EP"))) {
            return "";
        }
        switch (item.kind) {
            case "album":
                return "Album • ";
            case "single":
                return "Single • ";
            case "playlist":
                return "Playlist • ";
            case "artist":
                return "Artist • ";
            case "podcast":
                return "Podcast • ";
            default:
                return "";
        }
    }

    private View button(final Shelf s, final int index) {
        Context c = getContext();
        final Core.Item item = s.items.get(index);
        LinearLayout b = Ui.row(c);
        b.setBackground(new android.graphics.drawable.RippleDrawable(
                android.content.res.ColorStateList.valueOf(Ui.RIPPLE), Ui.rounded(Ui.PANEL, 8), null));
        View stripe = new View(c);
        stripe.setBackground(Ui.rounded(item.color != 0 ? item.color | 0xFF000000 : Ui.ACCENT, 3));
        b.addView(stripe, Ui.lin(Ui.dp(6), Ui.dp(40)));
        TextView t = Ui.lines(Ui.text(c, 14, Ui.TEXT, true), 1);
        t.setText(item.title);
        t.setPadding(Ui.dp(12), 0, Ui.dp(8), 0);
        b.addView(t, Ui.weight(Ui.WRAP));
        b.setOnClickListener(v -> tap(s, index));
        return b;
    }

    private final class Adapter extends BaseAdapter {
        @Override
        public int getCount() {
            return rows.size();
        }

        @Override
        public Object getItem(int position) {
            return rows.get(position);
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
            return rows.get(position).type;
        }

        @Override
        public boolean isEnabled(int position) {
            return false;
        }

        @Override
        public View getView(int position, View convert, ViewGroup parent) {
            // Rows are cheap and few; they are made fresh so carousels keep their own state.
            final Row r = rows.get(position);
            Context c = getContext();
            int width = Math.max(parent.getWidth(), Ui.dp(320));
            switch (r.type) {
                case TITLE: {
                    LinearLayout t = Ui.row(c);
                    t.setPadding(Ui.dp(16), Ui.dp(20), Ui.dp(8), Ui.dp(10));
                    TextView title = Ui.lines(Ui.text(c, 20, Ui.TEXT, true), 2);
                    title.setText(r.shelf.title);
                    t.addView(title, Ui.weight(Ui.WRAP));
                    if (r.shelf.more != null) {
                        TextView more = ListScreen.pillButton(c, "More", v -> host.open(r.shelf.more, null, 0, false));
                        t.addView(more, Ui.lin(Ui.WRAP, Ui.WRAP));
                    }
                    return t;
                }
                case CAROUSEL: {
                    LinearLayout line = Ui.row(c);
                    line.setGravity(Gravity.TOP);
                    line.setPadding(Ui.dp(16), 0, Ui.dp(4), Ui.dp(6));
                    int w = Ui.dp(150);
                    for (int i = 0; i < r.shelf.items.size(); i++) {
                        Core.Item it = r.shelf.items.get(i);
                        int cw = "video".equals(it.kind) || "episode".equals(it.kind) ? Ui.dp(240) : w;
                        LinearLayout.LayoutParams lp = Ui.lin(cw, Ui.WRAP);
                        lp.rightMargin = Ui.dp(12);
                        line.addView(card(r.shelf, i, cw), lp);
                    }
                    HorizontalScrollView scroll = new HorizontalScrollView(c);
                    scroll.setHorizontalScrollBarEnabled(false);
                    scroll.addView(line);
                    return scroll;
                }
                case PAGES: {
                    // Quick picks: columns of four rows, each nearly the screen's width.
                    LinearLayout line = Ui.row(c);
                    line.setGravity(Gravity.TOP);
                    int colWidth = Math.round(width * 0.88f);
                    for (int start = 0; start < r.shelf.items.size(); start += 4) {
                        LinearLayout col = Ui.column(c);
                        for (int i = start; i < Math.min(start + 4, r.shelf.items.size()); i++) {
                            col.addView(song(r.shelf, i), Ui.lin(Ui.MATCH, Ui.WRAP));
                        }
                        line.addView(col, Ui.lin(colWidth, Ui.WRAP));
                    }
                    HorizontalScrollView scroll = new HorizontalScrollView(c);
                    scroll.setHorizontalScrollBarEnabled(false);
                    scroll.addView(line);
                    return scroll;
                }
                case GRID: {
                    LinearLayout line = Ui.row(c);
                    line.setGravity(Gravity.TOP);
                    line.setPadding(Ui.dp(16), Ui.dp(4), Ui.dp(16), Ui.dp(10));
                    int w = (width - Ui.dp(16 * 2 + 12)) / 2;
                    for (int i = r.index; i < Math.min(r.index + 2, r.shelf.items.size()); i++) {
                        LinearLayout.LayoutParams lp = Ui.lin(w, Ui.WRAP);
                        if (i == r.index) {
                            lp.rightMargin = Ui.dp(12);
                        }
                        line.addView(card(r.shelf, i, w), lp);
                    }
                    return line;
                }
                case BUTTONS: {
                    LinearLayout line = Ui.row(c);
                    line.setPadding(Ui.dp(16), Ui.dp(4), Ui.dp(16), Ui.dp(4));
                    for (int i = r.index; i < Math.min(r.index + 2, r.shelf.items.size()); i++) {
                        LinearLayout.LayoutParams lp = new LinearLayout.LayoutParams(0, Ui.dp(48), 1f);
                        if (i == r.index) {
                            lp.rightMargin = Ui.dp(10);
                        }
                        line.addView(button(r.shelf, i), lp);
                    }
                    if (r.index + 1 >= r.shelf.items.size()) {
                        line.addView(new View(c), new LinearLayout.LayoutParams(0, Ui.dp(48), 1f));
                    }
                    return line;
                }
                case TEXT: {
                    final TextView t = Ui.text(c, 14, Ui.MUTE, false);
                    t.setText(r.shelf.text);
                    t.setLineSpacing(Ui.dp(4), 1f);
                    t.setPadding(Ui.dp(16), 0, Ui.dp(16), Ui.dp(12));
                    Ui.lines(t, 4);
                    t.setOnClickListener(v -> t.setMaxLines(t.getMaxLines() == 4 ? Integer.MAX_VALUE : 4));
                    return t;
                }
                default:
                    return song(r.shelf, r.index);
            }
        }
    }
}
