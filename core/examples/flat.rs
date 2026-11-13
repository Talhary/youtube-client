//! Check yt-dlp flat lists with the desktop app's signed-in session (a copy).
use streampipe_core::config::Paths;
use streampipe_core::model::Item;
use streampipe_core::ytdlp::YtDlp;

#[tokio::main]
async fn main() {
    let real = Paths::desktop().unwrap();
    let key = streampipe_core::secure::desktop_key(&real.data);
    let dir = std::env::temp_dir().join("streampipe-flat");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("yt-dlp")).unwrap();
    std::fs::copy(real.data.join("session.bin"), dir.join("session.bin")).unwrap();
    for f in ["yt-dlp.exe", "deno.exe"] {
        let src = std::path::Path::new("D:/yt-client/target/debug").join(f);
        let dst = if f == "deno.exe" { dir.join("deno") } else { dir.join("yt-dlp") };
        std::fs::create_dir_all(&dst).unwrap();
        std::fs::copy(src, dst.join(f)).unwrap();
    }
    let y = YtDlp::find(&dir, &dir.join("cache"), &key).expect("yt-dlp");
    for (name, url, n) in [
        ("playlist", "https://www.youtube.com/playlist?list=PL4fGSI1pDJn5kI81J1fYWK5eZRl1zJ5kM", 300),
        ("sub feed", "https://www.youtube.com/feed/subscriptions", 60),
        ("channels", "https://www.youtube.com/feed/channels", 1000),
    ] {
        let t = std::time::Instant::now();
        match y.flat(url, n).await {
            Ok(r) => {
                let (mut v, mut c, mut p) = (0, 0, 0);
                for i in &r.items { match i { Item::Video(_) => v += 1, Item::Channel(_) => c += 1, Item::Playlist(_) => p += 1, Item::Post(_) => {} } }
                println!("OK   {name:9} {:.1}s \"{}\" {v} videos, {c} channels, {p} playlists", t.elapsed().as_secs_f32(), r.title);
            }
            Err(e) => println!("FAIL {name:9} {:.1}s {}", t.elapsed().as_secs_f32(), e.chars().take(160).collect::<String>()),
        }
    }
    // Subscriptions feed the way the app now builds it: channels from yt-dlp, uploads via RSS.
    let t = std::time::Instant::now();
    let ch: Vec<_> = y.flat("https://www.youtube.com/feed/subscriptions", 2000).await.unwrap().items.into_iter()
        .filter_map(|i| match i { Item::Channel(c) => Some(c), _ => None }).collect();
    let paths = Paths::rooted(&dir, &dir.join("cache"), &dir.join("dl"));
    let api = streampipe_core::api::Api::new(&paths, streampipe_core::api::ApiOptions { key, botguard: None }).unwrap();
    let feed = api.rss_feed(&ch, 150).await;
    println!("OK   rss feed  {:.1}s {} channels -> {} videos; newest: {:?}", t.elapsed().as_secs_f32(), ch.len(), feed.items.len(),
        feed.items.first().map(|i| match i { Item::Video(v) => format!("{} ({})", v.title, v.published.clone().unwrap_or_default()), _ => String::new() }));
    let _ = std::fs::remove_dir_all(&dir);
}
