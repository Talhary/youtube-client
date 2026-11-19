//! The UI ↔ core boundary. The UI sends [`Command`]s and drains [`Event`]s; every network and
//! disk call runs on the core's tokio runtime, never on the UI thread. After posting an event
//! the core calls the shell's waker once so the UI repaints only when there is news.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::FutureExt;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::OnceCell;

use crate::api::{Api, ApiError, ApiOptions};
use crate::auth::{Auth, AuthEvent};
use crate::config::{Paths, Settings};
use crate::download::{DownloadEvent, DownloadKind, DownloadRequest, Downloads, JobId, Limits, ResolveFn};
use crate::model::*;
use crate::mux::{FfmpegMuxer, Muxer};
use crate::remote::RemoteConfig;
use crate::secure::Key;
use crate::streams::{self, HwCaps, QualityOption, ResolvedStreams, StreamChoice};
use crate::thumbs::{Rgba, Thumbs};
use crate::update::UpdateInfo;
use crate::ytdlp::YtDlp;

pub type ReqId = u64;

#[derive(Clone, Debug)]
pub enum Command {
    Home { req: ReqId },
    Search { req: ReqId, query: String, filter: crate::api::SearchOptions },
    /// A channel's "shorts", "live" or "posts" tab.
    ChannelTab { req: ReqId, id: String, tab: String },
    /// A channel's About page (answered as an action).
    ChannelAbout { req: ReqId, id: String },
    /// A destination page: "trending", "music", "gaming", "news", "sports", "live", "podcasts".
    Explore { req: ReqId, category: String },
    /// Caption tracks, preview pictures and live chat of a video (answered as an action).
    Extras { req: ReqId, id: String },
    /// One caption track's lines (`agent`: the browser name its link expects).
    Captions { req: ReqId, url: String, agent: Option<String> },
    /// New live chat messages.
    LiveChat { req: ReqId, token: String },
    /// The account's notifications.
    Notifications { req: ReqId },
    /// Choose a channel's bell setting with a handle from `Subscription`.
    SetBell { req: ReqId, params: String },
    /// The user's playlists and whether each holds a video.
    PlaylistOptions { req: ReqId, id: String },
    EditPlaylist { req: ReqId, playlist: String, video: String, add: bool },
    CreatePlaylist { req: ReqId, title: String, privacy: String, video: Option<String> },
    /// The watch history's clear and pause buttons.
    HistoryActions { req: ReqId },
    /// Send one of YouTube's feedback handles (clear or pause history).
    Feedback { req: ReqId, token: String },
    /// Shorts: `op` "feed" (optional `id` to start from), "more" (`token`), "info" (`id`,
    /// `params`) or "shelf" (the home page's Shorts). Answered as an action.
    Shorts { req: ReqId, op: String, id: Option<String>, params: Option<String>, token: Option<String> },
    /// A comment's replies, from its `replies_token`.
    CommentReplies { req: ReqId, token: String },
    /// Post a comment (`reply` false: `params` is the page's create handle) or a reply
    /// (`params` is the thread's reply handle). Answered as an action.
    PostComment { req: ReqId, params: String, text: String, reply: bool },
    /// Tell YouTube a video in the home feed is not interesting.
    NotInterested { req: ReqId, id: String },
    /// Videos on screen in a list: resolve their streams and read their details now, so
    /// one that is opened starts at once.
    PrefetchStreams { ids: Vec<String> },

    /// A YouTube Music page (answered as an action holding a [`crate::music::MusicPage`]).
    MusicBrowse { req: ReqId, id: String, params: Option<String> },
    /// A YouTube Music search; `tab` is "", "songs", "videos", "albums", "artists", "playlists" or "podcasts".
    MusicSearch { req: ReqId, query: String, tab: String },
    MusicSuggest { req: ReqId, query: String },
    /// More of a Music page, search or queue.
    MusicMore { req: ReqId, cursor: Cursor },
    /// What plays after a song (its radio), or the songs of a list.
    MusicQueue { req: ReqId, video: Option<String>, list: Option<String> },
    MusicLyrics { req: ReqId, id: String },
    /// Save an album or playlist to the library, or take it out.
    MusicSave { req: ReqId, list: String, on: bool },
    Suggest { req: ReqId, query: String },
    /// TVs on the local network that can play YouTube (DIAL), answered as an action.
    TvFind { req: ReqId },
    /// Ask a TV found by `TvFind` to play a video.
    TvPlay { req: ReqId, app_url: String, video: String, at: u32 },
    More { req: ReqId, cursor: Cursor },
    DropCursor(Cursor),
    Channel { req: ReqId, id: String },
    ChannelPlaylists { req: ReqId, id: String },
    Playlist { req: ReqId, id: String },
    Video { req: ReqId, id: String },
    /// The first page of a video's comments.
    Comments { req: ReqId, id: String },
    /// The next page of comments.
    MoreComments { req: ReqId, cursor: Cursor },
    /// The video was played for `seconds` in the app's own player: add it to the account's
    /// watch history, as YouTube's player would.
    Watched {
        id: String,
        seconds: u32,
        /// The video's length, 0 when unknown.
        total: u32,
    },
    /// The signed-in user's rating of a video.
    Rating { req: ReqId, id: String },
    /// Like or dislike a video, or take the rating back ("like", "dislike", "none").
    Rate { req: ReqId, id: String, rating: String },
    /// Is the signed-in user subscribed to a channel?
    Subscription { req: ReqId, id: String },
    /// Subscribe to a channel or leave it.
    Subscribe { req: ReqId, id: String, on: bool },
    /// Direct stream URLs of a video, for the app's own player.
    Streams { req: ReqId, id: String },
    /// Resolve streams for playback. `after_403` forces fresh URLs from yt-dlp.
    Resolve {
        req: ReqId,
        id: String,
        player_px_height: u32,
        after_403: bool,
    },
    /// Resolve a video in the background so it starts faster when opened.
    Prefetch { id: String },
    Thumbnail { url: String },
    /// Measured playback throughput in bits/s, used by auto quality.
    Throughput(u64),
    SetCaps(HwCaps),

    Feed { req: ReqId },
    Subscriptions { req: ReqId },
    History { req: ReqId },
    SavedPlaylists { req: ReqId },

    StartDeviceLogin,
    CancelLogin,
    ImportCookies(String),
    Logout,

    Download(DownloadRequest),
    /// `group`: the playlist's name, kept with each of its downloads.
    DownloadPlaylist { id: String, kind: DownloadKind, group: Option<String> },
    PauseDownload(JobId),
    ResumeDownload(JobId),
    CancelDownload(JobId),
    ClearDownloads,
    InstallFfmpeg,

    UpdateSettings(Settings),
    ExportDiagnostics(PathBuf),
    /// Delete cached thumbnails and yt-dlp's cache.
    ClearCache,
    /// Update the bundled yt-dlp to its latest release.
    UpdateYtDlp,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PlaybackInfo {
    pub video_id: String,
    pub title: String,
    pub channel: String,
    pub duration_s: u32,
    pub qualities: Vec<QualityOption>,
    /// Index into `qualities` chosen by the user's quality preference.
    pub selected: Option<usize>,
    pub audio_aac: Option<StreamChoice>,
    pub audio_opus: Option<StreamChoice>,
    /// Every video stream (all codecs), so the UI can rebuild `qualities` when codec support
    /// or the codec preference changes, without resolving again.
    #[serde(default)]
    pub all_video: Vec<StreamChoice>,
    /// HLS manifest for livestreams.
    pub hls: Option<String>,
    /// Stream URLs stop working after this (unix seconds); re-resolve before reuse.
    pub valid_until: i64,
    /// "yt-dlp" or the rustypipe client that resolved the streams.
    pub client: String,
    /// Dubbed sound tracks (the original first); empty for one track.
    #[serde(default)]
    pub audio_tracks: Vec<crate::streams::AudioTrack>,
    /// Loudness-levelled ("stable volume") AAC sound.
    #[serde(default)]
    pub audio_drc: Option<StreamChoice>,
}

impl PlaybackInfo {
    /// The audio stream that pairs with a quality (same container family when possible).
    pub fn audio_for(&self, q: Option<&QualityOption>) -> Option<&StreamChoice> {
        let want_aac = q.is_none_or(|q| q.stream.codec == "h264");
        if want_aac {
            self.audio_aac.as_ref().or(self.audio_opus.as_ref())
        } else {
            self.audio_opus.as_ref().or(self.audio_aac.as_ref())
        }
    }

    pub fn expired(&self) -> bool {
        time::OffsetDateTime::now_utc().unix_timestamp() + 60 >= self.valid_until
    }

