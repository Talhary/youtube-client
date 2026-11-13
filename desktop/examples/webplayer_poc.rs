//! Proof of concept: play a video in YouTube's own web player inside a web view, signed in
//! with the app's cookies, with ads removed from the player data before the player sees them.
//!
//!     cargo run -p streampipe --example webplayer_poc -- <video id> [seconds] [--no-strip]
//!
//! Prints what the page reports (signed in?, ad entries removed, ads that still showed,
//! playback position) and whether YouTube changed the session cookies while it ran.

use std::time::{Duration, Instant};

use tao::event::{Event, StartCause, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoop};
use tao::window::WindowBuilder;
use wry::{WebContext, WebViewBuilder};

/// Runs before any page script. Removes ad data from every player response (the page's first
/// one, and later ones fetched as JSON), hides everything but the player, and reports.
const INJECT: &str = r#"
(() => {
  if (window.top !== window || !location.hostname.endsWith('youtube.com')) return;
  const say = (m) => { try { window.ipc.postMessage(m); } catch (e) {} };
  const STRIP = __STRIP__;
  const AD_KEYS = ['adPlacements', 'adSlots', 'playerAds', 'adBreakHeartbeatParams'];
  let removed = 0;   // player responses that carried ad data
  let rewritten = 0; // network responses rewritten before the page read them
  let skipped = 0;   // ads that started anyway and were skipped

  // Layer 1: remove ad data from player responses, wherever they are nested.
  const prune = (o, depth) => {
    if (!o || typeof o !== 'object' || depth > 4) return o;
    if (Array.isArray(o)) { for (const x of o) prune(x, depth + 1); return o; }
    if (AD_KEYS.some((k) => k in o)) {
      removed++;
      if (STRIP) for (const k of AD_KEYS) delete o[k];
    }
    for (const k of ['playerResponse', 'response', 'args', 'raw_player_response']) {
      if (o[k] && typeof o[k] === 'object') prune(o[k], depth + 1);
    }
    return o;
  };
  const parse = JSON.parse;
  JSON.parse = function () { return prune(parse.apply(this, arguments), 0); };
  const json = Response.prototype.json;
  Response.prototype.json = function () { return json.apply(this, arguments).then((r) => prune(r, 0)); };
  for (const name of ['ytInitialPlayerResponse', 'ytInitialData']) {
    let held;
    try {
      Object.defineProperty(window, name, {
        configurable: true,
        get() { return held; },
        set(v) { held = name === 'ytInitialPlayerResponse' ? prune(v, 0) : v; },
      });
    } catch (e) {}
  }

  // Layer 2: rewrite the player endpoints' response text before any page code reads it.
  const PLAYER_URL = /\/youtubei\/v1\/(player|get_watch)|\/watch\?|\/playlist\?list=/;
  const scrub = (t) => t.replace(/"(adPlacements|adSlots|playerAds|adBreakHeartbeatParams)"/g, '"no_$1"');
  if (STRIP) {
    const realFetch = window.fetch;
    window.fetch = function (input, init) {
      const url = typeof input === 'string' ? input : (input && input.url) || '';
      const p = realFetch.apply(this, arguments);
      if (!PLAYER_URL.test(url)) return p;
      return p.then((resp) => resp.clone().text().then((t) => {
        const out = scrub(t);
        if (out === t) return resp;
        rewritten++;
        return new Response(out, { status: resp.status, statusText: resp.statusText, headers: resp.headers });
      }).catch(() => resp));
    };
  }

  const css = `
    html, body { overflow: hidden !important; background: #000 !important; }
    #masthead-container, #secondary, #below, #chat, ytd-popup-container, tp-yt-iron-overlay-backdrop,
    ytd-enforcement-message-view-model, .ytp-ad-overlay-container, .ytp-ad-message-container,
    .ytp-suggested-action, .ytp-paid-content-overlay, #player-ads, #panels { display: none !important; }
    #movie_player { position: fixed !important; inset: 0 !important; width: 100vw !important;
      height: 100vh !important; z-index: 2147483000 !important; }
    #movie_player .html5-video-container, #movie_player video { width: 100vw !important;
      height: 100vh !important; left: 0 !important; top: 0 !important; object-fit: contain !important; }
  `;
  const addCss = () => {
    const s = document.createElement('style'); s.textContent = css;
    (document.head || document.documentElement).appendChild(s);
  };
  if (document.documentElement) addCss(); else document.addEventListener('DOMContentLoaded', addCss);

  // Layer 3: if an ad starts anyway, jump to its end and press Skip.
  const adState = () => {
    const p = document.querySelector('#movie_player');
    if (!p) return null;
    const cls = p.classList.contains('ad-showing') || p.classList.contains('ad-interrupting');
    const mod = p.querySelector('.video-ads');
    const modKids = mod ? mod.childElementCount : 0;
    const overlay = !!p.querySelector('.ytp-ad-player-overlay, .ytp-ad-player-overlay-layout, .ytp-ad-skip-button-container, .ytp-skip-ad-button');
    return { cls, modKids, overlay, any: cls || overlay };
  };
  let adTicks = 0, firstAdAt = null, inAd = false, wall = false;
  const t0 = Date.now();
  setInterval(() => {
    const st = adState();
    if (!st || !st.any) { inAd = false; return; }
    adTicks++;
    if (firstAdAt === null) firstAdAt = Math.round((Date.now() - t0) / 100) / 10;
    if (!STRIP) return;
    if (!inAd) { inAd = true; skipped++; }
    const v = document.querySelector('#movie_player video');
    if (v) {
      v.muted = true;
      if (isFinite(v.duration) && v.duration > 0 && v.currentTime < v.duration - 0.3) v.currentTime = v.duration;
    }
    const btn = document.querySelector('.ytp-skip-ad-button, .ytp-ad-skip-button, .ytp-ad-skip-button-modern, .ytp-ad-skip-button-container button');
    if (btn) btn.click();
  }, 100);
  // The player mutes itself for the ad skip above; undo it once the real video is back.
  setInterval(() => {
    const st = adState();
    const v = document.querySelector('#movie_player video');
    if (STRIP && v && st && !st.any && v.muted && skipped > 0) v.muted = false;
  }, 500);

  setInterval(() => {
    const p = document.querySelector('#movie_player');
    const v = document.querySelector('#movie_player video');
    const st = adState() || {};
    if (document.querySelector('ytd-enforcement-message-view-model')) wall = true;
    let logged = null;
    try { logged = window.ytcfg && window.ytcfg.get('LOGGED_IN'); } catch (e) {}
    say(JSON.stringify({
      t: Math.round((Date.now() - t0) / 1000), signedIn: logged, removed, rewritten, skipped,
      adNow: !!st.any, adTicks, firstAdAt, adModuleKids: st.modKids, adblockWall: wall,
      pos: v ? Math.round(v.currentTime * 10) / 10 : null, paused: v ? v.paused : null, h: v ? v.videoHeight : null,
      err: p && p.querySelector('.ytp-error') ? p.querySelector('.ytp-error').innerText.slice(0, 80) : null,
    }));
  }, 3000);
})();
"#;


