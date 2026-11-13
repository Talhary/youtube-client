package dev.streampipe.app;

import android.animation.Animator;
import android.animation.AnimatorListenerAdapter;
import android.animation.ValueAnimator;
import android.content.Context;
import android.graphics.Color;
import android.view.Gravity;
import android.view.MotionEvent;
import android.view.VelocityTracker;
import android.view.View;
import android.view.ViewConfiguration;
import android.widget.FrameLayout;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.TextView;

import org.json.JSONObject;

/**
 * The watch screen. It sits over the whole app and moves between two shapes:
 *
 * - expanded: player at the top, details and "Up next" below;
 * - mini: a bar above the bottom navigation with a small player, the title and two buttons.
 *
 * Dragging the player down or up moves between them; `progress` is 0 when expanded and 1
 * when mini, and every position in between is drawn, so the motion follows the finger.
 * Swiping the mini bar sideways closes the video.
 */
final class WatchLayout extends FrameLayout {
    interface Host {
        /** 0 = expanded … 1 = mini; `open` is false once the video is closed. */
        void onWatchProgress(float progress, boolean open);

        void openChannel(String id, String name);

        /** What is playing changed (for the notification). */
        void onNowPlaying();

        /** A swipe on the player asks to enter or leave fullscreen. */
        void requestFullscreen(boolean on);

        /** The "Audio only" button was pressed. */
        void setAudioOnly(boolean on);
    }

    private static final float MINI_HEIGHT_DP = 58;

    private final Host host;
    private final Player player;
    private final View scrim;
    private final FrameLayout playerFrame;
    private final LinearLayout likeButton, dislikeButton;
    private MusicPanel musicPanel;
    /** Music layout: square artwork with the music controls under it. */
    private boolean music;

    /**
     * Sound is what is playing (a downloaded song, or audio-only mode) in the native player:
     * show the music layout. Otherwise the video layout.
     */
    private void updateMusic() {
        boolean on = open && current != null && player.playingDirect && (player.audioOnly || current.localAudio);
        if (music != on) {
            music = on;
            player.setMusic(on);
            controls.setMusic(on);
            musicPanel.setVisibility(on ? VISIBLE : GONE);
            requestLayout();
            post(this::apply);
        }
    }
    private final PlayerControls controls;
    private final FeedList details;
    private final CommentsPanel comments;
    private final LiveChatPanel chat;
    private final LinearLayout commentsCard;
    private final ImageView teaserAvatar;
    private final TextView teaserText;
    private final LinearLayout chatButton;
    /** The live chat handle of the current video, when it has one. */
    private String chatToken;
    /** Chapters under the description. */
    private final LinearLayout chapterRow;
    private final View chapterBox;
    /** Ambient mode: a soft glow in the picture's colour behind the details. */
    private final View glow;
    private int glowColor;
    private final Runnable ambientTick = this::ambient;
    private final LinearLayout miniBar;
    private final TextView miniTitle, miniChannel;
    private final ImageView miniPlay;
    private final TextView title, meta, channelName, description, descriptionToggle;
    /** Shown over the player while a video is loading: its thumbnail, dimmed, and a spinner. */
    private final FrameLayout loadingCover;
    private final ImageView loadingImage;
    private final Runnable hideLoadingLater = this::hideLoading;
    /** Fullscreen pinch: out zooms the picture to fill the screen, in shows all of it. */
    private final android.view.ScaleGestureDetector pinch;
    private float pinchScale = 1f;
    private boolean pinching;
    private boolean zoomToFill;
    private final TextView zoomLabel;
    private final Runnable hideZoomLabel = () -> zoomLabelView().animate().alpha(0f).setDuration(Ui.MEDIUM).start();
    private final LinearLayout audioButton;
    private final ImageView audioIcon;
    private final TextView audioLabel;
    private final View channelRow;
    private final ImageView channelAvatar;
    private final TextView channelSubs;

    private final int slop;
    private final int flingVelocity;
    private VelocityTracker velocity;
    private ValueAnimator animator;
    private float progress = 1f;
    private boolean open;
    private boolean pip;
    private boolean fullscreen;
    private int bottomInset;
    /** What the current drag does: move between expanded and mini, or switch fullscreen. */
    private static final int DRAG_NONE = 0, DRAG_SHRINK = 1, DRAG_TO_FULLSCREEN = 2, DRAG_FROM_FULLSCREEN = 3;
    private int drag = DRAG_NONE;

    private float downX, downY, startProgress;
    private boolean tracking, dragging, horizontal;

    /** The video on screen, the one that plays next, and a counter to drop stale answers. */
    Core.Item current;
    private Core.Item nextUp;
    private int detailsGeneration;
    private boolean descriptionOpen;

