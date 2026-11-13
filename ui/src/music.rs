//! YouTube Music, built in: Home, Explore, Library, search, artist/album/playlist pages, and
//! songs that play as sound (or video) with a radio that keeps going.

use egui::{Align, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense, Ui, vec2};
use serde_json::Value;
use streampipe_core::Command;
use streampipe_core::model::{Cursor, VideoCard};
use streampipe_core::music::{MusicItem, MusicPage, Shelf};
use streampipe_core::service::ReqId;

use crate::app::{App, Page, QueueItem, Tab};
use crate::theme::*;
use crate::widgets;

/// One Music page and its loading state.
#[derive(Default)]
pub(crate) struct MusicView {
    /// Unique per view, so an answer finds its page.
    pub key: u64,
    pub id: String,
    pub params: Option<String>,
    pub title: String,
    pub page: Option<MusicPage>,
    pub loading: bool,
    pub more: Option<ReqId>,
    pub error: Option<String>,
}

pub(crate) const LIBRARY: [(&str, &str); 6] = [
    ("Playlists", "FEmusic_liked_playlists"),
    ("Songs", "FEmusic_liked_videos"),
    ("Albums", "FEmusic_liked_albums"),
    ("Artists", "FEmusic_library_corpus_track_artists"),
    ("Subscriptions", "FEmusic_library_corpus_artists"),
    ("History", "FEmusic_history"),
];

pub(crate) const SEARCH_TABS: [(&str, &str); 7] = [
    ("All", ""),
    ("Songs", "songs"),
    ("Videos", "videos"),
    ("Albums", "albums"),
    ("Artists", "artists"),
    ("Playlists", "playlists"),
    ("Podcasts", "podcasts"),
];

#[derive(Default)]
pub(crate) struct MusicState {
    pub home: MusicView,
    pub explore: MusicView,
    pub library: MusicView,
    pub library_tab: usize,
    pub search: MusicView,
    pub search_tab: usize,
    pub query: String,
    /// The radio's next part (`MusicMore` cursor) while it plays.
    pub radio: Option<u64>,
    pub radio_busy: bool,
    next_key: u64,
}

/// Where a view lives, so an answer can find it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Slot {
    Home,
    Explore,
    Library,
    Search,
    Key(u64),
}

/// The card a song becomes for the player.
pub(crate) fn card(m: &MusicItem) -> VideoCard {
    let artists = m.artists.iter().map(|a| a.name.clone()).collect::<Vec<_>>().join(", ");
    VideoCard {
        id: m.id.clone(),
        title: m.title.clone(),
        channel_id: m.artists.iter().find_map(|a| a.id.clone()),
        channel_name: Some(if artists.is_empty() { m.subtitle.clone() } else { artists }),
        channel_avatar: None,
        duration_s: m.duration.as_deref().and_then(parse_duration),
        thumbnail: m.thumbnail.clone(),
        views: None,
        published: None,
        is_live: false,
        watched: None,
    }
}

fn parse_duration(t: &str) -> Option<u32> {
    let mut total = 0u32;
    for part in t.trim().split(':') {
        total = total * 60 + part.trim().parse::<u32>().ok()?;
    }
    Some(total)
}

pub(crate) fn playable(m: &MusicItem) -> bool {
    matches!(m.kind.as_str(), "song" | "video" | "episode")
}

fn items_of(v: &Value) -> Vec<MusicItem> {
    v.get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|i| serde_json::from_value(i.clone()).ok())
        .collect()
}

impl App {
    fn music_view(&mut self, slot: Slot) -> Option<&mut MusicView> {
        match slot {
            Slot::Home => Some(&mut self.music.home),
            Slot::Explore => Some(&mut self.music.explore),
            Slot::Library => Some(&mut self.music.library),
            Slot::Search => Some(&mut self.music.search),
            Slot::Key(k) => {
                if let Some(v) = self.now.as_mut().and_then(|n| n.music_related.as_mut()).filter(|v| v.key == k) {
                    return Some(v);
                }
                self.stack.iter_mut().find_map(|p| match p {
                    Page::Music(v) if v.key == k => Some(v),
                    _ => None,
                })
            }
        }
    }

