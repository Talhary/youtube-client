package dev.streampipe.app;

import android.annotation.SuppressLint;
import android.content.Context;
import android.content.SharedPreferences;
import android.graphics.Color;
import android.net.Uri;
import android.os.Handler;
import android.os.Looper;
import android.view.View;
import android.webkit.CookieManager;
import android.webkit.WebSettings;
import android.webkit.WebStorage;
import android.webkit.WebView;
import android.webkit.WebViewClient;
import android.widget.LinearLayout;

import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.Locale;

/**
 * Signing in happens on Google's own page inside a web view. Web views in one app share
 * their cookies, so once this page is signed in the player is too. The same cookies are
 * handed to the core so its own requests (feeds, history, playlists) are the user's.
 */
final class SignIn {
    private static final String LOGIN_URL =
            "https://accounts.google.com/ServiceLogin?service=youtube&uilel=3&passive=true"
                    + "&continue=https%3A%2F%2Fm.youtube.com%2Fsignin%3Faction_handle_signin%3Dtrue%26app%3Dm%26next%3D%252F";

    private SignIn() {}

    private static String youtubeCookies() {
        return CookieManager.getInstance().getCookie("https://www.youtube.com/");
    }

    /** Does this cookie header hold a signed-in YouTube session? */
    static boolean signedIn(String cookieHeader) {
        if (cookieHeader == null) {
            return false;
        }
        boolean sapisid = false, login = false;
        for (String part : cookieHeader.split(";")) {
            String name = part.trim();
            int eq = name.indexOf('=');
            if (eq <= 0 || eq == name.length() - 1) {
                continue;
            }
            name = name.substring(0, eq);
            sapisid |= name.equals("SAPISID") || name.equals("__Secure-3PAPISID");
            login |= name.equals("LOGIN_INFO");
        }
        return sapisid && login;
    }

    static boolean webSignedIn() {
        return signedIn(youtubeCookies());
    }

    private static SharedPreferences prefs(Context c) {
        return c.getSharedPreferences("shell", Context.MODE_PRIVATE);
    }

    /** Give the core the web session's cookies if they changed since the last time. */
    static void sync(Context c) {
        String header = youtubeCookies();
        if (!signedIn(header)) {
            return;
        }
        String hash = sha256(header);
        if (hash.equals(prefs(c).getString("cookieHash", ""))) {
            return;
        }
        StringBuilder txt = new StringBuilder("# Netscape HTTP Cookie File\n");
        for (String part : header.split(";")) {
            String p = part.trim();
            int eq = p.indexOf('=');
            if (eq <= 0) {
                continue;
            }
            txt.append(".youtube.com\tTRUE\t/\tTRUE\t2000000000\t").append(p, 0, eq).append('\t').append(p.substring(eq + 1)).append('\n');
        }
        Core.get().send("import_cookies", "text", txt.toString());
        prefs(c).edit().putString("cookieHash", hash).apply();
    }

    /** Sign out everywhere: the core's session and the web views' cookies and storage. */
    static void signOut(Context c) {
        Core.get().send("logout");
        CookieManager.getInstance().removeAllCookies(null);
        CookieManager.getInstance().flush();
        WebStorage.getInstance().deleteAllData();
        prefs(c).edit().remove("cookieHash").apply();
    }

    private static String sha256(String s) {
        try {
            byte[] d = MessageDigest.getInstance("SHA-256").digest(s.getBytes(StandardCharsets.UTF_8));
            StringBuilder sb = new StringBuilder();
            for (byte b : d) {
                sb.append(String.format(Locale.ROOT, "%02x", b));
            }
            return sb.toString();
        } catch (Exception e) {
            return String.valueOf(s.hashCode());
        }
    }

    /** The sign-in page as a screen. It closes itself once the session exists. */
    static final class Page extends Screen {
        private final Handler main = new Handler(Looper.getMainLooper());
        private final WebView web;
        private final Nav nav;
        private boolean done;

        @SuppressLint("SetJavaScriptEnabled")
        Page(final Context c, final Nav nav) {
            this.nav = nav;
            web = new WebView(c);
            web.setBackgroundColor(Color.WHITE);
            WebSettings s = web.getSettings();
            s.setJavaScriptEnabled(true);
            s.setDomStorageEnabled(true);
            s.setUserAgentString(Player.chromeUserAgent(s.getUserAgentString()));
            CookieManager.getInstance().setAcceptCookie(true);
            CookieManager.getInstance().setAcceptThirdPartyCookies(web, true);
            web.setWebViewClient(new WebViewClient() {
                @Override
                public void onPageFinished(WebView view, String url) {
                    check();
                }
            });
            LinearLayout col = (LinearLayout) page(c, bar(c, nav, true, barTitle(c, "Sign in to YouTube")), web);
            view = col;
            web.loadUrl(LOGIN_URL);
            main.postDelayed(this::poll, 1500);
        }

        private void poll() {
            if (done || web.getParent() == null && view.getParent() == null) {
                return;
            }
            check();
            main.postDelayed(this::poll, 1500);
        }

        /** Signed in once the page is back on YouTube with the session cookies set. */
        private void check() {
            if (done) {
                return;
            }
            String url = web.getUrl();
            String host = url == null ? null : Uri.parse(url).getHost();
            boolean onYouTube = host != null && (host.equals("youtube.com") || host.endsWith(".youtube.com"));
            if (onYouTube && webSignedIn()) {
                done = true;
                CookieManager.getInstance().flush();
                sync(view.getContext());
                web.stopLoading();
                nav.pop();
            }
        }

        boolean goBackInPage() {
            if (web.canGoBack()) {
                web.goBack();
                return true;
            }
            return false;
        }

        void destroy() {
            done = true;
            main.removeCallbacksAndMessages(null);
            web.stopLoading();
            web.destroy();
        }

        @Override
        void onShown() {
            view.setVisibility(View.VISIBLE);
        }
    }
}