    pub fn is_ytdlp(&self) -> bool {
        self.client == "yt-dlp"
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Status {
    pub botguard: Option<String>,
    pub ffmpeg: bool,
    pub version: String,
    /// yt-dlp version, if bundled or installed.
    pub ytdlp: Option<String>,
    /// Where downloads are saved.
    pub download_dir: PathBuf,
}

#[derive(Clone, Debug)]
pub enum Event {
    Ready(Status),
    Page { req: ReqId, result: Result<Page, String> },
    Suggestions { req: ReqId, items: Vec<String> },
    Channel { req: ReqId, result: Result<ChannelPage, String> },
    Playlist { req: ReqId, result: Result<PlaylistPage, String> },
    Video { req: ReqId, result: Result<VideoPage, String> },
    Comments { req: ReqId, result: Result<CommentPage, String> },
    Streams { req: ReqId, result: Result<ResolvedStreams, String> },
    /// Better streams for a video that is already playing: the first ones were a preview.
    StreamsUpdate { id: String, streams: ResolvedStreams },
    /// The answer to a like or subscription command, as a small JSON object.
    Action { req: ReqId, result: Result<serde_json::Value, String> },
    Playback { req: ReqId, result: Result<PlaybackInfo, String> },
    /// Fuller streams (from yt-dlp) for a video that started on the fast path.
    PlaybackUpgrade { video_id: String, info: PlaybackInfo },
    Thumbnail { url: String, image: Arc<Rgba> },
    ThumbnailFailed { url: String },
    Auth(AuthEvent),
    Download(DownloadEvent),
    Settings(Settings),
    Remote(RemoteConfig),
    UpdateAvailable(UpdateInfo),
    Toast(String),
}

pub struct CoreOptions {
    pub paths: Paths,
    pub key: Key,
    pub botguard: Option<PathBuf>,
    /// Muxer used by downloads (desktop: the FFmpeg muxer below; Android: MediaMuxer).
    pub muxer: Arc<dyn Muxer>,
    /// Set on desktop so `InstallFfmpeg` can point the muxer at a downloaded binary.
    pub ffmpeg: Option<Arc<FfmpegMuxer>>,
    pub caps: HwCaps,
    pub waker: Arc<dyn Fn() + Send + Sync>,
    pub version: String,
    /// GitHub builds check for updates; store builds do not.
    pub github_updates: bool,
}

#[derive(Clone)]
pub struct CoreHandle {
    tx: tokio::sync::mpsc::UnboundedSender<Command>,
}

impl CoreHandle {
    pub fn send(&self, c: Command) {
        let _ = self.tx.send(c);
    }
}

pub struct Core {
    rt: Option<tokio::runtime::Runtime>,
    pub handle: CoreHandle,
    events: Option<std::sync::mpsc::Receiver<Event>>,
}

impl Core {
    /// Take the event receiver to hand to the UI. Panics if called twice.
    pub fn take_events(&mut self) -> std::sync::mpsc::Receiver<Event> {
        self.events.take().expect("events already taken")
    }
}

impl Drop for Core {
    fn drop(&mut self) {
        if let Some(rt) = self.rt.take() {
            rt.shutdown_timeout(Duration::from_secs(1));
        }
    }
}

struct Sink {
    tx: std::sync::mpsc::Sender<Event>,
    waker: Arc<dyn Fn() + Send + Sync>,
}

impl Sink {
    fn emit(&self, e: Event) {
        if self.tx.send(e).is_ok() {
            (self.waker)();
        }
    }
}

type Cell = Arc<OnceCell<Result<ResolvedStreams, String>>>;

/// Turns video ids into streams. yt-dlp results are cached until shortly before the URLs
/// expire, and concurrent requests for one video share a single yt-dlp run.
struct Resolver {
    api: Arc<Api>,
    ytdlp: Option<Arc<YtDlp>>,
    cache: Mutex<HashMap<String, Cell>>,
    /// At most two background pre-loads run at a time.
    prefetch_slots: tokio::sync::Semaphore,
    prefetch_waiting: std::sync::atomic::AtomicUsize,
    /// Resolves the user is waiting for right now.
    urgent: Arc<std::sync::atomic::AtomicUsize>,
}

struct UrgentGuard(Arc<std::sync::atomic::AtomicUsize>);

impl UrgentGuard {
    fn new(c: &Arc<std::sync::atomic::AtomicUsize>) -> Self {
        c.fetch_add(1, Ordering::SeqCst);
        Self(c.clone())
    }
}

impl Drop for UrgentGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Re-resolve when fewer than this many seconds of URL validity remain.
const MIN_TTL: i64 = 30 * 60;

impl Resolver {
    fn cell(&self, id: &str, fresh: bool) -> Cell {
        let mut cache = self.cache.lock();
        let stale = fresh
            || cache.get(id).is_some_and(|c| match c.get() {
                Some(Ok(r)) => r.ttl() < MIN_TTL,
                Some(Err(_)) => true,
                None => false,
            });
        if stale {
            cache.remove(id);
        }
        if cache.len() > 200 {
            cache.retain(|_, c| matches!(c.get(), Some(Ok(r)) if r.ttl() >= MIN_TTL) || c.get().is_none());
        }
        cache.entry(id.to_string()).or_default().clone()
    }

    fn cached(&self, id: &str) -> Option<ResolvedStreams> {
        match self.cache.lock().get(id)?.get()? {
            Ok(r) if r.ttl() >= MIN_TTL => Some(r.clone()),
            _ => None,
        }
    }

    async fn ytdlp(&self, id: &str, fresh: bool, urgent: bool) -> Result<ResolvedStreams, String> {
        let Some(y) = self.ytdlp.clone() else {
            return Err("yt-dlp is not installed".into());
        };
        let cell = self.cell(id, fresh);
        let id2 = id.to_string();
        // While the user waits, background pre-loads hold off (see Command::Prefetch).
        let _waiting = urgent.then(|| UrgentGuard::new(&self.urgent));
        // Fresh URLs are only requested after a refusal, so use the full client set then.
        cell.get_or_init(|| async move { y.resolve(&id2, urgent, fresh).await })
            .await
            .clone()
    }

    /// rustypipe's quick path. Only H.264 video is kept: other codecs from this client are
    /// often refused by YouTube's servers.
    async fn fast(&self, id: &str) -> Result<ResolvedStreams, String> {
        let p = self.api.player(id, None).await.map_err(|e| e.to_string())?;
        let mut r = ResolvedStreams::from_player(&p);
        if r.video.iter().any(|v| v.codec == "h264") {
            r.video.retain(|v| v.codec == "h264");
        }
        if r.is_empty() {
            return Err("no playable streams".into());
        }
        Ok(r)
    }

    /// Best available: yt-dlp, else the quick path.
    async fn full(&self, id: &str, fresh: bool) -> Result<ResolvedStreams, String> {
        if self.ytdlp.is_none() {
            return self.fast(id).await;
        }
        match self.ytdlp(id, fresh, true).await {
            Ok(r) => Ok(r),
            Err(e) => {
                tracing::warn!("yt-dlp could not resolve {id}: {e}; trying the quick path");
                self.fast(id).await.map_err(|e2| format!("{e} ({e2})"))
            }
        }
    }
}

struct State {
    api: Arc<Api>,
    resolver: Arc<Resolver>,
    auth: Arc<Auth>,
    dl: Downloads,
    thumbs: Thumbs,
    http: reqwest::Client,
    paths: Paths,
    settings: Mutex<Settings>,
    remote: Mutex<RemoteConfig>,
    sink: Arc<Sink>,
    throughput: AtomicU64,
    caps: Mutex<HwCaps>,
    login: Mutex<Option<tokio::task::AbortHandle>>,
    muxer: Arc<dyn Muxer>,
    ffmpeg: Option<Arc<FfmpegMuxer>>,
    version: String,
    github_updates: bool,
    proxy: crate::proxy::StreamProxy,
    ytdlp_version: Mutex<Option<String>>,
    subs_cache: Mutex<Option<(std::time::Instant, Vec<ChannelCard>)>>,
    /// Becomes true once the startup sign-in check has finished.
    auth_ready: tokio::sync::watch::Receiver<bool>,
    auth_ready_tx: tokio::sync::watch::Sender<bool>,
    /// The encrypted session file (holds the signed-in YouTube cookie).
    session: crate::secure::EncryptedFile,
    /// "Load more" handles of lists read from YouTube's web pages. Their ids start above
    /// the ones the regular extractor hands out, so one cursor number names one list.
    web_cursors: Mutex<HashMap<u64, crate::webfeed::More>>,
    /// The same for comment lists read from YouTube's page API.
    web_comment_cursors: Mutex<HashMap<u64, crate::webfeed::CommentsMore>>,
    next_web_cursor: AtomicU64,
    /// Recently loaded video details, so reopening a video shows them at once.
    video_cache: Mutex<HashMap<String, (std::time::Instant, VideoPage)>>,
    /// Streams resolved ahead of time (full answers, already checked for previews), by video.
    stream_cache: Mutex<HashMap<String, crate::streams::ResolvedStreams>>,
    /// Videos being resolved ahead of time, and how many wait for a turn.
    stream_prefetching: Mutex<std::collections::HashSet<String>>,
    stream_slots: tokio::sync::Semaphore,
    /// Per video being watched: its playback id and how many seconds were reported so far.
    watch_reports: Mutex<HashMap<String, (String, u32)>>,
    /// Videos already watched, which Home leaves out.
    seen: Mutex<Seen>,
    /// Where to report playback of videos whose page was read (start, watch time).
    tracking: Mutex<HashMap<String, (String, String)>>,
    /// The user's rating of videos whose page was read ("like", "dislike", "none").
    ratings: Mutex<HashMap<String, String>>,
    /// The Shorts shelf of the last home page read.
    home_shorts: Mutex<Vec<serde_json::Value>>,
    /// "Not interested" handles of listed videos, by video id.
    dismiss: Mutex<HashMap<String, String>>,
    /// "Load more" handles of YouTube Music pages, searches and queues.
    music_cursors: Mutex<HashMap<u64, crate::music::More>>,
    /// What watch pages said about videos' chat, preview pictures and liveness.
    extras: Mutex<HashMap<String, crate::webfeed::Personal>>,
    /// Resolves direct stream URLs for the app's own player.
    ytp: Arc<crate::ytp::Ytp>,
}

/// First id used for web-page list cursors.
const WEB_CURSOR_BASE: u64 = 1 << 48;

/// Google sends ~17 KB of response headers on some hosts, above hyper's 16 KB HTTP/2 default,
/// which surfaces as an "unspecific protocol error". Every client we build raises the limit.
pub fn http_builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder().http2_max_header_list_size(256 * 1024)
}

pub fn http_client() -> reqwest::Client {
    http_builder()
        .user_agent(concat!("StreamPipe/", env!("CARGO_PKG_VERSION")))
        .pool_max_idle_per_host(crate::config::MAX_CONNECTIONS_CAP as usize)
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("http client")
}

impl Core {
    pub fn start(opts: CoreOptions) -> Result<Self, String> {
        crate::diagnostics::init();
        opts.paths.ensure().map_err(|e| e.to_string())?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("streampipe-core")
            .enable_all()
            .build()
            .map_err(|e| e.to_string())?;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Command>();
        let (etx, erx) = std::sync::mpsc::channel();
        let sink = Arc::new(Sink {
            tx: etx,
            waker: opts.waker.clone(),
        });

        let state = rt.block_on(async {
            let api = Arc::new(
                Api::new(
                    &opts.paths,
                    ApiOptions {
                        key: opts.key,
                        botguard: opts.botguard.clone(),
                    },
                )
                .map_err(|e| e.to_string())?,
            );
            let http = http_client();
            let settings = Settings::load(&opts.paths);
            let resolver = Arc::new(Resolver {
                api: api.clone(),
                ytdlp: YtDlp::find(&opts.paths.data, &opts.paths.cache, &opts.key).map(Arc::new),
                cache: Mutex::new(HashMap::new()),
                prefetch_slots: tokio::sync::Semaphore::new(2),
                prefetch_waiting: std::sync::atomic::AtomicUsize::new(0),
                urgent: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            });
            let r2 = resolver.clone();
            let ytp = Arc::new(crate::ytp::Ytp::new(http.clone(), &opts.paths.cache));
            let (ytp2, session2) = (ytp.clone(), crate::secure::EncryptedFile::new(opts.paths.data.join("session.bin"), &opts.key));
            let resolve_fn: ResolveFn = Arc::new(move |id: String, fresh: bool| {
                let (r, ytp, cookie) = (r2.clone(), ytp2.clone(), cookie_in(&session2));
                async move {
                    // With the yt-parser library (the phone), downloads use the same
                    // resolver as the player, with its check for preview-only URLs.
                    if crate::ytp::has_library() {
                        ytp.resolve(&id, cookie.as_deref()).await
                    } else {
                        r.full(&id, fresh).await
                    }
                }
                .boxed()
            });
            let s2 = sink.clone();
            let dl = Downloads::new(
                http.clone(),
                resolve_fn,
                opts.muxer.clone(),
                limits(&settings, None),
                settings.download_dir(&opts.paths),
                &opts.paths.data,
                Arc::new(move |e| s2.emit(Event::Download(e))),
            );
            let (auth_ready_tx, auth_ready_rx) = tokio::sync::watch::channel(false);
            let proxy = crate::proxy::StreamProxy::start(http.clone())
                .await
                .map_err(|e| format!("could not start the stream server: {e}"))?;
            Ok::<_, String>(Arc::new(State {
                proxy,
                resolver,
                auth: Arc::new(Auth::new(api.clone())),
                thumbs: Thumbs::new(http.clone(), opts.paths.cache.clone()),
                api,
                dl,
                http,
                paths: opts.paths.clone(),
                settings: Mutex::new(settings),
                remote: Mutex::new(RemoteConfig::default()),
                sink: sink.clone(),
                throughput: AtomicU64::new(0),
                caps: Mutex::new(opts.caps),
                login: Mutex::new(None),
                muxer: opts.muxer.clone(),
                ffmpeg: opts.ffmpeg.clone(),
                version: opts.version.clone(),
                github_updates: opts.github_updates,
                ytdlp_version: Mutex::new(None),
                subs_cache: Mutex::new(None),
                auth_ready: auth_ready_rx,
                auth_ready_tx,
                session: crate::secure::EncryptedFile::new(opts.paths.data.join("session.bin"), &opts.key),
                web_cursors: Mutex::new(HashMap::new()),
                web_comment_cursors: Mutex::new(HashMap::new()),
                next_web_cursor: AtomicU64::new(WEB_CURSOR_BASE),
                video_cache: Mutex::new(HashMap::new()),
                stream_cache: Mutex::new(HashMap::new()),
                stream_prefetching: Mutex::new(std::collections::HashSet::new()),
                stream_slots: tokio::sync::Semaphore::new(2),
                ratings: Mutex::new(HashMap::new()),
                home_shorts: Mutex::new(Vec::new()),
                dismiss: Mutex::new(HashMap::new()),
                music_cursors: Mutex::new(HashMap::new()),
                extras: Mutex::new(HashMap::new()),
                tracking: Mutex::new(HashMap::new()),
                watch_reports: Mutex::new(HashMap::new()),
                seen: Mutex::new(Seen::load(&opts.paths.data.join("watched.json"))),
                ytp,
            }))
        })?;

        // Restore the download queue before any command can enqueue new jobs.
        rt.block_on(async { state.dl.restore() });

        let st = state.clone();
        rt.spawn(async move { startup(st).await });

        let st = state.clone();
        rt.spawn(async move {
            while let Some(cmd) = rx.recv().await {
                let st = st.clone();
                tokio::spawn(async move { handle(st, cmd).await });
            }
        });

        Ok(Self {
            rt: Some(rt),
            handle: CoreHandle { tx },
            events: Some(erx),
        })
    }
}

fn limits(s: &Settings, remote: Option<&RemoteConfig>) -> Limits {
    let cap = remote
        .and_then(|r| r.max_connections)
        .unwrap_or(crate::config::MAX_CONNECTIONS_CAP);
    let max = s.max_connections.min(cap).max(1);
    Limits {
        max_connections: max,
        chunks_per_stream: s.chunks_per_stream.min(max),
        parallel_videos: s.parallel_videos.min(max),
    }
}

async fn status(st: &State) -> Status {
    Status {
        botguard: st.api.botguard_version().await,
        ffmpeg: st.muxer.available(),
        version: st.version.clone(),
        ytdlp: st.ytdlp_version.lock().clone(),
        download_dir: st.settings.lock().download_dir(&st.paths),
    }
}

async fn startup(st: Arc<State>) {
    let settings = st.settings.lock().clone();
    st.sink.emit(Event::Settings(settings.clone()));

    if let Some(y) = &st.resolver.ytdlp {
        *st.ytdlp_version.lock() = y.version().await;
    }
    st.sink.emit(Event::Ready(status(&st).await));

    match st.auth.startup_check().await {
        Some(m) => st.sink.emit(Event::Auth(AuthEvent::SignedIn(m))),
        None => st.sink.emit(Event::Auth(AuthEvent::SignedOut)),
    }
    let _ = st.auth_ready_tx.send(true);

    // uBlock Origin's YouTube rules for the web player, refreshed in the background.
    tokio::spawn(crate::adblock::keep_fresh(st.http.clone(), st.paths.cache.clone()));

    let remote = RemoteConfig::fetch(&st.http, &st.paths.cache).await;
    st.api.set_client_order(remote.client_types());
    st.dl.set_limits(limits(&settings, Some(&remote)));
    *st.remote.lock() = remote.clone();
    st.sink.emit(Event::Remote(remote));


    if st.github_updates && settings.check_updates {
        if let Some(u) = crate::update::check(&st.http, &st.version).await {
            st.sink.emit(Event::UpdateAvailable(u));
        }
    }

    // Token refresher: wakes every minute, refreshes at 80% of the token lifetime.
    loop {
        tokio::time::sleep(Duration::from_secs(60)).await;
        if st.auth.refresh_if_needed(false).await.is_err() {
            st.sink.emit(Event::Auth(AuthEvent::SignedOut));
            st.sink
                .emit(Event::Toast("Your session expired. You are now signed out.".into()));
        }
    }
}

/// Run a signed-in call; on an auth failure refresh once (single-flight) and retry.
async fn authed<T, F, Fut>(st: &State, f: F) -> Result<T, String>
where
    F: Fn() -> Fut,
    Fut: std::future::Future<Output = Result<T, ApiError>>,
{
    if st.auth.method().await.is_none() {
        return Err(ApiError::NeedsLogin.to_string());
    }
    match f().await {
        Err(e) if e.is_auth() => {
            st.auth.refresh_if_needed(true).await?;
            f().await.map_err(|e| e.to_string())
        }
        r => r.map_err(|e| e.to_string()),
    }
}

fn s<T>(r: Result<T, ApiError>) -> Result<T, String> {
    r.map_err(|e| e.to_string())
}

/// The signed-in user's subscribed channels (cached for 30 minutes).
async fn subscribed_channels(st: &State) -> Result<Vec<ChannelCard>, String> {
    if let Some((at, list)) = st.subs_cache.lock().as_ref() {
        if at.elapsed() < Duration::from_secs(1800) {
            return Ok(list.clone());
        }
    }
    let collect = |items: Vec<Item>| -> Vec<ChannelCard> {
        items
            .into_iter()
            .filter_map(|i| match i {
                Item::Channel(c) => Some(c),
                _ => None,
            })
            .collect()
    };
    let list = match authed(st, || async {
        let mut p = st.api.subscriptions().await?;
        // Follow every page: people subscribe to hundreds of channels.
        let mut items = std::mem::take(&mut p.items);
        let mut next = p.next;
        while let Some(c) = next {
            let more = st.api.more(c).await?;
            items.extend(more.items);
            next = more.next;
        }
        Ok(items)
    })
    .await
    {
        Ok(items) if !items.is_empty() => collect(items),
        Err(e) if e == ApiError::NeedsLogin.to_string() => return Err(e),
        other => {
            tracing::info!("subscriptions via web page or yt-dlp (rustypipe: {:?})", other.as_ref().err());
            match web_feed(st, crate::webfeed::Feed::Channels).await {
                Ok(p) => collect(p.items),
                Err(web) => collect(
                    flat_page(st, "https://www.youtube.com/feed/subscriptions", 2000)
                        .await
                        .map_err(|e| if st.resolver.ytdlp.is_some() { e } else { web })?
                        .items,
                ),
            }
        }
    };
    *st.subs_cache.lock() = Some((std::time::Instant::now(), list.clone()));
    Ok(list)
}

/// How long loaded video details are reused.
const VIDEO_CACHE_TTL: Duration = Duration::from_secs(10 * 60);

/// Details and related videos for one video.
///
/// Two sources are asked at the same time: the regular extractor (more complete: likes,
/// "load more" for related videos) and YouTube's own watch page. The extractor's answer is
/// used if it comes within a short grace period; after that, whichever answers first wins,
/// so a slow or failing extractor no longer holds the screen up.
async fn video_details(st: &Arc<State>, id: &str) -> Result<VideoPage, String> {
    if let Some((at, page)) = st.video_cache.lock().get(id) {
        if at.elapsed() < VIDEO_CACHE_TTL {
            return Ok(page.clone());
        }
    }
    let started = std::time::Instant::now();
    let web = {
        let (http, cookie, id, st) = (st.http.clone(), session_cookie(st), id.to_string(), st.clone());
        tokio::spawn(async move {
            // The page also says how the user rated the video; that is kept for `Rating`.
            let mut personal = crate::webfeed::Personal::default();
            let page = crate::webfeed::watch(&http, cookie.as_deref(), &id, Some(&mut personal)).await;
            remember_personal(&st, &id, personal);
            page
        })
    };
    let primary = st.api.video(id);
    tokio::pin!(primary);
    tokio::pin!(web);
    let flat = |r: Result<Result<VideoPage, String>, tokio::task::JoinError>| r.unwrap_or_else(|e| Err(e.to_string()));
    let (result, source) = match tokio::time::timeout(Duration::from_millis(1200), &mut primary).await {
        Ok(Ok(v)) => (Ok(v), "extractor"),
        Ok(Err(e)) => (flat((&mut web).await).map_err(|_| e.to_string()), "web page (extractor failed)"),
        Err(_) => tokio::select! {
            r = &mut primary => match r {
                Ok(v) => (Ok(v), "extractor (slow)"),
                Err(e) => (flat((&mut web).await).map_err(|_| e.to_string()), "web page (extractor failed)"),
            },
            r = &mut web => match flat(r) {
                Ok(v) => (Ok(v), "web page (extractor slow)"),
                Err(_) => (primary.await.map_err(|e| e.to_string()), "extractor (web page failed)"),
            },
        },
    };
    // The extractor can miss the channel, views and age of related videos (new page layouts);
    // the web page, already on its way, has them.
    let mut result = result;
    if let Ok(page) = &mut result {
        let videos: Vec<&VideoCard> = page.related.items.iter().filter_map(|i| if let Item::Video(v) = i { Some(v) } else { None }).collect();
        let bare = videos.iter().filter(|v| v.channel_name.is_none() && v.published.is_none() && v.views.is_none()).count();
        let poor = !videos.is_empty() && bare * 2 >= videos.len();
        let missing = page.subscribers.is_none() || page.video.channel_avatar.is_none() || page.chapters.is_empty();
        // Only where the web page has not been awaited yet (a finished task must not be polled again).
        if (source == "extractor" || source == "extractor (slow)") && (poor || missing) {
            if let Ok(Ok(Ok(web))) = tokio::time::timeout(Duration::from_secs(4), &mut web).await {
                if poor && !web.related.items.is_empty() {
                    tracing::info!("video details for {id}: related videos from the web page (extractor's were bare)");
                    page.related = web.related;
                }
                page.subscribers = page.subscribers.take().or(web.subscribers);
                page.video.channel_avatar = page.video.channel_avatar.take().or(web.video.channel_avatar);
                if page.chapters.is_empty() {
                    page.chapters = web.chapters;
                }
            }
        }
    }
    tracing::info!("video details for {id} in {:.1}s from the {source}", started.elapsed().as_secs_f32());
    if let Ok(page) = &result {
        let mut cache = st.video_cache.lock();
        if cache.len() >= 40 {
            cache.retain(|_, (at, _)| at.elapsed() < VIDEO_CACHE_TTL);
            if cache.len() >= 40 {
                cache.clear();
            }
        }
        // A list's "load more" handle works once, so the cached copy does not carry it.
        let mut copy = page.clone();
        copy.related.next = None;
        cache.insert(id.to_string(), (std::time::Instant::now(), copy));
    }
    result
}

/// A signed-in list read from YouTube's own web page with the session cookie
/// (see [`crate::webfeed`]). Errors when there is no cookie session. An empty list is a
/// valid answer (for example no subscriptions).
async fn web_feed(st: &State, feed: crate::webfeed::Feed) -> Result<Page, String> {
    let cookie = session_cookie(st).ok_or_else(|| ApiError::NeedsLogin.to_string())?;
    if feed == crate::webfeed::Feed::Home {
        // Both at once: the home page, and what the account has watched lately.
        let (home, _) = tokio::join!(crate::webfeed::fetch_home(&st.http, &cookie), sync_seen(st, &cookie));
        let (mut page, more, dismiss, shelf) = home?;
        if !shelf.is_empty() {
            *st.home_shorts.lock() = shelf;
        }
        remember_dismiss(st, dismiss);
        drop_seen(st, &mut page);
        return Ok(with_web_cursor(st, page, more));
    }
    let (page, more, dismiss) = crate::webfeed::fetch(&st.http, &cookie, feed).await?;
    remember_dismiss(st, dismiss);
    Ok(with_web_cursor(st, page, more))
}

/// Videos the user has already watched, here or anywhere on the account: Home leaves them out.
struct Seen {
    /// Oldest first.
    ids: Vec<String>,
    /// How much of each was watched, in percent; 0 when that is not known.
    set: HashMap<String, u8>,
    /// When the account's history was last read.
    synced: Option<std::time::Instant>,
}

impl Seen {
    const LIMIT: usize = 4000;

