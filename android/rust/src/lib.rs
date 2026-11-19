//! JNI bridge between the Android app and the StreamPipe core.
//!
//! The app's UI (a web page) sends JSON commands through `Native.send`; core events come
//! back as JSON through the static Java method `Native.onEvent`. All network and disk work
//! stays on the core's own threads.

use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use jni::objects::{GlobalRef, JClass, JString, JValue};
use jni::sys::jstring;
use jni::{JNIEnv, JavaVM};
use parking_lot::Mutex;
use serde::Deserialize;
use serde_json::{Value, json};
use streampipe_core::config::{Paths, Settings};
use streampipe_core::model::Cursor;
use streampipe_core::mux::Muxer;
use streampipe_core::streams::HwCaps;
use streampipe_core::{Command, Core, CoreHandle, CoreOptions, Event};

struct Shell {
    handle: CoreHandle,
    /// Owns the core's runtime; never dropped while the process lives.
    _core: Mutex<Core>,
}

static SHELL: OnceLock<Shell> = OnceLock::new();

/// Joins a downloaded picture stream and sound stream into one file with Android's own
/// muxer (`Native.mux` in Java: MediaExtractor + MediaMuxer, no re-encoding).
struct JavaMuxer {
    vm: jni::JavaVM,
    class: jni::objects::GlobalRef,
}

impl JavaMuxer {
    /// `video` empty = sound only.
    fn call(&self, video: &str, audio: &std::path::Path, out: &std::path::Path) -> Result<(), String> {
        let mut env = self.vm.attach_current_thread().map_err(|e| e.to_string())?;
        let v = env.new_string(video).map_err(|e| e.to_string())?;
        let a = env.new_string(audio.to_string_lossy()).map_err(|e| e.to_string())?;
        let o = env.new_string(out.to_string_lossy()).map_err(|e| e.to_string())?;
        let class: &JClass = self.class.as_obj().into();
        let answer = env
            .call_static_method(
                class,
                "mux",
                "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)Ljava/lang/String;",
                &[JValue::Object(&v), JValue::Object(&a), JValue::Object(&o)],
            )
            .and_then(|r| r.l());
        match answer {
            Ok(obj) if obj.is_null() => Ok(()),
            Ok(obj) => Err(env.get_string(&JString::from(obj)).map(String::from).unwrap_or_else(|_| "muxing failed".into())),
            Err(e) => {
                let _ = env.exception_clear();
                Err(format!("muxing failed: {e}"))
            }
        }
    }
}

impl Muxer for JavaMuxer {
    fn mux(&self, video: &std::path::Path, audio: &std::path::Path, out: &std::path::Path) -> Result<(), String> {
        self.call(&video.to_string_lossy(), audio, out)
    }
    fn remux_audio(&self, audio: &std::path::Path, out: &std::path::Path) -> Result<(), String> {
        self.call("", audio, out)
    }
    fn remuxes_m4a(&self) -> bool {
        true
    }
}

