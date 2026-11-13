//! YouTube access through yt-dlp (desktop).
//!
//! yt-dlp keeps up with YouTube's player and page changes far faster than our own extractor, so
//! playback streams, downloads and the account lists rustypipe can no longer read go through it.
//! Preferred backend: a pool of persistent workers (bundled Python + `ytworker/worker.py`), so
//! each request skips yt-dlp's ~2.5 s start-up. Fallback: the standalone `yt-dlp` executable,
//! one process per request. Deno is the JavaScript runtime either way. When the user is signed
//! in, their YouTube cookies (read from the app's encrypted session) go to the worker in the
//! request itself, or to the executable through a temp file deleted right after the call.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;

use crate::model::{ChannelCard, Item, PlaylistCard, VideoCard};
use crate::secure::{EncryptedFile, Key};
use crate::streams::{ResolvedStreams, StreamChoice};

enum Backend {
    Exe(PathBuf),
    Pool(std::sync::Arc<crate::ytworker::Pool>),
}

pub struct YtDlp {
    backend: Backend,
    js_runtime: Option<PathBuf>,
    cache_dir: PathBuf,
    tmp_dir: PathBuf,
    /// rustypipe's encrypted session; its `auth_cookie` is the signed-in YouTube cookie.
    session: EncryptedFile,
}

#[derive(Deserialize)]
struct Info {
    #[serde(default)]
    title: String,
    #[serde(default)]
    uploader: Option<String>,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    is_live: Option<bool>,
    #[serde(default)]
    formats: Vec<Format>,
}

#[derive(Deserialize)]
struct Format {
    format_id: String,
    url: String,
    #[serde(default)]
    protocol: String,
    #[serde(default)]
    vcodec: Option<String>,
    #[serde(default)]
    acodec: Option<String>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    fps: Option<f64>,
    #[serde(default)]
    tbr: Option<f64>,
    #[serde(default)]
    filesize: Option<u64>,
    #[serde(default)]
    ext: Option<String>,
    #[serde(default)]
    language_preference: Option<i32>,
    #[serde(default)]
    dynamic_range: Option<String>,
    #[serde(default)]
    http_headers: std::collections::BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct FlatList {
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    uploader: Option<String>,
    #[serde(default)]
    playlist_count: Option<u64>,
    #[serde(default)]
    thumbnails: Vec<Thumb>,
    #[serde(default)]
    entries: Vec<FlatEntry>,
}

#[derive(Deserialize)]
struct FlatEntry {
    #[serde(default)]
    id: String,
    #[serde(default)]
    url: String,
    #[serde(default)]
    ie_key: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    uploader: Option<String>,
    #[serde(default)]
    channel_id: Option<String>,
    #[serde(default)]
    view_count: Option<u64>,
    #[serde(default)]
    channel_follower_count: Option<u64>,
    #[serde(default)]
    live_status: Option<String>,
    #[serde(default)]
    thumbnails: Vec<Thumb>,
}

#[derive(Deserialize)]
struct Thumb {
    url: String,
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
}

/// A list fetched with `--flat-playlist`.
pub struct FlatResult {
    pub title: String,
    pub channel: Option<String>,
    pub count: Option<u64>,
    pub thumbnail: Option<String>,
    pub items: Vec<Item>,
}

fn exe(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_string() }
}

/// Look for a binary next to the app, in the app's data folder, then on PATH.
fn locate(name: &str, data_dir: &Path) -> Option<PathBuf> {
    let file = exe(name);
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join(&file)));
    bundled
        .into_iter()
        .chain([data_dir.join(name).join(&file)])
        .find(|p| p.is_file())
        .or_else(|| {
            std::env::var_os("PATH").and_then(|paths| {
                std::env::split_paths(&paths)
                    .map(|d| d.join(&file))
                    .find(|p| p.is_file())
            })
        })
}

