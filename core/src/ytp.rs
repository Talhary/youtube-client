//! Direct stream resolver: a video id becomes plain stream URLs in about a second.
//!
//! A port of the method in `talhary-dot/yt-parser` (its `MAP.md` has the measurements):
//!
//! - **public path**: the iOS client. One request, plain URLs, no cookies, no JavaScript.
//! - **signed-in path**: the TV client with the user's cookies. Works for age-restricted and
//!   account-gated videos and when the network is refused on the public path. Its URLs carry
//!   an `n` challenge (and sometimes a signature challenge) that must be solved with code
//!   from YouTube's player script; yt-dlp's EJS solver does that on the embedded JS engine.
//!
//! Both run at once; the public answer is used when it is good, otherwise the signed-in one.
//! The solver keeps one JS engine alive with the analysed player loaded, so after the first
//! video a solve costs milliseconds.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::streams::{ResolvedStreams, StreamChoice};

const ORIGIN: &str = "https://www.youtube.com";
const API: &str = "https://www.youtube.com/youtubei/v1/player?prettyPrint=false";
/// How long a player version is trusted before YouTube is asked again.
const PLAYER_TTL_S: i64 = 6 * 3600;

/// yt-dlp's EJS challenge solver (Unlicense), see `core/assets/ejs/LICENSE`.
const EJS_LIB: &str = include_str!("../assets/ejs/lib.min.js");
const EJS_CORE: &str = include_str!("../assets/ejs/core.min.js");

struct Client {
    name: &'static str,
    id: u32,
    version: &'static str,
    /// Extra `context.client` fields.
    fields: &'static [(&'static str, &'static str)],
    user_agent: &'static str,
    /// Sends the user's cookies and needs the challenge solver.
    signed_in: bool,
}

const IOS: Client = Client {
    name: "IOS",
    id: 5,
    version: "20.45.1",
    fields: &[
        ("deviceMake", "Apple"),
        ("deviceModel", "iPhone16,2"),
        ("osName", "iPhone"),
        ("osVersion", "18.6.2.22G100"),
    ],
    user_agent: "com.google.ios.youtube/20.45.1 (iPhone16,2; U; CPU iOS 18_6_2 like Mac OS X;)",
    signed_in: false,
};

const TV: Client = Client {
    name: "TVHTML5",
    id: 7,
    version: "5.20260707",
    fields: &[
        ("deviceMake", "Samsung"),
        ("deviceModel", "UKS9800"),
        ("osName", "Tizen"),
        ("osVersion", "2.4.0"),
    ],
    user_agent: "Mozilla/5.0 (SMART-TV; Linux; Tizen 2.4.0) AppleWebKit/538.1 (KHTML, like Gecko) Version/2.4.0 TV Safari/538.1",
    signed_in: true,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct PlayerInfo {
    id: String,
    sts: u32,
    /// Unix seconds when it was fetched.
    ts: i64,
}

fn now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid pattern"))
}

/// `Authorization` for a cookie session: one hash per SAPISID cookie present.
fn sapisid_auth(cookie: &str) -> String {
    use sha1::{Digest, Sha1};
    let value = |name: &str| {
        cookie.split(';').find_map(|p| {
            let (k, v) = p.trim().split_once('=')?;
            (k == name).then_some(v)
        })
    };
    let ts = now();
    [
        ("SAPISIDHASH", "SAPISID"),
        ("SAPISID1PHASH", "__Secure-1PAPISID"),
        ("SAPISID3PHASH", "__Secure-3PAPISID"),
    ]
    .iter()
    .filter_map(|(scheme, name)| {
        let v = value(name)?;
        let digest = Sha1::digest(format!("{ts} {v} {ORIGIN}").as_bytes());
        Some(format!("{scheme} {ts}_{}", hex::encode(digest)))
    })
    .collect::<Vec<_>>()
    .join(" ")
}