/// A command from the UI. `req` ties the answer to the request.
#[derive(Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum UiCmd {
    Home { req: u64 },
    Search {
        req: u64,
        query: String,
        #[serde(default)]
        filter: streampipe_core::api::SearchOptions,
    },
    Suggest { req: u64, query: String },
    ChannelTab { req: u64, id: String, tab: String },
    ChannelAbout { req: u64, id: String },
    Explore { req: u64, category: String },
    Extras { req: u64, id: String },
    Captions {
        req: u64,
        url: String,
        #[serde(default)]
        agent: Option<String>,
    },
    LiveChat { req: u64, token: String },
    Notifications { req: u64 },
    SetBell { req: u64, params: String },
    PlaylistOptions { req: u64, id: String },
    EditPlaylist { req: u64, playlist: String, video: String, add: bool },
    CreatePlaylist {
        req: u64,
        title: String,
        privacy: String,
        #[serde(default)]
        video: Option<String>,
    },
    HistoryActions { req: u64 },
    Shorts {
        req: u64,
        op: String,
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        params: Option<String>,
        #[serde(default)]
        token: Option<String>,
    },
    Feedback { req: u64, token: String },
    NotInterested { req: u64, id: String },
    PrefetchStreams { ids: Vec<String> },
    MusicBrowse {
        req: u64,
        id: String,
        #[serde(default)]
        params: Option<String>,
    },
    MusicSearch {
        req: u64,
        query: String,
        #[serde(default)]
        tab: String,
    },
    MusicSuggest { req: u64, query: String },
    MusicMore { req: u64, cursor: u64 },
    MusicQueue {
        req: u64,
        #[serde(default)]
        video: Option<String>,
        #[serde(default)]
        list: Option<String>,
    },
    MusicLyrics { req: u64, id: String },
    MusicSave { req: u64, list: String, on: bool },
    More { req: u64, cursor: u64 },
    DropCursor { cursor: u64 },
    Channel { req: u64, id: String },
    ChannelPlaylists { req: u64, id: String },
    Playlist { req: u64, id: String },
    Video { req: u64, id: String },
    Comments { req: u64, id: String },
    MoreComments { req: u64, cursor: u64 },
    CommentReplies { req: u64, token: String },
    PostComment {
        req: u64,
        params: String,
        text: String,
        #[serde(default)]
        reply: bool,
    },
    Streams { req: u64, id: String },
    Watched {
        id: String,
        seconds: u32,
        #[serde(default)]
        total: u32,
    },
    Rating { req: u64, id: String },
    Rate { req: u64, id: String, rating: String },
    Subscription { req: u64, id: String },
    Subscribe { req: u64, id: String, on: bool },
    /// `height` 0 = sound only (m4a); otherwise the tallest picture up to that height.
    Download { id: String, title: String, height: u32 },
    /// Every video of a playlist; `height` as in `Download`.
    DownloadPlaylist {
        id: String,
        height: u32,
        #[serde(default)]
        name: Option<String>,
    },
    PauseDownload { id: u64 },
    ResumeDownload { id: u64 },
    CancelDownload { id: u64 },
    ClearDownloads,
    Feed { req: u64 },
    Subscriptions { req: u64 },
    History { req: u64 },
    SavedPlaylists { req: u64 },
    /// Cookies of the signed-in web session, as Netscape cookies.txt text.
    ImportCookies { text: String },
    Logout,
    UpdateSettings { settings: Settings },
}

