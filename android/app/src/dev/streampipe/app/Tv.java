package dev.streampipe.app;

import android.content.Context;
import android.net.wifi.WifiManager;
import android.os.Handler;
import android.os.Looper;
import android.util.Log;
import android.widget.Toast;

import java.io.ByteArrayOutputStream;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.DatagramPacket;
import java.net.HttpURLConnection;
import java.net.InetAddress;
import java.net.MulticastSocket;
import java.net.SocketTimeoutException;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * "Play on TV": finds TVs and streaming sticks on the Wi-Fi with DIAL (the discovery that
 * YouTube's own apps used before Cast) and asks the YouTube app on the chosen one to play the
 * video from the current time. No Google Play services are needed. TVs that only speak the
 * Cast protocol do not answer.
 */
final class Tv {
    private static final String TAG = "StreamPipe";

    private static final class Device {
        String name;
        String appUrl;
    }

    private Tv() {}

    static void choose(final Context c, final String videoId, final int atSeconds) {
        final Sheet s = new Sheet(c, "Play on TV");
        final android.widget.TextView note = s.note("Looking for TVs on this Wi-Fi…");
        s.show();
        final Handler main = new Handler(Looper.getMainLooper());
        new Thread(() -> {
            final List<Device> found = discover(c);
            main.post(() -> {
                if (!s.isShowing()) {
                    return;
                }
                s.clear();
                if (found.isEmpty()) {
                    s.note("No TV answered. The TV must be on the same Wi-Fi with its YouTube app installed; TVs that only support Google Cast do not show here.");
                    return;
                }
                for (final Device d : found) {
                    s.add(R.drawable.ic_cast, d.name, () -> launch(c, d, videoId, atSeconds));
                }
            });
        }, "dial-discover").start();
    }

    private static List<Device> discover(Context c) {
        List<Device> out = new ArrayList<>();
        Map<String, Boolean> locations = new LinkedHashMap<>();
        WifiManager wifi = (WifiManager) c.getApplicationContext().getSystemService(Context.WIFI_SERVICE);
        WifiManager.MulticastLock lock = wifi == null ? null : wifi.createMulticastLock("streampipe-dial");
        MulticastSocket socket = null;
        try {
            if (lock != null) {
                lock.acquire();
            }
            socket = new MulticastSocket();
            socket.setSoTimeout(700);
            byte[] ask = ("M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\n"
                    + "ST: urn:dial-multiscreen-org:service:dial:1\r\n\r\n").getBytes(StandardCharsets.US_ASCII);
            InetAddress group = InetAddress.getByName("239.255.255.250");
            long until = System.currentTimeMillis() + 3200;
            int sent = 0;
            byte[] buf = new byte[2048];
            while (System.currentTimeMillis() < until) {
                if (sent < 3) {
                    socket.send(new DatagramPacket(ask, ask.length, group, 1900));
                    sent++;
                }
                try {
                    DatagramPacket p = new DatagramPacket(buf, buf.length);
                    socket.receive(p);
                    String answer = new String(p.getData(), 0, p.getLength(), StandardCharsets.UTF_8);
                    for (String line : answer.split("\r\n")) {
                        int colon = line.indexOf(':');
                        if (colon > 0 && line.substring(0, colon).trim().equalsIgnoreCase("LOCATION")) {
                            locations.put(line.substring(colon + 1).trim(), true);
                        }
                    }
                } catch (SocketTimeoutException ignored) {
                }
            }
        } catch (Exception e) {
            Log.w(TAG, "dial: " + e);
        } finally {
            if (socket != null) {
                socket.close();
            }
            if (lock != null && lock.isHeld()) {
                lock.release();
            }
        }
        for (String location : locations.keySet()) {
            HttpURLConnection h = null;
            try {
                h = (HttpURLConnection) new URL(location).openConnection();
                h.setConnectTimeout(2500);
                h.setReadTimeout(2500);
                String appUrl = h.getHeaderField("Application-URL");
                String xml = read(h.getInputStream());
                if (appUrl == null) {
                    continue;
                }
                Device d = new Device();
                d.appUrl = appUrl.endsWith("/") ? appUrl : appUrl + "/";
                java.util.regex.Matcher m = java.util.regex.Pattern.compile("<friendlyName>([^<]*)</friendlyName>").matcher(xml);
                d.name = m.find() ? m.group(1).trim() : new URL(location).getHost();
                out.add(d);
            } catch (Exception e) {
                Log.w(TAG, "dial: " + location + ": " + e);
            } finally {
                if (h != null) {
                    h.disconnect();
                }
            }
        }
        return out;
    }

    private static String read(InputStream in) throws java.io.IOException {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        byte[] b = new byte[4096];
        int n;
        while ((n = in.read(b)) > 0 && out.size() < 256 * 1024) {
            out.write(b, 0, n);
        }
        in.close();
        return out.toString("UTF-8");
    }

    private static void launch(final Context c, final Device d, final String videoId, final int at) {
        final Handler main = new Handler(Looper.getMainLooper());
        new Thread(() -> {
            String result;
            HttpURLConnection h = null;
            try {
                h = (HttpURLConnection) new URL(d.appUrl + "YouTube").openConnection();
                h.setConnectTimeout(4000);
                h.setReadTimeout(8000);
                h.setRequestMethod("POST");
                h.setDoOutput(true);
                h.setRequestProperty("Content-Type", "text/plain; charset=utf-8");
                byte[] body = ("v=" + videoId + (at > 0 ? "&t=" + at : "")).getBytes(StandardCharsets.UTF_8);
                try (OutputStream o = h.getOutputStream()) {
                    o.write(body);
                }
                int code = h.getResponseCode();
                result = code == 201 || code == 200 ? "Playing on " + d.name : d.name + " answered " + code;
            } catch (Exception e) {
                result = "Could not reach " + d.name + ": " + e.getMessage();
            } finally {
                if (h != null) {
                    h.disconnect();
                }
            }
            final String text = result;
            main.post(() -> Toast.makeText(c, text, Toast.LENGTH_LONG).show());
        }, "dial-launch").start();
    }
}