    fn load(path: &std::path::Path) -> Seen {
        let list: Vec<(String, u8)> = std::fs::read(path).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
        Seen { set: list.iter().cloned().collect(), ids: list.into_iter().map(|(id, _)| id).collect(), synced: None }
    }

    /// True if any of them was new.
    fn add(&mut self, ids: impl IntoIterator<Item = (String, u8)>) -> bool {
        let mut changed = false;
        for (id, percent) in ids {
            match self.set.get_mut(&id) {
                Some(known) if *known >= percent => {}
                Some(known) => {
                    *known = percent;
                    changed = true;
                }
                None => {
                    self.set.insert(id.clone(), percent);
                    self.ids.push(id);
                    changed = true;
                }
            }
        }
        if self.ids.len() > Self::LIMIT {
            let extra = self.ids.len() - Self::LIMIT;
            for old in self.ids.drain(..extra) {
                self.set.remove(&old);
            }
        }
        changed
    }
}

fn seen_file(st: &State) -> PathBuf {
    st.paths.data.join("watched.json")
}

fn add_seen(st: &State, ids: Vec<(String, u8)>) {
    let mut seen = st.seen.lock();
    if seen.add(ids) {
        let list: Vec<(&String, u8)> = seen.ids.iter().map(|id| (id, seen.set.get(id).copied().unwrap_or(0))).collect();
        if let Ok(bytes) = serde_json::to_vec(&list) {
            let _ = crate::config::write_atomic(&seen_file(st), &bytes);
        }
    }
}

/// Read the account's watch history (at most every few minutes) into the watched list.
async fn sync_seen(st: &State, cookie: &str) {
    if st.seen.lock().synced.is_some_and(|at| at.elapsed() < Duration::from_secs(180)) {
        return;
    }
    match crate::webfeed::fetch(&st.http, cookie, crate::webfeed::Feed::History).await {
        Ok((page, _, _)) => {
            // Oldest first, as the list is kept.
            let ids: Vec<(String, u8)> = page
                .items
                .iter()
                .rev()
                .filter_map(|i| match i {
                    Item::Video(v) => Some((v.id.clone(), v.watched.unwrap_or(0))),
                    _ => None,
                })
                .collect();
            tracing::info!("home feed: the account's history lists {} videos", ids.len());
            add_seen(st, ids);
            st.seen.lock().synced = Some(std::time::Instant::now());
        }
        Err(e) => tracing::warn!("home feed: could not read the watch history: {e}"),
    }
}

fn drop_seen(st: &State, page: &mut Page) {
    let seen = st.seen.lock();
    let before = page.items.len();
    // In the history or played here, and anything YouTube itself marks as nearly finished.
    page.items.retain(|i| !matches!(i, Item::Video(v) if seen.set.contains_key(&v.id) || v.watched.is_some_and(|p| p >= 85)));
    tracing::info!("home feed: left out {} of {} videos already watched ({} known)", before - page.items.len(), before, seen.set.len());
}

/// Keep what a watch page said for the signed-in user (rating, playback report addresses).
fn remember_personal(st: &State, id: &str, personal: crate::webfeed::Personal) {
    if let Some(r) = personal.rating.clone() {
        remember_rating(st, id, r);
    }
    {
        let mut map = st.extras.lock();
        if map.len() > 60 {
            map.clear();
        }
        map.insert(id.to_string(), personal.clone());
    }
    if let Some(t) = personal.tracking {
        let mut map = st.tracking.lock();
        if map.len() > 100 {
            map.clear();
        }
        map.insert(id.to_string(), t);
    }
}

/// A Music page as JSON for the app, with a cursor for more when there is more.
fn music_answer(st: &State, mut page: crate::music::MusicPage, more: Option<crate::music::More>) -> serde_json::Value {
    if let Some(m) = more {
        let id = st.next_web_cursor.fetch_add(1, Ordering::Relaxed);
        let mut map = st.music_cursors.lock();
        if map.len() > 64 {
            map.clear();
        }
        map.insert(id, m);
        page.next = Some(id);
    }
    serde_json::to_value(&page).unwrap_or_default()
}

fn remember_dismiss(st: &State, list: crate::webfeed::Dismiss) {
    let mut map = st.dismiss.lock();
    if map.len() > 600 {
        map.clear();
    }
    map.extend(list);
}

/// Pages anyone can see, read with the session when there is one.
/// Most of the videos in `p` came without views, age and length (nothing to show under them).
fn bare(p: &Page) -> bool {
    let videos: Vec<&VideoCard> = p.items.iter().filter_map(|i| if let Item::Video(v) = i { Some(v) } else { None }).collect();
    let empty = videos.iter().filter(|v| v.views.is_none() && v.published.is_none() && v.duration_s.is_none()).count();
    !videos.is_empty() && empty * 2 > videos.len()
}

async fn web_list(st: &State, url: &str, feed: crate::webfeed::Feed) -> Result<Page, String> {
    let cookie = session_cookie(st);
    let (page, more, dismiss) = crate::webfeed::fetch_url(&st.http, cookie.as_deref(), url, feed).await?;
    remember_dismiss(st, dismiss);
    Ok(with_web_cursor(st, page, more))
}

/// A signed-in action that answers with JSON.
async fn signed<F, Fut>(st: &State, f: F) -> Result<serde_json::Value, String>
where
    F: FnOnce(String) -> Fut,
    Fut: std::future::Future<Output = Result<serde_json::Value, String>>,
{
    match session_cookie(st) {
        None => Err(ApiError::NeedsLogin.to_string()),
        Some(cookie) => f(cookie).await,
    }
}

fn remember_rating(st: &State, id: &str, rating: String) {
    let mut map = st.ratings.lock();
    if map.len() > 300 {
        map.clear();
    }
    map.insert(id.to_string(), rating);
}

/// The signed-in YouTube cookie header from the session file.
fn session_cookie(st: &State) -> Option<String> {
    cookie_in(&st.session)
}

fn cookie_in(session: &crate::secure::EncryptedFile) -> Option<String> {
    session
        .read()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|j| j.get("auth_cookie")?.get("cookie")?.as_str().map(str::to_string))
}

/// Remember a web list's "load more" handle and put its cursor on the page.
fn with_web_cursor(st: &State, mut page: Page, more: Option<crate::webfeed::More>) -> Page {
    // What was played here counts where YouTube's list does not say (yet).
    {
        let seen = st.seen.lock();
        for item in &mut page.items {
            if let Item::Video(v) = item {
                if let Some(p) = seen.set.get(&v.id).copied().filter(|p| *p > 0) {
                    v.watched = Some(v.watched.unwrap_or(0).max(p));
                }
            }
        }
    }
    if let Some(m) = more {
        let id = st.next_web_cursor.fetch_add(1, Ordering::Relaxed);
        let mut map = st.web_cursors.lock();
        // Lists that were abandoned without being dropped must not pile up.
        if map.len() > 64 {
            map.clear();
        }
        map.insert(id, m);
        page.next = Some(Cursor(id));
    }
    page
}

/// A page of comments from the page API, with a cursor for the next one if there is more.
fn web_comment_page(st: &State, items: Vec<CommentCard>, more: Option<crate::webfeed::CommentsMore>) -> CommentPage {
    let next = more.map(|m| {
        let id = st.next_web_cursor.fetch_add(1, Ordering::Relaxed);
        let mut map = st.web_comment_cursors.lock();
        if map.len() > 32 {
            map.clear();
        }
        map.insert(id, m);
        Cursor(id)
    });
    CommentPage { items, next, create_params: None }
}

/// The next screen of a web list (single use, like the extractor's cursors).
async fn web_more(st: &State, cursor: Cursor) -> Result<Page, String> {
    let m = st.web_cursors.lock().remove(&cursor.0).ok_or("this list cannot load more")?;
    // Lists that are not personal (destination pages, channel tabs) load more signed out too.
    let cookie = session_cookie(st);
    let (mut page, more, dismiss) = crate::webfeed::more(&st.http, cookie.as_deref(), &m).await?;
    remember_dismiss(st, dismiss);
    if m.feed == crate::webfeed::Feed::Home {
        drop_seen(st, &mut page);
    }
    Ok(with_web_cursor(st, page, more))
}

/// A YouTube page fetched as a flat list by yt-dlp, for lists rustypipe can no longer read.
async fn flat_page(st: &State, url: &str, limit: usize) -> Result<crate::ytdlp::FlatResult, String> {
    match &st.resolver.ytdlp {
        Some(y) => y.flat(url, limit).await,
        None => Err("yt-dlp is not installed".into()),
    }
}

async fn handle(st: Arc<State>, cmd: Command) {
    let emit = |e| st.sink.emit(e);
    match cmd {
        Command::Home { req } => {
            // Wait (briefly) for the startup sign-in check so a signed-in user gets their feed.
            let mut ready = st.auth_ready.clone();
            let _ = tokio::time::timeout(Duration::from_secs(8), ready.wait_for(|r| *r)).await;
            let signed_in = st.auth.method().await.is_some();
            // Signed in: the user's own home page, read from YouTube's web page; yt-dlp is
            // the second choice where it exists.
            let mut personal = None;
            if signed_in {
                match web_feed(&st, crate::webfeed::Feed::Home).await {
                    Ok(p) if p.items.is_empty() => {}
                    Ok(p) => personal = Some(p),
                    Err(e) => tracing::warn!("personal home feed (web page): {e}"),
                }
            }
            if personal.is_none() && signed_in && st.resolver.ytdlp.is_some() {
                match flat_page(&st, ":ytrec", 60).await {
                    Ok(f) if !f.items.is_empty() => personal = Some(Page { items: f.items, next: None }),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("personal home feed: {e}"),
                }
            }
            let result = match personal {
                Some(p) => Ok(p),
                None => s(st.api.home().await),
            };
            emit(Event::Page { req, result })
        }
        Command::Search { req, query, filter } => emit(Event::Page {
            req,
            result: s(if filter.is_empty() { st.api.search(&query).await } else { st.api.search_filtered(&query, &filter).await }),
        }),
        Command::ChannelTab { req, id, tab } => {
            let result = match tab.as_str() {
                "posts" => web_list(&st, &format!("https://www.youtube.com/channel/{id}/posts"), crate::webfeed::Feed::Posts).await,
                "playlists" => s(st.api.channel_playlists(&id).await),
                other => match st.api.channel_tab(&id, other).await {
                    Ok(p) if other != "shorts" && bare(&p) => web_list(&st, &format!("https://www.youtube.com/channel/{id}/{other}"), crate::webfeed::Feed::Videos)
                        .await
                        .or(Ok(p)),
                    Ok(p) => Ok(p),
                    // The extractor can fail on a tab; the channel's own page is the second source.
                    Err(e) => web_list(&st, &format!("https://www.youtube.com/channel/{id}/{other}"), crate::webfeed::Feed::Videos)
                        .await
                        .map_err(|_| e.to_string()),
                },
            };
            emit(Event::Page { req, result })
        }
        Command::ChannelAbout { req, id } => emit(Event::Action { req, result: s(st.api.channel_about(&id).await) }),
        Command::Explore { req, category } => {
            use crate::api::SearchOptions;
            let page = |u: &str| format!("https://www.youtube.com/channel/{u}");
            let result = match category.as_str() {
                "music" => web_list(&st, &page("UC-9-kyTW8ZkZNDHQJ6FgpwQ"), crate::webfeed::Feed::Explore).await,
                "gaming" => web_list(&st, "https://www.youtube.com/gaming", crate::webfeed::Feed::Explore).await,
                "news" => web_list(&st, &page("UCYfdidRxbB8Qhf0Nx7ioOYw"), crate::webfeed::Feed::Explore).await,
                "sports" => web_list(&st, &page("UCEgdi0XIXXZ-qJOFPf4JSKw"), crate::webfeed::Feed::Explore).await,
                "live" => s(st.api.search_filtered("live", &SearchOptions { live: true, ..Default::default() }).await),
                "podcasts" => s(st
                    .api
                    .search_filtered("podcast full episode", &SearchOptions { kind: Some("video".into()), length: Some("long".into()), ..Default::default() })
                    .await),
                _ => s(st.api.home().await),
            };
            emit(Event::Page { req, result })
        }
        Command::Extras { req, id } => {
            // The watch page is usually being read for the details right now; wait for it a moment.
            let mut known = None;
            for _ in 0..30 {
                known = st.extras.lock().get(&id).cloned();
                if known.is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(150)).await;
            }
            let player = crate::webfeed::player_extras(&st.http, &id).await;
            if let Err(e) = &player {
                tracing::info!("captions for {id}: {e}");
            }
            let player = player.unwrap_or_default();
            let known = known.unwrap_or_default();
            let storyboard = player.get("storyboard").and_then(|v| v.as_str()).map(str::to_string).or(known.storyboard);
            emit(Event::Action {
                req,
                result: Ok(serde_json::json!({
                    "captions": player.get("captions").cloned().unwrap_or_else(|| serde_json::json!([])),
                    "storyboard": storyboard,
                    "live": known.live,
                    "chat": known.chat,
                })),
            })
        }
        Command::Captions { req, url, agent } => {
            let result = crate::webfeed::captions(&st.http, &url, agent.as_deref()).await;
            emit(Event::Action { req, result })
        }
        Command::LiveChat { req, token } => {
            let cookie = session_cookie(&st);
            let result = crate::webfeed::live_chat(&st.http, cookie.as_deref(), &token).await;
            emit(Event::Action { req, result })
        }
        Command::Notifications { req } => {
            let result = match session_cookie(&st) {
                None => Err(ApiError::NeedsLogin.to_string()),
                Some(c) => crate::webfeed::notifications(&st.http, &c).await,
            };
            emit(Event::Page { req, result })
        }
        Command::SetBell { req, params } => {
            let http = st.http.clone();
            let result = signed(&st, |c| async move { crate::webfeed::set_bell(&http, &c, &params).await.map(|()| serde_json::json!({})) }).await;
            emit(Event::Action { req, result })
        }
        Command::PlaylistOptions { req, id } => {
            let http = st.http.clone();
            let result = signed(&st, |c| async move { crate::webfeed::playlist_options(&http, &c, &id).await }).await;
            emit(Event::Action { req, result })
        }
        Command::EditPlaylist { req, playlist, video, add } => {
            let http = st.http.clone();
            let result = signed(&st, |c| async move {
                crate::webfeed::edit_playlist(&http, &c, &playlist, &video, add).await.map(|()| serde_json::json!({"added": add}))
            })
            .await;
            emit(Event::Action { req, result })
        }
        Command::CreatePlaylist { req, title, privacy, video } => {
            let http = st.http.clone();
            let result = signed(&st, |c| async move {
                crate::webfeed::create_playlist(&http, &c, &title, &privacy, video.as_deref()).await.map(|id| serde_json::json!({"id": id}))
            })
            .await;
            emit(Event::Action { req, result })
        }
        Command::Shorts { req, op, id, params, token } => {
            let cookie = session_cookie(&st);
            let c = cookie.as_deref();
            let result = match op.as_str() {
                "feed" => crate::shorts::feed(&st.http, c, id.as_deref()).await,
                "more" => crate::shorts::more(&st.http, c, token.as_deref().unwrap_or("")).await,
                "info" => crate::shorts::info(&st.http, c, id.as_deref().unwrap_or(""), params.as_deref().unwrap_or("")).await,
                "shelf" => Ok(serde_json::Value::Array(st.home_shorts.lock().clone())),
                // Not a Short, but the same kind of page-API read: the account for "You".
                "account" => match c {
                    Some(c) => crate::shorts::account(&st.http, c).await,
                    None => Err(ApiError::NeedsLogin.to_string()),
                },
                other => Err(format!("unknown Shorts request {other}")),
            };
            emit(Event::Action { req, result })
        }
        Command::HistoryActions { req } => {
            let http = st.http.clone();
            let result = signed(&st, |c| async move { crate::webfeed::history_actions(&http, &c).await }).await;
            emit(Event::Action { req, result })
        }
        Command::Feedback { req, token } => {
            let http = st.http.clone();
            let result = signed(&st, |c| async move { crate::webfeed::feedback(&http, &c, &token).await.map(|()| serde_json::json!({})) }).await;
            emit(Event::Action { req, result })
        }
        Command::PrefetchStreams { ids } => {
            let cookie = session_cookie(&st);
            let now = time::OffsetDateTime::now_utc().unix_timestamp();
            for id in ids.into_iter().take(6) {
                let fresh = st.stream_cache.lock().get(&id).is_some_and(|r| r.valid_until - now > 1800);
                // At most a handful wait; scrolling on drops the rest.
                let queued = {
                    let mut busy = st.stream_prefetching.lock();
                    if fresh || busy.contains(&id) || busy.len() >= 6 {
                        false
                    } else {
                        busy.insert(id.clone())
                    }
                };
                if !queued {
                    continue;
                }
                let (st, cookie) = (st.clone(), cookie.clone());
                tokio::spawn(async move {
                    let slot = st.stream_slots.acquire().await;
                    if slot.is_ok() {
                        // The details page too (cached for ten minutes); it is only a few KB.
                        let details = video_details(&st, &id);
                        let streams = st.ytp.resolve(&id, cookie.as_deref());
                        let (_, streams) = tokio::join!(details, streams);
                        match streams {
                            Ok(r) => {
                                let mut cache = st.stream_cache.lock();
                                if cache.len() > 60 {
                                    let now = time::OffsetDateTime::now_utc().unix_timestamp();
                                    cache.retain(|_, r| r.valid_until - now > 600);
                                    if cache.len() > 60 {
                                        cache.clear();
                                    }
                                }
                                cache.insert(id.clone(), r);
                            }
                            Err(e) => tracing::info!("prefetch {id}: {e}"),
                        }
                    }
                    st.stream_prefetching.lock().remove(&id);
                });
            }
        }
        Command::TvFind { req } => emit(Event::Action { req, result: crate::dial::discover(&st.http).await }),
        Command::TvPlay { req, app_url, video, at } => {
            emit(Event::Action { req, result: crate::dial::launch(&st.http, &app_url, &video, at).await })
        }
        Command::MusicBrowse { req, id, params } => {
            let cookie = session_cookie(&st);
            let result = crate::music::browse(&st.http, cookie.as_deref(), &id, params.as_deref())
                .await
                .map(|(page, more)| music_answer(&st, page, more));
            emit(Event::Action { req, result })
        }
        Command::MusicSearch { req, query, tab } => {
            let cookie = session_cookie(&st);
            let result = crate::music::search(&st.http, cookie.as_deref(), &query, &tab)
                .await
                .map(|(page, more)| music_answer(&st, page, more));
            emit(Event::Action { req, result })
        }
        Command::MusicSuggest { req, query } => {
            let cookie = session_cookie(&st);
            let items = crate::music::suggest(&st.http, cookie.as_deref(), &query).await.unwrap_or_default();
            emit(Event::Suggestions { req, items })
        }
        Command::MusicMore { req, cursor } => {
            let taken = st.music_cursors.lock().remove(&cursor.0);
            let result = match taken {
                None => Err("this list cannot load more".to_string()),
                Some(m) => {
                    let cookie = session_cookie(&st);
                    crate::music::more(&st.http, cookie.as_deref(), &m).await.map(|(page, more)| music_answer(&st, page, more))
                }
            };
            emit(Event::Action { req, result })
        }
        Command::MusicQueue { req, video, list } => {
            let cookie = session_cookie(&st);
            let result = crate::music::queue(&st.http, cookie.as_deref(), video.as_deref(), list.as_deref())
                .await
                .map(|(mut v, more)| {
                    if let Some(m) = more {
                        let id = st.next_web_cursor.fetch_add(1, Ordering::Relaxed);
                        st.music_cursors.lock().insert(id, m);
                        v["next"] = serde_json::json!(id);
                    }
                    v
                });
            emit(Event::Action { req, result })
        }
        Command::MusicLyrics { req, id } => {
            let cookie = session_cookie(&st);
            let result = crate::music::lyrics(&st.http, cookie.as_deref(), &id).await;
            emit(Event::Action { req, result })
        }
        Command::MusicSave { req, list, on } => {
            let http = st.http.clone();
            let result = signed(&st, |c| async move { crate::music::save(&http, &c, &list, on).await.map(|()| serde_json::json!({"saved": on})) }).await;
            emit(Event::Action { req, result })
        }
        Command::NotInterested { req, id } => {
            let token = st.dismiss.lock().get(&id).cloned();
            let http = st.http.clone();
            let result = match token {
                // Not every list offers it; the app still hides the video.
                None => Ok(serde_json::json!({"sent": false})),
                Some(t) => signed(&st, |c| async move { crate::webfeed::feedback(&http, &c, &t).await.map(|()| serde_json::json!({"sent": true})) }).await,
            };
            add_seen(&st, vec![(id, 0)]);
            emit(Event::Action { req, result })
        }
        Command::Suggest { req, query } => {
            let items = st.api.suggestions(&query).await.unwrap_or_default();
            emit(Event::Suggestions { req, items })
        }
        Command::More { req, cursor } => {
            let result = if cursor.0 >= WEB_CURSOR_BASE {
                web_more(&st, cursor).await
            } else {
                s(st.api.more(cursor).await)
            };
            emit(Event::Page { req, result })
        }
        Command::DropCursor(c) => {
            if c.0 >= WEB_CURSOR_BASE {
                st.web_cursors.lock().remove(&c.0);
                st.web_comment_cursors.lock().remove(&c.0);
            } else {
                st.api.drop_cursor(c)
            }
        }
        Command::Channel { req, id } => {
            let mut result = s(st.api.channel(&id).await);
            // On YouTube's newer channel layout the extractor's videos come without views,
            // age or length; the channel's own Videos page has them.
            if let Ok(page) = result.as_mut() {
                if bare(&page.videos) {
                    match web_list(&st, &format!("https://www.youtube.com/channel/{id}/videos"), crate::webfeed::Feed::Videos).await {
                        Ok(web) if !web.items.is_empty() => page.videos = web,
                        Ok(_) => {}
                        Err(e) => tracing::info!("channel {id}: videos page: {e}"),
                    }
                }
            }
            emit(Event::Channel { req, result })
        }
        Command::ChannelPlaylists { req, id } => emit(Event::Page {
            req,
            result: s(st.api.channel_playlists(&id).await),
        }),
        Command::Playlist { req, id } => {
            let result = match st.api.playlist(&id).await {
                Ok(p) if !p.videos.items.is_empty() => Ok(p),
                other => {
                    let why = other.err().map(|e| e.to_string()).unwrap_or_else(|| "empty".into());
                    let cookie = session_cookie(&st);
                    match crate::webfeed::playlist(&st.http, cookie.as_deref(), &id).await {
                        Ok((card, page, more)) if !page.items.is_empty() || st.resolver.ytdlp.is_none() => {
                            tracing::info!("playlist {id} from its web page (extractor: {why})");
                            let videos = with_web_cursor(&st, page, more);
                            emit(Event::Playlist { req, result: Ok(PlaylistPage { playlist: card, videos }) });
                            return;
                        }
                        Ok(_) => {}
                        Err(e) => tracing::info!("playlist {id}: web page: {e}"),
                    }
                    tracing::info!("playlist {id} via yt-dlp (rustypipe: {why})");
                    flat_page(&st, &format!("https://www.youtube.com/playlist?list={id}"), 300)
                        .await
                        .map(|f| PlaylistPage {
                            playlist: PlaylistCard {
                                id: id.clone(),
                                name: f.title,
                                thumbnail: f.thumbnail,
                                channel_name: f.channel,
                                video_count: f.count,
                            },
                            videos: Page { items: f.items, next: None },
                        })
                }
            };
            emit(Event::Playlist { req, result })
        }
        Command::Video { req, id } => {
            let result = video_details(&st, &id).await;
            emit(Event::Video { req, result })
        }
        Command::Comments { req, id } => {
            // YouTube's own page API first: it carries the replies' handles and, signed in,
            // the handles for commenting and replying. The extractor is the second source.
            let cookie = session_cookie(&st);
            let result = match crate::webfeed::comments(&st.http, cookie.as_deref(), &id).await {
                Ok((items, more, create)) => {
                    let mut page = web_comment_page(&st, items, more);
                    page.create_params = create;
                    Ok(page)
                }
                Err(e) => {
                    tracing::info!("comments for {id} from the extractor (page API: {e})");
                    s(st.api.comments(&id).await)
                }
            };
            emit(Event::Comments { req, result })
        }
        Command::CommentReplies { req, token } => {
            let cookie = session_cookie(&st);
            let result = crate::webfeed::comment_replies(&st.http, cookie.as_deref(), &token)
                .await
                .map(|(items, more)| web_comment_page(&st, items, more));
            emit(Event::Comments { req, result })
        }
        Command::PostComment { req, params, text, reply } => {
            let result = match session_cookie(&st) {
                None => Err(ApiError::NeedsLogin.to_string()),
                Some(cookie) => crate::webfeed::post_comment(&st.http, &cookie, &params, &text, reply).await,
            };
            emit(Event::Action { req, result })
        }
        Command::Watched { id, seconds, total } => {
            if let Some(cookie) = session_cookie(&st) {
                let mut tracking = st.tracking.lock().get(&id).cloned();
                if tracking.is_none() {
                    let mut personal = crate::webfeed::Personal::default();
                    let _ = crate::webfeed::watch(&st.http, Some(&cookie), &id, Some(&mut personal)).await;
                    tracking = personal.tracking.clone();
                    remember_personal(&st, &id, personal);
                }
                // Played here for a while: Home need not offer it again.
                if seconds >= 30 {
                    let percent = if total > 0 { (seconds as u64 * 100 / total as u64).min(100) as u8 } else { 0 };
                    add_seen(&st, vec![(id.clone(), percent)]);
                }
                // One playback id per video, and each report covers what was played since the last.
                let (cpn, from, first) = {
                    let mut reports = st.watch_reports.lock();
                    if reports.len() > 100 {
                        reports.clear();
                    }
                    match reports.get_mut(&id) {
                        // The position went back (the video was started again): a new playback.
                        Some((cpn, last)) if *last <= seconds => {
                            let from = *last;
                            *last = seconds;
                            (cpn.clone(), from, false)
                        }
                        _ => {
                            let cpn = crate::webfeed::playback_id();
                            reports.insert(id.clone(), (cpn.clone(), seconds));
                            (cpn, 0, true)
                        }
                    }
                };
                match tracking {
                    Some(t) => match crate::webfeed::mark_watched(&st.http, &cookie, &t, &cpn, from, seconds, first).await {
                        Ok(()) => tracing::info!("watch history: reported {id} ({from}-{seconds}s)"),
                        Err(e) => tracing::warn!("watch history: {id}: {e}"),
                    },
                    None => tracing::warn!("watch history: YouTube's page gave no report address for {id}"),
                }
            }
        }
        Command::Rating { req, id } => {
            let result = match session_cookie(&st) {
                None => Ok(serde_json::json!({"rating": null})),
                Some(cookie) => {
                    // The video's page is usually being read for its details right now.
                    let mut known = None;
                    for _ in 0..40 {
                        known = st.ratings.lock().get(&id).cloned();
                        if known.is_some() {
                            break;
                        }
                        tokio::time::sleep(Duration::from_millis(150)).await;
                    }
                    if known.is_none() {
                        let mut personal = crate::webfeed::Personal::default();
                        let _ = crate::webfeed::watch(&st.http, Some(&cookie), &id, Some(&mut personal)).await;
                        known = personal.rating.clone();
                        remember_personal(&st, &id, personal);
                    }
                    Ok(serde_json::json!({"rating": known}))
                }
            };
            emit(Event::Action { req, result })
        }
        Command::Rate { req, id, rating } => {
            let result = match session_cookie(&st) {
                None => Err(ApiError::NeedsLogin.to_string()),
                Some(cookie) => crate::webfeed::rate(&st.http, &cookie, &id, &rating).await.map(|()| {
                    remember_rating(&st, &id, rating.clone());
                    serde_json::json!({"rating": rating})
                }),
            };
            emit(Event::Action { req, result })
        }
        Command::Subscription { req, id } => {
            let result = match session_cookie(&st) {
                None => Ok(serde_json::json!({"subscribed": null})),
                Some(cookie) => crate::webfeed::channel_state(&st.http, &cookie, &id).await,
            };
            emit(Event::Action { req, result })
        }
        Command::Subscribe { req, id, on } => {
            let result = match session_cookie(&st) {
                None => Err(ApiError::NeedsLogin.to_string()),
                Some(cookie) => crate::webfeed::subscribe(&st.http, &cookie, &id, on).await.map(|()| {
                    // The list of subscriptions changed.
                    *st.subs_cache.lock() = None;
                    serde_json::json!({"subscribed": on})
                }),
            };
            emit(Event::Action { req, result })
        }
        Command::Streams { req, id } => {
            let cookie = session_cookie(&st);
            // Resolved ahead of time and still good for a while: start at once.
            let ready = st
                .stream_cache
                .lock()
                .get(&id)
                .filter(|r| r.valid_until - (time::OffsetDateTime::now_utc().unix_timestamp()) > 600)
                .cloned();
            if let Some(mut r) = ready {
                tracing::info!("direct streams for {id} from the prefetch cache");
                proxy_streams(&st, &id, &mut r);
                emit(Event::Streams { req, result: Ok(r) });
                return;
            }
            // The bytes go through the core's proxy, so they are fetched by the same client
            // (and from the same address) that asked for the URLs; YouTube ties them to it.
            let result = st.ytp.resolve_fast(&id, cookie.as_deref()).await.map(|mut r| {
                proxy_streams(&st, &id, &mut r);
                r
            });
            // The quick answer skipped the check for preview-only URLs (both the public and
            // the signed-in path can hand those out). It runs now, while the video starts; if
            // it answers from another path, or had to ask again, the player gets its streams.
            let quick = match &result {
                Ok(r) if r.source.starts_with("ytp-") => Some(r.source.clone()),
                _ => None,
            };
            emit(Event::Streams { req, result });
            if let Some(quick) = quick {
                if let Ok(mut full) = st.ytp.resolve(&id, cookie.as_deref()).await {
                    if full.source != quick {
                        tracing::info!("direct streams for {id} were a preview ({quick}); switching to {}", full.source);
                        proxy_streams(&st, &id, &mut full);
                        emit(Event::StreamsUpdate { id, streams: full });
                    }
                }
            }
        }
        Command::MoreComments { req, cursor } => {
            let result = if cursor.0 >= WEB_CURSOR_BASE {
                // Take the handle out first: the lock must not be held while waiting.
                let taken = st.web_comment_cursors.lock().remove(&cursor.0);
                match taken {
                    Some(m) => {
                        let cookie = session_cookie(&st);
                        crate::webfeed::comments_more(&st.http, cookie.as_deref(), &m)
                            .await
                            .map(|(items, more)| web_comment_page(&st, items, more))
                    }
                    None => Err("this list cannot load more".into()),
                }
            } else {
                s(st.api.more_comments(cursor).await)
            };
            emit(Event::Comments { req, result })
        }
        Command::Resolve {
            req,
            id,
            player_px_height,
            after_403,
        } => resolve_for_playback(&st, req, id, player_px_height, after_403).await,
        Command::Prefetch { id } => {
            if !st.settings.lock().preload || st.resolver.cached(&id).is_some() {
                return;
            }
            // Queue at most a dozen pre-loads; two run at a time so playback always has a
            // free worker.
            if st.resolver.prefetch_waiting.fetch_add(1, Ordering::SeqCst) >= 12 {
                st.resolver.prefetch_waiting.fetch_sub(1, Ordering::SeqCst);
                return;
            }
            let slot = st.resolver.prefetch_slots.acquire().await;
            st.resolver.prefetch_waiting.fetch_sub(1, Ordering::SeqCst);
            // Don't compete with a video the user is waiting for.
            while st.resolver.urgent.load(Ordering::SeqCst) > 0 {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            if slot.is_ok() && st.resolver.cached(&id).is_none() {
                let _ = st.resolver.ytdlp(&id, false, false).await;
            }
        }
        Command::Thumbnail { url } => match st.thumbs.get(&url).await {
            Ok(image) => emit(Event::Thumbnail { url, image }),
            Err(e) => {
                tracing::debug!("thumbnail {url}: {e}");
                emit(Event::ThumbnailFailed { url })
            }
        },
        // The player's download rate drops to the video's own bitrate once its buffer is
        // full, so a sample only proves the connection is at least that fast: keep the peak.
        // Storing the latest sample made every video start lower than the one before.
        Command::Throughput(bps) => {
            st.throughput.fetch_max(bps, Ordering::Relaxed);
        }
        Command::SetCaps(c) => {
            *st.caps.lock() = c;
            if let Ok(b) = serde_json::to_vec(&c) {
                let _ = crate::config::write_atomic(&st.paths.config.join("hwcaps.json"), &b);
            }
        }

        Command::Feed { req } => {
            let result = match authed(&st, || st.api.subscription_feed()).await {
                Ok(p) if !p.items.is_empty() => Ok(p),
                Err(e) if e == ApiError::NeedsLogin.to_string() => Err(e),
                other => {
                    tracing::info!("subscription feed via web page or RSS (rustypipe: {:?})", other.as_ref().map(|p| p.items.len()));
                    if let Ok(p) = web_feed(&st, crate::webfeed::Feed::Subscriptions).await {
                        emit(Event::Page { req, result: Ok(p) });
                        return;
                    }
                    match subscribed_channels(&st).await {
                        Ok(ch) if ch.is_empty() => Ok(Page::default()),
                        Ok(ch) => Ok(st.api.rss_feed(&ch, 150).await),
                        Err(e) => Err(e),
                    }
                }
            };
            emit(Event::Page { req, result })
        }
        Command::Subscriptions { req } => {
            let result = subscribed_channels(&st).await.map(|ch| Page {
                items: ch.into_iter().map(Item::Channel).collect(),
                next: None,
            });
            emit(Event::Page { req, result })
        }
        Command::History { req } => {
            let result = match authed(&st, || st.api.history()).await {
                Ok(p) if !p.items.is_empty() => Ok(p),
                Err(e) if e == ApiError::NeedsLogin.to_string() => Err(e),
                other => web_feed(&st, crate::webfeed::Feed::History).await.or(other),
            };
            emit(Event::Page { req, result })
        }
        Command::SavedPlaylists { req } => {
            let result = match authed(&st, || st.api.saved_playlists()).await {
                Ok(p) if !p.items.is_empty() => Ok(p),
                Err(e) if e == ApiError::NeedsLogin.to_string() => Err(e),
                other => web_feed(&st, crate::webfeed::Feed::Playlists).await.or(other),
            };
            emit(Event::Page { req, result })
        }

        Command::StartDeviceLogin => {
            let allowed = st.settings.lock().tv_sign_in && st.remote.lock().device_login;
            if !allowed {
                emit(Event::Auth(AuthEvent::Failed(
                    "Code sign-in is turned off. Use \"Sign in with Google\" instead.".into(),
                )));
                return;
            }
            if let Some(t) = st.login.lock().take() {
                t.abort();
            }
            let st2 = st.clone();
            let task = tokio::spawn(async move {
                let sink = st2.sink.clone();
                let r = st2
                    .auth
                    .device_flow(|e| sink.emit(Event::Auth(e)))
                    .await;
                if let Err(e) = r {
                    if e != "access denied" {
                        sink.emit(Event::Auth(AuthEvent::Failed(e)));
                    }
                }
            });
            *st.login.lock() = Some(task.abort_handle());
        }
        Command::CancelLogin => {
            if let Some(t) = st.login.lock().take() {
                t.abort();
            }
            emit(Event::Auth(AuthEvent::SignedOut));
        }
        Command::ImportCookies(txt) => match st.auth.import_cookies(&txt).await {
            Ok(()) => {
                // Streams resolved while signed out may differ; start fresh.
                st.resolver.cache.lock().clear();
                emit(Event::Auth(AuthEvent::SignedIn(crate::auth::SignInMethod::Cookies)))
            }
            Err(e) => emit(Event::Auth(AuthEvent::Failed(e))),
        },
        Command::Logout => {
            st.auth.logout().await;
            st.resolver.cache.lock().clear();
            *st.subs_cache.lock() = None;
            emit(Event::Auth(AuthEvent::SignedOut));
        }

        Command::Download(req) => {
            st.dl.enqueue(req);
        }
        Command::DownloadPlaylist { id, kind, group } => {
            let videos: Result<Vec<(String, String)>, String> = match st.api.playlist_all(&id).await {
                Ok(v) if !v.is_empty() => Ok(v.into_iter().map(|v| (v.id, v.title)).collect()),
                _ => flat_page(&st, &format!("https://www.youtube.com/playlist?list={id}"), 2000)
                    .await
                    .map(|f| {
                        f.items
                            .into_iter()
                            .filter_map(|i| match i {
                                Item::Video(v) => Some((v.id, v.title)),
                                _ => None,
                            })
                            .collect()
                    }),
            };
            match videos {
                Ok(videos) => {
                    let n = videos.len();
                    for (video_id, title) in videos {
                        st.dl.enqueue(DownloadRequest {
                            video_id,
                            title,
                            kind: kind.clone(),
                            group: group.clone(),
                        });
                    }
                    emit(Event::Toast(format!("Queued {n} videos")));
                }
                Err(e) => emit(Event::Toast(format!("Could not load playlist: {e}"))),
            }
        }
        Command::PauseDownload(id) => st.dl.pause(id),
        Command::ResumeDownload(id) => st.dl.resume(id),
        Command::CancelDownload(id) => st.dl.cancel(id),
        Command::ClearDownloads => st.dl.clear_finished(),
        Command::InstallFfmpeg => install_ffmpeg(&st).await,

        Command::UpdateSettings(mut new) => {
            new.clamp();
            if let Err(e) = new.save(&st.paths) {
                emit(Event::Toast(format!("Could not save settings: {e}")));
            }
            let remote = st.remote.lock().clone();
            st.dl.set_limits(limits(&new, Some(&remote)));
            st.dl.set_out_dir(new.download_dir(&st.paths));
            let dir_changed = st.settings.lock().download_dir != new.download_dir;
            *st.settings.lock() = new.clone();
            emit(Event::Settings(new));
            if dir_changed {
                emit(Event::Ready(status(&st).await));
            }
        }
        Command::ExportDiagnostics(path) => {
            let header = format!(
                "StreamPipe {} on {} {}\nclients: {:?}\nbotguard: {:?}\nyt-dlp: {:?}",
                st.version,
                std::env::consts::OS,
                std::env::consts::ARCH,
                st.api.client_order(),
                st.api.botguard_version().await,
                st.ytdlp_version.lock().clone(),
            );
            let path2 = path.clone();
            let r = tokio::task::spawn_blocking(move || {
                crate::diagnostics::export(&path2, &header)
            })
            .await;
            emit(Event::Toast(match r {
                Ok(Ok(())) => format!("Diagnostics saved to {}", path.display()),
                Ok(Err(e)) => format!("Could not save diagnostics: {e}"),
                Err(e) => format!("Could not save diagnostics: {e}"),
            }));
        }
        Command::ClearCache => {
            let freed = st.thumbs.clear();
            let mut freed_dl = 0;
            if let Some(y) = &st.resolver.ytdlp {
                freed_dl = dir_size(y.cache_dir());
                let _ = std::fs::remove_dir_all(y.cache_dir());
            }
            st.resolver.cache.lock().clear();
            emit(Event::Toast(format!(
                "Cleared {:.1} MB of cached data",
                (freed + freed_dl) as f64 / 1_048_576.0
            )));
        }
        Command::UpdateYtDlp => {
            let Some(y) = st.resolver.ytdlp.clone() else {
                emit(Event::Toast("yt-dlp is not installed".into()));
                return;
            };
            emit(Event::Toast("Updating yt-dlp…".into()));
            match y.self_update().await {
                Ok(msg) => {
                    *st.ytdlp_version.lock() = y.version().await;
                    emit(Event::Toast(msg));
                    emit(Event::Ready(status(&st).await));
                }
                Err(e) => emit(Event::Toast(format!("yt-dlp update failed: {e}"))),
            }
        }
    }
}

fn dir_size(p: &std::path::Path) -> u64 {
    let Ok(rd) = std::fs::read_dir(p) else { return 0 };
    rd.flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_size(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}

#[cfg(not(target_os = "android"))]
async fn install_ffmpeg(st: &State) {
    let Some(ff) = &st.ffmpeg else {
        return;
    };
    st.sink
        .emit(Event::Toast("Downloading FFmpeg (about 90 MB)…".into()));
    match crate::mux::download_ffmpeg(&st.http, &st.paths.data).await {
        Ok(bin) => {
            ff.set_bin(bin);
            st.sink.emit(Event::Toast("FFmpeg installed".into()));
            st.sink.emit(Event::Ready(status(st).await));
        }
        Err(e) => st.sink.emit(Event::Toast(e)),
    }
}

#[cfg(target_os = "android")]
async fn install_ffmpeg(_st: &State) {}

/// Playback start: reuse a cached or in-progress yt-dlp result for this video, otherwise
/// resolve now. `after_403` forces fresh URLs.
async fn resolve_for_playback(st: &Arc<State>, req: ReqId, id: String, px: u32, after_403: bool) {
    // With the yt-parser library (the phone's resolver, also built for the desktop): the
    // prefetch cache, then the quick answer, checked for preview-only URLs while it plays.
    if crate::ytp::has_library() {
        let cookie = session_cookie(st);
        let ready = (!after_403)
            .then(|| {
                st.stream_cache
                    .lock()
                    .get(&id)
                    .filter(|r| r.valid_until - time::OffsetDateTime::now_utc().unix_timestamp() > 600)
                    .cloned()
            })
            .flatten();
        if let Some(r) = ready {
            tracing::info!("playback for {id} from the prefetch cache ({})", r.source);
            st.sink.emit(Event::Playback { req, result: Ok(playback_info(st, &id, px, r)) });
            return;
        }
        let fast = if after_403 {
            st.ytp.resolve(&id, cookie.as_deref()).await
        } else {
            st.ytp.resolve_fast(&id, cookie.as_deref()).await
        };
        let quick = match &fast {
            Ok(r) if !after_403 => Some(r.source.clone()),
            _ => None,
        };
        match fast {
            Ok(r) => st.sink.emit(Event::Playback { req, result: Ok(playback_info(st, &id, px, r)) }),
            Err(e) => {
                st.sink.emit(Event::Playback { req, result: Err(e) });
                return;
            }
        }
        // As on the phone: the quick answer skipped the check for preview-only URLs. It runs
        // now, while the video starts; if the full answer comes from another path, the proxy
        // keeps the player's local addresses and fetches from the full streams instead.
        if let Some(quick) = quick {
            match st.ytp.resolve(&id, cookie.as_deref()).await {
                Ok(full) if full.source != quick => {
                    tracing::info!("streams for {id} were a preview ({quick}); switching to {}", full.source);
                    let info = playback_info(st, &id, px, full);
                    st.sink.emit(Event::PlaybackUpgrade { video_id: id, info });
                }
                Ok(_) => {}
                Err(e) => tracing::info!("full streams for {id}: {e}"),
            }
        }
        return;
    }
    let result = st
        .resolver
        .full(&id, after_403)
        .await
        .map(|r| playback_info(st, &id, px, r));
    st.sink.emit(Event::Playback { req, result });
}

/// Route streams through the local server, build the quality list and pick a start quality.
fn playback_info(st: &State, id: &str, px: u32, mut r: ResolvedStreams) -> PlaybackInfo {
    let group = if crate::ytp::has_library() { id.to_string() } else { format!("{id}:{}", r.source) };
    proxy_streams(st, &group, &mut r);
    let settings = st.settings.lock().clone();
    let caps = streams::apply_codec_pref(*st.caps.lock(), settings.codec);
    let qualities = streams::qualities_from(&r.video, &caps);
    let tp = st.throughput.load(Ordering::Relaxed);
    let pick = match settings.quality {
        crate::config::QualityPref::Auto => {
            streams::pick_auto(&qualities, px, (tp > 0).then_some(tp))
        }
        crate::config::QualityPref::Max(h) => streams::pick_max(&qualities, h),
    };
    let selected = pick.and_then(|q| qualities.iter().position(|x| x == q));
    tracing::info!(
        "playback for {id} via {}: {} rungs, start at {:?}",
        r.source,
        qualities.len(),
        selected.and_then(|i| qualities.get(i)).map(|q| format!("{} {}", q.label, q.stream.codec))
    );
    PlaybackInfo {
        video_id: id.to_string(),
        title: r.title,
        channel: r.channel,
        duration_s: r.duration_s,
        qualities,
        selected,
        audio_aac: r.audio_aac,
        audio_opus: r.audio_opus,
        all_video: r.video,
        hls: r.hls,
        valid_until: r.valid_until,
        client: r.source,
        audio_tracks: r.audio_tracks,
        audio_drc: r.audio_drc,
    }
}

/// Point players at the local stream server instead of googlevideo (see [`crate::proxy`]).
/// Streams whose size is unknown keep their direct URL.
fn proxy_streams(st: &State, group: &str, r: &mut ResolvedStreams) {
    let size_of = |s: &StreamChoice| {
        s.size
            .filter(|n| *n > 0)
            .or_else(|| crate::download::clen(&s.url))
    };
    let mut slots: Vec<&mut StreamChoice> = r.video.iter_mut().collect();
    slots.extend(r.audio_aac.as_mut());
    slots.extend(r.audio_opus.as_mut());
    slots.extend(r.audio_drc.as_mut());
    slots.extend(r.audio_tracks.iter_mut().map(|t| &mut t.stream));
    let slots: Vec<&mut StreamChoice> = slots.into_iter().filter(|s| size_of(s).is_some()).collect();
    let streams = slots
        .iter()
        .map(|s| crate::proxy::ProxyStream {
            url: s.url.clone(),
            size: size_of(s).unwrap(),
            mime: s.mime.clone(),
            headers: s.headers.clone(),
        })
        .collect();
    let urls = st.proxy.register(group, streams);
    for (s, u) in slots.into_iter().zip(urls) {
        s.url = u;
    }
}
