//! The watch page: the video (with ambient glow and captions), its controls (seek bar with
//! chapters and preview pictures, captions, sound track, loop, stable volume, sleep timer),
//! the video's details and actions, and the tabs under it (Up next with the queue,
//! comments, chapters, live chat; lyrics and related songs for YouTube Music).

use egui::{Align, Color32, CornerRadius, FontId, Layout, Rect, RichText, Sense, Stroke, Ui, vec2};
use serde_json::Value;
use streampipe_core::Command;
use streampipe_core::config::AudioOut;
use streampipe_core::download::{DownloadKind, DownloadRequest};
use streampipe_core::model::{Item, VideoCard, fmt_count};

use crate::app::{App, ListRef, Thumb};
use crate::extra::{Dialog, Sleep, WatchTab, apply_audio};
use crate::theme::*;
use crate::widgets::{self, fmt_time};
use crate::{FormFactor, motion};

const SPEEDS: [f32; 8] = [0.25, 0.5, 0.75, 1.0, 1.25, 1.5, 1.75, 2.0];

// ------------------------------------------------------------------ item menu

/// What the "⋮" menu of a video asks for.
pub(crate) enum ItemAct {
    PlayNext(VideoCard),
    Queue(VideoCard),
    WatchLater(String),
    SaveTo(String),
    NotInterested(String),
    Channel(String),
    Download(VideoCard),
    Tv(String),
}

/// The menu of a video in any list (right click).
pub(crate) fn video_menu(ui: &mut Ui, v: &VideoCard, home: bool, out: &mut Option<ItemAct>) {
    let pick = |ui: &mut Ui, label: &str, a: ItemAct, out: &mut Option<ItemAct>| {
        if ui.button(label).clicked() {
            *out = Some(a);
            ui.close();
        }
    };
    pick(ui, "Play next", ItemAct::PlayNext(v.clone()), out);
    pick(ui, "Add to queue", ItemAct::Queue(v.clone()), out);
    ui.separator();
    pick(ui, "Save to Watch later", ItemAct::WatchLater(v.id.clone()), out);
    pick(ui, "Save to playlist…", ItemAct::SaveTo(v.id.clone()), out);
    pick(ui, "Download", ItemAct::Download(v.clone()), out);
    pick(ui, "Play on TV…", ItemAct::Tv(v.id.clone()), out);
    if ui.button("Copy link").clicked() {
        ui.ctx().copy_text(format!("https://youtu.be/{}", v.id));
        ui.close();
    }
    if let Some(c) = &v.channel_id {
        pick(ui, "Go to channel", ItemAct::Channel(c.clone()), out);
    }
    if home {
        ui.separator();
        pick(ui, "Not interested", ItemAct::NotInterested(v.id.clone()), out);
    }
}

impl App {
    pub(crate) fn run_item_act(&mut self, a: ItemAct) {
        match a {
            ItemAct::PlayNext(v) => self.queue_add(v, false, true),
            ItemAct::Queue(v) => self.queue_add(v, false, false),
            ItemAct::WatchLater(id) => self.watch_later(id),
            ItemAct::SaveTo(id) => self.open_save(id),
            ItemAct::NotInterested(id) => self.not_interested(id),
            ItemAct::Channel(id) => self.open_channel(id),
            ItemAct::Tv(id) => self.open_tv(id, 0),
            ItemAct::Download(v) => {
                self.core.send(Command::Download(DownloadRequest {
                    video_id: v.id,
                    title: v.title,
                    kind: DownloadKind::Video { max_height: None },
                    group: None,
                }));
                self.toast("Added to downloads");
            }
        }
    }

    /// Audio only for videos; the Song/Video switch for YouTube Music.
    pub(crate) fn toggle_audio_only(&mut self) {
        self.audio_only = !self.audio_only;
        if self.now.as_ref().is_some_and(|n| n.music) {
            self.store.saved.music_video = !self.audio_only;
            self.store.touch();
        } else {
            self.video_audio_only = self.audio_only;
        }
        self.reload_stream();
    }

    /// Captions off → each track in turn → off (the C key).
    pub(crate) fn cycle_captions(&mut self) {
        let Some(n) = &self.now else { return };
        let count = n.extras.as_ref().map(|e| e.captions.len()).unwrap_or(0);
        if count == 0 {
            self.toast("No captions for this video");
            return;
        }
        let next = match n.caption {
            None => Some(0),
            Some(i) if i + 1 < count => Some(i + 1),
            Some(_) => None,
        };
        self.choose_caption(next, true);
        let name = next.and_then(|i| self.now.as_ref()?.extras.as_ref()?.captions.get(i).map(|t| t.name.clone()));
        self.toast(match name {
            Some(n) => format!("Captions: {n}"),
            None => "Captions off".into(),
        });
    }

    /// Switch the sound (dubbed track or stable volume) and continue where playback is.
    fn reapply_audio(&mut self) {
        let stable = self.store.saved.stable_volume;
        let Some(n) = &mut self.now else { return };
        let Some(info) = &mut n.info else { return };
        let level = apply_audio(info, &n.audio_orig, n.audio_track, stable);
        self.player.set_leveller(level);
        self.reload_stream();
    }
}

