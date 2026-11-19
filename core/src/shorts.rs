//! YouTube Shorts without a web view: the feed comes from the same `reel` page API the
//! YouTube site uses (`reel_item_watch` for a first Short and the sequence handle,
//! `reel_watch_sequence` for the Shorts after it), and each Short's channel, title, likes
//! and comment count come from `reel_item_watch` for that Short. The video itself plays in
//! the app's native player from the streams the resolver finds, like any other video.

use serde_json::{json, Value};
use std::time::Duration;

const WEB_VERSION: &str = "2.20261111.01.00";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

/// A POST to `youtubei/v1/<path>`, signed in when `cookie` is given.
async fn post(http: &reqwest::Client, cookie: Option<&str>, path: &str, mut body: Value) -> Result<Value, String> {
    body["context"] = json!({"client": {"clientName": "WEB", "clientVersion": WEB_VERSION, "hl": "en", "gl": "US"}});
    let mut req = http
        .post(format!("https://www.youtube.com/youtubei/v1/{path}?prettyPrint=false"))
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ORIGIN, "https://www.youtube.com")
        .header("X-Origin", "https://www.youtube.com")
        .header("X-Youtube-Client-Name", "1")
        .header("X-Youtube-Client-Version", WEB_VERSION)
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .timeout(Duration::from_secs(20))
        .json(&body);
    if let Some(c) = cookie {
        if let Some(auth) = crate::webfeed::sapisid_auth(c) {
            req = req
                .header(reqwest::header::COOKIE, c)
                .header(reqwest::header::AUTHORIZATION, auth)
                .header("X-Goog-AuthUser", "0");
        }
    }
    let resp = req.send().await.map_err(|e| format!("could not reach YouTube: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("YouTube answered HTTP {}", resp.status().as_u16()));
    }
    resp.json().await.map_err(|e| format!("unexpected answer from YouTube: {e}"))
}

/// One Short in the feed: its id, the player handle YouTube gave it, and its first frame.
fn entry(endpoint: &Value) -> Option<Value> {
    let id = endpoint.get("videoId")?.as_str()?;
    let thumb = endpoint
        .pointer("/thumbnail/thumbnails/0/url")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| format!("https://i.ytimg.com/vi/{id}/oardefault.jpg"));
    Some(json!({
        "id": id,
        "params": endpoint.get("playerParams").and_then(Value::as_str).unwrap_or(""),
        "thumb": thumb,
    }))
}

fn sequence(data: &Value) -> (Vec<Value>, Option<String>) {
    let items = data
        .get("entries")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|e| e.pointer("/command/reelWatchEndpoint").and_then(entry)).collect())
        .unwrap_or_default();
    let next = data
        .pointer("/continuationEndpoint/continuationCommand/token")
        .and_then(Value::as_str)
        .map(str::to_string);
    (items, next)
}

/// The start of the Shorts feed: `{items: [{id, params, thumb}], next}`. With `seed`, that
/// Short comes first and the feed follows on from it (as when a Short is opened from a list).
pub async fn feed(http: &reqwest::Client, cookie: Option<&str>, seed: Option<&str>) -> Result<Value, String> {
    let mut body = json!({"params": "CA8%3D", "disablePlayerResponse": true});
    match seed {
        Some(id) => body["playerRequest"] = json!({"videoId": id}),
        None => body["inputType"] = json!("REEL_WATCH_INPUT_TYPE_SEEDLESS"),
    }
    let first = post(http, cookie, "reel/reel_item_watch", body).await?;
    let mut items = Vec::new();
    match seed {
        Some(id) => items.push(json!({"id": id, "params": "", "thumb": format!("https://i.ytimg.com/vi/{id}/oardefault.jpg")})),
        None => {
            if let Some(e) = first.pointer("/replacementEndpoint/reelWatchEndpoint").and_then(entry) {
                items.push(e);
            }
        }
    }
    let token = match first.get("sequenceContinuation").and_then(Value::as_str) {
        Some(t) => t.to_string(),
        None if seed.is_some() => {
            // A seeded request does not always carry the feed handle: the tapped Short comes
            // first, then the regular feed follows.
            let rest = post(
                http,
                cookie,
                "reel/reel_item_watch",
                json!({"params": "CA8%3D", "disablePlayerResponse": true, "inputType": "REEL_WATCH_INPUT_TYPE_SEEDLESS"}),
            )
            .await?;
            if let Some(e) = rest.pointer("/replacementEndpoint/reelWatchEndpoint").and_then(entry) {
                if e["id"] != items[0]["id"] {
                    items.push(e);
                }
            }
            rest.get("sequenceContinuation").and_then(Value::as_str).ok_or("YouTube sent no Shorts feed")?.to_string()
        }
        None => return Err("YouTube sent no Shorts feed".into()),
    };
    let rest = post(http, cookie, "reel/reel_watch_sequence", json!({"sequenceParams": token})).await?;
    let (more, next) = sequence(&rest);
    for m in more {
        if !items.iter().any(|i| i["id"] == m["id"]) {
            items.push(m);
        }
    }
    Ok(json!({"items": items, "next": next}))
}

/// More of the feed, from the `next` handle of the previous answer.
pub async fn more(http: &reqwest::Client, cookie: Option<&str>, token: &str) -> Result<Value, String> {
    let data = post(http, cookie, "reel/reel_watch_sequence", json!({"sequenceParams": token})).await?;
    let (items, next) = sequence(&data);
    Ok(json!({"items": items, "next": next}))
}

