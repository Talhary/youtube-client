package dev.streampipe.app;

import android.content.Context;
import android.os.Handler;
import android.os.Looper;
import android.text.Editable;
import android.text.SpannableString;
import android.text.Spanned;
import android.text.TextWatcher;
import android.text.style.ForegroundColorSpan;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputMethodManager;
import android.widget.ArrayAdapter;
import android.widget.EditText;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ListView;
import android.widget.ScrollView;
import android.widget.Switch;
import android.widget.TextView;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.List;

/** What screens can ask the activity to do. */
interface Nav {
    void push(Screen screen);

    void pop();

    /** Open a video, channel or playlist. */
    void open(Core.Item item);

    void signIn();

    void signOut();

    boolean backgroundPlay();

    void setBackgroundPlay(boolean on);

    String adRulesText();

    /** Play stream URLs directly (true) or through YouTube's page (false). */
    boolean directPlayer();

    void setDirectPlayer(boolean on);

    /** "system", "light" or "dark". */
    String theme();

    /** Save the theme and redraw the app with it. */
    void setTheme(String theme);

    /** Play `item` after the current video (`next`), or at the end of the queue. */
    void queue(Core.Item item, boolean next);

    /** YouTube Music (true) or YouTube. */
    void setMusicMode(boolean on);

    boolean musicMode();

    /** The play queue changed (the watch screen lists it under Up next). */
    void queueChanged();

    void toast(String text);

    /** Show the Shorts tab starting at this Short. */
    void openShort(String id);
}

/** One page of the app: a view plus a few hooks. */
abstract class Screen {
    View view;

    /** Sign-in state or settings changed. */
    void onCoreChanged() {}

    /** The tab this screen belongs to was tapped while already showing. */
    void onReselected() {}

    /** The screen became the visible one. */
    void onShown() {}

    /** The screen is no longer the visible one (another tab, or the app left the screen). */
    void onHidden() {}

    /** A 56dp top bar: optional back button, a title view, then action buttons. */
    static LinearLayout bar(Context c, final Nav nav, boolean back, View title, View... actions) {
        LinearLayout bar = Ui.row(c);
        bar.setBackgroundColor(Ui.BG);
        bar.setPadding(back ? Ui.dp(4) : Ui.dp(16), 0, Ui.dp(4), 0);
        if (back) {
            bar.addView(Ui.iconButton(c, R.drawable.ic_back, "Back", v -> nav.pop()));
        }
        bar.addView(title, Ui.weight(Ui.WRAP));
        for (View a : actions) {
            bar.addView(a);
        }
        bar.setLayoutParams(Ui.lin(Ui.MATCH, Ui.dp(56)));
        return bar;
    }

    static TextView barTitle(Context c, String text) {
        TextView t = Ui.lines(Ui.text(c, 18, Ui.TEXT, true), 1);
        t.setText(text);
        return t;
    }

    /** A row of choice chips (as above YouTube's feeds); the chosen one is filled. */
    static android.widget.HorizontalScrollView chips(Context c, String[] labels, int selected, final Sheet.Pick pick) {
        final LinearLayout row = Ui.row(c);
        row.setPadding(Ui.dp(12), Ui.dp(4), Ui.dp(12), Ui.dp(12));
        final int[] chosen = {selected};
        final Runnable paint = () -> {
            for (int i = 0; i < row.getChildCount(); i++) {
                TextView t = (TextView) row.getChildAt(i);
                boolean on = i == chosen[0];
                t.setTextColor(on ? (Ui.light ? 0xFFFFFFFF : 0xFF0F0F0F) : Ui.TEXT);
                t.setBackground(new android.graphics.drawable.RippleDrawable(android.content.res.ColorStateList.valueOf(Ui.RIPPLE),
                        Ui.rounded(on ? (Ui.light ? 0xFF0F0F0F : 0xFFF1F1F1) : Ui.PANEL, 8), null));
            }
        };
        for (int i = 0; i < labels.length; i++) {
            final int index = i;
            TextView t = Ui.text(c, 14, Ui.TEXT, true);
            t.setText(labels[i]);
            t.setPadding(Ui.dp(12), Ui.dp(8), Ui.dp(12), Ui.dp(8));
            t.setOnClickListener(v -> {
                if (chosen[0] != index) {
                    chosen[0] = index;
                    paint.run();
                }
                pick.picked(index);
            });
            LinearLayout.LayoutParams lp = Ui.lin(Ui.WRAP, Ui.WRAP);
            lp.rightMargin = Ui.dp(8);
            row.addView(t, lp);
        }
        paint.run();
        android.widget.HorizontalScrollView scroll = new android.widget.HorizontalScrollView(c);
        scroll.setHorizontalScrollBarEnabled(false);
        scroll.addView(row);
        return scroll;
    }

    static LinearLayout page(Context c, View bar, View body) {
        LinearLayout col = Ui.column(c);
        col.setBackgroundColor(Ui.BG);
        col.addView(bar);
        col.addView(Ui.divider(c));
        col.addView(body, new LinearLayout.LayoutParams(Ui.MATCH, 0, 1f));
        return col;
    }
}

/** Home and Subscriptions: a feed under the app bar. Home has YouTube's explore chips. */
final class FeedScreen extends Screen {
    /** Home's chips: the personal feed, then YouTube's destination pages. */
    private static final String[] CHIPS = {"All", "Trending", "Music", "Gaming", "News", "Sports", "Live", "Podcasts"};
    private static final String[] CATEGORIES = {"", "trending", "music", "gaming", "news", "sports", "live", "podcasts"};

    private final boolean subscriptions;
    private final FeedList list;
    private final View bell;
    private Boolean loadedFor;
    private String category = "";

