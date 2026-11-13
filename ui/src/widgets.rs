use egui::{Align2, Color32, CornerRadius, FontId, Rect, Response, RichText, Sense, Stroke, StrokeKind, Ui, vec2};
use streampipe_core::model::{Item, fmt_count, fmt_duration};

use crate::app::{App, Thumb};
use crate::motion;
use crate::theme::*;

pub const ROW_H: f32 = 98.0;
pub const THUMB: egui::Vec2 = vec2(152.0, 86.0);
/// Minimum card width in grid view.
pub const CARD_MIN_W: f32 = 270.0;
pub const CARD_GAP: f32 = 18.0;
/// Text block under a card's thumbnail.
pub const CARD_TEXT_H: f32 = 76.0;

pub fn fmt_time(s: f64) -> String {
    if !s.is_finite() || s < 0.0 {
        return "0:00".into();
    }
    fmt_duration(s as u32)
}

pub fn fmt_bytes(n: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{n} B") } else { format!("{v:.1} {}", U[i]) }
}

/// Draw a thumbnail (or its placeholder) into `rect`.
pub fn thumb(app: &mut App, ui: &Ui, url: Option<&str>, rect: Rect, radius: u8) {
    let painter = ui.painter();
    let r = CornerRadius::same(radius);
    let Some(url) = url else {
        painter.rect_filled(rect, r, SKELETON);
        return;
    };
    // Only fetch thumbnails that are actually on screen.
    if ui.is_rect_visible(rect) {
        app.want_thumb(url);
    }
    match app.thumbs.get(url) {
        Some(Thumb::Ready(tex)) => {
            let t = motion::toggle(ui.ctx(), egui::Id::unique(("thumb-in", url)), true, motion::HOVER);
            painter.rect_filled(rect, r, SKELETON);
            egui::Image::new((tex.id(), rect.size()))
                .corner_radius(r)
                .tint(Color32::WHITE.linear_multiply(t.max(0.01)))
                .paint_at(ui, rect);
        }
        _ => {
            painter.rect_filled(rect, r, SKELETON);
        }
    }
}

fn badge(ui: &Ui, rect: Rect, text: &str, fill: Color32, fg: Color32) {
    let font = FontId::new(11.0, semibold());
    let galley = ui.painter().layout_no_wrap(text.to_string(), font, fg);
    let pad = vec2(6.0, 2.5);
    let size = galley.size() + pad * 2.0;
    let r = Rect::from_min_size(rect.right_bottom() - size - vec2(6.0, 6.0), size);
    ui.painter().rect_filled(r, CornerRadius::same(5), fill);
    ui.painter().galley(r.min + pad, galley, fg);
}

struct ItemText {
    title: String,
    line2: String,
    line3: String,
    thumb: Option<String>,
    accessible: String,
    round: bool,
}

fn item_text(item: &Item) -> ItemText {
    match item {
        Item::Video(v) => {
            let mut meta = Vec::new();
            if let Some(n) = v.views {
                meta.push(format!("{} views", fmt_count(n)));
            }
            if let Some(p) = &v.published {
                meta.push(p.clone());
            }
            ItemText {
                title: v.title.clone(),
                line2: v.channel_name.clone().unwrap_or_default(),
                line3: meta.join("  ·  "),
                thumb: v.thumbnail.clone(),
                accessible: format!("Video: {}", v.title),
                round: false,
            }
        }
        Item::Channel(c) => ItemText {
            title: c.name.clone(),
            line2: "Channel".into(),
            line3: c
                .subscribers
                .map(|n| format!("{} subscribers", fmt_count(n)))
                .unwrap_or_default(),
            thumb: c.avatar.clone(),
            accessible: format!("Channel: {}", c.name),
            round: true,
        },
        Item::Playlist(p) => ItemText {
            title: p.name.clone(),
            line2: p.channel_name.clone().unwrap_or_default(),
            line3: p
                .video_count
                .map(|n| format!("Playlist  ·  {n} videos"))
                .unwrap_or_else(|| "Playlist".into()),
            thumb: p.thumbnail.clone(),
            accessible: format!("Playlist: {}", p.name),
            round: false,
        },
        Item::Post(p) => ItemText {
            title: p.text.clone(),
            line2: p.author.clone().unwrap_or_default(),
            line3: p.published.clone().unwrap_or_default(),
            thumb: p.images.first().cloned().or_else(|| p.author_avatar.clone()),
            accessible: format!("Post: {}", p.text),
            round: false,
        },
    }
}