const LOGIN_URL: &str = "https://accounts.google.com/ServiceLogin?service=youtube&uilel=3&passive=true&continue=https%3A%2F%2Fwww.youtube.com%2Fsignin%3Faction_handle_signin%3Dtrue%26app%3Ddesktop%26next%3D%252F";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let strip = !args.iter().any(|a| a == "--no-strip");
    let pos: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    let id = pos.first().map(|s| s.as_str()).unwrap_or("aqz-KE-bpKQ").to_string();
    let secs: u64 = pos.get(1).and_then(|s| s.parse().ok()).unwrap_or(60);

    // Kept between runs: this profile owns the sign-in.
    let paths = streampipe_core::config::Paths::desktop().expect("no home directory");
    let profile = paths.data.join("webplayer-profile");
    let event_loop = EventLoop::new();
    let window = WindowBuilder::new()
        .with_title("StreamPipe web player test - sign in, then the test video starts")
        .with_inner_size(tao::dpi::LogicalSize::new(1000.0, 640.0))
        .build(&event_loop)
        .expect("window");
    let mut ctx = WebContext::new(Some(profile));
    let webview = WebViewBuilder::new_with_web_context(&mut ctx)
        .with_url(LOGIN_URL)
        .with_autoplay(true)
        .with_initialization_script(INJECT.replace("__STRIP__", if strip { "true" } else { "false" }))
        .with_ipc_handler(|req| println!("page: {}", req.body()))
        .build(&window)
        .expect("web view");
    println!("ad removal: {strip}; waiting for sign-in");

    let opened = Instant::now();
    let mut playing_since: Option<Instant> = None;
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(800));
        match event {
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                if let Some(t) = playing_since {
                    if t.elapsed() > Duration::from_secs(secs) {
                        println!("test finished");
                        std::process::exit(0);
                    }
                    return;
                }
                if opened.elapsed() > Duration::from_secs(900) {
                    println!("nobody signed in within 15 minutes");
                    std::process::exit(2);
                }
                let on_youtube = webview.url().map(|u| u.starts_with("https://www.youtube.com/")).unwrap_or(false);
                if !on_youtube {
                    return;
                }
                let Ok(cookies) = webview.cookies_for_url("https://www.youtube.com/") else { return };
                let has = |n: &str| cookies.iter().any(|c| c.name() == n && !c.value().is_empty());
                if (has("SAPISID") || has("__Secure-3PAPISID")) && has("LOGIN_INFO") {
                    println!("signed in after {:.0}s; starting {id}", opened.elapsed().as_secs_f32());
                    let _ = webview.load_url(&format!("https://www.youtube.com/watch?v={id}"));
                    playing_since = Some(Instant::now());
                }
            }
            Event::WindowEvent { event: WindowEvent::CloseRequested, .. } => {
                println!("window closed");
                std::process::exit(3);
            }
            _ => {}
        }
    })
}
