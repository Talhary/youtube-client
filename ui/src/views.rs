use std::time::Instant;

use egui::{Align, Color32, CornerRadius, FontId, Key, Layout, RichText, Sense, Stroke, Ui, vec2};
use streampipe_core::Command;
use streampipe_core::auth::SignInMethod;
use streampipe_core::config::{AudioOut, CodecPref, QualityPref};
use streampipe_core::download::{DownloadKind, JobStatus};
use streampipe_core::model::{Item, fmt_count};

use crate::app::{App, LibraryTab, ListRef, Page, Tab};
use crate::extra::ChannelTab;
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
    crate::watch::dialogs(app, &ctx);
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
                crate::watch::player_pane(app, ui);
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
                crate::watch::player_pane(app, ui);
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
        if widgets::logo(ui, 30.0).on_hover_text("Switch between YouTube and YouTube Music").clicked() {
            let on = !app.store.saved.music_mode;
            app.set_music_mode(on);
        }
        if !compact {
            ui.add_space(4.0);
            let name = if app.store.saved.music_mode { "Music" } else { "StreamPipe" };
            ui.label(RichText::new(name).font(heading_font(18.0)));
        }
    });
    ui.add_space(10.0);
    let music = app.store.saved.music_mode;
    if !compact {
        if let Some(i) = widgets::pills(ui, &["YouTube", "Music"], usize::from(music)) {
            app.set_music_mode(i == 1);
        }
    }
    ui.add_space(12.0);
    let downloading = app.downloads.values().filter(|d| d.status.is_active()).count();
    let items = [
        (Tab::Home, Icon::Home, "Home", None),
        (Tab::Search, Icon::Search, "Search", None),
        (Tab::Subscriptions, Icon::Subscriptions, if music { "Explore" } else { "Subscriptions" }, None),
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
        let right = if phone { 90.0 } else { 60.0 };
        let w = (ui.available_width() - right).min(620.0);
        search_box(app, ui, w);
        if !phone && app.signed_in() && !app.store.saved.music_mode {
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.add(egui::Button::new(RichText::new("🔔").size(18.0)).frame(false)).on_hover_text("Notifications").clicked() {
                    app.open_notifications();
                }
            });
        }
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
                        .hint_text(if app.store.saved.music_mode { "Search songs, albums and artists   /" } else { "Search videos, channels and playlists   /" })
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
    // Suggestions while typing; the search history when the box is empty.
    let history = app.search_text.trim().is_empty() && !app.store.saved.search_history_off;
    let shown: Vec<String> = if history { app.store.saved.searches.iter().take(10).cloned().collect() } else { app.suggestions.clone() };
    if resp.has_focus() && !shown.is_empty() {
        let mut pick = None;
        let mut forget = None;
        egui::Popup::from_response(&resp)
            .open(true)
            .width(width)
            .show(|ui| {
                for s in &shown {
                    ui.horizontal(|ui| {
                        let icon = if history { "🕘  " } else { "🔍  " };
                        if ui.add(egui::Button::new(RichText::new(format!("{icon}{s}"))).frame(false)).clicked() {
                            pick = Some(s.clone());
                        }
                        if history {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if ui.small_button("✖").on_hover_text("Remove from history").clicked() {
                                    forget = Some(s.clone());
                                }
                            });
                        }
                    });
                }
            });
        if let Some(f) = forget {
            app.store.saved.searches.retain(|x| *x != f);
            app.store.touch();
        }
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
    if app.store.saved.music_mode && matches!(app.tab, Tab::Home | Tab::Search | Tab::Subscriptions | Tab::Library) {
        crate::music::browse(app, ui);
        return;
    }
    match app.tab {
        Tab::Home => {
            heading(ui, "Home", "");
            let labels: Vec<&str> = crate::extra::EXPLORE.iter().map(|e| e.0).collect();
            if let Some(i) = widgets::pills(ui, &labels, app.home_chip) {
                app.explore(i);
            }
            ui.add_space(8.0);
            list_view(app, ui, ListRef::Home, "Nothing here yet", true);
        }
        Tab::Search => {
            if app.search_submitted.is_empty() {
                widgets::centered_message(ui, "Search YouTube", "Type in the search box above and press Enter.");
            } else {
                heading(ui, &format!("Results for “{}”", app.search_submitted), "");
                if search_filters(app, ui) {
                    app.search_text = app.search_submitted.clone();
                    app.submit_search();
                }
                ui.add_space(6.0);
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
            ui.horizontal(|ui| {
                if let Some(i) = widgets::pills(ui, &["History", "Playlists", "Channels"], current) {
                    app.library_tab = tabs[i];
                    app.load_library();
                    if tabs[i] == LibraryTab::History {
                        app.load_history_actions();
                    }
                }
                ui.add_space(10.0);
                if widgets::ghost(ui, "🕘 Watch later").clicked() {
                    app.open_playlist("WL".into());
                }
                if widgets::ghost(ui, "👍 Liked videos").clicked() {
                    app.open_playlist("LL".into());
                }
            });
            if app.library_tab == LibraryTab::History {
                if app.history_actions.is_empty() && !app.history.loaded_once {
                    app.load_history_actions();
                }
                let actions = app.history_actions.clone();
                if !actions.is_empty() {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        for (kind, token) in actions {
                            let label = match kind.as_str() {
                                "clear" => "Clear all watch history",
                                "pause" => "Pause watch history",
                                _ => "Turn on watch history",
                            };
                            if widgets::ghost(ui, label).clicked() {
                                app.history_action(kind.clone(), token.clone());
                            }
                        }
                    });
                }
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
        Some(Page::Music(v)) => {
            let key = v.key;
            crate::music::stack_page(app, ui, key);
            return;
        }
        Some(Page::Notifications { .. }) => {
            heading(ui, "Notifications", "");
            list_view(app, ui, ListRef::Stack, "No notifications", true);
            return;
        }
        Some(Page::Channel { header, error, .. }) => {
            if let Some(e) = error {
                error_box(app, ui, e.clone(), ListRef::Stack);
                return;
            }
            match header {
                Some(c) => {
                    let avatar = c.channel.avatar.clone();
                    let banner = c.banner.clone();
                    let name = c.channel.name.clone();
                    let subs = c.channel.subscribers;
                    let cid = c.channel.id.clone();
                    let mut line = Vec::new();
                    if let Some(h) = &c.handle {
                        line.push(h.clone());
                    }
                    if let Some(n) = subs {
                        line.push(format!("{} subscribers", fmt_count(n)));
                    }
                    if let Some(n) = c.video_count {
                        line.push(format!("{} videos", fmt_count(n)));
                    }
                    let verified = c.verified;
                    if let Some(b) = banner {
                        let w = ui.available_width();
                        let (r, _) = ui.allocate_exact_size(vec2(w, (w / 6.2).min(150.0)), Sense::hover());
                        widgets::thumb(app, ui, Some(&b), r, 12);
                    }
                    ui.add_space(6.0);
                    let sub = app.subs_state.get(&cid).cloned();
                    let mut sub_click = None;
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(72.0, 72.0), Sense::hover());
                        widgets::thumb(app, ui, avatar.as_deref(), r, 255);
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.add_space(4.0);
                            let title = if verified { format!("{name}  ✔") } else { name.clone() };
                            ui.label(RichText::new(title).font(heading_font(24.0)));
                            ui.label(RichText::new(line.join("  ·  ")).color(TEXT_DIM));
                            ui.add_space(4.0);
                            let on = sub.as_ref().and_then(|v| v.get("subscribed")).and_then(|v| v.as_bool()).unwrap_or(false);
                            ui.horizontal(|ui| {
                                let resp = if on { widgets::ghost(ui, "Subscribed") } else { widgets::primary(ui, "Subscribe") };
                                if resp.clicked() {
                                    sub_click = Some((cid.clone(), !on, None));
                                }
                                if on {
                                    let bells: Vec<(String, String)> = sub
                                        .as_ref()
                                        .and_then(|v| v.get("bell"))
                                        .and_then(|v| v.as_array())
                                        .into_iter()
                                        .flatten()
                                        .filter_map(|b| Some((b.get("label")?.as_str()?.to_string(), b.get("params")?.as_str()?.to_string())))
                                        .collect();
                                    if !bells.is_empty() {
                                        ui.menu_button("🔔", |ui| {
                                            for (label, params) in &bells {
                                                if ui.button(label).clicked() {
                                                    sub_click = Some((cid.clone(), true, Some((params.clone(), label.clone()))));
                                                    ui.close();
                                                }
                                            }
                                        });
                                    }
                                }
                            });
                        });
                    });
                    match sub_click {
                        Some((c, _, Some((params, label)))) => app.set_bell(c, params, label),
                        Some((c, on, None)) => app.subscribe(c, on),
                        None => {}
                    }
                    ui.add_space(8.0);
                    let tab = match app.stack.last() {
                        Some(Page::Channel { tab, .. }) => *tab,
                        _ => ChannelTab::Videos,
                    };
                    let labels: Vec<&str> = ChannelTab::ALL.iter().map(|t| t.label()).collect();
                    let current = ChannelTab::ALL.iter().position(|t| *t == tab).unwrap_or(0);
                    if let Some(i) = widgets::pills(ui, &labels, current) {
                        app.channel_tab(ChannelTab::ALL[i]);
                    }
                    if tab == ChannelTab::About {
                        about(app, ui);
                        return;
                    }
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
    let channel_click: Option<String> = None;
    let mut item_act = None;
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
                        crate::watch::video_menu(ui, v, which == ListRef::Home, &mut item_act);
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
    if let Some(a) = item_act {
        app.run_item_act(a);
    }
    if let Some(item) = open {
        app.open_item(&item);
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

        widgets::card(ui, "Search and history", "", |ui| {
            widgets::setting_row(ui, "Search history", "Show recent searches under the search box.", |ui| {
                let mut on = !app.store.saved.search_history_off;
                if ui.checkbox(&mut on, "").changed() {
                    app.store.saved.search_history_off = !on;
                    app.store.touch();
                }
                if ui.small_button("Clear").clicked() {
                    app.store.saved.searches.clear();
                    app.store.touch();
                }
            });
            widgets::setting_row(ui, "Resume positions", "Videos continue where you left them.", |ui| {
                if ui.small_button("Forget all").clicked() {
                    app.store.saved.resume.clear();
                    app.store.touch();
                }
            });
        });
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


/// Search filters (upload date, type, length, order, live). Answers whether one changed.
fn search_filters(app: &mut App, ui: &mut Ui) -> bool {
    let f = &mut app.search_filter;
    let before = f.clone();
    ui.horizontal_wrapped(|ui| {
        let combo = |ui: &mut Ui, id: &str, label: &str, value: &mut Option<String>, options: &[(&str, &str)]| {
            let shown = options
                .iter()
                .find(|(k, _)| value.as_deref() == Some(*k))
                .map(|(_, l)| l.to_string())
                .unwrap_or_else(|| label.to_string());
            egui::ComboBox::from_id_salt(id).selected_text(shown).width(120.0).show_ui(ui, |ui| {
                if ui.selectable_label(value.is_none(), format!("Any {}", label.to_lowercase())).clicked() {
                    *value = None;
                }
                for (k, l) in options {
                    if ui.selectable_label(value.as_deref() == Some(*k), *l).clicked() {
                        *value = Some(k.to_string());
                    }
                }
            });
        };
        combo(ui, "f-date", "Upload date", &mut f.date, &[("hour", "Last hour"), ("day", "Today"), ("week", "This week"), ("month", "This month"), ("year", "This year")]);
        combo(ui, "f-kind", "Type", &mut f.kind, &[("video", "Videos"), ("channel", "Channels"), ("playlist", "Playlists")]);
        combo(ui, "f-length", "Duration", &mut f.length, &[("short", "Under 4 minutes"), ("medium", "4–20 minutes"), ("long", "Over 20 minutes")]);
        combo(ui, "f-sort", "Sort", &mut f.sort, &[("date", "Upload date"), ("views", "View count"), ("rating", "Rating")]);
        ui.toggle_value(&mut f.live, "Live");
        if !f.is_empty() && ui.small_button("Clear filters").clicked() {
            *f = Default::default();
        }
    });
    let f = &app.search_filter;
    f.date != before.date || f.kind != before.kind || f.length != before.length || f.sort != before.sort || f.live != before.live
}

/// A channel's About tab.
fn about(app: &mut App, ui: &mut Ui) {
    let Some(Page::Channel { about, .. }) = app.stack.last() else { return };
    let Some(a) = about.clone() else {
        ui.add_space(10.0);
        widgets::skeleton_row(ui);
        return;
    };
    let text = |k: &str| a.get(k).and_then(|v| v.as_str()).map(str::to_string);
    let num = |k: &str| a.get(k).and_then(|v| v.as_u64());
    let mut open = None;
    egui::ScrollArea::vertical().auto_shrink([false, false]).id_salt("about").show(ui, |ui| {
        ui.add_space(10.0);
        if let Some(e) = text("error") {
            ui.label(RichText::new(e).color(TEXT_DIM));
            return;
        }
        if let Some(d) = text("description").filter(|d| !d.is_empty()) {
            ui.label(RichText::new("Description").font(heading_font(16.0)));
            ui.label(d);
            ui.add_space(10.0);
        }
        let links: Vec<(String, String)> = a
            .get("links")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
            .filter_map(|l| Some((l.get("name")?.as_str()?.to_string(), l.get("url")?.as_str()?.to_string())))
            .collect();
        if !links.is_empty() {
            ui.label(RichText::new("Links").font(heading_font(16.0)));
            for (name, url) in links {
                if ui.link(format!("{name}  ↗")).on_hover_text(&url).clicked() {
                    open = Some(url);
                }
            }
            ui.add_space(10.0);
        }
        ui.label(RichText::new("More info").font(heading_font(16.0)));
        if let Some(n) = num("subscribers") {
            ui.label(format!("{} subscribers", fmt_count(n)));
        }
        if let Some(n) = num("videos") {
            ui.label(format!("{} videos", fmt_count(n)));
        }
        if let Some(n) = num("views") {
            ui.label(format!("{} views", fmt_count(n)));
        }
        if let Some(j) = text("joined") {
            ui.label(format!("Joined {j}"));
        }
        if let Some(c) = text("country") {
            ui.label(c);
        }
    });
    if let Some(u) = open {
        app.platform.open_url(&u);
    }
}