    FeedScreen(final Context c, final Nav nav, boolean subscriptions) {
        this.subscriptions = subscriptions;
        this.nav = nav;
        View logo = logo(c, nav);
        list = new FeedList(c, false, nav::open);
        list.setPreview(true);
        // "Not interested" applies to what YouTube recommends.
        list.setMenu(nav, !subscriptions);
        if (!subscriptions) {
            list.setHeader(chips(c, CHIPS, 0, i -> {
                String wanted = CATEGORIES[i];
                if (!wanted.equals(category)) {
                    category = wanted;
                    list.load();
                } else {
                    list.scrollToTop();
                }
            }));
        } else {
            channels = Ui.row(c);
            channels.setPadding(Ui.dp(8), Ui.dp(4), Ui.dp(8), Ui.dp(8));
            android.widget.HorizontalScrollView hs = new android.widget.HorizontalScrollView(c);
            hs.setHorizontalScrollBarEnabled(false);
            hs.addView(channels);
            list.setHeader(hs);
        }
        list.setEmptyText(subscriptions ? "No recent videos from your subscriptions." : "Nothing to show right now.");
        list.setSource(r -> {
            if (!subscriptions && !category.isEmpty()) {
                Core.get().call((e, err) -> FeedList.deliver(e, err, r), "explore", "category", category);
            } else {
                Core.get().call((e, err) -> {
                    FeedList.deliver(e, err, r);
                    if (!subscriptions && err == null) {
                        loadShelf(c);
                    }
                }, subscriptions ? "feed" : "home");
            }
            if (subscriptions) {
                loadChannels(c);
            }
        });
        bell = Ui.iconButton(c, R.drawable.ic_bell_outline, "Notifications", v -> nav.push(ListScreen.notifications(c, nav)));
        View bar = bar(c, nav, false, logo, bell,
                Ui.iconButton(c, R.drawable.ic_search_outline, "Search", v -> nav.push(new SearchScreen(c, nav))));
        LinearLayout col = Ui.column(c);
        col.setBackgroundColor(Ui.BG);
        col.addView(bar);
        col.addView(list, new LinearLayout.LayoutParams(Ui.MATCH, 0, 1f));
        view = col;
        onCoreChanged();
    }

    private final Nav nav;
    private LinearLayout channels;

    /** YouTube's logo: the red play button and the word mark. Tapping it switches to YouTube Music. */
    static View logo(Context c, final Nav nav) {
        LinearLayout l = Ui.row(c);
        ImageView mark = new ImageView(c);
        mark.setImageResource(R.drawable.ic_yt_logo);
        mark.setScaleType(ImageView.ScaleType.FIT_CENTER);
        l.addView(mark, Ui.lin(Ui.dp(30), Ui.dp(22)));
        TextView word = Ui.text(c, 20, Ui.TEXT, false);
        word.setText("YouTube");
        word.setTypeface(android.graphics.Typeface.create("sans-serif-condensed", android.graphics.Typeface.BOLD));
        word.setLetterSpacing(-0.04f);
        word.setPadding(Ui.dp(2), 0, 0, Ui.dp(1));
        l.addView(word, Ui.lin(Ui.WRAP, Ui.WRAP));
        l.setContentDescription("YouTube. Tap to switch to YouTube Music");
        l.setOnClickListener(v -> nav.setMusicMode(true));
        FrameLayout wrap = new FrameLayout(c);
        wrap.addView(l, Ui.frame(Ui.WRAP, Ui.WRAP, Gravity.CENTER_VERTICAL | Gravity.START));
        return wrap;
    }

    /** The Shorts shelf YouTube puts in Home, from the same home page. */
    private void loadShelf(final Context c) {
        Core.get().call((e, err) -> {
            JSONArray a = e == null ? null : e.optJSONArray("ok");
            if (a == null || a.length() == 0) {
                list.setInline(null, 0);
                return;
            }
            list.setInline(shelf(c, nav, a), 2);
        }, "shorts", "op", "shelf");
    }

    static View shelf(final Context c, final Nav nav, JSONArray a) {
        LinearLayout col = Ui.column(c);
        col.setPadding(0, Ui.dp(4), 0, Ui.dp(22));
        LinearLayout head = Ui.row(c);
        head.setPadding(Ui.dp(14), Ui.dp(6), Ui.dp(12), Ui.dp(12));
        ImageView mark = new ImageView(c);
        mark.setImageResource(R.drawable.ic_shorts_logo);
        head.addView(mark, Ui.lin(Ui.dp(28), Ui.dp(28)));
        TextView t = Ui.text(c, 19, Ui.TEXT, true);
        t.setText("Shorts");
        t.setPadding(Ui.dp(10), 0, 0, 0);
        head.addView(t, Ui.weight(Ui.WRAP));
        col.addView(head);
        LinearLayout row = Ui.row(c);
        row.setGravity(Gravity.TOP);
        row.setPadding(Ui.dp(12), 0, Ui.dp(8), 0);
        int screen = Math.round(c.getResources().getDisplayMetrics().widthPixels);
        int w = Math.min(Ui.dp(190), (screen - Ui.dp(12 + 8 + 4)) / 2);
        for (int i = 0; i < a.length(); i++) {
            final JSONObject o = a.optJSONObject(i);
            if (o == null) {
                continue;
            }
            FrameLayout card = new FrameLayout(c);
            card.setBackgroundColor(Ui.PANEL);
            Ui.clipRound(card, 10);
            card.setForeground(Ui.ripple());
            ImageView img = new ImageView(c);
            img.setScaleType(ImageView.ScaleType.CENTER_CROP);
            card.addView(img, new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));
            Images.load(img, o.optString("thumb"));
            View shade = new View(c);
            shade.setBackground(new android.graphics.drawable.GradientDrawable(android.graphics.drawable.GradientDrawable.Orientation.BOTTOM_TOP,
                    new int[] {0xB0000000, 0x00000000}));
            card.addView(shade, Ui.frame(Ui.MATCH, Ui.dp(110), Gravity.BOTTOM));
            LinearLayout text = Ui.column(c);
            text.setPadding(Ui.dp(10), 0, Ui.dp(10), Ui.dp(10));
            TextView title = Ui.lines(Ui.text(c, 14, 0xFFFFFFFF, true), 2);
            title.setText(o.optString("title"));
            title.setShadowLayer(3, 0, 1, 0x99000000);
            title.setLineSpacing(Ui.dp(1), 1f);
            text.addView(title);
            TextView views = Ui.lines(Ui.text(c, 12, 0xFFFFFFFF, false), 1);
            views.setText(o.optString("views"));
            views.setPadding(0, Ui.dp(4), 0, 0);
            text.addView(views);
            card.addView(text, Ui.frame(Ui.MATCH, Ui.WRAP, Gravity.BOTTOM));
            ImageView more = Ui.icon(c, R.drawable.ic_more, 0xFFFFFFFF);
            card.addView(more, Ui.frame(Ui.dp(32), Ui.dp(32), Gravity.TOP | Gravity.END));
            card.setOnClickListener(v -> nav.openShort(o.optString("id")));
            LinearLayout.LayoutParams lp = Ui.lin(w, Math.round(w * 16f / 9f));
            lp.rightMargin = Ui.dp(4);
            row.addView(card, lp);
        }
        android.widget.HorizontalScrollView hs = new android.widget.HorizontalScrollView(c);
        hs.setHorizontalScrollBarEnabled(false);
        hs.addView(row);
        col.addView(hs);
        return col;
    }

    /** Subscriptions: the channels as a row of round pictures above the videos. */
    private void loadChannels(final Context c) {
        if (channels == null || !Core.get().isSignedIn()) {
            return;
        }
        Core.get().call((e, err) -> {
            if (err != null || e == null) {
                return;
            }
            Core.Page page = Core.Page.parse(e.optJSONObject("ok"));
            channels.removeAllViews();
            for (final Core.Item ch : page.items) {
                if (ch.type != Core.Item.CHANNEL) {
                    continue;
                }
                LinearLayout cell = Ui.column(c);
                cell.setGravity(Gravity.CENTER_HORIZONTAL);
                cell.setPadding(Ui.dp(6), Ui.dp(4), Ui.dp(6), Ui.dp(4));
                cell.setBackground(Ui.rippleRound());
                ImageView pic = new ImageView(c);
                pic.setScaleType(ImageView.ScaleType.CENTER_CROP);
                pic.setBackgroundColor(Ui.PANEL);
                Ui.clipCircle(pic);
                Images.load(pic, ch.thumb);
                cell.addView(pic, Ui.lin(Ui.dp(56), Ui.dp(56)));
                TextView n = Ui.lines(Ui.text(c, 11, Ui.MUTE, false), 1);
                n.setText(ch.title);
                n.setGravity(Gravity.CENTER);
                n.setPadding(0, Ui.dp(6), 0, 0);
                cell.addView(n, Ui.lin(Ui.dp(64), Ui.WRAP));
                cell.setOnClickListener(v -> nav.open(ch));
                channels.addView(cell);
            }
        }, "subscriptions");
    }

    private void reload() {
        if (subscriptions && !Core.get().isSignedIn()) {
            list.showMessage("Sign in from the You tab to see videos from your subscriptions.", false);
        } else {
            list.load();
        }
    }

    @Override
    void onCoreChanged() {
        bell.setVisibility(Core.get().isSignedIn() ? View.VISIBLE : View.GONE);
        // Signing in or out changes what these feeds contain.
        Boolean now = Core.get().signedIn;
        if (now != null && !now.equals(loadedFor)) {
            loadedFor = now;
            reload();
        }
    }

    @Override
    void onReselected() {
        // As on YouTube: at the top, a second tap reloads; elsewhere it goes back to the top.
        if (list.atTop()) {
            reload();
        } else {
            list.scrollToTop();
        }
    }
}