    fn new_key(&mut self) -> u64 {
        self.music.next_key += 1;
        self.music.next_key
    }

    /// Ask for a page into `slot`.
    fn music_load(&mut self, slot: Slot, make: impl FnOnce(ReqId) -> Command) {
        if let Some(v) = self.music_view(slot) {
            v.loading = true;
            v.error = None;
            v.page = None;
        }
        self.call(make, move |app, r| {
            let Some(v) = app.music_view(slot) else { return };
            v.loading = false;
            match r.and_then(|j| serde_json::from_value::<MusicPage>(j).map_err(|e| e.to_string())) {
                Ok(p) => v.page = Some(p),
                Err(e) => v.error = Some(e),
            }
        });
    }

    fn music_more(&mut self, slot: Slot) {
        let Some(v) = self.music_view(slot) else { return };
        if v.more.is_some() || v.loading {
            return;
        }
        let Some(next) = v.page.as_mut().and_then(|p| p.next.take()) else { return };
        let req_slot = slot;
        let req = {
            let r = self.req();
            if let Some(v) = self.music_view(slot) {
                v.more = Some(r);
            }
            r
        };
        self.actions.insert(
            req,
            Box::new(move |app: &mut App, r: Result<Value, String>| {
                let Some(v) = app.music_view(req_slot) else { return };
                v.more = None;
                match r.and_then(|j| serde_json::from_value::<MusicPage>(j).map_err(|e| e.to_string())) {
                    Ok(more) => {
                        let Some(page) = &mut v.page else { return };
                        if more.append {
                            let mut shelves = more.shelves.into_iter();
                            if let (Some(last), Some(first)) = (page.shelves.last_mut(), shelves.next()) {
                                last.items.extend(first.items);
                            }
                            page.shelves.extend(shelves);
                        } else {
                            page.shelves.extend(more.shelves);
                        }
                        page.next = more.next;
                    }
                    Err(e) => v.error = Some(e),
                }
            }),
        );
        self.core.send(Command::MusicMore { req, cursor: Cursor(next) });
    }

    /// Load the music tab's page if it has none yet.
    pub(crate) fn music_tab_shown(&mut self) {
        match self.tab {
            Tab::Home if self.music.home.page.is_none() && !self.music.home.loading => {
                self.music_load(Slot::Home, |req| Command::MusicBrowse { req, id: "FEmusic_home".into(), params: None })
            }
            Tab::Subscriptions if self.music.explore.page.is_none() && !self.music.explore.loading => {
                self.music_load(Slot::Explore, |req| Command::MusicBrowse { req, id: "FEmusic_explore".into(), params: None })
            }
            Tab::Library if self.music.library.page.is_none() && !self.music.library.loading => {
                let id = LIBRARY[self.music.library_tab].1.to_string();
                self.music_load(Slot::Library, |req| Command::MusicBrowse { req, id, params: None })
            }
            _ => {}
        }
    }

    pub(crate) fn music_search(&mut self, query: String) {
        self.music.query = query.clone();
        let tab = SEARCH_TABS[self.music.search_tab].1.to_string();
        self.music_load(Slot::Search, |req| Command::MusicSearch { req, query, tab });
    }

    pub(crate) fn set_music_mode(&mut self, on: bool) {
        if self.store.saved.music_mode == on {
            return;
        }
        self.store.saved.music_mode = on;
        self.store.touch();
        self.set_tab(Tab::Home);
        // Leaving Search: what was searched belongs to the other mode.
        self.search_submitted.clear();
        self.suggestions.clear();
        if on {
            self.music_tab_shown();
        }
    }

