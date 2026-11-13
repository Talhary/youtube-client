//! libmpv playback (PRD 2): the render API draws into an OpenGL framebuffer that is blitted
//! into the video rect from an egui paint callback, so video stays inside the window.
//!
//! - mpv's update callback requests a repaint only when a new frame is ready, so a paused or
//!   idle player costs no frames.
//! - DASH video and audio are separate streams: the audio URL rides along via `audio-files`.
//! - A dedicated thread drains mpv events into shared state.

use std::ffi::{CStr, CString, c_void};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use eframe::egui;
use eframe::egui_glow;
use eframe::glow::{self, HasContext};
use libmpv2::events::{Event, PropertyData};
use libmpv2::render::{OpenGLInitParams, RenderContext, RenderParam, RenderParamApiType};
use libmpv2::{Format, Mpv};
use parking_lot::Mutex;
use streampipe_ui::{LoadRequest, Player, PlayerEvent, PlayerState};

type ProcAddr = Arc<dyn Fn(&CStr) -> *const c_void + Send + Sync>;

fn get_proc_address(ctx: &ProcAddr, name: &str) -> *mut c_void {
    let Ok(c) = CString::new(name) else {
        return std::ptr::null_mut();
    };
    ctx(&c) as *mut c_void
}

/// GL-side state, touched only inside paint callbacks on the GL thread.
struct Gl {
    render: Option<RenderContext<'static>>,
    fbo: Option<(glow::Framebuffer, glow::Texture, i32, i32)>,
}

// SAFETY: `Gl` is only used from egui_glow paint callbacks, which all run on the thread that
// owns the GL context. The mutex serialises access.
unsafe impl Send for Gl {}

pub struct MpvPlayer {
    mpv: &'static Mpv,
    gl: Arc<Mutex<Gl>>,
    proc_addr: ProcAddr,
    ctx: egui::Context,
    state: Arc<Mutex<PlayerState>>,
    events: Receiver<PlayerEvent>,
    cache_speed: Arc<Mutex<Option<u64>>>,
    /// The render context asks for a fresh frame (set by mpv's update callback).
    frame_ready: Arc<AtomicBool>,
    /// Set once the OpenGL render context exists. mpv cannot open video output before that.
    render_ready: Arc<AtomicBool>,
    /// A video load waiting for the render context.
    pending: Option<LoadRequest>,
    current_codec: Arc<Mutex<Option<String>>>,
    /// When the last load was issued (for the load-to-first-frame log).
    load_at: Arc<Mutex<Option<Instant>>>,
}

impl MpvPlayer {
    pub fn new(ctx: egui::Context, proc_addr: ProcAddr) -> Result<Self, String> {
        let mpv = Mpv::with_initializer(|init| {
            init.set_property("vo", "libmpv")?;
            let hw = std::env::var("STREAMPIPE_HWDEC").unwrap_or_else(|_| hwdec().to_string());
            init.set_property("hwdec", hw.as_str())?;
            init.set_property("ytdl", false)?;
            init.set_property("keep-open", "yes")?;
            init.set_property("cache", "yes")?;
            init.set_property("demuxer-max-bytes", "150MiB")?;
            init.set_property("demuxer-max-back-bytes", "50MiB")?;
            init.set_property("input-default-bindings", false)?;
            init.set_property("osc", false)?;
            init.set_property("terminal", false)?;
            init.set_property("audio-client-name", "StreamPipe")?;
            init.set_property("title", "StreamPipe")?;
            init.set_property("network-timeout", 20i64)?;
            // Faster start: these DASH streams carry full headers, so skip lavf's stream
            // probing unless it found nothing, and cap the analysis window.
            // Measured: no difference in start time, so this is opt-in only.
            if std::env::var("STREAMPIPE_FASTPROBE").is_ok() {
                init.set_property("demuxer-lavf-probe-info", "nostreams")?;
                init.set_property("demuxer-lavf-analyzeduration", 0.5)?;
            }
            Ok(())
        })
        .map_err(|e| format!("could not start libmpv: {e}"))?;
        let mpv: &'static Mpv = Box::leak(Box::new(mpv));
        // Log warnings and errors so 403s can be told apart from other failures.
        unsafe {
            libmpv2_sys::mpv_request_log_messages(mpv.ctx.as_ptr(), c"warn".as_ptr());
        }
        let _ = mpv.disable_deprecated_events();
        for (i, (name, fmt)) in [
            ("time-pos", Format::Double),
            ("duration", Format::Double),
            ("pause", Format::Flag),
            ("paused-for-cache", Format::Flag),
            ("idle-active", Format::Flag),
            ("volume", Format::Double),
            ("cache-speed", Format::Int64),
            ("eof-reached", Format::Flag),
            ("seeking", Format::Flag),
        ]
        .into_iter()
        .enumerate()
        {
            let _ = mpv.observe_property(name, fmt, i as u64 + 1);
        }

        let state = Arc::new(Mutex::new(PlayerState {
            idle: true,
            volume: 100.0,
            ..Default::default()
        }));
        let cache_speed = Arc::new(Mutex::new(None));
        let current_codec = Arc::new(Mutex::new(None));
        let load_at = Arc::new(Mutex::new(None));
        let (tx, rx) = channel();
        spawn_event_thread(
            mpv,
            ctx.clone(),
            state.clone(),
            cache_speed.clone(),
            current_codec.clone(),
            load_at.clone(),
            tx,
        );

        Ok(Self {
            mpv,
            gl: Arc::new(Mutex::new(Gl {
                render: None,
                fbo: None,
            })),
            proc_addr,
            ctx,
            state,
            events: rx,
            cache_speed,
            frame_ready: Arc::new(AtomicBool::new(true)),
            render_ready: Arc::new(AtomicBool::new(false)),
            pending: None,
            current_codec,
            load_at,
        })
    }