fn hide_window(cmd: &mut tokio::process::Command) {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

fn last_error(stderr: &[u8]) -> String {
    let err = String::from_utf8_lossy(stderr);
    err.lines()
        .rev()
        .find(|l| l.contains("ERROR"))
        .unwrap_or_else(|| err.trim())
        .trim_start_matches("ERROR: ")
        .to_string()
}

/// The bundled worker: `ytworker/python/python(.exe)` + `ytworker/worker.py`, next to the app
/// or in the data folder.
fn locate_worker(data_dir: &Path) -> Option<(PathBuf, PathBuf)> {
    let py = if cfg!(windows) { "python/python.exe" } else { "python/bin/python3" };
    let next_to_exe = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join("ytworker")));
    next_to_exe
        .into_iter()
        .chain([data_dir.join("ytworker")])
        .map(|d| (d.join(py), d.join("worker.py")))
        .find(|(p, s)| p.is_file() && s.is_file())
}

impl YtDlp {
    /// Find yt-dlp (and Deno for YouTube's JavaScript challenges). `None` if yt-dlp is missing.
    /// Must be called inside the tokio runtime (it starts the worker pool).
    pub fn find(data_dir: &Path, cache_dir: &Path, key: &Key) -> Option<Self> {
        let js_runtime = locate("deno", data_dir);
        if js_runtime.is_none() {
            tracing::warn!("no Deno found; many videos will not resolve");
        }
        let cache = cache_dir.join("yt-dlp");
        let backend = if let Some((python, script)) = locate_worker(data_dir) {
            tracing::info!("yt-dlp workers: {} (deno: {:?})", script.display(), js_runtime);
            let mut env = vec![("STREAMPIPE_YTDLP_CACHE".to_string(), cache.display().to_string())];
            if let Some(js) = &js_runtime {
                env.push(("STREAMPIPE_DENO".into(), js.display().to_string()));
            }
            if let Some(bg) = locate("rustypipe-botguard", data_dir) {
                env.push(("STREAMPIPE_BOTGUARD".into(), bg.display().to_string()));
            }
            // Every worker warms up with the signed-in cookies while idle, so the first
            // video opened does not pay the start-up costs.
            let session = EncryptedFile::new(data_dir.join("session.bin"), key);
            let warm: crate::ytworker::WarmReq =
                std::sync::Arc::new(move || serde_json::json!({"op": "warm", "cookies": session_cookies(&session)}));
            Backend::Pool(crate::ytworker::Pool::start(
                crate::ytworker::Spawn { python, script, env },
                3,
                3,
                Some(warm),
            ))
        } else {
            let bin = locate("yt-dlp", data_dir)?;
            tracing::info!("yt-dlp: {} (deno: {:?})", bin.display(), js_runtime);
            Backend::Exe(bin)
        };
        Some(Self {
            backend,
            js_runtime,
            cache_dir: cache_dir.join("yt-dlp"),
            tmp_dir: cache_dir.join("yt-dlp-tmp"),
            session: EncryptedFile::new(data_dir.join("session.bin"), key),
        })
    }

    pub fn cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    /// The signed-in YouTube cookie as cookies.txt, taken from the app's session.
    fn cookies_txt(&self) -> Option<String> {
        session_cookies(&self.session)
    }