    /// Open an album, playlist, artist or mood page.
    pub(crate) fn open_music_page(&mut self, id: String, params: Option<String>, title: String) {
        let key = self.new_key();
        let (bid, bparams) = (id.clone(), params.clone());
        self.stack.push(Page::Music(MusicView {
            key,
            id,
            params,
            title,
            ..Default::default()
        }));
        if let Some(n) = &mut self.now {
            n.expanded = false;
        }
        self.music_load(Slot::Key(key), move |req| Command::MusicBrowse { req, id: bid, params: bparams });
    }

    /// Open or play something from a Music page; `list` is the list it was in.
    pub(crate) fn open_music_item(&mut self, item: &MusicItem, list: &[MusicItem]) {
        if playable(item) {
            let rest: Vec<MusicItem> = list.iter().skip_while(|i| i.id != item.id).skip(1).filter(|i| playable(i)).cloned().collect();
            self.play_music(item, if list.len() > 1 { Some(rest) } else { None });
            return;
        }
        if item.kind == "artist" || item.kind == "channel" || !item.id.is_empty() {
            self.open_music_page(item.id.clone(), item.params.clone(), item.title.clone());
        }
    }

    /// Play a song. With `rest`, the songs after it in its list come next; without, its radio.
    pub(crate) fn play_music(&mut self, item: &MusicItem, rest: Option<Vec<MusicItem>>) {
        let radio = rest.is_none();
        if let Some(rest) = rest {
            self.queue = rest.iter().map(|m| QueueItem { card: card(m), music: true }).collect();
            self.music.radio = None;
        }
        self.play_card(card(item), true);
        let id = item.id.clone();
        let want = id.clone();
        // The song's queue answer names its lyrics and related pages, and holds its radio.
        self.call(
            move |req| Command::MusicQueue { req, video: Some(id), list: None },
            move |app, r| {
                let Ok(v) = r else { return };
                if let Some(n) = app.now.as_mut().filter(|n| n.card.id == want) {
                    n.lyrics_id = v.get("lyrics").and_then(Value::as_str).map(str::to_string);
                    n.related_id = v.get("related").and_then(Value::as_str).map(str::to_string);
                }
                if radio {
                    let items: Vec<MusicItem> = items_of(&v).into_iter().filter(|i| i.id != want).collect();
                    app.queue = items.iter().map(|m| QueueItem { card: card(m), music: true }).collect();
                    app.music.radio = v.get("next").and_then(Value::as_u64);
                    let ids = app.queue.iter().take(2).map(|q| q.card.id.clone()).collect();
                    app.prefetch_many(ids);
                }
            },
        );
    }

    /// Play a whole list (album, playlist, radio), optionally shuffled.
    pub(crate) fn play_music_list(&mut self, list: String, shuffle: bool) {
        self.call(
            move |req| Command::MusicQueue { req, video: None, list: Some(list) },
            move |app, r| match r {
                Ok(v) => {
                    let mut items = items_of(&v);
                    if shuffle {
                        shuffle_in_place(&mut items);
                    }
                    if items.is_empty() {
                        app.toast("Nothing to play here");
                        return;
                    }
                    let first = items.remove(0);
                    let next = v.get("next").and_then(Value::as_u64);
                    app.play_music(&first, Some(items));
                    app.music.radio = if shuffle { None } else { next };
                }
                Err(e) => app.toast(format!("Could not play: {e}")),
            },
        );
    }