fn to_command(c: UiCmd) -> Command {
    match c {
        UiCmd::Home { req } => Command::Home { req },
        UiCmd::Search { req, query, filter } => Command::Search { req, query, filter },
        UiCmd::ChannelTab { req, id, tab } => Command::ChannelTab { req, id, tab },
        UiCmd::ChannelAbout { req, id } => Command::ChannelAbout { req, id },
        UiCmd::Explore { req, category } => Command::Explore { req, category },
        UiCmd::Extras { req, id } => Command::Extras { req, id },
        UiCmd::Captions { req, url, agent } => Command::Captions { req, url, agent },
        UiCmd::LiveChat { req, token } => Command::LiveChat { req, token },
        UiCmd::Notifications { req } => Command::Notifications { req },
        UiCmd::SetBell { req, params } => Command::SetBell { req, params },
        UiCmd::PlaylistOptions { req, id } => Command::PlaylistOptions { req, id },
        UiCmd::EditPlaylist { req, playlist, video, add } => Command::EditPlaylist { req, playlist, video, add },
        UiCmd::CreatePlaylist { req, title, privacy, video } => Command::CreatePlaylist { req, title, privacy, video },
        UiCmd::HistoryActions { req } => Command::HistoryActions { req },
        UiCmd::Shorts { req, op, id, params, token } => Command::Shorts { req, op, id, params, token },
        UiCmd::Feedback { req, token } => Command::Feedback { req, token },
        UiCmd::NotInterested { req, id } => Command::NotInterested { req, id },
        UiCmd::PrefetchStreams { ids } => Command::PrefetchStreams { ids },
        UiCmd::MusicBrowse { req, id, params } => Command::MusicBrowse { req, id, params },
        UiCmd::MusicSearch { req, query, tab } => Command::MusicSearch { req, query, tab },
        UiCmd::MusicSuggest { req, query } => Command::MusicSuggest { req, query },
        UiCmd::MusicMore { req, cursor } => Command::MusicMore { req, cursor: Cursor(cursor) },
        UiCmd::MusicQueue { req, video, list } => Command::MusicQueue { req, video, list },
        UiCmd::MusicLyrics { req, id } => Command::MusicLyrics { req, id },
        UiCmd::MusicSave { req, list, on } => Command::MusicSave { req, list, on },
        UiCmd::Suggest { req, query } => Command::Suggest { req, query },
        UiCmd::More { req, cursor } => Command::More { req, cursor: Cursor(cursor) },
        UiCmd::DropCursor { cursor } => Command::DropCursor(Cursor(cursor)),
        UiCmd::Channel { req, id } => Command::Channel { req, id },
        UiCmd::ChannelPlaylists { req, id } => Command::ChannelPlaylists { req, id },
        UiCmd::Playlist { req, id } => Command::Playlist { req, id },
        UiCmd::Video { req, id } => Command::Video { req, id },
        UiCmd::Comments { req, id } => Command::Comments { req, id },
        UiCmd::MoreComments { req, cursor } => Command::MoreComments { req, cursor: Cursor(cursor) },
        UiCmd::CommentReplies { req, token } => Command::CommentReplies { req, token },
        UiCmd::PostComment { req, params, text, reply } => Command::PostComment { req, params, text, reply },
        UiCmd::Streams { req, id } => Command::Streams { req, id },
        UiCmd::Watched { id, seconds, total } => Command::Watched { id, seconds, total },
        UiCmd::Rating { req, id } => Command::Rating { req, id },
        UiCmd::Rate { req, id, rating } => Command::Rate { req, id, rating },
        UiCmd::Subscription { req, id } => Command::Subscription { req, id },
        UiCmd::Subscribe { req, id, on } => Command::Subscribe { req, id, on },
        UiCmd::Download { id, title, height } => Command::Download(streampipe_core::download::DownloadRequest {
            video_id: id,
            title,
            kind: if height == 0 {
                streampipe_core::download::DownloadKind::Audio(streampipe_core::config::AudioOut::M4a)
            } else {
                streampipe_core::download::DownloadKind::Video { max_height: Some(height) }
            },
            group: None,
        }),
        UiCmd::DownloadPlaylist { id, height, name } => Command::DownloadPlaylist {
            id,
            group: name.filter(|n| !n.trim().is_empty()),
            kind: if height == 0 {
                streampipe_core::download::DownloadKind::Audio(streampipe_core::config::AudioOut::M4a)
            } else {
                streampipe_core::download::DownloadKind::Video { max_height: Some(height) }
            },
        },
        UiCmd::PauseDownload { id } => Command::PauseDownload(id),
        UiCmd::ResumeDownload { id } => Command::ResumeDownload(id),
        UiCmd::CancelDownload { id } => Command::CancelDownload(id),
        UiCmd::ClearDownloads => Command::ClearDownloads,
        UiCmd::Feed { req } => Command::Feed { req },
        UiCmd::Subscriptions { req } => Command::Subscriptions { req },
        UiCmd::History { req } => Command::History { req },
        UiCmd::SavedPlaylists { req } => Command::SavedPlaylists { req },
        UiCmd::ImportCookies { text } => Command::ImportCookies(text),
        UiCmd::Logout => Command::Logout,
        UiCmd::UpdateSettings { settings } => Command::UpdateSettings(settings),
    }
}

fn result<T: serde::Serialize>(ev: &str, req: u64, r: &Result<T, String>) -> Value {
    match r {
        Ok(v) => json!({"ev": ev, "req": req, "ok": v}),
        Err(e) => json!({"ev": ev, "req": req, "err": e}),
    }
}

/// Direct streams as the player page wants them: MP4 video formats and one AAC audio
/// format, each with every address it can be fetched from.
fn streams_json(s: &streampipe_core::streams::ResolvedStreams) -> Value {
    let one = |f: &streampipe_core::streams::StreamChoice, kind: &str| {
        let agent = f.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case("user-agent")).map(|(_, v)| v.as_str());
        json!({
            "kind": kind,
            "itag": f.itag,
            "mime": f.mime,
            "codec": f.codec,
            "height": f.height,
            "fps": f.fps,
            "bitrate": f.bitrate,
            "size": f.size,
            "init": f.init_range,
            "index": f.index_range,
            "urls": streampipe_core::ytp::mirrors(&f.url),
            "agent": agent,
        })
    };
    json!({
        "duration": s.duration_s,
        "source": s.source,
        "video": s.video.iter().map(|f| one(f, "video")).collect::<Vec<_>>(),
        "audio": s.audio_aac.as_ref().map(|f| one(f, "audio")),
        // The loudness-levelled sound ("stable volume"), and the languages of dubbed videos.
        "audio_drc": s.audio_drc.as_ref().map(|f| one(f, "audio")),
        "audio_tracks": s.audio_tracks.iter().map(|t| {
            let mut o = one(&t.stream, "audio");
            o["name"] = json!(t.name);
            o["original"] = json!(t.original);
            o
        }).collect::<Vec<_>>(),
        // Live streams play from their HLS list.
        "hls": s.hls,
        "live": s.hls.is_some() && s.duration_s == 0,
    })
}

