//! Exercise every browse feature with the desktop app's signed-in session (a copy of it).
//!     cargo run -p streampipe-core --example features

use streampipe_core::api::{Api, ApiOptions};
use streampipe_core::config::Paths;
use streampipe_core::model::{Item, Page};

fn summary(p: &Page) -> String {
    let (mut v, mut c, mut l) = (0, 0, 0);
    for i in &p.items {
        match i {
            Item::Video(_) => v += 1,
            Item::Channel(_) => c += 1,
            Item::Playlist(_) => l += 1,
            Item::Post(_) => {}
        }
    }
    format!("{} items ({v} videos, {c} channels, {l} playlists), more pages: {}", p.items.len(), p.next.is_some())
}

#[tokio::main]
async fn main() {
    let real = Paths::desktop().unwrap();
    let key = streampipe_core::secure::desktop_key(&real.data);
    let dir = std::env::temp_dir().join("streampipe-features");
    let _ = std::fs::remove_dir_all(&dir);
    let paths = Paths::rooted(&dir, &dir.join("cache"), &dir.join("dl"));
    paths.ensure().unwrap();
    std::fs::copy(real.data.join("session.bin"), paths.data.join("session.bin")).expect("no saved session");
    let api = Api::new(&paths, ApiOptions { key, botguard: Some("D:/yt-client/third_party/botguard/bin/rustypipe-botguard.exe".into()) }).unwrap();
    let rp = api.rp();
    println!("cookie check: {:?}", rp.user_auth_check_cookie().await.map_err(|e| e.to_string()));

    macro_rules! t {
        ($name:expr, $e:expr) => {
            match $e.await {
                Ok(x) => println!("OK   {:<18} {}", $name, x),
                Err(e) => println!("FAIL {:<18} {}", $name, e.to_string().chars().take(160).collect::<String>()),
            }
        };
    }
    t!("home", async { api.home().await.map(|p| summary(&p)) });
    t!("search", async { api.search("lofi").await.map(|p| summary(&p)) });
    t!("video page", async { api.video("dQw4w9WgXcQ").await.map(|v| format!("\"{}\", related {}", v.video.title, summary(&v.related))) });
    t!("channel", async { api.channel("UCuAXFkgsw1L7xaCfnd5JJOw").await.map(|c| format!("{}: {}", c.channel.name, summary(&c.videos))) });
    t!("playlist", async { api.playlist("PL4fGSI1pDJn5kI81J1fYWK5eZRl1zJ5kM").await.map(|p| format!("{}: {}", p.playlist.name, summary(&p.videos))) });
    t!("sub feed", async { api.subscription_feed().await.map(|p| summary(&p)) });
    t!("subscriptions", async { api.subscriptions().await.map(|p| summary(&p)) });
    t!("history", async { api.history().await.map(|p| summary(&p)) });
    t!("saved playlists", async { api.saved_playlists().await.map(|p| summary(&p)) });
    let _ = std::fs::remove_dir_all(&dir);
}
