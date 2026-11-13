//! Muxing (PRD 3.3). Desktop shells out to FFmpeg with stream copy; Android supplies a
//! `MediaMuxer`-backed implementation over JNI.

use std::path::{Path, PathBuf};

pub trait Muxer: Send + Sync {
    /// Combine separate video and audio streams without re-encoding.
    fn mux(&self, video: &Path, audio: &Path, out: &Path) -> Result<(), String>;
    /// Put a raw audio stream into its plain container (`.m4a`, `.opus`) without re-encoding.
    fn remux_audio(&self, audio: &Path, out: &Path) -> Result<(), String>;
    /// Re-encode audio to MP3. Desktop only.
    fn to_mp3(&self, _audio: &Path, _out: &Path) -> Result<(), String> {
        Err("MP3 export needs FFmpeg and is available on desktop only".into())
    }
    fn available(&self) -> bool {
        true
    }
    /// Should downloaded AAC audio also go through `remux_audio`? (The stream is a
    /// fragmented file; some music players only take a plain `.m4a`.)
    fn remuxes_m4a(&self) -> bool {
        false
    }
}

/// Muxer that runs an FFmpeg binary.
pub struct FfmpegMuxer {
    bin: parking_lot::RwLock<Option<PathBuf>>,
}

impl FfmpegMuxer {
    pub fn new(bin: Option<PathBuf>) -> Self {
        Self {
            bin: parking_lot::RwLock::new(bin),
        }
    }

    pub fn set_bin(&self, bin: PathBuf) {
        *self.bin.write() = Some(bin);
    }

    pub fn bin(&self) -> Option<PathBuf> {
        self.bin.read().clone()
    }

    fn run(&self, args: &[&std::ffi::OsStr]) -> Result<(), String> {
        let bin = self.bin().ok_or("FFmpeg not found")?;
        let mut cmd = std::process::Command::new(bin);
        cmd.args(["-hide_banner", "-loglevel", "error", "-y"]).args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let out = cmd.output().map_err(|e| format!("could not start FFmpeg: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "FFmpeg failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }
}

impl Muxer for FfmpegMuxer {
    fn mux(&self, video: &Path, audio: &Path, out: &Path) -> Result<(), String> {
        self.run(&[
            "-i".as_ref(),
            video.as_os_str(),
            "-i".as_ref(),
            audio.as_os_str(),
            "-map".as_ref(),
            "0:v:0".as_ref(),
            "-map".as_ref(),
            "1:a:0".as_ref(),
            "-c".as_ref(),
            "copy".as_ref(),
            out.as_os_str(),
        ])
    }

    fn remux_audio(&self, audio: &Path, out: &Path) -> Result<(), String> {
        self.run(&[
            "-i".as_ref(),
            audio.as_os_str(),
            "-vn".as_ref(),
            "-c".as_ref(),
            "copy".as_ref(),
            out.as_os_str(),
        ])
    }

    fn to_mp3(&self, audio: &Path, out: &Path) -> Result<(), String> {
        self.run(&[
            "-i".as_ref(),
            audio.as_os_str(),
            "-vn".as_ref(),
            "-c:a".as_ref(),
            "libmp3lame".as_ref(),
            "-q:a".as_ref(),
            "2".as_ref(),
            out.as_os_str(),
        ])
    }

    fn available(&self) -> bool {
        self.bin.read().is_some()
    }
}

/// FFmpeg lookup order (PRD 3.3): bundled next to the executable, then the app's data dir
/// (one-time download), then `PATH`.
pub fn find_ffmpeg(data_dir: &Path) -> Option<PathBuf> {
    let name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join(name)));
    let downloaded = data_dir.join("ffmpeg").join(name);
    bundled
        .into_iter()
        .chain(std::iter::once(downloaded))
        .find(|p| p.is_file())
        .or_else(|| {
            std::env::var_os("PATH").and_then(|paths| {
                std::env::split_paths(&paths)
                    .map(|d| d.join(name))
                    .find(|p| p.is_file())
            })
        })
}

/// One-time FFmpeg download for desktop users without one (static GPL build from BtbN).
#[cfg(not(target_os = "android"))]
pub async fn download_ffmpeg(http: &reqwest::Client, data_dir: &Path) -> Result<PathBuf, String> {
    use std::io::Read;

    let (asset, exe) = if cfg!(windows) {
        ("ffmpeg-master-latest-win64-gpl.zip", "ffmpeg.exe")
    } else if cfg!(target_arch = "aarch64") {
        ("ffmpeg-master-latest-linuxarm64-gpl.tar.xz", "ffmpeg")
    } else {
        ("ffmpeg-master-latest-linux64-gpl.tar.xz", "ffmpeg")
    };
    let url = format!("https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/{asset}");
    let bytes = http
        .get(&url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
        .map_err(|e| format!("FFmpeg download failed: {e}"))?
        .bytes()
        .await
        .map_err(|e| format!("FFmpeg download failed: {e}"))?;
    let dir = data_dir.join("ffmpeg");
    let dest = dir.join(exe);
    let dest2 = dest.clone();
    tokio::task::spawn_blocking(move || -> Result<(), String> {
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let mut found = Vec::new();
        if asset.ends_with(".zip") {
            let mut z = zip::ZipArchive::new(std::io::Cursor::new(&bytes[..]))
                .map_err(|e| e.to_string())?;
            for i in 0..z.len() {
                let mut f = z.by_index(i).map_err(|e| e.to_string())?;
                if f.name().ends_with(&format!("bin/{exe}")) {
                    f.read_to_end(&mut found).map_err(|e| e.to_string())?;
                    break;
                }
            }
        } else {
            let mut t = tar::Archive::new(xz2::read::XzDecoder::new(&bytes[..]));
            for e in t.entries().map_err(|e| e.to_string())? {
                let mut e = e.map_err(|e| e.to_string())?;
                let is_bin = e
                    .path()
                    .map(|p| p.ends_with(format!("bin/{exe}")))
                    .unwrap_or(false);
                if is_bin {
                    e.read_to_end(&mut found).map_err(|e| e.to_string())?;
                    break;
                }
            }
        }
        if found.is_empty() {
            return Err("FFmpeg archive did not contain a binary".into());
        }
        std::fs::write(&dest2, &found).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest2, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())??;
    Ok(dest)
}
