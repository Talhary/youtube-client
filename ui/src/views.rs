use std::time::Instant;

use egui::{Align, Color32, CornerRadius, FontId, Key, Layout, RichText, Sense, Stroke, Ui, vec2};
use streampipe_core::Command;
use streampipe_core::auth::SignInMethod;
use streampipe_core::config::{AudioOut, CodecPref, QualityPref};
use streampipe_core::download::{DownloadKind, JobStatus};
use streampipe_core::model::{Item, fmt_count};

use crate::app::{App, LibraryTab, ListRef, Page, Tab};
use crate::theme::*;
use crate::widgets::{self, CARD_GAP, CARD_MIN_W, CARD_TEXT_H, Icon, ROW_H, fmt_bytes, fmt_time};
use crate::{FileDialog, FormFactor, motion};

/// Prefetch the next page when the user scrolls within this many items of the end.
const PREFETCH: usize = 10;
/// Pre-load a video's streams after the pointer rests on it this long.
const HOVER_PRELOAD_S: f64 = 0.25;
const SPEEDS: [f32; 8] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

pub fn root(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    // Built-in egui animations (collapsing sections, menus) follow reduced motion too.
    let anim = if motion::reduced() { 0.0 } else { motion::EXPAND };
    if ui.style().animation_time != anim {
        ctx.all_styles_mut(|s| s.animation_time = anim);
    }
    match app.form {
        FormFactor::Desktop => desktop(app, ui),
        FormFactor::Phone => phone(app, ui),
    }
    auth_dialog(app, &ctx);
    toasts(app, &ctx);
}

// ------------------------------------------------------------------ layouts

fn desktop(app: &mut App, ui: &mut Ui) {
    let playing = app.now.is_some();
    let compact = playing && !app.theater;
    if !app.theater {
        egui::Panel::left("sidebar")
            .exact_size(if compact { 68.0 } else { 224.0 })
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(Color32::from_rgb(0x0f, 0x10, 0x11))
                    .inner_margin(egui::Margin::symmetric(10, 14)),
            )
            .show(ui, |ui| sidebar(app, ui, compact));
    }
    egui::Panel::top("topbar")
        .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(18, 12)))
        .show(ui, |ui| top_bar(app, ui));
    banners(app, ui);

    if playing && !app.theater {
        egui::Panel::left("browse")
            .resizable(true)
            .default_size(430.0)
            .size_range(320.0..=760.0)
            .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(10, 4)))
            .show(ui, |ui| browse(app, ui));
    }
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(20, 6)))
        .show(ui, |ui| {
            if playing {
                player_pane(app, ui);
            } else {
                browse(app, ui);
            }
        });
}

fn phone(app: &mut App, ui: &mut Ui) {
    let expanded = app.now.as_ref().is_some_and(|n| n.expanded);
    if !expanded {
        egui::Panel::top("top")
            .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(12, 10)))
            .show(ui, |ui| top_bar(app, ui));
        banners(app, ui);
    }
    egui::Panel::bottom("nav")
        .frame(egui::Frame::new().fill(SURFACE).inner_margin(egui::Margin::symmetric(4, 4)))
        .show(ui, |ui| bottom_nav(app, ui));
    if app.now.is_some() && !expanded {
        egui::Panel::bottom("mini")
            .frame(egui::Frame::new().fill(SURFACE_HI).inner_margin(egui::Margin::same(8)))
            .show(ui, |ui| mini_player(app, ui));
    }
    egui::CentralPanel::default()
        .frame(egui::Frame::new().fill(BG).inner_margin(egui::Margin::symmetric(8, 4)))
        .show(ui, |ui| {
            if expanded {
                player_pane(app, ui);
            } else {
                browse(app, ui);
            }
        });
}

fn sidebar(app: &mut App, ui: &mut Ui, compact: bool) {
    ui.horizontal(|ui| {
        if compact {
            ui.add_space((ui.available_width() - 30.0) / 2.0);
        }
        widgets::logo(ui, 30.0);
        if !compact {
            ui.add_space(4.0);
            ui.label(RichText::new("StreamPipe").font(heading_font(18.0)));
        }
    });
    ui.add_space(22.0);
    let downloading = app.downloads.values().filter(|d| d.status.is_active()).count();
    let items = [
        (Tab::Home, Icon::Home, "Home", None),
        (Tab::Search, Icon::Search, "Search", None),
        (Tab::Subscriptions, Icon::Subscriptions, "Subscriptions", None),
        (Tab::Library, Icon::Library, "Library", None),
        (Tab::Downloads, Icon::Download, "Downloads", Some(downloading)),
    ];
    for (tab, icon, label, badge) in items {
        let active = app.tab == tab && app.stack.is_empty() && !app.now.as_ref().is_some_and(|n| n.expanded);
        if widgets::nav_item(ui, icon, label, active, badge, compact).clicked() {
            app.set_tab(tab);
        }
        ui.add_space(2.0);
    }
    ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
        let active = app.tab == Tab::Settings && app.stack.is_empty();
        if widgets::nav_item(ui, Icon::Settings, "Settings", active, None, compact).clicked() {
            app.set_tab(Tab::Settings);
        }
        ui.add_space(6.0);
        account_block(app, ui, compact);
    });
}

fn account_block(app: &mut App, ui: &mut Ui, compact: bool) {
    match app.auth.method {
        Some(m) => {
            let label = if compact { "Me".to_string() } else { "Account".to_string() };
            let resp = ui.menu_button(label, |ui| {
                ui.label(RichText::new("Signed in").font(heading_font(14.0)));
                ui.label(
                    RichText::new(match m {
                        SignInMethod::Device => "with a code",
                        SignInMethod::Cookies => "with Google",
                    })
                    .small()
                    .color(TEXT_DIM),
                );
                ui.separator();
                if ui.button("Sign out").clicked() {
                    app.core.send(Command::Logout);
                    ui.close();
                }
            });
            resp.response.on_hover_text("Account");
        }
        None => {
            if compact {
                if widgets::icon_button(ui, Icon::Account, "Sign in").clicked() {
                    app.auth.dialog_open = true;
                    app.auth.error = None;
                }
            } else {
                ui.scope(|ui| {
                    ui.set_width(ui.available_width());
                    if widgets::primary(ui, "Sign in").clicked() {
                        app.auth.dialog_open = true;
                        app.auth.error = None;
                    }
                });
            }
        }
    }
}

fn top_bar(app: &mut App, ui: &mut Ui) {
    let phone = app.form == FormFactor::Phone;
    ui.horizontal(|ui| {
        let can_back = !app.stack.is_empty() || app.now.as_ref().is_some_and(|n| n.expanded);
        if can_back && widgets::ghost(ui, "←").on_hover_text("Back (Esc)").clicked() {
            app.back();
        }
        if phone {
            widgets::logo(ui, 26.0);
        }
        if app.theater && app.form == FormFactor::Desktop && widgets::ghost(ui, "Exit theater").clicked() {
            app.theater = false;
        }
        let right = if phone { 90.0 } else { 0.0 };
        let w = (ui.available_width() - right).min(620.0);
        search_box(app, ui, w);
        if phone {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if widgets::icon_button(ui, Icon::Settings, "Settings").clicked() {
                    app.set_tab(Tab::Settings);
                }
                account_block(app, ui, true);
            });
        }
    });
}

