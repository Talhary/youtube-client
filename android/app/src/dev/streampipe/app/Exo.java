package dev.streampipe.app;

import android.content.Context;
import android.graphics.Color;
import android.net.Uri;
import android.os.Handler;
import android.os.Looper;
import android.view.TextureView;
import android.view.View;
import android.widget.FrameLayout;
import android.widget.ImageView;

import androidx.media3.common.AudioAttributes;
import androidx.media3.common.C;
import androidx.media3.common.MediaItem;
import androidx.media3.common.PlaybackException;
import androidx.media3.common.PlaybackParameters;
import androidx.media3.common.Player;
import androidx.media3.common.VideoSize;
import androidx.media3.datasource.DataSource;
import androidx.media3.datasource.DefaultDataSource;
import androidx.media3.datasource.DefaultHttpDataSource;
import androidx.media3.exoplayer.ExoPlayer;
import androidx.media3.exoplayer.source.MediaSource;
import androidx.media3.exoplayer.source.MergingMediaSource;
import androidx.media3.exoplayer.source.ProgressiveMediaSource;
import androidx.media3.exoplayer.upstream.DefaultLoadErrorHandlingPolicy;

/**
 * The native player: Android's media decoders driven by ExoPlayer (Media3), drawing straight
 * to a view. It plays a video's separate picture and sound streams together, or a downloaded
 * file. No web view is involved.
 */
final class Exo {
    interface Events {
        void onPlaying(boolean playing);

        void onEnded();

        /** Seconds played, total and buffered; `waiting` while it has to load before going on. */
        void onProgress(double at, double total, double buffered, boolean paused, boolean waiting);

        void onError(String why);
    }

    /** The picture, sized to the video's shape inside whatever space the view has. */
    private static final class Screen extends FrameLayout {
        final TextureView picture;
        final ImageView cover;
        int videoWidth, videoHeight;
        boolean fill;