/// The red "watched" bar along the bottom of a thumbnail.
fn watched_bar(app: &App, ui: &Ui, item: &Item, trect: Rect) {
    let Item::Video(v) = item else { return };
    let frac = app.store.watched(&v.id).or(v.watched.map(|p| p as f32 / 100.0));
    let Some(f) = frac.filter(|f| *f > 0.02) else { return };
    let bar = Rect::from_min_max(egui::pos2(trect.left(), trect.bottom() - 4.0), trect.right_bottom());
    ui.painter().rect_filled(bar, 0.0, Color32::from_white_alpha(70));
    let mut done = bar;
    done.set_width(bar.width() * f.min(1.0));
    ui.painter().rect_filled(done, 0.0, DANGER);
}

fn item_badge(ui: &Ui, item: &Item, trect: Rect) {
    match item {
        Item::Video(v) if v.is_live => badge(ui, trect, "LIVE", DANGER, Color32::WHITE),
        Item::Video(v) => {
            if let Some(d) = v.duration_s {
                badge(ui, trect, &fmt_duration(d), Color32::from_black_alpha(210), Color32::WHITE);
            }
        }
        Item::Playlist(_) => badge(ui, trect, "PLAYLIST", Color32::from_black_alpha(210), ACCENT),
        _ => {}
    }
}

fn hover_frame(ui: &Ui, resp: &Response, rect: Rect, selected: bool) {
    let hover = motion::toggle(ui.ctx(), resp.id.with("hover"), resp.hovered() || selected, motion::HOVER);
    if hover > 0.0 {
        ui.painter().rect_filled(rect, CornerRadius::same(12), SURFACE_HI.linear_multiply(hover));
    }
    if selected {
        ui.painter().rect_stroke(rect, CornerRadius::same(12), Stroke::new(1.0, ACCENT), StrokeKind::Inside);
    }
}

fn text_block(ui: &Ui, rect: Rect, t: &ItemText, title_size: f32) {
    let painter = ui.painter_at(rect);
    let w = rect.width().max(20.0);
    let mut job = egui::text::LayoutJob::simple(t.title.clone(), FontId::new(title_size, semibold()), TEXT, w);
    job.wrap = egui::text::TextWrapping {
        max_width: w,
        max_rows: 2,
        break_anywhere: false,
        overflow_character: Some('…'),
    };
    let title_g = painter.layout_job(job);
    let mut y = rect.top();
    painter.galley(egui::pos2(rect.left(), y), title_g.clone(), TEXT);
    y += title_g.size().y + 4.0;
    for line in [&t.line2, &t.line3] {
        if line.is_empty() {
            continue;
        }
        let g = painter.layout_no_wrap(line.clone(), FontId::proportional(12.5), TEXT_DIM);
        painter
            .with_clip_rect(Rect::from_min_size(egui::pos2(rect.left(), y), vec2(w, 17.0)))
            .galley(egui::pos2(rect.left(), y), g, TEXT_DIM);
        y += 17.5;
    }
}

/// One list row. Returns the row's response (click to open).
pub fn item_row(app: &mut App, ui: &mut Ui, item: &Item, selected: bool) -> Response {
    let width = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let t = item_text(item);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &t.accessible));
    hover_frame(ui, &resp, rect.shrink(1.0), selected);
    let pad = (ROW_H - THUMB.y) / 2.0;
    let mut trect = Rect::from_min_size(rect.min + vec2(pad, pad), THUMB);
    let text_x = trect.right() + 12.0;
    if t.round {
        trect = Rect::from_center_size(trect.center(), vec2(THUMB.y, THUMB.y));
    }
    thumb(app, ui, t.thumb.as_deref(), trect, if t.round { 255 } else { 8 });
    item_badge(ui, item, trect);
    watched_bar(app, ui, item, trect);
    let text = Rect::from_min_max(egui::pos2(text_x, trect.top()), egui::pos2(rect.right() - 10.0, rect.bottom()));
    text_block(ui, text, &t, 14.0);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Grid card: 16:9 thumbnail with text beneath.
