package dev.streampipe.app;

import android.content.Context;
import android.content.SharedPreferences;

import org.json.JSONArray;
import org.json.JSONObject;

import java.util.LinkedHashMap;
import java.util.Map;

/**
 * Where each video was left, so it starts there again (and lists show the red "watched"
 * line), as YouTube does. Kept on the phone for the last few hundred videos.
 */
final class Resume {
    private static final int KEEP = 400;
    private static SharedPreferences prefs;
    /** Video id → {seconds, total}; oldest first. */
    private static final LinkedHashMap<String, double[]> spots = new LinkedHashMap<>(16, 0.75f, true);
    private static long lastSave;

    private Resume() {}

    static void init(Context c) {
        if (prefs != null) {
            return;
        }
        prefs = c.getSharedPreferences("resume", Context.MODE_PRIVATE);
        try {
            JSONObject all = new JSONObject(prefs.getString("spots", "{}"));
            JSONArray order = all.optJSONArray("order");
            JSONObject at = all.optJSONObject("at");
            for (int i = 0; order != null && at != null && i < order.length(); i++) {
                String id = order.optString(i);
                JSONArray v = at.optJSONArray(id);
                if (v != null) {
                    spots.put(id, new double[] {v.optDouble(0), v.optDouble(1)});
                }
            }
        } catch (Exception ignored) {
        }
    }

    /** Where to start a video: 0 for new ones and for ones watched (nearly) to the end. */
    static double startAt(String id) {
        double[] s = id == null ? null : spots.get(id);
        if (s == null || s[0] < 10 || (s[1] > 0 && s[0] > s[1] - 20)) {
            return 0;
        }
        return s[0];
    }

    /** How much of a video was watched here, in percent (0 when unknown). */
    static int percent(String id) {
        double[] s = id == null ? null : spots.get(id);
        if (s == null || s[1] <= 0) {
            return 0;
        }
        // Finished videos show as fully watched.
        return s[0] > s[1] - 20 ? 100 : (int) Math.round(100 * s[0] / s[1]);
    }

    /** The player is at `seconds` of `total` (live streams are not remembered). */
    static void played(String id, double seconds, double total) {
        if (id == null || total <= 0 || seconds < 5) {
            return;
        }
        spots.put(id, new double[] {seconds, total});
        while (spots.size() > KEEP) {
            String oldest = spots.keySet().iterator().next();
            spots.remove(oldest);
        }
        long now = System.currentTimeMillis();
        if (now - lastSave > 5000) {
            lastSave = now;
            save();
        }
    }

    static void save() {
        if (prefs == null) {
            return;
        }
        try {
            JSONArray order = new JSONArray();
            JSONObject at = new JSONObject();
            for (Map.Entry<String, double[]> e : spots.entrySet()) {
                order.put(e.getKey());
                at.put(e.getKey(), new JSONArray().put(Math.round(e.getValue()[0])).put(Math.round(e.getValue()[1])));
            }
            prefs.edit().putString("spots", new JSONObject().put("order", order).put("at", at).toString()).apply();
        } catch (Exception ignored) {
        }
    }

    /** Forget every position (with "Clear watch history"). */
    static void clear() {
        spots.clear();
        save();
    }
}
