package dev.streampipe.app;

import android.content.Context;
import android.content.res.ColorStateList;
import android.graphics.drawable.RippleDrawable;
import android.view.Gravity;
import android.view.View;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.TextView;
import android.widget.Toast;

import org.json.JSONArray;
import org.json.JSONObject;

/**
 * A channel, as on YouTube: banner, picture, name, handle and counts, Subscribe with its
 * bell, and the tabs Videos, Shorts, Live, Playlists, Posts and About.
 */
final class ChannelScreen extends Screen {
    private static final String[] TABS = {"Videos", "Shorts", "Live", "Playlists", "Posts", "About"};
    private static final String[] TAB_KEYS = {"videos", "shorts", "live", "playlists", "posts", "about"};

    private final Context c;
    private final String id;
    private final FeedList list;
    private final LinearLayout header;
    private final LinearLayout top;
    private final LinearLayout about;
    private final TextView barTitle;
    private int tab;
    /** The first answer (header and Videos tab), kept so going back to Videos needs no request. */
    private Core.Page firstVideos;

    ChannelScreen(Context context, Nav nav, String channelId, String name) {
        c = context;
        id = channelId;
        barTitle = barTitle(c, name == null ? "Channel" : name);
        header = Ui.column(c);
        top = Ui.column(c);
        header.addView(top, Ui.lin(Ui.MATCH, Ui.WRAP));
        header.addView(chips(c, TABS, 0, this::select), Ui.lin(Ui.MATCH, Ui.WRAP));
        about = Ui.column(c);
        about.setPadding(Ui.dp(16), Ui.dp(8), Ui.dp(16), Ui.dp(16));
        about.setVisibility(View.GONE);
        header.addView(about, Ui.lin(Ui.MATCH, Ui.WRAP));
        list = new FeedList(c, false, nav::open);
        list.setMenu(nav, false);
        list.setHeader(header);
        list.setEmptyText("This channel has no videos.");
        list.setSource(this::load);
        view = page(c, bar(c, nav, true, barTitle), list);
        list.load();
    }

    private void select(int index) {
        if (index == tab) {
            return;
        }
        tab = index;
        about.setVisibility(TAB_KEYS[tab].equals("about") ? View.VISIBLE : View.GONE);
        String[] empty = {"This channel has no videos.", "This channel has no Shorts.", "No live streams.",
            "This channel has no playlists.", "This channel has no posts.", ""};
        list.setEmptyText(empty[tab]);
        list.load();
    }

    private void load(final FeedList.Result r) {
        String key = TAB_KEYS[tab];
        switch (key) {
            case "videos":
                if (firstVideos != null) {
                    r.page(firstVideos, null);
                    firstVideos = null;
                    return;
                }
                Core.get().call((e, err) -> {
                    JSONObject page = e == null ? null : e.optJSONObject("ok");
                    if (err != null || page == null) {
                        r.page(null, err == null ? "No answer." : err);
                        return;
                    }
                    if (top.getChildCount() == 0) {
                        bindHeader(page);
                    }
                    r.page(Core.Page.parse(page.optJSONObject("videos")), null);
                }, "channel", "id", id);
                break;
            case "playlists":
                Core.get().call((e, err) -> FeedList.deliver(e, err, r), "channel_playlists", "id", id);
                break;
            case "about":
                r.page(new Core.Page(), null);
                if (about.getChildCount() == 0) {
                    loadAbout();
                }
                break;
            default:
                Core.get().call((e, err) -> FeedList.deliver(e, err, r), "channel_tab", "id", id, "tab", key);
        }
    }