    WatchLayout(Context c, Player player, Host host) {
        super(c);
        this.player = player;
        this.host = host;
        ViewConfiguration vc = ViewConfiguration.get(c);
        slop = vc.getScaledTouchSlop();
        flingVelocity = Ui.dp(700);
        setVisibility(GONE);

        scrim = new View(c);
        scrim.setBackgroundColor(Ui.BG);
        scrim.setClickable(true);
        addView(scrim, new LayoutParams(Ui.MATCH, Ui.MATCH));
        glow = new View(c);
        glow.setAlpha(0f);
        addView(glow, new LayoutParams(Ui.MATCH, Ui.dp(260)));

        // Details: a header with the video's information, then the related videos.
        LinearLayout header = Ui.column(c);
        title = Ui.text(c, 17, Ui.TEXT, true);
        title.setLineSpacing(Ui.dp(3), 1f);
        title.setPadding(Ui.dp(16), Ui.dp(14), Ui.dp(16), 0);
        header.addView(title, Ui.lin(Ui.MATCH, Ui.WRAP));
        meta = Ui.text(c, 12, Ui.MUTE, false);
        meta.setPadding(Ui.dp(16), Ui.dp(7), Ui.dp(16), Ui.dp(12));
        header.addView(meta, Ui.lin(Ui.MATCH, Ui.WRAP));
        musicPanel = new MusicPanel(c, player, new MusicPanel.Host() {
            @Override
            public boolean step(int by) {
                return WatchLayout.this.step(by);
            }

            @Override
            public void onEngineChanged() {
                post(WatchLayout.this::updateMusic);
            }
        });
        musicPanel.setVisibility(GONE);
        header.addView(musicPanel, Ui.lin(Ui.MATCH, Ui.WRAP));

        // Actions under the title, in a row that scrolls sideways.
        LinearLayout actions = Ui.row(c);
        actions.setPadding(Ui.dp(16), 0, Ui.dp(16), Ui.dp(14));
        likeButton = pill(c, R.drawable.ic_like, "Like", v -> rate("like"));
        actions.addView(likeButton, Ui.lin(Ui.WRAP, Ui.WRAP));
        dislikeButton = pill(c, R.drawable.ic_dislike, "Dislike", v -> rate("dislike"));
        actions.addView(dislikeButton, pillGap());
        actions.addView(pill(c, R.drawable.ic_share, "Share", v -> share()), pillGap());
        LinearLayout save = pill(c, R.drawable.ic_save, "Save", v -> {
            if (current != null) {
                ItemMenu.saveToPlaylist(getContext(), current.id);
            }
        });
        // A long press saves straight to Watch later.
        save.setOnLongClickListener(v -> {
            if (current != null) {
                ItemMenu.watchLater(getContext(), current.id);
            }
            return true;
        });
        actions.addView(save, pillGap());
        actions.addView(pill(c, R.drawable.ic_download, "Download", v -> chooseDownload()), pillGap());
        audioButton = Ui.row(c);
        audioButton.setPadding(Ui.dp(12), Ui.dp(8), Ui.dp(14), Ui.dp(8));
        audioIcon = Ui.icon(c, R.drawable.ic_headset, Ui.TEXT);
        audioButton.addView(audioIcon, Ui.lin(Ui.dp(20), Ui.dp(20)));
        audioLabel = Ui.text(c, 13, Ui.TEXT, true);
        audioLabel.setText("Audio only");
        audioLabel.setPadding(Ui.dp(8), 0, 0, 0);
        audioButton.addView(audioLabel, Ui.lin(Ui.WRAP, Ui.WRAP));
        audioButton.setOnClickListener(v -> {
            boolean on = !player.audioOnly;
            host.setAudioOnly(on);
            showAudioOnly(on);
            updateMusic();
        });
        actions.addView(audioButton, pillGap());

        // Comments are a card under the description, as on YouTube; not a button here.
        chatButton = pill(c, R.drawable.ic_chat, "Live chat", v -> {
            if (chatToken != null) {
                WatchLayout.this.chat.open(chatToken);
                apply();
            }
        });
        chatButton.setVisibility(GONE);
        actions.addView(chatButton, pillGap());
        actions.addView(pill(c, R.drawable.ic_cast, "Play on TV", v -> {
            if (current != null) {
                int at = player.positionSeconds();
                player.command("pause");
                Tv.choose(getContext(), current.id, at);
            }
        }), pillGap());
        android.widget.HorizontalScrollView actionScroll = new android.widget.HorizontalScrollView(c);
        actionScroll.setHorizontalScrollBarEnabled(false);
        actionScroll.addView(actions);
        header.addView(actionScroll, Ui.lin(Ui.MATCH, Ui.WRAP));
        showAudioOnly(player.audioOnly);
        showRating();
        header.addView(Ui.divider(c));

        LinearLayout ch = Ui.row(c);
        ch.setPadding(Ui.dp(16), Ui.dp(10), Ui.dp(8), Ui.dp(10));
        ch.setForeground(Ui.ripple());
        channelAvatar = new ImageView(c);
        channelAvatar.setScaleType(ImageView.ScaleType.CENTER_CROP);
        channelAvatar.setBackgroundColor(Ui.PANEL);
        Ui.clipCircle(channelAvatar);
        ch.addView(channelAvatar, Ui.lin(Ui.dp(36), Ui.dp(36)));
        LinearLayout who = Ui.column(c);
        who.setPadding(Ui.dp(12), 0, 0, 0);
        channelName = Ui.lines(Ui.text(c, 14, Ui.TEXT, true), 1);
        who.addView(channelName);
        channelSubs = Ui.lines(Ui.text(c, 12, Ui.MUTE, false), 1);
        channelSubs.setPadding(0, Ui.dp(2), 0, 0);
        channelSubs.setVisibility(GONE);
        who.addView(channelSubs);
        ch.addView(who, Ui.weight(Ui.WRAP));
        ch.addView(Ui.icon(c, R.drawable.ic_chevron, Ui.MUTE), Ui.lin(Ui.dp(40), Ui.dp(40)));
        ch.setOnClickListener(v -> {
            if (current != null && current.channelId != null) {
                host.openChannel(current.channelId, current.channelName);
            }
        });
        channelRow = ch;
        header.addView(ch, Ui.lin(Ui.MATCH, Ui.WRAP));
        header.addView(Ui.divider(c));

        LinearLayout desc = Ui.column(c);
        desc.setPadding(Ui.dp(16), Ui.dp(12), Ui.dp(16), Ui.dp(4));
        desc.setForeground(Ui.ripple());
        description = Ui.text(c, 13, Ui.MUTE, false);
        description.setLineSpacing(Ui.dp(4), 1f);
        Ui.lines(description, 2);
        description.setLinkTextColor(Ui.ACCENT);
        description.setMovementMethod(android.text.method.LinkMovementMethod.getInstance());
        desc.addView(description, Ui.lin(Ui.MATCH, Ui.WRAP));
        descriptionToggle = Ui.text(c, 12, Ui.ACCENT, true);
        descriptionToggle.setPadding(0, Ui.dp(8), 0, Ui.dp(8));
        desc.addView(descriptionToggle, Ui.lin(Ui.WRAP, Ui.WRAP));
        desc.setOnClickListener(v -> {
            descriptionOpen = !descriptionOpen;
            description.setMaxLines(descriptionOpen ? Integer.MAX_VALUE : 2);
            descriptionToggle.setText(descriptionOpen ? "Show less" : "Show more");
        });
        header.addView(desc, Ui.lin(Ui.MATCH, Ui.WRAP));

        // Comments: a card with the top comment; tapping it opens them all.
        commentsCard = Ui.column(c);
        commentsCard.setPadding(Ui.dp(14), Ui.dp(12), Ui.dp(14), Ui.dp(12));
        commentsCard.setBackground(new android.graphics.drawable.RippleDrawable(
                android.content.res.ColorStateList.valueOf(Ui.RIPPLE), Ui.rounded(Ui.PANEL, 12), null));
        TextView commentsTitle = Ui.text(c, 14, Ui.TEXT, true);
        commentsTitle.setText("Comments");
        commentsCard.addView(commentsTitle);
        LinearLayout teaserRow = Ui.row(c);
        teaserRow.setPadding(0, Ui.dp(10), 0, 0);
        teaserAvatar = new ImageView(c);
        teaserAvatar.setScaleType(ImageView.ScaleType.CENTER_CROP);
        teaserAvatar.setBackgroundColor(Ui.BG);
        Ui.clipCircle(teaserAvatar);
        teaserRow.addView(teaserAvatar, Ui.lin(Ui.dp(26), Ui.dp(26)));
        teaserText = Ui.lines(Ui.text(c, 13, Ui.TEXT, false), 2);
        teaserText.setPadding(Ui.dp(10), 0, 0, 0);
        teaserText.setLineSpacing(Ui.dp(2), 1f);
        teaserRow.addView(teaserText, Ui.weight(Ui.WRAP));
        commentsCard.addView(teaserRow, Ui.lin(Ui.MATCH, Ui.WRAP));
        commentsCard.setContentDescription("Comments");
        commentsCard.setOnClickListener(v -> {
            if (current != null) {
                openComments();
            }
        });
        LinearLayout.LayoutParams ccl = Ui.lin(Ui.MATCH, Ui.WRAP);
        ccl.setMargins(Ui.dp(12), Ui.dp(8), Ui.dp(12), Ui.dp(6));
        header.addView(commentsCard, ccl);

        // Chapters: one card per chapter, tap to go there.
        LinearLayout chapters = Ui.column(c);
        TextView chaptersTitle = Ui.text(c, 14, Ui.TEXT, true);
        chaptersTitle.setText("Chapters");
        chaptersTitle.setPadding(Ui.dp(16), Ui.dp(10), Ui.dp(16), Ui.dp(6));
        chapters.addView(chaptersTitle);
        chapterRow = Ui.row(c);
        chapterRow.setPadding(Ui.dp(12), 0, Ui.dp(12), Ui.dp(8));
        android.widget.HorizontalScrollView chapterScroll = new android.widget.HorizontalScrollView(c);
        chapterScroll.setHorizontalScrollBarEnabled(false);
        chapterScroll.addView(chapterRow);
        chapters.addView(chapterScroll);
        chapters.setVisibility(GONE);
        chapterBox = chapters;
        header.addView(chapters, Ui.lin(Ui.MATCH, Ui.WRAP));

        TextView upNext = Ui.text(c, 14, Ui.TEXT, true);
        upNext.setText("Up next");
        upNext.setPadding(Ui.dp(16), Ui.dp(10), Ui.dp(16), Ui.dp(6));
        header.addView(upNext, Ui.lin(Ui.MATCH, Ui.WRAP));

        details = new FeedList(c, true, item -> {
            if (item.type == Core.Item.VIDEO) {
                show(item);
            }
        });
        details.setHeader(header);
        if (host instanceof Nav) {
            details.setMenu((Nav) host, false);
        }
        details.setEmptyText("");
        details.setSource(this::loadDetails);
        addView(details, new LayoutParams(Ui.MATCH, Ui.MATCH));

        // Comments and the live chat cover the details when opened.
        comments = new CommentsPanel(c);
        addView(comments, new LayoutParams(Ui.MATCH, Ui.MATCH));
        chat = new LiveChatPanel(c);
        addView(chat, new LayoutParams(Ui.MATCH, Ui.MATCH));

        // The mini bar: its left end is left free for the shrunken player.
        miniBar = Ui.row(c);
        miniBar.setBackgroundColor(Ui.PANEL);
        LinearLayout texts = Ui.column(c);
        texts.setGravity(Gravity.CENTER_VERTICAL);
        miniTitle = Ui.lines(Ui.text(c, 13, Ui.TEXT, true), 1);
        texts.addView(miniTitle, Ui.lin(Ui.MATCH, Ui.WRAP));
        miniChannel = Ui.lines(Ui.text(c, 12, Ui.MUTE, false), 1);
        miniChannel.setPadding(0, Ui.dp(4), 0, 0);
        texts.addView(miniChannel, Ui.lin(Ui.MATCH, Ui.WRAP));
        miniBar.addView(texts, Ui.weight(Ui.WRAP));
        miniPlay = Ui.iconButton(c, R.drawable.ic_pause, "Pause", v -> player.command("toggle"));
        miniBar.addView(miniPlay);
        miniBar.addView(Ui.iconButton(c, R.drawable.ic_close, "Close video", v -> close()));
        addView(miniBar, Ui.frame(Ui.MATCH, Ui.dp(MINI_HEIGHT_DP), Gravity.TOP));

        playerFrame = new FrameLayout(c);
        playerFrame.setBackgroundColor(Color.BLACK);
        playerFrame.addView(player.web, new LayoutParams(Ui.MATCH, Ui.MATCH));
        // The native player's picture; the player shows it or the web view.
        playerFrame.addView(player.nativeView(), new LayoutParams(Ui.MATCH, Ui.MATCH));
        // The app's own controls, over the direct player.
        controls = new PlayerControls(c, player, new PlayerControls.Host() {
            @Override
            public void toggleFullscreen() {
                WatchLayout.this.host.requestFullscreen(!fullscreen);
            }

            @Override
            public boolean step(int by) {
                return WatchLayout.this.step(by);
            }
        });
        player.progress = controls;
        controls.mirror = musicPanel;
        playerFrame.addView(controls, new LayoutParams(Ui.MATCH, Ui.MATCH));
        // Not clickable, so touches still reach the player underneath.
        loadingCover = new FrameLayout(c);
        loadingCover.setBackgroundColor(Color.BLACK);
        loadingImage = new ImageView(c);
        loadingImage.setScaleType(ImageView.ScaleType.CENTER_CROP);
        loadingCover.addView(loadingImage, new LayoutParams(Ui.MATCH, Ui.MATCH));
        View dim = new View(c);
        dim.setBackgroundColor(0x99000000);
        loadingCover.addView(dim, new LayoutParams(Ui.MATCH, Ui.MATCH));
        loadingCover.addView(Ui.spinner(c), Ui.frame(Ui.dp(40), Ui.dp(40), Gravity.CENTER));
        loadingCover.setVisibility(GONE);
        playerFrame.addView(loadingCover, new LayoutParams(Ui.MATCH, Ui.MATCH));

        // A short note that says which way the pinch went.
        zoomLabel = Ui.text(c, 13, 0xFFFFFFFF, true);
        zoomLabel.setBackground(Ui.rounded(0xCC000000, 16));
        zoomLabel.setPadding(Ui.dp(14), Ui.dp(7), Ui.dp(14), Ui.dp(7));
        zoomLabel.setAlpha(0f);
        LayoutParams zl = Ui.frame(Ui.WRAP, Ui.WRAP, Gravity.TOP | Gravity.CENTER_HORIZONTAL);
        zl.topMargin = Ui.dp(18);
        playerFrame.addView(zoomLabel, zl);

        pinch = new android.view.ScaleGestureDetector(c, new android.view.ScaleGestureDetector.SimpleOnScaleGestureListener() {
            @Override
            public boolean onScaleBegin(android.view.ScaleGestureDetector d) {
                pinchScale = 1f;
                return true;
            }

            @Override
            public boolean onScale(android.view.ScaleGestureDetector d) {
                pinchScale *= d.getScaleFactor();
                // Switch as soon as the fingers have clearly moved apart or together.
                if (pinchScale > 1.12f && !zoomToFill) {
                    setZoomToFill(true);
                } else if (pinchScale < 0.9f && zoomToFill) {
                    setZoomToFill(false);
                }
                return true;
            }
        });
        addView(playerFrame, Ui.frame(Ui.MATCH, 0, Gravity.TOP));
    }

