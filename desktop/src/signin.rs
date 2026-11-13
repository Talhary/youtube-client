//! "Sign in with Google" in a browser window.
//!
//! The app relaunches itself with [`HELPER_ARG`]. That process opens a small native web view
//! (WebView2 on Windows, WebKitGTK on Linux) on Google's normal YouTube sign-in page. Once
//! YouTube's login cookies exist it prints them in Netscape cookies.txt format on stdout and
//! exits. The parent imports them like a cookies.txt file. The web view's profile lives in its
//! own folder, which the parent deletes afterwards so the session is not reused there
//! (YouTube rotates cookies for sessions that stay active in a browser).

use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop};
use tao::window::WindowBuilder;
use wry::{WebContext, WebViewBuilder};

pub const HELPER_ARG: &str = "--browser-sign-in";

const LOGIN_URL: &str = "https://accounts.google.com/ServiceLogin?service=youtube&uilel=3&passive=true&continue=https%3A%2F%2Fwww.youtube.com%2Fsignin%3Faction_handle_signin%3Dtrue%26app%3Ddesktop%26next%3D%252F";

/// Exit codes of the helper process.
pub const EXIT_CANCELLED: i32 = 2;
pub const EXIT_FAILED: i32 = 3;

/// Entry point of the helper process. Never returns.
pub fn run_helper(profile_dir: &Path) -> ! {
    let event_loop = EventLoop::new();
    let window = match WindowBuilder::new()
        .with_title("Sign in to YouTube · StreamPipe")
        .with_inner_size(tao::dpi::LogicalSize::new(500.0, 720.0))
        .build(&event_loop)
    {
        Ok(w) => w,
        Err(e) => {
            eprintln!("could not open the sign-in window: {e}");
            std::process::exit(EXIT_FAILED);
        }
    };
    let mut ctx = WebContext::new(Some(profile_dir.to_path_buf()));
    let webview = match WebViewBuilder::new_with_web_context(&mut ctx)
        .with_url(LOGIN_URL)
        .build(&window)
    {
        Ok(w) => w,
        Err(e) => {
            eprintln!("could not start the web view: {e}");
            std::process::exit(EXIT_FAILED);
        }
    };

    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(800));
        match event {
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                let on_youtube = webview
                    .url()
                    .map(|u| u.starts_with("https://www.youtube.com/") || u.starts_with("https://m.youtube.com/"))
                    .unwrap_or(false);
                if !on_youtube {
                    return;
                }
                let Ok(cookies) = webview.cookies_for_url("https://www.youtube.com/") else {
                    return;
                };
                let has = |n: &str| cookies.iter().any(|c| c.name() == n && !c.value().is_empty());
                let signed_in = (has("SAPISID") || has("__Secure-3PAPISID")) && has("LOGIN_INFO");
                if !signed_in {
                    return;
                }
                let txt = to_netscape(&cookies);
                let mut out = std::io::stdout().lock();
                let ok = out.write_all(txt.as_bytes()).and_then(|_| out.flush()).is_ok();
                std::process::exit(if ok { 0 } else { EXIT_FAILED });
            }
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => std::process::exit(EXIT_CANCELLED),
            _ => {}
        }
    })
}

fn to_netscape(cookies: &[wry::cookie::Cookie<'static>]) -> String {
    let mut s = String::from("# Netscape HTTP Cookie File\n");
    let far_future = 2_000_000_000i64;
    for c in cookies {
        let domain = c.domain().unwrap_or("youtube.com");
        let domain = if domain.starts_with('.') {
            domain.to_string()
        } else {
            format!(".{domain}")
        };
        let expiry = c
            .expires_datetime()
            .map(|t| t.unix_timestamp())
            .unwrap_or(far_future);
        s.push_str(&format!(
            "{domain}\tTRUE\t{}\t{}\t{expiry}\t{}\t{}\n",
            c.path().unwrap_or("/"),
            if c.secure().unwrap_or(true) { "TRUE" } else { "FALSE" },
            c.name(),
            c.value()
        ));
    }
    s
}

/// Parent side: run the helper and return the cookies.txt it printed.
pub fn sign_in(profile_dir: PathBuf) -> Result<String, String> {
    let _ = std::fs::remove_dir_all(&profile_dir);
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let out = std::process::Command::new(exe)
        .arg(HELPER_ARG)
        .arg(&profile_dir)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("could not open the sign-in window: {e}"))?;
    // Remove the web view's profile so this browser session is not reused.
    for _ in 0..10 {
        if std::fs::remove_dir_all(&profile_dir).is_ok() || !profile_dir.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(300));
    }
    match out.status.code() {
        Some(0) => {
            let txt = String::from_utf8_lossy(&out.stdout).into_owned();
            if txt.lines().count() < 3 {
                Err("sign-in finished but no YouTube cookies were received".into())
            } else {
                Ok(txt)
            }
        }
        Some(EXIT_CANCELLED) => Err("Sign-in window was closed.".into()),
        _ => Err(format!(
            "Sign-in window failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )),
    }
}