pub fn item_card(app: &mut App, ui: &mut Ui, item: &Item, width: f32, selected: bool) -> Response {
    let thumb_h = width * 9.0 / 16.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(width, thumb_h + CARD_TEXT_H), Sense::click());
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let t = item_text(item);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &t.accessible));
    hover_frame(ui, &resp, rect.expand(5.0), selected);
    let lift = motion::toggle(ui.ctx(), resp.id.with("lift"), resp.hovered(), motion::HOVER);
    let mut trect = Rect::from_min_size(rect.min - vec2(0.0, lift * 2.0), vec2(width, thumb_h));
    if t.round {
        let d = thumb_h * 0.8;
        trect = Rect::from_center_size(trect.center(), vec2(d, d));
    }
    thumb(app, ui, t.thumb.as_deref(), trect, if t.round { 255 } else { 12 });
    item_badge(ui, item, trect);
    watched_bar(app, ui, item, trect);
    let text = Rect::from_min_max(
        egui::pos2(rect.left() + 2.0, rect.top() + thumb_h + 10.0),
        egui::pos2(rect.right() - 2.0, rect.bottom()),
    );
    text_block(ui, text, &t, 14.5);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Placeholder row with the same geometry as a real one, so nothing jumps when data lands.
pub fn skeleton_row(ui: &mut Ui) {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
    let pad = (ROW_H - THUMB.y) / 2.0;
    let t = Rect::from_min_size(rect.min + vec2(pad, pad), THUMB);
    let p = ui.painter();
    p.rect_filled(t, CornerRadius::same(8), SKELETON);
    let x = t.right() + 12.0;
    let w = (rect.right() - x - 10.0).max(20.0);
    for (dy, frac, h) in [(2.0, 0.9, 12.0), (22.0, 0.6, 12.0), (46.0, 0.4, 10.0)] {
        p.rect_filled(
            Rect::from_min_size(egui::pos2(x, t.top() + dy), vec2(w * frac, h)),
            CornerRadius::same(4),
            SKELETON,
        );
    }
}

pub fn skeleton_card(ui: &mut Ui, width: f32) {
    let thumb_h = width * 9.0 / 16.0;
    let (rect, _) = ui.allocate_exact_size(vec2(width, thumb_h + CARD_TEXT_H), Sense::hover());
    let p = ui.painter();
    p.rect_filled(Rect::from_min_size(rect.min, vec2(width, thumb_h)), CornerRadius::same(12), SKELETON);
    for (dy, frac) in [(12.0, 0.92), (32.0, 0.55), (52.0, 0.35)] {
        p.rect_filled(
            Rect::from_min_size(rect.min + vec2(2.0, thumb_h + dy), vec2(width * frac, 11.0)),
            CornerRadius::same(4),
            SKELETON,
        );
    }
}

pub fn progress(ui: &mut Ui, frac: f32, height: f32) -> Rect {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    let r = CornerRadius::same((height / 2.0) as u8);
    ui.painter().rect_filled(rect, r, SURFACE_HI);
    let mut fill = rect;
    fill.set_width(rect.width() * frac.clamp(0.0, 1.0));
    ui.painter().rect_filled(fill, r, ACCENT);
    rect
}

/// Accent-filled button for primary actions.
pub fn primary(ui: &mut Ui, text: &str) -> Response {
    let resp = ui.add(
        egui::Button::new(RichText::new(text).color(ON_ACCENT).font(FontId::new(14.0, semibold())))
            .fill(ACCENT)
            .stroke(Stroke::NONE)
            .corner_radius(CornerRadius::same(8)),
    );
    if resp.hovered() {
        ui.painter().rect_filled(resp.rect, CornerRadius::same(8), ACCENT_HOVER.linear_multiply(0.25));
    }
    resp
}

/// Quiet bordered button.
pub fn ghost(ui: &mut Ui, text: impl Into<egui::WidgetText>) -> Response {
    ui.add(egui::Button::new(text).fill(Color32::TRANSPARENT).stroke(Stroke::new(1.0, BORDER)))
}

pub fn centered_message(ui: &mut Ui, title: &str, body: &str) {
    ui.add_space(48.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(title).font(heading_font(18.0)));
        if !body.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new(body).color(TEXT_DIM));
        }
    });
}