    private static LinearLayout.LayoutParams pillGap() {
        LinearLayout.LayoutParams lp = Ui.lin(Ui.WRAP, Ui.WRAP);
        lp.leftMargin = Ui.dp(10);
        return lp;
    }

    /** A rounded button with an icon and a word. */
    private static LinearLayout pill(Context c, int icon, String label, OnClickListener onClick) {
        LinearLayout p = Ui.row(c);
        p.setPadding(Ui.dp(12), Ui.dp(8), Ui.dp(14), Ui.dp(8));
        p.addView(Ui.icon(c, icon, Ui.TEXT), Ui.lin(Ui.dp(20), Ui.dp(20)));
        TextView t = Ui.text(c, 13, Ui.TEXT, true);
        t.setText(label);
        t.setPadding(Ui.dp(8), 0, 0, 0);
        p.addView(t, Ui.lin(Ui.WRAP, Ui.WRAP));
        p.setContentDescription(label);
        p.setOnClickListener(onClick);
        paintPill(p, false);
        return p;
    }

    /** Filled with the accent colour when on. */
    private static void paintPill(LinearLayout p, boolean on) {
        p.setBackground(new android.graphics.drawable.RippleDrawable(
                android.content.res.ColorStateList.valueOf(Ui.RIPPLE), Ui.rounded(on ? Ui.ACCENT : Ui.PANEL, 18), null));
        int color = on ? Ui.ON_ACCENT : Ui.TEXT;
        ((ImageView) p.getChildAt(0)).setImageTintList(android.content.res.ColorStateList.valueOf(color));
        ((TextView) p.getChildAt(1)).setTextColor(color);
    }