/** A pushed page with a title and one list: history, playlists, channels, a channel, a playlist. */
final class ListScreen extends Screen {
    interface Binder {
        /** Load the first page; may fill `header` and change `title` from the answer. */
        void load(FeedList.Result result, LinearLayout header, TextView title);
    }

    ListScreen(Context c, Nav nav, String title, boolean compact, String empty, final Binder binder) {
        final TextView t = barTitle(c, title);
        final LinearLayout header = Ui.column(c);
        FeedList list = new FeedList(c, compact, nav::open);
        list.setMenu(nav, false);
        list.setHeader(header);
        list.setEmptyText(empty);
        list.setSource(r -> binder.load(r, header, t));
        view = page(c, bar(c, nav, true, t), list);
        list.load();
    }

    static ListScreen account(Context c, Nav nav, String title, final String command) {
        return new ListScreen(c, nav, title, true, "Nothing in " + title.toLowerCase(java.util.Locale.ROOT) + " yet.",
                (r, header, t) -> Core.get().call((e, err) -> FeedList.deliver(e, err, r), command));
    }

    /** The bell: what YouTube notified the account about. */
    static ListScreen notifications(Context c, Nav nav) {
        return new ListScreen(c, nav, "Notifications", true, "No notifications.",
                (r, header, t) -> Core.get().call((e, err) -> FeedList.deliver(e, err, r), "notifications"));
    }

    /** Watch history, with YouTube's own clear and pause buttons above it. */
    static ListScreen history(final Context c, final Nav nav) {
        return new ListScreen(c, nav, "History", true, "Nothing in history yet.", (r, header, t) -> {
            Core.get().call((e, err) -> FeedList.deliver(e, err, r), "history");
            if (header.getChildCount() > 0) {
                return;
            }
            final LinearLayout row = Ui.row(c);
            row.setPadding(Ui.dp(12), Ui.dp(6), Ui.dp(12), Ui.dp(6));
            header.addView(row);
            final android.content.SharedPreferences prefs = c.getSharedPreferences("shell", Context.MODE_PRIVATE);
            Core.get().call((e, err) -> {
                JSONObject ok = e == null ? null : e.optJSONObject("ok");
                JSONArray actions = ok == null ? null : ok.optJSONArray("actions");
                for (int i = 0; actions != null && i < actions.length(); i++) {
                    final JSONObject a = actions.optJSONObject(i);
                    if (a == null) {
                        continue;
                    }
                    final String kind = a.optString("kind");
                    final String label = "clear".equals(kind) ? "Clear all watch history"
                            : "pause".equals(kind) ? "Pause watch history" : "Turn on watch history";
                    TextView b = pillButton(c, label, v -> {
                        Sheet s = new Sheet(c, label + "?");
                        s.note("clear".equals(kind)
                                ? "Your YouTube watch history will be cleared from all YouTube apps on all devices, and this app forgets where you stopped in each video."
                                : "pause".equals(kind) ? "YouTube will stop saving the videos you watch, here and in its other apps."
                                : "YouTube will save the videos you watch again.");
                        s.add(R.drawable.ic_check, label, () -> Core.get().call((e2, err2) -> {
                            if (err2 != null) {
                                android.widget.Toast.makeText(c, "YouTube did not take that: " + err2, android.widget.Toast.LENGTH_LONG).show();
                                return;
                            }
                            if ("clear".equals(kind)) {
                                Resume.clear();
                            }
                            prefs.edit().putBoolean("historyPaused", "pause".equals(kind)
                                    || (!"resume".equals(kind) && prefs.getBoolean("historyPaused", false))).apply();
                            android.widget.Toast.makeText(c, "Done", android.widget.Toast.LENGTH_SHORT).show();
                            nav.pop();
                        }, "feedback", "token", a.optString("token")));
                        s.show();
                    });
                    LinearLayout.LayoutParams lp = Ui.lin(Ui.WRAP, Ui.WRAP);
                    lp.rightMargin = Ui.dp(8);
                    row.addView(b, lp);
                }
            }, "history_actions");
        });
    }