// ------------------------------------------------------------------ player pane

pub(crate) fn player_pane(app: &mut App, ui: &mut Ui) {
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
    let max_h = if app.theater { avail.y - 120.0 } else { avail.y * 0.58 };
    let mut h = w * 9.0 / 16.0;
    if h > max_h {
        h = max_h.max(120.0);
        w = h * 16.0 / 9.0;
    }
    ui.add_space(4.0);
    let (outer, resp) = ui.allocate_exact_size(vec2(avail.x, h), Sense::click());
    let rect = Rect::from_center_size(outer.center(), vec2(w, h));
    app.video_px_height = (h * ui.ctx().pixels_per_point()) as u32;

    let Some(n) = &app.now else { return };
    let resolving = n.resolve_req.is_some() && n.error.is_none();
    let error = n.error.clone();
    let thumb = n.card.thumbnail.clone();
    let waited = n.requested_at.elapsed().as_secs();
    let music = n.music;

    // Ambient mode: the picture's colours glow softly around the player.
    if !app.store.saved.ambient_off && !app.theater && !phone {
        if let Some(Thumb::Ready(tex)) = thumb.as_deref().and_then(|t| app.thumbs.get(t)) {
            let id = tex.id();
            for (grow, alpha) in [(46.0, 10u8), (32.0, 16), (20.0, 22), (10.0, 30)] {
                ui.painter().image(
                    id,
                    outer.expand2(vec2(grow * 0.6, grow)),
                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    Color32::from_white_alpha(alpha),
                );
            }
        }
    }
    ui.painter().rect_filled(outer, CornerRadius::same(14), Color32::BLACK);

    if app.audio_only || resolving {
        if music {
            // Album art, square, in the middle.
            let art = Rect::from_center_size(rect.center(), vec2(h * 0.86, h * 0.86));
            widgets::thumb(app, ui, thumb.as_deref(), art, 10);
        } else {
            widgets::thumb(app, ui, thumb.as_deref(), rect, 14);
            if app.audio_only && !resolving {
                ui.painter().rect_filled(rect, CornerRadius::same(14), Color32::from_black_alpha(150));
                ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, "♫  Audio only", heading_font(20.0), TEXT);
            }
        }
    }
    if !app.audio_only && !resolving {
        app.player.show_video(ui, rect);
    }
    // Captions.
    if !resolving {
        if let Some(text) = app.caption_at(st.position_s) {
            let size = (h / 20.0).clamp(14.0, 34.0);
            let g = ui.painter().layout(text, FontId::proportional(size), Color32::WHITE, rect.width() * 0.8);
            let pos = egui::pos2(rect.center().x - g.size().x / 2.0, rect.bottom() - g.size().y - h * 0.08);
            let bg = Rect::from_min_size(pos, g.size()).expand2(vec2(8.0, 4.0));
            ui.painter().rect_filled(bg, CornerRadius::same(4), Color32::from_black_alpha(190));
            ui.painter().galley(pos, g, Color32::WHITE);
        }
    }
    let preparing = format!("Preparing video…  {waited}s");
    let overlay = if resolving {
        ui.ctx().request_repaint_after(std::time::Duration::from_millis(500));
        Some(preparing.as_str())
    } else if st.buffering && !st.idle {
        Some("Buffering…")
    } else {
        None
    };
    if let Some(text) = overlay {
        let pill = Rect::from_center_size(rect.center(), vec2(text.len() as f32 * 8.0 + 30.0, 34.0));
        ui.painter().rect_filled(pill, CornerRadius::same(17), Color32::from_black_alpha(190));
        ui.painter().text(pill.center(), egui::Align2::CENTER_CENTER, text, FontId::proportional(14.0), TEXT);
    }
    if let Some(e) = &error {
        ui.painter().rect_filled(rect, CornerRadius::same(14), Color32::from_black_alpha(210));
        ui.painter().text(rect.center() - vec2(0.0, 10.0), egui::Align2::CENTER_CENTER, "Playback failed", heading_font(16.0), TEXT);
        let short: String = e.chars().take(90).collect();
        ui.painter().text(rect.center() + vec2(0.0, 14.0), egui::Align2::CENTER_CENTER, short, FontId::proportional(12.0), TEXT_DIM);
    }
    if resp.clicked() && !st.idle {
        app.player.set_paused(!st.paused);
    }
    if resp.double_clicked() && !phone {
        app.theater = !app.theater;
    }

    controls(app, ui, &st, error.is_some());
    if app.theater {
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("watch-scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            info_block(app, ui, &st);
            ui.add_space(8.0);
            tabs(app, ui, &st);
        });
}