fn first_str<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    fn go<'a>(v: &'a Value, key: &str, depth: usize) -> Option<&'a Value> {
        if depth > 40 {
            return None;
        }
        match v {
            Value::Object(o) => {
                if let Some(x) = o.get(key) {
                    return Some(x);
                }
                o.values().find_map(|c| go(c, key, depth + 1))
            }
            Value::Array(a) => a.iter().find_map(|c| go(c, key, depth + 1)),
            _ => None,
        }
    }
    go(v, key, 0)
}

/// What the Shorts player shows over a Short: channel, title, likes, comments and the
/// account's like and subscription state.
pub async fn info(http: &reqwest::Client, cookie: Option<&str>, id: &str, params: &str) -> Result<Value, String> {
    let mut req = json!({"videoId": id});
    if !params.is_empty() {
        req["params"] = json!(params);
    }
    let data = post(http, cookie, "reel/reel_item_watch", json!({"playerRequest": req, "params": "CA8%3D", "disablePlayerResponse": true})).await?;
    let overlay = data
        .pointer("/overlay/reelPlayerOverlayRenderer/playerOverlay/reelPlayerOverlayViewModel")
        .cloned()
        .unwrap_or(Value::Null);
    let bar = first_str(&overlay, "reelChannelBarViewModel").cloned().unwrap_or(Value::Null);
    let channel = bar.pointer("/channelName/content").and_then(Value::as_str).unwrap_or("");
    let avatar = first_str(&bar, "sources").and_then(|s| s.pointer("/0/url")).and_then(Value::as_str).unwrap_or("");
    let sub = first_str(&bar, "subscribeButtonViewModel").cloned().unwrap_or(Value::Null);
    let channel_id = first_str(&sub, "channelId").and_then(Value::as_str).unwrap_or("");
    let subscribed = sub.pointer("/subscribeButtonViewModel/subscribeButtonContent/subscribeState/subscribed").and_then(Value::as_bool);
    let title = first_str(&overlay, "shortsVideoTitleViewModel")
        .and_then(|t| t.pointer("/text/content"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let buttons = overlay.pointer("/actionBar/reelActionBarViewModel/buttonViewModels").and_then(Value::as_array);
    let mut likes = String::new();
    let mut liked = String::from("INDIFFERENT");
    let mut comments = String::new();
    for b in buttons.into_iter().flatten() {
        if let Some(l) = b.get("likeButtonViewModel") {
            likes = first_str(l, "title").and_then(Value::as_str).unwrap_or("").to_string();
            if let Some(s) = l.pointer("/likeStatusEntity/likeStatus").and_then(Value::as_str) {
                liked = s.to_string();
            }
        } else if let Some(t) = b.pointer("/buttonViewModel/title").and_then(Value::as_str) {
            // Comments is the first plain button after the like/dislike pair.
            let icon = b.pointer("/buttonViewModel/iconName").and_then(Value::as_str).unwrap_or("");
            if comments.is_empty() && (icon.contains("COMMENT") || icon.is_empty()) && t != "Share" {
                comments = t.to_string();
            }
        }
    }
    let ago = first_str(&data, "reelPlayerHeaderRenderer")
        .and_then(|h| h.pointer("/timestampText/simpleText"))
        .and_then(Value::as_str)
        .unwrap_or("");
    Ok(json!({
        "id": id,
        "title": title,
        "channel": channel,
        "channel_id": channel_id,
        "avatar": avatar,
        "subscribed": subscribed,
        "likes": likes,
        "liked": liked,
        "comments": comments,
        "ago": ago,
    }))
}

/// The Shorts shelf of a page (Home): `[{id, title, views, thumb}]`.
pub fn shelf(data: &Value) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    crate::webfeed::find_all_pub(data, "shortsLockupViewModel", &mut |s| {
        let id = s
            .pointer("/onTap/innertubeCommand/reelWatchEndpoint/videoId")
            .and_then(Value::as_str)
            .or_else(|| s.get("entityId").and_then(Value::as_str).and_then(|e| e.rsplit('-').next()));
        let Some(id) = id else { return };
        if out.iter().any(|o| o["id"] == id) {
            return;
        }
        let title = s.pointer("/overlayMetadata/primaryText/content").and_then(Value::as_str).unwrap_or("");
        let views = s.pointer("/overlayMetadata/secondaryText/content").and_then(Value::as_str).unwrap_or("");
        let thumb = s
            .pointer("/thumbnail/sources/0/url")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| format!("https://i.ytimg.com/vi/{id}/oardefault.jpg"));
        out.push(json!({"id": id, "title": title, "views": views, "thumb": thumb}));
    });
    out
}

/// The signed-in account for the "You" tab: `{name, handle, avatar, channel_id}`.
pub async fn account(http: &reqwest::Client, cookie: &str) -> Result<Value, String> {
    let data = post(http, Some(cookie), "account/account_menu", json!({})).await?;
    let head = first_str(&data, "activeAccountHeaderRenderer").cloned().ok_or("YouTube sent no account details")?;
    let text = |k: &str| {
        head.get(k)
            .and_then(|t| t.get("simpleText").and_then(Value::as_str).map(str::to_string).or_else(|| {
                t.get("runs").and_then(Value::as_array).map(|r| r.iter().filter_map(|x| x.get("text").and_then(Value::as_str)).collect::<String>())
            }))
            .unwrap_or_default()
    };
    let avatar = head
        .pointer("/accountPhoto/thumbnails")
        .and_then(Value::as_array)
        .and_then(|a| a.last())
        .and_then(|t| t.get("url"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let mut channel_id = String::new();
    crate::webfeed::find_all_pub(&data, "browseId", &mut |v| {
        if channel_id.is_empty() {
            if let Some(s) = v.as_str().filter(|s| s.starts_with("UC")) {
                channel_id = s.to_string();
            }
        }
    });
    Ok(json!({"name": text("accountName"), "handle": text("channelHandle"), "avatar": avatar, "channel_id": channel_id}))
}