fn search_box(app: &mut App, ui: &mut Ui, width: f32) {
    let id = egui::Id::unique("search-box");
    let frame = egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, if ui.memory(|m| m.has_focus(id)) { ACCENT.linear_multiply(0.7) } else { BORDER }))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::symmetric(12, 7));
    let resp = frame
        .show(ui, |ui| {
            ui.set_width(width);
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                widgets::paint_icon(ui.painter(), Icon::Search, r.center(), 15.0, TEXT_DIM);
                ui.add(
                    egui::TextEdit::singleline(&mut app.search_text)
                        .id(id)
                        .frame(egui::Frame::NONE)
                        .hint_text("Search videos, channels and playlists   /")
                        .desired_width(ui.available_width()),
                )
            })
            .inner
        })
        .inner;
    if app.focus_search {
        resp.request_focus();
        app.focus_search = false;
    }
    if resp.changed() {
        app.search_text_changed();
    }
    if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
        app.submit_search();
    }
    if resp.has_focus() && !app.suggestions.is_empty() {
        let mut pick = None;
        egui::Popup::from_response(&resp)
            .open(true)
            .width(width)
            .show(|ui| {
                for s in &app.suggestions {
                    if ui
                        .add(egui::Button::new(RichText::new(s)).frame(false))
                        .clicked()
                    {
                        pick = Some(s.clone());
                    }
                }
            });
        if let Some(s) = pick {
            app.search_text = s;
            app.submit_search();
        }
    }
}

fn bottom_nav(app: &mut App, ui: &mut Ui) {
    let tabs = [
        (Tab::Home, Icon::Home, "Home"),
        (Tab::Search, Icon::Search, "Search"),
        (Tab::Subscriptions, Icon::Subscriptions, "Subs"),
        (Tab::Library, Icon::Library, "Library"),
        (Tab::Downloads, Icon::Download, "Downloads"),
    ];
    let w = ui.available_width() / tabs.len() as f32;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for (tab, icon, label) in tabs {
            let active = app.tab == tab && app.stack.is_empty();
            let color = if active { ACCENT } else { TEXT_DIM };
            let (rect, resp) = ui.allocate_exact_size(vec2(w, 54.0), Sense::click());
            let p = ui.painter();
            widgets::paint_icon(p, icon, rect.center() - vec2(0.0, 9.0), 20.0, color);
            p.text(rect.center() + vec2(0.0, 14.0), egui::Align2::CENTER_CENTER, label, FontId::proportional(11.0), color);
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, active, label));
            if resp.clicked() {
                if let Some(n) = &mut app.now {
                    n.expanded = false;
                }
                app.set_tab(tab);
            }
        }
    });
}

fn mini_player(app: &mut App, ui: &mut Ui) {
    let st = app.player.state();
    let Some(n) = &app.now else { return };
    let title = n.details.as_ref().map(|d| d.video.title.clone()).unwrap_or_else(|| n.card.title.clone());
    let channel = n.card.channel_name.clone().unwrap_or_default();
    let thumb = n.card.thumbnail.clone();
    let frac = if st.duration_s > 0.0 { (st.position_s / st.duration_s) as f32 } else { 0.0 };
    let mut expand = false;
    ui.horizontal(|ui| {
        let (r, resp) = ui.allocate_exact_size(vec2(96.0, 54.0), Sense::click());
        widgets::thumb(app, ui, thumb.as_deref(), r, 8);
        expand |= resp.clicked();
        let text = ui
            .vertical(|ui| {
                ui.set_width(ui.available_width() - 100.0);
                ui.add(egui::Label::new(RichText::new(&title).font(heading_font(14.0))).truncate());
                ui.add(egui::Label::new(RichText::new(&channel).color(TEXT_DIM).small()).truncate());
            })
            .response
            .interact(Sense::click());
        expand |= text.clicked();
        let icon = if st.paused { "▶" } else { "⏸" };
        if ui.add(egui::Button::new(RichText::new(icon).size(20.0))).on_hover_text("Play/pause").clicked() {
            app.player.set_paused(!st.paused);
        }
        if ui.button("✖").on_hover_text("Close player").clicked() {
            app.player.stop();
            app.platform.background_playback(false);
            app.now = None;
        }
    });
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 2.0), Sense::hover());
    ui.painter().rect_filled(r, 0.0, BORDER);
    let mut f = r;
    f.set_width(r.width() * frac.clamp(0.0, 1.0));
    ui.painter().rect_filled(f, 0.0, ACCENT);
    if expand {
        if let Some(n) = &mut app.now {
            n.expanded = true;
        }
    }
}

fn banners(app: &mut App, ui: &mut Ui) {
    let msg = app.remote.message.clone();
    let update = app.update.clone();
    if msg.is_none() && update.is_none() {
        return;
    }
    egui::Panel::top("banners")
        .frame(
            egui::Frame::new()
                .fill(ACCENT.linear_multiply(0.08))
                .inner_margin(egui::Margin::symmetric(18, 8)),
        )
        .show(ui, |ui| {
            if let Some(m) = msg {
                ui.label(RichText::new(m).color(ACCENT));
            }
            if let Some(u) = update {
                ui.horizontal(|ui| {
                    ui.label(format!("StreamPipe {} is available.", u.version));
                    if ui.link("Download").clicked() {
                        app.platform.open_url(&u.url);
                    }
                    if ui.small_button("Dismiss").clicked() {
                        app.update = None;
                    }
                });
            }
        });
}

// ------------------------------------------------------------------ browse

/// Fade-and-rise transition whenever the visible page changes.
fn nav_transition(ui: &mut Ui, key: u64) {
    let ctx = ui.ctx().clone();
    let id = egui::Id::unique("nav-transition");
    let (last, t0): (u64, f64) = ctx.data(|d| d.get_temp(id)).unwrap_or((key, -10.0));
    let now = ctx.input(|i| i.time);
    let t0 = if last != key { now } else { t0 };
    ctx.data_mut(|d| d.insert_temp(id, (key, t0)));
    let dur = if motion::reduced() { 0.0 } else { motion::NAV as f64 };
    let t = if dur == 0.0 { 1.0 } else { ((now - t0) / dur).clamp(0.0, 1.0) as f32 };
    if t < 1.0 {
        ctx.request_repaint();
    }
    let e = motion::ease_out(t);
    ui.set_opacity(e);
    ui.add_space((1.0 - e) * 14.0);
}