    // ------------------------------------------------------------------ like, share, download

    /** The user's rating of the current video: "like", "dislike", "none", or null if unknown. */
    private String rating;

    private void showRating() {
        paintPill(likeButton, "like".equals(rating));
        paintPill(dislikeButton, "dislike".equals(rating));
        ((TextView) likeButton.getChildAt(1)).setText("like".equals(rating) ? "Liked" : "Like");
        ((TextView) dislikeButton.getChildAt(1)).setText("dislike".equals(rating) ? "Disliked" : "Dislike");
    }

    /** Ask how the user rated this video (signed in only). */
    private void loadRating(final String id) {
        rating = null;
        showRating();
        if (!Core.get().isSignedIn()) {
            return;
        }
        Core.get().call((e, err) -> {
            JSONObject ok = e == null ? null : e.optJSONObject("ok");
            if (ok != null && current != null && id.equals(current.id) && !ok.isNull("rating")) {
                rating = ok.optString("rating");
                showRating();
            }
        }, "rating", "id", id);
    }

    /** Press Like or Dislike: set it, or take it back if it was already set. */
    private void rate(String pressed) {
        if (current == null) {
            return;
        }
        if (!Core.get().isSignedIn()) {
            android.widget.Toast.makeText(getContext(), "Sign in to like or dislike videos.", android.widget.Toast.LENGTH_SHORT).show();
            return;
        }
        final String id = current.id;
        final String before = rating;
        final String wanted = pressed.equals(rating) ? "none" : pressed;
        rating = wanted;
        showRating();
        Core.get().call((e, err) -> {
            if (err != null && current != null && id.equals(current.id)) {
                rating = before;
                showRating();
                android.widget.Toast.makeText(getContext(), "YouTube did not take that: " + err, android.widget.Toast.LENGTH_LONG).show();
            }
        }, "rate", "id", id, "rating", wanted);
    }

    private void share() {
        final Core.Item v = current;
        if (v == null) {
            return;
        }
        final int at = player.positionSeconds();
        if (at < 5) {
            ItemMenu.share(getContext(), v, 0);
            return;
        }
        Sheet s = new Sheet(getContext(), "Share");
        s.add(R.drawable.ic_share, "Share video", () -> ItemMenu.share(getContext(), v, 0));
        s.add(R.drawable.ic_watch_later, "Share from " + Ui.duration(at), "The link starts the video at this point", () -> ItemMenu.share(getContext(), v, at));
        s.show();
    }

    // ------------------------------------------------------------------ chapters, extras, ambient

    private static final java.util.regex.Pattern CHAPTER_LINE = java.util.regex.Pattern.compile(
            "^\\s*[\\[(]?((?:\\d{1,2}:)?\\d{1,2}:\\d{2})[\\])]?\\s*[-–—:|]?\\s*(.+?)\\s*$");

    /** Chapters from the page, or else from timestamped lines in the description (as YouTube makes them). */
    private void bindChapters(JSONObject page, String description) {
        java.util.List<double[]> starts = new java.util.ArrayList<>();
        java.util.List<String> titles = new java.util.ArrayList<>();
        org.json.JSONArray list = page == null ? null : page.optJSONArray("chapters");
        for (int i = 0; list != null && i < list.length(); i++) {
            JSONObject ch = list.optJSONObject(i);
            if (ch != null) {
                starts.add(new double[] {ch.optDouble("start")});
                titles.add(ch.optString("title"));
            }
        }
        if (starts.isEmpty() && description != null) {
            for (String line : description.split("\n")) {
                java.util.regex.Matcher m = CHAPTER_LINE.matcher(line);
                if (m.matches()) {
                    int s = 0;
                    for (String part : m.group(1).split(":")) {
                        s = s * 60 + Integer.parseInt(part);
                    }
                    if (starts.isEmpty() ? s == 0 : s > starts.get(starts.size() - 1)[0]) {
                        starts.add(new double[] {s});
                        titles.add(m.group(2));
                    }
                }
            }
            // YouTube needs at least three, starting at 0:00.
            if (starts.size() < 3) {
                starts.clear();
                titles.clear();
            }
        }
        double[] at = new double[starts.size()];
        String[] names = titles.toArray(new String[0]);
        for (int i = 0; i < at.length; i++) {
            at[i] = starts.get(i)[0];
        }
        controls.setChapters(at, names);
        chapterRow.removeAllViews();
        Context c = getContext();
        for (int i = 0; i < at.length; i++) {
            final double seconds = at[i];
            LinearLayout card = Ui.column(c);
            card.setPadding(Ui.dp(12), Ui.dp(10), Ui.dp(12), Ui.dp(10));
            card.setBackground(new android.graphics.drawable.RippleDrawable(
                    android.content.res.ColorStateList.valueOf(Ui.RIPPLE), Ui.rounded(Ui.PANEL, 10), null));
            TextView t = Ui.lines(Ui.text(c, 13, Ui.TEXT, true), 2);
            t.setText(names[i]);
            card.addView(t, Ui.lin(Ui.dp(150), Ui.WRAP));
            TextView when = Ui.text(c, 12, Ui.ACCENT, true);
            when.setText(Ui.duration((int) seconds));
            when.setPadding(0, Ui.dp(6), 0, 0);
            card.addView(when);
            card.setOnClickListener(v -> player.command("seek", seconds));
            LinearLayout.LayoutParams lp = Ui.lin(Ui.WRAP, Ui.WRAP);
            lp.rightMargin = Ui.dp(8);
            chapterRow.addView(card, lp);
        }
        chapterBox.setVisibility(at.length > 0 ? VISIBLE : GONE);
    }

    /** Fill the comments card with the video's top comment. */
    private void loadCommentTeaser(final String id) {
        if (current != null && current.localUri != null) {
            // Downloaded videos may be watched offline; comments load when opened.
            teaserText.setText("Tap to see comments");
            teaserAvatar.setVisibility(GONE);
            return;
        }
        teaserText.setText("Loading comments…");
        teaserText.setTextColor(Ui.MUTE);
        teaserAvatar.setVisibility(GONE);
        comments.preload(id, (first, more, err) -> {
            if (current == null || !id.equals(current.id)) {
                return;
            }
            if (first == null) {
                teaserText.setText(err != null ? "Comments did not load. Tap to try again." : "No comments yet, or they are turned off.");
                return;
            }
            teaserText.setTextColor(Ui.TEXT);
            teaserText.setText(first.optString("text"));
            teaserAvatar.setVisibility(VISIBLE);
            Images.load(teaserAvatar, Core.url(Core.str(first, "avatar")));
        });
    }