    /// The radio is running out: fetch its next part.
    pub(crate) fn grow_radio(&mut self) {
        if self.music.radio_busy || self.queue.len() >= 4 || !self.now.as_ref().is_some_and(|n| n.music) {
            return;
        }
        let Some(cursor) = self.music.radio.take() else { return };
        self.music.radio_busy = true;
        self.call(
            move |req| Command::MusicMore { req, cursor: Cursor(cursor) },
            |app, r| {
                app.music.radio_busy = false;
                let Ok(v) = r else { return };
                for shelf in v.get("shelves").and_then(Value::as_array).into_iter().flatten() {
                    for m in items_of(shelf) {
                        if playable(&m) && !app.queue.iter().any(|q| q.card.id == m.id) {
                            app.queue.push(QueueItem { card: card(&m), music: true });
                        }
                    }
                }
                app.music.radio = v.get("next").and_then(Value::as_u64);
            },
        );
    }

    pub(crate) fn load_lyrics(&mut self) {
        let Some(n) = &mut self.now else { return };
        if n.lyrics.is_some() {
            return;
        }
        let Some(id) = n.lyrics_id.clone() else {
            n.lyrics = Some(Err("No lyrics for this song.".into()));
            return;
        };
        n.lyrics = Some(Ok(String::new()));
        let song = n.card.id.clone();
        self.call(
            move |req| Command::MusicLyrics { req, id },
            move |app, r| {
                let Some(n) = app.now.as_mut().filter(|n| n.card.id == song) else { return };
                n.lyrics = Some(match r {
                    Ok(v) => match v.get("text").and_then(Value::as_str) {
                        Some(t) => {
                            let src = v.get("source").and_then(Value::as_str).map(|s| format!("\n\n{s}")).unwrap_or_default();
                            Ok(format!("{t}{src}"))
                        }
                        None => Err("No lyrics for this song.".into()),
                    },
                    Err(e) => Err(e),
                });
            },
        );
    }

    pub(crate) fn load_music_related(&mut self) {
        let key = self.new_key();
        let Some(n) = &mut self.now else { return };
        if n.music_related.is_some() {
            return;
        }
        let Some(id) = n.related_id.clone() else { return };
        n.music_related = Some(MusicView { key, id: id.clone(), ..Default::default() });
        self.music_load(Slot::Key(key), move |req| Command::MusicBrowse { req, id, params: None });
    }

    fn music_save(&mut self, list: String, on: bool) {
        if !self.signed_in() {
            self.auth.dialog_open = true;
            return;
        }
        self.call(
            move |req| Command::MusicSave { req, list, on },
            move |app, r| match r {
                Ok(_) => app.toast(if on { "Saved to your library" } else { "Removed from your library" }),
                Err(e) => app.toast(format!("Could not save: {e}")),
            },
        );
    }
}

fn shuffle_in_place(items: &mut [MusicItem]) {
    // A small xorshift seeded from the clock is plenty for shuffling a playlist.
    let mut x = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7) | 1;
    for i in (1..items.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        items.swap(i, (x % (i as u64 + 1)) as usize);
    }
}

// ------------------------------------------------------------------ views

/// The Music tabs (Home, Explore, Library, Search).
pub(crate) fn browse(app: &mut App, ui: &mut Ui) {
    app.music_tab_shown();
    match app.tab {
        Tab::Home => {
            heading(ui, "Music");
            view(app, ui, Slot::Home);
        }
        Tab::Subscriptions => {
            heading(ui, "Explore");
            view(app, ui, Slot::Explore);
        }
        Tab::Library => {
            heading(ui, "Library");
            if !app.signed_in() {
                widgets::centered_message(ui, "Sign in to see your library", "Your playlists, liked songs, albums and artists from YouTube Music.");
                return;
            }
            let labels: Vec<&str> = LIBRARY.iter().map(|l| l.0).collect();
            if let Some(i) = widgets::pills(ui, &labels, app.music.library_tab) {
                app.music.library_tab = i;
                let id = LIBRARY[i].1.to_string();
                app.music_load(Slot::Library, |req| Command::MusicBrowse { req, id, params: None });
            }
            ui.add_space(8.0);
            view(app, ui, Slot::Library);
        }
        Tab::Search => {
            if app.music.query.is_empty() {
                widgets::centered_message(ui, "Search YouTube Music", "Songs, albums, artists, playlists and podcasts. Type above and press Enter.");
                return;
            }
            heading(ui, &format!("Results for “{}”", app.music.query));
            let labels: Vec<&str> = SEARCH_TABS.iter().map(|l| l.0).collect();
            if let Some(i) = widgets::pills(ui, &labels, app.music.search_tab) {
                app.music.search_tab = i;
                let q = app.music.query.clone();
                app.music_search(q);
            }
            ui.add_space(8.0);
            view(app, ui, Slot::Search);
        }
        _ => {}
    }
}

