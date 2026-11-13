//! The YouTube-web features on top of the basic browser and player: captions, preview
//! pictures, chapters, live chat, comments, likes, subscriptions and the bell, playlists,
//! the queue, "Play on TV", resume positions, history reporting and the sleep timer.
//!
//! Most of these talk to the core with commands answered as JSON (`Event::Action`); each
//! request carries a callback in `App::actions`.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use serde_json::Value;
use streampipe_core::model::*;
use streampipe_core::service::ReqId;
use streampipe_core::streams::StreamChoice;
use streampipe_core::{Command, PlaybackInfo};

use crate::app::{App, List, Page, QueueItem};

pub(crate) type ActionFn = Box<dyn FnOnce(&mut App, Result<Value, String>)>;

// ------------------------------------------------------------------ types

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ChannelTab {
    Videos,
    Shorts,
    Live,
    Playlists,
    Posts,
    About,
}

impl ChannelTab {
    pub const ALL: [ChannelTab; 6] = [Self::Videos, Self::Shorts, Self::Live, Self::Playlists, Self::Posts, Self::About];

    pub fn label(self) -> &'static str {
        match self {
            Self::Videos => "Videos",
            Self::Shorts => "Shorts",
            Self::Live => "Live",
            Self::Playlists => "Playlists",
            Self::Posts => "Posts",
            Self::About => "About",
        }
    }
}

pub(crate) struct CaptionTrack {
    pub lang: String,
    pub name: String,
    pub auto: bool,
    pub url: String,
    pub agent: Option<String>,
}

/// YouTube's preview pictures: sheets of `cols`×`rows` frames, `interval_ms` apart.
pub(crate) struct Storyboard {
    url: String,
    count: u32,
    cols: u32,
    rows: u32,
    interval_ms: u32,
    pub aspect: f32,
}

impl Storyboard {
    /// "base|w#h#count#cols#rows#interval#name#sigh|…": one part per level; the largest is used.
    pub fn parse(spec: &str) -> Option<Self> {
        let parts: Vec<&str> = spec.split('|').collect();
        if parts.len() < 2 {
            return None;
        }
        let level = parts.len() - 2;
        let f: Vec<&str> = parts.last()?.split('#').collect();
        if f.len() < 8 {
            return None;
        }
        let n = |i: usize| f[i].parse::<u32>().ok();
        let (w, h) = (n(0)?, n(1)?);
        Some(Self {
            url: format!("{}&sigh={}", parts[0].replace("$L", &level.to_string()).replace("$N", f[6]), f[7]),
            count: n(2)?,
            cols: n(3)?.max(1),
            rows: n(4)?.max(1),
            interval_ms: n(5)?,
            aspect: if h > 0 { w as f32 / h as f32 } else { 16.0 / 9.0 },
        })
    }

    /// The sheet and the part of it that shows the frame at `at` seconds.
    pub fn frame(&self, at: f64, length: f64) -> Option<(String, egui::Rect)> {
        if self.count == 0 {
            return None;
        }
        let interval = if self.interval_ms > 0 {
            self.interval_ms as f64
        } else if length > 0.0 {
            (length * 1000.0 / self.count as f64).max(1.0)
        } else {
            return None;
        };
        let index = ((at * 1000.0 / interval) as u32).min(self.count - 1);
        let per = self.cols * self.rows;
        let (page, within) = (index / per, index % per);
        let (c, r) = (within % self.cols, within / self.cols);
        let (fw, fh) = (1.0 / self.cols as f32, 1.0 / self.rows as f32);
        let uv = egui::Rect::from_min_size(egui::pos2(c as f32 * fw, r as f32 * fh), egui::vec2(fw, fh));
        Some((self.url.replace("$M", &page.to_string()), uv))
    }
}

pub(crate) struct Cue {
    pub s: f64,
    pub e: f64,
    pub t: String,
}