fn browse(app: &mut App, ui: &mut Ui) {
    let key = (app.tab as u64) << 8 | app.stack.len() as u64 | (app.library_tab as u64) << 16;
    nav_transition(ui, key);

    if !app.stack.is_empty() {
        stack_page(app, ui);
        return;
    }
    match app.tab {
        Tab::Home => {
            heading(ui, "Home", "");
            list_view(app, ui, ListRef::Home, "Nothing here yet", true);
        }
        Tab::Search => {
            if app.search_submitted.is_empty() {
                widgets::centered_message(ui, "Search YouTube", "Type in the search box above and press Enter.");
            } else {
                heading(ui, &format!("Results for “{}”", app.search_submitted), "");
                list_view(app, ui, ListRef::Search, "No results", true);
            }
        }
        Tab::Subscriptions => {
            heading(ui, "Subscriptions", "Latest uploads from channels you follow");
            if app.auth.method.is_none() {
                sign_in_prompt(app, ui);
            } else {
                list_view(app, ui, ListRef::Feed, "No new videos from your subscriptions", true);
            }
        }
        Tab::Library => {
            heading(ui, "Library", "");
            if app.auth.method.is_none() {
                sign_in_prompt(app, ui);
                return;
            }
            let tabs = [LibraryTab::History, LibraryTab::Playlists, LibraryTab::Channels];
            let current = tabs.iter().position(|t| *t == app.library_tab).unwrap_or(0);
            if let Some(i) = widgets::pills(ui, &["History", "Playlists", "Channels"], current) {
                app.library_tab = tabs[i];
                app.load_library();
            }
            ui.add_space(8.0);
            let which = match app.library_tab {
                LibraryTab::History => ListRef::History,
                LibraryTab::Playlists => ListRef::Saved,
                LibraryTab::Channels => ListRef::Subs,
            };
            list_view(app, ui, which, "Nothing here yet", true);
        }
        Tab::Downloads => downloads(app, ui),
        Tab::Settings => settings(app, ui),
    }
}

fn heading(ui: &mut Ui, text: &str, sub: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text).font(heading_font(24.0)));
    if !sub.is_empty() {
        ui.label(RichText::new(sub).color(TEXT_DIM));
    }
    ui.add_space(10.0);
}

fn sign_in_prompt(app: &mut App, ui: &mut Ui) {
    widgets::centered_message(
        ui,
        "Sign in to see this",
        "Search, playback and downloads work without an account. Signing in adds subscriptions, history and playlists.",
    );
    ui.vertical_centered(|ui| {
        ui.add_space(10.0);
        if widgets::primary(ui, "Sign in").clicked() {
            app.auth.dialog_open = true;
        }
    });
}

fn stack_page(app: &mut App, ui: &mut Ui) {
    let mut dl_playlist = None;
    match app.stack.last() {
        Some(Page::Channel { header, error, .. }) => {
            if let Some(e) = error {
                error_box(app, ui, e.clone(), ListRef::Stack);
                return;
            }
            match header {
                Some(c) => {
                    let avatar = c.channel.avatar.clone();
                    let name = c.channel.name.clone();
                    let subs = c.channel.subscribers;
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
                        widgets::thumb(app, ui, avatar.as_deref(), r, 255);
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.add_space(8.0);
                            ui.label(RichText::new(&name).font(heading_font(24.0)));
                            if let Some(n) = subs {
                                ui.label(RichText::new(format!("{} subscribers", fmt_count(n))).color(TEXT_DIM));
                            }
                        });
                    });
                }
                None => {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
                        ui.painter().circle_filled(r.center(), 36.0, SKELETON);
                        let (r, _) = ui.allocate_exact_size(vec2(200.0, 22.0), Sense::hover());
                        ui.painter().rect_filled(r, CornerRadius::same(4), SKELETON);
                    });
                }
            }
            ui.add_space(12.0);
        }
        Some(Page::Playlist { header, error, id, .. }) => {
            if let Some(e) = error {
                error_box(app, ui, e.clone(), ListRef::Stack);
                return;
            }
            let id = id.clone();
            match header {
                Some(p) => {
                    ui.add_space(6.0);
                    ui.label(RichText::new(&p.playlist.name).font(heading_font(24.0)));
                    let mut meta = Vec::new();
                    if let Some(c) = &p.playlist.channel_name {
                        meta.push(c.clone());
                    }
                    if let Some(n) = p.playlist.video_count {
                        meta.push(format!("{n} videos"));
                    }
                    ui.label(RichText::new(meta.join("  ·  ")).color(TEXT_DIM));
                    ui.add_space(6.0);
                    ui.menu_button("Download all", |ui| {
                        for (label, kind) in [
                            ("Video (best quality)", DownloadKind::Video { max_height: None }),
                            ("Video (up to 1080p)", DownloadKind::Video { max_height: Some(1080) }),
                            ("Video (up to 720p)", DownloadKind::Video { max_height: Some(720) }),
                            ("Audio (M4A)", DownloadKind::Audio(AudioOut::M4a)),
                            ("Audio (Opus)", DownloadKind::Audio(AudioOut::Opus)),
                        ] {
                            if ui.button(label).clicked() {
                                dl_playlist = Some((id.clone(), kind));
                                ui.close();
                            }
                        }
                    });
                }
                None => {
                    let (r, _) = ui.allocate_exact_size(vec2(260.0, 24.0), Sense::hover());
                    ui.painter().rect_filled(r, CornerRadius::same(4), SKELETON);
                }
            }
            ui.add_space(12.0);
        }
        None => {}
    }
    if let Some((id, kind)) = dl_playlist {
        app.core.send(Command::DownloadPlaylist { id, kind, group: None });
    }
    list_view(app, ui, ListRef::Stack, "No videos", true);
}

fn error_box(app: &mut App, ui: &mut Ui, e: String, which: ListRef) {
    ui.add_space(30.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new("Couldn't load this").font(heading_font(16.0)));
        ui.add_space(2.0);
        ui.label(RichText::new(e).color(TEXT_DIM).small());
        ui.add_space(8.0);
        if widgets::ghost(ui, "Retry").clicked() {
            app.retry(which);
        }
    });
}

/// Grid geometry for a width: (columns, card width, row height).
fn grid_dims(width: f32) -> (usize, f32, f32) {
    let cols = (((width + CARD_GAP) / (CARD_MIN_W + CARD_GAP)).floor() as usize).clamp(1, 6);
    let card_w = (width - CARD_GAP * (cols as f32 - 1.0)) / cols as f32;
    (cols, card_w, card_w * 9.0 / 16.0 + CARD_TEXT_H)
}