/// An album, playlist, artist or mood page on the stack.
pub(crate) fn stack_page(app: &mut App, ui: &mut Ui, key: u64) {
    view(app, ui, Slot::Key(key));
}

/// YouTube Music's "Related" tab under a playing song.
pub(crate) fn related(app: &mut App, ui: &mut Ui) {
    app.load_music_related();
    let key = app.now.as_ref().and_then(|n| n.music_related.as_ref()).map(|v| v.key);
    match key {
        Some(k) => {
            shelves_inline(app, ui, Slot::Key(k));
        }
        None => {
            ui.label(RichText::new("Nothing related for this song.").color(TEXT_DIM));
        }
    }
}

fn heading(ui: &mut Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).font(heading_font(24.0)));
    ui.add_space(8.0);
}

enum Act {
    Open(MusicItem, Vec<MusicItem>),
    PlayList(String, bool),
    Save(String, bool),
    Subscribe(String, bool),
    Queue(MusicItem, bool),
    Radio(MusicItem),
    Retry,
}

/// A whole page in its own scroll area.
fn view(app: &mut App, ui: &mut Ui, slot: Slot) {
    let key = match slot {
        Slot::Home => 1,
        Slot::Explore => 2,
        Slot::Library => 3,
        Slot::Search => 4,
        Slot::Key(k) => 100 + k,
    };
    egui::ScrollArea::vertical()
        .id_salt(("music", key))
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let near_end = shelves_inline(app, ui, slot);
            if near_end {
                app.music_more(slot);
            }
        });
}

/// Draw a page's header and shelves; answers whether the end of the page is on screen.
fn shelves_inline(app: &mut App, ui: &mut Ui, slot: Slot) -> bool {
    let Some(v) = app.music_view(slot) else { return false };
    if v.loading {
        for _ in 0..6 {
            widgets::skeleton_row(ui);
        }
        return false;
    }
    if let Some(e) = v.error.clone().filter(|_| v.page.is_none()) {
        let mut retry = false;
        ui.vertical_centered(|ui| {
            ui.add_space(30.0);
            ui.label(RichText::new("Couldn't load this").font(heading_font(16.0)));
            ui.label(RichText::new(e).small().color(TEXT_DIM));
            retry = widgets::ghost(ui, "Retry").clicked();
        });
        if retry {
            run(app, slot, Act::Retry);
        }
        return false;
    }
    let Some(page) = v.page.clone() else { return false };
    let more_loading = v.more.is_some();
    let has_more = page.next.is_some();
    let mut act = None;
    if let Some(h) = &page.header {
        header(app, ui, h, &mut act);
    }
    for (si, shelf) in page.shelves.iter().enumerate() {
        shelf_view(app, ui, shelf, si, &mut act);
    }
    if page.shelves.is_empty() && page.header.is_none() {
        widgets::centered_message(ui, "Nothing here yet", "");
    }
    let end = ui.allocate_response(vec2(1.0, 1.0), Sense::hover());
    if more_loading {
        widgets::skeleton_row(ui);
    }
    if let Some(a) = act {
        run(app, slot, a);
    }
    has_more && ui.is_rect_visible(end.rect)
}

