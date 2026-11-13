//! User settings and on-disk locations.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Hard ceiling for concurrent HTTP connections used by downloads (PRD 3.3).
pub const MAX_CONNECTIONS_CAP: u8 = 16;

/// Where the core keeps its files.
#[derive(Clone, Debug)]
pub struct Paths {
    pub config: PathBuf,
    pub cache: PathBuf,
    pub data: PathBuf,
    pub downloads: PathBuf,
}

impl Paths {
    /// Standard per-user locations on Windows and Linux.
    pub fn desktop() -> Option<Self> {
        let dirs = directories::ProjectDirs::from("dev", "StreamPipe", "StreamPipe")?;
        let downloads = directories::UserDirs::new()
            .and_then(|u| u.download_dir().map(|d| d.join("StreamPipe")))
            .unwrap_or_else(|| dirs.data_dir().join("downloads"));
        Some(Self {
            config: dirs.config_dir().into(),
            cache: dirs.cache_dir().into(),
            data: dirs.data_dir().into(),
            downloads,
        })
    }

    /// Locations handed in by a platform shell (Android passes its app directories).
    pub fn rooted(files: &Path, cache: &Path, downloads: &Path) -> Self {
        Self {
            config: files.join("config"),
            cache: cache.to_path_buf(),
            data: files.join("data"),
            downloads: downloads.to_path_buf(),
        }
    }

    pub fn ensure(&self) -> std::io::Result<()> {
        for p in [&self.config, &self.cache, &self.data, &self.downloads] {
            std::fs::create_dir_all(p)?;
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum QualityPref {
    /// Enough resolution to fill the player (at least 720p), as far as throughput sustains.
    Auto,
    /// Never exceed this many vertical pixels.
    Max(u32),
}

/// Which video codec playback prefers when several are available.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum CodecPref {
    /// Best codec the device decodes in hardware.
    Auto,
    /// Most compatible; YouTube offers H.264 up to 1080p.
    H264,
    Vp9,
    Av1,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum AudioOut {
    M4a,
    Opus,
    /// Re-encoded through FFmpeg; desktop only (PRD 3.3).
    Mp3,
}

impl AudioOut {
    pub fn ext(self) -> &'static str {
        match self {
            AudioOut::M4a => "m4a",
            AudioOut::Opus => "opus",
            AudioOut::Mp3 => "mp3",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// `None` uses [`Paths::downloads`].
    pub download_dir: Option<PathBuf>,
    pub max_connections: u8,
    pub chunks_per_stream: u8,
    pub parallel_videos: u8,
    pub quality: QualityPref,
    pub audio_only: bool,
    pub audio_out: AudioOut,
    /// `None` follows the OS setting.
    pub reduced_motion: Option<bool>,
    /// Sign in with a code shown on screen (YouTube's TV/device flow). On by default: it is
    /// the only sign-in method that works on phones. The remote config can still disable it.
    pub tv_sign_in: bool,
    pub check_updates: bool,
    pub volume: f32,
    pub codec: CodecPref,
    /// Play the first "Up next" video when one ends.
    pub autoplay: bool,
    /// Default playback speed (0.25–3).
    pub speed: f32,
    /// Interface zoom (0.75–1.75).
    pub ui_scale: f32,
    /// Resolve videos in the background when hovered and for "Up next", so they start faster.
    pub preload: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            download_dir: None,
            max_connections: 8,
            chunks_per_stream: 4,
            parallel_videos: 2,
            quality: QualityPref::Auto,
            audio_only: false,
            audio_out: AudioOut::M4a,
            reduced_motion: None,
            tv_sign_in: true,
            check_updates: true,
            volume: 100.0,
            codec: CodecPref::Auto,
            autoplay: true,
            speed: 1.0,
            ui_scale: 1.0,
            preload: true,
        }
    }
}

impl Settings {
    pub fn clamp(&mut self) {
        self.max_connections = self.max_connections.clamp(1, MAX_CONNECTIONS_CAP);
        self.chunks_per_stream = self.chunks_per_stream.clamp(1, self.max_connections);
        self.parallel_videos = self.parallel_videos.clamp(1, self.max_connections);
        self.volume = self.volume.clamp(0.0, 130.0);
        self.speed = self.speed.clamp(0.25, 3.0);
        self.ui_scale = self.ui_scale.clamp(0.75, 1.75);
    }

    fn file(paths: &Paths) -> PathBuf {
        paths.config.join("settings.json")
    }

    pub fn load(paths: &Paths) -> Self {
        let mut s: Settings = std::fs::read(Self::file(paths))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        s.clamp();
        s
    }

    pub fn save(&self, paths: &Paths) -> std::io::Result<()> {
        write_atomic(&Self::file(paths), &serde_json::to_vec_pretty(self)?)
    }

    pub fn download_dir(&self, paths: &Paths) -> PathBuf {
        self.download_dir.clone().unwrap_or_else(|| paths.downloads.clone())
    }
}

/// Write via a temp file and rename so a crash never leaves a half-written file.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamp_caps_connections() {
        let mut s = Settings {
            max_connections: 40,
            chunks_per_stream: 30,
            parallel_videos: 0,
            ..Default::default()
        };
        s.clamp();
        assert_eq!(s.max_connections, 16);
        assert_eq!(s.chunks_per_stream, 16);
        assert_eq!(s.parallel_videos, 1);
    }
}
