//! StreamPipe's egui interface, shared by the desktop app and Android's browse UI.
//!
//! Shells provide a [`Player`] (desktop: libmpv; Android: Media3 over JNI) and a
//! [`Platform`] (dialogs, URLs, OS settings). The UI talks to the core only through the
//! command/event channels in [`streampipe_core::service`].

mod app;
mod extra;
mod motion;
mod music;
mod store;
mod theme;
mod views;
mod watch;
mod widgets;

use std::path::{Path, PathBuf};

pub use app::App;
pub use motion::os_reduced_motion;
use streampipe_core::PlaybackInfo;

/// Desktop gets the keyboard-first dual pane; phones get a single pane with bottom
/// navigation and a mini-player.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormFactor {
    Desktop,
    Phone,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerState {
    pub position_s: f64,
    pub duration_s: f64,
    pub paused: bool,
    pub buffering: bool,
    /// Nothing loaded.
    pub idle: bool,
    pub volume: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlayerEvent {
    /// Playback reached the end of the file.
    Ended,
    /// Loading or playback failed. `forbidden` means the stream URL was rejected (403).
    Error { forbidden: bool, message: String },
    /// Which decoder ended up in use for `codec` ("h264", "vp9", "av1").
    Decoder { codec: String, hardware: bool },
}

/// What to play: a video+audio pair, or audio only.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadRequest {
    pub video_url: Option<String>,
    pub audio_url: Option<String>,
    /// HLS manifest for livestreams (contains both tracks).
    pub hls_url: Option<String>,
    pub video_codec: Option<String>,
    pub title: String,
    pub start_s: f64,
}

impl LoadRequest {
    pub fn from_info(info: &PlaybackInfo, quality: Option<usize>, audio_only: bool, start_s: f64) -> Self {
        let q = if audio_only {
            None
        } else {
            quality.and_then(|i| info.qualities.get(i))
        };
        let audio = info.audio_for(q).map(|a| a.url.clone());
        Self {
            hls_url: if audio_only { None } else { info.hls.clone() },
            video_url: q.map(|q| q.stream.url.clone()),
            video_codec: q.map(|q| q.stream.codec.clone()),
            audio_url: audio,
            title: info.title.clone(),
            start_s,
        }
    }
}

/// A playback backend. Calls come from the UI thread and must not block.
pub trait Player {
    fn load(&mut self, req: LoadRequest);
    fn stop(&mut self);
    fn set_paused(&mut self, paused: bool);
    fn seek(&mut self, position_s: f64);
    /// 0–130.
    fn set_volume(&mut self, volume: f32);
    /// Playback rate, 1.0 = normal.
    fn set_speed(&mut self, speed: f32);
    fn state(&self) -> PlayerState;
    fn poll_events(&mut self) -> Vec<PlayerEvent>;
    /// Draw (desktop) or position (Android) the video inside `rect`.
    fn show_video(&mut self, ui: &mut egui::Ui, rect: egui::Rect);
    /// Measured network throughput in bits/s, for auto quality.
    fn throughput_bps(&self) -> Option<u64> {
        None
    }
    /// Play the file again from the start when it ends.
    fn set_loop(&mut self, _on: bool) {}
    /// Even out loudness ("stable volume") when the video has no levelled sound track.
    fn set_leveller(&mut self, _on: bool) {}
    /// Lets a shell reach its concrete player (e.g. to free GL resources on exit).
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileDialog {
    /// Pick a Netscape cookies.txt.
    OpenCookies,
    /// Pick the downloads folder.
    PickFolder,
    /// Choose where to save a diagnostics export.
    SaveDiagnostics,
}

pub type FileCallback = Box<dyn FnOnce(Option<PathBuf>) + Send>;

/// OS services the UI needs. Dialogs must not block the UI thread: run them elsewhere and
/// call `done` when finished.
pub trait Platform {
    fn form_factor(&self) -> FormFactor;
    fn open_url(&self, url: &str);
    fn open_path(&self, path: &Path);
    fn file_dialog(&self, kind: FileDialog, done: FileCallback);
    /// The OS "reduce motion" / "disable animations" setting.
    fn os_reduced_motion(&self) -> bool;
    /// MP3 export and cookie import need desktop facilities.
    fn is_desktop(&self) -> bool {
        self.form_factor() == FormFactor::Desktop
    }
    /// Sign in with Google in a browser window owned by the app. Calls `done` with the YouTube
    /// cookies (Netscape cookies.txt text) or an error message. Must not block.
    fn browser_sign_in(&self, done: Box<dyn FnOnce(Result<String, String>) + Send>) {
        done(Err("Browser sign-in is not available on this platform.".into()));
    }
    fn supports_browser_sign_in(&self) -> bool {
        false
    }
    /// A folder where the interface may keep small files (resume positions, search history).
    fn data_dir(&self) -> Option<PathBuf> {
        None
    }
    /// Notify the shell that the app wants playback to continue in the background.
    fn background_playback(&self, _playing: bool) {}
}