fn run(app: &mut App, slot: Slot, a: Act) {
    match a {
        Act::Open(item, list) => app.open_music_item(&item, &list),
        Act::PlayList(list, shuffle) => app.play_music_list(list, shuffle),
        Act::Save(list, on) => app.music_save(list, on),
        Act::Subscribe(id, on) => app.subscribe(id, on),
        Act::Queue(m, next) => app.queue_add(card(&m), true, next),
        Act::Radio(m) => app.play_music(&m, None),
        Act::Retry => match slot {
            Slot::Search => {
                let q = app.music.query.clone();
                app.music_search(q);
            }
            Slot::Key(_) => {
                if let Some(v) = app.music_view(slot) {
                    let (id, params) = (v.id.clone(), v.params.clone());
                    app.music_load(slot, move |req| Command::MusicBrowse { req, id, params });
                }
            }
            _ => {
                if let Some(v) = app.music_view(slot) {
                    v.page = None;
                }
                if slot == Slot::Library {
                    let id = LIBRARY[app.music.library_tab].1.to_string();
                    app.music_load(slot, |req| Command::MusicBrowse { req, id, params: None });
                } else {
                    app.music_tab_shown();
                }
            }
        },
    }
}

fn header(app: &mut App, ui: &mut Ui, h: &streampipe_core::music::Header, act: &mut Option<Act>) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let artist = h.channel_id.is_some();
        let size = if artist { 150.0 } else { 190.0 };
        let (r, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
        widgets::thumb(app, ui, h.thumbnail.as_deref(), r, if artist { 255 } else { 10 });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.add_space(10.0);
            ui.label(RichText::new(&h.title).font(heading_font(26.0)));
            if !h.subtitle.is_empty() {
                ui.label(RichText::new(&h.subtitle).color(TEXT_DIM));
            }
            if let Some(s) = &h.second_subtitle {
                ui.label(RichText::new(s).color(TEXT_DIM).small());
            }
            if let Some(s) = &h.subscribers {
                ui.label(RichText::new(s).color(TEXT_DIM).small());
            }
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
                if let Some(list) = &h.list_id {
                    if widgets::primary(ui, "▶  Play").clicked() {
                        *act = Some(Act::PlayList(list.clone(), false));
                    }
                    if widgets::ghost(ui, "🔀  Shuffle").clicked() {
                        *act = Some(Act::PlayList(list.clone(), true));
                    }
                    if widgets::ghost(ui, "＋  Save to library").clicked() {
                        *act = Some(Act::Save(list.clone(), true));
                    }
                }
                if let Some(radio) = &h.radio {
                    if widgets::ghost(ui, "📻  Radio").clicked() {
                        *act = Some(Act::PlayList(radio.clone(), false));
                    }
                }
                if let Some(c) = &h.channel_id {
                    let on = h.subscribed.unwrap_or(false);
                    let label = if on { "Subscribed" } else { "Subscribe" };
                    let clicked = if on { widgets::ghost(ui, label).clicked() } else { widgets::primary(ui, label).clicked() };
                    if clicked {
                        *act = Some(Act::Subscribe(c.clone(), !on));
                    }
                }
            });
        });
    });
    if let Some(d) = h.description.as_ref().filter(|d| !d.is_empty()) {
        ui.add_space(6.0);
        egui::CollapsingHeader::new(RichText::new("About").color(TEXT_DIM))
            .id_salt(("music-about", &h.title))
            .show(ui, |ui| {
                ui.label(RichText::new(d).small());
            });
    }
    ui.add_space(14.0);
}