    /** A small rounded button with text. */
    static TextView pillButton(Context c, String label, View.OnClickListener onClick) {
        TextView b = Ui.text(c, 13, Ui.TEXT, true);
        b.setText(label);
        b.setPadding(Ui.dp(14), Ui.dp(8), Ui.dp(14), Ui.dp(8));
        b.setBackground(new android.graphics.drawable.RippleDrawable(
                android.content.res.ColorStateList.valueOf(Ui.RIPPLE), Ui.rounded(Ui.PANEL, 18), null));
        b.setOnClickListener(onClick);
        return b;
    }

    static ListScreen channel(final Context c, Nav nav, final String id, String name) {
        return new ListScreen(c, nav, name == null ? "Channel" : name, false, "This channel has no videos.", (r, header, t) ->
                Core.get().call((e, err) -> {
                    JSONObject page = e == null ? null : e.optJSONObject("ok");
                    if (err != null || page == null) {
                        r.page(null, err == null ? "No answer." : err);
                        return;
                    }
                    JSONObject ch = page.optJSONObject("channel");
                    if (ch != null && header.getChildCount() == 0) {
                        t.setText(ch.optString("name"));
                        LinearLayout row = Ui.row(c);
                        row.setPadding(Ui.dp(16), Ui.dp(16), Ui.dp(16), Ui.dp(16));
                        ImageView avatar = new ImageView(c);
                        avatar.setScaleType(ImageView.ScaleType.CENTER_CROP);
                        avatar.setBackgroundColor(Ui.PANEL);
                        Ui.clipCircle(avatar);
                        Images.load(avatar, Core.url(Core.str(ch, "avatar")));
                        row.addView(avatar, Ui.lin(Ui.dp(56), Ui.dp(56)));
                        LinearLayout col = Ui.column(c);
                        col.setPadding(Ui.dp(14), 0, 0, 0);
                        TextView n = Ui.lines(Ui.text(c, 18, Ui.TEXT, true), 1);
                        n.setText(ch.optString("name"));
                        col.addView(n);
                        if (!ch.isNull("subscribers")) {
                            TextView s = Ui.text(c, 12, Ui.MUTE, false);
                            s.setText(Ui.count(ch.optLong("subscribers")) + " subscribers");
                            s.setPadding(0, Ui.dp(5), 0, 0);
                            col.addView(s);
                        }
                        row.addView(col, Ui.weight(Ui.WRAP));
                        row.setGravity(android.view.Gravity.CENTER_VERTICAL);
                        row.addView(subscribeButton(c, id));
                        header.addView(row);
                    }
                    r.page(Core.Page.parse(page.optJSONObject("videos")), null);
                }, "channel", "id", id));
    }

    /** "Subscribe" / "Subscribed" for a channel; shows the state once YouTube has told it. */
    private static TextView subscribeButton(final Context c, final String channelId) {
        final TextView b = Ui.text(c, 13, Ui.ON_ACCENT, true);
        b.setPadding(Ui.dp(16), Ui.dp(9), Ui.dp(16), Ui.dp(9));
        // state[0]: null = not known yet, otherwise whether the user is subscribed.
        final Boolean[] state = {null};
        final Runnable paint = () -> {
            boolean on = Boolean.TRUE.equals(state[0]);
            b.setText(on ? "Subscribed" : "Subscribe");
            b.setTextColor(on ? Ui.TEXT : Ui.ON_ACCENT);
            b.setBackground(new android.graphics.drawable.RippleDrawable(
                    android.content.res.ColorStateList.valueOf(Ui.RIPPLE), Ui.rounded(on ? Ui.PANEL : Ui.ACCENT, 18), null));
        };
        paint.run();
        if (Core.get().isSignedIn()) {
            Core.get().call((e, err) -> {
                JSONObject ok = e == null ? null : e.optJSONObject("ok");
                if (ok != null && !ok.isNull("subscribed")) {
                    state[0] = ok.optBoolean("subscribed");
                    paint.run();
                }
            }, "subscription", "id", channelId);
        }
        b.setOnClickListener(v -> {
            if (!Core.get().isSignedIn()) {
                android.widget.Toast.makeText(c, "Sign in to subscribe.", android.widget.Toast.LENGTH_SHORT).show();
                return;
            }
            final Boolean before = state[0];
            final boolean wanted = !Boolean.TRUE.equals(before);
            state[0] = wanted;
            paint.run();
            Core.get().call((e, err) -> {
                if (err != null) {
                    state[0] = before;
                    paint.run();
                    android.widget.Toast.makeText(c, "YouTube did not take that: " + err, android.widget.Toast.LENGTH_LONG).show();
                }
            }, "subscribe", "id", channelId, "on", wanted);
        });
        return b;
    }

    static ListScreen playlist(final Context c, Nav nav, final String id, String name) {
        return new ListScreen(c, nav, name == null ? "Playlist" : name, true, "This playlist is empty.", (r, header, t) ->
                Core.get().call((e, err) -> {
                    JSONObject page = e == null ? null : e.optJSONObject("ok");
                    if (err != null || page == null) {
                        r.page(null, err == null ? "No answer." : err);
                        return;
                    }
                    JSONObject pl = page.optJSONObject("playlist");
                    if (pl != null && header.getChildCount() == 0) {
                        t.setText(pl.optString("name"));
                        String sub = Ui.join(Core.str(pl, "channel_name"),
                                pl.isNull("video_count") ? null : pl.optLong("video_count") + " videos");
                        if (!sub.isEmpty()) {
                            TextView s = Ui.text(c, 12, Ui.MUTE, false);
                            s.setText(sub);
                            s.setPadding(Ui.dp(16), Ui.dp(10), Ui.dp(16), Ui.dp(6));
                            header.addView(s);
                        }
                        // Download every video of the playlist.
                        final String playlistName = pl.optString("name");
                        LinearLayout all = Ui.row(c);
                        all.setGravity(android.view.Gravity.CENTER_VERTICAL);
                        all.setPadding(Ui.dp(12), Ui.dp(8), Ui.dp(14), Ui.dp(8));
                        all.setBackground(new android.graphics.drawable.RippleDrawable(
                                android.content.res.ColorStateList.valueOf(Ui.RIPPLE), Ui.rounded(Ui.PANEL, 18), null));
                        all.addView(Ui.icon(c, R.drawable.ic_download, Ui.TEXT), Ui.lin(Ui.dp(20), Ui.dp(20)));
                        TextView label = Ui.text(c, 13, Ui.TEXT, true);
                        label.setText("Download all");
                        label.setPadding(Ui.dp(8), 0, 0, 0);
                        all.addView(label, Ui.lin(Ui.WRAP, Ui.WRAP));
                        all.setContentDescription("Download all videos of this playlist");
                        all.setOnClickListener(v -> Downloads.get().choosePlaylist(c, id, playlistName));
                        LinearLayout.LayoutParams al = Ui.lin(Ui.WRAP, Ui.WRAP);
                        al.leftMargin = Ui.dp(16);
                        al.topMargin = Ui.dp(6);
                        al.bottomMargin = Ui.dp(10);
                        header.addView(all, al);
                    }
                    r.page(Core.Page.parse(page.optJSONObject("videos")), null);
                }, "playlist", "id", id));
    }
}