/// Seek bar with chapter gaps; hovering shows the preview picture, time and chapter.
fn seek_bar(app: &mut App, ui: &mut Ui, pos: f64, dur: f64, width: f32) -> Option<f64> {
    let (rect, resp) = ui.allocate_exact_size(vec2(width.max(40.0), 22.0), Sense::click_and_drag());
    let active = resp.hovered() || resp.dragged();
    let grow = motion::toggle(ui.ctx(), resp.id.with("knob"), active, motion::HOVER);
    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0 + 2.0 * grow));
    let chapters: Vec<(u32, String)> = app
        .now
        .as_ref()
        .and_then(|n| n.details.as_ref())
        .map(|d| d.chapters.iter().map(|c| (c.start, c.title.clone())).collect())
        .unwrap_or_default();
    let p = ui.painter();
    let frac = if dur > 0.0 { (pos / dur).clamp(0.0, 1.0) } else { 0.0 } as f32;
    p.rect_filled(track, CornerRadius::same(3), BORDER);
    let mut done = track;
    done.set_width(track.width() * frac);
    p.rect_filled(done, CornerRadius::same(3), ACCENT);
    if dur > 0.0 {
        for (start, _) in chapters.iter().skip(1) {
            let x = track.left() + track.width() * (*start as f64 / dur) as f32;
            p.rect_filled(Rect::from_center_size(egui::pos2(x, track.center().y), vec2(3.0, track.height() + 2.0)), 0.0, Color32::BLACK);
        }
    }
    p.circle_filled(egui::pos2(done.right(), track.center().y), 4.0 + 3.0 * grow, ACCENT);
    resp.widget_info(|| egui::WidgetInfo::slider(true, pos, format!("Position {} of {}", fmt_time(pos), fmt_time(dur))));
    if active && dur > 0.0 {
        if let Some(pt) = resp.hover_pos().or_else(|| resp.interact_pointer_pos()) {
            let at = ((pt.x - track.left()) / track.width()).clamp(0.0, 1.0) as f64 * dur;
            let chapter = chapters.iter().rev().find(|(s, _)| (*s as f64) <= at).map(|(_, t)| t.clone());
            let frame = app
                .now
                .as_ref()
                .and_then(|n| n.extras.as_ref()?.storyboard.as_ref())
                .and_then(|sb| sb.frame(at, dur).map(|(u, uv)| (u, uv, sb.aspect)));
            let picture = frame.and_then(|(url, uv, aspect)| {
                app.want_thumb(&url);
                match app.thumbs.get(&url) {
                    Some(Thumb::Ready(t)) => Some((t.id(), uv, aspect)),
                    _ => None,
                }
            });
            egui::Tooltip::always_open(ui.ctx().clone(), ui.layer_id(), resp.id.with("tip"), egui::PopupAnchor::Pointer).show(|ui| {
                if let Some((tex, uv, aspect)) = picture {
                    let w = 176.0;
                    ui.add(egui::Image::new((tex, vec2(w, w / aspect))).uv(uv).corner_radius(CornerRadius::same(6)));
                }
                ui.vertical_centered(|ui| {
                    if let Some(c) = &chapter {
                        ui.label(RichText::new(c).small().strong());
                    }
                    ui.label(fmt_time(at));
                });
            });
        }
    }
    if (resp.dragged() || resp.clicked()) && dur > 0.0 {
        if let Some(pt) = resp.interact_pointer_pos() {
            let f = ((pt.x - track.left()) / track.width()).clamp(0.0, 1.0) as f64;
            return Some(f * dur);
        }
    }
    None
}