/// A virtualized, paginated list: a card grid on wide screens, rows otherwise. Renders the
/// first page as soon as it arrives and prefetches the next one near the end. `keyboard`
/// enables arrow-key selection and Enter to open.
fn list_view(app: &mut App, ui: &mut Ui, which: ListRef, empty: &str, keyboard: bool) {
    let width = ui.available_width();
    let grid = app.form == FormFactor::Desktop && which != ListRef::Related && width >= CARD_MIN_W * 2.0 + CARD_GAP;
    let (cols, card_w, card_h) = if grid { grid_dims(width) } else { (1, width, ROW_H) };

    let Some(l) = app.list_mut(which) else { return };
    if l.is_empty_loading() {
        egui::ScrollArea::vertical()
            .id_salt(("list", which as u8))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if grid {
                    for _ in 0..3 {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = CARD_GAP;
                            for _ in 0..cols {
                                widgets::skeleton_card(ui, card_w);
                            }
                        });
                        ui.add_space(CARD_GAP);
                    }
                } else {
                    for _ in 0..8 {
                        widgets::skeleton_row(ui);
                    }
                }
            });
        return;
    }
    if l.items.is_empty() {
        if let Some(e) = l.error.clone() {
            error_box(app, ui, e, which);
        } else if l.loaded_once {
            widgets::centered_message(ui, empty, "");
        }
        return;
    }

    let n = l.items.len();
    let has_more = l.next.is_some() || l.more.is_some();
    let tail_error = l.error.clone();
    let mut selected = l.selected;

    // Keyboard selection.
    let mut scroll_to = None;
    if keyboard && !ui.ctx().egui_wants_keyboard_input() {
        let (down, up, left, right, enter) = ui.input_mut(|i| {
            (
                i.consume_key(egui::Modifiers::NONE, Key::ArrowDown),
                i.consume_key(egui::Modifiers::NONE, Key::ArrowUp),
                grid && i.consume_key(egui::Modifiers::NONE, Key::ArrowLeft),
                grid && i.consume_key(egui::Modifiers::NONE, Key::ArrowRight),
                i.consume_key(egui::Modifiers::NONE, Key::Enter),
            )
        });
        let step = |s: Option<usize>, d: isize| {
            Some(s.map_or(0, |s| (s as isize + d).clamp(0, n as isize - 1) as usize))
        };
        if down {
            selected = step(selected, cols as isize);
        }
        if up {
            selected = step(selected, -(cols as isize));
        }
        if right {
            selected = step(selected, 1);
        }
        if left {
            selected = step(selected, -1);
        }
        if down || up || left || right {
            scroll_to = selected;
        }
        if enter {
            if let Some(i) = selected {
                let item = app.list_mut(which).unwrap().items[i].clone();
                app.open_item(&item);
                return;
            }
        }
    }

    let rows = n.div_ceil(cols);
    let total_rows = rows + usize::from(has_more || tail_error.is_some());
    let row_h = if grid { card_h + CARD_GAP - ui.spacing().item_spacing.y } else { ROW_H };
    let mut open: Option<Item> = None;
    let mut want_more = false;
    let mut retry = false;
    let mut channel_click = None;
    let mut prefetch = None;
    let mut sa = egui::ScrollArea::vertical()
        .id_salt(("list", which as u8))
        .auto_shrink([false, false]);
    if let Some(i) = scroll_to {
        let spacing = ui.spacing().item_spacing.y;
        let y = (i / cols) as f32 * (row_h + spacing);
        let view_h = ui.available_height();
        let off = ui
            .ctx()
            .data(|d| d.get_temp::<f32>(egui::Id::unique(("list-off", which as u8))))
            .unwrap_or(0.0);
        if y < off {
            sa = sa.vertical_scroll_offset(y);
        } else if y + row_h > off + view_h {
            sa = sa.vertical_scroll_offset(y + row_h - view_h);
        }
    }
    let still = ui.input(|i| i.pointer.time_since_last_movement()) as f64;
    let out = sa.show_rows(ui, row_h, total_rows, |ui, range| {
        for r in range.clone() {
            if r >= rows {
                if tail_error.is_some() {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Couldn't load more.").color(TEXT_DIM));
                        if ui.button("Retry").clicked() {
                            retry = true;
                        }
                    });
                } else if grid {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = CARD_GAP;
                        for _ in 0..cols {
                            widgets::skeleton_card(ui, card_w);
                        }
                    });
                } else {
                    widgets::skeleton_row(ui);
                }
                continue;
            }
            let mut draw = |ui: &mut Ui, i: usize| {
                let item = app.list_mut(which).unwrap().items[i].clone();
                let resp = if grid {
                    widgets::item_card(app, ui, &item, card_w, selected == Some(i))
                } else {
                    widgets::item_row(app, ui, &item, selected == Some(i))
                };
                if resp.clicked() {
                    selected = Some(i);
                    open = Some(item.clone());
                }
                if let Item::Video(v) = &item {
                    if resp.hovered() {
                        if still >= HOVER_PRELOAD_S {
                            prefetch = Some(v.id.clone());
                        } else {
                            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(HOVER_PRELOAD_S - still + 0.02));
                        }
                    }
                }
                resp.context_menu(|ui| {
                    if let Item::Video(v) = &item {
                        if let Some(c) = &v.channel_id {
                            if ui.button("Go to channel").clicked() {
                                channel_click = Some(c.clone());
                                ui.close();
                            }
                        }
                        if ui.button("Copy link").clicked() {
                            ui.ctx().copy_text(format!("https://youtu.be/{}", v.id));
                            ui.close();
                        }
                    }
                });
            };
            if grid {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = CARD_GAP;
                    for i in r * cols..((r + 1) * cols).min(n) {
                        draw(ui, i);
                    }
                });
                ui.add_space(CARD_GAP - ui.spacing().item_spacing.y);
            } else {
                draw(ui, r);
            }
        }
        if range.end * cols + PREFETCH >= n && has_more {
            want_more = true;
        }
    });
    ui.ctx().data_mut(|d| {
        d.insert_temp(egui::Id::unique(("list-off", which as u8)), out.state.clamped_offset().y)
    });
    if let Some(l) = app.list_mut(which) {
        l.selected = selected;
    }
    if retry {
        if let Some(l) = app.list_mut(which) {
            l.error = None;
        }
        want_more = true;
    }
    if want_more && tail_error.is_none() || retry {
        app.load_more(which);
    }
    if let Some(id) = prefetch {
        app.prefetch(&id);
    }
    if let Some(c) = channel_click {
        app.open_channel(c);
    }
    if let Some(item) = open {
        app.open_item(&item);
    }
}

// ------------------------------------------------------------------ player

fn player_pane(app: &mut App, ui: &mut Ui) {
    let phone = app.form == FormFactor::Phone;
    if phone {
        ui.horizontal(|ui| {
            if ui.button("⌄").on_hover_text("Minimize").clicked() {
                if let Some(n) = &mut app.now {
                    n.expanded = false;
                }
            }
        });
    }
    let st = app.player.state();
    let avail = ui.available_size();
    let mut w = avail.x;
    let max_h = if app.theater { avail.y - 120.0 } else { avail.y * 0.62 };
    let mut h = w * 9.0 / 16.0;
    if h > max_h {
        h = max_h.max(120.0);
        w = h * 16.0 / 9.0;
    }
    ui.add_space(4.0);
    let (outer, resp) = ui.allocate_exact_size(vec2(avail.x, h), Sense::click());
    let rect = egui::Rect::from_center_size(outer.center(), vec2(w, h));
    ui.painter().rect_filled(outer, CornerRadius::same(14), Color32::BLACK);
    app.video_px_height = (h * ui.ctx().pixels_per_point()) as u32;

    let (resolving, error, thumb, waited) = match &app.now {
        Some(n) => (
            n.resolve_req.is_some() && n.error.is_none(),
            n.error.clone(),
            n.card.thumbnail.clone(),
            n.requested_at.elapsed().as_secs(),
        ),
        None => return,
    };
    if app.audio_only || resolving {
        widgets::thumb(app, ui, thumb.as_deref(), rect, 14);
        if app.audio_only && !resolving {
            ui.painter().rect_filled(rect, CornerRadius::same(14), Color32::from_black_alpha(150));
            ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "♫  Audio only", heading_font(20.0), TEXT);
        }
    }
    if !app.audio_only && !resolving {
        app.player.show_video(ui, rect);
    }
    let preparing = format!("Preparing video…  {waited}s");
    let overlay = if resolving {
        // Repaint once a second so the timer moves.
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        Some(preparing.as_str())
    } else if st.buffering && !st.idle {
        Some("Buffering…")
    } else {
        None
    };
    if let Some(text) = overlay {
        let pill = egui::Rect::from_center_size(rect.center(), vec2(text.len() as f32 * 8.0 + 30.0, 34.0));
        ui.painter().rect_filled(pill, CornerRadius::same(17), Color32::from_black_alpha(190));
        ui.painter().text(pill.center(), egui::Align2::CENTER_CENTER, text, FontId::proportional(14.0), TEXT);
    }
    if let Some(e) = &error {
        ui.painter().rect_filled(rect, CornerRadius::same(14), Color32::from_black_alpha(210));
        ui.painter().text(rect.center() - vec2(0.0, 10.0), egui::Align2::CENTER_CENTER, "Playback failed", heading_font(16.0), TEXT);
        ui.painter().text(
            rect.center() + vec2(0.0, 14.0),
            egui::Align2::CENTER_CENTER,
            truncate(e, 90),
            FontId::proportional(12.0),
            TEXT_DIM,
        );
    }
    if resp.clicked() && !st.idle {
        app.player.set_paused(!st.paused);
    }
    if resp.double_clicked() && !phone {
        app.theater = !app.theater;
    }

    controls(app, ui, &st, error.is_some());
    info_block(app, ui);
    if !app.theater {
        ui.add_space(6.0);
        ui.label(RichText::new("Up next").font(heading_font(15.0)));
        ui.add_space(2.0);
        list_view(app, ui, ListRef::Related, "No related videos", false);
    }
}