/** Search: a text field, suggestions while typing, then results. */
final class SearchScreen extends Screen {
    private final Handler main = new Handler(Looper.getMainLooper());
    private final EditText field;
    private final FeedList results;
    private final ListView suggestions;
    private final ArrayAdapter<String> suggestionAdapter;
    private final List<String> suggestionItems = new ArrayList<>();
    private String query = "";
    private int suggestGeneration;
    private Runnable pendingSuggest;
    /** The list under the field shows past searches (true) or suggestions. */
    private boolean showingHistory;
    private final android.content.SharedPreferences prefs;
    /** Filters: sort, upload date, type, duration (index 0 of each is "any"). */
    private final int[] filter = new int[4];
    private static final String[] FILTER_NAMES = {"Sort by", "Upload date", "Type", "Duration"};
    private static final String[][] FILTER_LABELS = {
        {"Relevance", "Upload date", "View count", "Rating"},
        {"Any time", "Last hour", "Today", "This week", "This month", "This year"},
        {"All", "Videos", "Channels", "Playlists"},
        {"Any length", "Under 4 minutes", "4 - 20 minutes", "Over 20 minutes"},
    };
    private static final String[][] FILTER_VALUES = {
        {null, "date", "views", "rating"},
        {null, "hour", "day", "week", "month", "year"},
        {null, "video", "channel", "playlist"},
        {null, "short", "medium", "long"},
    };
    private static final String[] FILTER_KEYS = {"sort", "date", "kind", "length"};
    private LinearLayout filterRow;

    /** Past searches, newest first. */
    private List<String> history() {
        List<String> out = new ArrayList<>();
        try {
            JSONArray a = new JSONArray(prefs.getString("searchHistory", "[]"));
            for (int i = 0; i < a.length(); i++) {
                out.add(a.optString(i));
            }
        } catch (Exception ignored) {
        }
        return out;
    }

    private void saveHistory(List<String> list) {
        JSONArray a = new JSONArray();
        for (int i = 0; i < Math.min(list.size(), 40); i++) {
            a.put(list.get(i));
        }
        prefs.edit().putString("searchHistory", a.toString()).apply();
    }

    private void remember(String q) {
        if (prefs.getBoolean("searchHistoryOff", false)) {
            return;
        }
        List<String> list = history();
        list.remove(q);
        list.add(0, q);
        saveHistory(list);
    }

    /** Show past searches while the field is empty. */
    private void showHistory() {
        List<String> past = history();
        showingHistory = true;
        suggestionItems.clear();
        suggestionItems.addAll(past);
        suggestionAdapter.notifyDataSetChanged();
        suggestions.setVisibility(past.isEmpty() ? View.GONE : View.VISIBLE);
    }

    private void paintFilters() {
        for (int i = 0; i < filterRow.getChildCount(); i++) {
            TextView t = (TextView) filterRow.getChildAt(i);
            boolean set = filter[i] != 0;
            t.setText((set ? FILTER_LABELS[i][filter[i]] : FILTER_NAMES[i]) + "  ▾");
            t.setTextColor(set ? Ui.ON_ACCENT : Ui.TEXT);
            t.setBackground(new android.graphics.drawable.RippleDrawable(android.content.res.ColorStateList.valueOf(Ui.RIPPLE),
                    Ui.rounded(set ? Ui.ACCENT : Ui.PANEL, 8), null));
        }
    }

    private JSONObject filterJson() {
        JSONObject f = new JSONObject();
        try {
            for (int i = 0; i < 4; i++) {
                String v = FILTER_VALUES[i][filter[i]];
                if (v != null) {
                    f.put(FILTER_KEYS[i], v);
                }
            }
        } catch (Exception ignored) {
        }
        return f;
    }