/// The JSON the UI receives for a core event; `None` for events this shell does not use.
fn to_json(e: &Event) -> Option<Value> {
    Some(match e {
        Event::Ready(s) => json!({"ev": "ready", "status": s}),
        Event::Page { req, result: r } => result("page", *req, r),
        Event::Suggestions { req, items } => json!({"ev": "suggestions", "req": req, "items": items}),
        Event::Channel { req, result: r } => result("channel", *req, r),
        Event::Playlist { req, result: r } => result("playlist", *req, r),
        Event::Video { req, result: r } => result("video", *req, r),
        Event::Comments { req, result: r } => result("comments", *req, r),
        Event::Streams { req, result: r } => result("streams", *req, &r.as_ref().map(streams_json).map_err(String::clone)),
        Event::Action { req, result: r } => result("action", *req, r),
        Event::StreamsUpdate { id, streams } => json!({"ev": "streams_update", "id": id, "streams": streams_json(streams)}),
        Event::Auth(a) => json!({"ev": "auth", "auth": a}),
        Event::Settings(s) => json!({"ev": "settings", "settings": s}),
        Event::Remote(r) => json!({"ev": "remote", "remote": r}),
        Event::UpdateAvailable(u) => json!({"ev": "update", "update": u}),
        Event::Toast(t) => json!({"ev": "toast", "text": t}),
        Event::Playback { .. }
        | Event::PlaybackUpgrade { .. }
        | Event::Thumbnail { .. }
        | Event::ThumbnailFailed { .. } => return None,
        Event::Download(d) => json!({"ev": "download", "download": d}),
    })
}

/// A key for the encrypted session file, kept in the app's private storage (which Android
/// already isolates per app).
fn storage_key(dir: &std::path::Path) -> [u8; 32] {
    let file = dir.join("key.bin");
    if let Ok(b) = std::fs::read(&file) {
        if let Ok(k) = <[u8; 32]>::try_from(b.as_slice()) {
            return k;
        }
    }
    let k = streampipe_core::secure::random_key();
    let _ = std::fs::create_dir_all(dir);
    let _ = std::fs::write(&file, k);
    k
}

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(prio: libc::c_int, tag: *const libc::c_char, text: *const libc::c_char) -> libc::c_int;
}

/// The core logs to stderr, which Android discards: send stderr to the system log instead
/// (tag `StreamPipeCore`), one log entry per line.
fn stderr_to_logcat() {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: plain POSIX calls on descriptors this function owns.
    unsafe {
        if libc::pipe(fds.as_mut_ptr()) != 0 || libc::dup2(fds[1], 2) < 0 {
            return;
        }
        libc::close(fds[1]);
    }
    let read_fd = fds[0];
    let _ = std::thread::Builder::new().name("streampipe-log".into()).spawn(move || {
        use std::io::BufRead;
        use std::os::fd::FromRawFd;
        // SAFETY: `read_fd` is the read end of the pipe created above, owned by this thread.
        let file = unsafe { std::fs::File::from_raw_fd(read_fd) };
        for line in std::io::BufReader::new(file).lines().map_while(Result::ok) {
            // Drop the colour codes the log formatter adds for terminals.
            let mut clean = String::with_capacity(line.len());
            let mut chars = line.chars();
            while let Some(c) = chars.next() {
                if c == '\u{1b}' {
                    for n in chars.by_ref() {
                        if n.is_ascii_alphabetic() {
                            break;
                        }
                    }
                } else {
                    clean.push(c);
                }
            }
            if let Ok(text) = std::ffi::CString::new(clean) {
                // SAFETY: both strings are valid and NUL-terminated for the call.
                unsafe { __android_log_write(4, c"StreamPipeCore".as_ptr(), text.as_ptr()) };
            }
        }
    });
}