fn controls(app: &mut App, ui: &mut Ui, st: &crate::PlayerState, failed: bool) {
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let playing = !(st.paused || st.idle);
        if widgets::play_button(ui, playing, !st.idle).on_hover_text("Play/pause (Space)").clicked() {
            app.player.set_paused(!st.paused);
        }
        ui.label(RichText::new(fmt_time(st.position_s)).monospace().small().color(TEXT_DIM));
        let right_w = if app.form == FormFactor::Phone { 64.0 } else { 190.0 };
        let bar_w = (ui.available_width() - right_w).max(60.0);
        if let Some(p) = widgets::seek_bar(ui, st.position_s, st.duration_s, bar_w) {
            let expired = app.now.as_ref().and_then(|n| n.info.as_ref()).is_some_and(|i| i.expired());
            if expired {
                app.reresolve(false);
            }
            app.player.seek(p);
        }
        ui.label(RichText::new(fmt_time(st.duration_s)).monospace().small().color(TEXT_DIM));
        if app.form == FormFactor::Desktop {
            let mut vol = st.volume;
            ui.spacing_mut().slider_width = 90.0;
            if ui
                .add(egui::Slider::new(&mut vol, 0.0..=130.0).show_value(false).text("Vol"))
                .on_hover_text("Volume (+/−)")
                .changed()
            {
                app.player.set_volume(vol);
                app.update_settings(|s| s.volume = vol);
            }
        }
    });
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        if failed && widgets::primary(ui, "Retry").clicked() {
            if let Some(n) = &mut app.now {
                n.retried_403 = false;
                n.error = None;
                n.info = None;
            }
            app.reresolve(true);
        }
        // Quality.
        let (labels, current, auto, source) = match app.now.as_ref().and_then(|n| n.info.as_ref().map(|i| (i, n.quality))) {
            Some((info, q)) => (
                info.qualities.iter().map(|q| format!("{}  {}", q.label, q.stream.codec.to_uppercase())).collect::<Vec<_>>(),
                q,
                info.selected,
                info.client.clone(),
            ),
            None => (Vec::new(), None, None, String::new()),
        };
        let shown = current
            .and_then(|i| labels.get(i).cloned())
            .unwrap_or_else(|| "Quality".into());
        let mut pick = None;
        ui.add_enabled_ui(!labels.is_empty() && !app.audio_only, |ui| {
            egui::ComboBox::from_id_salt("quality")
                .selected_text(shown.clone())
                .width(150.0)
                .show_ui(ui, |ui| {
                    for (i, l) in labels.iter().enumerate() {
                        let mut text = l.clone();
                        if Some(i) == auto {
                            text.push_str("  · auto");
                        }
                        if ui.selectable_label(current == Some(i), text).clicked() {
                            pick = Some(i);
                        }
                    }
                    if !source.is_empty() {
                        ui.separator();
                        ui.label(RichText::new(format!("Source: {source}")).small().color(TEXT_DIM));
                    }
                })
                .response
                .on_hover_text("Quality");
        });
        if let Some(i) = pick {
            if let Some(n) = &mut app.now {
                n.quality = Some(i);
            }
            app.reload_stream();
        }
        // Speed.
        let speed = app.settings.as_ref().map(|s| s.speed).unwrap_or(1.0);
        let mut new_speed = None;
        egui::ComboBox::from_id_salt("speed")
            .selected_text(format!("{speed}×"))
            .width(62.0)
            .show_ui(ui, |ui| {
                for s in SPEEDS {
                    if ui.selectable_label((s - speed).abs() < 0.01, format!("{s}×")).clicked() {
                        new_speed = Some(s);
                    }
                }
            })
            .response
            .on_hover_text("Playback speed");
        if let Some(s) = new_speed {
            app.player.set_speed(s);
            app.update_settings(|st| st.speed = s);
        }
        let mut ao = app.audio_only;
        if ui.toggle_value(&mut ao, "Audio only").on_hover_text("Audio only (A)").changed() {
            app.audio_only = ao;
            app.reload_stream();
        }
        let desktop = app.platform.is_desktop();
        let mut dl = None;
        ui.menu_button("Download", |ui| {
            if ui.button("Video (best quality)").clicked() {
                dl = Some(DownloadKind::Video { max_height: None });
            }
            for h in [1080, 720, 480] {
                if ui.button(format!("Video (up to {h}p)")).clicked() {
                    dl = Some(DownloadKind::Video { max_height: Some(h) });
                }
            }
            ui.separator();
            if ui.button("Audio (M4A)").clicked() {
                dl = Some(DownloadKind::Audio(AudioOut::M4a));
            }
            if ui.button("Audio (Opus)").clicked() {
                dl = Some(DownloadKind::Audio(AudioOut::Opus));
            }
            if desktop && ui.button("Audio (MP3, re-encoded)").clicked() {
                dl = Some(DownloadKind::Audio(AudioOut::Mp3));
            }
            if dl.is_some() {
                ui.close();
            }
        });
        if let Some(k) = dl {
            app.download(k);
        }
        if app.form == FormFactor::Desktop {
            let label = if app.theater { "Exit theater" } else { "Theater" };
            if ui.button(label).on_hover_text("Toggle theater mode (T)").clicked() {
                app.theater = !app.theater;
            }
            if ui.button("Full screen").on_hover_text("Full screen (F11)").clicked() {
                let full = ui.ctx().input(|i| i.viewport().fullscreen.unwrap_or(false));
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Fullscreen(!full));
            }
        }
    });
}