/// A bordered card with a heading, used to group settings.
pub fn card<R>(ui: &mut Ui, title: &str, subtitle: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(14))
        .inner_margin(egui::Margin::same(18))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(title).font(heading_font(16.0)));
            if !subtitle.is_empty() {
                ui.label(RichText::new(subtitle).small().color(TEXT_DIM));
            }
            ui.add_space(10.0);
            add(ui)
        })
        .inner
}

/// A settings row: label (+ hint) on the left, control on the right.
pub fn setting_row(ui: &mut Ui, label: &str, hint: &str, control: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() * 0.45).max(180.0));
            ui.label(label);
            if !hint.is_empty() {
                ui.label(RichText::new(hint).small().color(TEXT_DIM));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
    ui.add_space(6.0);
}

/// Segmented pill control. Returns the clicked index.
pub fn pills(ui: &mut Ui, labels: &[&str], selected: usize) -> Option<usize> {
    let mut clicked = None;
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(egui::Margin::same(3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                for (i, l) in labels.iter().enumerate() {
                    let on = i == selected;
                    let text = RichText::new(*l).color(if on { ON_ACCENT } else { TEXT_DIM });
                    let b = egui::Button::new(text)
                        .fill(if on { ACCENT } else { Color32::TRANSPARENT })
                        .stroke(Stroke::NONE)
                        .corner_radius(CornerRadius::same(8));
                    if ui.add(b).clicked() {
                        clicked = Some(i);
                    }
                }
            });
        });
    clicked
}

/// Sidebar navigation entry. `compact` shows only the icon.
pub fn nav_item(ui: &mut Ui, icon: Icon, label: &str, active: bool, badge: Option<usize>, compact: bool) -> Response {
    let h = 40.0;
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::click());
    let hover = motion::toggle(ui.ctx(), resp.id.with("nav"), resp.hovered() || active, motion::HOVER);
    let p = ui.painter();
    if hover > 0.0 {
        let fill = if active { ACCENT.linear_multiply(0.12) } else { SURFACE_HI.linear_multiply(hover) };
        p.rect_filled(rect, CornerRadius::same(9), fill);
    }
    if active {
        p.rect_filled(
            Rect::from_min_size(rect.left_top() + vec2(0.0, 10.0), vec2(3.0, h - 20.0)),
            CornerRadius::same(2),
            ACCENT,
        );
    }
    let color = if active { ACCENT } else if resp.hovered() { TEXT } else { TEXT_DIM };
    let icon_x = if compact { rect.center().x } else { rect.left() + 22.0 };
    paint_icon(p, icon, egui::pos2(icon_x, rect.center().y), 18.0, color);
    if !compact {
        let font = if active { FontId::new(14.0, semibold()) } else { FontId::proportional(14.0) };
        p.text(egui::pos2(rect.left() + 44.0, rect.center().y), Align2::LEFT_CENTER, label, font, if active { TEXT } else { color });
    }
    if let Some(n) = badge.filter(|n| *n > 0) {
        let c = if compact { rect.right_top() + vec2(-14.0, 10.0) } else { egui::pos2(rect.right() - 18.0, rect.center().y) };
        p.circle_filled(c, 9.0, ACCENT);
        p.text(c, Align2::CENTER_CENTER, n.to_string(), FontId::new(10.5, semibold()), ON_ACCENT);
    }
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, active, label));
    if compact { resp.on_hover_text(label) } else { resp }
}

/// The StreamPipe mark: a green rounded square with a play triangle.
pub fn logo(ui: &mut Ui, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same((size * 0.24) as u8), ACCENT);
    let c = rect.center();
    let s = size * 0.22;
    p.add(egui::Shape::convex_polygon(
        vec![c + vec2(-s * 0.8, -s), c + vec2(-s * 0.8, s), c + vec2(s * 1.1, 0.0)],
        ON_ACCENT,
        Stroke::NONE,
    ));
    resp
}

/// Line icons drawn with the painter, so they stay monochrome and take the theme colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    Home,
    Search,
    Subscriptions,
    Library,
    Download,
    Settings,
    Account,
}