fn start(vm: JavaVM, class: GlobalRef, files: PathBuf, cache: PathBuf, downloads: PathBuf, version: String) -> Result<(), String> {
    if SHELL.get().is_some() {
        return Ok(());
    }
    stderr_to_logcat();
    let paths = Paths::rooted(&files, &cache, &downloads);
    paths.ensure().map_err(|e| e.to_string())?;
    let key = storage_key(&paths.data);
    let mut core = Core::start(CoreOptions {
        paths,
        key,
        botguard: None,
        muxer: Arc::new(JavaMuxer {
            // SAFETY: the pointer comes from a live JavaVM, which outlives the process's threads.
            vm: unsafe { jni::JavaVM::from_raw(vm.get_java_vm_pointer()) }.map_err(|e| e.to_string())?,
            class: class.clone(),
        }),
        ffmpeg: None,
        caps: HwCaps::default(),
        // The event thread below blocks on the channel, so nothing needs waking.
        waker: Arc::new(|| {}),
        version,
        github_updates: false,
    })?;
    let events = core.take_events();
    let handle = core.handle.clone();
    std::thread::Builder::new()
        .name("streampipe-events".into())
        .spawn(move || {
            let Ok(mut env) = vm.attach_current_thread_permanently() else { return };
            while let Ok(e) = events.recv() {
                let Some(v) = to_json(&e) else { continue };
                let Ok(s) = env.new_string(v.to_string()) else { continue };
                let class: &JClass = class.as_obj().into();
                let _ = env.call_static_method(class, "onEvent", "(Ljava/lang/String;)V", &[JValue::Object(&s)]);
                if env.exception_check().unwrap_or(false) {
                    let _ = env.exception_clear();
                }
                let _ = env.delete_local_ref(s);
            }
        })
        .map_err(|e| e.to_string())?;
    let _ = SHELL.set(Shell { handle, _core: Mutex::new(core) });
    Ok(())
}

fn string(env: &mut JNIEnv, s: &JString) -> String {
    env.get_string(s).map(|s| s.into()).unwrap_or_default()
}

/// `Native.start(filesDir, cacheDir, downloadsDir, version)`: returns null, or an error text.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_streampipe_app_Native_start<'l>(
    mut env: JNIEnv<'l>,
    class: JClass<'l>,
    files: JString<'l>,
    cache: JString<'l>,
    downloads: JString<'l>,
    version: JString<'l>,
) -> jstring {
    let r = (|| {
        let vm = env.get_java_vm().map_err(|e| e.to_string())?;
        let class = env.new_global_ref(&class).map_err(|e| e.to_string())?;
        let (f, c, d, v) = (
            string(&mut env, &files),
            string(&mut env, &cache),
            string(&mut env, &downloads),
            string(&mut env, &version),
        );
        match YtParser::load(std::path::Path::new(&c)) {
            Ok(lib) => streampipe_core::ytp::use_library(Box::new(move |id, cookie, fast| lib.resolve(id, cookie, fast))),
            Err(e) => eprintln!("yt-parser library: {e}; using the built-in resolver"),
        }
        start(vm, class, f.into(), c.into(), d.into(), v)
    })();
    match r {
        Ok(()) => std::ptr::null_mut(),
        Err(e) => env.new_string(e).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut()),
    }
}

/// The yt-parser library (`libytp.so`, C ABI): resolves a video to stream URLs.
struct YtParser {
    init: unsafe extern "C" fn(*const libc::c_char, *const libc::c_char) -> *mut libc::c_char,
    resolve: unsafe extern "C" fn(*const libc::c_char, libc::c_int) -> *mut libc::c_char,
    free: unsafe extern "C" fn(*mut libc::c_char),
    cache: std::ffi::CString,
    /// The cookies the library was last given (`None` before the first call).
    cookies: parking_lot::Mutex<Option<String>>,
    /// The public path was refused last time: ask the signed-in one first.
    cookie_first: std::sync::atomic::AtomicBool,
    /// The signed-in mobile-web client answered with nothing playable: leave it out (flag 4).
    no_mweb: std::sync::atomic::AtomicBool,
}

impl YtParser {
    fn load(cache_dir: &std::path::Path) -> Result<Self, String> {
        use std::ffi::CString;
        let name = CString::new("libytp.so").unwrap();
        // SAFETY: plain dynamic loading; the symbols have the signatures in libytp.h.
        unsafe {
            let handle = libc::dlopen(name.as_ptr(), libc::RTLD_NOW);
            if handle.is_null() {
                return Err("libytp.so did not load".into());
            }
            let sym = |name: &str| {
                let c = CString::new(name).unwrap();
                let p = libc::dlsym(handle, c.as_ptr());
                if p.is_null() { Err(format!("libytp.so has no {name}")) } else { Ok(p) }
            };
            Ok(Self {
                init: std::mem::transmute::<*mut libc::c_void, _>(sym("ytp_init")?),
                resolve: std::mem::transmute::<*mut libc::c_void, _>(sym("ytp_resolve")?),
                free: std::mem::transmute::<*mut libc::c_void, _>(sym("ytp_free")?),
                cache: CString::new(cache_dir.join("ytp").to_string_lossy().into_owned()).map_err(|e| e.to_string())?,
                cookies: parking_lot::Mutex::new(None),
                cookie_first: std::sync::atomic::AtomicBool::new(false),
                no_mweb: std::sync::atomic::AtomicBool::new(false),
            })
        }
    }