fn info_block(app: &mut App, ui: &mut Ui) {
    let Some(n) = &app.now else { return };
    let title = n
        .details
        .as_ref()
        .map(|d| d.video.title.clone())
        .or_else(|| n.info.as_ref().map(|i| i.title.clone()).filter(|t| !t.is_empty()))
        .unwrap_or_else(|| n.card.title.clone());
    let channel = n
        .details
        .as_ref()
        .and_then(|d| d.video.channel_name.clone())
        .or_else(|| n.card.channel_name.clone());
    let channel_id = n
        .details
        .as_ref()
        .and_then(|d| d.video.channel_id.clone())
        .or_else(|| n.card.channel_id.clone());
    let mut meta = Vec::new();
    if let Some(d) = &n.details {
        if let Some(v) = d.video.views {
            meta.push(format!("{} views", fmt_count(v)));
        }
        if let Some(p) = &d.video.published {
            meta.push(p.clone());
        }
        if let Some(l) = d.likes {
            meta.push(format!("{} likes", fmt_count(l as u64)));
        }
    }
    let description = n.details.as_ref().map(|d| d.description.clone());
    let video_id = n.card.id.clone();

    ui.add_space(10.0);
    ui.label(RichText::new(title).font(heading_font(19.0)));
    ui.add_space(2.0);
    let mut go_channel = false;
    ui.horizontal(|ui| {
        if let Some(c) = &channel {
            if ui.link(RichText::new(c).color(TEXT).font(FontId::new(14.0, semibold()))).clicked() {
                go_channel = true;
            }
        }
        if !meta.is_empty() {
            ui.label(RichText::new(meta.join("  ·  ")).color(TEXT_DIM).small());
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("Open on YouTube ↗").clicked() {
                app.platform.open_url(&format!("https://www.youtube.com/watch?v={video_id}"));
            }
            if ui.small_button("Copy link").clicked() {
                ui.ctx().copy_text(format!("https://youtu.be/{video_id}"));
                app.toast("Link copied");
            }
        });
    });
    if go_channel {
        if let Some(id) = channel_id {
            app.open_channel(id);
        }
    }
    if let Some(d) = description.filter(|d| !d.is_empty()) {
        ui.add_space(4.0);
        egui::CollapsingHeader::new(RichText::new("Description").color(TEXT_DIM))
            .id_salt(("desc", &video_id))
            .default_open(false)
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .max_height(180.0)
                    .id_salt("desc-scroll")
                    .show(ui, |ui| {
                        ui.label(RichText::new(d).small());
                    });
            });
    }
}

// ------------------------------------------------------------------ downloads

fn downloads(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| heading(ui, "Downloads", ""));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let any_done = app
                .downloads
                .values()
                .any(|d| matches!(d.status, JobStatus::Done(_) | JobStatus::Failed(_) | JobStatus::Cancelled));
            if ui.add_enabled(any_done, egui::Button::new("Clear finished")).clicked() {
                app.core.send(Command::ClearDownloads);
            }
            if let Some(dir) = app.status.as_ref().map(|s| s.download_dir.clone()) {
                if app.platform.is_desktop() && ui.button("Open folder").on_hover_text(dir.display().to_string()).clicked() {
                    let _ = std::fs::create_dir_all(&dir);
                    app.platform.open_path(&dir);
                }
            }
        });
    });
    if let Some(st) = &app.status {
        if !st.ffmpeg && app.platform.is_desktop() {
            ui.horizontal(|ui| {
                ui.label(RichText::new("FFmpeg is needed to merge video and audio.").color(ACCENT));
                if ui.button("Install FFmpeg").clicked() {
                    app.core.send(Command::InstallFfmpeg);
                }
            });
        }
    }
    if app.downloads.is_empty() {
        widgets::centered_message(ui, "No downloads yet", "Use Download in the player to save a video or its audio.");
        return;
    }
    let ids: Vec<u64> = app.downloads.keys().rev().copied().collect();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        for id in ids {
            let Some(d) = app.downloads.get(&id) else { continue };
            let title = d.req.title.clone();
            let status = d.status.clone();
            let (done, total, bps) = (d.done, d.total, d.bps);
            let thumb = format!("https://i.ytimg.com/vi/{}/mqdefault.jpg", d.req.video_id);
            let kind = match &d.req.kind {
                DownloadKind::Video { max_height: Some(h) } => format!("Video ≤{h}p"),
                DownloadKind::Video { max_height: None } => "Video".into(),
                DownloadKind::Audio(a) => format!("Audio {}", a.ext().to_uppercase()),
            };
            egui::Frame::new()
                .fill(SURFACE)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(CornerRadius::same(12))
                .inner_margin(egui::Margin::same(12))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(112.0, 63.0), Sense::hover());
                        widgets::thumb(app, ui, Some(&thumb), r, 8);
                        ui.add_space(4.0);
                        ui.vertical(|ui| {
                            ui.add(egui::Label::new(RichText::new(&title).font(heading_font(14.0))).truncate());
                            let (line, color) = match &status {
                                JobStatus::Queued => ("Queued".to_string(), TEXT_DIM),
                                JobStatus::Resolving => ("Preparing…".into(), TEXT_DIM),
                                JobStatus::Downloading => {
                                    let eta = if bps > 0 && total > done {
                                        format!("  ·  {} left", fmt_time(((total - done) / bps) as f64))
                                    } else {
                                        String::new()
                                    };
                                    (format!("{} of {}  ·  {}/s{eta}", fmt_bytes(done), fmt_bytes(total), fmt_bytes(bps)), TEXT_DIM)
                                }
                                JobStatus::Muxing => ("Merging…".into(), TEXT_DIM),
                                JobStatus::Paused => (format!("Paused  ·  {} of {}", fmt_bytes(done), fmt_bytes(total)), TEXT_DIM),
                                JobStatus::Done(p) => (format!("Saved to {}", p.display()), SUCCESS),
                                JobStatus::Failed(e) => (format!("Failed: {e}"), DANGER),
                                JobStatus::Cancelled => ("Cancelled".into(), TEXT_DIM),
                            };
                            ui.label(RichText::new(format!("{kind}  ·  {line}")).color(color).small());
                            if matches!(status, JobStatus::Downloading | JobStatus::Paused | JobStatus::Muxing) {
                                let frac = if total > 0 { done as f32 / total as f32 } else { 0.0 };
                                let frac = motion::value(ui.ctx(), egui::Id::unique(("dl", id)), frac, motion::EXPAND);
                                widgets::progress(ui, frac, 6.0);
                            }
                            ui.horizontal(|ui| match &status {
                                s if s.is_active() => {
                                    if ui.small_button("Pause").clicked() {
                                        app.core.send(Command::PauseDownload(id));
                                    }
                                    if ui.small_button("Cancel").clicked() {
                                        app.core.send(Command::CancelDownload(id));
                                    }
                                }
                                JobStatus::Paused | JobStatus::Failed(_) => {
                                    if ui.small_button("Resume").clicked() {
                                        app.core.send(Command::ResumeDownload(id));
                                    }
                                    if ui.small_button("Cancel").clicked() {
                                        app.core.send(Command::CancelDownload(id));
                                    }
                                }
                                JobStatus::Done(p) => {
                                    if ui.small_button("Open").clicked() {
                                        app.platform.open_path(p);
                                    }
                                    if let Some(dir) = p.parent() {
                                        if app.platform.is_desktop() && ui.small_button("Show in folder").clicked() {
                                            app.platform.open_path(dir);
                                        }
                                    }
                                }
                                _ => {}
                            });
                        });
                    });
                });
            ui.add_space(8.0);
        }
    });
}

// ------------------------------------------------------------------ settings

