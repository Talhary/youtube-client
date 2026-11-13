package dev.streampipe.app;

import android.content.Context;
import android.content.Intent;
import android.widget.Toast;

import org.json.JSONArray;
import org.json.JSONObject;

/**
 * The "more" menu of a video (the ⋮ button or a long press), and saving a video to a
 * playlist, as on YouTube.
 */
final class ItemMenu {
    private ItemMenu() {}

    /** `hide` removes the video from the list it is in ("Not interested"); null where that makes no sense. */
    static void show(final Context c, final Nav nav, final Core.Item item, final Runnable hide) {
        if (item.type == Core.Item.POST) {
            if (item.attached != null) {
                show(c, nav, item.attached, null);
            }
            return;
        }
        if (item.type != Core.Item.VIDEO) {
            return;
        }
        Sheet s = new Sheet(c, item.title);
        s.add(R.drawable.ic_queue, "Play next", () -> nav.queue(item, true));
        s.add(R.drawable.ic_playlist, "Add to queue", () -> nav.queue(item, false));
        s.add(R.drawable.ic_watch_later, "Save to Watch later", () -> watchLater(c, item.id));
        s.add(R.drawable.ic_save, "Save to playlist", () -> saveToPlaylist(c, item.id));
        s.add(R.drawable.ic_download, "Download", () -> Downloads.get().choose(c, item));
        s.add(R.drawable.ic_share, "Share", () -> share(c, item, 0));
        if (item.channelId != null) {
            s.add(R.drawable.ic_person, "Go to channel", item.channelName, () -> {
                Core.Item ch = new Core.Item();
                ch.type = Core.Item.CHANNEL;
                ch.id = item.channelId;
                ch.title = item.channelName == null ? "Channel" : item.channelName;
                nav.open(ch);
            });
        }
        if (hide != null) {
            s.add(R.drawable.ic_close, "Not interested", "Hides it, and tells YouTube when signed in", () -> {
                hide.run();
                Core.get().call((e, err) -> {
                    if (err != null) {
                        Toast.makeText(c, "Hidden here; YouTube did not take it: " + err, Toast.LENGTH_LONG).show();
                    }
                }, "not_interested", "id", item.id);
            });
        }
        s.show();
    }

    static void share(Context c, Core.Item item, int atSeconds) {
        Intent send = new Intent(Intent.ACTION_SEND);
        send.setType("text/plain");
        send.putExtra(Intent.EXTRA_SUBJECT, item.title);
        send.putExtra(Intent.EXTRA_TEXT, "https://youtu.be/" + item.id + (atSeconds > 0 ? "?t=" + atSeconds : ""));
        c.startActivity(Intent.createChooser(send, "Share video"));
    }

    private static boolean needSignIn(Context c) {
        if (Core.get().isSignedIn()) {
            return false;
        }
        Toast.makeText(c, "Sign in to save videos to playlists.", Toast.LENGTH_SHORT).show();
        return true;
    }

    static void watchLater(final Context c, String videoId) {
        if (needSignIn(c)) {
            return;
        }
        Core.get().call((e, err) -> Toast.makeText(c, err == null ? "Saved to Watch later" : "YouTube did not take that: " + err,
                Toast.LENGTH_SHORT).show(), "edit_playlist", "playlist", "WL", "video", videoId, "add", true);
    }

    /** The user's playlists with a check on those that hold the video; tapping one adds or removes it. */
    static void saveToPlaylist(final Context c, final String videoId) {
        if (needSignIn(c)) {
            return;
        }
        final Sheet s = new Sheet(c, "Save video to…");
        s.note("Loading your playlists…");
        s.show();
        Core.get().call((e, err) -> {
            if (!s.isShowing()) {
                return;
            }
            s.clear();
            JSONObject ok = e == null ? null : e.optJSONObject("ok");
            JSONArray lists = ok == null ? null : ok.optJSONArray("playlists");
            if (err != null || lists == null) {
                s.note("Could not load your playlists" + (err == null ? "." : ": " + err));
                return;
            }
            for (int i = 0; i < lists.length(); i++) {
                final JSONObject p = lists.optJSONObject(i);
                if (p == null) {
                    continue;
                }
                final String id = p.optString("id");
                String privacy = p.optString("privacy");
                String hint = privacy.isEmpty() ? null : privacy.charAt(0) + privacy.substring(1).toLowerCase(java.util.Locale.ROOT);
                s.check(p.optString("title"), hint, p.optBoolean("contains"), wanted -> {
                    Core.get().call((e2, err2) -> {
                        if (err2 != null) {
                            Toast.makeText(c, "YouTube did not take that: " + err2, Toast.LENGTH_LONG).show();
                        } else {
                            Toast.makeText(c, (wanted ? "Saved to " : "Removed from ") + p.optString("title"), Toast.LENGTH_SHORT).show();
                        }
                    }, "edit_playlist", "playlist", id, "video", videoId, "add", wanted);
                    return true;
                });
            }
            s.add(R.drawable.ic_save, "New playlist", () -> createPlaylist(c, videoId));
        }, "playlist_options", "id", videoId);
    }

    static void createPlaylist(final Context c, final String videoId) {
        Sheet.ask(c, "New playlist", "Name", "Next", name -> Sheet.choose(c, "Who can see \"" + name + "\"?",
                new String[] {"Private", "Unlisted", "Public"}, 0, which -> {
                    String privacy = new String[] {"PRIVATE", "UNLISTED", "PUBLIC"}[which];
                    Core.get().call((e, err) -> Toast.makeText(c, err == null ? "Made \"" + name + "\"" + (videoId == null ? "" : " and saved the video")
                            : "YouTube did not take that: " + err, Toast.LENGTH_SHORT).show(),
                            "create_playlist", "title", name, "privacy", privacy, "video", videoId == null ? JSONObject.NULL : videoId);
                }));
    }
}