    /// Take the library's answer (JSON text it allocated) and hand the memory back.
    fn take(&self, p: *mut libc::c_char) -> Result<Value, String> {
        if p.is_null() {
            return Err("the resolver gave no answer".into());
        }
        // SAFETY: the library returns a NUL-terminated string that `ytp_free` releases.
        let text = unsafe { std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned() };
        unsafe { (self.free)(p) };
        serde_json::from_str(&text).map_err(|e| format!("resolver answer: {e}"))
    }

    fn resolve(&self, id: &str, cookie: Option<&str>, fast: bool) -> Result<Value, String> {
        use std::ffi::CString;
        let wanted = cookie.unwrap_or("").to_string();
        let signed_in = !wanted.is_empty();
        {
            let mut current = self.cookies.lock();
            if current.as_deref() != Some(wanted.as_str()) {
                let c = CString::new(wanted.clone()).map_err(|e| e.to_string())?;
                // SAFETY: both arguments are valid C strings for the duration of the call.
                let answer = self.take(unsafe { (self.init)(self.cache.as_ptr(), c.as_ptr()) })?;
                if let Some(e) = answer.get("error").and_then(Value::as_str) {
                    return Err(format!("resolver start: {e}"));
                }
                *current = Some(wanted);
            }
        }
        let id = CString::new(id).map_err(|e| e.to_string())?;
        // Public first, then the signed-in path; the other way round once the public one
        // has been refused (a connection YouTube challenges stays that way for a while).
        use std::sync::atomic::Ordering;
        let first = signed_in && self.cookie_first.load(Ordering::Relaxed);
        // Flags: 1 = signed-in path first, 2 = skip the preview check, 4 = no mobile-web client.
        let flags = first as libc::c_int | (fast as libc::c_int) << 1 | (self.no_mweb.load(Ordering::Relaxed) as libc::c_int) << 2;
        let mut answer = self.take(unsafe { (self.resolve)(id.as_ptr(), flags) })?;
        // Some sessions get formats from the mobile-web client that cannot be played, while
        // tv still serves them: ask again without it, and leave it out from now on.
        if answer.get("client").and_then(Value::as_str) == Some("mweb") && streampipe_core::ytp::from_library(&answer).is_err() {
            eprintln!("yt-parser: mobile-web answer for this session is not playable; using tv from now on");
            self.no_mweb.store(true, Ordering::Relaxed);
            answer = self.take(unsafe { (self.resolve)(id.as_ptr(), flags | 4) })?;
        }
        match answer.get("client").and_then(Value::as_str) {
            Some("tv") => self.cookie_first.store(true, Ordering::Relaxed),
            Some(_) => self.cookie_first.store(false, Ordering::Relaxed),
            None => {}
        }
        Ok(answer)
    }
}

/// `Native.send(json)`: one UI command. Returns null, or why the command was not understood.
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_streampipe_app_Native_send<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    msg: JString<'l>,
) -> jstring {
    let text = string(&mut env, &msg);
    let err = match (SHELL.get(), serde_json::from_str::<UiCmd>(&text)) {
        (None, _) => Some("core not started".to_string()),
        (Some(_), Err(e)) => Some(e.to_string()),
        (Some(shell), Ok(cmd)) => {
            shell.handle.send(to_command(cmd));
            None
        }
    };
    match err {
        None => std::ptr::null_mut(),
        Some(e) => env.new_string(e).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut()),
    }
}

/// `Native.rulesFile(cacheDir)`: where the core keeps the ad-blocking rules (JSON).
#[unsafe(no_mangle)]
pub extern "system" fn Java_dev_streampipe_app_Native_rulesFile<'l>(
    mut env: JNIEnv<'l>,
    _class: JClass<'l>,
    cache: JString<'l>,
) -> jstring {
    let dir: PathBuf = string(&mut env, &cache).into();
    let p = streampipe_core::adblock::rules_file(&dir);
    env.new_string(p.to_string_lossy()).map(|s| s.into_raw()).unwrap_or(std::ptr::null_mut())
}