    /** Captions, preview pictures and chat of the video now playing. */
    private void loadExtras(final String id) {
        chatToken = null;
        chatButton.setVisibility(GONE);
        chat.reset();
        controls.setStoryboard(null);
        Core.get().call((e, err) -> {
            JSONObject ok = e == null ? null : e.optJSONObject("ok");
            if (ok == null || current == null || !id.equals(current.id)) {
                return;
            }
            player.setExtras(id, ok);
            controls.setStoryboard(ok.isNull("storyboard") ? null : ok.optString("storyboard", null));
            chatToken = ok.isNull("chat") ? null : ok.optString("chat", null);
            chatButton.setVisibility(chatToken != null ? VISIBLE : GONE);
        }, "extras", "id", id);
    }

    /** Ambient mode on (the default)? */
    private boolean ambientOn() {
        return getContext().getSharedPreferences("shell", Context.MODE_PRIVATE).getBoolean("ambient", true);
    }

    /** Every few seconds, tint the glow with the picture's colour. */
    private void ambient() {
        removeCallbacks(ambientTick);
        if (!open || !ambientOn() || pip || fullscreen || music) {
            glow.animate().alpha(0f).setDuration(Ui.MEDIUM).start();
            if (open) {
                postDelayed(ambientTick, 4000);
            }
            return;
        }
        int color = player.frameColor();
        if (color != 0) {
            // Soften: keep the hue, lower the strength so text stays readable.
            int target = Color.argb(Ui.light ? 70 : 110, Color.red(color), Color.green(color), Color.blue(color));
            if (target != glowColor) {
                glowColor = target;
                android.graphics.drawable.GradientDrawable g = new android.graphics.drawable.GradientDrawable(
                        android.graphics.drawable.GradientDrawable.Orientation.TOP_BOTTOM, new int[] {target, 0x00000000});
                glow.setBackground(g);
            }
            glow.animate().alpha(1f).setDuration(Ui.MEDIUM).start();
        }
        postDelayed(ambientTick, 2500);
    }

    /** The queue changed: show it at the top of Up next. */
    void refreshUpNext() {
        if (open && current != null) {
            details.load();
        }
    }

    /** Ask what to download: sound only, or the picture up to a chosen quality. */
    private void chooseDownload() {
        final Core.Item video = current;
        if (video == null) {
            return;
        }
        Downloads.get().choose(getContext(), video);
    }

    private static final java.util.regex.Pattern LINK = java.util.regex.Pattern.compile(
            "(https?://[^\\s<>\"]+[^\\s<>\".,;:!?)\\]])|(?<![\\d:])((?:\\d{1,2}:)?\\d{1,2}:\\d{2})(?![\\d:])");
    private static final java.util.regex.Pattern YOUTUBE_LINK = java.util.regex.Pattern.compile(
            "^https?://(?:www\\.|m\\.|music\\.)?(?:youtube\\.com/(?:watch\\?|shorts/|live/|embed/)|youtu\\.be/)");

    /** The description with its web links and timestamps made tappable. */
    private CharSequence linked(String text) {
        android.text.SpannableString out = new android.text.SpannableString(text);
        java.util.regex.Matcher m = LINK.matcher(text);
        while (m.find()) {
            final String url = m.group(1);
            final String time = m.group(2);
            out.setSpan(new android.text.style.ClickableSpan() {
                @Override
                public void onClick(View widget) {
                    if (url != null) {
                        openLink(url);
                    } else {
                        int seconds = 0;
                        for (String part : time.split(":")) {
                            seconds = seconds * 60 + Integer.parseInt(part);
                        }
                        player.command("seek", seconds);
                    }
                }

                @Override
                public void updateDrawState(android.text.TextPaint paint) {
                    paint.setColor(Ui.ACCENT);
                    paint.setUnderlineText(false);
                }
            }, m.start(), m.end(), android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE);
        }
        return out;
    }

