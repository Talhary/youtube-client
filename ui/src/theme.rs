//! Visual theme: near-black surfaces with a bright green accent, in the style of neon.tech,
//! set in Inter.

use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
use egui::{Color32, CornerRadius, FontData, FontFamily, FontId, Stroke, TextStyle, Visuals};

pub const ACCENT: Color32 = Color32::from_rgb(0x00, 0xe5, 0x99);
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(0x34, 0xf0, 0xb1);
/// Text on accent-filled buttons.
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x06, 0x1a, 0x12);
pub const BG: Color32 = Color32::from_rgb(0x0c, 0x0d, 0x0d);
pub const SURFACE: Color32 = Color32::from_rgb(0x13, 0x14, 0x15);
pub const SURFACE_HI: Color32 = Color32::from_rgb(0x1b, 0x1c, 0x1e);
pub const BORDER: Color32 = Color32::from_rgb(0x26, 0x28, 0x2b);
pub const SKELETON: Color32 = Color32::from_rgb(0x1e, 0x20, 0x22);
pub const TEXT: Color32 = Color32::from_rgb(0xed, 0xee, 0xef);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8f, 0x94, 0x9b);
pub const DANGER: Color32 = Color32::from_rgb(0xff, 0x63, 0x69);
pub const SUCCESS: Color32 = ACCENT;

/// Inter SemiBold, for headings and emphasis.
pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

pub fn heading_font(size: f32) -> FontId {
    FontId::new(size, semibold())
}

fn fonts(ctx: &egui::Context) {
    ctx.add_font(FontInsert::new(
        "inter",
        FontData::from_static(include_bytes!("../../assets/fonts/Inter-Regular.ttf")),
        vec![InsertFontFamily {
            family: FontFamily::Proportional,
            priority: FontPriority::Highest,
        }],
    ));
    ctx.add_font(FontInsert::new(
        "inter-semibold",
        FontData::from_static(include_bytes!("../../assets/fonts/Inter-SemiBold.ttf")),
        vec![InsertFontFamily {
            family: semibold(),
            priority: FontPriority::Highest,
        }],
    ));
    // Fall back to the regular stack (incl. system fonts) for glyphs Inter lacks.
    ctx.add_font(FontInsert::new(
        "inter-fallback",
        FontData::from_static(include_bytes!("../../assets/fonts/Inter-Regular.ttf")),
        vec![InsertFontFamily {
            family: semibold(),
            priority: FontPriority::Lowest,
        }],
    ));
}

pub fn apply(ctx: &egui::Context, phone: bool) {
    fonts(ctx);
    let mut v = Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.extreme_bg_color = Color32::from_rgb(0x10, 0x11, 0x12);
    v.faint_bg_color = SURFACE;
    v.code_bg_color = SURFACE_HI;
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT.linear_multiply(0.30);
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    v.override_text_color = Some(TEXT);
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(10);
    v.window_shadow = egui::Shadow {
        offset: [0, 12],
        blur: 32,
        spread: 0,
        color: Color32::from_black_alpha(160),
    };
    v.popup_shadow = egui::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(140),
    };
    v.slider_trailing_fill = true;

    let r = CornerRadius::same(8);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = SURFACE;
    w.noninteractive.weak_bg_fill = SURFACE;
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
    w.inactive.bg_fill = SURFACE_HI;
    w.inactive.weak_bg_fill = SURFACE_HI;
    w.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    w.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    w.hovered.bg_fill = Color32::from_rgb(0x23, 0x25, 0x27);
    w.hovered.weak_bg_fill = Color32::from_rgb(0x23, 0x25, 0x27);
    w.hovered.bg_stroke = Stroke::new(1.0, ACCENT.linear_multiply(0.55));
    w.hovered.fg_stroke = Stroke::new(1.0, TEXT);
    w.active.bg_fill = ACCENT.linear_multiply(0.25);
    w.active.weak_bg_fill = ACCENT.linear_multiply(0.25);
    w.active.bg_stroke = Stroke::new(1.0, ACCENT);
    w.active.fg_stroke = Stroke::new(1.5, TEXT);
    w.open.weak_bg_fill = SURFACE_HI;
    w.open.bg_stroke = Stroke::new(1.0, BORDER);
    for wv in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        wv.corner_radius = r;
    }
    ctx.set_visuals(v);

    ctx.all_styles_mut(|s| {
        let k = if phone { 1.12 } else { 1.0 };
        s.text_styles = [
            (TextStyle::Heading, FontId::new(22.0 * k, semibold())),
            (TextStyle::Body, FontId::new(14.0 * k, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(13.5 * k, FontFamily::Proportional)),
            (TextStyle::Small, FontId::new(12.0 * k, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.5 * k, FontFamily::Monospace)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(8.0, 7.0);
        s.spacing.button_padding = egui::vec2(12.0, 6.0);
        s.spacing.interact_size.y = if phone { 40.0 } else { 28.0 };
        s.spacing.slider_rail_height = 4.0;
        s.spacing.menu_margin = egui::Margin::same(8);
        s.interaction.selectable_labels = false;
    });
}