fn controls(app: &mut App, ui: &mut Ui, st: &crate::PlayerState, failed: bool) {
    let live = app.now.as_ref().and_then(|n| n.info.as_ref()).is_some_and(|i| i.hls.is_some())
        || app.now.as_ref().and_then(|n| n.extras.as_ref()).is_some_and(|e| e.live);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let playing = !(st.paused || st.idle);
        if widgets::play_button(ui, playing, !st.idle).on_hover_text("Play/pause (Space)").clicked() {
            app.player.set_paused(!st.paused);
        }
        if !app.queue.is_empty() && ui.button("⏭").on_hover_text("Next in queue (N)").clicked() {
            let q = app.queue.remove(0);
            app.play_card(q.card, q.music);
            return;
        }
        ui.label(RichText::new(fmt_time(st.position_s)).monospace().small().color(TEXT_DIM));
        let right_w = if app.form == FormFactor::Phone { 64.0 } else { 200.0 };
        let bar_w = (ui.available_width() - right_w).max(60.0);
        if let Some(p) = seek_bar(app, ui, st.position_s, st.duration_s, bar_w) {
            let expired = app.now.as_ref().and_then(|n| n.info.as_ref()).is_some_and(|i| i.expired());
            if expired {
                app.reresolve(false);
            }
            app.player.seek(p);
        }
        ui.label(RichText::new(fmt_time(st.duration_s)).monospace().small().color(TEXT_DIM));
        if live {
            let behind = st.duration_s - st.position_s > 15.0;
            let text = RichText::new("● LIVE").small().color(if behind { TEXT_DIM } else { DANGER });
            if ui.add(egui::Button::new(text).frame(false)).on_hover_text("Go to the live edge").clicked() {
                app.player.seek((st.duration_s - 3.0).max(0.0));
            }
        }
        if app.form == FormFactor::Desktop {
            let mut vol = st.volume;
            ui.spacing_mut().slider_width = 80.0;
            if ui.add(egui::Slider::new(&mut vol, 0.0..=130.0).show_value(false).text("Vol")).on_hover_text("Volume (+/−)").changed() {
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
        let music = app.now.as_ref().is_some_and(|n| n.music);
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
        let shown = current.and_then(|i| labels.get(i).cloned()).unwrap_or_else(|| "Quality".into());
        let mut pick = None;
        ui.add_enabled_ui(!labels.is_empty() && !app.audio_only, |ui| {
            egui::ComboBox::from_id_salt("quality")
                .selected_text(shown)
                .width(140.0)
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
            .width(58.0)
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
        // Song / Video (YouTube Music) or Audio only.
        if music {
            let label = if app.audio_only { "♫ Song" } else { "▶ Video" };
            if ui.button(label).on_hover_text("Switch between the song and its video (A)").clicked() {
                app.toggle_audio_only();
            }
        } else {
            let mut ao = app.audio_only;
            if ui.toggle_value(&mut ao, "Audio only").on_hover_text("Audio only (A)").changed() {
                app.toggle_audio_only();
            }
        }
        // Captions.
        let tracks: Vec<String> = app
            .now
            .as_ref()
            .and_then(|n| n.extras.as_ref())
            .map(|e| e.captions.iter().map(|t| if t.auto { format!("{} (auto)", t.name) } else { t.name.clone() }).collect())
            .unwrap_or_default();
        if !tracks.is_empty() {
            let current = app.now.as_ref().and_then(|n| n.caption);
            let mut choice = None;
            egui::ComboBox::from_id_salt("captions")
                .selected_text(if current.is_some() { "CC ●" } else { "CC" })
                .width(60.0)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(current.is_none(), "Off").clicked() {
                        choice = Some(None);
                    }
                    for (i, t) in tracks.iter().enumerate() {
                        if ui.selectable_label(current == Some(i), t).clicked() {
                            choice = Some(Some(i));
                        }
                    }
                })
                .response
                .on_hover_text("Captions (C)");
            if let Some(c) = choice {
                app.choose_caption(c, true);
            }
        }
        // Dubbed sound tracks.
        let audio: Vec<String> = app
            .now
            .as_ref()
            .and_then(|n| n.info.as_ref())
            .map(|i| i.audio_tracks.iter().map(|t| t.name.clone()).collect())
            .unwrap_or_default();
        if audio.len() > 1 {
            let current = app.now.as_ref().and_then(|n| n.audio_track);
            let mut choice = None;
            egui::ComboBox::from_id_salt("audio-track")
                .selected_text("🔊 Track")
                .width(90.0)
                .show_ui(ui, |ui| {
                    for (i, t) in audio.iter().enumerate() {
                        let on = current == Some(i) || (current.is_none() && t.to_lowercase().contains("original"));
                        if ui.selectable_label(on, t).clicked() {
                            choice = Some(i);
                        }
                    }
                })
                .response
                .on_hover_text("Audio track");
            if let Some(i) = choice {
                if let Some(n) = &mut app.now {
                    n.audio_track = Some(i);
                }
                app.reapply_audio();
            }
        }
        // More: loop, stable volume, ambient, sleep timer.
        ui.menu_button("⚙", |ui| {
            let mut lp = app.store.saved.loop_video;
            if ui.checkbox(&mut lp, "Loop").changed() {
                app.store.saved.loop_video = lp;
                app.store.touch();
                app.player.set_loop(lp);
            }
            let mut sv = app.store.saved.stable_volume;
            if ui.checkbox(&mut sv, "Stable volume").on_hover_text("Evens out loud and quiet parts").changed() {
                app.store.saved.stable_volume = sv;
                app.store.touch();
                app.reapply_audio();
            }
            let mut amb = !app.store.saved.ambient_off;
            if ui.checkbox(&mut amb, "Ambient mode").changed() {
                app.store.saved.ambient_off = !amb;
                app.store.touch();
            }
            ui.separator();
            ui.label(RichText::new("Sleep timer").small().color(TEXT_DIM));
            let left = match &app.sleep {
                Some(Sleep::At(t)) => Some(format!("{} min left", (t.saturating_duration_since(std::time::Instant::now()).as_secs() + 59) / 60)),
                Some(Sleep::EndOfVideo) => Some("At the end of this video".into()),
                None => None,
            };
            if let Some(l) = left {
                ui.label(RichText::new(l).small());
            }
            ui.horizontal_wrapped(|ui| {
                for m in [10u64, 15, 30, 45, 60, 90] {
                    if ui.small_button(format!("{m} min")).clicked() {
                        app.sleep = Some(Sleep::At(std::time::Instant::now() + std::time::Duration::from_secs(m * 60)));
                        app.toast(format!("Pausing in {m} minutes"));
                    }
                }
                if ui.small_button("End of video").clicked() {
                    app.sleep = Some(Sleep::EndOfVideo);
                    app.toast("Stopping at the end of this video");
                }
                if app.sleep.is_some() && ui.small_button("Off").clicked() {
                    app.sleep = None;
                    app.toast("Sleep timer off");
                }
            });
        })
        .response
        .on_hover_text("Loop, stable volume, ambient mode, sleep timer");
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

fn info_block(app: &mut App, ui: &mut Ui, st: &crate::PlayerState) {
    let Some(n) = &app.now else { return };
    let title = n
        .details
        .as_ref()
        .map(|d| d.video.title.clone())
        .or_else(|| n.info.as_ref().map(|i| i.title.clone()).filter(|t| !t.is_empty()))
        .unwrap_or_else(|| n.card.title.clone());
    let channel = n.details.as_ref().and_then(|d| d.video.channel_name.clone()).or_else(|| n.card.channel_name.clone());
    let channel_id = n.details.as_ref().and_then(|d| d.video.channel_id.clone()).or_else(|| n.card.channel_id.clone());
    let avatar = n.details.as_ref().and_then(|d| d.video.channel_avatar.clone()).or_else(|| n.card.channel_avatar.clone());
    let subscribers = n.details.as_ref().and_then(|d| d.subscribers.clone());
    let mut meta = Vec::new();
    if let Some(d) = &n.details {
        if let Some(v) = d.video.views {
            meta.push(format!("{} views", fmt_count(v)));
        }
        if let Some(p) = &d.video.published {
            meta.push(p.clone());
        }
    }
    let likes = n.details.as_ref().and_then(|d| d.likes);
    let description = n.details.as_ref().map(|d| d.description.clone());
    let video_id = n.card.id.clone();
    let rating = n.rating.clone();
    let music = n.music;
    let sub = channel_id.as_ref().and_then(|c| app.subs_state.get(c)).cloned();

    ui.add_space(10.0);
    ui.label(RichText::new(title).font(heading_font(19.0)));
    ui.add_space(6.0);
    let mut go_channel = false;
    let mut sub_click = None;
    let mut bell = None;
    let mut act = None;
    ui.horizontal_wrapped(|ui| {
        let (r, resp) = ui.allocate_exact_size(vec2(38.0, 38.0), Sense::click());
        widgets::thumb(app, ui, avatar.as_deref(), r, 255);
        go_channel |= resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked();
        ui.vertical(|ui| {
            if let Some(c) = &channel {
                if ui.link(RichText::new(c).color(TEXT).font(FontId::new(14.5, semibold()))).clicked() {
                    go_channel = true;
                }
            }
            if let Some(s) = &subscribers {
                ui.label(RichText::new(s).small().color(TEXT_DIM));
            }
        });
        ui.add_space(8.0);
        if let Some(c) = &channel_id {
            let on = sub.as_ref().and_then(|v| v.get("subscribed")).and_then(Value::as_bool).unwrap_or(false);
            let resp = if on { widgets::ghost(ui, "Subscribed") } else { widgets::primary(ui, "Subscribe") };
            if resp.clicked() {
                sub_click = Some((c.clone(), !on));
            }
            if on {
                bell_menu(ui, c, sub.as_ref(), &mut bell);
            }
        }
        ui.add_space(12.0);
        // Like / dislike.
        let liked = rating.as_deref() == Some("like");
        let disliked = rating.as_deref() == Some("dislike");
        let like_text = match likes {
            Some(l) => format!("👍 {}", fmt_count(l as u64)),
            None => "👍 Like".into(),
        };
        if ui.add(egui::Button::new(RichText::new(like_text).color(if liked { ACCENT } else { TEXT })).selected(liked)).clicked() {
            act = Some("like");
        }
        if ui.add(egui::Button::new(RichText::new("👎").color(if disliked { ACCENT } else { TEXT })).selected(disliked)).on_hover_text("Dislike").clicked() {
            act = Some("dislike");
        }
        ui.menu_button("💾 Save", |ui| {
            if ui.button("Watch later").clicked() {
                act = Some("wl");
                ui.close();
            }
            if ui.button("Save to playlist…").clicked() {
                act = Some("save");
                ui.close();
            }
        });
        ui.menu_button("↗ Share", |ui| {
            if ui.button("Copy link").clicked() {
                ui.ctx().copy_text(if music { format!("https://music.youtube.com/watch?v={video_id}") } else { format!("https://youtu.be/{video_id}") });
                act = Some("copied");
                ui.close();
            }
            let at = st.position_s as u32;
            if at > 0 && ui.button(format!("Copy link at {}", fmt_time(st.position_s))).clicked() {
                ui.ctx().copy_text(format!("https://youtu.be/{video_id}?t={at}"));
                act = Some("copied");
                ui.close();
            }
            if ui.button("Open on YouTube ↗").clicked() {
                act = Some("open");
                ui.close();
            }
        });
        if ui.button("📺 Play on TV").clicked() {
            act = Some("tv");
        }
    });
    if go_channel {
        if let Some(id) = channel_id.clone() {
            if music {
                let name = channel.clone().unwrap_or_default();
                app.open_music_page(id, None, name);
            } else {
                app.open_channel(id);
            }
        }
    }
    if let Some((c, on)) = sub_click {
        app.subscribe(c, on);
    }
    if let Some((params, label)) = bell {
        if let Some(c) = channel_id.clone() {
            app.set_bell(c, params, label);
        }
    }
    match act {
        Some("like") => app.rate("like"),
        Some("dislike") => app.rate("dislike"),
        Some("wl") => app.watch_later(video_id.clone()),
        Some("save") => app.open_save(video_id.clone()),
        Some("copied") => app.toast("Link copied"),
        Some("open") => app.platform.open_url(&format!("https://www.youtube.com/watch?v={video_id}")),
        Some("tv") => app.open_tv(video_id.clone(), st.position_s as u32),
        _ => {}
    }
    if !meta.is_empty() || description.as_ref().is_some_and(|d| !d.is_empty()) {
        ui.add_space(8.0);
        egui::Frame::new().fill(SURFACE).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if !meta.is_empty() {
                ui.label(RichText::new(meta.join("  ·  ")).strong().small());
            }
            if let Some(d) = description.filter(|d| !d.is_empty()) {
                let id = ui.make_persistent_id(("desc-open", &video_id));
                let open = ui.data(|m| m.get_temp::<bool>(id)).unwrap_or(false);
                let shown: String = if open { d.clone() } else { d.lines().take(3).collect::<Vec<_>>().join("\n") };
                ui.label(RichText::new(shown).small());
                if d.lines().count() > 3 && ui.link(if open { "Show less" } else { "…more" }).clicked() {
                    ui.data_mut(|m| m.insert_temp(id, !open));
                }
            }
        });
    }
}

/// The bell: all notifications, personalized, or none.
fn bell_menu(ui: &mut Ui, _channel: &str, sub: Option<&Value>, out: &mut Option<(String, String)>) {
    let choices: Vec<(String, String)> = sub
        .and_then(|v| v.get("bell"))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|b| Some((b.get("label")?.as_str()?.to_string(), b.get("params")?.as_str()?.to_string())))
        .collect();
    if choices.is_empty() {
        return;
    }
    let now = sub.and_then(|v| v.get("bell_now")).and_then(Value::as_str).unwrap_or("").to_string();
    ui.menu_button("🔔", |ui| {
        for (label, params) in &choices {
            if ui.selectable_label(now.eq_ignore_ascii_case(label), label).clicked() {
                *out = Some((params.clone(), label.clone()));
                ui.close();
            }
        }
    })
    .response
    .on_hover_text("Notifications from this channel");
}