    /// Free GL objects while the context is still current (called from `on_exit`).
    pub fn destroy_gl(&self, gl: &glow::Context) {
        let mut g = self.gl.lock();
        g.render = None;
        if let Some((fbo, tex, _, _)) = g.fbo.take() {
            unsafe {
                gl.delete_framebuffer(fbo);
                gl.delete_texture(tex);
            }
        }
        let _ = self.mpv.command("quit", &[]);
    }
}

fn hwdec() -> &'static str {
    // The OpenGL render API cannot share D3D11/VA-API surfaces directly, so decode in hardware
    // and copy frames back. Each list falls through to software if no decoder works.
    if cfg!(windows) {
        "d3d11va-copy,nvdec-copy,auto-copy-safe"
    } else {
        "vaapi-copy,nvdec-copy,auto-copy-safe"
    }
}

/// mpv's `%len%` quoting so URLs with commas and `=` survive option-list parsing.
fn quote(s: &str) -> String {
    format!("%{}%{}", s.len(), s)
}

fn spawn_event_thread(
    mpv: &'static Mpv,
    ctx: egui::Context,
    state: Arc<Mutex<PlayerState>>,
    cache_speed: Arc<Mutex<Option<u64>>>,
    codec: Arc<Mutex<Option<String>>>,
    load_at: Arc<Mutex<Option<Instant>>>,
    tx: Sender<PlayerEvent>,
) {
    std::thread::Builder::new()
        .name("mpv-events".into())
        .spawn(move || {
            let mut last_tick = Instant::now();
            let mut last_error_403 = false;
            loop {
                let Some(ev) = mpv.wait_event(1.0) else { continue };
                let mut urgent = true;
                match ev {
                    Ok(Event::Shutdown) => break,
                    Ok(Event::LogMessage { text, prefix, .. }) => {
                        if text.contains("403") || text.contains("Forbidden") {
                            last_error_403 = true;
                        }
                        eprintln!("[mpv/{prefix}] {}", text.trim_end());
                        urgent = false;
                    }
                    Ok(Event::StartFile) => {
                        last_error_403 = false;
                        let mut s = state.lock();
                        s.buffering = true;
                        s.idle = false;
                    }
                    Ok(Event::FileLoaded) | Ok(Event::PlaybackRestart) => {
                        state.lock().buffering = false;
                        if let Ok(Event::PlaybackRestart) = ev {
                            if let Some(t) = load_at.lock().take() {
                                eprintln!("[timing] playback started {:.2}s after load", t.elapsed().as_secs_f32());
                            }
                        }
                    }
                    Ok(Event::VideoReconfig) => {
                        let hw: String = mpv.get_property("hwdec-current").unwrap_or_default();
                        let c: String = mpv
                            .get_property("current-tracks/video/codec")
                            .unwrap_or_default();
                        if !c.is_empty() {
                            let mut cur = codec.lock();
                            if cur.as_deref() != Some(c.as_str()) {
                                *cur = Some(c.clone());
                                let _ = tx.send(PlayerEvent::Decoder {
                                    codec: normalize_codec(&c),
                                    hardware: !hw.is_empty() && hw != "no",
                                });
                            }
                        }
                    }
                    Ok(Event::EndFile(_)) => {}
                    Err(e) => {
                        // End-of-file errors arrive here: loading failed or the stream died.
                        let _ = tx.send(PlayerEvent::Error {
                            forbidden: last_error_403,
                            message: e.to_string(),
                        });
                        state.lock().buffering = false;
                    }
                    Ok(Event::PropertyChange { name, change, .. }) => {
                        let mut s = state.lock();
                        match (name, change) {
                            ("time-pos", PropertyData::Double(v)) => {
                                s.position_s = v;
                                // Position ticks often; while video plays the frame callback
                                // already repaints, so only nudge the UI 4× a second.
                                urgent = false;
                                if last_tick.elapsed() > Duration::from_millis(250) {
                                    last_tick = Instant::now();
                                    ctx.request_repaint();
                                }
                            }
                            ("duration", PropertyData::Double(v)) => s.duration_s = v,
                            ("pause", PropertyData::Flag(v)) => s.paused = v,
                            ("paused-for-cache", PropertyData::Flag(v)) => s.buffering = v,
                            ("seeking", PropertyData::Flag(v)) => s.buffering = v,
                            ("idle-active", PropertyData::Flag(v)) => s.idle = v,
                            ("volume", PropertyData::Double(v)) => s.volume = v as f32,
                            ("cache-speed", PropertyData::Int64(v)) => {
                                urgent = false;
                                if v > 0 {
                                    *cache_speed.lock() = Some(v as u64 * 8);
                                }
                            }
                            ("eof-reached", PropertyData::Flag(true)) => {
                                let _ = tx.send(PlayerEvent::Ended);
                            }
                            _ => urgent = false,
                        }
                    }
                    _ => urgent = false,
                }
                if urgent {
                    ctx.request_repaint();
                }
            }
        })
        .expect("spawn mpv event thread");
}