    /// Run the yt-dlp executable with `args` (+ common flags and cookies) and return stdout.
    async fn run(&self, bin: &Path, args: &[&str], url: &str, timeout_s: u64) -> Result<Vec<u8>, String> {
        let mut cmd = tokio::process::Command::new(bin);
        cmd.args(["--no-warnings", "--no-progress", "--ignore-config"])
            .args(args)
            .arg("--cache-dir")
            .arg(&self.cache_dir);
        if let Some(js) = &self.js_runtime {
            cmd.arg("--js-runtimes").arg(format!("deno:{}", js.display()));
        }
        let cookie_file = match self.cookies_txt() {
            Some(txt) => {
                let _ = std::fs::create_dir_all(&self.tmp_dir);
                let mut name = [0u8; 8];
                rand::RngCore::fill_bytes(&mut rand::rng(), &mut name);
                let p = self.tmp_dir.join(format!("c-{}.txt", hex::encode(name)));
                std::fs::write(&p, txt).map_err(|e| e.to_string())?;
                cmd.arg("--cookies").arg(&p);
                Some(p)
            }
            None => None,
        };
        cmd.arg(url)
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        hide_window(&mut cmd);
        let out = tokio::time::timeout(Duration::from_secs(timeout_s), cmd.output()).await;
        if let Some(p) = &cookie_file {
            let _ = std::fs::remove_file(p);
        }
        let out = out
            .map_err(|_| "yt-dlp took too long".to_string())?
            .map_err(|e| format!("could not run yt-dlp: {e}"))?;
        if !out.status.success() {
            return Err(last_error(&out.stderr));
        }
        Ok(out.stdout)
    }

    /// Playable streams for one video.
    /// `urgent`: the user is waiting (playback/download), so skip ahead of background work.
    /// `full`: skip the fast single-client path and use yt-dlp's full client set (used after
    /// YouTube refused streams from the fast path).
    pub async fn resolve(&self, video_id: &str, urgent: bool, full: bool) -> Result<ResolvedStreams, String> {
        let started = std::time::Instant::now();
        let url = format!("https://www.youtube.com/watch?v={video_id}");
        let r = match &self.backend {
            Backend::Pool(pool) => {
                let req = serde_json::json!({"op": "resolve", "url": url, "full": full, "cookies": self.cookies_txt()});
                let v = pool.call_with(req, Duration::from_secs(75), urgent).await?;
                parse_info(serde_json::from_value(v).map_err(|e| format!("unexpected worker output: {e}"))?)
            }
            Backend::Exe(bin) => parse(&self.run(bin, &["-J", "--no-playlist"], &url, 75).await?),
        };
        tracing::info!("yt-dlp resolved {video_id} in {:.1}s", started.elapsed().as_secs_f32());
        r
    }

    /// A playlist or feed page as a flat list (no per-video requests). `limit` caps entries.
    pub async fn flat(&self, url: &str, limit: usize) -> Result<FlatResult, String> {
        match &self.backend {
            Backend::Pool(pool) => {
                let req = serde_json::json!({"op": "flat", "url": url, "limit": limit, "cookies": self.cookies_txt()});
                let v = pool.call(req, Duration::from_secs(90)).await?;
                flat_from(serde_json::from_value(v).map_err(|e| format!("unexpected worker output: {e}"))?)
            }
            Backend::Exe(bin) => {
                let end = limit.to_string();
                parse_flat(&self.run(bin, &["-J", "--flat-playlist", "--playlist-end", &end], url, 90).await?)
            }
        }
    }

    pub async fn version(&self) -> Option<String> {
        let bin = match &self.backend {
            Backend::Pool(pool) => {
                let v = pool.call(serde_json::json!({"op": "version"}), Duration::from_secs(30)).await.ok()?;
                return v["version"].as_str().map(str::to_string);
            }
            Backend::Exe(bin) => bin,
        };
        let mut cmd = tokio::process::Command::new(bin);
        cmd.arg("--version").stdin(std::process::Stdio::null());
        hide_window(&mut cmd);
        let out = cmd.output().await.ok()?;
        Some(String::from_utf8_lossy(&out.stdout).trim().to_string()).filter(|v| !v.is_empty())
    }