fn settings(app: &mut App, ui: &mut Ui) {
    let Some(mut s) = app.settings.clone() else {
        widgets::skeleton_row(ui);
        return;
    };
    let before = s.clone();
    let desktop = app.platform.is_desktop();
    let ctx = ui.ctx().clone();
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        ui.set_max_width(820.0);
        heading(ui, "Settings", "Changes are saved automatically.");

        widgets::card(ui, "Playback", "", |ui| {
            widgets::setting_row(ui, "Quality", "Auto picks the best quality your player size and connection sustain.", |ui| {
                let label = match s.quality {
                    QualityPref::Auto => "Auto".to_string(),
                    QualityPref::Max(h) => format!("Up to {h}p"),
                };
                egui::ComboBox::from_id_salt("set-quality").selected_text(label).width(130.0).show_ui(ui, |ui| {
                    if ui.selectable_label(s.quality == QualityPref::Auto, "Auto").clicked() {
                        s.quality = QualityPref::Auto;
                    }
                    for h in [2160, 1440, 1080, 720, 480, 360] {
                        if ui.selectable_label(s.quality == QualityPref::Max(h), format!("Up to {h}p")).clicked() {
                            s.quality = QualityPref::Max(h);
                        }
                    }
                });
            });
            widgets::setting_row(ui, "Video codec", "H.264 is the most compatible; VP9 and AV1 unlock 1440p and 4K when your GPU decodes them.", |ui| {
                egui::ComboBox::from_id_salt("set-codec")
                    .selected_text(match s.codec {
                        CodecPref::Auto => "Auto (best for this GPU)",
                        CodecPref::H264 => "H.264",
                        CodecPref::Vp9 => "VP9",
                        CodecPref::Av1 => "AV1",
                    })
                    .width(180.0)
                    .show_ui(ui, |ui| {
                        for (c, l) in [(CodecPref::Auto, "Auto (best for this GPU)"), (CodecPref::H264, "H.264"), (CodecPref::Vp9, "VP9"), (CodecPref::Av1, "AV1")] {
                            if ui.selectable_label(s.codec == c, l).clicked() {
                                s.codec = c;
                            }
                        }
                    });
            });
            widgets::setting_row(ui, "Default speed", "", |ui| {
                egui::ComboBox::from_id_salt("set-speed").selected_text(format!("{}×", s.speed)).width(80.0).show_ui(ui, |ui| {
                    for sp in SPEEDS {
                        if ui.selectable_label((sp - s.speed).abs() < 0.01, format!("{sp}×")).clicked() {
                            s.speed = sp;
                        }
                    }
                });
            });
            widgets::setting_row(ui, "Autoplay", "Play the next video in “Up next” when one ends.", |ui| {
                ui.checkbox(&mut s.autoplay, "");
            });
            widgets::setting_row(ui, "Pre-load videos", "Prepare videos you hover and the next one in line, so they start faster.", |ui| {
                ui.checkbox(&mut s.preload, "");
            });
            widgets::setting_row(ui, "Start in audio-only mode", "", |ui| {
                ui.checkbox(&mut s.audio_only, "");
            });
        });
        ui.add_space(12.0);

        widgets::card(ui, "Downloads", "", |ui| {
            let dir = app.status.as_ref().map(|st| st.download_dir.display().to_string()).unwrap_or_default();
            widgets::setting_row(ui, "Save to", &dir, |ui| {
                if desktop && ui.button("Change…").clicked() {
                    app.file_dialog(FileDialog::PickFolder, &ctx);
                }
                if s.download_dir.is_some() && ui.button("Reset").clicked() {
                    s.download_dir = None;
                }
            });
            widgets::setting_row(ui, "Audio format", "Used for one-click audio downloads.", |ui| {
                ui.selectable_value(&mut s.audio_out, AudioOut::Opus, "Opus");
                ui.selectable_value(&mut s.audio_out, AudioOut::M4a, "M4A");
                if desktop {
                    ui.selectable_value(&mut s.audio_out, AudioOut::Mp3, "MP3");
                }
            });
            widgets::setting_row(ui, "Connections in total", "Across all downloads (max 16).", |ui| {
                ui.add(egui::Slider::new(&mut s.max_connections, 1..=16));
            });
            let maxc = s.max_connections;
            widgets::setting_row(ui, "Pieces per file at once", "", |ui| {
                ui.add(egui::Slider::new(&mut s.chunks_per_stream, 1..=maxc));
            });
            widgets::setting_row(ui, "Videos at once", "", |ui| {
                ui.add(egui::Slider::new(&mut s.parallel_videos, 1..=maxc));
            });
            if desktop {
                let ff = app.status.as_ref().is_some_and(|s| s.ffmpeg);
                widgets::setting_row(ui, "FFmpeg", "Merges video with audio and converts MP3.", |ui| {
                    if ff {
                        ui.label(RichText::new("Installed").color(SUCCESS));
                    } else if ui.button("Install").clicked() {
                        app.core.send(Command::InstallFfmpeg);
                    }
                });
            }
        });
        ui.add_space(12.0);

        widgets::card(ui, "Appearance", "", |ui| {
            widgets::setting_row(ui, "Interface size", "", |ui| {
                ui.add(egui::Slider::new(&mut s.ui_scale, 0.75..=1.75).step_by(0.05).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
            });
            let mut follow_os = s.reduced_motion.is_none();
            widgets::setting_row(ui, "Animations", "Follow the system setting, or choose.", |ui| {
                if let Some(r) = &mut s.reduced_motion {
                    let mut on = !*r;
                    if ui.checkbox(&mut on, "On").changed() {
                        *r = !on;
                    }
                }
                ui.checkbox(&mut follow_os, "System");
            });
            if follow_os != s.reduced_motion.is_none() {
                s.reduced_motion = if follow_os { None } else { Some(motion::reduced()) };
            }
        });
        ui.add_space(12.0);

        widgets::card(ui, "Account", "", |ui| {
            match app.auth.method {
                Some(m) => {
                    widgets::setting_row(
                        ui,
                        "Signed in",
                        match m {
                            SignInMethod::Device => "with a code",
                            SignInMethod::Cookies => "with Google",
                        },
                        |ui| {
                            if ui.button("Sign out").clicked() {
                                app.core.send(Command::Logout);
                            }
                        },
                    );
                }
                None => {
                    widgets::setting_row(ui, "Not signed in", "Subscriptions, history and playlists need an account.", |ui| {
                        if widgets::primary(ui, "Sign in").clicked() {
                            app.auth.dialog_open = true;
                        }
                    });
                }
            }
            widgets::setting_row(ui, "Allow sign-in with a code", "Sign in by typing a short code at youtube.com/activate.", |ui| {
                ui.checkbox(&mut s.tv_sign_in, "");
            });
            if !app.remote.device_login {
                ui.label(RichText::new("Code sign-in is temporarily disabled.").small().color(TEXT_DIM));
            }
        });
        ui.add_space(12.0);

        widgets::card(ui, "Advanced", "", |ui| {
            let yv = app.status.as_ref().and_then(|s| s.ytdlp.clone());
            widgets::setting_row(ui, "yt-dlp", yv.as_deref().map(|v| format!("Version {v}")).as_deref().unwrap_or("Not found"), |ui| {
                if yv.is_some() && ui.button("Update").on_hover_text("Get the latest fixes for YouTube changes").clicked() {
                    app.core.send(Command::UpdateYtDlp);
                }
            });
            widgets::setting_row(ui, "Cache", "Thumbnails and stream data.", |ui| {
                if ui.button("Clear cache").clicked() {
                    app.thumbs.clear();
                    app.core.send(Command::ClearCache);
                }
            });
            widgets::setting_row(ui, "Diagnostics", "Recent logs with tokens and cookies removed, for bug reports.", |ui| {
                if ui.button("Export…").clicked() {
                    app.file_dialog(FileDialog::SaveDiagnostics, &ctx);
                }
            });
            widgets::setting_row(ui, "Check for updates on launch", "", |ui| {
                ui.checkbox(&mut s.check_updates, "");
            });
        });
        ui.add_space(12.0);

        if let Some(st) = &app.status {
            ui.label(
                RichText::new(format!(
                    "StreamPipe {}  ·  PO token helper: {}",
                    st.version,
                    st.botguard.clone().unwrap_or_else(|| "not found".into())
                ))
                .small()
                .color(TEXT_DIM),
            );
        }
        ui.add_space(20.0);
    });
    if s.chunks_per_stream > s.max_connections {
        s.chunks_per_stream = s.max_connections;
    }
    if s.parallel_videos > s.max_connections {
        s.parallel_videos = s.max_connections;
    }
    if format!("{before:?}") != format!("{s:?}") {
        if before.reduced_motion != s.reduced_motion {
            motion::set_reduced(s.reduced_motion.unwrap_or_else(|| app.platform.os_reduced_motion()));
        }
        app.settings = Some(s.clone());
        app.core.send(Command::UpdateSettings(s));
    }
}