    private void bindHeader(JSONObject page) {
        JSONObject ch = page.optJSONObject("channel");
        if (ch == null) {
            return;
        }
        barTitle.setText(ch.optString("name"));
        String banner = Core.url(Core.str(page, "banner"));
        if (banner != null) {
            FrameLayout frame = new FrameLayout(c) {
                @Override
                protected void onMeasure(int w, int h) {
                    int width = MeasureSpec.getSize(w);
                    super.onMeasure(w, MeasureSpec.makeMeasureSpec(Math.round(width / 5.2f), MeasureSpec.EXACTLY));
                }
            };
            frame.setBackgroundColor(Ui.PANEL);
            Ui.clipRound(frame, 12);
            ImageView img = new ImageView(c);
            img.setScaleType(ImageView.ScaleType.CENTER_CROP);
            frame.addView(img, new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));
            Images.load(img, banner);
            LinearLayout.LayoutParams bl = Ui.lin(Ui.MATCH, Ui.WRAP);
            bl.setMargins(Ui.dp(12), Ui.dp(8), Ui.dp(12), 0);
            top.addView(frame, bl);
        }
        LinearLayout row = Ui.row(c);
        row.setPadding(Ui.dp(16), Ui.dp(14), Ui.dp(16), Ui.dp(10));
        ImageView avatar = new ImageView(c);
        avatar.setScaleType(ImageView.ScaleType.CENTER_CROP);
        avatar.setBackgroundColor(Ui.PANEL);
        Ui.clipCircle(avatar);
        Images.load(avatar, Core.url(Core.str(ch, "avatar")));
        row.addView(avatar, Ui.lin(Ui.dp(64), Ui.dp(64)));
        LinearLayout col = Ui.column(c);
        col.setPadding(Ui.dp(14), 0, 0, 0);
        TextView n = Ui.lines(Ui.text(c, 20, Ui.TEXT, true), 2);
        n.setText(ch.optString("name") + (page.optBoolean("verified") ? "  ✓" : ""));
        col.addView(n);
        String line = Ui.join(Core.str(page, "handle"),
                ch.isNull("subscribers") ? null : Ui.count(ch.optLong("subscribers")) + " subscribers",
                page.isNull("video_count") ? null : Ui.count(page.optLong("video_count")) + " videos");
        if (!line.isEmpty()) {
            TextView s = Ui.lines(Ui.text(c, 12, Ui.MUTE, false), 2);
            s.setText(line);
            s.setPadding(0, Ui.dp(5), 0, 0);
            col.addView(s);
        }
        row.addView(col, Ui.weight(Ui.WRAP));
        top.addView(row, Ui.lin(Ui.MATCH, Ui.WRAP));
        String description = page.optString("description");
        if (!description.isEmpty()) {
            TextView d = Ui.lines(Ui.text(c, 13, Ui.MUTE, false), 2);
            d.setText(description);
            d.setPadding(Ui.dp(16), 0, Ui.dp(16), Ui.dp(10));
            d.setOnClickListener(v -> select(TABS.length - 1));
            top.addView(d, Ui.lin(Ui.MATCH, Ui.WRAP));
        }
        top.addView(subscribeRow(), Ui.lin(Ui.MATCH, Ui.WRAP));
    }

    /** Subscribe / Subscribed, and once subscribed the bell with its three choices. */
    private View subscribeRow() {
        LinearLayout row = Ui.row(c);
        row.setPadding(Ui.dp(16), Ui.dp(2), Ui.dp(16), Ui.dp(6));
        final TextView b = Ui.text(c, 14, Ui.ON_ACCENT, true);
        b.setGravity(Gravity.CENTER);
        b.setPadding(Ui.dp(16), Ui.dp(10), Ui.dp(16), Ui.dp(10));
        row.addView(b, Ui.weight(Ui.WRAP));
        final ImageView bell = Ui.icon(c, R.drawable.ic_bell, Ui.TEXT);
        bell.setBackground(new RippleDrawable(ColorStateList.valueOf(Ui.RIPPLE), Ui.rounded(Ui.PANEL, 20), null));
        bell.setContentDescription("Notifications for this channel");
        LinearLayout.LayoutParams bl = Ui.lin(Ui.dp(44), Ui.dp(40));
        bl.leftMargin = Ui.dp(8);
        row.addView(bell, bl);
        final Boolean[] state = {null};
        final JSONArray[] bellChoices = {null};
        final Runnable paint = () -> {
            boolean on = Boolean.TRUE.equals(state[0]);
            b.setText(on ? "Subscribed" : "Subscribe");
            b.setTextColor(on ? Ui.TEXT : Ui.ON_ACCENT);
            b.setBackground(new RippleDrawable(ColorStateList.valueOf(Ui.RIPPLE), Ui.rounded(on ? Ui.PANEL : Ui.ACCENT, 20), null));
            bell.setVisibility(on && bellChoices[0] != null && bellChoices[0].length() > 0 ? View.VISIBLE : View.GONE);
        };
        paint.run();
        final Runnable ask = () -> Core.get().call((e, err) -> {
            JSONObject ok = e == null ? null : e.optJSONObject("ok");
            if (ok == null) {
                return;
            }
            if (!ok.isNull("subscribed")) {
                state[0] = ok.optBoolean("subscribed");
            }
            bellChoices[0] = ok.optJSONArray("bell");
            String now = ok.optString("bell_now");
            bell.setImageResource("NOTIFICATIONS_ACTIVE".equals(now) ? R.drawable.ic_bell_active
                    : "NOTIFICATIONS_OFF".equals(now) ? R.drawable.ic_bell_off : R.drawable.ic_bell);
            paint.run();
        }, "subscription", "id", id);
        if (Core.get().isSignedIn()) {
            ask.run();
        }
        b.setOnClickListener(v -> {
            if (!Core.get().isSignedIn()) {
                Toast.makeText(c, "Sign in to subscribe.", Toast.LENGTH_SHORT).show();
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
                    Toast.makeText(c, "YouTube did not take that: " + err, Toast.LENGTH_LONG).show();
                } else {
                    // The bell choices come with the new state.
                    ask.run();
                }
            }, "subscribe", "id", id, "on", wanted);
        });
        bell.setOnClickListener(v -> {
            JSONArray choices = bellChoices[0];
            if (choices == null) {
                return;
            }
            Sheet s = new Sheet(c, "Notifications");
            for (int i = 0; i < choices.length(); i++) {
                final JSONObject o = choices.optJSONObject(i);
                if (o == null) {
                    continue;
                }
                String icon = o.optString("icon");
                int res = icon.contains("ACTIVE") ? R.drawable.ic_bell_active : icon.contains("OFF") ? R.drawable.ic_bell_off : R.drawable.ic_bell;
                s.add(res, o.optString("label"), () -> Core.get().call((e, err) -> {
                    if (err != null) {
                        Toast.makeText(c, "YouTube did not take that: " + err, Toast.LENGTH_LONG).show();
                    } else {
                        bell.setImageResource(res);
                    }
                }, "set_bell", "params", o.optString("params")));
            }
            s.show();
        });
        return row;
    }

    private void loadAbout() {
        final TextView wait = Ui.text(c, 13, Ui.MUTE, false);
        wait.setText("Loading…");
        about.addView(wait);
        Core.get().call((e, err) -> {
            about.removeAllViews();
            JSONObject ok = e == null ? null : e.optJSONObject("ok");
            if (ok == null) {
                TextView t = Ui.text(c, 13, Ui.MUTE, false);
                t.setText("Could not load this channel's details" + (err == null ? "." : ": " + err));
                about.addView(t);
                return;
            }
            TextView d = Ui.text(c, 14, Ui.TEXT, false);
            d.setText(ok.optString("description"));
            d.setLineSpacing(Ui.dp(4), 1f);
            d.setAutoLinkMask(android.text.util.Linkify.WEB_URLS);
            d.setLinkTextColor(Ui.ACCENT);
            about.addView(d);
            JSONArray links = ok.optJSONArray("links");
            if (links != null && links.length() > 0) {
                about.addView(heading("Links"));
                for (int i = 0; i < links.length(); i++) {
                    JSONObject l = links.optJSONObject(i);
                    if (l == null) {
                        continue;
                    }
                    final String url = l.optString("url");
                    TextView t = Ui.text(c, 14, Ui.ACCENT, false);
                    t.setText(l.optString("name") + "\n" + url);
                    t.setPadding(0, Ui.dp(6), 0, Ui.dp(6));
                    t.setOnClickListener(v -> {
                        try {
                            c.startActivity(new android.content.Intent(android.content.Intent.ACTION_VIEW,
                                    android.net.Uri.parse(url.startsWith("http") ? url : "https://" + url)));
                        } catch (Exception ignored) {
                        }
                    });
                    about.addView(t);
                }
            }
            about.addView(heading("More info"));
            String[] lines = {
                ok.optString("url"),
                ok.isNull("subscribers") ? null : Ui.count(ok.optLong("subscribers")) + " subscribers",
                ok.isNull("videos") ? null : Ui.count(ok.optLong("videos")) + " videos",
                ok.isNull("views") ? null : String.format(java.util.Locale.US, "%,d views", ok.optLong("views")),
                ok.isNull("joined") ? null : "Joined " + ok.optString("joined"),
                ok.isNull("country") ? null : ok.optString("country"),
            };
            for (String line : lines) {
                if (line != null && !line.isEmpty()) {
                    TextView t = Ui.text(c, 14, Ui.TEXT, false);
                    t.setText(line);
                    t.setPadding(0, Ui.dp(5), 0, Ui.dp(5));
                    about.addView(t);
                }
            }
        }, "channel_about", "id", id);
    }

    private TextView heading(String text) {
        TextView t = Ui.text(c, 16, Ui.TEXT, true);
        t.setText(text);
        t.setPadding(0, Ui.dp(18), 0, Ui.dp(6));
        return t;
    }
}