pub fn paint_icon(p: &egui::Painter, icon: Icon, c: egui::Pos2, size: f32, color: Color32) {
    let s = size / 2.0;
    let st = Stroke::new((size / 11.0).max(1.4), color);
    let pt = |x: f32, y: f32| c + vec2(x * s, y * s);
    match icon {
        Icon::Home => {
            p.line_segment([pt(-0.85, -0.05), pt(0.0, -0.8)], st);
            p.line_segment([pt(0.0, -0.8), pt(0.85, -0.05)], st);
            p.rect_stroke(
                Rect::from_min_max(pt(-0.6, -0.2), pt(0.6, 0.8)),
                CornerRadius::same(2),
                st,
                StrokeKind::Middle,
            );
        }
        Icon::Search => {
            p.circle_stroke(pt(-0.15, -0.15), s * 0.55, st);
            p.line_segment([pt(0.27, 0.27), pt(0.8, 0.8)], st);
        }
        Icon::Subscriptions => {
            p.rect_stroke(
                Rect::from_min_max(pt(-0.85, -0.6), pt(0.85, 0.6)),
                CornerRadius::same(3),
                st,
                StrokeKind::Middle,
            );
            p.add(egui::Shape::convex_polygon(
                vec![pt(-0.22, -0.3), pt(-0.22, 0.3), pt(0.32, 0.0)],
                color,
                Stroke::NONE,
            ));
        }
        Icon::Library => {
            for x in [-0.6, -0.15] {
                p.line_segment([pt(x, -0.75), pt(x, 0.75)], st);
            }
            p.line_segment([pt(0.25, -0.7), pt(0.7, 0.72)], st);
        }
        Icon::Download => {
            p.line_segment([pt(0.0, -0.8), pt(0.0, 0.35)], st);
            p.line_segment([pt(-0.42, -0.05), pt(0.0, 0.38)], st);
            p.line_segment([pt(0.42, -0.05), pt(0.0, 0.38)], st);
            p.line_segment([pt(-0.75, 0.8), pt(0.75, 0.8)], st);
        }
        Icon::Settings => {
            p.circle_stroke(c, s * 0.32, st);
            for k in 0..8 {
                let a = k as f32 * std::f32::consts::FRAC_PI_4;
                let (sn, cs) = a.sin_cos();
                p.line_segment([pt(cs * 0.55, sn * 0.55), pt(cs * 0.85, sn * 0.85)], st);
            }
        }
        Icon::Account => {
            p.circle_stroke(pt(0.0, -0.35), s * 0.36, st);
            let mut pts = Vec::new();
            for k in 0..=16 {
                let a = std::f32::consts::PI * (1.0 + k as f32 / 16.0);
                pts.push(pt(a.cos() * 0.75, 0.85 + a.sin() * 0.6));
            }
            p.add(egui::Shape::line(pts, st));
        }
    }
}

/// Icon-only button.
pub fn icon_button(ui: &mut Ui, icon: Icon, tip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::click());
    let hover = motion::toggle(ui.ctx(), resp.id.with("ib"), resp.hovered(), motion::HOVER);
    if hover > 0.0 {
        ui.painter().rect_filled(rect, CornerRadius::same(9), SURFACE_HI.linear_multiply(hover));
    }
    let color = if resp.hovered() { TEXT } else { TEXT_DIM };
    paint_icon(ui.painter(), icon, rect.center(), 18.0, color);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tip));
    resp.on_hover_text(tip)
}

/// Round accent play/pause button with drawn glyphs.
pub fn play_button(ui: &mut Ui, playing: bool, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(38.0, 38.0), if enabled { Sense::click() } else { Sense::hover() });
    let hover = motion::toggle(ui.ctx(), resp.id.with("pb"), resp.hovered(), motion::HOVER);
    let p = ui.painter();
    let fill = if enabled { ACCENT } else { BORDER };
    p.circle_filled(rect.center(), 19.0 + hover, fill);
    let c = rect.center();
    if playing {
        for dx in [-4.0, 4.0] {
            p.rect_filled(Rect::from_center_size(c + vec2(dx, 0.0), vec2(4.0, 14.0)), CornerRadius::same(1), ON_ACCENT);
        }
    } else {
        p.add(egui::Shape::convex_polygon(
            vec![c + vec2(-4.5, -7.5), c + vec2(-4.5, 7.5), c + vec2(8.0, 0.0)],
            ON_ACCENT,
            Stroke::NONE,
        ));
    }
    let label = if playing { "Pause" } else { "Play" };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, label));
    resp
}
