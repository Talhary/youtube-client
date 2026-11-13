//! Resolve a video with the direct resolver and fetch the first bytes of its streams.
//!
//!     cargo run -p streampipe-core --example direct -- <video id> [--signed-in-only]
use std::time::Instant;

#[tokio::main]
async fn main() {
    streampipe_core::diagnostics::init();
    let mut args = std::env::args().skip(1);
    let id = args.next().unwrap_or_else(|| "aqz-KE-bpKQ".into());
    let paths = streampipe_core::config::Paths::desktop().unwrap();
    let key = streampipe_core::secure::desktop_key(&paths.data);
    let cookie = streampipe_core::secure::EncryptedFile::new(paths.data.join("session.bin"), &key)
        .read()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|j| j["auth_cookie"]["cookie"].as_str().map(str::to_string));
    println!("session cookie: {}", if cookie.is_some() { "present" } else { "none" });
    if std::env::args().any(|a| a == "--node") {
        streampipe_core::ytp::use_node(Box::new(|script, port| {
            std::process::Command::new("node").arg(script).arg(port).spawn().map(|_| ()).map_err(|e| e.to_string())
        }));
    }
    let http = streampipe_core::service::http_client();
    let ytp = streampipe_core::ytp::Ytp::new(http.clone(), &std::env::temp_dir().join("streampipe-direct-test"));
    for round in 1..=2 {
        let t = Instant::now();
        let signed_only = std::env::args().any(|a| a == "--signed-in-only");
        let result = match (&cookie, signed_only) {
            (Some(c), true) => ytp.resolve_signed_in(&id, c).await,
            _ => ytp.resolve(&id, cookie.as_deref()).await,
        };
        match result {
            Ok(s) => {
                println!("round {round}: {:.2}s via {} | {} | {} video formats | aac {} | opus {}",
                    t.elapsed().as_secs_f32(), s.source, s.title, s.video.len(), s.audio_aac.is_some(), s.audio_opus.is_some());
                if round == 2 {
                    let mut picks: Vec<&streampipe_core::streams::StreamChoice> = Vec::new();
                    picks.extend(s.video.iter().filter(|v| v.codec == "h264").max_by_key(|v| v.height));
                    picks.extend(s.audio_aac.as_ref());
                    for f in picks {
                        let ua = f.headers.first().map(|h| h.1.clone()).unwrap_or_default();
                        let r = http.get(format!("{}&range=0-524287", f.url)).header("User-Agent", ua).send().await;
                        match r {
                            Ok(r) => {
                                let status = r.status();
                                let n = r.bytes().await.map(|b| b.len()).unwrap_or(0);
                                println!("   itag {} {}p {} init {:?} index {:?} -> HTTP {} {} bytes", f.itag, f.height, f.codec, f.init_range, f.index_range, status.as_u16(), n);
                            }
                            Err(e) => println!("   itag {}: {e}", f.itag),
                        }
                    }
                }
            }
            Err(e) => println!("round {round}: FAILED in {:.2}s: {e}", t.elapsed().as_secs_f32()),
        }
    }
}