fn shelf_view(app: &mut App, ui: &mut Ui, shelf: &Shelf, si: usize, act: &mut Option<Act>) {
    if !shelf.title.is_empty() {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(&shelf.title).font(heading_font(18.0)));
            if let Some(m) = &shelf.more {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if widgets::ghost(ui, "More").clicked() {
                        *act = Some(Act::Open(m.clone(), Vec::new()));
                    }
                });
            }
        });
        ui.add_space(4.0);
    }
    match shelf.style.as_str() {
        "text" => {
            if let Some(t) = &shelf.text {
                ui.label(RichText::new(t).color(TEXT_DIM));
            }
        }
        "buttons" => {
            ui.horizontal_wrapped(|ui| {
                for m in &shelf.items {
                    let (r, resp) = ui.allocate_exact_size(vec2(180.0, 44.0), Sense::click());
                    let p = ui.painter();
                    p.rect_filled(r, CornerRadius::same(8), if resp.hovered() { SURFACE_HI } else { SURFACE });
                    let stripe = m.color.map(|c| Color32::from_rgb((c >> 16) as u8, (c >> 8) as u8, c as u8)).unwrap_or(ACCENT);
                    p.rect_filled(Rect::from_min_size(r.min, vec2(5.0, r.height())), CornerRadius::same(3), stripe);
                    p.text(r.left_center() + vec2(14.0, 0.0), egui::Align2::LEFT_CENTER, &m.title, FontId::new(14.0, semibold()), TEXT);
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        *act = Some(Act::Open(m.clone(), Vec::new()));
                    }
                }
            });
        }
        "list" => {
            for m in &shelf.items {
                song_row(app, ui, m, &shelf.items, act);
            }
        }
        _ => {
            // Cards: side by side in a row that scrolls (carousel), or wrapped (grid).
            let grid = shelf.style == "grid";
            let draw = |app: &mut App, ui: &mut Ui, act: &mut Option<Act>| {
                for m in &shelf.items {
                    music_card(app, ui, m, &shelf.items, act);
                }
            };
            if grid {
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(14.0, 14.0);
                    draw(app, ui, act);
                });
            } else {
                egui::ScrollArea::horizontal()
                    .id_salt(("shelf", si, &shelf.title))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 14.0;
                            draw(app, ui, act);
                        });
                    });
            }
        }
    }
    ui.add_space(6.0);
}

fn menu(ui: &mut Ui, m: &MusicItem, act: &mut Option<Act>) {
    if playable(m) {
        if ui.button("Play next").clicked() {
            *act = Some(Act::Queue(m.clone(), true));
            ui.close();
        }
        if ui.button("Add to queue").clicked() {
            *act = Some(Act::Queue(m.clone(), false));
            ui.close();
        }
        if ui.button("Start radio").clicked() {
            *act = Some(Act::Radio(m.clone()));
            ui.close();
        }
    }
    if let Some(list) = &m.play_list {
        if ui.button("Play").clicked() {
            *act = Some(Act::PlayList(list.clone(), false));
            ui.close();
        }
        if ui.button("Shuffle").clicked() {
            *act = Some(Act::PlayList(list.clone(), true));
            ui.close();
        }
    }
    if let Some(a) = m.artists.iter().find(|a| a.id.is_some()) {
        if ui.button(format!("Go to {}", a.name)).clicked() {
            let artist = MusicItem {
                kind: "artist".into(),
                id: a.id.clone().unwrap_or_default(),
                title: a.name.clone(),
                ..Default::default()
            };
            *act = Some(Act::Open(artist, Vec::new()));
            ui.close();
        }
    }
    if let Some(al) = m.album.as_ref().filter(|a| a.id.is_some()) {
        if ui.button("Go to album").clicked() {
            let album = MusicItem {
                kind: "album".into(),
                id: al.id.clone().unwrap_or_default(),
                title: al.name.clone(),
                ..Default::default()
            };
            *act = Some(Act::Open(album, Vec::new()));
            ui.close();
        }
    }
    if playable(m) && ui.button("Copy link").clicked() {
        ui.ctx().copy_text(format!("https://music.youtube.com/watch?v={}", m.id));
        ui.close();
    }
}