// ------------------------------------------------------------------ dialogs

fn auth_dialog(app: &mut App, ctx: &egui::Context) {
    let open = app.auth.dialog_open && app.auth.method.is_none();
    let t = motion::toggle(ctx, egui::Id::unique("auth-dialog"), open, motion::NAV);
    if t <= 0.0 {
        if app.auth.method.is_some() {
            app.auth.dialog_open = false;
        }
        return;
    }
    let device_allowed = app.settings.as_ref().is_some_and(|s| s.tv_sign_in) && app.remote.device_login;
    let desktop = app.platform.is_desktop();
    let mut close = false;
    // Dim the app behind the dialog.
    egui::Area::new(egui::Id::unique("auth-scrim"))
        .order(egui::Order::Middle)
        .fixed_pos(egui::Pos2::ZERO)
        .interactable(true)
        .show(ctx, |ui| {
            let r = ctx.content_rect();
            ui.allocate_rect(r, Sense::click());
            ui.painter().rect_filled(r, 0.0, Color32::from_black_alpha((150.0 * t) as u8));
        });
    egui::Window::new("auth")
        .title_bar(false)
        .collapsible(false)
        .resizable(false)
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, vec2(0.0, (1.0 - t) * 20.0))
        .fade_in(false)
        .default_width(430.0)
        .frame(
            egui::Frame::new()
                .fill(SURFACE)
                .stroke(Stroke::new(1.0, BORDER))
                .corner_radius(CornerRadius::same(16))
                .inner_margin(egui::Margin::same(24))
                .shadow(egui::Shadow { offset: [0, 16], blur: 40, spread: 0, color: Color32::from_black_alpha(170) }),
        )
        .show(ctx, |ui| {
            ui.set_opacity(t);
            ui.horizontal(|ui| {
                widgets::logo(ui, 28.0);
                ui.label(RichText::new("Sign in").font(heading_font(20.0)));
            });
            ui.add_space(6.0);
            ui.label(
                RichText::new("Signing in is optional. It adds your subscriptions, history and playlists.")
                    .color(TEXT_DIM),
            );
            ui.add_space(14.0);
            if let Some((url, code)) = app.auth.device_code.clone() {
                ui.label("On this or any device, open:");
                if ui.link(RichText::new(&url).size(16.0)).clicked() {
                    app.platform.open_url(&url);
                }
                ui.add_space(4.0);
                ui.label("and enter this code:");
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&code).monospace().size(28.0).color(ACCENT));
                    if ui.button("Copy").clicked() {
                        ui.ctx().copy_text(code.clone());
                    }
                });
                ui.add_space(4.0);
                ui.label(RichText::new("Waiting for you to approve…").color(TEXT_DIM));
                if ui.button("Cancel").clicked() {
                    app.core.send(Command::CancelLogin);
                }
            } else if app.auth.browser_open {
                ui.label("Finish signing in in the Google window that just opened.");
                ui.label(RichText::new("It closes by itself when you're signed in.").color(TEXT_DIM));
            } else if app.auth.checking {
                ui.label(RichText::new("Checking your account…").color(TEXT_DIM));
            } else {
                ui.vertical_centered_justified(|ui| {
                    if app.platform.supports_browser_sign_in() && widgets::primary(ui, "Sign in with Google").clicked() {
                        app.browser_sign_in(ctx);
                    }
                    if device_allowed {
                        ui.add_space(6.0);
                        let code_btn = if app.platform.supports_browser_sign_in() {
                            widgets::ghost(ui, "Sign in with a code instead")
                        } else {
                            widgets::primary(ui, "Sign in with a code")
                        };
                        if code_btn.clicked() && !app.auth.waiting {
                            app.auth.waiting = true;
                            app.auth.error = None;
                            app.core.send(Command::StartDeviceLogin);
                        }
                        if app.auth.waiting {
                            ui.label(RichText::new("Requesting a code…").color(TEXT_DIM));
                        }
                    }
                    if desktop {
                        ui.add_space(6.0);
                        if widgets::ghost(ui, "Import cookies.txt…").clicked() {
                            app.file_dialog(FileDialog::OpenCookies, ctx);
                        }
                    }
                });
                if !device_allowed && !app.platform.supports_browser_sign_in() {
                    ui.label(RichText::new("Turn on \"Allow sign-in with a code\" in Settings to sign in.").color(TEXT_DIM));
                }
            }
            if let Some(e) = &app.auth.error {
                ui.add_space(8.0);
                ui.label(RichText::new(e).color(DANGER));
            }
            ui.add_space(12.0);
            ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        });
    if close || ctx.input(|i| i.key_pressed(Key::Escape)) {
        if app.auth.waiting {
            app.core.send(Command::CancelLogin);
        }
        app.auth.dialog_open = false;
        app.auth.waiting = false;
    }
}

fn toasts(app: &mut App, ctx: &egui::Context) {
    if app.toasts.is_empty() {
        return;
    }
    let bottom = if app.form == FormFactor::Phone { -140.0 } else { -28.0 };
    egui::Area::new(egui::Id::unique("toasts"))
        .anchor(egui::Align2::CENTER_BOTTOM, vec2(0.0, bottom))
        .order(egui::Order::Tooltip)
        .interactable(false)
        .show(ctx, |ui| {
            for (msg, t) in &app.toasts {
                let age = Instant::now().duration_since(*t).as_secs_f32();
                let fade_in = if motion::reduced() { 1.0 } else { (age / motion::NAV).min(1.0) };
                if fade_in < 1.0 {
                    ctx.request_repaint();
                }
                egui::Frame::new()
                    .fill(SURFACE_HI)
                    .stroke(Stroke::new(1.0, ACCENT.linear_multiply(0.45)))
                    .corner_radius(CornerRadius::same(12))
                    .inner_margin(egui::Margin::symmetric(16, 10))
                    .multiply_with_opacity(fade_in)
                    .show(ui, |ui| ui.label(msg));
                ui.add_space(6.0);
            }
        });
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}