pub(crate) struct ChatMsg {
    pub author: String,
    pub text: String,
    pub amount: Option<String>,
}

#[derive(Default)]
pub(crate) struct Chat {
    pub messages: VecDeque<ChatMsg>,
    pub next: Option<String>,
    pub due: Option<Instant>,
    pub busy: bool,
    pub ended: Option<String>,
}

#[derive(Default)]
pub(crate) struct Comments {
    pub items: Vec<CommentCard>,
    pub next: Option<Cursor>,
    pub req: Option<ReqId>,
    pub more: Option<ReqId>,
    pub error: Option<String>,
    pub loaded: bool,
}

#[derive(Default)]
pub(crate) struct Extras {
    pub captions: Vec<CaptionTrack>,
    pub storyboard: Option<Storyboard>,
    pub live: bool,
}

pub(crate) struct PlaylistChoice {
    pub id: String,
    pub title: String,
    pub contains: bool,
}

pub(crate) enum Dialog {
    SaveTo {
        video: String,
        lists: Option<Vec<PlaylistChoice>>,
        error: Option<String>,
        new_name: String,
        privacy: usize,
    },
    Tv {
        video: String,
        at: u32,
        devices: Option<Vec<(String, String)>>,
        error: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WatchTab {
    UpNext,
    Comments,
    Chapters,
    Chat,
    Lyrics,
    Related,
}

pub(crate) enum Sleep {
    At(Instant),
    EndOfVideo,
}

/// What a video's sound comes from: the chosen dubbed track, the levelled ("stable volume")
/// stream, or the usual one. Answers whether the player must level the sound itself.
pub(crate) fn apply_audio(
    info: &mut PlaybackInfo,
    orig: &(Option<StreamChoice>, Option<StreamChoice>),
    track: Option<usize>,
    stable: bool,
) -> bool {
    info.audio_aac = orig.0.clone();
    info.audio_opus = orig.1.clone();
    if let Some(t) = track.and_then(|i| info.audio_tracks.get(i)).filter(|t| !t.original) {
        info.audio_aac = Some(t.stream.clone());
        info.audio_opus = None;
        return stable;
    }
    if stable {
        if let Some(d) = &info.audio_drc {
            info.audio_aac = Some(d.clone());
            info.audio_opus = None;
            return false;
        }
        return true;
    }
    false
}

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

// ------------------------------------------------------------------ actions

impl App {
    /// Send a command answered as JSON and run `done` with the answer.
    pub(crate) fn call(
        &mut self,
        make: impl FnOnce(ReqId) -> Command,
        done: impl FnOnce(&mut App, Result<Value, String>) + 'static,
    ) {
        let req = self.req();
        self.actions.insert(req, Box::new(done));
        self.core.send(make(req));
    }

    pub(crate) fn on_action(&mut self, req: ReqId, result: Result<Value, String>) {
        if let Some(f) = self.actions.remove(&req) {
            f(self, result);
        }
    }

    pub(crate) fn signed_in(&self) -> bool {
        self.auth.method.is_some()
    }

    fn now_is(&self, id: &str) -> bool {
        self.now.as_ref().is_some_and(|n| n.card.id == id)
    }

    // -------------------------------------------------------------- watch page extras

    /// Captions, preview pictures and live chat of the video that just started.
    pub(crate) fn load_extras(&mut self, id: String) {
        let ask = id.clone();
        self.call(
            move |req| Command::Extras { req, id: ask },
            move |app, r| {
                let Ok(v) = r else { return };
                if !app.now_is(&id) {
                    return;
                }
                let mut ex = Extras::default();
                for t in v.get("captions").and_then(Value::as_array).into_iter().flatten() {
                    let url = s(t, "url");
                    if url.is_empty() {
                        continue;
                    }
                    ex.captions.push(CaptionTrack {
                        lang: s(t, "lang"),
                        name: s(t, "name"),
                        auto: t.get("auto").and_then(Value::as_bool).unwrap_or(false),
                        url,
                        agent: t.get("agent").and_then(Value::as_str).map(str::to_string),
                    });
                }
                ex.storyboard = v.get("storyboard").and_then(Value::as_str).and_then(Storyboard::parse);
                ex.live = v.get("live").and_then(Value::as_bool).unwrap_or(false);
                let chat = v.get("chat").and_then(Value::as_str).map(str::to_string);
                // The language chosen last time: a written track first, then an automatic one.
                let lang = app.store.saved.caption_lang.clone();
                let pick = if lang.is_empty() {
                    None
                } else {
                    ex.captions
                        .iter()
                        .position(|t| t.lang == lang && !t.auto)
                        .or_else(|| ex.captions.iter().position(|t| t.lang == lang))
                        .or_else(|| ex.captions.iter().position(|t| t.lang.split('-').next() == lang.split('-').next()))
                };
                if let Some(n) = &mut app.now {
                    n.extras = Some(ex);
                    if let Some(token) = chat {
                        n.chat.next = Some(token);
                        n.chat.due = Some(Instant::now());
                    }
                }
                if pick.is_some() {
                    app.choose_caption(pick, false);
                }
            },
        );
    }

    /// Show one caption track (or none).
    pub(crate) fn choose_caption(&mut self, track: Option<usize>, remember: bool) {
        let Some(n) = &mut self.now else { return };
        n.caption = track;
        n.cues.clear();
        let Some(t) = track.and_then(|i| n.extras.as_ref()?.captions.get(i)) else {
            if remember {
                self.store.saved.caption_lang.clear();
                self.store.touch();
            }
            return;
        };
        let (url, agent, lang, id) = (t.url.clone(), t.agent.clone(), t.lang.clone(), n.card.id.clone());
        if remember {
            self.store.saved.caption_lang = lang;
            self.store.touch();
        }
        self.call(
            move |req| Command::Captions { req, url, agent },
            move |app, r| {
                let Some(n) = app.now.as_mut().filter(|n| n.card.id == id && n.caption == track) else { return };
                match r {
                    Ok(v) => {
                        n.cues = v
                            .get("cues")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                            .filter_map(|c| {
                                Some(Cue {
                                    s: c.get("s")?.as_f64()? / 1000.0,
                                    e: c.get("e")?.as_f64()? / 1000.0,
                                    t: c.get("t")?.as_str()?.to_string(),
                                })
                            })
                            .collect();
                        // Some answers are in seconds already.
                        let longest = n.cues.iter().map(|c| c.e).fold(0.0, f64::max);
                        let dur = n.info.as_ref().map(|i| i.duration_s as f64).unwrap_or(0.0);
                        if dur > 0.0 && longest > 0.0 && longest < dur / 100.0 {
                            for c in &mut n.cues {
                                c.s *= 1000.0;
                                c.e *= 1000.0;
                            }
                        }
                    }
                    Err(e) => app.toast(format!("Captions: {e}")),
                }
            },
        );
    }

    /// The caption line(s) to show at `at` seconds.
    pub(crate) fn caption_at(&self, at: f64) -> Option<String> {
        let n = self.now.as_ref()?;
        n.caption?;
        let lines: Vec<&str> = n.cues.iter().filter(|c| c.s <= at && at < c.e).map(|c| c.t.trim()).filter(|t| !t.is_empty()).collect();
        (!lines.is_empty()).then(|| lines.join("\n"))
    }

    /// The signed-in user's rating of the video and the channel's subscribe state.
    pub(crate) fn load_account_state(&mut self) {
        if !self.signed_in() {
            return;
        }
        let Some(n) = &self.now else { return };
        let id = n.card.id.clone();
        let channel = n.details.as_ref().and_then(|d| d.video.channel_id.clone()).or_else(|| n.card.channel_id.clone());
        let ask = id.clone();
        self.call(
            move |req| Command::Rating { req, id: ask },
            move |app, r| {
                if let (Ok(v), Some(n)) = (r, app.now.as_mut().filter(|n| n.card.id == id)) {
                    n.rating = v.get("rating").and_then(Value::as_str).map(str::to_string);
                }
            },
        );
        if let Some(c) = channel {
            self.load_subscription(c);
        }
    }

    /// Subscribed or not, and the bell choices, for a channel (watch page and channel page).
    pub(crate) fn load_subscription(&mut self, channel: String) {
        let ask = channel.clone();
        self.call(
            move |req| Command::Subscription { req, id: ask },
            move |app, r| {
                let Ok(v) = r else { return };
                app.subs_state.insert(channel, v);
            },
        );
    }

    pub(crate) fn subscribe(&mut self, channel: String, on: bool) {
        if !self.signed_in() {
            self.auth.dialog_open = true;
            return;
        }
        if let Some(v) = self.subs_state.get_mut(&channel) {
            v["subscribed"] = Value::Bool(on);
        }
        let ask = channel.clone();
        self.call(
            move |req| Command::Subscribe { req, id: ask, on },
            move |app, r| match r {
                Ok(_) => {
                    app.toast(if on { "Subscribed" } else { "Unsubscribed" });
                    app.load_subscription(channel);
                }
                Err(e) => {
                    app.toast(format!("Could not change the subscription: {e}"));
                    app.load_subscription(channel);
                }
            },
        );
    }

    pub(crate) fn set_bell(&mut self, channel: String, params: String, label: String) {
        self.call(
            move |req| Command::SetBell { req, params },
            move |app, r| {
                match r {
                    Ok(_) => app.toast(format!("Notifications: {label}")),
                    Err(e) => app.toast(format!("Could not change notifications: {e}")),
                }
                app.load_subscription(channel);
            },
        );
    }

    pub(crate) fn rate(&mut self, rating: &str) {
        if !self.signed_in() {
            self.auth.dialog_open = true;
            return;
        }
        let Some(n) = &mut self.now else { return };
        let rating = if n.rating.as_deref() == Some(rating) { "none".to_string() } else { rating.to_string() };
        n.rating = Some(rating.clone());
        let id = n.card.id.clone();
        self.call(
            move |req| Command::Rate { req, id, rating },
            |app, r| {
                if let Err(e) = r {
                    app.toast(format!("Could not rate the video: {e}"));
                }
            },
        );
    }

    // -------------------------------------------------------------- comments

    pub(crate) fn load_comments(&mut self) {
        let req = self.req();
        let Some(n) = &mut self.now else { return };
        if n.comments.loaded || n.comments.req.is_some() {
            return;
        }
        n.comments.req = Some(req);
        let id = n.card.id.clone();
        self.core.send(Command::Comments { req, id });
    }

    pub(crate) fn more_comments(&mut self) {
        let req = self.req();
        let Some(n) = &mut self.now else { return };
        if n.comments.more.is_some() {
            return;
        }
        if let Some(cursor) = n.comments.next.take() {
            n.comments.more = Some(req);
            self.core.send(Command::MoreComments { req, cursor });
        }
    }

    pub(crate) fn on_comments(&mut self, req: ReqId, result: Result<CommentPage, String>) {
        let Some(n) = &mut self.now else { return };
        let c = &mut n.comments;
        if c.req == Some(req) {
            c.req = None;
            c.loaded = true;
            match result {
                Ok(p) => {
                    c.items = p.items;
                    c.next = p.next;
                }
                Err(e) => c.error = Some(e),
            }
        } else if c.more == Some(req) {
            c.more = None;
            match result {
                Ok(p) => {
                    c.items.extend(p.items);
                    c.next = p.next;
                }
                Err(e) => c.error = Some(e),
            }
        }
    }

    // -------------------------------------------------------------- playlists

    pub(crate) fn open_save(&mut self, video: String) {
        if !self.signed_in() {
            self.auth.dialog_open = true;
            return;
        }
        self.dialog = Some(Dialog::SaveTo {
            video: video.clone(),
            lists: None,
            error: None,
            new_name: String::new(),
            privacy: 0,
        });
        let ask = video.clone();
        self.call(
            move |req| Command::PlaylistOptions { req, id: ask },
            move |app, r| {
                if let Some(Dialog::SaveTo { video: v, lists, error, .. }) = &mut app.dialog {
                    if *v != video {
                        return;
                    }
                    match r {
                        Ok(v) => {
                            *lists = Some(
                                v.get("playlists")
                                    .and_then(Value::as_array)
                                    .into_iter()
                                    .flatten()
                                    .map(|p| PlaylistChoice {
                                        id: s(p, "id"),
                                        title: s(p, "title"),
                                        contains: p.get("contains").and_then(Value::as_bool).unwrap_or(false),
                                    })
                                    .collect(),
                            )
                        }
                        Err(e) => *error = Some(e),
                    }
                }
            },
        );
    }

    pub(crate) fn edit_playlist(&mut self, playlist: String, video: String, add: bool, title: String) {
        self.call(
            move |req| Command::EditPlaylist { req, playlist, video, add },
            move |app, r| match r {
                Ok(_) => app.toast(if add { format!("Saved to {title}") } else { format!("Removed from {title}") }),
                Err(e) => app.toast(format!("Could not change {title}: {e}")),
            },
        );
    }

    pub(crate) fn watch_later(&mut self, video: String) {
        if !self.signed_in() {
            self.auth.dialog_open = true;
            return;
        }
        self.edit_playlist("WL".into(), video, true, "Watch later".into());
    }

    pub(crate) fn create_playlist(&mut self, title: String, privacy: &str, video: String) {
        let privacy = privacy.to_string();
        let name = title.clone();
        self.call(
            move |req| Command::CreatePlaylist { req, title, privacy, video: Some(video) },
            move |app, r| match r {
                Ok(_) => app.toast(format!("Created {name}")),
                Err(e) => app.toast(format!("Could not create the playlist: {e}")),
            },
        );
    }

    // -------------------------------------------------------------- TV

    pub(crate) fn open_tv(&mut self, video: String, at: u32) {
        self.dialog = Some(Dialog::Tv {
            video,
            at,
            devices: None,
            error: None,
        });
        self.call(
            |req| Command::TvFind { req },
            |app, r| {
                if let Some(Dialog::Tv { devices, error, .. }) = &mut app.dialog {
                    match r {
                        Ok(v) => {
                            *devices = Some(
                                v.get("devices")
                                    .and_then(Value::as_array)
                                    .into_iter()
                                    .flatten()
                                    .map(|d| (s(d, "name"), s(d, "app_url")))
                                    .collect(),
                            )
                        }
                        Err(e) => *error = Some(e),
                    }
                }
            },
        );
    }

    pub(crate) fn play_on_tv(&mut self, name: String, app_url: String, video: String, at: u32) {
        self.dialog = None;
        self.toast(format!("Sending to {name}…"));
        self.call(
            move |req| Command::TvPlay { req, app_url, video, at },
            move |app, r| match r {
                Ok(_) => {
                    app.toast(format!("Playing on {name}"));
                    app.player.set_paused(true);
                }
                Err(e) => app.toast(format!("{name}: {e}")),
            },
        );
    }

    // -------------------------------------------------------------- lists and pages

    pub(crate) fn open_notifications(&mut self) {
        let req = self.req();
        self.core.send(Command::Notifications { req });
        self.stack.push(Page::Notifications {
            list: List {
                loading: Some(req),
                ..Default::default()
            },
        });
        if let Some(n) = &mut self.now {
            n.expanded = false;
        }
    }

    /// Show one tab of the channel page on top of the stack.
    pub(crate) fn channel_tab(&mut self, tab: ChannelTab) {
        let req = self.req();
        let Some(Page::Channel { id, tab: current, tab_list, about, .. }) = self.stack.last_mut() else { return };
        if *current == tab {
            return;
        }
        *current = tab;
        if let Some(c) = tab_list.next.take() {
            self.core.send(Command::DropCursor(c));
        }
        *tab_list = List::default();
        let id = id.clone();
        let cmd = match tab {
            ChannelTab::Videos => return,
            ChannelTab::About => {
                if about.is_some() {
                    return;
                }
                let want = id.clone();
                self.call(
                    move |req| Command::ChannelAbout { req, id },
                    move |app, r| {
                        for p in &mut app.stack {
                            if let Page::Channel { id, about, .. } = p {
                                if *id == want {
                                    *about = Some(r.clone().unwrap_or_else(|e| serde_json::json!({ "error": e })));
                                }
                            }
                        }
                    },
                );
                return;
            }
            ChannelTab::Playlists => Command::ChannelPlaylists { req, id },
            ChannelTab::Shorts => Command::ChannelTab { req, id, tab: "shorts".into() },
            ChannelTab::Live => Command::ChannelTab { req, id, tab: "live".into() },
            ChannelTab::Posts => Command::ChannelTab { req, id, tab: "posts".into() },
        };
        if let Some(Page::Channel { tab_list, .. }) = self.stack.last_mut() {
            tab_list.loading = Some(req);
        }
        self.core.send(cmd);
    }

    /// Home, or one of YouTube's destination pages, into the Home list.
    pub(crate) fn explore(&mut self, chip: usize) {
        self.home_chip = chip;
        if chip == 0 {
            self.reload_home();
            return;
        }
        let req = self.req();
        if let Some(c) = self.home.next.take() {
            self.core.send(Command::DropCursor(c));
        }
        self.home = List {
            loading: Some(req),
            ..Default::default()
        };
        let category = EXPLORE[chip].1.to_string();
        self.core.send(Command::Explore { req, category });
    }

    pub(crate) fn load_history_actions(&mut self) {
        if !self.signed_in() {
            return;
        }
        self.call(
            |req| Command::HistoryActions { req },
            |app, r| {
                if let Ok(v) = r {
                    app.history_actions = v
                        .get("actions")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .map(|a| (s(a, "kind"), s(a, "token")))
                        .collect();
                }
            },
        );
    }

    pub(crate) fn history_action(&mut self, kind: String, token: String) {
        self.call(
            move |req| Command::Feedback { req, token },
            move |app, r| match r {
                Ok(_) => {
                    app.toast(match kind.as_str() {
                        "clear" => "Watch history cleared",
                        "pause" => "Watch history paused",
                        _ => "Watch history turned on",
                    });
                    app.history = List::default();
                    app.load_library();
                    app.load_history_actions();
                }
                Err(e) => app.toast(format!("YouTube refused: {e}")),
            },
        );
    }

    pub(crate) fn not_interested(&mut self, id: String) {
        self.home.items.retain(|i| !matches!(i, Item::Video(v) if v.id == id));
        self.call(
            move |req| Command::NotInterested { req, id },
            |app, r| match r {
                Ok(v) if v.get("sent").and_then(Value::as_bool) == Some(false) => {
                    app.toast("Hidden here (YouTube offered no feedback for this video)")
                }
                Ok(_) => app.toast("Got it: fewer videos like this"),
                Err(e) => app.toast(format!("Could not tell YouTube: {e}")),
            },
        );
    }

    // -------------------------------------------------------------- queue

    pub(crate) fn queue_add(&mut self, card: VideoCard, music: bool, next: bool) {
        let item = QueueItem { card, music };
        if next {
            self.queue.insert(0, item);
            self.toast("Plays next");
        } else {
            self.queue.push(item);
            self.toast("Added to queue");
        }
    }

    // -------------------------------------------------------------- per frame

    /// Resume positions, history reporting, the sleep timer and live chat.
    pub(crate) fn tick(&mut self, ctx: &egui::Context) {
        let st = self.player.state();
        let playing = !st.idle && !st.paused && self.now.as_ref().is_some_and(|n| n.info.is_some());
        if playing {
            ctx.request_repaint_after(Duration::from_millis(250));
        }
        let signed_in = self.signed_in();
        let history_paused = self.history_actions.iter().any(|(k, _)| k == "resume");
        let mut report = None;
        if let Some(n) = &mut self.now {
            let live = n.info.as_ref().is_some_and(|i| i.hls.is_some()) || n.extras.as_ref().is_some_and(|e| e.live);
            if playing && !live && st.duration_s > 0.0 && n.saved_at.elapsed() > Duration::from_secs(2) {
                n.saved_at = Instant::now();
                self.store.played(&n.card.id, st.position_s, st.duration_s);
            }
            // YouTube's own player reports what it plays; this one has to say so, or the video
            // never reaches the account's history. First after ten seconds, then every minute.
            if playing && signed_in && !history_paused {
                let at = st.position_s;
                let due = match n.reported_at {
                    None => at >= 10.0,
                    Some(r) => at - r >= 60.0 || at < r - 5.0 || (st.duration_s > 0.0 && at >= st.duration_s - 3.0 && at - r >= 3.0),
                };
                if due {
                    n.reported_at = Some(at);
                    report = Some((n.card.id.clone(), at.round() as u32, st.duration_s.round() as u32));
                }
            }
        }
        if let Some((id, seconds, total)) = report {
            self.core.send(Command::Watched { id, seconds, total });
        }
        if let Some(Sleep::At(t)) = &self.sleep {
            if Instant::now() >= *t {
                self.sleep = None;
                self.player.set_paused(true);
                self.toast("Sleep timer: paused");
            } else {
                ctx.request_repaint_after(Duration::from_secs(1));
            }
        }
        self.poll_chat(ctx);
        self.grow_radio();
        self.store.flush(false);
    }

    fn poll_chat(&mut self, ctx: &egui::Context) {
        let Some(n) = &mut self.now else { return };
        if n.chat.busy || self.watch_tab != WatchTab::Chat {
            return;
        }
        let (Some(token), Some(due)) = (n.chat.next.clone(), n.chat.due) else { return };
        let now = Instant::now();
        if now < due {
            ctx.request_repaint_after(due - now);
            return;
        }
        n.chat.busy = true;
        let id = n.card.id.clone();
        self.call(
            move |req| Command::LiveChat { req, token },
            move |app, r| {
                let Some(n) = app.now.as_mut().filter(|n| n.card.id == id) else { return };
                let chat = &mut n.chat;
                chat.busy = false;
                match r {
                    Ok(v) => {
                        for m in v.get("messages").and_then(Value::as_array).into_iter().flatten() {
                            chat.messages.push_back(ChatMsg {
                                author: s(m, "author"),
                                text: s(m, "text"),
                                amount: m.get("amount").and_then(Value::as_str).map(str::to_string),
                            });
                        }
                        while chat.messages.len() > 300 {
                            chat.messages.pop_front();
                        }
                        chat.next = v.get("next").and_then(Value::as_str).map(str::to_string);
                        let wait = v.get("wait_ms").and_then(Value::as_u64).unwrap_or(5000);
                        chat.due = Some(Instant::now() + Duration::from_millis(wait));
                    }
                    Err(e) => {
                        chat.next = None;
                        chat.ended = Some(e);
                    }
                }
            },
        );
    }
}

/// Home's chips: label and the core's destination name.
pub(crate) const EXPLORE: [(&str, &str); 8] = [
    ("All", ""),
    ("Trending", "trending"),
    ("Music", "music"),
    ("Gaming", "gaming"),
    ("News", "news"),
    ("Sports", "sports"),
    ("Live", "live"),
    ("Podcasts", "podcasts"),
];
