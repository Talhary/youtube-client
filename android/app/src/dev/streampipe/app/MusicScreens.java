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
import android.widget.TextView;
import android.widget.Toast;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.ArrayList;
import java.util.List;

/** Playing YouTube Music: what plays after what, and the menu of a song. */
final class MusicPlay {
    private MusicPlay() {}

    /** Songs and videos of a list (what can go in the queue). */
    static List<Core.Item> playable(List<Core.Item> list) {
        List<Core.Item> out = new ArrayList<>();
        if (list != null) {
            for (Core.Item i : list) {
                if (i.playable()) {
                    out.add(i);
                }
            }
        }
        return out;
    }

    /**
     * Play a song. From an album or playlist the rest of the list follows; anywhere else the
     * song's radio (an endless mix, as YouTube Music does).
     */
    static void play(Nav nav, Core.Item item, List<Core.Item> list, boolean asList) {
        if (asList) {
            Downloads.get().setQueue(playable(list), item, false);
            nav.open(item);
            return;
        }
        List<Core.Item> one = new ArrayList<>();
        one.add(item);
        Downloads.get().setQueue(one, item, false);
        nav.open(item);
        radio(nav, item.id, null, false);
    }

    /** Shuffle a list and play it. */
    static void shuffle(Nav nav, List<Core.Item> list) {
        Core.Item first = Downloads.get().setQueue(playable(list), null, true);
        if (first != null) {
            nav.open(first);
        }
    }

    /**
     * A radio: after `video` (its mix), or a mix list such as an artist's. `startIt`: the
     * mix is the whole queue and its first song starts.
     */
    static void radio(final Nav nav, final String video, String list, final boolean startIt) {
        Core.get().call((e, err) -> {
            JSONObject ok = e == null ? null : e.optJSONObject("ok");
            JSONArray a = ok == null ? null : ok.optJSONArray("items");
            if (a == null) {
                if (startIt) {
                    nav.toast("Could not start the radio" + (err == null ? "." : ": " + err));
                }
                return;
            }
            List<Core.Item> items = new ArrayList<>();
            for (int i = 0; i < a.length(); i++) {
                JSONObject o = a.optJSONObject(i);
                if (o != null && !(video != null && video.equals(o.optString("id")))) {
                    items.add(Core.Item.music(o));
                }
            }
            Downloads.get().radioCursor = ok.isNull("next") ? -1 : ok.optLong("next", -1);
            if (startIt) {
                Core.Item first = Downloads.get().setQueue(items, null, false);
                if (first != null) {
                    nav.open(first);
                }
            } else {
                Downloads.get().appendToQueue(video, items);
                nav.queueChanged();
            }
        }, "music_queue", "video", video == null ? JSONObject.NULL : video, "list", list == null ? JSONObject.NULL : list);
    }

    /** The ⋮ menu of a Music entry. */
    static void menu(final Context c, final Nav nav, final Core.Item item) {
        if (!item.playable()) {
            Sheet s = new Sheet(c, item.title);
            s.add(R.drawable.ic_chevron, "Open", () -> nav.open(item));
            s.add(R.drawable.ic_share, "Share", () -> shareLink(c, item));
            s.show();
            return;
        }
        Sheet s = new Sheet(c, item.title);
        s.add(R.drawable.ic_queue, "Play next", () -> nav.queue(item, true));
        s.add(R.drawable.ic_playlist, "Add to queue", () -> nav.queue(item, false));
        s.add(R.drawable.ic_explore, "Start radio", () -> radio(nav, item.id, "RDAMVM" + item.id, true));
        s.add(R.drawable.ic_like, "Like", () -> Core.get().call((e, err) -> Toast.makeText(c,
                err == null ? "Added to Liked music" : "YouTube did not take that: " + err, Toast.LENGTH_SHORT).show(),
                "rate", "id", item.id, "rating", "like"));
        s.add(R.drawable.ic_save, "Save to playlist", () -> ItemMenu.saveToPlaylist(c, item.id));
        if (item.channelId != null) {
            s.add(R.drawable.ic_person, "Go to artist", item.channelName, () -> nav.open(page(item.channelId, "artist", item.channelName)));
        }
        if (item.albumId != null) {
            s.add(R.drawable.ic_playlist, "Go to album", item.albumName, () -> nav.open(page(item.albumId, "album", item.albumName)));
        }
        s.add(R.drawable.ic_download, "Download", () -> Downloads.get().choose(c, item));
        s.add(R.drawable.ic_share, "Share", () -> shareLink(c, item));
        s.show();
    }