fn normalize_codec(c: &str) -> String {
    match c {
        "h264" | "avc1" => "h264",
        "vp9" | "vp09" => "vp9",
        "av1" | "av01" => "av1",
        other => other,
    }
    .to_string()
}

impl Player for MpvPlayer {
    fn load(&mut self, req: LoadRequest) {
        // Video needs the render context, which is created the first time the video area is
        // drawn. Until then, park the request; `show_video` starts it once the context exists.
        if req.video_url.is_some() && !self.render_ready.load(Ordering::Acquire) {
            self.pending = Some(req);
            self.state.lock().buffering = true;
            self.ctx.request_repaint();
            return;
        }
        self.pending = None;
        *self.load_at.lock() = Some(Instant::now());
        // Only seek when resuming mid-video; `start=0` still costs a seek before playback.
        let mut opts = Vec::new();
        if req.start_s > 0.5 {
            opts.push(format!("start={:.3}", req.start_s));
        }
        opts.push(format!("force-media-title={}", quote(&req.title)));
        let main = if let Some(v) = &req.video_url {
            if let Some(a) = &req.audio_url {
                opts.push(format!("audio-files={}", quote(a)));
            }
            v.clone()
        } else if let Some(h) = &req.hls_url {
            h.clone()
        } else if let Some(a) = &req.audio_url {
            opts.push("vid=no".into());
            a.clone()
        } else {
            return;
        };
        *self.current_codec.lock() = None;
        let opts = opts.join(",");
        if let Err(e) = self.mpv.command("loadfile", &[&main, "replace", "-1", &opts]) {
            eprintln!("mpv loadfile failed: {e}");
        }
        let _ = self.mpv.set_property("pause", false);
        // Diagnostics: STREAMPIPE_SHOT=<file.png> saves mpv's own decoded frame 8 s in,
        // bypassing our OpenGL drawing, to tell decoder problems from rendering problems.
        if let Ok(path) = std::env::var("STREAMPIPE_SHOT") {
            let mpv = self.mpv;
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(8));
                let _ = mpv.command("screenshot-to-file", &[&path, "video"]);
            });
        }
        self.frame_ready.store(true, Ordering::Relaxed);
    }

    fn stop(&mut self) {
        self.pending = None;
        let _ = self.mpv.command("stop", &[]);
        let mut s = self.state.lock();
        s.idle = true;
        s.position_s = 0.0;
        s.duration_s = 0.0;
    }

    fn set_paused(&mut self, paused: bool) {
        let _ = self.mpv.set_property("pause", paused);
    }

    fn seek(&mut self, position_s: f64) {
        let _ = self
            .mpv
            .command("seek", &[&format!("{position_s:.3}"), "absolute+keyframes"]);
        self.state.lock().position_s = position_s;
    }

    fn set_volume(&mut self, volume: f32) {
        let _ = self.mpv.set_property("volume", volume as f64);
        self.state.lock().volume = volume;
    }

    fn set_speed(&mut self, speed: f32) {
        let _ = self.mpv.set_property("speed", speed as f64);
    }

    fn set_loop(&mut self, on: bool) {
        let _ = self.mpv.set_property("loop-file", if on { "inf" } else { "no" });
    }

    fn set_leveller(&mut self, on: bool) {
        let _ = self.mpv.set_property("af", if on { "lavfi=[dynaudnorm=f=250:g=15]" } else { "" });
    }

    fn state(&self) -> PlayerState {
        *self.state.lock()
    }

    fn poll_events(&mut self) -> Vec<PlayerEvent> {
        self.events.try_iter().collect()
    }

    fn throughput_bps(&self) -> Option<u64> {
        *self.cache_speed.lock()
    }

    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }

    fn show_video(&mut self, ui: &mut egui::Ui, rect: egui::Rect) {
        if self.pending.is_some() {
            if self.render_ready.load(Ordering::Acquire) {
                let req = self.pending.take().unwrap();
                self.load(req);
            } else {
                // The paint callback below creates the context this frame; check again next.
                ui.ctx().request_repaint();
            }
        }
        let gl = self.gl.clone();
        let mpv = self.mpv;
        let proc_addr = self.proc_addr.clone();
        let ctx = self.ctx.clone();
        let frame_ready = self.frame_ready.clone();
        let render_ready = self.render_ready.clone();
        let cb = egui_glow::CallbackFn::new(move |info, painter| {
            let glc = painter.gl();
            let mut g = gl.lock();
            if g.render.is_none() {
                let r = mpv.create_render_context(vec![
                    RenderParam::ApiType(RenderParamApiType::OpenGl),
                    RenderParam::InitParams(OpenGLInitParams {
                        get_proc_address,
                        ctx: proc_addr.clone(),
                    }),
                ]);
                match r {
                    Ok(mut r) => {
                        let cb_ctx = ctx.clone();
                        let fr = frame_ready.clone();
                        r.set_update_callback(move || {
                            fr.store(true, Ordering::Relaxed);
                            cb_ctx.request_repaint();
                        });
                        g.render = Some(r);
                        render_ready.store(true, Ordering::Release);
                        ctx.request_repaint();
                    }
                    Err(e) => {
                        eprintln!("mpv render context failed: {e}");
                        return;
                    }
                }
            }
            let vp = info.viewport_in_pixels();
            let (w, h) = (vp.width_px.max(1), vp.height_px.max(1));
            unsafe {
                // (Re)create the framebuffer when the video rect changes size.
                let stale = g.fbo.is_none_or(|(_, _, fw, fh)| fw != w || fh != h);
                if stale {
                    if let Some((f, t, _, _)) = g.fbo.take() {
                        glc.delete_framebuffer(f);
                        glc.delete_texture(t);
                    }
                    let tex = glc.create_texture().unwrap();
                    glc.bind_texture(glow::TEXTURE_2D, Some(tex));
                    glc.tex_image_2d(
                        glow::TEXTURE_2D,
                        0,
                        glow::RGBA8 as i32,
                        w,
                        h,
                        0,
                        glow::RGBA,
                        glow::UNSIGNED_BYTE,
                        glow::PixelUnpackData::Slice(None),
                    );
                    glc.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
                    glc.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
                    let fbo = glc.create_framebuffer().unwrap();
                    glc.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                    glc.framebuffer_texture_2d(
                        glow::FRAMEBUFFER,
                        glow::COLOR_ATTACHMENT0,
                        glow::TEXTURE_2D,
                        Some(tex),
                        0,
                    );
                    glc.bind_framebuffer(glow::FRAMEBUFFER, None);
                    g.fbo = Some((fbo, tex, w, h));
                    frame_ready.store(true, Ordering::Relaxed);
                }
                let (fbo, _, _, _) = g.fbo.unwrap();
                // Render a new mpv frame only when mpv has one (or the size changed).
                if frame_ready.swap(false, Ordering::Relaxed) {
                    if let Some(r) = &g.render {
                        // egui leaves its clip rectangle (window coordinates) active. mpv draws
                        // into our own framebuffer from (0,0), so the clip would cut most of
                        // each frame away and leave stale or uninitialised rows behind.
                        // mpv also sets its own clip box and leaves it behind, so save egui's
                        // exact box and restore it for the blit into the window.
                        let scissor = glc.is_enabled(glow::SCISSOR_TEST);
                        let mut boxx = [0i32; 4];
                        glc.get_parameter_i32_slice(glow::SCISSOR_BOX, &mut boxx);
                        glc.disable(glow::SCISSOR_TEST);
                        glc.viewport(0, 0, w, h);
                        let id = fbo.0.get() as i32;
                        let _ = r.render::<ProcAddr>(id, w, h, true);
                        glc.scissor(boxx[0], boxx[1], boxx[2], boxx[3]);
                        if scissor {
                            glc.enable(glow::SCISSOR_TEST);
                        } else {
                            glc.disable(glow::SCISSOR_TEST);
                        }
                    }
                }
                // Blit into the rect. GL's origin is bottom-left.
                let x0 = vp.left_px;
                let y0 = vp.from_bottom_px;
                glc.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(fbo));
                glc.bind_framebuffer(glow::DRAW_FRAMEBUFFER, None);
                glc.blit_framebuffer(
                    0,
                    0,
                    w,
                    h,
                    x0,
                    y0,
                    x0 + w,
                    y0 + h,
                    glow::COLOR_BUFFER_BIT,
                    glow::LINEAR,
                );
                glc.bind_framebuffer(glow::FRAMEBUFFER, None);
                debug_capture(glc, fbo, x0, y0, w, h, mpv);
            }
            if let Some(r) = &g.render {
                r.report_swap();
            }
        });
        ui.painter().add(egui::PaintCallback {
            rect,
            callback: Arc::new(cb),
        });
    }
}

