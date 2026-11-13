//! Per-client playback check: resolve a video with each client type, then fetch the first
//! 4 MB of a video stream (the way the downloader does) and report status and speed.
//!     cargo run -p streampipe-core --example clients -- [video_id]

use std::time::{Duration, Instant};

use rustypipe::client::ClientType::*;
use rustypipe::client::RustyPipe;

#[tokio::main]
async fn main() {
    let id = std::env::args().nth(1).unwrap_or("aqz-KE-bpKQ".into());
    let rp = RustyPipe::builder()
        .no_storage()
        .botguard_bin("D:/yt-client/third_party/botguard/bin/rustypipe-botguard.exe")
        .build_with_client(streampipe_core::service::http_builder())
        .unwrap();
    let http = streampipe_core::service::http_client();
    for c in [Desktop, Tv, Mobile, Android, Ios] {
        let t = Instant::now();
        let p = match rp.query().player_from_client(&id, c).await {
            Ok(p) => p,
            Err(e) => {
                println!("{c:?}: resolve failed: {e}");
                continue;
            }
        };
        let resolve = t.elapsed();
        let Some(v) = p.video_only_streams.iter().filter(|v| v.height <= 720).max_by_key(|v| v.height) else {
            println!("{c:?}: no video streams");
            continue;
        };
        let t = Instant::now();
        let r = http
            .get(format!("{}&range=0-4194303", v.url))
            .timeout(Duration::from_secs(60))
            .send()
            .await;
        match r {
            Ok(r) => {
                let status = r.status();
                let n = r.bytes().await.map(|b| b.len()).unwrap_or(0);
                let secs = t.elapsed().as_secs_f64();
                println!(
                    "{c:?}: resolved in {:.1}s, {}p, HTTP {status}, {:.1} MB in {secs:.1}s = {:.0} KB/s",
                    resolve.as_secs_f64(),
                    v.height,
                    n as f64 / 1e6,
                    n as f64 / 1024.0 / secs
                );
            }
            Err(e) => println!("{c:?}: fetch failed: {e}"),
        }
    }
}