    /// Update the bundled yt-dlp to the latest release (yt-dlp's own `-U`).
    pub async fn self_update(&self) -> Result<String, String> {
        let bin = match &self.backend {
            Backend::Pool(pool) => {
                let v = pool.call(serde_json::json!({"op": "update"}), Duration::from_secs(300)).await?;
                if v["restart"].as_bool() == Some(true) {
                    pool.restart();
                }
                return Ok(v["message"].as_str().unwrap_or("yt-dlp updated").to_string());
            }
            Backend::Exe(bin) => bin,
        };
        let mut cmd = tokio::process::Command::new(bin);
        cmd.args(["-U", "--no-warnings"]).stdin(std::process::Stdio::null());
        hide_window(&mut cmd);
        let out = tokio::time::timeout(Duration::from_secs(180), cmd.output())
            .await
            .map_err(|_| "update timed out".to_string())?
            .map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&out.stdout);
        if out.status.success() {
            Ok(text.lines().last().unwrap_or("yt-dlp is up to date").trim().to_string())
        } else {
            Err(last_error(&out.stderr))
        }
    }
}

/// The signed-in YouTube cookie in the app's session, as cookies.txt.
fn session_cookies(session: &EncryptedFile) -> Option<String> {
    let json: serde_json::Value = serde_json::from_slice(&session.read()?).ok()?;
    let header = json.get("auth_cookie")?.get("cookie")?.as_str()?;
    Some(netscape_from_header(header))
}

/// Turn a `name=value; name2=value2` cookie header into Netscape cookies.txt for youtube.com.
pub(crate) fn netscape_from_header(header: &str) -> String {
    let mut s = String::from("# Netscape HTTP Cookie File\n");
    for pair in header.split(';') {
        if let Some((k, v)) = pair.trim().split_once('=') {
            if !k.is_empty() {
                s.push_str(&format!(".youtube.com\tTRUE\t/\tTRUE\t2000000000\t{k}\t{v}\n"));
            }
        }
    }
    s
}

fn norm_vcodec(c: &str) -> Option<&'static str> {
    if c.starts_with("avc1") {
        Some("h264")
    } else if c.starts_with("vp9") || c.starts_with("vp09") {
        Some("vp9")
    } else if c.starts_with("av01") {
        Some("av1")
    } else {
        None
    }
}

fn expire(url: &str) -> Option<i64> {
    url.split(['?', '&'])
        .find_map(|kv| kv.strip_prefix("expire="))
        .and_then(|v| v.parse().ok())
}

fn choice(f: &Format, codec: &str) -> StreamChoice {
    StreamChoice {
        url: f.url.clone(),
        itag: f
            .format_id
            .split('-')
            .next()
            .and_then(|i| i.parse().ok())
            .unwrap_or(0),
        mime: match (f.ext.as_deref(), f.vcodec.as_deref()) {
            (Some("webm"), Some(v)) if v != "none" => "video/webm".into(),
            (Some("webm"), _) => "audio/webm".into(),
            (_, Some(v)) if v != "none" => "video/mp4".into(),
            _ => "audio/mp4".into(),
        },
        codec: codec.into(),
        height: f.height.unwrap_or(0),
        fps: f.fps.unwrap_or(0.0).round() as u8,
        bitrate: (f.tbr.unwrap_or(0.0) * 1000.0) as u32,
        // Only exact sizes: the stream server promises players an exact Content-Length.
        // Without one, the `clen` URL parameter is used instead.
        size: f.filesize,
        headers: f
            .http_headers
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect(),
        init_range: None,
        index_range: None,
    }
}

pub(crate) fn parse(json: &[u8]) -> Result<ResolvedStreams, String> {
    parse_info(serde_json::from_slice(json).map_err(|e| format!("unexpected yt-dlp output: {e}"))?)
}