fn tabs(app: &mut App, ui: &mut Ui, st: &crate::PlayerState) {
    let Some(n) = &app.now else { return };
    let music = n.music;
    let has_chapters = n.details.as_ref().is_some_and(|d| !d.chapters.is_empty());
    let has_chat = n.chat.next.is_some() || !n.chat.messages.is_empty();
    let mut list: Vec<(WatchTab, &str)> = vec![(WatchTab::UpNext, "Up next")];
    if music {
        list.push((WatchTab::Lyrics, "Lyrics"));
        list.push((WatchTab::Related, "Related"));
    } else {
        list.push((WatchTab::Comments, "Comments"));
        if has_chapters {
            list.push((WatchTab::Chapters, "Chapters"));
        }
        if has_chat {
            list.push((WatchTab::Chat, "Live chat"));
        }
    }
    if !list.iter().any(|(t, _)| *t == app.watch_tab) {
        app.watch_tab = WatchTab::UpNext;
    }
    let current = list.iter().position(|(t, _)| *t == app.watch_tab).unwrap_or(0);
    let labels: Vec<&str> = list.iter().map(|(_, l)| *l).collect();
    if let Some(i) = widgets::pills(ui, &labels, current) {
        app.watch_tab = list[i].0;
    }
    ui.add_space(6.0);
    match app.watch_tab {
        WatchTab::UpNext => up_next(app, ui),
        WatchTab::Comments => {
            app.load_comments();
            comments(app, ui);
        }
        WatchTab::Chapters => chapters(app, ui, st),
        WatchTab::Chat => chat(app, ui),
        WatchTab::Lyrics => {
            app.load_lyrics();
            match app.now.as_ref().and_then(|n| n.lyrics.clone()) {
                Some(Ok(t)) if t.is_empty() => {
                    ui.label(RichText::new("Loading lyrics…").color(TEXT_DIM));
                }
                Some(Ok(t)) => {
                    ui.label(RichText::new(t).size(16.0).line_height(Some(26.0)));
                }
                Some(Err(e)) => {
                    ui.label(RichText::new(e).color(TEXT_DIM));
                }
                None => {
                    ui.label(RichText::new("Loading lyrics…").color(TEXT_DIM));
                }
            }
        }
        WatchTab::Related => crate::music::related(app, ui),
    }
}