    SearchScreen(final Context c, Nav nav) {
        prefs = c.getSharedPreferences("shell", Context.MODE_PRIVATE);
        field = new EditText(c);
        field.setHint("Search YouTube");
        field.setHintTextColor(Ui.MUTE);
        field.setTextColor(Ui.TEXT);
        field.setTextSize(15);
        field.setSingleLine(true);
        field.setImeOptions(EditorInfo.IME_ACTION_SEARCH | EditorInfo.IME_FLAG_NO_EXTRACT_UI);
        field.setBackground(Ui.rounded(Ui.PANEL, 20));
        field.setPadding(Ui.dp(16), 0, Ui.dp(16), 0);
        LinearLayout.LayoutParams fl = new LinearLayout.LayoutParams(0, Ui.dp(40), 1f);
        fl.rightMargin = Ui.dp(4);
        field.setLayoutParams(fl);

        results = new FeedList(c, false, nav::open);
        results.setPreview(true);
        results.setMenu(nav, false);
        results.setSource(r -> Core.get().call((e, err) -> FeedList.deliver(e, err, r), "search", "query", query, "filter", filterJson()));
        results.showMessage("Search for videos, channels and playlists.", false);

        suggestionAdapter = new ArrayAdapter<String>(c, 0, suggestionItems) {
            @Override
            public View getView(int position, View view, ViewGroup parent) {
                LinearLayout row = (LinearLayout) view;
                if (row == null) {
                    row = Ui.row(c);
                    row.setPadding(Ui.dp(8), 0, Ui.dp(16), 0);
                    row.setMinimumHeight(Ui.dp(48));
                    row.addView(Ui.icon(c, R.drawable.ic_search, Ui.MUTE), Ui.lin(Ui.dp(40), Ui.dp(40)));
                    TextView t = Ui.lines(Ui.text(c, 15, Ui.TEXT, false), 1);
                    row.addView(t, Ui.weight(Ui.WRAP));
                }
                ((ImageView) row.getChildAt(0)).setImageResource(showingHistory ? R.drawable.ic_history : R.drawable.ic_search);
                ((TextView) row.getChildAt(1)).setText(getItem(position));
                return row;
            }
        };
        suggestions = new ListView(c);
        suggestions.setDivider(null);
        suggestions.setBackgroundColor(Ui.BG);
        suggestions.setSelector(Ui.ripple());
        suggestions.setAdapter(suggestionAdapter);
        suggestions.setVisibility(View.GONE);
        suggestions.setOnItemClickListener((parent, v, position, id) -> run(suggestionItems.get(position)));
        // A long press on a past search removes it.
        suggestions.setOnItemLongClickListener((parent, v, position, id) -> {
            if (!showingHistory) {
                return false;
            }
            final String q = suggestionItems.get(position);
            Sheet s = new Sheet(c, q);
            s.add(R.drawable.ic_close, "Remove from search history", () -> {
                List<String> list = history();
                list.remove(q);
                saveHistory(list);
                showHistory();
            });
            s.add(R.drawable.ic_history, "Clear all search history", () -> {
                saveHistory(new ArrayList<>());
                showHistory();
            });
            s.show();
            return true;
        });

        // Filters above the results.
        filterRow = Ui.row(c);
        filterRow.setPadding(Ui.dp(12), Ui.dp(6), Ui.dp(12), Ui.dp(6));
        for (int i = 0; i < 4; i++) {
            final int which = i;
            TextView chip = Ui.text(c, 13, Ui.TEXT, true);
            chip.setPadding(Ui.dp(12), Ui.dp(7), Ui.dp(12), Ui.dp(7));
            chip.setOnClickListener(v -> Sheet.choose(c, FILTER_NAMES[which], FILTER_LABELS[which], filter[which], pick -> {
                filter[which] = pick;
                paintFilters();
                if (!query.isEmpty()) {
                    results.load();
                }
            }));
            LinearLayout.LayoutParams lp = Ui.lin(Ui.WRAP, Ui.WRAP);
            lp.rightMargin = Ui.dp(8);
            filterRow.addView(chip, lp);
        }
        paintFilters();
        android.widget.HorizontalScrollView filterScroll = new android.widget.HorizontalScrollView(c);
        filterScroll.setHorizontalScrollBarEnabled(false);
        filterScroll.addView(filterRow);
        LinearLayout resultsColumn = Ui.column(c);
        resultsColumn.addView(filterScroll, Ui.lin(Ui.MATCH, Ui.WRAP));
        resultsColumn.addView(results, new LinearLayout.LayoutParams(Ui.MATCH, 0, 1f));

        FrameLayout body = new FrameLayout(c);
        body.addView(resultsColumn, new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));
        body.addView(suggestions, new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));

        View clear = Ui.iconButton(c, R.drawable.ic_close, "Clear", v -> {
            field.setText("");
            focus();
        });
        LinearLayout bar = Ui.row(c);
        bar.setPadding(Ui.dp(4), 0, Ui.dp(4), 0);
        bar.addView(Ui.iconButton(c, R.drawable.ic_back, "Back", v -> {
            hideKeyboard();
            nav.pop();
        }));
        bar.addView(field);
        bar.addView(clear);
        bar.setLayoutParams(Ui.lin(Ui.MATCH, Ui.dp(56)));
        view = page(c, bar, body);

        field.setOnEditorActionListener((v, action, event) -> {
            run(field.getText().toString());
            return true;
        });
        field.addTextChangedListener(new TextWatcher() {
            @Override
            public void beforeTextChanged(CharSequence s, int a, int b, int d) {}

            @Override
            public void onTextChanged(CharSequence s, int a, int b, int d) {}

            @Override
            public void afterTextChanged(Editable s) {
                suggest(s.toString().trim());
            }
        });
    }

    @Override
    void onShown() {
        if (query.isEmpty()) {
            focus();
            showHistory();
        }
    }

    private void focus() {
        field.requestFocus();
        main.postDelayed(() -> {
            InputMethodManager imm = (InputMethodManager) field.getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
            if (imm != null) {
                imm.showSoftInput(field, InputMethodManager.SHOW_IMPLICIT);
            }
        }, 120);
    }

    private void hideKeyboard() {
        InputMethodManager imm = (InputMethodManager) field.getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
        if (imm != null) {
            imm.hideSoftInputFromWindow(field.getWindowToken(), 0);
        }
        field.clearFocus();
    }

    /** Ask for suggestions after a short pause in typing. */
    private void suggest(final String text) {
        if (pendingSuggest != null) {
            main.removeCallbacks(pendingSuggest);
        }
        final int mine = ++suggestGeneration;
        if (text.isEmpty()) {
            showHistory();
            return;
        }
        if (text.equals(query)) {
            suggestions.setVisibility(View.GONE);
            return;
        }
        pendingSuggest = () -> Core.get().call((e, err) -> {
            if (mine != suggestGeneration || e == null) {
                return;
            }
            JSONArray a = e.optJSONArray("items");
            showingHistory = false;
            suggestionItems.clear();
            // Past searches that match come first, as on YouTube.
            for (String past : history()) {
                if (past.toLowerCase(java.util.Locale.ROOT).startsWith(text.toLowerCase(java.util.Locale.ROOT)) && suggestionItems.size() < 3) {
                    suggestionItems.add(past);
                }
            }
            for (int i = 0; a != null && i < a.length(); i++) {
                suggestionItems.add(a.optString(i));
            }
            suggestionAdapter.notifyDataSetChanged();
            suggestions.setVisibility(suggestionItems.isEmpty() ? View.GONE : View.VISIBLE);
        }, "suggest", "query", text);
        main.postDelayed(pendingSuggest, 220);
    }

    private void run(String text) {
        String q = text.trim();
        if (q.isEmpty()) {
            return;
        }
        query = q;
        remember(q);
        suggestGeneration++;
        if (!q.equals(field.getText().toString())) {
            field.setText(q);
            field.setSelection(q.length());
        }
        suggestions.setVisibility(View.GONE);
        hideKeyboard();
        results.setEmptyText("No results for \"" + q + "\".");
        results.load();
    }
}

/** Library: sign-in prompt, or the signed-in lists. */
final class LibraryScreen extends Screen {
    private final LinearLayout prompt;
    private final LinearLayout rows;
    private final TextView signInButton;

