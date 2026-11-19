package dev.streampipe.app;

import android.content.Context;
import android.view.Gravity;
import android.view.View;
import android.widget.FrameLayout;
import android.widget.HorizontalScrollView;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.ScrollView;
import android.widget.TextView;

import org.json.JSONObject;

/**
 * The "You" tab, laid out as in YouTube's app: the account (picture, name, handle, View
 * channel), the History shelf, then the library (Watch later, Liked videos, playlists,
 * downloads, channels). Settings and notifications sit in the top bar.
 */
final class YouScreen extends Screen {
    /** `{name, handle, avatar, channel_id}` of the signed-in account, set by the activity. */
    static JSONObject account;

    private final Context c;
    private final Nav nav;
    private final LinearLayout signedOut;
    private final ScrollView signedIn;
    private final ImageView avatar;
    private final TextView name, handle;
    private final LinearLayout history;
    private final View historyBlock;
    private Boolean loadedFor;

    YouScreen(final Context c, final Nav nav) {
        this.c = c;
        this.nav = nav;

        TextView space = new TextView(c);
        View bar = bar(c, nav, false, space,
                Ui.iconButton(c, R.drawable.ic_bell_outline, "Notifications", v -> nav.push(ListScreen.notifications(c, nav))),
                Ui.iconButton(c, R.drawable.ic_search_outline, "Search", v -> nav.push(new SearchScreen(c, nav))),
                Ui.iconButton(c, R.drawable.ic_settings, "Settings", v -> nav.push(new SettingsScreen(c, nav))));

        // Signed out: YouTube's prompt.
        signedOut = Ui.column(c);
        signedOut.setGravity(Gravity.CENTER);
        signedOut.setPadding(Ui.dp(36), 0, Ui.dp(36), Ui.dp(40));
        ImageView person = Ui.icon(c, R.drawable.ic_person, Ui.MUTE);
        person.setScaleX(2.6f);
        person.setScaleY(2.6f);
        signedOut.addView(person, Ui.lin(Ui.dp(96), Ui.dp(96)));
        TextView head = Ui.text(c, 20, Ui.TEXT, true);
        head.setText("Enjoy your favorite videos");
        head.setPadding(0, Ui.dp(16), 0, 0);
        signedOut.addView(head, Ui.lin(Ui.WRAP, Ui.WRAP));
        TextView about = Ui.text(c, 14, Ui.MUTE, false);
        about.setText("Sign in to access videos that you've liked or saved");
        about.setGravity(Gravity.CENTER);
        about.setPadding(0, Ui.dp(10), 0, Ui.dp(22));
        signedOut.addView(about, Ui.lin(Ui.WRAP, Ui.WRAP));
        TextView signIn = Ui.text(c, 14, Ui.LINK, true);
        signIn.setText("Sign in");
        signIn.setPadding(Ui.dp(18), Ui.dp(9), Ui.dp(18), Ui.dp(9));
        signIn.setBackground(Ui.rounded(0x00000000, 18));
        android.graphics.drawable.GradientDrawable outline = new android.graphics.drawable.GradientDrawable();
        outline.setCornerRadius(Ui.dp(18));
        outline.setStroke(Math.max(1, Ui.dp(1)), Ui.LINE);
        signIn.setBackground(outline);
        signIn.setOnClickListener(v -> nav.signIn());
        signedOut.addView(signIn, Ui.lin(Ui.WRAP, Ui.WRAP));

        // Signed in.
        LinearLayout col = Ui.column(c);
        col.setPadding(0, 0, 0, Ui.dp(24));
        LinearLayout who = Ui.row(c);
        who.setPadding(Ui.dp(16), Ui.dp(8), Ui.dp(16), Ui.dp(16));
        avatar = new ImageView(c);
        avatar.setScaleType(ImageView.ScaleType.CENTER_CROP);
        avatar.setBackgroundColor(Ui.PANEL);
        Ui.clipCircle(avatar);
        who.addView(avatar, Ui.lin(Ui.dp(72), Ui.dp(72)));
        LinearLayout names = Ui.column(c);
        names.setPadding(Ui.dp(16), 0, 0, 0);
        name = Ui.lines(Ui.text(c, 24, Ui.TEXT, true), 1);
        names.addView(name);
        handle = Ui.lines(Ui.text(c, 12, Ui.MUTE, false), 1);
        handle.setPadding(0, Ui.dp(6), 0, 0);
        names.addView(handle);
        who.addView(names, Ui.weight(Ui.WRAP));
        col.addView(who);
        LinearLayout buttons = Ui.row(c);
        buttons.setPadding(Ui.dp(16), 0, Ui.dp(16), Ui.dp(8));
        buttons.addView(pill(c, "View channel", v -> {
            if (account != null && !account.optString("channel_id").isEmpty()) {
                nav.push(new ChannelScreen(c, nav, account.optString("channel_id"), account.optString("name")));
            }
        }), Ui.weight(Ui.WRAP));
        View gap = new View(c);
        buttons.addView(gap, Ui.lin(Ui.dp(10), 1));
        buttons.addView(pill(c, "Switch to YouTube Music", v -> nav.setMusicMode(true)), Ui.weight(Ui.WRAP));
        col.addView(buttons);

        LinearLayout hb = Ui.column(c);
        hb.addView(heading(c, "History", v -> nav.push(ListScreen.history(c, nav))));
        history = Ui.row(c);
        history.setGravity(Gravity.TOP);
        history.setPadding(Ui.dp(16), 0, Ui.dp(4), 0);
        HorizontalScrollView hs = new HorizontalScrollView(c);
        hs.setHorizontalScrollBarEnabled(false);
        hs.addView(history);
        hb.addView(hs);
        historyBlock = hb;
        col.addView(hb);

        col.addView(heading(c, "Playlists", v -> nav.push(ListScreen.account(c, nav, "Playlists", "saved_playlists"))));
        col.addView(row(c, R.drawable.ic_watch_later, "Watch later", v -> nav.push(ListScreen.playlist(c, nav, "WL", "Watch later"))));
        col.addView(row(c, R.drawable.ic_like_outline, "Liked videos", v -> nav.push(ListScreen.playlist(c, nav, "LL", "Liked videos"))));
        col.addView(row(c, R.drawable.ic_playlist, "Your playlists", v -> nav.push(ListScreen.account(c, nav, "Playlists", "saved_playlists"))));
        col.addView(row(c, R.drawable.ic_add, "New playlist", v -> ItemMenu.createPlaylist(c, null)));
        View line = Ui.divider(c);
        LinearLayout.LayoutParams ll = Ui.lin(Ui.MATCH, Math.max(1, Ui.dp(0.75f)));
        ll.topMargin = Ui.dp(8);
        ll.bottomMargin = Ui.dp(8);
        col.addView(line, ll);
        col.addView(row(c, R.drawable.ic_play, "Your videos", v -> {
            if (account != null && !account.optString("channel_id").isEmpty()) {
                nav.push(new ChannelScreen(c, nav, account.optString("channel_id"), account.optString("name")));
            }
        }));
        col.addView(row(c, R.drawable.ic_download_outline, "Downloads", v -> nav.push(new Downloads.Page(c, nav))));
        col.addView(row(c, R.drawable.ic_subs_outline, "Your channels", v -> nav.push(ListScreen.account(c, nav, "Channels", "subscriptions"))));
        col.addView(row(c, R.drawable.ic_history, "Watch history", v -> nav.push(ListScreen.history(c, nav))));
        View line2 = Ui.divider(c);
        col.addView(line2, ll);
        col.addView(row(c, R.drawable.ic_settings, "Settings", v -> nav.push(new SettingsScreen(c, nav))));
        signedIn = new ScrollView(c);
        signedIn.setVerticalScrollBarEnabled(false);
        signedIn.addView(col);

        FrameLayout body = new FrameLayout(c);
        body.addView(signedIn, new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));
        body.addView(signedOut, new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));
        LinearLayout page = Ui.column(c);
        page.setBackgroundColor(Ui.BG);
        page.addView(bar);
        page.addView(body, new LinearLayout.LayoutParams(Ui.MATCH, 0, 1f));
        view = page;
        onCoreChanged();
    }

    private static TextView pill(Context c, String label, View.OnClickListener onClick) {
        TextView t = Ui.lines(Ui.text(c, 13, Ui.TEXT, true), 1);
        t.setText(label);
        t.setGravity(Gravity.CENTER);
        t.setPadding(Ui.dp(12), Ui.dp(9), Ui.dp(12), Ui.dp(9));
        t.setBackground(new android.graphics.drawable.RippleDrawable(android.content.res.ColorStateList.valueOf(Ui.RIPPLE),
                Ui.rounded(Ui.PANEL, 18), null));
        t.setOnClickListener(onClick);
        return t;
    }

    /** "History  >" — a section title that opens the full list. */
    private static View heading(Context c, String label, View.OnClickListener onClick) {
        LinearLayout r = Ui.row(c);
        r.setPadding(Ui.dp(16), Ui.dp(18), Ui.dp(16), Ui.dp(12));
        TextView t = Ui.text(c, 20, Ui.TEXT, true);
        t.setText(label);
        r.addView(t);
        ImageView chev = Ui.icon(c, R.drawable.ic_chevron, Ui.TEXT);
        r.addView(chev, Ui.lin(Ui.dp(28), Ui.dp(24)));
        r.setOnClickListener(onClick);
        return r;
    }

    private static View row(Context c, int icon, String label, View.OnClickListener onClick) {
        LinearLayout r = Ui.row(c);
        r.setMinimumHeight(Ui.dp(48));
        r.setPadding(Ui.dp(16), 0, Ui.dp(16), 0);
        r.setForeground(Ui.ripple());
        r.addView(Ui.icon(c, icon, Ui.TEXT), Ui.lin(Ui.dp(24), Ui.dp(24)));
        TextView t = Ui.text(c, 15, Ui.TEXT, false);
        t.setText(label);
        t.setPadding(Ui.dp(22), 0, 0, 0);
        r.addView(t, Ui.weight(Ui.WRAP));
        r.setOnClickListener(onClick);
        return r;
    }

    private void loadHistory() {
        Core.get().call((e, err) -> {
            if (err != null || e == null) {
                return;
            }
            Core.Page page = Core.Page.parse(e.optJSONObject("ok"));
            history.removeAllViews();
            int n = 0;
            for (final Core.Item it : page.items) {
                if (it.type != Core.Item.VIDEO || n >= 15) {
                    continue;
                }
                n++;
                history.addView(card(it), Ui.lin(Ui.dp(160), Ui.WRAP));
            }
            historyBlock.setVisibility(n == 0 ? View.GONE : View.VISIBLE);
        }, "history");
    }

    private View card(final Core.Item it) {
        LinearLayout col = Ui.column(c);
        col.setPadding(0, 0, Ui.dp(12), 0);
        FeedList.RatioFrame thumb = new FeedList.RatioFrame(c);
        thumb.setBackgroundColor(Ui.PANEL);
        Ui.clipRound(thumb, 10);
        ImageView img = new ImageView(c);
        img.setScaleType(ImageView.ScaleType.CENTER_CROP);
        thumb.addView(img, new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));
        Images.load(img, it.thumb);
        if (it.durationS > 0) {
            TextView d = Ui.text(c, 11, 0xFFFFFFFF, true);
            d.setText(Ui.duration(it.durationS));
            d.setPadding(Ui.dp(4), Ui.dp(1), Ui.dp(4), Ui.dp(1));
            d.setBackground(Ui.rounded(0xCC000000, 4));
            FrameLayout.LayoutParams dl = Ui.frame(Ui.WRAP, Ui.WRAP, Gravity.BOTTOM | Gravity.END);
            dl.setMargins(0, 0, Ui.dp(5), Ui.dp(5));
            thumb.addView(d, dl);
        }
        FeedList.WatchLine.in(thumb).set(Math.max(it.watched, Resume.percent(it.id)));
        col.addView(thumb, Ui.lin(Ui.MATCH, Ui.WRAP));
        TextView t = Ui.lines(Ui.text(c, 13, Ui.TEXT, false), 2);
        t.setText(it.title);
        t.setPadding(0, Ui.dp(8), 0, 0);
        t.setLineSpacing(Ui.dp(2), 1f);
        col.addView(t);
        TextView ch = Ui.lines(Ui.text(c, 12, Ui.MUTE, false), 1);
        ch.setText(it.channelName == null ? "" : it.channelName);
        ch.setPadding(0, Ui.dp(4), 0, 0);
        col.addView(ch);
        col.setOnClickListener(v -> nav.open(it));
        return col;
    }

    @Override
    void onCoreChanged() {
        Boolean in = Core.get().signedIn;
        boolean yes = Boolean.TRUE.equals(in);
        signedOut.setVisibility(yes ? View.GONE : View.VISIBLE);
        signedIn.setVisibility(yes ? View.VISIBLE : View.GONE);
        if (account != null) {
            name.setText(account.optString("name"));
            handle.setText(account.optString("handle"));
            Images.load(avatar, account.optString("avatar"));
        }
        if (in != null && !in.equals(loadedFor)) {
            loadedFor = in;
            if (yes) {
                loadHistory();
            }
        }
    }

    @Override
    void onShown() {
        onCoreChanged();
    }

    @Override
    void onReselected() {
        signedIn.smoothScrollTo(0, 0);
    }
}