fn up_next(app: &mut App, ui: &mut Ui) {
    let music = app.now.as_ref().is_some_and(|n| n.music);
    let mut play = None;
    let mut remove = None;
    let mut act = None;
    if !app.queue.is_empty() {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Queue · {}", app.queue.len())).font(heading_font(14.0)));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("Clear").clicked() {
                    remove = Some(usize::MAX);
                }
            });
        });
        let queue = app.queue.clone();
        for (i, q) in queue.iter().enumerate().take(60) {
            let resp = widgets::item_row(app, ui, &Item::Video(q.card.clone()), false);
            let r = resp.rect;
            let x = Rect::from_center_size(egui::pos2(r.right() - 22.0, r.top() + 20.0), vec2(22.0, 22.0));
            let xr = ui.interact(x, resp.id.with("rm"), Sense::click());
            ui.painter().text(x.center(), egui::Align2::CENTER_CENTER, "✖", FontId::proportional(13.0), if xr.hovered() { TEXT } else { TEXT_DIM });
            if xr.clicked() {
                remove = Some(i);
            } else if resp.clicked() {
                play = Some(i);
            }
        }
        if !music {
            ui.add_space(8.0);
            ui.label(RichText::new("Up next").font(heading_font(14.0)));
        }
    }
    if music {
        if app.queue.is_empty() {
            let busy = app.music.radio_busy || app.music.radio.is_some();
            ui.label(RichText::new(if busy { "Loading the radio…" } else { "Nothing queued." }).color(TEXT_DIM));
        }
    } else {
        let Some(n) = &app.now else { return };
        let related = n.related.items.clone();
        let loading = n.related.loading.is_some();
        let error = n.related.error.clone();
        let has_more = n.related.next.is_some();
        if loading && related.is_empty() {
            for _ in 0..5 {
                widgets::skeleton_row(ui);
            }
        }
        if let (Some(e), true) = (error, related.is_empty()) {
            ui.label(RichText::new(e).color(TEXT_DIM).small());
            if ui.button("Retry").clicked() {
                app.retry(ListRef::Related);
            }
        }
        let mut open = None;
        for item in &related {
            let resp = widgets::item_row(app, ui, item, false);
            if resp.clicked() {
                open = Some(item.clone());
            }
            if let Item::Video(v) = item {
                if resp.hovered() {
                    app.prefetch(&v.id);
                }
                resp.context_menu(|ui| video_menu(ui, v, false, &mut act));
            }
        }
        if has_more {
            let r = ui.allocate_response(vec2(1.0, 1.0), Sense::hover());
            if ui.is_rect_visible(r.rect) {
                app.load_more(ListRef::Related);
            }
            widgets::skeleton_row(ui);
        }
        if let Some(i) = open {
            app.open_item(&i);
            return;
        }
    }
    if let Some(i) = remove {
        if i == usize::MAX {
            app.queue.clear();
            app.music.radio = None;
        } else if i < app.queue.len() {
            app.queue.remove(i);
        }
    }
    if let Some(i) = play {
        if i < app.queue.len() {
            let q = app.queue.remove(i);
            app.play_card(q.card, q.music);
        }
    }
    if let Some(a) = act {
        app.run_item_act(a);
    }
}