    LibraryScreen(final Context c, final Nav nav) {
        prompt = Ui.column(c);
        prompt.setGravity(Gravity.CENTER);
        prompt.setPadding(Ui.dp(36), 0, Ui.dp(36), 0);
        ImageView person = Ui.icon(c, R.drawable.ic_person, Ui.ACCENT);
        person.setBackground(Ui.rounded(Ui.PANEL, 34));
        prompt.addView(person, Ui.lin(Ui.dp(68), Ui.dp(68)));
        TextView head = Ui.text(c, 19, Ui.TEXT, true);
        head.setText("Sign in to YouTube");
        head.setPadding(0, Ui.dp(20), 0, 0);
        prompt.addView(head, Ui.lin(Ui.WRAP, Ui.WRAP));
        TextView about = Ui.text(c, 14, Ui.MUTE, false);
        about.setText("See your own home feed, subscriptions, history and playlists. You sign in on Google's page; the session stays on this device.");
        about.setGravity(Gravity.CENTER);
        about.setLineSpacing(Ui.dp(4), 1f);
        about.setPadding(0, Ui.dp(10), 0, Ui.dp(24));
        prompt.addView(about, Ui.lin(Ui.WRAP, Ui.WRAP));
        signInButton = Ui.button(c, "Sign in with Google", v -> nav.signIn());
        prompt.addView(signInButton, Ui.lin(Ui.WRAP, Ui.WRAP));

        rows = Ui.column(c);
        rows.setPadding(0, Ui.dp(6), 0, 0);
        rows.addView(row(c, R.drawable.ic_bell, "Notifications", null, v -> nav.push(ListScreen.notifications(c, nav))));
        rows.addView(row(c, R.drawable.ic_history, "History", null, v -> nav.push(ListScreen.history(c, nav))));
        rows.addView(row(c, R.drawable.ic_watch_later, "Watch later", null, v -> nav.push(ListScreen.playlist(c, nav, "WL", "Watch later"))));
        rows.addView(row(c, R.drawable.ic_like, "Liked videos", null, v -> nav.push(ListScreen.playlist(c, nav, "LL", "Liked videos"))));
        rows.addView(row(c, R.drawable.ic_playlist, "Playlists", null, v -> nav.push(ListScreen.account(c, nav, "Playlists", "saved_playlists"))));
        rows.addView(row(c, R.drawable.ic_save, "New playlist", null, v -> ItemMenu.createPlaylist(c, null)));
        rows.addView(row(c, R.drawable.ic_person, "Channels", "Channels you are subscribed to",
                v -> nav.push(ListScreen.account(c, nav, "Channels", "subscriptions"))));

        FrameLayout body = new FrameLayout(c);
        body.addView(rows, new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));
        body.addView(prompt, new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));
        view = page(c, bar(c, nav, false, barTitle(c, "Library")), body);
        onCoreChanged();
    }

    static View row(Context c, int icon, String label, String hint, View.OnClickListener onClick) {
        LinearLayout row = Ui.row(c);
        row.setMinimumHeight(Ui.dp(56));
        row.setPadding(Ui.dp(8), Ui.dp(6), Ui.dp(8), Ui.dp(6));
        row.setForeground(Ui.ripple());
        row.addView(Ui.icon(c, icon, Ui.MUTE), Ui.lin(Ui.dp(48), Ui.dp(40)));
        LinearLayout col = Ui.column(c);
        col.setPadding(Ui.dp(6), 0, 0, 0);
        TextView t = Ui.text(c, 15, Ui.TEXT, false);
        t.setText(label);
        col.addView(t);
        if (hint != null) {
            TextView h = Ui.text(c, 12, Ui.MUTE, false);
            h.setText(hint);
            h.setPadding(0, Ui.dp(4), 0, 0);
            col.addView(h);
        }
        row.addView(col, Ui.weight(Ui.WRAP));
        row.addView(Ui.icon(c, R.drawable.ic_chevron, Ui.MUTE), Ui.lin(Ui.dp(40), Ui.dp(40)));
        row.setOnClickListener(onClick);
        return row;
    }

    @Override
    void onCoreChanged() {
        Boolean in = Core.get().signedIn;
        boolean signedIn = Boolean.TRUE.equals(in);
        prompt.setVisibility(signedIn ? View.GONE : View.VISIBLE);
        rows.setVisibility(signedIn ? View.VISIBLE : View.GONE);
        // Until the saved session has been checked, signing in again would be premature.
        signInButton.setEnabled(in != null);
        signInButton.setAlpha(in != null ? 1f : 0.5f);
    }
}

/** Settings: playback switches, account and build information. */
final class SettingsScreen extends Screen {
    private final Nav nav;
    private final Switch autoplay, background;
    private final TextView account, rules;

