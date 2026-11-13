use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use egui::{Key, TextureHandle};
use streampipe_core::auth::{AuthEvent, SignInMethod};
use streampipe_core::config::Settings;
use streampipe_core::download::{DownloadEvent, DownloadRequest, JobId, JobStatus};
use streampipe_core::model::*;
use streampipe_core::remote::RemoteConfig;
use streampipe_core::service::ReqId;
use streampipe_core::streams::HwCaps;
use streampipe_core::update::UpdateInfo;
use streampipe_core::{Command, CoreHandle, Event, PlaybackInfo, Status};

use crate::extra::{ActionFn, ChannelTab, Chat, Comments, Dialog, Extras, Sleep, WatchTab, apply_audio};
use crate::{FileDialog, FormFactor, LoadRequest, Platform, Player, PlayerEvent, motion, theme};

/// How many videos at the top of a freshly loaded list are prepared in the background.
/// Zero: pre-loading follows the user (hover, "Up next") to keep request volume low.
const PRELOAD_FIRST: usize = 4;

/// A paged list of items with its in-flight requests.
#[derive(Default)]
pub(crate) struct List {
    pub items: Vec<Item>,
    pub next: Option<Cursor>,
    /// First page in flight.
    pub loading: Option<ReqId>,
    /// Next page in flight.
    pub more: Option<ReqId>,
    pub error: Option<String>,
    pub selected: Option<usize>,
    pub loaded_once: bool,
}