fn parse_info(info: Info) -> Result<ResolvedStreams, String> {
    let http = |f: &&Format| f.protocol == "https" || f.protocol == "http";
    let mut video = Vec::new();
    for f in info.formats.iter().filter(http) {
        let v = f.vcodec.as_deref().unwrap_or("none");
        let a = f.acodec.as_deref().unwrap_or("none");
        if a != "none" || f.format_id.contains("-drc") {
            continue;
        }
        if f.dynamic_range.as_deref().is_some_and(|d| d != "SDR") {
            continue;
        }
        if let Some(c) = norm_vcodec(v) {
            video.push(choice(f, c));
        }
    }
    let best_audio = |prefix: &str, name: &str| {
        info.formats
            .iter()
            .filter(http)
            .filter(|f| f.vcodec.as_deref().unwrap_or("none") == "none")
            .filter(|f| !f.format_id.contains("-drc"))
            .filter(|f| f.acodec.as_deref().is_some_and(|a| a.starts_with(prefix)))
            .max_by(|a, b| {
                (a.language_preference.unwrap_or(0), a.tbr.unwrap_or(0.0) as i64)
                    .cmp(&(b.language_preference.unwrap_or(0), b.tbr.unwrap_or(0.0) as i64))
            })
            .map(|f| choice(f, name))
    };
    let audio_aac = best_audio("mp4a", "aac");
    let audio_opus = best_audio("opus", "opus");
    let hls = info
        .is_live
        .unwrap_or(false)
        .then(|| {
            info.formats
                .iter()
                .filter(|f| f.protocol.starts_with("m3u8"))
                .max_by_key(|f| f.height.unwrap_or(0))
                .map(|f| f.url.clone())
        })
        .flatten();
    let valid_until = video
        .iter()
        .chain(audio_aac.iter())
        .find_map(|s| expire(&s.url))
        .unwrap_or_else(|| time::OffsetDateTime::now_utc().unix_timestamp() + 5 * 3600);
    let r = ResolvedStreams {
        title: info.title,
        channel: info.channel.or(info.uploader).unwrap_or_default(),
        duration_s: info.duration.unwrap_or(0.0) as u32,
        video,
        audio_aac,
        audio_opus,
        hls,
        valid_until,
        source: "yt-dlp".into(),
        audio_tracks: Vec::new(),
        audio_drc: None,
    };
    if r.is_empty() {
        return Err("yt-dlp returned no playable formats".into());
    }
    Ok(r)
}

/// Pick the thumbnail closest to `target` px wide (same rule as the rest of the app).
fn thumb_url(thumbs: &[Thumb], target: u32) -> Option<String> {
    thumbs
        .iter()
        .min_by_key(|t| {
            let w = t.width.or(t.height.map(|h| h * 16 / 9)).unwrap_or(target);
            if w >= target { w - target } else { (target - w) * 4 }
        })
        .map(|t| if t.url.starts_with("//") { format!("https:{}", t.url) } else { t.url.clone() })
}

pub(crate) fn parse_flat(json: &[u8]) -> Result<FlatResult, String> {
    flat_from(serde_json::from_slice(json).map_err(|e| format!("unexpected yt-dlp output: {e}"))?)
}

fn flat_from(list: FlatList) -> Result<FlatResult, String> {
    let items = list
        .entries
        .into_iter()
        .filter_map(|e| {
            let name = e.title.clone().unwrap_or_default();
            let is_tab = e.ie_key.as_deref() == Some("YoutubeTab");
            if is_tab && (e.url.contains("/channel/") || e.url.contains("/@") || e.id.starts_with("UC")) {
                return Some(Item::Channel(ChannelCard {
                    id: e.id,
                    name,
                    avatar: thumb_url(&e.thumbnails, 88),
                    subscribers: e.channel_follower_count,
                }));
            }
            if is_tab && (e.url.contains("list=") || e.id.starts_with("PL")) {
                return Some(Item::Playlist(PlaylistCard {
                    id: e.id,
                    name,
                    thumbnail: thumb_url(&e.thumbnails, 360),
                    channel_name: e.channel.or(e.uploader),
                    video_count: None,
                }));
            }
            if e.id.len() != 11 {
                return None;
            }
            Some(Item::Video(VideoCard {
                channel_avatar: None,
                thumbnail: thumb_url(&e.thumbnails, 360)
                    .or_else(|| Some(format!("https://i.ytimg.com/vi/{}/mqdefault.jpg", e.id))),
                id: e.id,
                title: name,
                channel_id: e.channel_id,
                channel_name: e.channel.or(e.uploader),
                duration_s: e.duration.map(|d| d as u32),
                views: e.view_count,
                published: None,
                is_live: e.live_status.as_deref() == Some("is_live"),
                watched: None,
            }))
        })
        .collect();
    Ok(FlatResult {
        title: list.title.unwrap_or_default(),
        channel: list.channel.or(list.uploader),
        count: list.playlist_count,
        thumbnail: thumb_url(&list.thumbnails, 360),
        items,
    })
}

