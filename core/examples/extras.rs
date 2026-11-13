//! Checks the web readers added for the Android app against live YouTube, signed out:
//! destination pages, channel posts, captions, preview pictures, chapters and live chat.
//!
//!     cargo run -p streampipe-core --example extras [LIVE_VIDEO_ID]

use streampipe_core::model::Item;
use streampipe_core::webfeed::{self, Feed};

#[tokio::main]
async fn main() {
    let http = streampipe_core::service::http_client();
    let count = |items: &[Item]| {
        let (mut v, mut p, mut o) = (0, 0, 0);
        for i in items {
            match i {
                Item::Video(_) => v += 1,
                Item::Playlist(_) => p += 1,
                Item::Post(_) => o += 1,
                Item::Channel(_) => {}
            }
        }
        format!("{v} videos, {p} playlists, {o} posts")
    };
    for (name, url) in [
        ("music", "https://www.youtube.com/channel/UC-9-kyTW8ZkZNDHQJ6FgpwQ"),
        ("gaming", "https://www.youtube.com/gaming"),
        ("sports", "https://www.youtube.com/channel/UCEgdi0XIXXZ-qJOFPf4JSKw"),
    ] {
        match webfeed::fetch_url(&http, None, url, Feed::Explore).await {
            Ok((p, more, _)) => println!("explore {name}: {} (more: {})", count(&p.items), more.is_some()),
            Err(e) => println!("explore {name}: FAILED {e}"),
        }
    }
    match webfeed::fetch_url(&http, None, "https://www.youtube.com/@mkbhd/posts", Feed::Posts).await {
        Ok((p, _, _)) => {
            println!("posts: {}", count(&p.items));
            if let Some(Item::Post(post)) = p.items.first() {
                println!("   {:?} | {:.60} | {:?} | images {} | video {}", post.author, post.text, post.likes, post.images.len(), post.video.is_some());
            }
        }
        Err(e) => println!("posts: FAILED {e}"),
    }
    match webfeed::player_extras(&http, "dQw4w9WgXcQ").await {
        Ok(x) => {
            let tracks = x["captions"].as_array().cloned().unwrap_or_default();
            println!("captions: {} tracks, storyboard: {}", tracks.len(), x["storyboard"].is_string());
            if let Some(t) = tracks.first() {
                match webfeed::captions(&http, t["url"].as_str().unwrap(), t["agent"].as_str()).await {
                    Ok(c) => println!("   {} lines, first: {}", c["cues"].as_array().map_or(0, |a| a.len()), c["cues"][0]),
                    Err(e) => println!("   lines FAILED {e}"),
                }
            }
        }
        Err(e) => println!("captions: FAILED {e}"),
    }
    match webfeed::watch(&http, None, "rfscVS0vtbw", None).await {
        Ok(p) => println!("chapters: {} (first {:?})", p.chapters.len(), p.chapters.get(1).map(|c| (&c.title, c.start))),
        Err(e) => println!("chapters: FAILED {e}"),
    }
    if let Some(live) = std::env::args().nth(1) {
        let mut personal = webfeed::Personal::default();
        let _ = webfeed::watch(&http, None, &live, Some(&mut personal)).await;
        println!("live: {} chat: {}", personal.live, personal.chat.is_some());
        if let Some(t) = personal.chat {
            match webfeed::live_chat(&http, None, &t).await {
                Ok(c) => println!("   {} messages, next: {}, wait {}", c["messages"].as_array().map_or(0, |a| a.len()), c["next"].is_string(), c["wait_ms"]),
                Err(e) => println!("   chat FAILED {e}"),
            }
        }
    }
}
