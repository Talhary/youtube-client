#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod mpv;
mod platform;
mod signin;

use std::sync::Arc;

use eframe::egui;
use streampipe_core::config::Paths;
use streampipe_core::mux::{FfmpegMuxer, find_ffmpeg};
use streampipe_core::streams::HwCaps;
use streampipe_core::{Core, CoreOptions};

struct DesktopApp {
    ui: streampipe_ui::App,
    caps_file: std::path::PathBuf,
    // Dropped last: owns the core's runtime.
    _core: Core,
}

impl eframe::App for DesktopApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui.logic(ctx);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.ui.ui(ui);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.02, 0.02, 0.02, 1.0]
    }

    fn on_exit(&mut self, gl: Option<&eframe::glow::Context>) {
        if let Ok(b) = serde_json::to_vec(&self.ui.hw_caps()) {
            let _ = streampipe_core::config::write_atomic(&self.caps_file, &b);
        }
        if let Some(gl) = gl {
            if let Some(p) = self.ui.player_mut().as_any_mut().downcast_mut::<mpv::MpvPlayer>() {
                p.destroy_gl(gl);
            }
        }
    }
}

fn load_icon() -> Option<egui::IconData> {
    let bytes = include_bytes!("../../assets/icon-256.png");
    let img = image::load_from_memory(bytes).ok()?.to_rgba8();
    Some(egui::IconData {
        width: img.width(),
        height: img.height(),
        rgba: img.into_raw(),
    })
}

fn main() -> eframe::Result {
    // Helper mode: the "Sign in with Google" window runs in its own process.
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some(signin::HELPER_ARG) {
        let dir = args.get(2).map(std::path::PathBuf::from).unwrap_or_else(|| std::env::temp_dir().join("streampipe-signin"));
        signin::run_helper(&dir);
    }
    let paths = Paths::desktop().expect("no home directory");
    if let Err(e) = paths.ensure() {
        eprintln!("could not create app folders: {e}");
    }
    let key = streampipe_core::secure::desktop_key(&paths.data);
    let ffmpeg = Arc::new(FfmpegMuxer::new(find_ffmpeg(&paths.data)));
    let caps_file = paths.config.join("hwcaps.json");
    // Start optimistic; the player downgrades a codec the first time it decodes in software.
    let caps: HwCaps = std::fs::read(&caps_file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(HwCaps {
            h264: true,
            vp9: true,
            av1: true,
        });

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("StreamPipe")
        .with_app_id("dev.streampipe.StreamPipe")
        .with_inner_size([1320.0, 820.0])
        .with_min_inner_size([760.0, 480.0]);
    if let Some(icon) = load_icon() {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::run_native(
        "StreamPipe",
        options,
        Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();
            let waker_ctx = ctx.clone();
            let mut core = Core::start(CoreOptions {
                paths: paths.clone(),
                key,
                botguard: None,
                muxer: ffmpeg.clone(),
                ffmpeg: Some(ffmpeg),
                caps,
                waker: Arc::new(move || waker_ctx.request_repaint()),
                version: env!("CARGO_PKG_VERSION").into(),
                github_updates: cfg!(feature = "github-updates"),
            })?;
            let proc_addr = cc
                .get_proc_address
                .clone()
                .ok_or("this build needs the OpenGL renderer")?;
            let player = Box::new(mpv::MpvPlayer::new(ctx, proc_addr)?);
            let mut ui = streampipe_ui::App::new(
                core.handle.clone(),
                core.take_events(),
                player,
                Box::new(platform::Desktop {
                    signin_profile: paths.cache.join("signin-webview"),
                }),
                caps,
            );
            if let Some(link) = std::env::args().nth(1).filter(|a| !a.starts_with("--")) {
                ui.open_link(&link);
            }
            Ok(Box::new(DesktopApp {
                ui,
                caps_file,
                _core: core,
            }))
        }),
    )
}