fn song_row(app: &mut App, ui: &mut Ui, m: &MusicItem, list: &[MusicItem], act: &mut Option<Act>) {
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, 58.0), Sense::click());
    if !ui.is_rect_visible(r) {
        return;
    }
    let playing = app.now.as_ref().is_some_and(|n| n.card.id == m.id);
    if resp.hovered() || playing {
        ui.painter().rect_filled(r, CornerRadius::same(8), SURFACE_HI);
    }
    let t = Rect::from_min_size(r.min + vec2(6.0, 5.0), vec2(48.0, 48.0));
    widgets::thumb(app, ui, m.thumbnail.as_deref(), t, if m.kind == "artist" { 255 } else { 4 });
    let p = ui.painter_at(r);
    let x = t.right() + 12.0;
    let right = r.right() - 70.0;
    let title_c = if playing { ACCENT } else { TEXT };
    let g = p.layout_no_wrap(m.title.clone(), FontId::new(14.5, semibold()), title_c);
    p.with_clip_rect(Rect::from_min_max(egui::pos2(x, r.top()), egui::pos2(right, r.bottom())))
        .galley(egui::pos2(x, r.top() + 10.0), g, title_c);
    let mut sub = m.subtitle.clone();
    if m.explicit {
        sub = format!("🅴 {sub}");
    }
    let g = p.layout_no_wrap(sub, FontId::proportional(12.5), TEXT_DIM);
    p.with_clip_rect(Rect::from_min_max(egui::pos2(x, r.top()), egui::pos2(right, r.bottom())))
        .galley(egui::pos2(x, r.top() + 31.0), g, TEXT_DIM);
    if let Some(d) = &m.duration {
        p.text(r.right_center() - vec2(14.0, 0.0), egui::Align2::RIGHT_CENTER, d, FontId::proportional(12.5), TEXT_DIM);
    }
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() {
        *act = Some(Act::Open(m.clone(), list.to_vec()));
    }
    resp.context_menu(|ui| menu(ui, m, act));
}

fn music_card(app: &mut App, ui: &mut Ui, m: &MusicItem, list: &[MusicItem], act: &mut Option<Act>) {
    let video = m.kind == "video" || m.kind == "episode";
    let w = if video { 256.0 } else { 168.0 };
    let th = if video { w * 9.0 / 16.0 } else { w };
    let (r, resp) = ui.allocate_exact_size(vec2(w, th + 52.0), Sense::click());
    if !ui.is_rect_visible(r) {
        return;
    }
    let t = Rect::from_min_size(r.min, vec2(w, th));
    let round = m.kind == "artist";
    widgets::thumb(app, ui, m.thumbnail.as_deref(), t, if round { 255 } else { 8 });
    if resp.hovered() {
        ui.painter().rect_filled(t, CornerRadius::same(if round { 255 } else { 8 }), Color32::from_black_alpha(70));
        if playable(m) || m.play_list.is_some() {
            ui.painter().circle_filled(t.center(), 20.0, Color32::from_black_alpha(170));
            ui.painter().text(t.center(), egui::Align2::CENTER_CENTER, "▶", FontId::proportional(18.0), Color32::WHITE);
        }
    }
    let p = ui.painter_at(r);
    let mut job = egui::text::LayoutJob::simple(m.title.clone(), FontId::new(14.0, semibold()), TEXT, w);
    job.wrap.max_rows = 1;
    job.wrap.overflow_character = Some('…');
    p.galley(egui::pos2(r.left(), t.bottom() + 6.0), p.layout_job(job), TEXT);
    let mut job = egui::text::LayoutJob::simple(m.subtitle.clone(), FontId::proportional(12.5), TEXT_DIM, w);
    job.wrap.max_rows = 1;
    job.wrap.overflow_character = Some('…');
    p.galley(egui::pos2(r.left(), t.bottom() + 26.0), p.layout_job(job), TEXT_DIM);
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.clicked() {
        *act = Some(Act::Open(m.clone(), list.to_vec()));
    }
    resp.context_menu(|ui| menu(ui, m, act));
}