/// Diagnostics: with STREAMPIPE_GLSHOT=<dir>, save the off-screen frame (`fbo-N.png`) and
/// what was drawn into the window (`win-N.png`) at a few points, plus their sizes.
unsafe fn debug_capture(glc: &glow::Context, fbo: glow::Framebuffer, x: i32, y: i32, w: i32, h: i32, mpv: &Mpv) {
    use std::sync::atomic::AtomicU32;
    static FRAMES: AtomicU32 = AtomicU32::new(0);
    let Ok(dir) = std::env::var("STREAMPIPE_GLSHOT") else { return };
    let n = FRAMES.fetch_add(1, Ordering::Relaxed);
    if ![60, 200, 400].contains(&n) {
        return;
    }
    let osd: String = mpv.get_property::<String>("osd-dimensions").unwrap_or_default();
    let dw: i64 = mpv.get_property("dwidth").unwrap_or(0);
    let dh: i64 = mpv.get_property("dheight").unwrap_or(0);
    eprintln!("[glshot {n}] rect=({x},{y}) {w}x{h} video={dw}x{dh} osd={osd}");
    let save = |name: &str, read_fb: Option<glow::Framebuffer>, rx: i32, ry: i32| {
        let mut px = vec![0u8; (w * h * 4) as usize];
        unsafe {
            glc.bind_framebuffer(glow::READ_FRAMEBUFFER, read_fb);
            glc.read_pixels(rx, ry, w, h, glow::RGBA, glow::UNSIGNED_BYTE, glow::PixelPackData::Slice(Some(&mut px)));
            glc.bind_framebuffer(glow::READ_FRAMEBUFFER, None);
        }
        let row = (w * 4) as usize;
        let flipped: Vec<u8> = px.chunks(row).rev().flatten().copied().collect();
        if let Some(img) = image::RgbaImage::from_raw(w as u32, h as u32, flipped) {
            let _ = img.save(std::path::Path::new(&dir).join(format!("{name}-{n}.png")));
        }
    };
    save("fbo", Some(fbo), 0, 0);
    save("win", None, x, y);
}