    /** YouTube video links play here; everything else goes to the browser. */
    private void openLink(String url) {
        android.content.Intent view = new android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(url));
        if (YOUTUBE_LINK.matcher(url).find()) {
            view.setPackage(getContext().getPackageName());
        }
        try {
            getContext().startActivity(view);
        } catch (Exception e) {
            android.widget.Toast.makeText(getContext(), "No app can open that link.", android.widget.Toast.LENGTH_SHORT).show();
        }
    }

    /**
     * The next (+1) or previous (-1) video: along the play queue when there is one, otherwise
     * the first video under "Up next". False if there is nowhere to go.
     */
    boolean step(int by) {
        if (current == null) {
            return false;
        }
        Core.Item to = Downloads.get().step(current.id, by);
        if (to == null && by > 0) {
            to = nextUp;
        }
        if (to == null) {
            return false;
        }
        show(to);
        return true;
    }

    /** Draw the audio-only pill as on (filled, accent) or off (outlined). */
    private void showAudioOnly(boolean on) {
        audioButton.setBackground(new android.graphics.drawable.RippleDrawable(
                android.content.res.ColorStateList.valueOf(Ui.RIPPLE), Ui.rounded(on ? Ui.ACCENT : Ui.PANEL, 18), null));
        int color = on ? Ui.ON_ACCENT : Ui.TEXT;
        audioIcon.setImageTintList(android.content.res.ColorStateList.valueOf(color));
        audioLabel.setTextColor(color);
        audioButton.setContentDescription(on ? "Audio only, on" : "Audio only, off");
    }

    /** Cover the player with the video's thumbnail and a spinner until it starts playing. */
    private void showLoading(Core.Item video) {
        removeCallbacks(hideLoadingLater);
        loadingCover.animate().cancel();
        Images.load(loadingImage, video == null ? null : video.thumb);
        loadingCover.setAlpha(1f);
        loadingCover.setVisibility(VISIBLE);
        // Never leave the cover up for good: an error page or a video that needs a tap
        // must stay reachable.
        postDelayed(hideLoadingLater, 12000);
    }

    private void hideLoading() {
        removeCallbacks(hideLoadingLater);
        if (loadingCover.getVisibility() != VISIBLE) {
            return;
        }
        loadingCover.animate().alpha(0f).setDuration(Ui.FAST).withEndAction(() -> loadingCover.setVisibility(GONE)).start();
    }

    private void openComments() {
        comments.open(current.id);
        apply();
    }

    /** Close the comments or the live chat if open. Returns whether one was. */
    boolean closeComments() {
        if (chat.isShowing()) {
            chat.close();
            return true;
        }
        if (!comments.isShowing()) {
            return false;
        }
        comments.close();
        return true;
    }

    private TextView zoomLabelView() {
        return zoomLabel;
    }

    private void setZoomToFill(boolean on) {
        zoomToFill = on;
        pinchScale = 1f;
        player.setFill(on);
        zoomLabel.setText(on ? "Zoomed to fill" : "Original");
        zoomLabel.animate().cancel();
        zoomLabel.setAlpha(1f);
        removeCallbacks(hideZoomLabel);
        postDelayed(hideZoomLabel, 900);
    }

    // ------------------------------------------------------------------ state

    boolean isOpen() {
        return open;
    }

    boolean isExpanded() {
        return open && progress < 0.5f;
    }

    /** Height of the bottom navigation: the mini bar sits right above it. */
    void setBottomInset(int px) {
        bottomInset = px;
        apply();
    }

    /** Picture-in-picture: only the player is shown, filling the small window. */
    void setPip(boolean on) {
        pip = on;
        player.shapeChanging();
        if (on) {
            cancelAnimation();
            progress = 0f;
        }
        details.setVisibility(on || fullscreen ? GONE : VISIBLE);
        requestLayout();
        post(this::apply);
    }

    /** Fullscreen: the player fills the (rotated) screen; nothing else is shown. */
    void setFullscreen(boolean on) {
        if (fullscreen == on) {
            return;
        }
        fullscreen = on;
        player.shapeChanging();
        controls.setFullscreen(on);
        // Filling the screen only makes sense in fullscreen; the choice is kept for next time.
        player.setFill(on && zoomToFill);
        pinching = false;
        if (on) {
            cancelAnimation();
            progress = 0f;
        }
        details.setVisibility(on || pip ? GONE : VISIBLE);
        requestLayout();
        post(this::apply);
    }

    boolean isFullscreen() {
        return fullscreen;
    }

    /** Play `video` and expand. */
    void show(Core.Item video) {
        // A video that is already downloaded plays from its file, wherever it was tapped.
        Downloads.get().useFile(video);
        boolean same = current != null && current.id.equals(video.id) && open
                && java.util.Objects.equals(current.localUri, video.localUri);
        current = video;
        // The mode carries over between videos; the button must say so.
        showAudioOnly(player.audioOnly);
        bindHeader(null);
        details.load();
        if (!same) {
            loadRating(video.id);
            comments.reset();
            showLoading(video);
            post(this::ambient);
            if (video.localUri != null) {
                player.openLocal(video.id, android.net.Uri.parse(video.localUri), video.localMime, video.localAudio);
            } else {
                player.open(video.id);
            }
        }
        miniTitle.setText(video.title);
        miniChannel.setText(video.channelName == null ? "" : video.channelName);
        post(this::updateMusic);
        if (!same) {
            // After the player has taken the new video (it drops the old one's extras).
            final String id = video.id;
            post(() -> {
                loadExtras(id);
                loadCommentTeaser(id);
            });
        }
        if (!open) {
            open = true;
            setVisibility(VISIBLE);
            setAlpha(1f);
            progress = 1f;
            // Wait for the first layout so the motion starts from the right place.
            post(() -> {
                apply();
                animateTo(0f, 0);
            });
        } else {
            animateTo(0f, 0);
        }
        host.onNowPlaying();
    }

    void expand() {
        animateTo(0f, 0);
    }

    void minimize() {
        animateTo(1f, 0);
    }

    void close() {
        if (!open) {
            return;
        }
        open = false;
        cancelAnimation();
        player.close();
        chat.reset();
        removeCallbacks(ambientTick);
        current = null;
        nextUp = null;
        detailsGeneration++;
        host.onWatchProgress(1f, false);
        host.onNowPlaying();
        animate().alpha(0f).setDuration(Ui.FAST).withEndAction(() -> {
            setVisibility(GONE);
            playerFrame.setTranslationX(0);
            miniBar.setTranslationX(0);
        }).start();
    }

    // ------------------------------------------------------------------ player events

    /** The page moved to another video by itself. */
    void onPlayerVideo(String id) {
        if (!open || (current != null && current.id.equals(id))) {
            return;
        }
        Core.Item i = new Core.Item();
        i.type = Core.Item.VIDEO;
        i.id = id;
        current = i;
        comments.reset();
        loadCommentTeaser(id);
        showLoading(null);
        bindHeader(null);
        details.load();
        miniTitle.setText("");
        miniChannel.setText("");
        host.onNowPlaying();
    }

    void onPlayerEnded(String id) {
        if (!open || current == null || !current.id.equals(id)) {
            return;
        }
        int repeat = Downloads.get().repeat;
        if (repeat == 2) {
            // Repeat this one.
            player.command("play");
            return;
        }
        Core.Item queued = Downloads.get().step(id, 1);
        if (queued == null && repeat == 1) {
            queued = Downloads.get().restart(id);
            if (queued == null || queued.id.equals(id)) {
                // Nothing else to go round to: the same one again.
                player.command("play");
                return;
            }
        }
        if (queued != null) {
            float keep = progress;
            show(queued);
            if (keep > 0.5f) {
                minimize();
            }
            return;
        }
        if (nextUp != null && Core.get().autoplay()) {
            Core.Item n = nextUp;
            float keep = progress;
            show(n);
            if (keep > 0.5f) {
                minimize();
            }
        }
    }

    void onPlayerPlaying(boolean playing) {
        if (playing) {
            hideLoading();
        }
        miniPlay.setImageResource(playing ? R.drawable.ic_pause : R.drawable.ic_play);
        miniPlay.setContentDescription(playing ? "Pause" : "Play");
        host.onNowPlaying();
    }

    // ------------------------------------------------------------------ details

    private void bindHeader(JSONObject page) {
        Core.Item v = current;
        if (v == null) {
            return;
        }
        title.setText(v.title);
        String likes = page != null && !page.isNull("likes") ? Ui.count(page.optLong("likes")) + " likes" : null;
        String m = Ui.join(v.views >= 0 ? Ui.count(v.views) + " views" : null, v.published, likes);
        meta.setText(m);
        meta.setVisibility(m.isEmpty() ? GONE : VISIBLE);
        channelName.setText(v.channelName == null ? "" : v.channelName);
        channelRow.setVisibility(v.channelName == null ? GONE : VISIBLE);
        Images.load(channelAvatar, v.channelAvatar);
        String subs = page == null || page.isNull("subscribers") ? null : page.optString("subscribers", null);
        channelSubs.setText(subs == null ? "" : subs);
        channelSubs.setVisibility(subs == null || subs.isEmpty() ? GONE : VISIBLE);
        String d = page == null ? "" : page.optString("description");
        description.setText(linked(d));
        bindChapters(page, d);
        descriptionOpen = false;
        description.setMaxLines(2);
        descriptionToggle.setText("Show more");
        ((View) description.getParent()).setVisibility(d.isEmpty() ? GONE : VISIBLE);
    }

    private void loadDetails(final FeedList.Result result) {
        final Core.Item v = current;
        if (v == null) {
            result.page(new Core.Page(), null);
            return;
        }
        final int mine = ++detailsGeneration;
        // Under a downloaded video the list is the other downloads, so it also works offline.
        final boolean local = v.localUri != null;
        Core.get().call((e, err) -> {
            if (mine != detailsGeneration || current != v) {
                return;
            }
            if (err != null || e == null) {
                if (local) {
                    Core.Page others = Downloads.get().upNext(v.id);
                    nextUp = others.items.isEmpty() ? null : others.items.get(0);
                    result.page(others, null);
                } else {
                    result.page(null, err == null ? "No answer." : err);
                }
                return;
            }
            JSONObject page = e.optJSONObject("ok");
            if (page == null) {
                result.page(new Core.Page(), null);
                return;
            }
            JSONObject video = page.optJSONObject("video");
            if (video != null) {
                Core.Item full = Core.Item.video(video);
                // Keep what the list already knew when the details lack it.
                if (full.title.isEmpty()) {
                    full.title = v.title;
                }
                if (full.thumb == null) {
                    full.thumb = v.thumb;
                }
                if (full.published == null && full.views < 0) {
                    full.published = v.published;
                    full.views = v.views;
                }
                if (full.channelAvatar == null) {
                    full.channelAvatar = v.channelAvatar;
                }
                full.id = v.id;
                full.localUri = v.localUri;
                full.localMime = v.localMime;
                full.localAudio = v.localAudio;
                current = full;
                miniTitle.setText(full.title);
                miniChannel.setText(full.channelName == null ? "" : full.channelName);
                host.onNowPlaying();
            }
            bindHeader(page);
            Core.Page related = local ? Downloads.get().upNext(v.id) : Core.Page.parse(page.optJSONObject("related"));
            if (!local) {
                // What the user queued plays first and is listed first.
                java.util.List<Core.Item> queued = Downloads.get().queuedAfter(v.id);
                if (!queued.isEmpty()) {
                    related.items.addAll(0, queued);
                }
            }
            if (!local && Core.get().prefetch) {
                // The first videos under Up next are the likely next ones (autoplay plays the
                // first): get them ready now, even while they are below the details.
                org.json.JSONArray ids = new org.json.JSONArray();
                for (Core.Item i : related.items) {
                    if (i.type == Core.Item.VIDEO && !i.live && ids.length() < 2) {
                        ids.put(i.id);
                    }
                }
                if (ids.length() > 0) {
                    // After this video's own start, so they do not compete with it.
                    postDelayed(() -> Core.get().send("prefetch_streams", "ids", ids), 2500);
                }
            }
            nextUp = null;
            for (Core.Item i : related.items) {
                if (i.type == Core.Item.VIDEO) {
                    nextUp = i;
                    break;
                }
            }
            result.page(related, null);
        }, "video", "id", v.id);
    }

    // ------------------------------------------------------------------ layout and motion

    private float miniHeight() {
        return Ui.dp(MINI_HEIGHT_DP);
    }

    private float miniWidth() {
        return miniHeight() * 16f / 9f;
    }

    private float miniTop() {
        return getHeight() - bottomInset - miniHeight();
    }

    private int playerHeight(int width, int height) {
        // In picture-in-picture the window is the player.
        if (pip || fullscreen) {
            return height;
        }
        // Music: a square for the artwork, as far as the screen allows.
        return music ? Math.min(width, Math.round(height * 0.46f)) : Math.min(Math.round(width * 9f / 16f), Math.round(height * 0.6f));
    }

    @Override
    protected void onMeasure(int w, int h) {
        int width = MeasureSpec.getSize(w), height = MeasureSpec.getSize(h);
        int ph = playerHeight(width, height);
        LayoutParams pl = (LayoutParams) playerFrame.getLayoutParams();
        LayoutParams dl = (LayoutParams) details.getLayoutParams();
        pl.height = ph;
        dl.topMargin = ph;
        ((LayoutParams) comments.getLayoutParams()).topMargin = ph;
        ((LayoutParams) chat.getLayoutParams()).topMargin = ph;
        ((LayoutParams) glow.getLayoutParams()).topMargin = ph;
        miniBar.setPadding(Math.round(miniWidth()) + Ui.dp(12), 0, 0, 0);
        super.onMeasure(w, h);
    }

    @Override
    protected void onLayout(boolean changed, int l, int t, int r, int b) {
        super.onLayout(changed, l, t, r, b);
        apply();
    }

    private static float clamp(float v) {
        return Math.max(0f, Math.min(1f, v));
    }

    /** Position everything for the current progress. */
    private void apply() {
        int w = getWidth(), h = getHeight();
        if (w == 0 || h == 0) {
            return;
        }
        float p = pip || fullscreen ? 0f : progress;
        float ph = playerFrame.getHeight();
        float scale = 1f + (miniWidth() / w - 1f) * p;
        // The player keeps its 16:9 shape; in mini it is exactly as tall as the bar.
        float scaleY = ph <= 0 ? scale : 1f + (miniHeight() / ph - 1f) * p;
        float y = p * miniTop();
        playerFrame.setPivotX(0);
        playerFrame.setPivotY(0);
        playerFrame.setScaleX(scale);
        playerFrame.setScaleY(scaleY);
        playerFrame.setTranslationY(y);

        details.setTranslationY(y + ph * scaleY - ph);
        details.setAlpha(clamp(1f - p * 2.4f));
        glow.setTranslationY(details.getTranslationY());
        glow.setVisibility(pip || fullscreen || p >= 0.42f ? INVISIBLE : VISIBLE);
        if (chat.isShowing()) {
            if (pip || fullscreen || p >= 0.42f) {
                chat.setVisibility(INVISIBLE);
            } else {
                chat.setVisibility(VISIBLE);
                chat.setTranslationY(details.getTranslationY());
            }
        }
        if (!pip && !fullscreen) {
            details.setVisibility(p >= 0.42f ? INVISIBLE : VISIBLE);
        }
        // The comments move and fade with the details they cover.
        if (comments.isShowing()) {
            if (pip || fullscreen || p >= 0.42f) {
                comments.setVisibility(INVISIBLE);
            } else {
                comments.setVisibility(VISIBLE);
                comments.setTranslationY(details.getTranslationY());
                if (p > 0.001f) {
                    comments.setAlpha(details.getAlpha());
                }
            }
        }

        scrim.setAlpha(clamp(1f - p * 1.15f));
        scrim.setVisibility(p >= 0.999f ? GONE : VISIBLE);

        miniBar.setTranslationY(miniTop());
        miniBar.setAlpha(clamp((p - 0.5f) / 0.5f));
        miniBar.setVisibility(p <= 0.5f ? INVISIBLE : VISIBLE);

        player.setCompact(pip || p > 0.5f);
        controls.setCompact(pip || p > 0.5f);
        if (open) {
            host.onWatchProgress(p, true);
        }
    }

    private void setProgress(float p) {
        progress = clamp(p);
        apply();
    }

    private void cancelAnimation() {
        if (animator != null) {
            animator.cancel();
            animator = null;
        }
    }

    /** Glide to expanded (0) or mini (1). A fling passes its speed so the glide matches it. */
    private void animateTo(float target, float velocityPx) {
        cancelAnimation();
        float distance = Math.abs(target - progress);
        if (distance < 0.001f) {
            setProgress(target);
            return;
        }
        long duration = Ui.MEDIUM;
        if (Math.abs(velocityPx) > 1 && getHeight() > 0) {
            duration = Math.max(120, Math.min(Ui.MEDIUM, Math.round(1000f * distance * miniTop() / Math.abs(velocityPx) * 1.6f)));
        }
        animator = ValueAnimator.ofFloat(progress, target);
        animator.setDuration(duration);
        animator.setInterpolator(Ui.EASE_OUT);
        animator.addUpdateListener(a -> setProgress((float) a.getAnimatedValue()));
        animator.addListener(new AnimatorListenerAdapter() {
            @Override
            public void onAnimationEnd(Animator a) {
                animator = null;
            }
        });
        animator.start();
    }

    // ------------------------------------------------------------------ touch

    /** Is the point on the player (expanded) or on the mini bar outside its buttons? */
    private boolean onHandle(float x, float y) {
        if (progress > 0.5f) {
            return y >= miniTop() && y <= miniTop() + miniHeight() && x < getWidth() - Ui.dp(100);
        }
        return y <= playerFrame.getHeight();
    }

    /**
     * The web view asks its parents to stop watching a touch as soon as YouTube's page
     * handles it, which with a real finger happens before a drag is recognised here. The
     * request is ignored so dragging the player (down to shrink, up for fullscreen) keeps
     * working; taps and sideways moves still go to the page.
     */
    @Override
    public void requestDisallowInterceptTouchEvent(boolean disallow) {
        // Deliberately not passed on.
    }

    @Override
    public boolean onInterceptTouchEvent(MotionEvent e) {
        if (!open || pip || controls.seeking()) {
            return false;
        }
        if (fullscreen) {
            // Two fingers on the fullscreen player are a pinch, which belongs to this layout.
            pinch.onTouchEvent(e);
            if (e.getPointerCount() >= 2) {
                pinching = true;
                tracking = false;
                return true;
            }
        }
        switch (e.getActionMasked()) {
            case MotionEvent.ACTION_DOWN:
                downX = e.getX();
                downY = e.getY();
                startProgress = progress;
                dragging = false;
                horizontal = false;
                drag = DRAG_NONE;
                // In fullscreen the sides belong to the brightness and volume swipes.
                tracking = fullscreen ? !controls.ownsSwipeAt(downX) : onHandle(downX, downY);
                if (tracking) {
                    cancelAnimation();
                    if (velocity == null) {
                        velocity = VelocityTracker.obtain();
                    }
                    velocity.clear();
                    velocity.addMovement(e);
                }
                // In mini the whole bar belongs to this layout (tap to expand, drag, swipe).
                return tracking && !fullscreen && progress > 0.5f;
            case MotionEvent.ACTION_MOVE:
                if (!tracking) {
                    return false;
                }
                velocity.addMovement(e);
                float dy = e.getY() - downY, dx = e.getX() - downX;
                // Expanded: a clear downward drag on the player starts minimising. Taps and
                // sideways moves stay with YouTube's own controls.
                boolean vertical = Math.abs(dy) > slop && Math.abs(dy) > Math.abs(dx) * 1.3f;
                if (!vertical) {
                    return false;
                }
                if (fullscreen) {
                    // Fullscreen: only a downward swipe is ours (leave fullscreen).
                    if (dy < 0) {
                        return false;
                    }
                    drag = DRAG_FROM_FULLSCREEN;
                } else {
                    // Expanded: down shrinks to the mini bar, up goes fullscreen.
                    drag = dy > 0 ? DRAG_SHRINK : DRAG_TO_FULLSCREEN;
                }
                dragging = true;
                return true;
            default:
                return false;
        }
    }

    @Override
    public boolean onTouchEvent(MotionEvent e) {
        if (pinching) {
            pinch.onTouchEvent(e);
            int action = e.getActionMasked();
            if (action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_CANCEL) {
                pinching = false;
            }
            return true;
        }
        if (!tracking) {
            return false;
        }
        velocity.addMovement(e);
        float dy = e.getY() - downY, dx = e.getX() - downX;
        switch (e.getActionMasked()) {
            case MotionEvent.ACTION_DOWN:
                return true;
            case MotionEvent.ACTION_MOVE:
                if (!dragging && (Math.abs(dy) > slop || Math.abs(dx) > slop)) {
                    dragging = true;
                    horizontal = startProgress > 0.5f && Math.abs(dx) > Math.abs(dy);
                }
                if (dragging && (drag == DRAG_TO_FULLSCREEN || drag == DRAG_FROM_FULLSCREEN)) {
                    // A hint that the swipe is recognised: the player follows a little.
                    float hint = Math.max(-1f, Math.min(1f, dy / Ui.dp(160)));
                    playerFrame.setScaleX(1f + (drag == DRAG_TO_FULLSCREEN ? -hint : -hint) * 0.06f);
                    playerFrame.setScaleY(playerFrame.getScaleX());
                    playerFrame.setPivotX(getWidth() / 2f);
                    playerFrame.setPivotY(playerFrame.getHeight() / 2f);
                } else if (dragging) {
                    if (horizontal) {
                        playerFrame.setTranslationX(dx);
                        miniBar.setTranslationX(dx);
                        float fade = 1f - Math.min(1f, Math.abs(dx) / (getWidth() * 0.6f));
                        playerFrame.setAlpha(fade);
                        miniBar.setAlpha(fade);
                    } else if (miniTop() > 0) {
                        setProgress(startProgress + dy / miniTop());
                    }
                }
                return true;
            case MotionEvent.ACTION_UP:
            case MotionEvent.ACTION_CANCEL: {
                velocity.computeCurrentVelocity(1000);
                float vx = velocity.getXVelocity(), vy = velocity.getYVelocity();
                tracking = false;
                boolean cancelled = e.getActionMasked() == MotionEvent.ACTION_CANCEL;
                if (drag == DRAG_TO_FULLSCREEN || drag == DRAG_FROM_FULLSCREEN) {
                    boolean far = Math.abs(dy) > Ui.dp(48) || Math.abs(vy) > flingVelocity;
                    int kind = drag;
                    drag = DRAG_NONE;
                    dragging = false;
                    apply();
                    if (!cancelled && far) {
                        host.requestFullscreen(kind == DRAG_TO_FULLSCREEN);
                    }
                    return true;
                }
                if (!dragging) {
                    if (!cancelled && startProgress > 0.5f) {
                        expand();
                    }
                } else if (horizontal) {
                    boolean dismiss = !cancelled && (Math.abs(dx) > getWidth() / 3f || Math.abs(vx) > flingVelocity);
                    if (dismiss) {
                        float to = Math.signum(dx == 0 ? vx : dx) * getWidth();
                        playerFrame.animate().translationX(to).alpha(0f).setDuration(Ui.FAST).start();
                        miniBar.animate().translationX(to).alpha(0f).setDuration(Ui.FAST).withEndAction(() -> {
                            close();
                            playerFrame.setAlpha(1f);
                        }).start();
                    } else {
                        playerFrame.animate().translationX(0).alpha(1f).setDuration(Ui.FAST).start();
                        miniBar.animate().translationX(0).alpha(1f).setDuration(Ui.FAST).start();
                    }
                } else if (Math.abs(vy) > flingVelocity) {
                    animateTo(vy > 0 ? 1f : 0f, vy);
                } else {
                    animateTo(progress > 0.5f ? 1f : 0f, 0);
                }
                dragging = false;
                return true;
            }
            default:
                return true;
        }
    }
}