    static Core.Item page(String id, String kind, String title) {
        Core.Item i = new Core.Item();
        i.music = true;
        i.kind = kind;
        i.id = id;
        i.title = title == null ? "" : title;
        i.type = "artist".equals(kind) ? Core.Item.CHANNEL : Core.Item.PLAYLIST;
        return i;
    }

    static void shareLink(Context c, Core.Item item) {
        String url = item.playable() ? "https://music.youtube.com/watch?v=" + item.id
                : "artist".equals(item.kind) ? "https://music.youtube.com/channel/" + item.id
                : "https://music.youtube.com/browse/" + item.id;
        android.content.Intent send = new android.content.Intent(android.content.Intent.ACTION_SEND);
        send.setType("text/plain");
        send.putExtra(android.content.Intent.EXTRA_SUBJECT, item.title);
        send.putExtra(android.content.Intent.EXTRA_TEXT, url);
        c.startActivity(android.content.Intent.createChooser(send, "Share"));
    }

    /** What a tap on an entry of a Music list does. */
    static MusicList.Host host(final Context c, final Nav nav) {
        return new MusicList.Host() {
            @Override
            public void open(Core.Item item, List<Core.Item> list, int index, boolean listIsAlbum) {
                if (item.playable()) {
                    play(nav, item, list, listIsAlbum);
                } else {
                    nav.open(item);
                }
            }

            @Override
            public void menu(Core.Item item) {
                MusicPlay.menu(c, nav, item);
            }
        };
    }
}

/** The "Music" title: tapping it goes back to YouTube. */
final class MusicBar {
    private MusicBar() {}

