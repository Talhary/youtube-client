//! End-to-end download through the Core command channel.
//!     cargo run -p streampipe-core --example download -- <video_id> [audio]
use std::sync::Arc;
use std::time::{Duration, Instant};
use streampipe_core::config::{AudioOut, Paths};
use streampipe_core::download::{DownloadEvent, DownloadKind, DownloadRequest, JobStatus};
use streampipe_core::mux::{FfmpegMuxer, find_ffmpeg};
use streampipe_core::{Command, Core, CoreOptions, Event};

fn main() {
    let id = std::env::args().nth(1).unwrap_or("jNQXAC9IVRw".into());
    let audio = std::env::args().nth(2).is_some();
    let dir = std::env::temp_dir().join("streampipe-dl-test");
    let paths = Paths::rooted(&dir, &dir.join("cache"), &dir.join("out"));
    // Make the bundled yt-dlp + Deno visible to the core (it looks in <data>/<tool>/).
    for (tool, file) in [("yt-dlp", "yt-dlp.exe"), ("deno", "deno.exe")] {
        let d = paths.data.join(tool);
        std::fs::create_dir_all(&d).unwrap();
        let _ = std::fs::copy(std::path::Path::new("D:/yt-client/target/debug").join(file), d.join(file));
    }
    let ff = Arc::new(FfmpegMuxer::new(find_ffmpeg(&paths.data)));
    let mut core = Core::start(CoreOptions {
        paths, key: streampipe_core::secure::random_key(),
        botguard: Some("D:/yt-client/third_party/botguard/bin/rustypipe-botguard.exe".into()),
        muxer: ff.clone(), ffmpeg: Some(ff), caps: Default::default(),
        waker: Arc::new(|| {}), version: "0.1.0".into(), github_updates: false,
    }).unwrap();
    let kind = if audio { DownloadKind::Audio(AudioOut::Opus) } else { DownloadKind::Video { max_height: Some(360) } };
    core.handle.send(Command::Download(DownloadRequest { video_id: id, title: "test video".into(), kind, group: None }));
    let events = core.take_events();
    let t = Instant::now();
    loop {
        match events.recv_timeout(Duration::from_secs(120)) {
            Ok(Event::Download(DownloadEvent::Status { status, .. })) => {
                println!("[{:>5.1}s] {status:?}", t.elapsed().as_secs_f32());
                match status { JobStatus::Done(p) => { println!("size {} bytes", std::fs::metadata(&p).unwrap().len()); break }
                    JobStatus::Failed(_) => std::process::exit(1), _ => {} }
            }
            Ok(Event::Download(DownloadEvent::Progress { done, total, .. })) => println!("  {done}/{total}"),
            Ok(_) => {}
            Err(_) => { eprintln!("timeout"); std::process::exit(1) }
        }
    }
}
