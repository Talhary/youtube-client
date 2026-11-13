//! The yt-parser library (`ytp.dll` / `libytp.so`, C ABI, see third_party/yt-parser-src/lib.go):
//! the same stream resolver the Android app uses, so playback starts in about a second.

use std::ffi::{CStr, CString, c_char, c_int};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;
use serde_json::Value;

type Init = unsafe extern "C" fn(*const c_char, *const c_char) -> *mut c_char;
type Resolve = unsafe extern "C" fn(*const c_char, c_int) -> *mut c_char;
type Free = unsafe extern "C" fn(*mut c_char);

pub struct YtParser {
    _lib: libloading::Library,
    init: Init,
    resolve: Resolve,
    free: Free,
    cache: CString,
    /// The cookies the library was last given (`None` before the first call).
    cookies: Mutex<Option<String>>,
    /// The public path was refused last time: ask the signed-in one first.
    cookie_first: AtomicBool,
}

#[cfg(windows)]
const NAME: &str = "ytp.dll";
#[cfg(not(windows))]
const NAME: &str = "libytp.so";

impl YtParser {
    /// Load the library from beside the executable (or the working folder).
    pub fn load(cache_dir: &Path) -> Result<Self, String> {
        let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
        let candidates = exe_dir.into_iter().map(|d| d.join(NAME)).chain([std::path::PathBuf::from(NAME)]);
        let mut last = String::from("not found");
        for path in candidates {
            if !path.exists() {
                continue;
            }
            // SAFETY: the library's exports have the signatures in ytp.h.
            match unsafe { libloading::Library::new(&path) } {
                Ok(lib) => unsafe {
                    let init = *lib.get::<Init>(b"ytp_init\0").map_err(|e| e.to_string())?;
                    let resolve = *lib.get::<Resolve>(b"ytp_resolve\0").map_err(|e| e.to_string())?;
                    let free = *lib.get::<Free>(b"ytp_free\0").map_err(|e| e.to_string())?;
                    return Ok(Self {
                        _lib: lib,
                        init,
                        resolve,
                        free,
                        cache: CString::new(cache_dir.join("ytp").to_string_lossy().into_owned()).map_err(|e| e.to_string())?,
                        cookies: Mutex::new(None),
                        cookie_first: AtomicBool::new(false),
                    });
                },
                Err(e) => last = e.to_string(),
            }
        }
        Err(format!("{NAME}: {last}"))
    }

    fn take(&self, p: *mut c_char) -> Result<Value, String> {
        if p.is_null() {
            return Err("the resolver gave no answer".into());
        }
        // SAFETY: the library returns a NUL-terminated string that `ytp_free` releases.
        let text = unsafe { CStr::from_ptr(p).to_string_lossy().into_owned() };
        unsafe { (self.free)(p) };
        serde_json::from_str(&text).map_err(|e| format!("resolver answer: {e}"))
    }

    pub fn resolve(&self, id: &str, cookie: Option<&str>, fast: bool) -> Result<Value, String> {
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
        let first = signed_in && self.cookie_first.load(Ordering::Relaxed);
        // Flags: 1 = signed-in path first, 2 = skip the preview check.
        let flags = first as c_int | (fast as c_int) << 1;
        let answer = self.take(unsafe { (self.resolve)(id.as_ptr(), flags) })?;
        match answer.get("client").and_then(Value::as_str) {
            Some("tv") => self.cookie_first.store(true, Ordering::Relaxed),
            Some(_) => self.cookie_first.store(false, Ordering::Relaxed),
            None => {}
        }
        Ok(answer)
    }
}