fn comments(app: &mut App, ui: &mut Ui) {
    let Some(n) = &app.now else { return };
    let c = &n.comments;
    if c.req.is_some() {
        for _ in 0..4 {
            widgets::skeleton_row(ui);
        }
        return;
    }
    if let Some(e) = c.error.clone().filter(|_| c.items.is_empty()) {
        ui.label(RichText::new(format!("Comments could not be loaded: {e}")).color(TEXT_DIM));
        return;
    }
    if c.loaded && c.items.is_empty() {
        ui.label(RichText::new("No comments.").color(TEXT_DIM));
        return;
    }
    let items = c.items.clone();
    let has_more = c.next.is_some() || c.more.is_some();
    let mut channel = None;
    for cm in &items {
        ui.horizontal_top(|ui| {
            let (r, resp) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
            widgets::thumb(app, ui, cm.avatar.as_deref(), r, 255);
            if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                channel = cm.author_id.clone();
            }
            ui.vertical(|ui| {
                ui.horizontal_wrapped(|ui| {
                    let mut name = RichText::new(cm.author.clone().unwrap_or_default()).strong().small();
                    if cm.by_owner {
                        name = name.background_color(SURFACE_HI);
                    }
                    ui.label(name);
                    ui.label(RichText::new(&cm.published).small().color(TEXT_DIM));
                    if cm.pinned {
                        ui.label(RichText::new("📌 Pinned").small().color(TEXT_DIM));
                    }
                });
                ui.label(&cm.text);
                ui.horizontal(|ui| {
                    if let Some(l) = cm.likes {
                        ui.label(RichText::new(format!("👍 {}", fmt_count(l as u64))).small().color(TEXT_DIM));
                    }
                    if cm.hearted {
                        ui.label(RichText::new("❤").small().color(DANGER));
                    }
                    if cm.replies > 0 {
                        ui.label(RichText::new(format!("{} replies", cm.replies)).small().color(ACCENT));
                    }
                });
            });
        });
        ui.add_space(10.0);
    }
    if has_more {
        let r = ui.allocate_response(vec2(1.0, 1.0), Sense::hover());
        if ui.is_rect_visible(r.rect) {
            app.more_comments();
        }
        ui.label(RichText::new("Loading more…").small().color(TEXT_DIM));
    }
    if let Some(c) = channel {
        app.open_channel(c);
    }
}

