//! Live smoke test (PRD 5.1): search, player resolution and a first-chunk download against the
//! real API. Exits non-zero on any failure so the nightly CI job can alert.
//!
//!     cargo run -p streampipe-core --example smoke [-- path/to/rustypipe-botguard]

use std::time::Duration;

use streampipe_core::api::{Api, ApiOptions};
use streampipe_core::config::Paths;
use streampipe_core::model::Item;
use streampipe_core::secure::random_key;
use streampipe_core::streams::{self, HwCaps};

#[tokio::main]
async fn main() {
    streampipe_core::diagnostics::init();
    if let Err(e) = run().await {
        eprintln!("SMOKE FAILED: {e}");
        std::process::exit(1);
    }
    println!("SMOKE OK");
}

async fn run() -> Result<(), String> {
    let dir = std::env::temp_dir().join(format!("streampipe-smoke-{}", std::process::id()));
    let paths = Paths::rooted(&dir, &dir.join("cache"), &dir.join("dl"));
    paths.ensure().map_err(|e| e.to_string())?;
    let api = Api::new(
        &paths,
        ApiOptions {
            key: random_key(),
            botguard: std::env::args().nth(1).map(Into::into),
        },
    )
    .map_err(|e| e.to_string())?;
    println!("botguard: {:?}", api.botguard_version().await);

    let page = api.search("lofi hip hop").await.map_err(|e| format!("search: {e}"))?;
    let video = page
        .items
        .iter()
        .find_map(|i| match i {
            Item::Video(v) if !v.is_live => Some(v.clone()),
            _ => None,
        })
        .ok_or("search returned no videos")?;
    println!("search: {} items, first video {} \"{}\"", page.items.len(), video.id, video.title);

    if let Some(c) = page.next {
        let p2 = api.more(c).await.map_err(|e| format!("continuation: {e}"))?;
        println!("continuation: {} more items", p2.items.len());
    }

    let player = api
        .player(&video.id, None)
        .await
        .map_err(|e| format!("player: {e}"))?;
    let q = streams::qualities(&player.video_only_streams, &HwCaps::default());
    println!(
        "player: client {:?}, {} video rungs ({}), {} audio streams",
        player.client_type,
        q.len(),
        q.iter().map(|q| q.label.as_str()).collect::<Vec<_>>().join(" "),
        player.audio_streams.len()
    );
    let audio = streams::pick_audio(&player.audio_streams, rustypipe::model::AudioCodec::Mp4a)
        .ok_or("no audio stream")?;

    let http = streampipe_core::service::http_client();
    let r = http
        .get(format!("{}&range=0-262143", audio.url))
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("first chunk: {e}"))?;
    let status = r.status();
    let n = r.bytes().await.map_err(|e| e.to_string())?.len();
    if !status.is_success() || n != 262_144 {
        return Err(format!("first chunk: HTTP {status}, {n} bytes"));
    }
    println!("first chunk: HTTP {status}, {n} bytes");
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}