#[cfg(test)]
mod tests {
    use crate::model::Item;

    #[test]
    fn parses_formats() {
        let j = br#"{"title":"T","channel":"C","duration":10.5,"formats":[
          {"format_id":"140","url":"https://x/a?expire=123","protocol":"https","vcodec":"none","acodec":"mp4a.40.2","tbr":129.5,"filesize":100,"ext":"m4a","http_headers":{"User-Agent":"UA"}},
          {"format_id":"140-drc","url":"https://x/d","protocol":"https","vcodec":"none","acodec":"mp4a.40.2","tbr":300,"ext":"m4a"},
          {"format_id":"251","url":"https://x/o","protocol":"https","vcodec":"none","acodec":"opus","tbr":140,"ext":"webm"},
          {"format_id":"136","url":"https://x/v","protocol":"https","vcodec":"avc1.4d401f","acodec":"none","height":720,"fps":30,"tbr":1200,"ext":"mp4"},
          {"format_id":"18","url":"https://x/m","protocol":"https","vcodec":"avc1.42001E","acodec":"mp4a.40.2","height":360,"ext":"mp4"},
          {"format_id":"sb0","url":"https://x/sb","protocol":"mhtml","vcodec":"none","acodec":"none"}]}"#;
        let r = super::parse(j).unwrap();
        assert_eq!((r.title.as_str(), r.channel.as_str(), r.duration_s), ("T", "C", 10));
        assert_eq!(r.video.len(), 1);
        assert_eq!((r.video[0].codec.as_str(), r.video[0].height, r.video[0].itag), ("h264", 720, 136));
        let a = r.audio_aac.unwrap();
        assert_eq!((a.itag, a.headers.len()), (140, 1));
        assert_eq!(r.audio_opus.unwrap().itag, 251);
        assert_eq!(r.valid_until, 123);
    }

    #[test]
    fn cookie_header_to_netscape() {
        let t = super::netscape_from_header("SID=a; __Secure-3PAPISID=b=c; bad");
        let lines: Vec<&str> = t.lines().skip(1).collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[1].ends_with("__Secure-3PAPISID\tb=c"));
    }

    #[test]
    fn parses_flat_lists() {
        let j = br#"{"title":"Mix","channel":"Me","playlist_count":3,"entries":[
          {"id":"dQw4w9WgXcQ","url":"https://www.youtube.com/watch?v=dQw4w9WgXcQ","ie_key":"Youtube","title":"Song","duration":212,"channel":"Rick","view_count":5,
           "thumbnails":[{"url":"https://i.ytimg.com/a.jpg","width":336,"height":188}]},
          {"id":"UCuAXFkgsw1L7xaCfnd5JJOw","url":"https://www.youtube.com/channel/UCuAXFkgsw1L7xaCfnd5JJOw","ie_key":"YoutubeTab","title":"Rick Astley",
           "thumbnails":[{"url":"//yt3.ggpht.com/x.jpg","width":88,"height":88}]},
          {"id":"bad","url":"x","ie_key":"Youtube"}]}"#;
        let r = super::parse_flat(j).unwrap();
        assert_eq!((r.title.as_str(), r.count), ("Mix", Some(3)));
        assert_eq!(r.items.len(), 2);
        assert!(matches!(&r.items[0], Item::Video(v) if v.duration_s == Some(212)));
        assert!(matches!(&r.items[1], Item::Channel(c) if c.avatar.as_deref() == Some("https://yt3.ggpht.com/x.jpg")));
    }
}
