//! Read signed-in lists from YouTube's web pages with the desktop app's session.
//!
//!     cargo run -p streampipe-core --example webfeed
use streampipe_core::model::Item;
use streampipe_core::webfeed::{Feed, fetch};

#[tokio::main]
async fn main() {
    streampipe_core::diagnostics::init();
    let paths = streampipe_core::config::Paths::desktop().unwrap();
    let key = streampipe_core::secure::desktop_key(&paths.data);
    let session = streampipe_core::secure::EncryptedFile::new(paths.data.join("session.bin"), &key).read().expect("no session");
    let json: serde_json::Value = serde_json::from_slice(&session).unwrap();
    let cookie = json["auth_cookie"]["cookie"].as_str().expect("not signed in with cookies").to_string();
    let http = streampipe_core::service::http_client();
    for feed in [Feed::Home, Feed::Subscriptions, Feed::Channels, Feed::History, Feed::Playlists] {
        match fetch(&http, &cookie, feed).await {
            Ok((p, more, _)) => {
                println!("{feed:?}: {} items, more: {}", p.items.len(), more.is_some());
                if let Some(m) = &more {
                    match streampipe_core::webfeed::more(&http, Some(&cookie), m).await {
                        Ok((p2, m2, _)) => println!("   next screen: {} items, more: {}", p2.items.len(), m2.is_some()),
                        Err(e) => println!("   next screen FAILED: {e}"),
                    }
                }
                for i in p.items.iter().take(3) {
                    match i {
                        Item::Video(v) => println!("   video  {:<40.40} | {:?} | {:?}s | {:?}", v.title, v.channel_name, v.duration_s, v.published),
                        Item::Channel(c) => println!("   channel {:<30.30} | avatar: {}", c.name, c.avatar.is_some()),
                        Item::Playlist(p) => println!("   playlist {:<30.30} | {:?}", p.name, p.channel_name),
                        Item::Post(p) => println!("   post {:<40.40}", p.text),
                    }
                }
            }
            Err(e) => println!("{feed:?}: FAILED: {e}"),
        }
    }
}