    static TextView logo(Context c, final Nav nav) {
        SpannableString s = new SpannableString("StreamPipe Music  ⇄");
        s.setSpan(new ForegroundColorSpan(Ui.ACCENT), 11, 16, Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
        s.setSpan(new ForegroundColorSpan(Ui.MUTE), 16, s.length(), Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
        TextView t = Ui.lines(Ui.text(c, 20, Ui.TEXT, true), 1);
        t.setText(s);
        t.setContentDescription("YouTube Music. Tap to switch to YouTube");
        t.setOnClickListener(v -> nav.setMusicMode(false));
        return t;
    }

    static View search(Context c, Nav nav) {
        return Ui.iconButton(c, R.drawable.ic_search, "Search music", v -> nav.push(new MusicSearchScreen(c, nav)));
    }
}

/** Music Home: the account's shelves (Quick picks, Listen again, mixes, …). */
final class MusicHomeScreen extends Screen {
    private final MusicList list;
    private Boolean loadedFor;

    MusicHomeScreen(Context c, Nav nav) {
        list = new MusicList(c, MusicPlay.host(c, nav));
        list.setEmptyText("Nothing to show right now.");
        list.setSource(done -> Core.get().call((e, err) -> MusicList.deliver(e, err, done), "music_browse", "id", "FEmusic_home"));
        view = page(c, bar(c, nav, false, MusicBar.logo(c, nav),
                Ui.iconButton(c, R.drawable.ic_refresh, "Refresh", v -> list.load()), MusicBar.search(c, nav)), list);
        onCoreChanged();
    }

    @Override
    void onCoreChanged() {
        Boolean now = Core.get().signedIn;
        if (now != null && !now.equals(loadedFor)) {
            loadedFor = now;
            list.load();
        }
    }

    @Override
    void onReselected() {
        list.scrollToTop();
    }
}

/** Explore: new releases, charts, moods and genres, new videos. */
final class MusicExploreScreen extends Screen {
    private final MusicList list;

    MusicExploreScreen(Context c, Nav nav) {
        list = new MusicList(c, MusicPlay.host(c, nav));
        list.setSource(done -> Core.get().call((e, err) -> MusicList.deliver(e, err, done), "music_browse", "id", "FEmusic_explore"));
        view = page(c, bar(c, nav, false, barTitle(c, "Explore"), MusicBar.search(c, nav)), list);
        list.load();
    }

    @Override
    void onReselected() {
        list.scrollToTop();
    }
}

/** The music library: playlists, liked songs, albums, artists, subscriptions, history. */
final class MusicLibraryScreen extends Screen {
    private static final String[] NAMES = {"Playlists", "Songs", "Albums", "Artists", "Subscriptions", "History"};
    private static final String[] PAGES = {"FEmusic_liked_playlists", "FEmusic_liked_videos", "FEmusic_liked_albums",
        "FEmusic_library_corpus_track_artists", "FEmusic_library_corpus_artists", "FEmusic_history"};
    private final MusicList list;
    private int chosen;
    private Boolean loadedFor;

    MusicLibraryScreen(final Context c, final Nav nav) {
        list = new MusicList(c, MusicPlay.host(c, nav));
        list.setHeader(chips(c, NAMES, 0, i -> {
            chosen = i;
            list.load();
        }));
        list.setEmptyText("Nothing here yet.");
        list.setSource(done -> {
            if (!Core.get().isSignedIn()) {
                done.page(null, "Sign in from the You tab in YouTube to see your music library.");
                return;
            }
            // Liked songs play as one list, like a playlist.
            list.albumPage = chosen == 1;
            Core.get().call((e, err) -> MusicList.deliver(e, err, done), "music_browse", "id", PAGES[chosen]);
        });
        view = page(c, bar(c, nav, false, barTitle(c, "Library"), MusicBar.search(c, nav)), list);
        onCoreChanged();
    }

    @Override
    void onCoreChanged() {
        Boolean now = Core.get().signedIn;
        if (now != null && !now.equals(loadedFor)) {
            loadedFor = now;
            list.load();
        }
    }
}

/** An artist, album, playlist, mood or "more" page. */
final class MusicPageScreen extends Screen {
    private final Context c;
    private final Nav nav;
    private final MusicList list;
    private final LinearLayout header;
    private final TextView barTitle;

    MusicPageScreen(Context context, Nav nav, final Core.Item item) {
        this.c = context;
        this.nav = nav;
        barTitle = barTitle(c, item.title == null || item.title.isEmpty() ? "Music" : item.title);
        header = Ui.column(c);
        list = new MusicList(c, MusicPlay.host(c, nav));
        list.setHeader(header);
        list.albumPage = "album".equals(item.kind) || "single".equals(item.kind) || "playlist".equals(item.kind);
        list.setPageListener(this::bindHeader);
        list.setSource(done -> Core.get().call((e, err) -> MusicList.deliver(e, err, done), "music_browse", "id", item.id,
                "params", item.params == null ? JSONObject.NULL : item.params));
        view = page(c, bar(c, nav, true, barTitle, MusicBar.search(c, nav)), list);
        list.load();
    }

    private void bindHeader(JSONObject page) {
        header.removeAllViews();
        final JSONObject h = page.optJSONObject("header");
        if (h == null) {
            return;
        }
        final boolean artist = !h.isNull("channel_id");
        barTitle.setText(h.optString("title"));
        String pic = Core.url(Core.str(h, "thumbnail"));
        if (pic != null) {
            FrameLayout frame = new FrameLayout(c);
            ImageView img = new ImageView(c);
            img.setScaleType(ImageView.ScaleType.CENTER_CROP);
            frame.addView(img, new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));
            Images.load(img, pic);
            int side = Math.round(c.getResources().getDisplayMetrics().widthPixels * (artist ? 1f : 0.62f));
            if (artist) {
                header.addView(frame, Ui.lin(Ui.MATCH, Math.round(side * 0.75f)));
            } else {
                Ui.clipRound(frame, 10);
                LinearLayout.LayoutParams lp = Ui.lin(side, side);
                lp.gravity = Gravity.CENTER_HORIZONTAL;
                lp.topMargin = Ui.dp(16);
                header.addView(frame, lp);
            }
        }
        TextView title = Ui.text(c, 24, Ui.TEXT, true);
        title.setText(h.optString("title"));
        title.setGravity(artist ? Gravity.START : Gravity.CENTER_HORIZONTAL);
        title.setPadding(Ui.dp(16), Ui.dp(14), Ui.dp(16), 0);
        header.addView(title, Ui.lin(Ui.MATCH, Ui.WRAP));
        String sub = Ui.join(h.optString("subtitle", null), Core.str(h, "second_subtitle"), Core.str(h, "subscribers"));
        if (!sub.isEmpty()) {
            TextView s = Ui.text(c, 13, Ui.MUTE, false);
            s.setText(sub);
            s.setGravity(artist ? Gravity.START : Gravity.CENTER_HORIZONTAL);
            s.setPadding(Ui.dp(16), Ui.dp(6), Ui.dp(16), 0);
            header.addView(s, Ui.lin(Ui.MATCH, Ui.WRAP));
        }
        String about = Core.str(h, "description");
        if (about != null && !artist) {
            final TextView d = Ui.text(c, 13, Ui.MUTE, false);
            d.setText(about);
            Ui.lines(d, 2);
            d.setGravity(Gravity.CENTER_HORIZONTAL);
            d.setPadding(Ui.dp(16), Ui.dp(8), Ui.dp(16), 0);
            d.setOnClickListener(v -> d.setMaxLines(d.getMaxLines() == 2 ? Integer.MAX_VALUE : 2));
            header.addView(d, Ui.lin(Ui.MATCH, Ui.WRAP));
        }
        LinearLayout buttons = Ui.row(c);
        buttons.setGravity(artist ? Gravity.START : Gravity.CENTER_HORIZONTAL);
        buttons.setPadding(Ui.dp(16), Ui.dp(14), Ui.dp(16), Ui.dp(6));
        if (artist) {
            final String channel = h.optString("channel_id");
            final boolean[] on = {h.optBoolean("subscribed")};
            final TextView sub2 = Ui.button(c, on[0] ? "Subscribed" : "Subscribe", null);
            sub2.setOnClickListener(v -> {
                if (!Core.get().isSignedIn()) {
                    Toast.makeText(c, "Sign in to subscribe.", Toast.LENGTH_SHORT).show();
                    return;
                }
                on[0] = !on[0];
                sub2.setText(on[0] ? "Subscribed" : "Subscribe");
                Core.get().call((e, err) -> {
                    if (err != null) {
                        on[0] = !on[0];
                        sub2.setText(on[0] ? "Subscribed" : "Subscribe");
                        Toast.makeText(c, "YouTube did not take that: " + err, Toast.LENGTH_LONG).show();
                    }
                }, "subscribe", "id", channel, "on", on[0]);
            });
            buttons.addView(sub2);
            final String radio = Core.str(h, "radio");
            if (radio != null) {
                addPill(buttons, R.drawable.ic_explore, "Radio", v -> MusicPlay.radio(nav, null, radio, true));
            }
            addPill(buttons, R.drawable.ic_shuffle, "Shuffle", v -> MusicPlay.shuffle(nav, list.tracks()));
        } else {
            addPill(buttons, R.drawable.ic_play, "Play", v -> {
                List<Core.Item> tracks = MusicPlay.playable(list.tracks());
                if (!tracks.isEmpty()) {
                    MusicPlay.play(nav, tracks.get(0), tracks, true);
                }
            });
            addPill(buttons, R.drawable.ic_shuffle, "Shuffle", v -> MusicPlay.shuffle(nav, list.tracks()));
            final String listId = Core.str(h, "list_id");
            if (listId != null) {
                addPill(buttons, R.drawable.ic_save, "Save", v -> Core.get().call((e, err) -> Toast.makeText(c,
                        err == null ? "Saved to your library" : "YouTube did not take that: " + err, Toast.LENGTH_SHORT).show(),
                        "music_save", "list", listId, "on", true));
                final String plain = listId.startsWith("VL") ? listId.substring(2) : listId;
                addPill(buttons, R.drawable.ic_download, "Download", v -> Downloads.get().choosePlaylist(c, plain, h.optString("title")));
            }
        }
        android.widget.HorizontalScrollView scroll = new android.widget.HorizontalScrollView(c);
        scroll.setHorizontalScrollBarEnabled(false);
        scroll.addView(buttons);
        header.addView(scroll, Ui.lin(Ui.MATCH, Ui.WRAP));
    }

    private void addPill(LinearLayout into, int icon, String label, View.OnClickListener onClick) {
        LinearLayout p = Ui.row(c);
        p.setPadding(Ui.dp(12), Ui.dp(9), Ui.dp(16), Ui.dp(9));
        p.setBackground(new android.graphics.drawable.RippleDrawable(
                android.content.res.ColorStateList.valueOf(Ui.RIPPLE), Ui.rounded(Ui.PANEL, 20), null));
        p.addView(Ui.icon(c, icon, Ui.TEXT), Ui.lin(Ui.dp(20), Ui.dp(20)));
        TextView t = Ui.text(c, 14, Ui.TEXT, true);
        t.setText(label);
        t.setPadding(Ui.dp(8), 0, 0, 0);
        p.addView(t);
        p.setOnClickListener(onClick);
        LinearLayout.LayoutParams lp = Ui.lin(Ui.WRAP, Ui.WRAP);
        lp.leftMargin = into.getChildCount() == 0 ? 0 : Ui.dp(10);
        into.addView(p, lp);
    }
}

/** Music search: suggestions while typing, then results by tab (songs, albums, …). */
final class MusicSearchScreen extends Screen {
    private static final String[] TABS = {"All", "Songs", "Videos", "Albums", "Artists", "Playlists", "Podcasts"};
    private static final String[] KEYS = {"", "songs", "videos", "albums", "artists", "playlists", "podcasts"};
    private final Handler main = new Handler(Looper.getMainLooper());
    private final EditText field;
    private final MusicList results;
    private final ListView suggestions;
    private final List<String> suggestionItems = new ArrayList<>();
    private final ArrayAdapter<String> suggestionAdapter;
    private String query = "";
    private int tab;
    private int generation;
    private Runnable pending;