/// Percent-encode a query value (everything but unreserved characters).
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(b) => {
                        out.push(b);
                        i += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        i += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A query string as decoded pairs.
fn query_pairs(q: &str) -> Vec<(String, String)> {
    q.split('&')
        .filter(|p| !p.is_empty())
        .map(|p| match p.split_once('=') {
            Some((k, v)) => (decode(k), decode(v)),
            None => (decode(p), String::new()),
        })
        .collect()
}

/// The decoded value of one query parameter of a URL.
fn param(url: &str, name: &str) -> Option<String> {
    let q = url.split_once('?')?.1;
    query_pairs(q).into_iter().find(|(k, _)| k == name).map(|(_, v)| v)
}

/// A format as YouTube's player answer lists it, before its URL is made usable.
struct RawFormat {
    json: Value,
    url: String,
    /// Signature challenge and the parameter its answer goes in.
    sig: Option<(String, String)>,
}

fn raw_formats(streaming: &Value) -> Vec<RawFormat> {
    let list = |key: &str| streaming.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
    list("formats")
        .into_iter()
        .chain(list("adaptiveFormats"))
        .filter(|f| f.get("drmFamilies").is_none())
        .filter_map(|f| {
            if let Some(u) = f.get("url").and_then(Value::as_str) {
                return Some(RawFormat { url: u.to_string(), sig: None, json: f });
            }
            let cipher = query_pairs(f.get("signatureCipher").and_then(Value::as_str)?);
            let get = |k: &str| cipher.iter().find(|(key, _)| key == k).map(|(_, v)| v.clone());
            let url = get("url")?;
            let sig = get("s").map(|s| (s, get("sp").unwrap_or_else(|| "signature".into())));
            Some(RawFormat { url, sig, json: f })
        })
        .collect()
}

fn range(f: &Value, key: &str) -> Option<(u64, u64)> {
    let r = f.get(key)?;
    let n = |k: &str| r.get(k).and_then(Value::as_str).and_then(|s| s.parse::<u64>().ok());
    Some((n("start")?, n("end")?))
}

/// One usable stream out of a player answer's format entry.
fn choice(f: &Value, url: String, codec: &str, user_agent: &str) -> StreamChoice {
    let num = |k: &str| f.get(k).and_then(Value::as_u64).unwrap_or(0);
    StreamChoice {
        url,
        itag: num("itag") as u32,
        // Kept whole (with `codecs="…"`): the player needs it to open a decoder.
        mime: f.get("mimeType").and_then(Value::as_str).unwrap_or("").to_string(),
        codec: codec.into(),
        height: num("height") as u32,
        fps: num("fps").min(255) as u8,
        bitrate: num("bitrate") as u32,
        size: f.get("contentLength").and_then(Value::as_str).and_then(|s| s.parse().ok()),
        headers: vec![("User-Agent".into(), user_agent.into())],
        init_range: range(f, "initRange"),
        index_range: range(f, "indexRange"),
    }
}

fn codecs_of(f: &Value) -> String {
    static CODECS: OnceLock<Regex> = OnceLock::new();
    let mime = f.get("mimeType").and_then(Value::as_str).unwrap_or("");
    re(&CODECS, r#"codecs="([^"]+)""#)
        .captures(mime)
        .map(|c| c[1].to_string())
        .unwrap_or_default()
}

/// Turn a player answer plus finished URLs into the app's stream set.
fn build(player: &Value, formats: Vec<(Value, String)>, client: &Client) -> ResolvedStreams {
    let details = player.get("videoDetails").cloned().unwrap_or(Value::Null);
    let streaming = player.get("streamingData").cloned().unwrap_or(Value::Null);
    let text = |k: &str| details.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    let mut video = Vec::new();
    let mut aac: Option<(i64, StreamChoice)> = None;
    let mut opus: Option<(i64, StreamChoice)> = None;
    for (f, url) in formats {
        let mime = f.get("mimeType").and_then(Value::as_str).unwrap_or("");
        let codecs = codecs_of(&f);
        if codecs.contains(',') {
            continue; // muxed (360p) formats are not needed: video and audio play as a pair
        }
        if mime.starts_with("video/") {
            // HDR needs a display path the player does not handle; skip those variants.
            let hdr = f.get("qualityLabel").and_then(Value::as_str).is_some_and(|q| q.contains("HDR"));
            let codec = match codecs.as_str() {
                c if c.starts_with("avc1") => "h264",
                c if c.starts_with("vp9") || c.starts_with("vp09") => "vp9",
                c if c.starts_with("av01") => "av1",
                _ => continue,
            };
            if !hdr {
                video.push(choice(&f, url, codec, client.user_agent));
            }
        } else if mime.starts_with("audio/") {
            if f.get("isDrc").and_then(Value::as_bool).unwrap_or(false) {
                continue;
            }
            // The original-language track wins, then the higher bitrate.
            let default = f
                .get("audioTrack")
                .map(|t| t.get("audioIsDefault").and_then(Value::as_bool).unwrap_or(false))
                .unwrap_or(true);
            let score = i64::from(default) * 1_000_000_000 + f.get("bitrate").and_then(Value::as_i64).unwrap_or(0);
            let slot = if codecs.starts_with("mp4a") {
                &mut aac
            } else if codecs.starts_with("opus") {
                &mut opus
            } else {
                continue;
            };
            let name = if codecs.starts_with("mp4a") { "aac" } else { "opus" };
            if slot.as_ref().is_none_or(|(best, _)| score > *best) {
                *slot = Some((score, choice(&f, url, name, client.user_agent)));
            }
        }
    }
    let live = details.get("isLive").and_then(Value::as_bool).unwrap_or(false);
    let expires = streaming
        .get("expiresInSeconds")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(5 * 3600);
    ResolvedStreams {
        title: text("title"),
        channel: text("author"),
        duration_s: text("lengthSeconds").parse().unwrap_or(0),
        video,
        audio_aac: aac.map(|(_, c)| c),
        audio_opus: opus.map(|(_, c)| c),
        hls: live
            .then(|| streaming.get("hlsManifestUrl").and_then(Value::as_str).map(str::to_string))
            .flatten(),
        valid_until: now() + expires,
        source: format!("direct-{}", client.name.to_lowercase()),
        audio_tracks: Vec::new(),
        audio_drc: None,
    }
}

// ------------------------------------------------------------------ library resolver

/// A complete resolver outside this module (the yt-parser library, `libytp.so`): given a
/// video id and the session's `Cookie` header, it answers with yt-parser's JSON. The last
/// argument asks for the quick answer, without the library's preview check.
pub type Library = Box<dyn Fn(&str, Option<&str>, bool) -> Result<Value, String> + Send + Sync>;

static LIBRARY: OnceLock<Library> = OnceLock::new();

/// Resolve through the yt-parser library instead of the code in this module.
pub fn use_library(resolve: Library) {
    let _ = LIBRARY.set(resolve);
}

/// Is the yt-parser library in use?
pub fn has_library() -> bool {
    LIBRARY.get().is_some()
}

/// The app's stream set from yt-parser's JSON (`ytp URL -j`).
pub fn from_library(j: &Value) -> Result<ResolvedStreams, String> {
    if let Some(e) = j.get("error").and_then(Value::as_str) {
        let tried: Vec<&str> = j.get("tried").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
        return Err(if tried.is_empty() { e.to_string() } else { format!("{e} ({})", tried.join("; ")) });
    }
    let text = |k: &str| j.get(k).and_then(Value::as_str).unwrap_or("").to_string();
    // Which requests the library made and how long each took.
    tracing::info!(
        "yt-parser: {} timing {}",
        j.get("tried").map(|v| v.to_string()).unwrap_or_default(),
        j.get("timing").map(|v| v.to_string()).unwrap_or_default()
    );
    let mut video = Vec::new();
    // Videos with dubbed audio list one track per language; "audio_default" marks the original.
    let has_default = j.get("formats").and_then(Value::as_array).into_iter().flatten()
        .any(|f| f.get("audio_default").and_then(Value::as_bool).unwrap_or(false));
    let mut aac: Option<StreamChoice> = None;
    let mut opus: Option<StreamChoice> = None;
    let mut drc: Option<StreamChoice> = None;
    // Dubbed sound: the best AAC stream of each language, by its name.
    let mut tracks: Vec<crate::streams::AudioTrack> = Vec::new();
    for f in j.get("formats").and_then(Value::as_array).into_iter().flatten() {
        let s = |k: &str| f.get(k).and_then(Value::as_str).unwrap_or("");
        let n = |k: &str| f.get(k).and_then(Value::as_u64).unwrap_or(0);
        let flag = |k: &str| f.get(k).and_then(Value::as_bool).unwrap_or(false);
        let (kind, codecs, url) = (s("kind"), s("codecs"), s("url"));
        // Not used: protected streams, the loudness-levelled audio variants, HDR pictures
        // and files that carry picture and sound together.
        if url.is_empty() || flag("drm") || s("res").contains("HDR") || codecs.contains(',') {
            continue;
        }
        let other_language = kind == "audio" && has_default && !flag("audio_default");
        let codec = match codecs.split('.').next().unwrap_or("") {
            "avc1" => "h264",
            "av01" => "av1",
            "vp09" | "vp9" => "vp9",
            "mp4a" => "aac",
            "opus" => "opus",
            _ => continue,
        };
        let choice = StreamChoice {
            url: url.to_string(),
            itag: n("itag") as u32,
            mime: format!("{kind}/{}; codecs=\"{codecs}\"", s("ext")),
            codec: codec.to_string(),
            height: n("height") as u32,
            fps: n("fps").min(255) as u8,
            bitrate: n("bitrate").min(u32::MAX as u64) as u32,
            size: f.get("size").and_then(Value::as_u64).filter(|n| *n > 0),
            headers: vec![("User-Agent".to_string(), s("ua").to_string())],
            init_range: None,
            index_range: None,
        };
        let better = |old: &Option<StreamChoice>| old.as_ref().map_or(true, |o| choice.bitrate > o.bitrate);
        if kind == "audio" && codec == "aac" && has_default && !flag("drc") {
            let name = f.get("lang").and_then(Value::as_str).unwrap_or("").to_string();
            match tracks.iter_mut().find(|t| t.name == name) {
                Some(t) if choice.bitrate > t.stream.bitrate => t.stream = choice.clone(),
                Some(_) => {}
                None => tracks.push(crate::streams::AudioTrack { name, original: flag("audio_default"), stream: choice.clone() }),
            }
        }
        if other_language {
            continue;
        }
        if flag("drc") {
            if kind == "audio" && codec == "aac" && better(&drc) {
                drc = Some(choice);
            }
            continue;
        }
        match (kind, codec) {
            ("video", _) => video.push(choice),
            ("audio", "aac") if better(&aac) => aac = Some(choice),
            ("audio", "opus") if better(&opus) => opus = Some(choice),
            _ => {}
        }
    }
    let streams = ResolvedStreams {
        title: text("title"),
        channel: text("author"),
        duration_s: j.get("duration").and_then(Value::as_u64).unwrap_or(0) as u32,
        video,
        audio_aac: aac,
        audio_opus: opus,
        audio_tracks: {
            tracks.sort_by_key(|t| !t.original);
            if tracks.len() > 1 { tracks } else { Vec::new() }
        },
        audio_drc: drc,
        hls: j.get("hls").and_then(Value::as_str).map(str::to_string),
        valid_until: now() + j.get("expires_in").and_then(Value::as_i64).unwrap_or(0),
        // "-nopyv": the library found the first URLs to be a preview and asked again.
        source: format!(
            "ytp-{}{}",
            text("client"),
            if j.pointer("/timing/player_tv_nopyv").is_some() { "-nopyv" } else { "" }
        ),
    };
    if streams.is_empty() {
        return Err("no playable streams".into());
    }
    Ok(streams)
}

// ------------------------------------------------------------------ challenge solver

struct Job {
    player_id: String,
    player_js: PathBuf,
    preprocessed: PathBuf,
    n: Vec<String>,
    sig: Vec<String>,
    reply: tokio::sync::oneshot::Sender<Result<HashMap<String, String>, String>>,
}

/// One long-lived JS engine on its own thread (the engine is not thread-safe, and analysing
/// the player script needs a deep stack).
struct Solver {
    tx: std::sync::Mutex<Option<std::sync::mpsc::Sender<Job>>>,
}

impl Solver {
    fn new() -> Self {
        Self { tx: std::sync::Mutex::new(None) }
    }

    async fn solve(&self, job_parts: (String, PathBuf, PathBuf, Vec<String>, Vec<String>)) -> Result<HashMap<String, String>, String> {
        if NODE.get().is_some() {
            let (_, player_js, preprocessed, n, sig) = job_parts;
            return tokio::task::spawn_blocking(move || node_solve(&player_js, &preprocessed, &n, &sig))
                .await
                .map_err(|e| format!("the solver stopped: {e}"))?;
        }
        let (reply, rx) = tokio::sync::oneshot::channel();
        let job = Job {
            player_id: job_parts.0,
            player_js: job_parts.1,
            preprocessed: job_parts.2,
            n: job_parts.3,
            sig: job_parts.4,
            reply,
        };
        {
            let mut tx = self.tx.lock().map_err(|_| "solver lock poisoned".to_string())?;
            if tx.is_none() {
                let (s, r) = std::sync::mpsc::channel::<Job>();
                std::thread::Builder::new()
                    .name("streampipe-solver".into())
                    .stack_size(64 << 20)
                    .spawn(move || solver_thread(r))
                    .map_err(|e| format!("could not start the solver: {e}"))?;
                *tx = Some(s);
            }
            tx.as_ref().unwrap().send(job).map_err(|_| "the solver stopped".to_string())?;
        }
        tokio::time::timeout(Duration::from_secs(std::env::var("SP_SOLVER_TIMEOUT").ok().and_then(|v| v.parse().ok()).unwrap_or(90)), rx)
            .await
            .map_err(|_| "the solver took too long".to_string())?
            .map_err(|_| "the solver stopped".to_string())?
    }
}

// ---- Node.js as the solver's engine

/// Starts Node.js on the given script with one argument (a file the script writes its port
/// to) and returns at once; Node keeps running.
pub type NodeLauncher = Box<dyn Fn(&std::path::Path, &std::path::Path) -> Result<(), String> + Send + Sync>;

static NODE: OnceLock<NodeLauncher> = OnceLock::new();
/// The port of the running solver server.
static NODE_PORT: std::sync::Mutex<Option<u16>> = std::sync::Mutex::new(None);

const EJS_SERVER: &str = include_str!("../assets/ejs/solver.js");

/// Run the challenge solver on Node.js (started on first use) instead of the built-in engine.
pub fn use_node(launch: NodeLauncher) {
    let _ = NODE.set(launch);
}

fn node_port(dir: &std::path::Path) -> Result<u16, String> {
    let mut port = NODE_PORT.lock().map_err(|_| "solver lock poisoned".to_string())?;
    if let Some(p) = *port {
        return Ok(p);
    }
    let launch = NODE.get().ok_or("Node.js is not set up")?;
    std::fs::create_dir_all(dir).map_err(|e| format!("solver files: {e}"))?;
    for (name, text) in [("lib.min.js", EJS_LIB), ("core.min.js", EJS_CORE), ("solver.js", EJS_SERVER)] {
        std::fs::write(dir.join(name), text).map_err(|e| format!("solver files: {e}"))?;
    }
    let port_file = dir.join("port");
    let _ = std::fs::remove_file(&port_file);
    let started = std::time::Instant::now();
    launch(&dir.join("solver.js"), &port_file)?;
    while started.elapsed() < Duration::from_secs(30) {
        if let Some(p) = std::fs::read_to_string(&port_file).ok().and_then(|t| t.trim().parse::<u16>().ok()) {
            tracing::info!("Node.js solver ready in {:.1}s", started.elapsed().as_secs_f32());
            *port = Some(p);
            return Ok(p);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Err("Node.js did not start".into())
}

fn node_solve(player_js: &std::path::Path, preprocessed: &std::path::Path, n: &[String], sig: &[String]) -> Result<HashMap<String, String>, String> {
    use std::io::{BufRead, Write};
    let dir = player_js.parent().unwrap_or(std::path::Path::new(".")).join("node");
    let port = node_port(&dir)?;
    let mut socket = std::net::TcpStream::connect(("127.0.0.1", port)).map_err(|e| {
        // The server is gone: start it again next time.
        if let Ok(mut p) = NODE_PORT.lock() {
            *p = None;
        }
        format!("solver connection: {e}")
    })?;
    let _ = socket.set_read_timeout(Some(Duration::from_secs(60)));
    let job = json!({"player": player_js, "pre": preprocessed, "n": n, "sig": sig});
    socket.write_all(format!("{job}\n").as_bytes()).map_err(|e| format!("solver request: {e}"))?;
    let mut line = String::new();
    std::io::BufReader::new(socket).read_line(&mut line).map_err(|e| format!("solver answer: {e}"))?;
    let reply: Value = serde_json::from_str(&line).map_err(|e| format!("solver answer: {e}"))?;
    if let Some(err) = reply.get("err").and_then(Value::as_str) {
        return Err(format!("solver: {err}"));
    }
    Ok(reply
        .get("ok")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
        .collect())
}

fn solver_thread(jobs: std::sync::mpsc::Receiver<Job>) {
    let engine = (|| -> Result<(rquickjs::Runtime, rquickjs::Context), rquickjs::Error> {
        let rt = rquickjs::Runtime::new()?;
        rt.set_max_stack_size(48 << 20);
        let ctx = rquickjs::Context::full(&rt)?;
        ctx.with(|ctx| {
            ctx.eval::<(), _>(format!("{EJS_LIB}\nvar meriyah=lib.meriyah,astring=lib.astring;\n{EJS_CORE}"))
        })?;
        Ok((rt, ctx))
    })();
    let (_rt, ctx) = match engine {
        Ok(e) => e,
        Err(e) => {
            let msg = format!("could not start the JS engine: {e}");
            for job in jobs {
                let _ = job.reply.send(Err(msg.clone()));
            }
            return;
        }
    };
    // The player whose analysed form is loaded in the engine.
    let mut loaded: Option<String> = None;
    for job in jobs {
        let result = ctx.with(|ctx| -> Result<HashMap<String, String>, String> {
            let js = |e: rquickjs::Error| match e {
                rquickjs::Error::Exception => format!("solver exception: {:?}", ctx.catch()),
                other => other.to_string(),
            };
            let globals = ctx.globals();
            if loaded.as_deref() != Some(job.player_id.as_str()) {
                loaded = None;
                match std::fs::read_to_string(&job.preprocessed) {
                    Ok(pre) => {
                        globals.set("__pre", pre).map_err(js)?;
                        globals.set("__player", rquickjs::Undefined).map_err(js)?;
                    }
                    Err(_) => {
                        let player = std::fs::read_to_string(&job.player_js).map_err(|e| format!("player script: {e}"))?;
                        globals.set("__player", player).map_err(js)?;
                        globals.set("__pre", rquickjs::Undefined).map_err(js)?;
                    }
                }
            }
            let mut requests = Vec::new();
            if !job.n.is_empty() {
                requests.push(json!({"type": "n", "challenges": job.n}));
            }
            if !job.sig.is_empty() {
                requests.push(json!({"type": "sig", "challenges": job.sig}));
            }
            globals.set("__req", Value::Array(requests).to_string()).map_err(js)?;
            // The first call for a player analyses its script and keeps the result in the
            // engine; the analysed form comes back once so it can be stored on disk.
            let out: String = ctx
                .eval(
                    r#"(function () {
                        var requests = JSON.parse(__req), r, fresh = null;
                        if (typeof __pre === "string") {
                            r = jsc({ type: "preprocessed", preprocessed_player: __pre, requests: requests });
                        } else {
                            r = jsc({ type: "player", player: __player, requests: requests, output_preprocessed: true });
                            if (r.preprocessed_player) { globalThis.__pre = r.preprocessed_player; globalThis.__player = undefined; fresh = r.preprocessed_player; }
                        }
                        return JSON.stringify({ responses: r.responses, fresh: fresh });
                    })()"#,
                )
                .map_err(js)?;
            let parsed: Value = serde_json::from_str(&out).map_err(|e| format!("solver output: {e}"))?;
            if let Some(fresh) = parsed.get("fresh").and_then(Value::as_str) {
                let _ = crate::config::write_atomic(&job.preprocessed, fresh.as_bytes());
            }
            let mut answers = HashMap::new();
            for r in parsed.get("responses").and_then(Value::as_array).into_iter().flatten() {
                if r.get("type").and_then(Value::as_str) != Some("result") {
                    let why = r.get("error").and_then(Value::as_str).unwrap_or("unknown error");
                    return Err(format!("solver: {}", why.chars().take(300).collect::<String>()));
                }
                for (k, v) in r.get("data").and_then(Value::as_object).into_iter().flatten() {
                    if let Some(v) = v.as_str() {
                        answers.insert(k.clone(), v.to_string());
                    }
                }
            }
            Ok(answers)
        });
        if result.is_ok() {
            loaded = Some(job.player_id.clone());
        }
        let _ = job.reply.send(result);
    }
}

// ------------------------------------------------------------------ resolver

/// What one client answered: streams, or why not.
type Attempt = Result<ResolvedStreams, String>;

pub struct Ytp {
    http: reqwest::Client,
    dir: PathBuf,
    solver: Solver,
    player: tokio::sync::Mutex<Option<PlayerInfo>>,
}

impl Ytp {
    /// `cache_dir` holds the player script and its analysed form between runs.
    pub fn new(http: reqwest::Client, cache_dir: &std::path::Path) -> Self {
        Self {
            http,
            dir: cache_dir.join("direct-player"),
            solver: Solver::new(),
            player: tokio::sync::Mutex::new(None),
        }
    }

    async fn get_text(&self, url: &str, timeout_s: u64) -> Result<String, String> {
        let r = self
            .http
            .get(url)
            .timeout(Duration::from_secs(timeout_s))
            .send()
            .await
            .map_err(|e| format!("{url}: {e}"))?;
        if !r.status().is_success() {
            return Err(format!("{url}: HTTP {}", r.status().as_u16()));
        }
        r.text().await.map_err(|e| e.to_string())
    }

    /// The current player version: its id and signature timestamp. Cached for six hours.
    async fn player_info(&self) -> Result<PlayerInfo, String> {
        static PLAYER_ID: OnceLock<Regex> = OnceLock::new();
        static STS: OnceLock<Regex> = OnceLock::new();
        let mut slot = self.player.lock().await;
        if let Some(p) = slot.as_ref() {
            if now() - p.ts < PLAYER_TTL_S {
                return Ok(p.clone());
            }
        }
        let meta = self.dir.join("player.json");
        if let Some(p) = std::fs::read(&meta).ok().and_then(|b| serde_json::from_slice::<PlayerInfo>(&b).ok()) {
            if now() - p.ts < PLAYER_TTL_S && self.dir.join(format!("{}.tv.js", p.id)).is_file() {
                *slot = Some(p.clone());
                return Ok(p);
            }
        }
        let api = self.get_text(&format!("{ORIGIN}/iframe_api"), 15).await?;
        let id = re(&PLAYER_ID, r"player\\?/([0-9a-f]{8})\\?/")
            .captures(&api)
            .map(|c| c[1].to_string())
            .ok_or("YouTube's player version was not found")?;
        let js_path = self.dir.join(format!("{id}.tv.js"));
        let js = match std::fs::read_to_string(&js_path) {
            Ok(js) => js,
            Err(_) => {
                let js = self
                    .get_text(&format!("{ORIGIN}/s/player/{id}/tv-player-ias.vflset/tv-player-ias.js"), 40)
                    .await?;
                crate::config::write_atomic(&js_path, js.as_bytes()).map_err(|e| e.to_string())?;
                self.prune(&id);
                js
            }
        };
        let sts = re(&STS, r"(?:signatureTimestamp|sts)\s*:\s*(\d{5})")
            .captures(&js)
            .and_then(|c| c[1].parse().ok())
            .ok_or("the player script has no signature timestamp")?;
        let p = PlayerInfo { id, sts, ts: now() };
        if let Ok(b) = serde_json::to_vec(&p) {
            let _ = crate::config::write_atomic(&meta, &b);
        }
        *slot = Some(p.clone());
        Ok(p)
    }

    /// Delete the files of older player versions (each is several megabytes).
    fn prune(&self, keep: &str) {
        let Ok(dir) = std::fs::read_dir(&self.dir) else { return };
        for e in dir.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.contains(".tv.") && !name.starts_with(keep) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }

    /// Fetch the player version (and its script) ahead of the first video.
    pub async fn warm_up(&self) {
        if let Err(e) = self.player_info().await {
            tracing::info!("direct player warm-up: {e}");
        }
    }

    async fn player_request(&self, id: &str, client: &Client, cookie: Option<&str>, sts: Option<u32>, pyv: bool) -> Result<Value, String> {
        let mut ctx = json!({"clientName": client.name, "clientVersion": client.version, "hl": "en", "gl": "US"});
        for (k, v) in client.fields {
            ctx[*k] = json!(v);
        }
        let mut body = json!({
            "context": {"client": ctx},
            "videoId": id,
            "contentCheckOk": true,
            "racyCheckOk": true,
        });
        if let Some(sts) = sts {
            body["playbackContext"] = json!({
                "contentPlaybackContext": {"html5Preference": "HTML5_PREF_WANTS", "signatureTimestamp": sts},
            });
            if pyv {
                // Without this a signed-in session without Premium is given a pre-roll ad
                // and the content URLs answer 403 for the first seconds. On age-restricted
                // videos it yields preview-only URLs instead; see `attempt`.
                body["playbackContext"]["adPlaybackContext"] = json!({"pyv": true});
            }
        }
        let mut req = self
            .http
            .post(API)
            .timeout(Duration::from_secs(15))
            .header(reqwest::header::USER_AGENT, client.user_agent)
            .header(reqwest::header::ORIGIN, ORIGIN)
            .header("X-YouTube-Client-Name", client.id.to_string())
            .header("X-YouTube-Client-Version", client.version)
            .json(&body);
        if let (true, Some(c)) = (client.signed_in, cookie) {
            req = req
                .header(reqwest::header::COOKIE, c)
                .header(reqwest::header::AUTHORIZATION, sapisid_auth(c))
                .header("X-Origin", ORIGIN)
                .header("X-Goog-AuthUser", "0");
        }
        let resp = req.send().await.map_err(|e| format!("{e}"))?;
        if !resp.status().is_success() {
            return Err(format!("HTTP {}", resp.status().as_u16()));
        }
        resp.json().await.map_err(|e| format!("unexpected answer: {e}"))
    }

    /// True if the URLs serve only the first minute or so of the video (403 past that).
    /// Nothing in YouTube's answer marks this, so the last bytes of the smallest stream are
    /// asked for.
    async fn preview_only(&self, s: &ResolvedStreams) -> bool {
        let all = s.video.iter().chain(s.audio_aac.iter()).chain(s.audio_opus.iter());
        let Some((f, size)) = all.filter_map(|f| Some((f, f.size.filter(|n| *n > 1024)?))).min_by_key(|(_, n)| *n) else {
            return false;
        };
        let mut req = self
            .http
            .get(format!("{}&range={}-{}", f.url, size - 1024, size - 1))
            .timeout(Duration::from_secs(4));
        for (k, v) in &f.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        matches!(req.send().await, Ok(r) if r.status().as_u16() == 403)
    }

    async fn attempt(&self, id: &str, client: &Client, cookie: Option<&str>) -> Attempt {
        let streams = self.attempt_with(id, client, cookie, true).await?;
        // Age-restricted videos asked for with the ad flag come back as previews; without the
        // flag they are whole (and have no ad gate).
        if client.signed_in && self.preview_only(&streams).await {
            tracing::info!("direct streams for {id}: preview-only URLs, asking again without the ad flag");
            return self.attempt_with(id, client, cookie, false).await;
        }
        Ok(streams)
    }

    async fn attempt_with(&self, id: &str, client: &Client, cookie: Option<&str>, pyv: bool) -> Attempt {
        let info = if client.signed_in { Some(self.player_info().await?) } else { None };
        let player = self.player_request(id, client, cookie, info.as_ref().map(|p| p.sts), pyv).await?;
        let status = player.get("playabilityStatus");
        let state = status.and_then(|s| s.get("status")).and_then(Value::as_str).unwrap_or("?");
        let raw = raw_formats(player.get("streamingData").unwrap_or(&Value::Null));
        if state != "OK" || raw.is_empty() {
            let reason = status.and_then(|s| s.get("reason")).and_then(Value::as_str).unwrap_or("");
            let mut why = format!("{state} {reason}").trim().to_string();
            if state == "OK" {
                why = "no usable formats".into();
            }
            if client.signed_in {
                let logged_in = player
                    .pointer("/responseContext/serviceTrackingParams")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .flat_map(|x| x.get("params").and_then(Value::as_array).into_iter().flatten())
                    .any(|p| p.get("key").and_then(Value::as_str) == Some("logged_in") && p.get("value").and_then(Value::as_str) == Some("1"));
                if !logged_in {
                    why.push_str(" (the saved session was not accepted)");
                }
            }
            return Err(why);
        }
        let formats: Vec<(Value, String)> = match info {
            None => raw.into_iter().map(|f| (f.json, f.url)).collect(),
            Some(info) => {
                // Every distinct challenge is solved in one call.
                let mut n: Vec<String> = raw.iter().filter_map(|f| param(&f.url, "n")).collect();
                n.sort();
                n.dedup();
                let mut sig: Vec<String> = raw.iter().filter_map(|f| f.sig.as_ref().map(|(s, _)| s.clone())).collect();
                sig.sort();
                sig.dedup();
                let answers = self
                    .solver
                    .solve((
                        info.id.clone(),
                        self.dir.join(format!("{}.tv.js", info.id)),
                        self.dir.join(format!("{}.tv.pre.js", info.id)),
                        n,
                        sig,
                    ))
                    .await?;
                static N: OnceLock<Regex> = OnceLock::new();
                let n_re = re(&N, r"([?&])n=[^&]+");
                let mut done = Vec::with_capacity(raw.len());
                for f in raw {
                    let mut url = f.url;
                    if let Some(challenge) = param(&url, "n") {
                        let answer = answers.get(&challenge).ok_or("the n challenge was not solved")?;
                        url = n_re.replace(&url, format!("${{1}}n={}", encode(answer)).as_str()).into_owned();
                    }
                    if let Some((challenge, name)) = &f.sig {
                        let answer = answers.get(challenge).ok_or("the signature challenge was not solved")?;
                        url.push_str(&format!("&{name}={}", encode(answer)));
                    }
                    done.push((f.json, url));
                }
                done
            }
        };
        let streams = build(&player, formats, client);
        if streams.is_empty() {
            return Err("no playable streams".into());
        }
        Ok(streams)
    }

    /// Streams from the signed-in path alone (for testing that path and the solver).
    pub async fn resolve_signed_in(&self, id: &str, cookie: &str) -> Result<ResolvedStreams, String> {
        self.attempt(id, &TV, Some(cookie)).await
    }

    /// Streams for `id`: the public path first, then the signed-in one (`cookie` is the
    /// session's `Cookie` header) when YouTube refuses the public request.
    pub async fn resolve(&self, id: &str, cookie: Option<&str>) -> Result<ResolvedStreams, String> {
        self.resolve_with(id, cookie, false).await
    }

    /// The quick answer: as `resolve`, but without checking whether the URLs serve the whole
    /// video (some videos come back as a one-minute preview, from either client). Call `resolve`
    /// afterwards; if its `source` differs or ends in "-nopyv", use its streams instead.
    pub async fn resolve_fast(&self, id: &str, cookie: Option<&str>) -> Result<ResolvedStreams, String> {
        self.resolve_with(id, cookie, true).await
    }

    async fn resolve_with(&self, id: &str, cookie: Option<&str>, fast: bool) -> Result<ResolvedStreams, String> {
        let started = std::time::Instant::now();
        let outcome = if LIBRARY.get().is_some() {
            let (id, cookie) = (id.to_string(), cookie.map(str::to_string));
            tokio::task::spawn_blocking(move || {
                let resolve = LIBRARY.get().expect("checked above");
                resolve(&id, cookie.as_deref(), fast).and_then(|j| from_library(&j))
            })
            .await
            .unwrap_or_else(|e| Err(format!("the resolver stopped: {e}")))
        } else {
            match self.attempt(id, &IOS, None).await {
                Ok(s) => Ok(s),
                Err(pe) => match cookie {
                    Some(c) => self.attempt(id, &TV, Some(c)).await.map_err(|se| format!("public: {pe}; signed in: {se}")),
                    None => Err(format!("public: {pe}")),
                },
            }
        };
        match &outcome {
            Ok(s) => tracing::info!(
                "direct streams for {id} via {} in {:.1}s ({} video formats)",
                s.source,
                started.elapsed().as_secs_f32(),
                s.video.len()
            ),
            Err(e) => tracing::info!("direct streams for {id} failed in {:.1}s: {e}", started.elapsed().as_secs_f32()),
        }
        outcome
    }
}

/// The same stream on each cache node YouTube named for it (`mn=`), for when the first is
/// unreachable.
pub fn mirrors(url: &str) -> Vec<String> {
    static HOST: OnceLock<Regex> = OnceLock::new();
    let Some(c) = re(&HOST, r"^(https://rr\d+---)([\w-]+)(\.googlevideo\.com/.*)$").captures(url) else {
        return vec![url.to_string()];
    };
    let mut out = vec![url.to_string()];
    for (i, node) in param(url, "mn").unwrap_or_default().split(',').enumerate() {
        if !node.is_empty() && node != &c[2] {
            out.push(format!("{}{node}{}&fallback_count={}", &c[1], &c[3], i + 1));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_helpers() {
        assert_eq!(encode("a b/+~"), "a%20b%2F%2B~");
        assert_eq!(decode("a%20b%2F%2B+c%zz%4"), "a b/+ c%zz%4");
        assert_eq!(param("https://x/v?a=1&n=ab%3Dc&b=2", "n").as_deref(), Some("ab=c"));
        assert_eq!(param("https://x/v", "n"), None);
    }

    #[test]
    fn reads_plain_and_ciphered_formats() {
        let sd = json!({
            "formats": [{"itag": 18, "url": "https://a/x?n=1"}],
            "adaptiveFormats": [
                {"itag": 137, "signatureCipher": "s=AB%3DC&sp=sig&url=https%3A%2F%2Fb%2Fy%3Fn%3D2"},
                {"itag": 1, "url": "https://drm", "drmFamilies": ["WIDEVINE"]}
            ]
        });
        let raw = raw_formats(&sd);
        assert_eq!(raw.len(), 2);
        assert_eq!(raw[1].url, "https://b/y?n=2");
        assert_eq!(raw[1].sig, Some(("AB=C".into(), "sig".into())));
    }

    #[test]
    fn reads_the_library_answer() {
        let j = json!({
            "title": "T", "author": "A", "duration": 61, "client": "tv", "expires_in": 100,
            "formats": [
                {"itag": 401, "kind": "video", "ext": "mp4", "codecs": "av01.0.13M.08", "res": "2160p60", "height": 2160, "fps": 60, "bitrate": 9, "size": 10, "url": "https://v", "ua": "X"},
                {"itag": 315, "kind": "video", "ext": "webm", "codecs": "vp09.00.51.08", "res": "2160p60 HDR", "height": 2160, "url": "https://h", "ua": "X"},
                {"itag": 140, "kind": "audio", "ext": "mp4", "codecs": "mp4a.40.2", "res": "", "bitrate": 130, "size": 5, "drc": true, "url": "https://d", "ua": "X"},
                {"itag": 140, "kind": "audio", "ext": "mp4", "codecs": "mp4a.40.2", "res": "", "bitrate": 130, "size": 5, "url": "https://a", "ua": "X"},
                {"itag": 139, "kind": "audio", "ext": "mp4", "codecs": "mp4a.40.5", "res": "", "bitrate": 50, "size": 2, "url": "https://low", "ua": "X"}
            ]
        });
        let s = from_library(&j).unwrap();
        assert_eq!((s.title.as_str(), s.duration_s, s.source.as_str()), ("T", 61, "ytp-tv"));
        assert_eq!(s.video.len(), 1);
        assert_eq!(s.video[0].mime, "video/mp4; codecs=\"av01.0.13M.08\"");
        assert_eq!(s.video[0].headers, vec![("User-Agent".to_string(), "X".to_string())]);
        assert_eq!(s.audio_aac.unwrap().url, "https://a");
        assert!(from_library(&json!({"error": "nope", "tried": ["ios: LOGIN_REQUIRED"]})).unwrap_err().contains("LOGIN_REQUIRED"));
    }

    #[test]
    fn builds_a_stream_set() {
        let player = json!({
            "videoDetails": {"title": "T", "author": "A", "lengthSeconds": "61"},
            "streamingData": {"expiresInSeconds": "100"}
        });
        let f = |v: Value| (v, "https://u".to_string());
        let s = build(
            &player,
            vec![
                f(json!({"itag": 18, "mimeType": "video/mp4; codecs=\"avc1.42001E, mp4a.40.2\"", "height": 360})),
                f(json!({"itag": 137, "mimeType": "video/mp4; codecs=\"avc1.640028\"", "height": 1080, "fps": 30, "bitrate": 9,
                    "contentLength": "1234", "initRange": {"start": "0", "end": "740"}, "indexRange": {"start": "741", "end": "999"}})),
                f(json!({"itag": 400, "mimeType": "video/mp4; codecs=\"av01.0.12M.10\"", "height": 1440, "qualityLabel": "1440p HDR"})),
                f(json!({"itag": 140, "mimeType": "audio/mp4; codecs=\"mp4a.40.2\"", "bitrate": 130})),
                f(json!({"itag": 141, "mimeType": "audio/mp4; codecs=\"mp4a.40.2\"", "bitrate": 250, "isDrc": true})),
                f(json!({"itag": 251, "mimeType": "audio/webm; codecs=\"opus\"", "bitrate": 140})),
            ],
            &IOS,
        );
        assert_eq!((s.title.as_str(), s.duration_s, s.video.len()), ("T", 61, 1));
        let v = &s.video[0];
        assert_eq!((v.itag, v.codec.as_str(), v.size, v.init_range, v.index_range), (137, "h264", Some(1234), Some((0, 740)), Some((741, 999))));
        assert!(v.mime.contains("avc1.640028"));
        assert_eq!(s.audio_aac.unwrap().itag, 140);
        assert_eq!(s.audio_opus.unwrap().itag, 251);
        assert_eq!(s.source, "direct-ios");
    }

    #[test]
    fn mirror_hosts() {
        let m = mirrors("https://rr3---sn-aaa.googlevideo.com/videoplayback?x=1&mn=sn-aaa%2Csn-bbb");
        assert_eq!(m.len(), 2);
        assert!(m[1].starts_with("https://rr3---sn-bbb.googlevideo.com/videoplayback?") && m[1].ends_with("&fallback_count=2"));
        assert_eq!(mirrors("https://example.com/v").len(), 1);
    }

    /// The bundled solver loads and runs on the embedded engine.
    #[test]
    fn solver_engine_starts() {
        let rt = rquickjs::Runtime::new().unwrap();
        rt.set_max_stack_size(8 << 20);
        let ctx = rquickjs::Context::full(&rt).unwrap();
        let kind: String = ctx
            .with(|ctx| {
                ctx.eval::<(), _>(format!("{EJS_LIB}\nvar meriyah=lib.meriyah,astring=lib.astring;\n{EJS_CORE}"))?;
                ctx.eval("typeof jsc + ' ' + typeof meriyah.parse")
            })
            .unwrap();
        assert_eq!(kind, "function function");
    }
}
