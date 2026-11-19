package dev.streampipe.app;

import android.app.Activity;
import android.app.PictureInPictureParams;
import android.content.SharedPreferences;
import android.content.pm.ActivityInfo;
import android.content.pm.PackageManager;
import android.content.res.ColorStateList;
import android.content.res.Configuration;
import android.graphics.Color;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.util.Rational;
import android.view.Gravity;
import android.view.View;
import android.view.Window;
import android.view.WindowManager;
import android.webkit.CookieManager;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.TextView;
import android.widget.Toast;

import java.util.ArrayList;
import java.util.List;

/**
 * The app's single screen: four tabs with a bottom bar, pages pushed over them, and the
 * watch layer (player + details) on top, which shrinks to a mini bar while browsing.
 */
public final class MainActivity extends Activity
        implements Nav, Core.Listener, Player.Listener, WatchLayout.Host, PlaybackService.Controller, SettingsSheet.Host {
    private static final int NAV_HEIGHT_DP = 56;
    /** YouTube's bottom bar: outline icons, filled for the tab on screen; the middle one is "Create". */
    private static final int[] TAB_ICONS = {R.drawable.ic_home_outline, R.drawable.ic_shorts_outline, R.drawable.ic_add, R.drawable.ic_subs_outline, R.drawable.ic_person};
    private static final int[] TAB_ICONS_ON = {R.drawable.ic_home_filled, R.drawable.ic_shorts_filled, R.drawable.ic_add, R.drawable.ic_subs_filled, R.drawable.ic_person};
    private static final String[] TAB_LABELS = {"Home", "Shorts", "", "Subscriptions", "You"};
    private static final int TAB_COUNT = 5;
    private static final int TAB_SHORTS = 1;
    private static final int TAB_CREATE = 2;
    private static final int TAB_YOU = 4;
    /** The account's picture in the "You" tab, as in YouTube's bar. */
    private ImageView youAvatar;

    private final Handler main = new Handler(Looper.getMainLooper());
    private FrameLayout root;
    private FrameLayout content;
    private LinearLayout navBar;
    private WatchLayout watch;
    private Player player;
    private boolean fullscreen;
    private SettingsSheet sheet;

    private static final int[] MUSIC_ICONS = {R.drawable.ic_home, R.drawable.ic_explore, R.drawable.ic_library, R.drawable.ic_settings};
    private static final String[] MUSIC_LABELS = {"Home", "Explore", "Library", "Settings"};
    /** YouTube's tabs and YouTube Music's; `tabs` is the set on screen. */
    private final Screen[] videoTabs = new Screen[TAB_COUNT];
    private final Screen[] musicTabs = new Screen[TAB_COUNT];
    private Screen[] tabs = videoTabs;
    private boolean musicMode;
    private final ImageView[] tabIcons = new ImageView[TAB_COUNT];
    private final TextView[] tabTexts = new TextView[TAB_COUNT];
    private int tabIndex = -1;
    private final List<Screen> stack = new ArrayList<>();
    private boolean watchOpen;
    private Boolean accountFor;
    private boolean notificationShown;
    private boolean askedNotifications;

    // ------------------------------------------------------------------ lifecycle

    @Override
    protected void onCreate(Bundle state) {
        super.onCreate(state);
        requestWindowFeature(Window.FEATURE_NO_TITLE);
        setRequestedOrientation(ActivityInfo.SCREEN_ORIENTATION_PORTRAIT);
        Ui.init(this, lightTheme());
        getWindow().setStatusBarColor(Ui.BG);
        getWindow().setNavigationBarColor(Ui.BG);
        getWindow().setBackgroundDrawable(new android.graphics.drawable.ColorDrawable(Ui.BG));
        getWindow().getDecorView().setSystemUiVisibility(baseSystemUi());
        Images.init(this);
        CookieManager.getInstance().setAcceptCookie(true);

        Core core = Core.get();
        core.start(getApplicationContext(), versionName());
        core.addListener(this);

        root = new FrameLayout(this);
        root.setBackgroundColor(Ui.BG);
        setContentView(root);

        content = new FrameLayout(this);
        FrameLayout.LayoutParams cl = new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH);
        cl.bottomMargin = Ui.dp(NAV_HEIGHT_DP);
        root.addView(content, cl);

        navBar = Ui.row(this);
        navBar.setBackgroundColor(Ui.BG);
        for (int i = 0; i < TAB_COUNT; i++) {
            final int index = i;
            LinearLayout item = Ui.column(this);
            item.setGravity(Gravity.CENTER);
            item.setBackground(Ui.rippleRound());
            tabIcons[i] = Ui.icon(this, TAB_ICONS[i], Ui.TEXT);
            FrameLayout iconBox = new FrameLayout(this);
            iconBox.addView(tabIcons[i], new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));
            if (i == TAB_YOU) {
                youAvatar = new ImageView(this);
                youAvatar.setScaleType(ImageView.ScaleType.CENTER_CROP);
                Ui.clipCircle(youAvatar);
                youAvatar.setVisibility(View.GONE);
                iconBox.addView(youAvatar, Ui.frame(Ui.dp(24), Ui.dp(24), Gravity.CENTER));
            }
            item.addView(iconBox, Ui.lin(Ui.dp(i == TAB_CREATE ? 40 : 28), Ui.dp(i == TAB_CREATE ? 40 : 26)));
            if (i == TAB_CREATE) {
                tabIcons[i].setBackground(Ui.rounded(Ui.PANEL, 20));
            }
            tabTexts[i] = Ui.text(this, 10, Ui.TEXT, false);
            tabTexts[i].setText(TAB_LABELS[i]);
            tabTexts[i].setPadding(0, Ui.dp(2), 0, 0);
            if (i == TAB_CREATE) {
                tabTexts[i].setVisibility(View.GONE);
            }
            item.addView(tabTexts[i], Ui.lin(Ui.WRAP, Ui.WRAP));
            item.setContentDescription(TAB_LABELS[i]);
            item.setOnClickListener(v -> onTabTapped(index));
            navBar.addView(item, new LinearLayout.LayoutParams(0, Ui.MATCH, 1f));
        }
        FrameLayout navWrap = new FrameLayout(this);
        View line = new View(this);
        line.setBackgroundColor(Ui.LINE);
        navLine = line;
        navWrap.addView(navBar, new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));
        navWrap.addView(line, Ui.frame(Ui.MATCH, Math.max(1, Ui.dp(0.75f)), Gravity.TOP));
        root.addView(navWrap, Ui.frame(Ui.MATCH, Ui.dp(NAV_HEIGHT_DP), Gravity.BOTTOM));
        navBarWrap = navWrap;

        player = new Player(this, this);
        watch = new WatchLayout(this, player, this);
        watch.setBottomInset(Ui.dp(NAV_HEIGHT_DP));
        root.addView(watch, new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));
        player.preferredQuality = prefs().getString("quality", "auto");
        player.preferredRate = prefs().getFloat("rate", 1f);
        player.audioOnly = prefs().getBoolean("audioOnly", false);
        player.stableVolume = prefs().getBoolean("stable", false);
        PlayerControls.swipeControls = prefs().getBoolean("swipeControls", false);
        player.preferredCaption = prefs().getString("captionLang", "");
        player.setDirect(directPlayer());
        Resume.init(this);
        Core.get().prefetch = directPlayer() && prefs().getBoolean("prefetch", true);
        sheet = new SettingsSheet(this, this);
        root.addView(sheet, new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));

        PlaybackService.controller = this;
        if (prefs().getBoolean("musicMode", false)) {
            musicMode = true;
            tabs = musicTabs;
            labelTabs();
        }
        // After a theme change the app is redrawn; come back to the tab the user was on.
        selectTab(restoreTab >= 0 && restoreTab < TAB_COUNT ? restoreTab : 0);
        restoreTab = -1;
        player.readRulesInfo();
        // Load YouTube once in the background so the first video opens without a page load.
        main.postDelayed(() -> player.warmUp(), 2500);
        // Keep the core's copy of the web session current.
        main.post(this::syncCookiesLoop);
        openLink(getIntent());
    }

    @Override
    protected void onNewIntent(android.content.Intent intent) {
        super.onNewIntent(intent);
        setIntent(intent);
        openLink(intent);
    }

    private static final java.util.regex.Pattern LINK_ID = java.util.regex.Pattern.compile(
            "(?:[?&]v=|youtu\\.be/|/shorts/|/embed/|/live/|vnd\\.youtube:(?://)?)([A-Za-z0-9_-]{11})(?![A-Za-z0-9_-])");

    /** A YouTube link opened with the app, or shared to it: play that video. */
    private void openLink(android.content.Intent intent) {
        if (intent == null) {
            return;
        }
        String text = null;
        if (android.content.Intent.ACTION_VIEW.equals(intent.getAction()) && intent.getData() != null) {
            text = intent.getData().toString();
        } else if (android.content.Intent.ACTION_SEND.equals(intent.getAction())) {
            text = intent.getStringExtra(android.content.Intent.EXTRA_TEXT);
        }
        if (text == null) {
            return;
        }
        // Handle it once: the activity may be recreated with the same intent.
        intent.setAction(android.content.Intent.ACTION_MAIN);
        java.util.regex.Matcher m = LINK_ID.matcher(text);
        if (!m.find()) {
            Toast.makeText(this, "That link is not a YouTube video.", Toast.LENGTH_LONG).show();
            return;
        }
        final Core.Item video = new Core.Item();
        video.type = Core.Item.VIDEO;
        video.id = m.group(1);
        video.title = "";
        video.thumb = "https://i.ytimg.com/vi/" + video.id + "/mqdefault.jpg";
        main.post(() -> open(video));
    }

    private View navBarWrap;
    private View navLine;
    /** The tab to show after the activity is recreated for a theme change. */
    private static int restoreTab = -1;

    /** Light theme chosen, or "same as the phone" while the phone is in its light theme. */
    private boolean lightTheme() {
        String t = theme();
        if ("light".equals(t)) {
            return true;
        }
        if ("dark".equals(t)) {
            return false;
        }
        int night = getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK;
        return night == Configuration.UI_MODE_NIGHT_NO;
    }

    /** System bar appearance outside fullscreen: dark icons on the light theme. */
    private int baseSystemUi() {
        int flags = View.SYSTEM_UI_FLAG_VISIBLE;
        if (Ui.light) {
            flags |= View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR;
            if (Build.VERSION.SDK_INT >= 26) {
                flags |= View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR;
            }
        }
        return flags;
    }

    @Override
    public String theme() {
        return prefs().getString("theme", "system");
    }

    @Override
    public void setTheme(String theme) {
        prefs().edit().putString("theme", theme).apply();
        restoreTab = tabIndex;
        // Every view takes its colours when it is built, so the screen is built again.
        recreate();
    }

    private String versionName() {
        try {
            return getPackageManager().getPackageInfo(getPackageName(), 0).versionName;
        } catch (Exception e) {
            return "0";
        }
    }

    private void syncCookiesLoop() {
        SignIn.sync(this);
        main.postDelayed(this::syncCookiesLoop, 20 * 60 * 1000L);
    }

    private SharedPreferences prefs() {
        return getSharedPreferences("shell", MODE_PRIVATE);
    }

    /** Should the player keep running when the app leaves the screen? */
    private boolean keepPlaying() {
        return backgroundPlay() && watch.isOpen() && player.playing;
    }

    @Override
    protected void onPause() {
        super.onPause();
        // Decide before the window is hidden: the web view stops media when told it is hidden.
        player.web.keepAlive = keepPlaying();
        android.util.Log.i("StreamPipe", "app: onPause keepAlive=" + player.web.keepAlive + " playing=" + player.playing);
        CookieManager.getInstance().flush();
    }

    @Override
    protected void onStop() {
        super.onStop();
        android.util.Log.i("StreamPipe", "app: onStop keepPlaying=" + keepPlaying());
        if (onShorts()) {
            tabs[TAB_SHORTS].onHidden();
        }
        if (!keepPlaying()) {
            player.web.keepAlive = false;
            if (watch.isOpen()) {
                player.command("pause");
            }
            player.web.onPause();
        }
    }

    @Override
    protected void onStart() {
        super.onStart();
        android.util.Log.i("StreamPipe", "app: onStart");
        if (onShorts() && stack.isEmpty() && !watch.isExpanded()) {
            tabs[TAB_SHORTS].onShown();
        }
        player.web.keepAlive = false;
        player.web.onResume();
    }

    @Override
    protected void onDestroy() {
        Core.get().removeListener(this);
        if (PlaybackService.controller == this) {
            PlaybackService.controller = null;
        }
        PlaybackService.stop(this);
        main.removeCallbacksAndMessages(null);
        for (Screen s : stack) {
            if (s instanceof SignIn.Page) {
                ((SignIn.Page) s).destroy();
            }
        }
        root.removeAllViews();
        if (videoTabs[TAB_SHORTS] != null) {
            ((ShortsScreen) videoTabs[TAB_SHORTS]).destroy();
        }
        player.web.destroy();
        player.release();
        super.onDestroy();
    }

    /** Home pressed while a video is playing on screen: shrink to picture-in-picture. */
    @Override
    protected void onUserLeaveHint() {
        super.onUserLeaveHint();
        // Audio only has no picture worth floating: it just keeps playing in the background.
        if (Build.VERSION.SDK_INT >= 26 && watch.isExpanded() && player.playing && !player.audioOnly
                && getPackageManager().hasSystemFeature(PackageManager.FEATURE_PICTURE_IN_PICTURE)) {
            try {
                setFullscreen(false);
                enterPictureInPictureMode(new PictureInPictureParams.Builder().setAspectRatio(new Rational(16, 9)).build());
            } catch (RuntimeException e) {
                // Picture-in-picture is switched off for this app in system settings.
            }
        }
    }

    @Override
    public void onPictureInPictureModeChanged(boolean inPip, Configuration newConfig) {
        super.onPictureInPictureModeChanged(inPip, newConfig);
        android.util.Log.i("StreamPipe", "app: picture-in-picture " + inPip);
        if (inPip) {
            sheet.dismiss();
        }
        content.setVisibility(inPip ? View.GONE : View.VISIBLE);
        navBarWrap.setVisibility(inPip ? View.GONE : View.VISIBLE);
        watch.setPip(inPip);
    }

    @Override
    public void onBackPressed() {
        Screen top = stack.isEmpty() ? null : stack.get(stack.size() - 1);
        if (sheet.isShowing()) {
            sheet.dismiss();
        } else if (fullscreen) {
            setFullscreen(false);
        } else if (top instanceof SignIn.Page && ((SignIn.Page) top).goBackInPage()) {
            // Went back inside Google's page.
        } else if (watch.isExpanded() && watch.closeComments()) {
            // Closed the comments.
        } else if (watch.isExpanded()) {
            watch.minimize();
        } else if (top != null) {
            pop();
        } else if (onShorts() && ((ShortsScreen) tabs[TAB_SHORTS]).goBack()) {
            // Went back to the previous Short.
        } else if (tabIndex != 0) {
            selectTab(0);
        } else {
            moveTaskToBack(true);
        }
    }

    // ------------------------------------------------------------------ tabs and pages

    /** The Shorts tab is the one on screen. */
    private boolean onShorts() {
        return !musicMode && tabIndex == TAB_SHORTS && tabs[TAB_SHORTS] != null;
    }

    private Screen tab(int i) {
        if (tabs[i] == null && musicMode) {
            switch (i) {
                case 0:
                    tabs[i] = new MusicHomeScreen(this, this);
                    break;
                case 1:
                    tabs[i] = new MusicExploreScreen(this, this);
                    break;
                case 2:
                    tabs[i] = new MusicLibraryScreen(this, this);
                    break;
                default:
                    tabs[i] = new SettingsScreen(this, this);
            }
            tabs[i].view.setVisibility(View.GONE);
            content.addView(tabs[i].view, 0, new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));
        }
        if (tabs[i] == null) {
            switch (i) {
                case 0:
                    tabs[i] = new FeedScreen(this, this, false);
                    break;
                case TAB_SHORTS:
                    tabs[i] = new ShortsScreen(this, this);
                    break;
                case 3:
                    tabs[i] = new FeedScreen(this, this, true);
                    break;
                default:
                    tabs[i] = new YouScreen(this, this);
            }
            tabs[i].view.setVisibility(View.GONE);
            content.addView(tabs[i].view, 0, new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));
        }
        return tabs[i];
    }

    private void onTabTapped(int i) {
        if (!musicMode && i == TAB_CREATE) {
            Toast.makeText(this, "Uploading is not part of this app.", Toast.LENGTH_SHORT).show();
            return;
        }
        if (!stack.isEmpty()) {
            // A tab tap leaves whatever was pushed on top.
            while (!stack.isEmpty()) {
                removeTop(false);
            }
            if (i == tabIndex) {
                tabs[i].view.setVisibility(View.VISIBLE);
                tabs[i].view.setAlpha(1f);
                tabs[i].view.setTranslationX(0);
                return;
            }
        }
        if (i == tabIndex) {
            tabs[i].onReselected();
        } else {
            selectTab(i);
        }
    }

    private void selectTab(int i) {
        final Screen old = tabIndex >= 0 ? tabs[tabIndex] : null;
        Screen next = tab(i);
        tabIndex = i;
        paintTabs(i);
        if (old != null && old != next) {
            old.view.animate().cancel();
            old.view.setVisibility(View.GONE);
            old.onHidden();
        }
        // Shorts and a playing video would talk over each other; as on YouTube, the mini
        // player steps aside while Shorts are on screen and comes back on the other tabs.
        if (!musicMode && i == TAB_SHORTS && watch.isOpen() && player.playing) {
            player.command("pause");
        }
        if (watch.isOpen() && !watch.isExpanded()) {
            watch.setVisibility(!musicMode && i == TAB_SHORTS ? View.INVISIBLE : View.VISIBLE);
        }
        next.view.animate().cancel();
        next.view.setTranslationX(0);
        next.view.setVisibility(View.VISIBLE);
        next.view.setAlpha(0f);
        next.view.setTranslationY(Ui.dp(10));
        next.view.animate().alpha(1f).translationY(0).setDuration(Ui.FAST).setInterpolator(Ui.EASE_OUT).start();
        next.onShown();
    }

    /** The tab on screen has a filled icon; on Shorts the bar turns black, as in YouTube. */
    private void paintTabs(int i) {
        boolean shorts = !musicMode && i == TAB_SHORTS;
        int bg = shorts ? 0xFF000000 : Ui.BG;
        int fg = shorts ? 0xFFFFFFFF : Ui.TEXT;
        navBar.setBackgroundColor(bg);
        navLine.setVisibility(shorts ? View.GONE : View.VISIBLE);
        if (!fullscreen) {
            getWindow().setStatusBarColor(bg);
            getWindow().setNavigationBarColor(bg);
            getWindow().getDecorView().setSystemUiVisibility(shorts ? View.SYSTEM_UI_FLAG_VISIBLE : baseSystemUi());
        }
        for (int t = 0; t < TAB_COUNT; t++) {
            if (!musicMode) {
                tabIcons[t].setImageResource(t == i ? TAB_ICONS_ON[t] : TAB_ICONS[t]);
            }
            int color = musicMode ? (t == i ? Ui.TEXT : Ui.MUTE) : fg;
            tabIcons[t].setImageTintList(ColorStateList.valueOf(color));
            tabTexts[t].setTextColor(color);
            tabTexts[t].setTypeface(android.graphics.Typeface.create(t == i ? "sans-serif-medium" : "sans-serif", android.graphics.Typeface.NORMAL));
        }
        if (!musicMode) {
            tabIcons[TAB_CREATE].setBackground(Ui.rounded(shorts ? 0x33FFFFFF : Ui.PANEL, 20));
        }
        boolean avatar = !musicMode && youAvatar.getDrawable() != null;
        youAvatar.setVisibility(avatar ? View.VISIBLE : View.GONE);
        tabIcons[TAB_YOU].setVisibility(avatar ? View.INVISIBLE : View.VISIBLE);
        if (avatar) {
            youAvatar.setForeground(i == TAB_YOU ? ringed(fg) : null);
        }
    }

    private android.graphics.drawable.Drawable ringed(int color) {
        android.graphics.drawable.GradientDrawable g = new android.graphics.drawable.GradientDrawable();
        g.setShape(android.graphics.drawable.GradientDrawable.OVAL);
        g.setStroke(Ui.dp(1.5f), color);
        return g;
    }

    /** The signed-in account's name, handle and picture (the "You" tab). */
    private void loadAccount() {
        if (!Core.get().isSignedIn()) {
            youAvatar.setImageDrawable(null);
            YouScreen.account = null;
            paintTabs(tabIndex);
            return;
        }
        Core.get().call((e, err) -> {
            org.json.JSONObject ok = e == null ? null : e.optJSONObject("ok");
            if (ok == null) {
                return;
            }
            YouScreen.account = ok;
            Images.load(youAvatar, ok.optString("avatar"));
            main.postDelayed(() -> paintTabs(tabIndex), 800);
            if (videoTabs[TAB_YOU] != null) {
                videoTabs[TAB_YOU].onCoreChanged();
            }
        }, "shorts", "op", "account");
    }

    @Override
    public void openShort(String id) {
        while (!stack.isEmpty()) {
            removeTop(false);
        }
        if (watch.isExpanded()) {
            watch.minimize();
        }
        ShortsScreen s = (ShortsScreen) tab(TAB_SHORTS);
        s.openAt(id);
        if (tabIndex == TAB_SHORTS) {
            s.onShown();
        } else {
            selectTab(TAB_SHORTS);
        }
    }

    private View below() {
        return stack.isEmpty() ? tabs[tabIndex].view : stack.get(stack.size() - 1).view;
    }

    @Override
    public void push(final Screen screen) {
        if (watch.isExpanded()) {
            watch.minimize();
        }
        final View under = below();
        stack.add(screen);
        int w = Math.max(content.getWidth(), 1);
        content.addView(screen.view, new FrameLayout.LayoutParams(Ui.MATCH, Ui.MATCH));
        screen.view.setTranslationX(w);
        screen.view.animate().translationX(0).setDuration(Ui.MEDIUM).setInterpolator(Ui.EASE_OUT)
                .withEndAction(() -> {
                    // Hidden pages are not drawn; they come back when this one is popped.
                    if (stack.contains(screen)) {
                        under.setVisibility(View.INVISIBLE);
                    }
                    screen.onShown();
                }).start();
        under.animate().translationX(-w * 0.25f).alpha(0.5f).setDuration(Ui.MEDIUM).setInterpolator(Ui.EASE_OUT).start();
    }

    @Override
    public void pop() {
        if (!stack.isEmpty()) {
            removeTop(true);
        }
    }

    private void removeTop(boolean animated) {
        final Screen top = stack.remove(stack.size() - 1);
        final View under = below();
        under.animate().cancel();
        under.setVisibility(View.VISIBLE);
        top.view.animate().cancel();
        final Runnable finish = () -> {
            content.removeView(top.view);
            if (top instanceof SignIn.Page) {
                ((SignIn.Page) top).destroy();
            }
        };
        if (animated) {
            int w = Math.max(content.getWidth(), 1);
            top.view.animate().translationX(w).setDuration(Ui.MEDIUM).setInterpolator(Ui.EASE).withEndAction(finish).start();
            under.animate().translationX(0).alpha(1f).setDuration(Ui.MEDIUM).setInterpolator(Ui.EASE).start();
        } else {
            under.setTranslationX(0);
            under.setAlpha(1f);
            finish.run();
        }
    }

    @Override
    public void open(Core.Item item) {
        if (item.music && !item.playable()) {
            push(new MusicPageScreen(this, this, item));
            return;
        }
        switch (item.type) {
            case Core.Item.CHANNEL:
                push(new ChannelScreen(this, this, item.id, item.title));
                break;
            case Core.Item.PLAYLIST:
                push(ListScreen.playlist(this, this, item.id, item.title));
                break;
            default:
                if (onShorts()) {
                    tabs[TAB_SHORTS].onHidden();
                }
                watch.show(item);
        }
    }

    @Override
    public void signIn() {
        push(new SignIn.Page(this, this));
    }

    @Override
    public void signOut() {
        watch.close();
        SignIn.signOut(this);
    }

    @Override
    public boolean backgroundPlay() {
        return prefs().getBoolean("backgroundPlay", true);
    }

    @Override
    public void setBackgroundPlay(boolean on) {
        prefs().edit().putBoolean("backgroundPlay", on).apply();
    }

    @Override
    public boolean directPlayer() {
        return !"youtube".equals(prefs().getString("player", "direct"));
    }

    /** Takes effect with the next video. */
    @Override
    public void setDirectPlayer(boolean on) {
        prefs().edit().putString("player", on ? "direct" : "youtube").apply();
        player.setDirect(on);
        Core.get().prefetch = on && prefs().getBoolean("prefetch", true);
    }

    @Override
    public String adRulesText() {
        return player.adRulesText;
    }

    // ------------------------------------------------------------------ core events

    @Override
    public void onCoreChanged() {
        Boolean in = Core.get().signedIn;
        if (in != null && !in.equals(accountFor)) {
            accountFor = in;
            loadAccount();
        }
        for (Screen[] set : new Screen[][] {videoTabs, musicTabs}) {
            for (Screen s : set) {
                if (s != null) {
                    s.onCoreChanged();
                }
            }
        }
        for (Screen s : stack) {
            s.onCoreChanged();
        }
    }

    @Override
    public void onToast(String text) {
        if (text != null && !text.isEmpty()) {
            Toast.makeText(this, text, Toast.LENGTH_LONG).show();
        }
    }

    // ------------------------------------------------------------------ watch layer

    @Override
    public void onWatchProgress(float progress, boolean open) {
        // The bottom bar slides away as the video expands and returns as it shrinks.
        navBarWrap.setTranslationY(open ? (1f - progress) * Ui.dp(NAV_HEIGHT_DP) : 0);
        if (open != watchOpen) {
            watchOpen = open;
            // Leave room at the bottom of the lists for the mini bar.
            FrameLayout.LayoutParams lp = (FrameLayout.LayoutParams) content.getLayoutParams();
            // The mini player floats over the lists, as on YouTube: no room is kept for it.
            lp.bottomMargin = Ui.dp(NAV_HEIGHT_DP);
            content.setLayoutParams(lp);
        }
    }

    @Override
    public void openChannel(String id, String name) {
        if (watch.current != null && watch.current.music) {
            push(new MusicPageScreen(this, this, MusicPlay.page(id, "artist", name)));
            return;
        }
        push(new ChannelScreen(this, this, id, name));
    }

    @Override
    public boolean musicMode() {
        return musicMode;
    }

    @Override
    public void setMusicMode(boolean on) {
        if (on == musicMode) {
            return;
        }
        prefs().edit().putBoolean("musicMode", on).apply();
        while (!stack.isEmpty()) {
            removeTop(false);
        }
        if (tabIndex >= 0 && tabs[tabIndex] != null) {
            tabs[tabIndex].view.setVisibility(View.GONE);
            tabs[tabIndex].onHidden();
        }
        musicMode = on;
        tabs = on ? musicTabs : videoTabs;
        labelTabs();
        tabIndex = -1;
        selectTab(0);
    }

    /** The bottom bar's names and icons for the mode (music has four tabs). */
    private void labelTabs() {
        for (int i = 0; i < TAB_COUNT; i++) {
            View item = (View) tabIcons[i].getParent();
            if (musicMode && i >= MUSIC_LABELS.length) {
                item.setVisibility(View.GONE);
                continue;
            }
            item.setVisibility(View.VISIBLE);
            tabIcons[i].setImageResource(musicMode ? MUSIC_ICONS[i] : TAB_ICONS[i]);
            tabIcons[i].setBackground(!musicMode && i == TAB_CREATE ? Ui.rounded(Ui.PANEL, 20) : null);
            tabTexts[i].setVisibility(!musicMode && i == TAB_CREATE ? View.GONE : View.VISIBLE);
            tabTexts[i].setText(musicMode ? MUSIC_LABELS[i] : TAB_LABELS[i]);
            item.setContentDescription(musicMode ? MUSIC_LABELS[i] : TAB_LABELS[i]);
        }
    }

    @Override
    public void queueChanged() {
        watch.refreshUpNext();
    }

    @Override
    public void toast(String text) {
        Toast.makeText(this, text, Toast.LENGTH_SHORT).show();
    }

    @Override
    public void queue(Core.Item item, boolean next) {
        if (!watch.isOpen() || watch.current == null) {
            open(item);
            return;
        }
        Downloads.get().enqueue(watch.current, item, next);
        watch.refreshUpNext();
        Toast.makeText(this, next ? "Playing next" : "Added to queue", Toast.LENGTH_SHORT).show();
    }

    @Override
    public void onNowPlaying() {
        Core.Item v = watch.current;
        if (!watch.isOpen() || v == null) {
            if (notificationShown) {
                notificationShown = false;
                PlaybackService.stop(this);
            }
            getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
            return;
        }
        if (player.playing) {
            getWindow().addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        } else {
            getWindow().clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON);
        }
        // The notification appears once something has actually started playing.
        if (player.playing || notificationShown) {
            if (!notificationShown && Build.VERSION.SDK_INT >= 33 && !askedNotifications
                    && checkSelfPermission("android.permission.POST_NOTIFICATIONS") != PackageManager.PERMISSION_GRANTED) {
                askedNotifications = true;
                requestPermissions(new String[] {"android.permission.POST_NOTIFICATIONS"}, 1);
            }
            notificationShown = true;
            PlaybackService.update(this, v.title, v.channelName, player.playing, Images.cached(v.thumb));
        }
    }

    // ------------------------------------------------------------------ player events

    @Override
    public void onVideo(String id) {
        watch.onPlayerVideo(id);
    }

    @Override
    public void onEnded(String id) {
        if (sleepAtEnd) {
            // The sleep timer was set to the end of this video: stop here.
            sleepAtEnd = false;
            Toast.makeText(this, "Sleep timer: playback stopped", Toast.LENGTH_SHORT).show();
            return;
        }
        watch.onPlayerEnded(id);
    }

    @Override
    public void onPlaying(boolean playing) {
        watch.onPlayerPlaying(playing);
    }

    @Override
    public void onSettings(org.json.JSONObject info) {
        try {
            info.put("loop", Downloads.get().repeat == 2);
            long left = sleepAt - System.currentTimeMillis();
            info.put("sleep", sleepAtEnd ? "End of video" : sleepAt > 0 && left > 0 ? "Off in " + Math.max(1, left / 60000) + " min" : "Off");
            info.put("sleep_minutes", sleepAtEnd ? -1 : sleepAt > 0 ? sleepMinutes : 0);
        } catch (org.json.JSONException ignored) {
        }
        sheet.audioOnly = player.audioOnly;
        sheet.show(info);
    }

    /** A choice from the settings sheet: apply it now and remember quality and speed. */
    @Override
    public void onPlayerSetting(String what, Object value) {
        if ("loop".equals(what)) {
            Downloads.get().repeat = Boolean.TRUE.equals(value) ? 2 : 0;
            return;
        }
        if ("sleep".equals(what)) {
            setSleep(value instanceof Number ? ((Number) value).intValue() : 0);
            return;
        }
        if ("stable".equals(what)) {
            boolean on = Boolean.TRUE.equals(value);
            player.command("stable", on ? 1 : 0);
            prefs().edit().putBoolean("stable", on).apply();
            return;
        }
        if ("captions".equals(what)) {
            player.command(what, value);
            prefs().edit().putString("captionLang", player.preferredCaption).apply();
            return;
        }
        if ("audio".equals(what)) {
            boolean on = Boolean.TRUE.equals(value);
            player.setAudioOnly(on);
            prefs().edit().putBoolean("audioOnly", on).apply();
            return;
        }
        player.command(what, value);
        if ("quality".equals(what)) {
            player.preferredQuality = String.valueOf(value);
            prefs().edit().putString("quality", player.preferredQuality).apply();
        } else if ("rate".equals(what) && value instanceof Number) {
            player.preferredRate = ((Number) value).doubleValue();
            prefs().edit().putFloat("rate", (float) player.preferredRate).apply();
        }
    }

    // ------------------------------------------------------------------ sleep timer

    private long sleepAt;
    private int sleepMinutes;
    private boolean sleepAtEnd;
    private final Runnable sleepNow = () -> {
        sleepAt = 0;
        player.command("pause");
        Toast.makeText(this, "Sleep timer: playback paused", Toast.LENGTH_SHORT).show();
    };

    /** Pause after `minutes` (-1: at the end of this video, 0: off). */
    private void setSleep(int minutes) {
        main.removeCallbacks(sleepNow);
        sleepAt = 0;
        sleepAtEnd = minutes < 0;
        sleepMinutes = minutes;
        if (minutes > 0) {
            sleepAt = System.currentTimeMillis() + minutes * 60_000L;
            main.postDelayed(sleepNow, minutes * 60_000L);
        }
        Toast.makeText(this, minutes == 0 ? "Sleep timer off" : minutes < 0 ? "Stops at the end of this video"
                : "Pauses in " + minutes + " minutes", Toast.LENGTH_SHORT).show();
    }

    @Override
    public void setAudioOnly(boolean on) {
        player.setAudioOnly(on);
        prefs().edit().putBoolean("audioOnly", on).apply();
    }

    @Override
    public void onFullscreenToggle() {
        setFullscreen(!fullscreen);
    }

    @Override
    public void requestFullscreen(boolean on) {
        setFullscreen(on);
    }

    /**
     * Fullscreen is done by the app, not by the page: the screen turns to landscape, the
     * system bars hide and the watch layer lets the player fill everything. The video stays
     * in the same view throughout, so it keeps playing.
     */
    private void setFullscreen(boolean on) {
        if (fullscreen == on || (on && !watch.isOpen())) {
            return;
        }
        fullscreen = on;
        android.util.Log.i("StreamPipe", "app: fullscreen " + on);
        if (on) {
            watch.expand();
            getWindow().addFlags(WindowManager.LayoutParams.FLAG_FULLSCREEN);
            getWindow().getDecorView().setSystemUiVisibility(View.SYSTEM_UI_FLAG_FULLSCREEN
                    | View.SYSTEM_UI_FLAG_HIDE_NAVIGATION | View.SYSTEM_UI_FLAG_IMMERSIVE_STICKY);
            setRequestedOrientation(ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE);
        } else {
            getWindow().clearFlags(WindowManager.LayoutParams.FLAG_FULLSCREEN);
            getWindow().getDecorView().setSystemUiVisibility(baseSystemUi());
            setRequestedOrientation(ActivityInfo.SCREEN_ORIENTATION_PORTRAIT);
        }
        watch.setFullscreen(on);
    }

    // ------------------------------------------------------------------ notification buttons

    @Override
    public void togglePlayback() {
        main.post(() -> player.command("toggle"));
    }

    @Override
    public void stopPlayback() {
        main.post(() -> {
            setFullscreen(false);
            watch.close();
        });
    }
}