fn chapters(app: &mut App, ui: &mut Ui, st: &crate::PlayerState) {
    let Some(d) = app.now.as_ref().and_then(|n| n.details.as_ref()) else { return };
    let list = d.chapters.clone();
    let thumb = app.now.as_ref().and_then(|n| n.extras.as_ref()?.storyboard.as_ref().map(|_| ()));
    let _ = thumb;
    let mut seek = None;
    for (i, c) in list.iter().enumerate() {
        let end = list.get(i + 1).map(|n| n.start as f64).unwrap_or(f64::MAX);
        let here = st.position_s >= c.start as f64 && st.position_s < end;
        let text = RichText::new(format!("{}   {}", fmt_time(c.start as f64), c.title)).color(if here { ACCENT } else { TEXT });
        if ui.add(egui::Button::new(text).frame(here).fill(SURFACE_HI)).clicked() {
            seek = Some(c.start as f64);
        }
    }
    if let Some(s) = seek {
        app.player.seek(s);
        app.player.set_paused(false);
    }
}

fn chat(app: &mut App, ui: &mut Ui) {
    let Some(n) = &app.now else { return };
    let c = &n.chat;
    if let Some(e) = &c.ended {
        ui.label(RichText::new(format!("Chat: {e}")).color(TEXT_DIM));
    }
    if c.messages.is_empty() && c.ended.is_none() {
        ui.label(RichText::new("Connecting to the chat…").color(TEXT_DIM));
    }
    egui::Frame::new().fill(SURFACE).corner_radius(CornerRadius::same(10)).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("live-chat")
            .max_height(420.0)
            .stick_to_bottom(true)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for m in &c.messages {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new(&m.author).small().color(TEXT_DIM).strong());
                        if let Some(a) = &m.amount {
                            ui.label(RichText::new(a).small().color(ON_ACCENT).background_color(ACCENT));
                        }
                        ui.label(RichText::new(&m.text).small());
                    });
                }
            });
    });
}

// ------------------------------------------------------------------ dialogs

pub(crate) fn dialogs(app: &mut App, ctx: &egui::Context) {
    let Some(d) = &mut app.dialog else { return };
    let mut open = true;
    let mut close = false;
    enum Do {
        Toggle(String, String, bool, String),
        Create(String, &'static str, String),
        Tv(String, String, String, u32),
    }
    let mut todo = None;
    match d {
        Dialog::SaveTo { video, lists, error, new_name, privacy } => {
            egui::Window::new("Save to…").open(&mut open).collapsible(false).resizable(false).default_width(320.0).show(ctx, |ui| {
                if let Some(e) = error {
                    ui.label(RichText::new(e.as_str()).color(DANGER));
                }
                match lists {
                    None if error.is_none() => {
                        ui.label(RichText::new("Loading your playlists…").color(TEXT_DIM));
                    }
                    None => {}
                    Some(ls) => {
                        egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                            for p in ls.iter_mut() {
                                let mut on = p.contains;
                                if ui.checkbox(&mut on, &p.title).changed() {
                                    p.contains = on;
                                    todo = Some(Do::Toggle(p.id.clone(), video.clone(), on, p.title.clone()));
                                }
                            }
                        });
                    }
                }
                ui.separator();
                ui.label(RichText::new("New playlist").small().color(TEXT_DIM));
                ui.text_edit_singleline(new_name);
                let options = ["Private", "Unlisted", "Public"];
                ui.horizontal(|ui| {
                    for (i, o) in options.iter().enumerate() {
                        ui.radio_value(privacy, i, *o);
                    }
                });
                if ui.add_enabled(!new_name.trim().is_empty(), egui::Button::new("Create")).clicked() {
                    let p = ["PRIVATE", "UNLISTED", "PUBLIC"][*privacy];
                    todo = Some(Do::Create(new_name.trim().to_string(), p, video.clone()));
                    close = true;
                }
            });
        }
        Dialog::Tv { video, at, devices, error } => {
            egui::Window::new("Play on TV").open(&mut open).collapsible(false).resizable(false).default_width(320.0).show(ctx, |ui| {
                if let Some(e) = error {
                    ui.label(RichText::new(e.as_str()).color(DANGER));
                }
                match devices {
                    None => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label("Looking for TVs on this network…");
                        });
                    }
                    Some(ds) if ds.is_empty() => {
                        ui.label(RichText::new(
                            "No TV answered. The TV must be on the same network with its YouTube app installed; TVs that only support Google Cast do not show here.",
                        )
                        .color(TEXT_DIM));
                    }
                    Some(ds) => {
                        for (name, url) in ds.iter() {
                            if ui.add(egui::Button::new(format!("📺  {name}")).min_size(vec2(ui.available_width(), 30.0))).clicked() {
                                todo = Some(Do::Tv(name.clone(), url.clone(), video.clone(), *at));
                            }
                        }
                    }
                }
            });
        }
    }
    if !open || close {
        app.dialog = None;
    }
    match todo {
        Some(Do::Toggle(list, video, add, title)) => app.edit_playlist(list, video, add, title),
        Some(Do::Create(title, privacy, video)) => app.create_playlist(title, privacy, video),
        Some(Do::Tv(name, url, video, at)) => app.play_on_tv(name, url, video, at),
        None => {}
    }
    let _ = Stroke::NONE;
}
