//! Motion helpers (PRD 3.4). Transitions use egui's time-based animations, which repaint
//! only while a value is moving; there is never an unconditional `request_repaint()`.
//! Reduced motion sets every duration to zero.

use std::sync::atomic::{AtomicBool, Ordering};

static REDUCED: AtomicBool = AtomicBool::new(false);

pub fn set_reduced(reduced: bool) {
    REDUCED.store(reduced, Ordering::Relaxed);
}

pub fn reduced() -> bool {
    REDUCED.load(Ordering::Relaxed)
}

/// Navigation and dialogs.
pub const NAV: f32 = 0.22;
/// List expansion (descriptions, menus).
pub const EXPAND: f32 = 0.20;
/// Hover feedback.
pub const HOVER: f32 = 0.15;

fn dur(secs: f32) -> f32 {
    if reduced() { 0.0 } else { secs }
}

/// 0 → 1 as `on` becomes true, eased.
pub fn toggle(ctx: &egui::Context, id: egui::Id, on: bool, secs: f32) -> f32 {
    let t = ctx.animate_bool_with_time(id, on, dur(secs));
    ease_out(t)
}

/// Animate toward `target`.
pub fn value(ctx: &egui::Context, id: egui::Id, target: f32, secs: f32) -> f32 {
    ctx.animate_value_with_time(id, target, dur(secs))
}

pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t).powi(3)
}

/// Read the OS-level reduced-motion preference.
#[cfg(windows)]
pub fn os_reduced_motion() -> bool {
    // SPI_GETCLIENTAREAANIMATION = 0x1042; FALSE means animations are off.
    #[link(name = "user32")]
    unsafe extern "system" {
        fn SystemParametersInfoW(action: u32, param: u32, pv: *mut core::ffi::c_void, win_ini: u32) -> i32;
    }
    let mut enabled: i32 = 1;
    let ok = unsafe { SystemParametersInfoW(0x1042, 0, (&mut enabled as *mut i32).cast(), 0) };
    ok != 0 && enabled == 0
}

#[cfg(all(unix, not(target_os = "android")))]
pub fn os_reduced_motion() -> bool {
    std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "enable-animations"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "false")
        .unwrap_or(false)
}

#[cfg(target_os = "android")]
pub fn os_reduced_motion() -> bool {
    false
}