    MusicSearchScreen(final Context c, final Nav nav) {
        field = new EditText(c);
        field.setHint("Search songs, albums, artists");
        field.setHintTextColor(Ui.MUTE);
        field.setTextColor(Ui.TEXT);
        field.setTextSize(15);
        field.setSingleLine(true);
        field.setImeOptions(EditorInfo.IME_ACTION_SEARCH | EditorInfo.IME_FLAG_NO_EXTRACT_UI);
        field.setBackground(Ui.rounded(Ui.PANEL, 20));
        field.setPadding(Ui.dp(16), 0, Ui.dp(16), 0);
        LinearLayout.LayoutParams fl = new LinearLayout.LayoutParams(0, Ui.dp(40), 1f);
        fl.rightMargin = Ui.dp(12);
        field.setLayoutParams(fl);

        results = new MusicList(c, MusicPlay.host(c, nav));
        results.setHeader(chips(c, TABS, 0, i -> {
            tab = i;
            if (!query.isEmpty()) {
                results.load();
            }
        }));
        results.setEmptyText("");
        results.setSource(done -> {
            if (query.isEmpty()) {
                done.page(new JSONObject(), null);
                return;
            }
            Core.get().call((e, err) -> MusicList.deliver(e, err, done), "music_search", "query", query, "tab", KEYS[tab]);
        });

        suggestionAdapter = new ArrayAdapter<String>(c, 0, suggestionItems) {
            @Override
            public View getView(int position, View view, ViewGroup parent) {
                LinearLayout row = (LinearLayout) view;
                if (row == null) {
                    row = Ui.row(c);
                    row.setPadding(Ui.dp(8), 0, Ui.dp(16), 0);
                    row.setMinimumHeight(Ui.dp(48));
                    row.addView(Ui.icon(c, R.drawable.ic_search, Ui.MUTE), Ui.lin(Ui.dp(40), Ui.dp(40)));
                    row.addView(Ui.lines(Ui.text(c, 15, Ui.TEXT, false), 1), Ui.weight(Ui.WRAP));
                }
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
        suggestions.setOnItemClickListener((p, v, position, id) -> run(suggestionItems.get(position)));

        FrameLayout body = new FrameLayout(c);
        body.addView(results, new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));
        body.addView(suggestions, new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));
        LinearLayout bar = Ui.row(c);
        bar.setPadding(Ui.dp(4), 0, Ui.dp(4), 0);
        bar.addView(Ui.iconButton(c, R.drawable.ic_back, "Back", v -> {
            hideKeyboard();
            nav.pop();
        }));
        bar.addView(field);
        bar.setLayoutParams(Ui.lin(Ui.MATCH, Ui.dp(56)));
        view = page(c, bar, body);
        field.setOnEditorActionListener((v, a, e) -> {
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
            field.requestFocus();
            main.postDelayed(() -> {
                InputMethodManager imm = (InputMethodManager) field.getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
                if (imm != null) {
                    imm.showSoftInput(field, InputMethodManager.SHOW_IMPLICIT);
                }
            }, 120);
        }
    }

    private void hideKeyboard() {
        InputMethodManager imm = (InputMethodManager) field.getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
        if (imm != null) {
            imm.hideSoftInputFromWindow(field.getWindowToken(), 0);
        }
        field.clearFocus();
    }

    private void suggest(final String text) {
        if (pending != null) {
            main.removeCallbacks(pending);
        }
        final int mine = ++generation;
        if (text.isEmpty() || text.equals(query)) {
            suggestions.setVisibility(View.GONE);
            return;
        }
        pending = () -> Core.get().call((e, err) -> {
            if (mine != generation || e == null) {
                return;
            }
            JSONArray a = e.optJSONArray("items");
            suggestionItems.clear();
            for (int i = 0; a != null && i < a.length(); i++) {
                suggestionItems.add(a.optString(i));
            }
            suggestionAdapter.notifyDataSetChanged();
            suggestions.setVisibility(suggestionItems.isEmpty() ? View.GONE : View.VISIBLE);
        }, "music_suggest", "query", text);
        main.postDelayed(pending, 220);
    }

    private void run(String text) {
        String q = text.trim();
        if (q.isEmpty()) {
            return;
        }
        query = q;
        generation++;
        if (!q.equals(field.getText().toString())) {
            field.setText(q);
            field.setSelection(q.length());
        }
        suggestions.setVisibility(View.GONE);
        hideKeyboard();
        results.load();
    }
}