        Screen(Context c) {
            super(c);
            setBackgroundColor(Color.BLACK);
            picture = new TextureView(c);
            addView(picture, new LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT));
            cover = new ImageView(c);
            cover.setScaleType(ImageView.ScaleType.FIT_CENTER);
            cover.setBackgroundColor(Color.BLACK);
            cover.setVisibility(GONE);
            addView(cover, new LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.MATCH_PARENT));
        }

        @Override
        protected void onLayout(boolean changed, int left, int top, int right, int bottom) {
            super.onLayout(changed, left, top, right, bottom);
            int w = right - left, h = bottom - top;
            if (videoWidth <= 0 || videoHeight <= 0 || w <= 0 || h <= 0) {
                return;
            }
            // Fit: the whole picture is visible. Fill: it covers the view, cropped at the edges.
            float fit = Math.min(w / (float) videoWidth, h / (float) videoHeight);
            float cover = Math.max(w / (float) videoWidth, h / (float) videoHeight);
            float scale = fill ? cover : fit;
            int pw = Math.round(videoWidth * scale), ph = Math.round(videoHeight * scale);
            int x = (w - pw) / 2, y = (h - ph) / 2;
            picture.layout(x, y, x + pw, y + ph);
        }
    }

    private final Handler main = new Handler(Looper.getMainLooper());
    private final ExoPlayer player;
    private final Screen screen;
    private final Events events;
    private final DataSource.Factory files;
    private final DefaultHttpDataSource.Factory http;
    private boolean active;
    private boolean ended;

    Exo(Context c, Events e) {
        events = e;
        screen = new Screen(c);
        // Start (and carry on after a seek) with less in the buffer than the default, so the
        // picture answers a tap sooner; the buffer still fills well ahead while playing.
        androidx.media3.exoplayer.DefaultLoadControl load = new androidx.media3.exoplayer.DefaultLoadControl.Builder()
                .setBufferDurationsMs(15000, 50000, 700, 1500)
                .build();
        player = new ExoPlayer.Builder(c.getApplicationContext()).setLoadControl(load).build();
        player.setVideoTextureView(screen.picture);
        // Other sound makes way for the video, and unplugging headphones pauses it.
        player.setAudioAttributes(new AudioAttributes.Builder().setUsage(C.USAGE_MEDIA)
                .setContentType(C.AUDIO_CONTENT_TYPE_MOVIE).build(), true);
        player.setHandleAudioBecomingNoisy(true);
        player.setWakeMode(C.WAKE_MODE_NETWORK);
        files = new DefaultDataSource.Factory(c.getApplicationContext());
        http = new DefaultHttpDataSource.Factory().setConnectTimeoutMs(10000).setReadTimeoutMs(20000);
        player.addListener(new Player.Listener() {
            @Override
            public void onIsPlayingChanged(boolean isPlaying) {
                if (active) {
                    events.onPlaying(isPlaying);
                    report();
                }
            }

            @Override
            public void onPlayWhenReadyChanged(boolean playWhenReady, int reason) {
                report();
            }

            @Override
            public void onPositionDiscontinuity(Player.PositionInfo from, Player.PositionInfo to, int reason) {
                report();
            }

            @Override
            public void onPlaybackStateChanged(int state) {
                if (!active) {
                    return;
                }
                if (state == Player.STATE_ENDED && !ended) {
                    ended = true;
                    events.onEnded();
                }
                report();
            }

            @Override
            public void onPlayerError(PlaybackException error) {
                if (active) {
                    Throwable cause = error.getCause();
                    events.onError(error.getErrorCodeName() + (cause == null ? "" : ": " + cause.getMessage()));
                }
            }

            @Override
            public void onVideoSizeChanged(VideoSize size) {
                if (size.width > 0 && size.height > 0) {
                    screen.videoWidth = Math.round(size.width * size.pixelWidthHeightRatio);
                    screen.videoHeight = size.height;
                    screen.requestLayout();
                }
            }
        });
    }

    View view() {
        return screen;
    }

    private MediaSource source(String url, boolean local) {
        // A fresh address can be refused for its first seconds (the app's stream proxy
        // passes that on), so a failed load is tried again quickly, several times.
        DefaultLoadErrorHandlingPolicy patient = new DefaultLoadErrorHandlingPolicy(50) {
            @Override
            public long getRetryDelayMsFor(LoadErrorInfo info) {
                long wait = super.getRetryDelayMsFor(info);
                return wait == C.TIME_UNSET ? wait : Math.min(wait, 500);
            }
        };
        return new ProgressiveMediaSource.Factory(local ? files : http)
                .setLoadErrorHandlingPolicy(patient)
                .createMediaSource(MediaItem.fromUri(Uri.parse(url)));
    }

    private void begin(MediaSource source, long positionMs, boolean play, String coverUrl) {
        active = true;
        ended = false;
        showCover(coverUrl);
        player.setMediaSource(source, Math.max(0, positionMs));
        player.prepare();
        player.setPlayWhenReady(play);
        main.removeCallbacks(tick);
        main.post(tick);
    }

    /**
     * Play a video from its streams. `video` null = sound only (then `coverUrl` is shown
     * in place of the picture).
     */
    void play(String video, String audio, long positionMs, boolean play, String coverUrl) {
        MediaSource a = source(audio, false);
        begin(video == null ? a : new MergingMediaSource(source(video, false), a), positionMs, play, video == null ? coverUrl : null);
    }

    /** Play a live stream from its HLS list (fetched with the browser name `agent`). */
    void playLive(String hls, String agent, boolean play) {
        DefaultHttpDataSource.Factory f = new DefaultHttpDataSource.Factory().setConnectTimeoutMs(10000).setReadTimeoutMs(20000);
        if (agent != null) {
            f.setUserAgent(agent);
        }
        MediaSource source = new androidx.media3.exoplayer.hls.HlsMediaSource.Factory(f)
                .createMediaSource(MediaItem.fromUri(Uri.parse(hls)));
        active = true;
        ended = false;
        showCover(null);
        player.setMediaSource(source);
        player.prepare();
        player.setPlayWhenReady(play);
        main.removeCallbacks(tick);
        main.post(tick);
    }

    boolean isLive() {
        return player.isCurrentMediaItemLive();
    }

    /** Back to the live edge of a live stream. */
    void goLive() {
        player.seekToDefaultPosition();
    }

    /** How far behind the live edge playback is, in seconds (0 when not live). */
    double behindLive() {
        if (!isLive()) {
            return 0;
        }
        long offset = player.getCurrentLiveOffset();
        long target = player.getCurrentMediaItem() == null || player.getCurrentMediaItem().liveConfiguration == null ? C.TIME_UNSET
                : player.getCurrentMediaItem().liveConfiguration.targetOffsetMs;
        if (offset == C.TIME_UNSET) {
            return 0;
        }
        return Math.max(0, (offset - (target == C.TIME_UNSET ? 0 : target)) / 1000.0);
    }

    int audioSession() {
        return player.getAudioSessionId();
    }

    /** The average colour of the picture now on screen (0 when there is none). */
    int frameColor() {
        if (!active || screen.picture.getVisibility() != View.VISIBLE || !screen.picture.isAvailable()) {
            return 0;
        }
        android.graphics.Bitmap b = screen.picture.getBitmap(8, 5);
        if (b == null) {
            return 0;
        }
        long r = 0, g = 0, bl = 0;
        int n = 0;
        for (int y = 0; y < b.getHeight(); y++) {
            for (int x = 0; x < b.getWidth(); x++) {
                int p = b.getPixel(x, y);
                r += (p >> 16) & 0xFF;
                g += (p >> 8) & 0xFF;
                bl += p & 0xFF;
                n++;
            }
        }
        b.recycle();
        return n == 0 ? 0 : Color.rgb((int) (r / n), (int) (g / n), (int) (bl / n));
    }

    /** Play a downloaded file. `coverUrl` non-null hides the picture (sound-only file, or audio-only mode). */
    void playFile(Uri file, long positionMs, boolean play, String coverUrl) {
        begin(source(file.toString(), true), positionMs, play, coverUrl);
    }

    /** Cover the picture with an image (audio only), or show the picture again with null. */
    void showCover(String url) {
        if (url == null) {
            screen.cover.setVisibility(View.GONE);
        } else {
            Images.load(screen.cover, url);
            screen.cover.setVisibility(View.VISIBLE);
        }
    }

    void stop() {
        active = false;
        main.removeCallbacks(tick);
        player.stop();
        player.clearMediaItems();
        screen.cover.setVisibility(View.GONE);
    }

    void release() {
        stop();
        player.release();
    }

    boolean isActive() {
        return active;
    }

    void setPlaying(boolean on) {
        if (on && player.getPlaybackState() == Player.STATE_ENDED) {
            ended = false;
            player.seekTo(0);
        }
        player.setPlayWhenReady(on);
    }

    boolean wantsToPlay() {
        return player.getPlayWhenReady() && player.getPlaybackState() != Player.STATE_ENDED;
    }

    long positionMs() {
        return Math.max(0, player.getCurrentPosition());
    }

    void seekTo(double seconds) {
        long total = player.getDuration();
        long to = Math.max(0, Math.round(seconds * 1000));
        if (total != C.TIME_UNSET && total > 1000) {
            to = Math.min(to, total - 500);
        }
        ended = false;
        player.seekTo(to);
        report();
    }

    void seekBy(double seconds) {
        seekTo(player.getCurrentPosition() / 1000.0 + seconds);
    }

    void setRate(double rate) {
        player.setPlaybackParameters(new PlaybackParameters((float) rate));
    }

    double rate() {
        return player.getPlaybackParameters().speed;
    }

    /**
     * Music: the cover fills the (square) view. The thumbnail is a 4:3 picture with black
     * bars above and below, so it is enlarged until the bars are outside the view.
     */
    void setMusic(boolean on) {
        screen.cover.setScaleType(on ? ImageView.ScaleType.CENTER_CROP : ImageView.ScaleType.FIT_CENTER);
        screen.cover.setScaleX(on ? 1.34f : 1f);
        screen.cover.setScaleY(on ? 1.34f : 1f);
        screen.setClipChildren(true);
    }

    void setFill(boolean on) {
        screen.fill = on;
        screen.requestLayout();
    }

    private void report() {
        if (!active) {
            return;
        }
        long total = player.getDuration();
        int state = player.getPlaybackState();
        events.onProgress(player.getCurrentPosition() / 1000.0, total == C.TIME_UNSET ? 0 : total / 1000.0,
                player.getBufferedPosition() / 1000.0, !player.getPlayWhenReady() || state == Player.STATE_ENDED,
                player.getPlayWhenReady() && (state == Player.STATE_BUFFERING || state == Player.STATE_IDLE));
    }

    private final Runnable tick = new Runnable() {
        @Override
        public void run() {
            if (active) {
                report();
                main.postDelayed(this, 250);
            }
        }
    };
}