impl List {
    pub fn is_empty_loading(&self) -> bool {
        self.loading.is_some() && self.items.is_empty()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
    Home,
    Search,
    Subscriptions,
    Library,
    Downloads,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LibraryTab {
    History,
    Playlists,
    Channels,
}

/// A page pushed on top of a tab.
pub(crate) enum Page {
    Channel {
        id: String,
        req: ReqId,
        header: Option<ChannelPage>,
        videos: List,
        error: Option<String>,
        tab: ChannelTab,
        /// The list of the tab on screen (other than Videos).
        tab_list: List,
        about: Option<serde_json::Value>,
    },
    Notifications {
        list: List,
    },
    Music(crate::music::MusicView),
    Playlist {
        id: String,
        req: ReqId,
        header: Option<PlaylistPage>,
        videos: List,
        error: Option<String>,
    },
}

impl Page {
    /// Every list on the page.
    pub(crate) fn lists(&mut self) -> Vec<&mut List> {
        match self {
            Page::Channel { videos, tab_list, .. } => vec![videos, tab_list],
            Page::Playlist { videos, .. } => vec![videos],
            Page::Notifications { list } => vec![list],
            Page::Music(_) => vec![],
        }
    }

    /// The list on screen.
    pub(crate) fn shown(&mut self) -> Option<&mut List> {
        match self {
            Page::Channel { videos, tab: ChannelTab::Videos, .. } => Some(videos),
            Page::Channel { tab_list, .. } => Some(tab_list),
            Page::Playlist { videos, .. } => Some(videos),
            Page::Notifications { list } => Some(list),
            Page::Music(_) => None,
        }
    }
}

/// A video waiting in the queue.
#[derive(Clone)]
pub(crate) struct QueueItem {
    pub card: VideoCard,
    /// A YouTube Music song (plays as sound with its album art).
    pub music: bool,
}

pub(crate) struct NowPlaying {
    pub card: VideoCard,
    pub resolve_req: Option<ReqId>,
    pub details_req: Option<ReqId>,
    pub info: Option<PlaybackInfo>,
    pub details: Option<VideoPage>,
    pub related: List,
    pub quality: Option<usize>,
    pub error: Option<String>,
    pub retried_403: bool,
    /// Position to restore after a re-resolve.
    pub resume_at: Option<f64>,
    /// Phone: expanded from the mini-player into the full player page.
    pub expanded: bool,
    /// The quick stream source was refused; waiting for yt-dlp's result since this moment.
    pub awaiting_upgrade: Option<Instant>,
    /// Pre-loaded the first "Up next" video.
    pub prefetched_next: bool,
    /// When the current stream request started (for the "Preparing…" timer).
    pub requested_at: Instant,
    /// Formats YouTube refused for this video (by itag); never offered again.
    pub refused: std::collections::HashSet<u32>,
    /// A YouTube Music song.
    pub music: bool,
    pub extras: Option<Extras>,
    /// The caption track shown, and its lines.
    pub caption: Option<usize>,
    pub cues: Vec<crate::extra::Cue>,
    pub chat: Chat,
    pub comments: Comments,
    /// "like", "dislike" or "none".
    pub rating: Option<String>,
    /// The chosen dubbed sound track.
    pub audio_track: Option<usize>,
    /// The streams' own sound (before a dubbed or levelled track replaced it).
    pub audio_orig: (Option<streampipe_core::streams::StreamChoice>, Option<streampipe_core::streams::StreamChoice>),
    /// Where playback was last reported to the account's history.
    pub reported_at: Option<f64>,
    pub saved_at: Instant,
    /// YouTube Music: the song's lyrics and related pages, and what they hold.
    pub lyrics_id: Option<String>,
    pub related_id: Option<String>,
    pub lyrics: Option<Result<String, String>>,
    pub music_related: Option<crate::music::MusicView>,
}

pub(crate) enum Thumb {
    Loading,
    Ready(TextureHandle),
    Failed,
}

pub(crate) struct DlRow {
    pub req: DownloadRequest,
    pub status: JobStatus,
    pub done: u64,
    pub total: u64,
    pub bps: u64,
}

#[derive(Default)]
pub(crate) struct AuthUi {
    pub method: Option<SignInMethod>,
    pub device_code: Option<(String, String)>,
    pub waiting: bool,
    pub error: Option<String>,
    pub dialog_open: bool,
    pub browser_open: bool,
    /// Cookies from the browser window are being validated by the core.
    pub checking: bool,
}

pub(crate) enum FileResult {
    Cookies(Option<std::path::PathBuf>),
    Folder(Option<std::path::PathBuf>),
    Diagnostics(Option<std::path::PathBuf>),
    BrowserSignIn(Result<String, String>),
}

pub struct App {
    pub(crate) core: CoreHandle,
    events: Receiver<Event>,
    pub(crate) player: Box<dyn Player>,
    pub(crate) platform: Box<dyn Platform>,
    pub(crate) form: FormFactor,
    next_req: ReqId,

    pub(crate) tab: Tab,
    pub(crate) stack: Vec<Page>,
    pub(crate) home: List,
    pub(crate) search_text: String,
    pub(crate) search_submitted: String,
    pub(crate) search: List,
    pub(crate) suggestions: Vec<String>,
    suggest_req: Option<ReqId>,
    suggest_due: Option<(Instant, String)>,
    pub(crate) focus_search: bool,
    pub(crate) feed: List,
    pub(crate) library_tab: LibraryTab,
    pub(crate) history: List,
    pub(crate) saved: List,
    pub(crate) subs: List,

    pub(crate) now: Option<NowPlaying>,
    pub(crate) audio_only: bool,
    pub(crate) caps: HwCaps,
    pub(crate) theater: bool,

    pub(crate) thumbs: HashMap<String, Thumb>,
    pub(crate) downloads: BTreeMap<JobId, DlRow>,
    pub(crate) settings: Option<Settings>,
    pub(crate) remote: RemoteConfig,
    pub(crate) status: Option<Status>,
    pub(crate) update: Option<UpdateInfo>,
    pub(crate) auth: AuthUi,
    pub(crate) toasts: Vec<(String, Instant)>,

    files_tx: Sender<FileResult>,
    files_rx: Receiver<FileResult>,
    last_throughput: Instant,
    theme_applied: bool,
    pub(crate) video_px_height: u32,
    pub(crate) prefetched: std::collections::HashSet<String>,
    applied_scale: f32,

    pub(crate) actions: HashMap<ReqId, ActionFn>,
    pub(crate) store: crate::store::Store,
    pub(crate) queue: Vec<QueueItem>,
    pub(crate) home_chip: usize,
    pub(crate) search_filter: streampipe_core::api::SearchOptions,
    pub(crate) dialog: Option<Dialog>,
    pub(crate) sleep: Option<Sleep>,
    pub(crate) watch_tab: WatchTab,
    /// The watch history's buttons: (kind, handle).
    pub(crate) history_actions: Vec<(String, String)>,
    /// Channel id -> its subscribe state (`{subscribed, bell, bell_now}`).
    pub(crate) subs_state: HashMap<String, serde_json::Value>,
    pub(crate) music: crate::music::MusicState,
    /// The Audio only choice for videos (songs follow the Song/Video switch).
    pub(crate) video_audio_only: bool,
}

impl App {
    pub fn new(
        core: CoreHandle,
        events: Receiver<Event>,
        player: Box<dyn Player>,
        platform: Box<dyn Platform>,
        caps: HwCaps,
    ) -> Self {
        let (files_tx, files_rx) = channel();
        let form = platform.form_factor();
        let store = crate::store::Store::load(platform.data_dir());
        let mut app = Self {
            core,
            events,
            player,
            platform,
            form,
            next_req: 1,
            tab: Tab::Home,
            stack: Vec::new(),
            home: List::default(),
            search_text: String::new(),
            search_submitted: String::new(),
            search: List::default(),
            suggestions: Vec::new(),
            suggest_req: None,
            suggest_due: None,
            focus_search: false,
            feed: List::default(),
            library_tab: LibraryTab::History,
            history: List::default(),
            saved: List::default(),
            subs: List::default(),
            now: None,
            audio_only: false,
            caps,
            theater: false,
            thumbs: HashMap::new(),
            downloads: BTreeMap::new(),
            settings: None,
            remote: RemoteConfig::default(),
            status: None,
            update: None,
            auth: AuthUi::default(),
            toasts: Vec::new(),
            files_tx,
            files_rx,
            last_throughput: Instant::now(),
            theme_applied: false,
            video_px_height: 720,
            prefetched: std::collections::HashSet::new(),
            applied_scale: 1.0,
            actions: HashMap::new(),
            store,
            queue: Vec::new(),
            home_chip: 0,
            search_filter: Default::default(),
            dialog: None,
            sleep: None,
            watch_tab: WatchTab::UpNext,
            history_actions: Vec::new(),
            subs_state: HashMap::new(),
            music: Default::default(),
            video_audio_only: false,
        };
        app.player.set_loop(app.store.saved.loop_video);
        let req = app.req();
        app.home.loading = Some(req);
        app.core.send(Command::Home { req });
        app
    }

    pub(crate) fn req(&mut self) -> ReqId {
        let r = self.next_req;
        self.next_req += 1;
        r
    }

    pub(crate) fn toast(&mut self, msg: impl Into<String>) {
        self.toasts.push((msg.into(), Instant::now()));
    }

    // ---------------------------------------------------------------- navigation

    pub(crate) fn set_tab(&mut self, tab: Tab) {
        if self.tab == tab && self.stack.is_empty() {
            return;
        }
        self.tab = tab;
        self.drop_stack();
        if self.store.saved.music_mode {
            if tab == Tab::Search {
                self.focus_search = true;
            }
            self.music_tab_shown();
            return;
        }
        let signed_in = self.auth.method.is_some();
        match tab {
            Tab::Subscriptions if signed_in && !self.feed.loaded_once && self.feed.loading.is_none() => {
                let req = self.req();
                self.feed.loading = Some(req);
                self.core.send(Command::Feed { req });
            }
            Tab::Library if signed_in => self.load_library(),
            Tab::Search => self.focus_search = true,
            _ => {}
        }
    }

    pub(crate) fn load_library(&mut self) {
        let lt = self.library_tab;
        let req = self.req();
        let (list, cmd) = match lt {
            LibraryTab::History => (&mut self.history, Command::History { req }),
            LibraryTab::Playlists => (&mut self.saved, Command::SavedPlaylists { req }),
            LibraryTab::Channels => (&mut self.subs, Command::Subscriptions { req }),
        };
        if list.loading.is_none() && !list.loaded_once {
            list.loading = Some(req);
            self.core.send(cmd);
        }
    }

    fn drop_stack(&mut self) {
        for mut p in std::mem::take(&mut self.stack) {
            for l in p.lists() {
                if let Some(c) = l.next.take() {
                    self.core.send(Command::DropCursor(c));
                }
            }
        }
    }

    pub(crate) fn back(&mut self) -> bool {
        if let Some(now) = &mut self.now {
            if now.expanded && self.form == FormFactor::Phone {
                now.expanded = false;
                return true;
            }
        }
        match self.stack.pop() {
            Some(mut p) => {
                for l in p.lists() {
                    if let Some(c) = l.next.take() {
                        self.core.send(Command::DropCursor(c));
                    }
                }
                true
            }
            None if self.tab != Tab::Home => {
                self.tab = Tab::Home;
                true
            }
            None => false,
        }
    }

    pub(crate) fn open_channel(&mut self, id: String) {
        let req = self.req();
        self.core.send(Command::Channel { req, id: id.clone() });
        if self.signed_in() {
            self.load_subscription(id.clone());
        }
        self.stack.push(Page::Channel {
            id,
            req,
            header: None,
            videos: List {
                loading: Some(req),
                ..Default::default()
            },
            error: None,
            tab: ChannelTab::Videos,
            tab_list: List::default(),
            about: None,
        });
        if let Some(n) = &mut self.now {
            n.expanded = false;
        }
    }

    pub(crate) fn open_playlist(&mut self, id: String) {
        let req = self.req();
        self.core.send(Command::Playlist { req, id: id.clone() });
        self.stack.push(Page::Playlist {
            id,
            req,
            header: None,
            videos: List {
                loading: Some(req),
                ..Default::default()
            },
            error: None,
        });
    }

    pub(crate) fn open_item(&mut self, item: &Item) {
        match item {
            Item::Video(v) => self.play(v.clone()),
            Item::Channel(c) => self.open_channel(c.id.clone()),
            Item::Playlist(p) => self.open_playlist(p.id.clone()),
            Item::Post(p) => {
                if let Some(v) = &p.video {
                    self.play(v.clone())
                }
            }
        }
    }

    /// Load Home again (after signing in or out, so it matches the account).
    pub(crate) fn reload_home(&mut self) {
        let req = self.req();
        if let Some(c) = self.home.next.take() {
            self.core.send(Command::DropCursor(c));
        }
        self.home = List {
            loading: Some(req),
            ..Default::default()
        };
        self.core.send(Command::Home { req });
    }

    pub(crate) fn submit_search(&mut self) {
        let q = self.search_text.trim().to_string();
        if q.is_empty() {
            return;
        }
        self.tab = Tab::Search;
        self.drop_stack();
        if let Some(c) = self.search.next.take() {
            self.core.send(Command::DropCursor(c));
        }
        let req = self.req();
        self.search = List {
            loading: Some(req),
            ..Default::default()
        };
        self.search_submitted = q.clone();
        self.suggestions.clear();
        self.suggest_due = None;
        self.store.searched(&q);
        if self.store.saved.music_mode {
            self.search.loading = None;
            self.music_search(q);
            return;
        }
        let filter = self.search_filter.clone();
        self.core.send(Command::Search { req, query: q, filter });
    }

    pub(crate) fn search_text_changed(&mut self) {
        let q = self.search_text.trim().to_string();
        if q.is_empty() {
            self.suggestions.clear();
            self.suggest_due = None;
        } else {
            // Debounce suggestion requests by 200 ms.
            self.suggest_due = Some((Instant::now() + Duration::from_millis(200), q));
        }
    }

    pub(crate) fn load_more(&mut self, which: ListRef) {
        let req = self.req();
        let Some(list) = self.list_mut(which) else {
            return;
        };
        if list.more.is_some() || list.loading.is_some() {
            return;
        }
        if let Some(cursor) = list.next.take() {
            list.more = Some(req);
            self.core.send(Command::More { req, cursor });
        }
    }

    pub(crate) fn retry(&mut self, which: ListRef) {
        let req = self.req();
        let cmd = match which {
            ListRef::Home => Command::Home { req },
            ListRef::Search => Command::Search {
                req,
                filter: self.search_filter.clone(),
                query: self.search_submitted.clone(),
            },
            ListRef::Feed => Command::Feed { req },
            ListRef::History => Command::History { req },
            ListRef::Saved => Command::SavedPlaylists { req },
            ListRef::Subs => Command::Subscriptions { req },
            ListRef::Related => {
                if let Some(n) = &self.now {
                    let id = n.card.id.clone();
                    self.play_id_refresh(id);
                }
                return;
            }
            ListRef::Stack => {
                match self.stack.pop() {
                    Some(Page::Channel { id, .. }) => self.open_channel(id),
                    Some(Page::Playlist { id, .. }) => self.open_playlist(id),
                    Some(Page::Notifications { .. }) => self.open_notifications(),
                    Some(Page::Music(v)) => self.open_music_page(v.id, v.params, v.title),
                    None => {}
                }
                return;
            }
        };
        if let Some(l) = self.list_mut(which) {
            *l = List {
                loading: Some(req),
                ..Default::default()
            };
        }
        self.core.send(cmd);
    }

    pub(crate) fn list_mut(&mut self, which: ListRef) -> Option<&mut List> {
        Some(match which {
            ListRef::Home => &mut self.home,
            ListRef::Search => &mut self.search,
            ListRef::Feed => &mut self.feed,
            ListRef::History => &mut self.history,
            ListRef::Saved => &mut self.saved,
            ListRef::Subs => &mut self.subs,
            ListRef::Related => &mut self.now.as_mut()?.related,
            ListRef::Stack => self.stack.last_mut()?.shown()?,
        })
    }

    // ---------------------------------------------------------------- playback

    pub(crate) fn play(&mut self, card: VideoCard) {
        self.play_card(card, false);
    }

    /// Start a video, or a YouTube Music song (`music`).
    pub(crate) fn play_card(&mut self, card: VideoCard, music: bool) {
        if let Some(n) = &self.now {
            let st = self.player.state();
            if st.duration_s > 0.0 && n.info.as_ref().is_some_and(|i| i.hls.is_none()) {
                self.store.played(&n.card.id, st.position_s, st.duration_s);
            }
        }
        // Songs follow the Song/Video switch; videos the user's Audio only choice.
        self.audio_only = if music { !self.store.saved.music_video } else { self.video_audio_only };
        // Keep the tab under the player where it still applies (Comments, Lyrics, Related).
        let keep = if music {
            matches!(self.watch_tab, WatchTab::Lyrics | WatchTab::Related)
        } else {
            self.watch_tab == WatchTab::Comments
        };
        if !keep {
            self.watch_tab = WatchTab::UpNext;
        }
        if let Some(old) = self.now.take() {
            if let Some(c) = old.related.next {
                self.core.send(Command::DropCursor(c));
            }
        }
        self.player.stop();
        let resolve = self.req();
        let details = self.req();
        self.core.send(Command::Resolve {
            req: resolve,
            id: card.id.clone(),
            player_px_height: self.video_px_height,
            after_403: false,
        });
        self.core.send(Command::Video {
            req: details,
            id: card.id.clone(),
        });
        self.now = Some(NowPlaying {
            card,
            resolve_req: Some(resolve),
            details_req: Some(details),
            info: None,
            details: None,
            related: List {
                loading: Some(details),
                ..Default::default()
            },
            quality: None,
            error: None,
            retried_403: false,
            resume_at: None,
            expanded: self.form == FormFactor::Phone,
            awaiting_upgrade: None,
            prefetched_next: false,
            requested_at: Instant::now(),
            refused: Default::default(),
            music,
            extras: None,
            caption: None,
            cues: Vec::new(),
            chat: Chat::default(),
            comments: Comments::default(),
            rating: None,
            audio_track: None,
            audio_orig: (None, None),
            reported_at: None,
            saved_at: Instant::now(),
            lyrics_id: None,
            related_id: None,
            lyrics: None,
            music_related: None,
        });
        let id = self.now.as_ref().map(|n| n.card.id.clone()).unwrap_or_default();
        let resume = if music { None } else { self.store.resume_at(&id) };
        if let Some(n) = &mut self.now {
            n.resume_at = resume;
        }
        self.load_extras(id);
        if self.watch_tab == WatchTab::Comments {
            self.load_comments();
        }
    }

    fn play_id_refresh(&mut self, id: String) {
        let req = self.req();
        if let Some(n) = &mut self.now {
            n.details_req = Some(req);
            n.related = List {
                loading: Some(req),
                ..Default::default()
            };
        }
        self.core.send(Command::Video { req, id });
    }

    /// Re-resolve stream URLs and continue at the current position.
    pub(crate) fn reresolve(&mut self, after_403: bool) {
        let pos = self.player.state().position_s;
        let req = self.req();
        let px = self.video_px_height;
        if let Some(n) = &mut self.now {
            n.resolve_req = Some(req);
            n.resume_at = n.resume_at.or(Some(pos));
            n.requested_at = Instant::now();
            self.core.send(Command::Resolve {
                req,
                id: n.card.id.clone(),
                player_px_height: px,
                after_403,
            });
        }
    }

    /// Switch quality or audio-only, resuming at the saved position.
    pub(crate) fn reload_stream(&mut self) {
        let pos = self.player.state().position_s;
        let audio_only = self.audio_only;
        let Some(n) = &mut self.now else { return };
        let Some(info) = &n.info else { return };
        if info.expired() {
            self.reresolve(false);
            return;
        }
        let req = LoadRequest::from_info(info, n.quality, audio_only, pos);
        self.player.load(req);
    }

    pub(crate) fn download(&mut self, kind: streampipe_core::download::DownloadKind) {
        if let Some(n) = &self.now {
            self.core.send(Command::Download(DownloadRequest {
                video_id: n.card.id.clone(),
                title: n
                    .details
                    .as_ref()
                    .map(|d| d.video.title.clone())
                    .unwrap_or_else(|| n.card.title.clone()),
                kind,
                group: None,
            }));
            self.toast("Added to downloads");
        }
    }

    /// Resolve a video in the background so it starts quickly if opened.
    pub(crate) fn prefetch(&mut self, id: &str) {
        if self.settings.as_ref().is_some_and(|s| s.preload) && self.prefetched.insert(id.to_string()) {
            self.core.send(Command::PrefetchStreams { ids: vec![id.to_string()] });
        }
    }

    /// Resolve several videos ahead (the first rows of a list that just loaded).
    pub(crate) fn prefetch_many(&mut self, ids: Vec<String>) {
        if !self.settings.as_ref().is_some_and(|s| s.preload) {
            return;
        }
        let ids: Vec<String> = ids.into_iter().filter(|id| self.prefetched.insert(id.clone())).collect();
        if !ids.is_empty() {
            self.core.send(Command::PrefetchStreams { ids });
        }
    }

    pub(crate) fn want_thumb(&mut self, url: &str) {
        if !self.thumbs.contains_key(url) {
            self.thumbs.insert(url.to_string(), Thumb::Loading);
            self.core.send(Command::Thumbnail {
                url: url.to_string(),
            });
        }
    }

    pub(crate) fn update_settings(&mut self, f: impl FnOnce(&mut Settings)) {
        if let Some(s) = &mut self.settings {
            f(s);
            self.core.send(Command::UpdateSettings(s.clone()));
        }
    }

    pub(crate) fn browser_sign_in(&mut self, ctx: &egui::Context) {
        let tx = self.files_tx.clone();
        let ctx = ctx.clone();
        self.auth.waiting = true;
        self.auth.browser_open = true;
        self.auth.error = None;
        self.platform.browser_sign_in(Box::new(move |r| {
            let _ = tx.send(FileResult::BrowserSignIn(r));
            ctx.request_repaint();
        }));
    }

    pub(crate) fn file_dialog(&self, kind: FileDialog, ctx: &egui::Context) {
        let tx = self.files_tx.clone();
        let ctx = ctx.clone();
        self.platform.file_dialog(
            kind,
            Box::new(move |p| {
                let _ = tx.send(match kind {
                    FileDialog::OpenCookies => FileResult::Cookies(p),
                    FileDialog::PickFolder => FileResult::Folder(p),
                    FileDialog::SaveDiagnostics => FileResult::Diagnostics(p),
                });
                ctx.request_repaint();
            }),
        );
    }

    // ---------------------------------------------------------------- events

    fn route_page(&mut self, req: ReqId, result: Result<streampipe_core::model::Page, String>) {
        let mut lists: Vec<&mut List> = vec![
            &mut self.home,
            &mut self.search,
            &mut self.feed,
            &mut self.history,
            &mut self.saved,
            &mut self.subs,
        ];
        if let Some(n) = &mut self.now {
            lists.push(&mut n.related);
        }
        for p in &mut self.stack {
            lists.extend(p.lists());
        }
        let mut first_videos: Vec<String> = Vec::new();
        for l in lists {
            if l.loading == Some(req) {
                l.loading = None;
                l.loaded_once = true;
                match result {
                    Ok(p) => {
                        l.items = p.items;
                        l.next = p.next;
                        l.error = None;
                        first_videos = l
                            .items
                            .iter()
                            .filter_map(|i| match i {
                                Item::Video(v) if !v.is_live => Some(v.id.clone()),
                                _ => None,
                            })
                            .take(PRELOAD_FIRST)
                            .collect();
                    }
                    Err(e) => l.error = Some(e),
                }
                self.prefetch_many(first_videos);
                return;
            }
            if l.more == Some(req) {
                l.more = None;
                match result {
                    Ok(p) => {
                        l.items.extend(p.items);
                        l.next = p.next;
                    }
                    Err(e) => l.error = Some(e),
                }
                return;
            }
        }
        // No list waits for it any more (the user navigated away): free its cursor.
        if let Ok(p) = result {
            if let Some(c) = p.next {
                self.core.send(Command::DropCursor(c));
            }
        }
    }

    fn handle_event(&mut self, ctx: &egui::Context, e: Event) {
        match e {
            Event::Ready(s) => self.status = Some(s),
            Event::Page { req, result } => self.route_page(req, result),
            Event::Suggestions { req, items } => {
                if self.suggest_req == Some(req) {
                    self.suggestions = items.into_iter().take(8).collect();
                }
            }
            Event::Channel { req, result } => {
                for p in &mut self.stack {
                    if let Page::Channel {
                        req: r,
                        header,
                        videos,
                        error,
                        ..
                    } = p
                    {
                        if *r == req {
                            videos.loading = None;
                            videos.loaded_once = true;
                            match result {
                                Ok(mut c) => {
                                    videos.items = std::mem::take(&mut c.videos.items);
                                    videos.next = c.videos.next.take();
                                    *header = Some(c);
                                }
                                Err(e) => *error = Some(e),
                            }
                            return;
                        }
                    }
                }
            }
            Event::Playlist { req, result } => {
                for p in &mut self.stack {
                    if let Page::Playlist {
                        req: r,
                        header,
                        videos,
                        error,
                        ..
                    } = p
                    {
                        if *r == req {
                            videos.loading = None;
                            videos.loaded_once = true;
                            match result {
                                Ok(mut pl) => {
                                    videos.items = std::mem::take(&mut pl.videos.items);
                                    videos.next = pl.videos.next.take();
                                    *header = Some(pl);
                                }
                                Err(e) => *error = Some(e),
                            }
                            return;
                        }
                    }
                }
            }
            Event::Video { req, result } => {
                if let Some(n) = &mut self.now {
                    if n.details_req == Some(req) {
                        n.details_req = None;
                        n.related.loading = None;
                        n.related.loaded_once = true;
                        match result {
                            Ok(mut d) => {
                                n.related.items = std::mem::take(&mut d.related.items);
                                n.related.next = d.related.next.take();
                                if n.card.title.is_empty() {
                                    n.card.title = d.video.title.clone();
                                }
                                if n.card.channel_id.is_none() {
                                    n.card.channel_id = d.video.channel_id.clone();
                                    n.card.channel_name = d.video.channel_name.clone();
                                }
                                n.details = Some(d);
                                n.prefetched_next = true;
                                let next: Vec<String> = n
                                    .related
                                    .items
                                    .iter()
                                    .filter_map(|i| match i {
                                        Item::Video(v) if !v.is_live => Some(v.id.clone()),
                                        _ => None,
                                    })
                                    .take(2)
                                    .collect();
                                let queued = self.queue.first().map(|q| q.card.id.clone());
                                self.prefetch_many(queued.into_iter().chain(next).collect());
                                self.load_account_state();
                            }
                            Err(e) => n.related.error = Some(e),
                        }
                    }
                }
            }
            Event::Playback { req, result } => {
                let audio_only = self.audio_only;
                let stable = self.store.saved.stable_volume;
                if let Some(n) = &mut self.now {
                    if n.resolve_req == Some(req) {
                        n.resolve_req = None;
                        match result {
                            Ok(mut info) => {
                                n.audio_orig = (info.audio_aac.clone(), info.audio_opus.clone());
                                let level = apply_audio(&mut info, &n.audio_orig, n.audio_track, stable);
                                self.player.set_leveller(level);
                                // Keep the user's manual quality across re-resolves.
                                let keep = n
                                    .quality
                                    .and_then(|i| n.info.as_ref()?.qualities.get(i))
                                    .map(|q| (q.height, q.fps));
                                n.quality = keep
                                    .and_then(|(h, f)| {
                                        info.qualities.iter().position(|q| q.height == h && q.fps == f)
                                    })
                                    .or(info.selected);
                                let start = n.resume_at.take().unwrap_or(0.0);
                                let load = LoadRequest::from_info(&info, n.quality, audio_only, start);
                                n.info = Some(info);
                                n.error = None;
                                n.awaiting_upgrade = None;
                                self.player.load(load);
                                if let Some(s) = &self.settings {
                                    self.player.set_speed(s.speed);
                                }
                                self.platform.background_playback(true);
                            }
                            Err(e) => n.error = Some(e),
                        }
                    }
                }
            }
            Event::PlaybackUpgrade { video_id, mut info } => {
                let audio_only = self.audio_only;
                let stable = self.store.saved.stable_volume;
                let Some(n) = &mut self.now else { return };
                if n.card.id != video_id {
                    return;
                }
                n.audio_orig = (info.audio_aac.clone(), info.audio_opus.clone());
                let level = apply_audio(&mut info, &n.audio_orig, n.audio_track, stable);
                self.player.set_leveller(level);
                let keep = n
                    .quality
                    .and_then(|i| n.info.as_ref()?.qualities.get(i))
                    .map(|q| (q.height, q.fps));
                let mapped = keep.and_then(|(h, f)| {
                    info.qualities.iter().position(|q| q.height == h && q.fps == f)
                });
                // The direct resolver's quick URLs were a preview (they stop after about a
                // minute). As on the phone: the proxy keeps the local addresses and fetches from
                // the full streams, so the player only restarts if its addresses changed.
                let preview = n.info.as_ref().is_some_and(|i| i.client.starts_with("ytp-") || i.client.starts_with("direct-"));
                let playing = n.error.is_none() && n.awaiting_upgrade.is_none() && n.info.is_some();
                if preview && playing {
                    let pos = self.player.state().position_s;
                    let before = n.info.as_ref().map(|i| LoadRequest::from_info(i, n.quality, audio_only, pos));
                    n.quality = mapped.or(info.selected);
                    let after = LoadRequest::from_info(&info, n.quality, audio_only, pos);
                    let same = before.is_some_and(|b| b.video_url == after.video_url && b.audio_url == after.audio_url);
                    n.info = Some(info);
                    if !same {
                        self.player.load(after);
                    }
                } else if n.awaiting_upgrade.is_some() || n.error.is_some() || n.info.is_none() {
                    // Continue on the new streams where playback was.
                    n.quality = mapped.or(info.selected);
                    let start = n.resume_at.take().unwrap_or(0.0);
                    let load = LoadRequest::from_info(&info, n.quality, audio_only, start);
                    n.info = Some(info);
                    n.error = None;
                    n.awaiting_upgrade = None;
                    self.player.load(load);
                } else {
                    // Already playing: keep going, but offer the fuller quality list and use
                    // these streams for any later switch or retry.
                    n.quality = mapped.or(n.quality.and(info.selected));
                    n.info = Some(info);
                }
            }
            Event::Thumbnail { url, image } => {
                let img = egui::ColorImage::from_rgba_unmultiplied(
                    [image.width as usize, image.height as usize],
                    &image.pixels,
                );
                let tex = ctx.load_texture(&url, img, egui::TextureOptions::LINEAR);
                self.thumbs.insert(url, Thumb::Ready(tex));
            }
            Event::ThumbnailFailed { url } => {
                self.thumbs.insert(url, Thumb::Failed);
            }
            Event::Auth(a) => match a {
                AuthEvent::DeviceCode {
                    verification_url,
                    user_code,
                    ..
                } => {
                    self.auth.device_code = Some((verification_url, user_code));
                    self.auth.waiting = true;
                    self.auth.error = None;
                }
                AuthEvent::SignedIn(m) => {
                    let was = self.auth.method;
                    self.auth = AuthUi {
                        method: Some(m),
                        ..Default::default()
                    };
                    if was.is_none() {
                        self.toast("Signed in");
                        self.reload_home();
                        self.home_chip = 0;
                        self.feed = List::default();
                        self.history = List::default();
                        self.saved = List::default();
                        self.subs = List::default();
                    }
                }
                AuthEvent::SignedOut => {
                    if self.auth.method.is_some() {
                        self.reload_home();
                    }
                    self.auth.method = None;
                    self.auth.waiting = false;
                    self.auth.device_code = None;
                }
                AuthEvent::Denied => {
                    self.auth.waiting = false;
                    self.auth.device_code = None;
                    self.auth.error = Some("Sign-in was declined on the other device.".into());
                }
                AuthEvent::Failed(e) => {
                    self.auth.waiting = false;
                    self.auth.browser_open = false;
                    self.auth.checking = false;
                    self.auth.device_code = None;
                    self.auth.error = Some(e);
                }
            },
            Event::Download(d) => match d {
                DownloadEvent::Added { id, req, status } => {
                    self.downloads.insert(
                        id,
                        DlRow {
                            req,
                            status,
                            done: 0,
                            total: 0,
                            bps: 0,
                        },
                    );
                }
                DownloadEvent::Status { id, status } => {
                    if let Some(r) = self.downloads.get_mut(&id) {
                        if let JobStatus::Done(p) = &status {
                            let name = p.file_name().map(|n| n.to_string_lossy().into_owned());
                            self.toasts.push((
                                format!("Downloaded {}", name.unwrap_or_default()),
                                Instant::now(),
                            ));
                        }
                        if let JobStatus::Failed(e) = &status {
                            if e.contains("FFmpeg") {
                                self.toasts.push((e.clone(), Instant::now()));
                            }
                        }
                        r.status = status;
                    }
                }
                DownloadEvent::Progress {
                    id,
                    done,
                    total,
                    bytes_per_s,
                } => {
                    if let Some(r) = self.downloads.get_mut(&id) {
                        r.done = done;
                        r.total = total;
                        r.bps = bytes_per_s;
                    }
                }
                DownloadEvent::Removed { id } => {
                    self.downloads.remove(&id);
                }
            },
            Event::Settings(s) => {
                if self.settings.is_none() {
                    self.audio_only = s.audio_only;
                    self.video_audio_only = s.audio_only;
                }
                motion::set_reduced(
                    s.reduced_motion
                        .unwrap_or_else(|| self.platform.os_reduced_motion()),
                );
                self.player.set_volume(s.volume);
                self.player.set_speed(s.speed);
                let codec_changed = self.settings.as_ref().is_some_and(|old| old.codec != s.codec);
                if (s.ui_scale - self.applied_scale).abs() > 0.001 {
                    ctx.set_zoom_factor(s.ui_scale);
                    self.applied_scale = s.ui_scale;
                }
                self.settings = Some(s);
                if codec_changed {
                    self.rebuild_qualities(true);
                }
            }
            Event::Remote(r) => self.remote = r,
            Event::UpdateAvailable(u) => self.update = Some(u),
            Event::Toast(t) => self.toast(t),
            Event::Comments { req, result } => self.on_comments(req, result),
            // Direct streams are used by the Android app only.
            Event::Streams { .. } => {}
            Event::Action { req, result } => self.on_action(req, result),
            Event::StreamsUpdate { .. } => {}
        }
    }

    fn handle_player_events(&mut self) {
        for e in self.player.poll_events() {
            match e {
                PlayerEvent::Ended => {
                    if let Some(n) = &self.now {
                        // Finished: start from the top next time.
                        self.store.saved.resume.remove(&n.card.id);
                        self.store.touch();
                    }
                    if matches!(self.sleep, Some(Sleep::EndOfVideo)) {
                        self.sleep = None;
                        self.toast("Sleep timer: stopped at the end of the video");
                        self.platform.background_playback(false);
                        continue;
                    }
                    if !self.queue.is_empty() {
                        let q = self.queue.remove(0);
                        self.play_card(q.card, q.music);
                        continue;
                    }
                    if self.now.as_ref().is_some_and(|n| n.music) {
                        self.platform.background_playback(false);
                        continue;
                    }
                    let autoplay = self.settings.as_ref().is_some_and(|s| s.autoplay);
                    let next = self.now.as_ref().and_then(|n| {
                        n.related.items.iter().find_map(|i| match i {
                            Item::Video(v) if !v.is_live => Some(v.clone()),
                            _ => None,
                        })
                    });
                    match (autoplay, next) {
                        (true, Some(v)) => self.play(v),
                        _ => self.platform.background_playback(false),
                    }
                }
                PlayerEvent::Error { forbidden, message } => {
                    let pos = self.player.state().position_s;
                    let Some(n) = &mut self.now else { continue };
                    let expired = n.info.as_ref().is_some_and(|i| i.expired());
                    // A refused format: try the next quality down from the streams we already
                    // have before asking for fresh URLs.
                    // Step down at most once: a second refusal suggests the whole result is
                    // affected, and a fresh full resolve is the better move.
                    if forbidden && !expired && n.refused.is_empty() {
                        if let Some(info) = &mut n.info {
                            let failed = n.quality.and_then(|i| info.qualities.get(i));
                            let failed_height = failed.map(|q| q.height);
                            if let Some(q) = failed {
                                n.refused.insert(q.stream.itag);
                            }
                            let refused = n.refused.clone();
                            info.all_video.retain(|s| !refused.contains(&s.itag));
                            // Rebuild from every remaining stream, so the same height in another
                            // codec can stand in for the refused one.
                            let pref = self
                                .settings
                                .as_ref()
                                .map(|s| s.codec)
                                .unwrap_or(streampipe_core::config::CodecPref::Auto);
                            let caps = streampipe_core::streams::apply_codec_pref(self.caps, pref);
                            info.qualities = if info.all_video.is_empty() {
                                info.qualities.iter().filter(|q| !refused.contains(&q.stream.itag)).cloned().collect()
                            } else {
                                streampipe_core::streams::qualities_from(&info.all_video, &caps)
                            };
                            // Best remaining quality at or below the one that failed.
                            let next = info
                                .qualities
                                .iter()
                                .position(|q| failed_height.is_none_or(|h| q.height <= h));
                            if let Some(i) = next {
                                n.quality = Some(i);
                                n.resume_at = Some(pos);
                                let load = LoadRequest::from_info(info, n.quality, self.audio_only, pos);
                                self.player.load(load);
                                continue;
                            }
                        }
                    }
                    if (forbidden || expired) && !n.retried_403 {
                        // Fresh URLs, continuing where playback stopped.
                        n.retried_403 = true;
                        n.resume_at = Some(pos);
                        self.reresolve(true);
                    } else {
                        n.error = Some(message);
                        n.awaiting_upgrade = None;
                    }
                }
                PlayerEvent::Decoder { codec, hardware } => {
                    if !hardware {
                        let before = self.caps;
                        match codec.as_str() {
                            "vp9" => self.caps.vp9 = false,
                            "av1" => self.caps.av1 = false,
                            _ => {}
                        }
                        if before != self.caps {
                            // Software decode of VP9/AV1 is costly: drop those rungs and switch
                            // to the best remaining one at the current position.
                            self.core.send(Command::SetCaps(self.caps));
                            self.rebuild_qualities(true);
                        }
                    }
                }
            }
        }
    }

    /// Recompute the quality list from every known stream (after codec support or the codec
    /// preference changed), keeping the current height where possible.
    pub(crate) fn rebuild_qualities(&mut self, reload: bool) {
        let pref = self
            .settings
            .as_ref()
            .map(|s| s.codec)
            .unwrap_or(streampipe_core::config::CodecPref::Auto);
        let caps = streampipe_core::streams::apply_codec_pref(self.caps, pref);
        let Some(n) = &mut self.now else { return };
        let Some(info) = &mut n.info else { return };
        if info.all_video.is_empty() {
            return;
        }
        let keep = n.quality.and_then(|i| info.qualities.get(i)).map(|q| q.height);
        let before = n.quality.and_then(|i| info.qualities.get(i)).map(|q| q.stream.url.clone());
        info.qualities = streampipe_core::streams::qualities_from(&info.all_video, &caps);
        n.quality = keep
            .and_then(|h| info.qualities.iter().position(|q| q.height <= h))
            .or(if info.qualities.is_empty() { None } else { Some(0) });
        let after = n.quality.and_then(|i| info.qualities.get(i)).map(|q| q.stream.url.clone());
        if reload && before != after {
            self.reload_stream();
        }
    }

    fn keyboard(&mut self, ctx: &egui::Context) {
        let typing = ctx.egui_wants_keyboard_input();
        let input = |k: Key| ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, k));
        let cmd = |k: Key| ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, k));
        if cmd(Key::L) || cmd(Key::K) || (!typing && input(Key::Slash)) {
            self.focus_search = true;
        }
        if typing {
            return;
        }
        let st = self.player.state();
        if !st.idle {
            if input(Key::Space) || input(Key::K) {
                self.player.set_paused(!st.paused);
            }
            if input(Key::ArrowRight) || input(Key::L) {
                self.player.seek((st.position_s + 5.0).min(st.duration_s));
            }
            if input(Key::ArrowLeft) || input(Key::J) {
                self.player.seek((st.position_s - 5.0).max(0.0));
            }
            if input(Key::Plus) || input(Key::Equals) {
                let v = (st.volume + 5.0).min(130.0);
                self.update_settings(|s| s.volume = v);
                self.player.set_volume(v);
            }
            if input(Key::Minus) {
                let v = (st.volume - 5.0).max(0.0);
                self.update_settings(|s| s.volume = v);
                self.player.set_volume(v);
            }
            if input(Key::A) {
                self.toggle_audio_only();
            }
            if input(Key::C) {
                self.cycle_captions();
            }
            if input(Key::N) && !self.queue.is_empty() {
                let q = self.queue.remove(0);
                self.play_card(q.card, q.music);
            }
        }
        if input(Key::T) {
            self.theater = !self.theater;
        }
        if input(Key::F11) {
            let full = ctx.input(|i| i.viewport().fullscreen.unwrap_or(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
        }
        if input(Key::Escape) {
            self.back();
        }
        if input(Key::Num1) {
            self.set_tab(Tab::Home);
        }
        if input(Key::Num2) {
            self.set_tab(Tab::Subscriptions);
        }
        if input(Key::Num3) {
            self.set_tab(Tab::Library);
        }
        if input(Key::Num4) {
            self.set_tab(Tab::Downloads);
        }
        if input(Key::Num5) {
            self.set_tab(Tab::Settings);
        }
    }

    /// Play a video from a YouTube URL or bare id (command-line argument, "open with").
    pub fn open_link(&mut self, link: &str) -> bool {
        let Some(id) = video_id_from_link(link) else {
            return false;
        };
        self.play(VideoCard {
            id: id.clone(),
            title: String::new(),
            channel_id: None,
            channel_name: None,
            channel_avatar: None,
            duration_s: None,
            thumbnail: Some(format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg")),
            views: None,
            published: None,
            is_live: false,
            watched: None,
        });
        true
    }

    /// Codec support learned so far (persist it between runs).
    pub fn hw_caps(&self) -> HwCaps {
        self.caps
    }

    pub fn player_mut(&mut self) -> &mut dyn Player {
        self.player.as_mut()
    }

    /// Per-frame bookkeeping. Call before [`App::ui`].
    pub fn logic(&mut self, ctx: &egui::Context) {
        if !self.theme_applied {
            theme::apply(ctx, self.form == FormFactor::Phone);
            motion::set_reduced(self.platform.os_reduced_motion());
            self.theme_applied = true;
        }
        while let Ok(e) = self.events.try_recv() {
            self.handle_event(ctx, e);
        }
        while let Ok(f) = self.files_rx.try_recv() {
            match f {
                FileResult::Cookies(Some(p)) => match std::fs::read_to_string(&p) {
                    Ok(txt) => {
                        self.auth.error = None;
                        self.core.send(Command::ImportCookies(txt));
                    }
                    Err(e) => self.auth.error = Some(format!("Could not read the file: {e}")),
                },
                FileResult::Folder(Some(p)) => self.update_settings(|s| s.download_dir = Some(p)),
                FileResult::Diagnostics(Some(p)) => self.core.send(Command::ExportDiagnostics(p)),
                FileResult::BrowserSignIn(r) => {
                    self.auth.browser_open = false;
                    match r {
                        // Still waiting: the core validates the cookies and reports back.
                        Ok(txt) => {
                            self.auth.checking = true;
                            self.core.send(Command::ImportCookies(txt));
                        }
                        Err(e) => {
                            self.auth.waiting = false;
                            self.auth.error = Some(e);
                        }
                    }
                }
                _ => {}
            }
        }
        self.handle_player_events();

        if let Some((due, q)) = &self.suggest_due {
            let now = Instant::now();
            if now >= *due {
                let q = q.clone();
                self.suggest_due = None;
                let req = self.req();
                self.suggest_req = Some(req);
                if self.store.saved.music_mode {
                    self.core.send(Command::MusicSuggest { req, query: q });
                } else {
                    self.core.send(Command::Suggest { req, query: q });
                }
            } else {
                ctx.request_repaint_after(*due - now);
            }
        }

        // If the quick source failed and yt-dlp hasn't answered in 25 s, ask it directly.
        let stuck = self
            .now
            .as_ref()
            .and_then(|n| n.awaiting_upgrade)
            .is_some_and(|t| t.elapsed() > Duration::from_secs(25));
        if stuck {
            if let Some(n) = &mut self.now {
                n.awaiting_upgrade = None;
                n.retried_403 = true;
            }
            self.reresolve(true);
        } else if self.now.as_ref().is_some_and(|n| n.awaiting_upgrade.is_some()) {
            ctx.request_repaint_after(Duration::from_secs(1));
        }

        // Pre-load the first "Up next" video once this one is playing.
        let preload = self.settings.as_ref().is_some_and(|s| s.preload);
        if preload {
            let next = self.now.as_ref().filter(|n| !n.prefetched_next && n.info.is_some()).and_then(|n| {
                n.related.items.iter().find_map(|i| match i {
                    Item::Video(v) if !v.is_live => Some(v.id.clone()),
                    _ => None,
                })
            });
            if let Some(id) = next {
                self.prefetch(&id);
                if let Some(n) = &mut self.now {
                    n.prefetched_next = true;
                }
            }
        }

        // Feed measured throughput to auto quality every 5 s of playback.
        if self.last_throughput.elapsed() > Duration::from_secs(5) {
            self.last_throughput = Instant::now();
            if let Some(bps) = self.player.throughput_bps() {
                self.core.send(Command::Throughput(bps));
            }
        }

        self.tick(ctx);

        self.toasts
            .retain(|(_, t)| t.elapsed() < Duration::from_secs(4));
        if let Some((_, t)) = self.toasts.first() {
            ctx.request_repaint_after(Duration::from_secs(4).saturating_sub(t.elapsed()));
        }

        self.keyboard(ctx);
    }

    /// Draw the whole interface into `ui`.
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        crate::views::root(self, ui);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ListRef {
    Home,
    Search,
    Feed,
    History,
    Saved,
    Subs,
    Related,
    Stack,
}

/// Extract an 11-character video id from common YouTube URL shapes or a bare id.
pub fn video_id_from_link(link: &str) -> Option<String> {
    let l = link.trim();
    let valid = |s: &str| s.len() == 11 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if valid(l) {
        return Some(l.to_string());
    }
    let candidates = [
        l.split("v=").nth(1),
        l.split("youtu.be/").nth(1),
        l.split("/shorts/").nth(1),
        l.split("/embed/").nth(1),
        l.split("/live/").nth(1),
    ];
    candidates
        .into_iter()
        .flatten()
        .map(|rest| rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect::<String>())
        .find(|id| valid(id))
}

#[cfg(test)]
mod tests {
    use super::video_id_from_link as f;

    #[test]
    fn parses_links() {
        assert_eq!(f("jNQXAC9IVRw").as_deref(), Some("jNQXAC9IVRw"));
        assert_eq!(f("https://www.youtube.com/watch?v=jNQXAC9IVRw&t=3").as_deref(), Some("jNQXAC9IVRw"));
        assert_eq!(f("https://youtu.be/jNQXAC9IVRw?si=x").as_deref(), Some("jNQXAC9IVRw"));
        assert_eq!(f("https://youtube.com/shorts/jNQXAC9IVRw").as_deref(), Some("jNQXAC9IVRw"));
        assert_eq!(f("hello"), None);
    }
}