    SettingsScreen(Context c, final Nav nav) {
        this.nav = nav;
        LinearLayout col = Ui.column(c);
        col.setPadding(0, Ui.dp(4), 0, Ui.dp(24));

        col.addView(section(c, "Playback"));
        autoplay = toggle(c, col, "Play next video automatically", "When a video ends, start the first one under Up next");
        autoplay.setOnCheckedChangeListener((b, on) -> {
            if (b.isPressed()) {
                Core.get().setAutoplay(on);
            }
        });
        background = toggle(c, col, "Keep playing in the background", "Sound continues with the screen off or in another app");
        background.setChecked(nav.backgroundPlay());
        background.setOnCheckedChangeListener((b, on) -> nav.setBackgroundPlay(on));
        Switch directPlayer = toggle(c, col, "Direct player",
                "Native player: plays the video streams themselves with Android's own decoders, no web view. Off: YouTube's own player page (captions, watch history)");
        directPlayer.setChecked(nav.directPlayer());
        directPlayer.setOnCheckedChangeListener((b, on) -> nav.setDirectPlayer(on));
        final android.content.SharedPreferences prefs = c.getSharedPreferences("shell", Context.MODE_PRIVATE);
        Switch preload = toggle(c, col, "Preload videos",
                "Gets the videos on screen ready while you browse, so the one you tap starts at once (direct player)");
        preload.setChecked(prefs.getBoolean("prefetch", true));
        preload.setOnCheckedChangeListener((b, on) -> {
            prefs.edit().putBoolean("prefetch", on).apply();
            Core.get().prefetch = on && nav.directPlayer();
        });
        final String[] feedModes = {"always", "wifi", "off"};
        final String[] feedLabels = {"Always on", "Wi-Fi only", "Off"};
        final TextView feedRow = Ui.text(c, 15, Ui.TEXT, false);
        final Runnable paintFeed = () -> {
            String m = FeedPreview.mode(c);
            int i = java.util.Arrays.asList(feedModes).indexOf(m);
            feedRow.setText("Playback in feeds: " + feedLabels[Math.max(0, i)]);
        };
        paintFeed.run();
        feedRow.setPadding(Ui.dp(16), Ui.dp(14), Ui.dp(16), Ui.dp(4));
        feedRow.setBackground(Ui.ripple());
        feedRow.setOnClickListener(v -> Sheet.choose(c, "Playback in feeds", feedLabels,
                Math.max(0, java.util.Arrays.asList(feedModes).indexOf(FeedPreview.mode(c))), pick -> {
                    prefs.edit().putString("feedPreview", feedModes[pick]).apply();
                    if ("off".equals(feedModes[pick])) {
                        FeedPreview.get(c).stop();
                    }
                    paintFeed.run();
                }));
        col.addView(feedRow, Ui.lin(Ui.MATCH, Ui.WRAP));
        TextView feedHint = Ui.text(c, 12, Ui.MUTE, false);
        feedHint.setText("The video most in view, or the one you touch, plays in Home, Subscriptions, Search and channels");
        feedHint.setPadding(Ui.dp(16), 0, Ui.dp(16), Ui.dp(10));
        col.addView(feedHint);
        Switch feedSound = toggle(c, col, "Sound in feed playback", "Off: feed videos play muted (tap the speaker on a card to change)");
        feedSound.setChecked(!prefs.getBoolean("feedPreviewMuted", false));
        feedSound.setOnCheckedChangeListener((b, on) -> prefs.edit().putBoolean("feedPreviewMuted", !on).apply());
        Switch swipes = toggle(c, col, "Swipe controls in fullscreen",
                "Swipe up and down on the left side for brightness and on the right side for volume");
        swipes.setChecked(prefs.getBoolean("swipeControls", false));
        swipes.setOnCheckedChangeListener((b, on) -> {
            prefs.edit().putBoolean("swipeControls", on).apply();
            PlayerControls.swipeControls = on;
        });
        Switch stable = toggle(c, col, "Stable volume", "Evens out loud and quiet parts (direct player)");
        stable.setChecked(prefs.getBoolean("stable", false));
        stable.setOnCheckedChangeListener((b, on) -> prefs.edit().putBoolean("stable", on).apply());
        Switch ambient = toggle(c, col, "Ambient mode", "A soft glow in the video's colours behind the details");
        ambient.setChecked(prefs.getBoolean("ambient", true));
        ambient.setOnCheckedChangeListener((b, on) -> prefs.edit().putBoolean("ambient", on).apply());

        col.addView(section(c, "History"));
        Switch searchHistory = toggle(c, col, "Save search history", "Past searches are shown when you start a new one");
        searchHistory.setChecked(!prefs.getBoolean("searchHistoryOff", false));
        searchHistory.setOnCheckedChangeListener((b, on) -> {
            prefs.edit().putBoolean("searchHistoryOff", !on).apply();
            if (!on) {
                prefs.edit().remove("searchHistory").apply();
            }
        });

        col.addView(section(c, "Downloads"));
        col.addView(LibraryScreen.row(c, R.drawable.ic_download, "Downloads", "Progress and finished files (saved to " + Downloads.FOLDER + ")",
                v -> nav.push(new Downloads.Page(c, nav))));

        col.addView(section(c, "Appearance"));
        String theme = nav.theme();
        final String label = "light".equals(theme) ? "Light" : "dark".equals(theme) ? "Dark" : "Same as the phone";
        // One tap moves to the next choice: same as the phone, light, dark.
        final String following = "system".equals(theme) ? "light" : "light".equals(theme) ? "dark" : "system";
        col.addView(LibraryScreen.row(c, R.drawable.ic_theme, "Theme", label, v -> nav.setTheme(following)));

        col.addView(section(c, "Account"));
        account = Ui.text(c, 15, Ui.ACCENT, false);
        account.setPadding(Ui.dp(16), Ui.dp(16), Ui.dp(16), Ui.dp(16));
        account.setForeground(Ui.ripple());
        account.setOnClickListener(v -> {
            if (Core.get().isSignedIn()) {
                nav.signOut();
            } else {
                nav.signIn();
            }
        });
        col.addView(account, Ui.lin(Ui.MATCH, Ui.WRAP));

        col.addView(section(c, "About"));
        TextView version = Ui.text(c, 14, Ui.MUTE, false);
        version.setText("StreamPipe " + Core.get().version);
        version.setPadding(Ui.dp(16), Ui.dp(10), Ui.dp(16), Ui.dp(4));
        col.addView(version);
        rules = Ui.text(c, 14, Ui.MUTE, false);
        rules.setLineSpacing(Ui.dp(3), 1f);
        rules.setPadding(Ui.dp(16), Ui.dp(6), Ui.dp(16), Ui.dp(10));
        col.addView(rules);

        ScrollView scroll = new ScrollView(c);
        scroll.setVerticalScrollBarEnabled(false);
        scroll.addView(col);
        view = page(c, bar(c, nav, !nav.musicMode(), barTitle(c, "Settings")), scroll);
        onCoreChanged();
    }

    private static TextView section(Context c, String name) {
        TextView t = Ui.text(c, 12, Ui.MUTE, true);
        t.setText(name.toUpperCase(java.util.Locale.ROOT));
        t.setLetterSpacing(0.06f);
        t.setPadding(Ui.dp(16), Ui.dp(22), Ui.dp(16), Ui.dp(6));
        return t;
    }

    private static Switch toggle(Context c, LinearLayout into, String label, String hint) {
        LinearLayout row = Ui.row(c);
        row.setPadding(Ui.dp(16), Ui.dp(12), Ui.dp(12), Ui.dp(12));
        row.setForeground(Ui.ripple());
        LinearLayout col = Ui.column(c);
        TextView t = Ui.text(c, 15, Ui.TEXT, false);
        t.setText(label);
        col.addView(t);
        TextView h = Ui.text(c, 12, Ui.MUTE, false);
        h.setText(hint);
        h.setLineSpacing(Ui.dp(2), 1f);
        h.setPadding(0, Ui.dp(5), Ui.dp(8), 0);
        col.addView(h);
        row.addView(col, Ui.weight(Ui.WRAP));
        final Switch s = new Switch(c);
        s.setContentDescription(label);
        row.addView(s);
        row.setOnClickListener(v -> {
            s.setPressed(true);
            s.toggle();
            s.setPressed(false);
        });
        into.addView(row, Ui.lin(Ui.MATCH, Ui.WRAP));
        return s;
    }

    @Override
    void onCoreChanged() {
        autoplay.setChecked(Core.get().autoplay());
        boolean in = Core.get().isSignedIn();
        account.setText(in ? "Sign out" : "Sign in with Google");
        account.setTextColor(in ? Ui.DANGER : Ui.ACCENT);
        rules.setText("Ad rules: " + nav.adRulesText());
    }

    @Override
    void onShown() {
        onCoreChanged();
    }
}
