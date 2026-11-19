//! Signed-in lists read from YouTube's own web pages.
//!
//! YouTube's pages carry their first screen of content as JSON (`ytInitialData`). With the
//! user's session cookie the page is the personal one, so this gives the personal home feed,
//! the subscription feed, subscribed channels and history without yt-dlp, which is not
//! available on Android. Further screens come from the same continuation requests the
//! page itself makes when it is scrolled ([`more`]).

use std::time::Duration;

use serde_json::Value;

use crate::model::{ChannelCard, Chapter, CommentCard, Item, Page, PlaylistCard, PostCard, VideoCard, VideoPage};

/// A desktop browser: the desktop page embeds the data as plain JSON.
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Feed {
    Home,
    Subscriptions,
    Channels,
    History,
    Playlists,
    /// A destination page (Music, Sports, …): videos and playlists, shelves included.
    Explore,
    /// A channel's posts.
    Posts,
    /// Videos only (a playlist page, a channel tab).
    Videos,
}

impl Feed {
    fn url(self) -> &'static str {
        match self {
            Feed::Home => "https://www.youtube.com/",
            Feed::Subscriptions => "https://www.youtube.com/feed/subscriptions",
            Feed::Channels => "https://www.youtube.com/feed/channels",
            Feed::History => "https://www.youtube.com/feed/history",
            Feed::Playlists => "https://www.youtube.com/feed/playlists",
            Feed::Explore | Feed::Posts | Feed::Videos => "https://www.youtube.com/",
        }
    }

    /// Lists read from the signed-in user's own pages.
    fn personal(self) -> bool {
        matches!(self, Feed::Home | Feed::Subscriptions | Feed::Channels | Feed::History | Feed::Playlists)
    }
}

/// What is needed to ask YouTube for the next screen of a list.
#[derive(Clone, Debug)]
pub struct More {
    pub feed: Feed,
    token: String,
    /// The page's own client description, sent back unchanged.
    context: Value,
    /// Which signed-in account of the browser session the page was for.
    auth_user: String,
}

/// "Not interested" handles of the videos in a list: (video id, handle).
pub type Dismiss = Vec<(String, String)>;

fn keep(feed: Feed, items: &mut Vec<Item>) {
    match feed {
        Feed::Channels => items.retain(|i| matches!(i, Item::Channel(_))),
        Feed::Playlists => items.retain(|i| matches!(i, Item::Playlist(_))),
        Feed::Posts => items.retain(|i| matches!(i, Item::Post(_))),
        Feed::Explore => items.retain(|i| matches!(i, Item::Video(_) | Item::Playlist(_))),
        Feed::Home | Feed::Subscriptions => items.retain(|i| matches!(i, Item::Video(_) | Item::Post(_))),
        _ => items.retain(|i| matches!(i, Item::Video(_))),
    }
}

/// The continuation token of the list's last "load more" marker.
fn continuation(data: &Value) -> Option<String> {
    fn walk(v: &Value, depth: usize, last: &mut Option<String>) {
        if depth > 40 {
            return;
        }
        match v {
            Value::Array(a) => a.iter().for_each(|c| walk(c, depth + 1, last)),
            Value::Object(o) => {
                for (k, child) in o {
                    match k.as_str() {
                        // Not part of the list: comments, the sidebar, chips above the feed.
                        "guideRenderer" | "topbar" | "frameworkUpdates" | "header" | "engagementPanels" => {}
                        "continuationItemRenderer" => {
                            if let Some(t) = first(child, "token", 0).and_then(Value::as_str) {
                                *last = Some(t.to_string());
                            }
                        }
                        _ => walk(child, depth + 1, last),
                    }
                }
            }
            _ => {}
        }
    }
    let mut last = None;
    walk(data, 0, &mut last);
    last
}

/// A JSON value that follows `marker` in a page's source.
fn json_after(html: &str, marker: &str) -> Option<Value> {
    let rest = &html[html.find(marker)? + marker.len()..];
    serde_json::Deserializer::from_str(rest).into_iter::<Value>().next()?.ok()
}

/// `Authorization` for YouTube's page API: a hash of the time, the SAPISID cookie and the origin.
fn sapisid_hash(cookie: &str) -> Option<String> {
    use sha1::{Digest, Sha1};
    let value = |name: &str| {
        cookie.split(';').find_map(|p| {
            let (k, v) = p.trim().split_once('=')?;
            (k == name).then(|| v.to_string())
        })
    };
    let sapisid = value("SAPISID").or_else(|| value("__Secure-3PAPISID"))?;
    let now = time::OffsetDateTime::now_utc().unix_timestamp();
    let digest = Sha1::digest(format!("{now} {sapisid} https://www.youtube.com").as_bytes());
    Some(format!("SAPISIDHASH {now}_{}", hex::encode(digest)))
}

/// The next screen of a list. Returns its items and, if there is more, the next handle.
pub async fn more(http: &reqwest::Client, cookie: Option<&str>, m: &More) -> Result<(Page, Option<More>, Dismiss), String> {
    let body = serde_json::json!({"context": m.context, "continuation": m.token});
    let mut req = http
        .post("https://www.youtube.com/youtubei/v1/browse?prettyPrint=false")
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ORIGIN, "https://www.youtube.com")
        .header("X-Origin", "https://www.youtube.com")
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .timeout(Duration::from_secs(25))
        .json(&body);
    if let Some(c) = cookie {
        let auth = sapisid_hash(c).ok_or("the saved session has no SAPISID cookie")?;
        req = req
            .header(reqwest::header::COOKIE, c)
            .header(reqwest::header::AUTHORIZATION, auth)
            .header("X-Goog-AuthUser", m.auth_user.as_str());
    }
    let resp = req.send().await.map_err(|e| format!("could not load YouTube: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("YouTube answered HTTP {}", resp.status().as_u16()));
    }
    let data: Value = resp.json().await.map_err(|e| format!("unexpected answer from YouTube: {e}"))?;
    let mut items = collect_with(&data, m.feed == Feed::Explore);
    keep(m.feed, &mut items);
    let next = continuation(&data).filter(|_| !items.is_empty()).map(|token| More { token, ..m.clone() });
    Ok((Page { items, next: None }, next, dismiss_tokens(&data)))
}

/// Fetch one signed-in list. `cookie` is the session's `Cookie` header value.
/// The second value is the handle for the next screen, when there is one.
pub async fn fetch(http: &reqwest::Client, cookie: &str, feed: Feed) -> Result<(Page, Option<More>, Dismiss), String> {
    fetch_url(http, Some(cookie), feed.url(), feed).await
}

/// The signed-in home page, with its Shorts shelf (`[{id, title, views, thumb}]`).
pub async fn fetch_home(http: &reqwest::Client, cookie: &str) -> Result<(Page, Option<More>, Dismiss, Vec<Value>), String> {
    let html = get_page(http, Some(cookie), Feed::Home.url()).await?;
    let (page, more, dismiss, data) = read_list(&html, Feed::Home)?;
    Ok((page, more, dismiss, crate::shorts::shelf(&data)))
}

/// `Authorization` for a session's `Cookie` header value.
pub fn sapisid_auth(cookie: &str) -> Option<String> {
    sapisid_hash(cookie)
}

/// Every value under `key`, anywhere in `v`.
pub fn find_all_pub(v: &Value, key: &str, hit: &mut dyn FnMut(&Value)) {
    find_all(v, key, hit)
}

/// A page's source, signed in when `cookie` is given.
async fn get_page(http: &reqwest::Client, cookie: Option<&str>, url: &str) -> Result<String, String> {
    let mut req = http
        .get(url)
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .timeout(Duration::from_secs(25));
    if let Some(c) = cookie {
        req = req.header(reqwest::header::COOKIE, c);
    }
    let resp = req.send().await.map_err(|e| format!("could not load YouTube: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("YouTube answered HTTP {}", resp.status().as_u16()));
    }
    resp.text().await.map_err(|e| e.to_string())
}

/// Any YouTube list page (a feed, a channel tab, a playlist, a destination page), signed in
/// when `cookie` is given. Also returns the "Not interested" handles of its videos.
pub async fn fetch_url(http: &reqwest::Client, cookie: Option<&str>, url: &str, feed: Feed) -> Result<(Page, Option<More>, Dismiss), String> {
    let html = get_page(http, cookie, url).await?;
    let (page, more, dismiss, _) = read_list(&html, feed)?;
    Ok((page, more, dismiss))
}

/// The list in a page's source; the last value is the page's data, for callers that want more of it.
fn read_list(html: &str, feed: Feed) -> Result<(Page, Option<More>, Dismiss, Value), String> {
    let data = initial_data(html).ok_or("YouTube's page had no list data")?;
    if feed.personal() && !signed_in(&data) {
        return Err("YouTube did not accept the saved session. Sign in again.".into());
    }
    let mut items = collect_with(&data, feed == Feed::Explore);
    keep(feed, &mut items);
    if items.is_empty() {
        // Empty lists are normal (no subscriptions yet); the shapes present help tell that
        // apart from a page layout this reader does not know.
        let has = |k: &str| html.contains(k);
        tracing::info!(
            "web list {feed:?} is empty ({} bytes; videoRenderer: {}, lockupViewModel: {}, channelRenderer: {}, messageRenderer: {})",
            html.len(), has("\"videoRenderer\""), has("\"lockupViewModel\""), has("\"channelRenderer\""), has("\"messageRenderer\"")
        );
    }
    let more = match (continuation(&data), json_after(html, "\"INNERTUBE_CONTEXT\":")) {
        (Some(token), Some(context)) if !items.is_empty() => Some(More {
            feed,
            token,
            context,
            auth_user: json_after(html, "\"SESSION_INDEX\":")
                .and_then(|v| v.as_str().map(str::to_string))
                .unwrap_or_else(|| "0".into()),
        }),
        _ => None,
    };
    let dismiss = dismiss_tokens(&data);
    Ok((Page { items, next: None }, more, dismiss, data))
}

/// A playlist read from its page (works for the private ones too, such as Watch later and
/// Liked videos, when signed in): its name, first videos and the handle for more.
pub async fn playlist(http: &reqwest::Client, cookie: Option<&str>, id: &str) -> Result<(PlaylistCard, Page, Option<More>), String> {
    let html = get_page(http, cookie, &format!("https://www.youtube.com/playlist?list={id}")).await?;
    let (page, more, _, data) = read_list(&html, Feed::Videos)?;
    let name = first(&data, "playlistMetadataRenderer", 0)
        .and_then(|m| m.get("title"))
        .and_then(|t| t.as_str().map(str::to_string).or_else(|| text(t)))
        .or_else(|| first(&data, "pageHeaderRenderer", 0).and_then(|h| h.get("pageTitle")).and_then(|t| t.as_str().map(str::to_string)))
        .unwrap_or_else(|| match id {
            "WL" => "Watch later".into(),
            "LL" => "Liked videos".into(),
            _ => "Playlist".into(),
        });
    Ok((PlaylistCard { id: id.to_string(), name, thumbnail: None, channel_name: None, video_count: None }, page, more))
}

/// Details and related videos for one video, read from its watch page. Works signed out too
/// (`cookie` = `None`). Used when the regular extractor cannot read a video's page.
///
/// `personal`, when given, receives what the same page says for the signed-in user.
pub async fn watch(http: &reqwest::Client, cookie: Option<&str>, id: &str, personal: Option<&mut Personal>) -> Result<VideoPage, String> {
    let mut req = http
        .get(format!("https://www.youtube.com/watch?v={id}"))
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .timeout(Duration::from_secs(25));
    if let Some(c) = cookie {
        req = req.header(reqwest::header::COOKIE, c);
    }
    let resp = req.send().await.map_err(|e| format!("could not load YouTube: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("YouTube answered HTTP {}", resp.status().as_u16()));
    }
    let html = resp.text().await.map_err(|e| e.to_string())?;
    let data = initial_data(&html).ok_or("YouTube's page had no video data")?;
    if let Some(found) = personal {
        found.rating = rating_of(&data);
        found.chat = live_chat_token(&data);
        let player = json_after(&html, "ytInitialPlayerResponse = ");
        found.storyboard = player.as_ref().and_then(storyboard_spec);
        found.live = player
            .as_ref()
            .and_then(|p| p.pointer("/videoDetails/isLive"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        found.tracking = json_after(&html, "ytInitialPlayerResponse = ").and_then(|p| {
            let url = |key: &str| p.pointer(&format!("/playbackTracking/{key}/baseUrl"))?.as_str().map(str::to_string);
            Some((url("videostatsPlaybackUrl")?, url("videostatsWatchtimeUrl")?))
        });
    }
    watch_page(&data, id).ok_or_else(|| "YouTube's page did not describe this video".to_string())
}

/// What a watch page holds for the signed-in user.
#[derive(Clone, Debug, Default)]
pub struct Personal {
    /// "like", "dislike" or "none".
    pub rating: Option<String>,
    /// The two addresses YouTube's own player reports playback to (start, watch time).
    pub tracking: Option<(String, String)>,
    /// The handle that opens a live stream's chat.
    pub chat: Option<String>,
    /// Where the seek bar's preview pictures are (YouTube's "storyboard" spec).
    pub storyboard: Option<String>,
    pub live: bool,
}

/// A random id for one playback, as YouTube's player makes them.
pub fn playback_id() -> String {
    use rand::Rng;
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut rng = rand::rng();
    (0..16).map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char).collect()
}

/// Tell YouTube the video was watched, the way its own player does, so it appears in the
/// account's history and the recommendations take it into account. `seconds` is how much
/// of it was played.
///
/// `cpn` names one playback (see [`playback_id`]); `from`..`to` is the stretch of the video,
/// in seconds, played since the last report. The first report of a playback also tells
/// YouTube that it started.
pub async fn mark_watched(
    http: &reqwest::Client,
    cookie: &str,
    tracking: &(String, String),
    cpn: &str,
    from: u32,
    to: u32,
    first: bool,
) -> Result<(), String> {
    let to = to.max(from + 1);
    let mut reports = Vec::new();
    if first {
        reports.push(format!("{}&ver=2&cpn={cpn}", tracking.0));
    }
    reports.push(format!("{}&ver=2&cpn={cpn}&cmt={to}&el=detailpage&st={from}&et={to}", tracking.1));
    for url in reports {
        let resp = http
            .get(&url)
            .header(reqwest::header::USER_AGENT, USER_AGENT)
            .header(reqwest::header::COOKIE, cookie)
            .header(reqwest::header::REFERER, "https://www.youtube.com/")
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| format!("could not reach YouTube: {e}"))?;
        if !resp.status().is_success() {
            return Err(format!("YouTube answered HTTP {}", resp.status().as_u16()));
        }
    }
    Ok(())
}

// ------------------------------------------------------------------ likes and subscriptions

/// The web client YouTube's action endpoints are called as.
const WEB_VERSION: &str = "2.20261111.01.00";

/// A value from the page's entity store (`frameworkUpdates`), e.g. the like state.
fn entity(data: &Value, entity: &str, field: &str) -> Option<Value> {
    let mut found = None;
    find_key(data.get("frameworkUpdates")?, entity, 0, &mut |v| {
        if found.is_none() {
            found = v.get(field).cloned();
        }
    });
    found
}

/// What the signed-in user said about the video: "like", "dislike" or "none".
fn rating_of(data: &Value) -> Option<String> {
    Some(
        match entity(data, "likeStatusEntity", "likeStatus")?.as_str()? {
            "LIKE" => "like",
            "DISLIKE" => "dislike",
            _ => "none",
        }
        .to_string(),
    )
}

/// One signed-in action: a POST to `youtubei/v1/<path>`.
async fn act(http: &reqwest::Client, cookie: &str, path: &str, body: Value) -> Result<(), String> {
    post_json(http, cookie, path, body).await.map(|_| ())
}

/// Like a video, dislike it, or take the rating back (`rating`: "like", "dislike", "none").
pub async fn rate(http: &reqwest::Client, cookie: &str, id: &str, rating: &str) -> Result<(), String> {
    let path = match rating {
        "like" => "like/like",
        "dislike" => "like/dislike",
        "none" => "like/removelike",
        other => return Err(format!("unknown rating {other}")),
    };
    act(http, cookie, path, serde_json::json!({"target": {"videoId": id}})).await
}

/// Subscribe to a channel or leave it.
pub async fn subscribe(http: &reqwest::Client, cookie: &str, channel: &str, on: bool) -> Result<(), String> {
    let path = if on { "subscription/subscribe" } else { "subscription/unsubscribe" };
    act(http, cookie, path, serde_json::json!({"channelIds": [channel]})).await
}

/// Is the signed-in user subscribed to this channel? `None` when the page does not say.
pub async fn subscribed(http: &reqwest::Client, cookie: &str, channel: &str) -> Result<Option<bool>, String> {
    let resp = http
        .get(format!("https://www.youtube.com/channel/{channel}"))
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .header(reqwest::header::COOKIE, cookie)
        .timeout(Duration::from_secs(25))
        .send()
        .await
        .map_err(|e| format!("could not load YouTube: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("YouTube answered HTTP {}", resp.status().as_u16()));
    }
    let html = resp.text().await.map_err(|e| e.to_string())?;
    let data = initial_data(&html).ok_or("YouTube's page had no channel data")?;
    Ok(entity(&data, "subscriptionStateEntity", "subscribed").and_then(|v| v.as_bool()))
}

/// What is needed to ask YouTube for a page of comments.
#[derive(Clone, Debug)]
pub struct CommentsMore {
    token: String,
    context: Value,
    auth_user: String,
}

/// "1.2K" → 1200.
fn count(s: &str) -> Option<u32> {
    let s = s.trim().replace(',', "");
    let (number, scale) = match s.chars().last()? {
        'K' | 'k' => (&s[..s.len() - 1], 1e3),
        'M' | 'm' => (&s[..s.len() - 1], 1e6),
        'B' | 'b' => (&s[..s.len() - 1], 1e9),
        _ => (s.as_str(), 1.0),
    };
    number.trim().parse::<f64>().ok().map(|n| (n * scale) as u32)
}

/// The handle that opens a watch page's comment section.
fn comments_token(data: &Value) -> Option<String> {
    fn walk<'a>(v: &'a Value, depth: usize) -> Option<&'a Value> {
        if depth > 30 {
            return None;
        }
        match v {
            Value::Array(a) => a.iter().find_map(|c| walk(c, depth + 1)),
            Value::Object(o) => {
                if let Some(section) = o.get("itemSectionRenderer") {
                    if section.get("sectionIdentifier").and_then(Value::as_str) == Some("comment-item-section") {
                        return Some(section);
                    }
                }
                o.values().find_map(|c| walk(c, depth + 1))
            }
            _ => None,
        }
    }
    let section = walk(data, 0)?;
    first(section.get("contents")?, "token", 0)?.as_str().map(str::to_string)
}

/// Comments out of a page API answer: top-level comments in order, plus the handle for
/// the next page (reply threads have their own handles, which are not followed).
fn read_comments(data: &Value) -> (Vec<CommentCard>, Option<String>) {
    fn walk(v: &Value, depth: usize, out: &mut Vec<CommentCard>, next: &mut Option<String>) {
        if depth > 40 {
            return;
        }
        match v {
            Value::Array(a) => a.iter().for_each(|c| walk(c, depth + 1, out, next)),
            Value::Object(o) => {
                for (k, child) in o {
                    match k.as_str() {
                        "commentEntityPayload" => {
                            let props = child.get("properties");
                            let author = child.get("author");
                            let toolbar = child.get("toolbar");
                            let get = |from: Option<&Value>, key: &str| {
                                from.and_then(|f| f.get(key)).and_then(Value::as_str).map(str::to_string)
                            };
                            // Replies arrive in their own threads; only top-level comments here.
                            let level = props.and_then(|p| p.get("replyLevel")).and_then(Value::as_u64).unwrap_or(0);
                            let id = get(props, "commentId");
                            let body = props.and_then(|p| p.get("content")).and_then(text);
                            if let (0, Some(id), Some(body)) = (level, id, body) {
                                out.push(CommentCard {
                                    id,
                                    author: get(author, "displayName"),
                                    author_id: get(author, "channelId"),
                                    avatar: get(author, "avatarThumbnailUrl"),
                                    text: body,
                                    published: get(props, "publishedTime").unwrap_or_default(),
                                    likes: get(toolbar, "likeCountNotliked").as_deref().and_then(count),
                                    replies: get(toolbar, "replyCount").as_deref().and_then(count).unwrap_or(0),
                                    pinned: false,
                                    by_owner: author.and_then(|a| a.get("isCreator")).and_then(Value::as_bool).unwrap_or(false),
                                    hearted: false,
                                });
                            }
                        }
                        // The handles inside a thread load its replies, not the next page.
                        "commentRepliesRenderer" => {}
                        "continuationItemRenderer" => {
                            if let Some(t) = first(child, "token", 0).and_then(Value::as_str) {
                                *next = Some(t.to_string());
                            }
                        }
                        _ => walk(child, depth + 1, out, next),
                    }
                }
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    let mut next = None;
    walk(data, 0, &mut out, &mut next);
    (out, next)
}

/// One page of comments from YouTube's page API.
pub async fn comments_more(
    http: &reqwest::Client,
    cookie: Option<&str>,
    m: &CommentsMore,
) -> Result<(Vec<CommentCard>, Option<CommentsMore>), String> {
    let body = serde_json::json!({"context": m.context, "continuation": m.token});
    let mut req = http
        .post("https://www.youtube.com/youtubei/v1/next?prettyPrint=false")
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ORIGIN, "https://www.youtube.com")
        .header("X-Origin", "https://www.youtube.com")
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .timeout(Duration::from_secs(25))
        .json(&body);
    if let Some(c) = cookie {
        req = req.header(reqwest::header::COOKIE, c).header("X-Goog-AuthUser", m.auth_user.as_str());
        if let Some(auth) = sapisid_hash(c) {
            req = req.header(reqwest::header::AUTHORIZATION, auth);
        }
    }
    let resp = req.send().await.map_err(|e| format!("could not load YouTube: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("YouTube answered HTTP {}", resp.status().as_u16()));
    }
    let data: Value = resp.json().await.map_err(|e| format!("unexpected answer from YouTube: {e}"))?;
    let (items, token) = read_comments(&data);
    let next = token.filter(|_| !items.is_empty()).map(|token| CommentsMore { token, ..m.clone() });
    Ok((items, next))
}

/// The first page of a video's comments, read from its watch page and YouTube's page API.
/// Used when the regular extractor cannot read the video's page.
pub async fn comments(
    http: &reqwest::Client,
    cookie: Option<&str>,
    id: &str,
) -> Result<(Vec<CommentCard>, Option<CommentsMore>), String> {
    let mut req = http
        .get(format!("https://www.youtube.com/watch?v={id}"))
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .timeout(Duration::from_secs(25));
    if let Some(c) = cookie {
        req = req.header(reqwest::header::COOKIE, c);
    }
    let resp = req.send().await.map_err(|e| format!("could not load YouTube: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("YouTube answered HTTP {}", resp.status().as_u16()));
    }
    let html = resp.text().await.map_err(|e| e.to_string())?;
    let data = initial_data(&html).ok_or("YouTube's page had no video data")?;
    // A page without a comment section has comments turned off: that is an empty list.
    let Some(token) = comments_token(&data) else {
        return Ok((Vec::new(), None));
    };
    let context = json_after(&html, "\"INNERTUBE_CONTEXT\":").ok_or("YouTube's page had no client description")?;
    let auth_user = json_after(&html, "\"SESSION_INDEX\":")
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "0".into());
    comments_more(http, cookie, &CommentsMore { token, context, auth_user }).await
}

/// First value stored under `key`, searching depth-first.
fn first<'a>(v: &'a Value, key: &str, depth: usize) -> Option<&'a Value> {
    if depth > 14 {
        return None;
    }
    match v {
        Value::Object(o) => o.get(key).or_else(|| o.values().find_map(|c| first(c, key, depth + 1))),
        Value::Array(a) => a.iter().find_map(|c| first(c, key, depth + 1)),
        _ => None,
    }
}

/// The like count on a watch page: the exact number in the button's spoken label
/// ("like this video along with 73,123 other people"), else its short title ("73K").
fn like_count(data: &Value) -> Option<u32> {
    let mut found = None;
    find_all(data, "likeButtonViewModel", &mut |l| {
        if found.is_some() {
            return;
        }
        let mut label = None;
        find_all(l, "accessibilityText", &mut |t| {
            if label.is_none() {
                label = t.as_str().map(str::to_string);
            }
        });
        if let Some(digits) = label.as_deref().and_then(|t| t.split_whitespace().find(|w| w.chars().next().is_some_and(|c| c.is_ascii_digit()))) {
            found = count(digits);
        }
        if found.is_none() {
            find_all(l, "title", &mut |t| {
                if found.is_none() {
                    found = t.as_str().and_then(count);
                }
            });
        }
    });
    found
}

fn watch_page(data: &Value, id: &str) -> Option<VideoPage> {
    let primary = first(data, "videoPrimaryInfoRenderer", 0)?;
    let secondary = first(data, "videoSecondaryInfoRenderer", 0);
    let owner = secondary
        .and_then(|s| first(s, "videoOwnerRenderer", 0))
        .or_else(|| first(data, "videoOwnerRenderer", 0));
    let description = secondary
        .and_then(|s| s.get("attributedDescription").or_else(|| s.get("description")))
        .and_then(text)
        .unwrap_or_default();
    let views = first(primary, "videoViewCountRenderer", 0)
        .and_then(|r| r.get("shortViewCount").or_else(|| r.get("viewCount")))
        .and_then(text);
    let date = primary.get("dateText").and_then(text);
    let about: Vec<String> = [views, date].into_iter().flatten().collect();
    let related = first(data, "secondaryResults", 0)
        .map(collect)
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !matches!(i, Item::Video(v) if v.id == id))
        .collect();
    Some(VideoPage {
        subscribers: owner.and_then(|o| o.get("subscriberCountText")).and_then(text),
        chapters: chapters(data),
        video: VideoCard {
            id: id.to_string(),
            title: primary.get("title").and_then(text)?,
            channel_id: owner.and_then(browse_id),
            channel_name: owner.and_then(|o| o.get("title")).and_then(text),
            channel_avatar: owner.and_then(|o| thumb(o.get("thumbnail").and_then(|t| t.get("thumbnails")))),
            duration_s: None,
            thumbnail: Some(format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg")),
            views: None,
            published: (!about.is_empty()).then(|| about.join(" · ")),
            is_live: false,
            watched: None,
        },
        description,
        likes: like_count(data),
        related: Page { items: related, next: None },
    })
}

/// The JSON assigned to `ytInitialData` in a page.
fn initial_data(html: &str) -> Option<Value> {
    for marker in ["var ytInitialData = ", "window[\"ytInitialData\"] = ", "ytInitialData = "] {
        if let Some(at) = html.find(marker) {
            let rest = &html[at + marker.len()..];
            // Parse one JSON value and ignore whatever follows it.
            let mut stream = serde_json::Deserializer::from_str(rest).into_iter::<Value>();
            if let Some(Ok(v)) = stream.next() {
                if v.is_object() {
                    return Some(v);
                }
            }
        }
    }
    None
}

fn signed_in(data: &Value) -> bool {
    let mut found = None;
    find_key(data, "loggedOut", 0, &mut |v| found = v.as_bool());
    // Pages that do not say are trusted; the caller only asks when it has a session.
    found != Some(true)
}

/// How far the user got in a listed video, in percent: YouTube attaches it to the
/// thumbnail (the red line) of videos the account has started.
fn watched_percent(card: &Value) -> Option<u8> {
    fn scan(v: &Value, depth: usize, best: &mut Option<f64>) {
        if depth > 16 {
            return;
        }
        match v {
            Value::Object(o) => {
                for (k, child) in o {
                    if k == "percentDurationWatched" || k == "startPercent" {
                        if let Some(p) = child.as_f64() {
                            *best = Some(best.unwrap_or(0.0).max(p));
                        }
                    } else {
                        scan(child, depth + 1, best);
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|c| scan(c, depth + 1, best)),
            _ => {}
        }
    }
    let mut best = None;
    scan(card, 0, &mut best);
    best.map(|p| p.clamp(0.0, 100.0).round() as u8)
}

fn find_key(v: &Value, key: &str, depth: usize, hit: &mut dyn FnMut(&Value)) {
    if depth > 6 {
        return;
    }
    match v {
        Value::Object(o) => {
            for (k, child) in o {
                if k == key {
                    hit(child);
                } else {
                    find_key(child, key, depth + 1, hit);
                }
            }
        }
        Value::Array(a) => a.iter().for_each(|c| find_key(c, key, depth + 1, hit)),
        _ => {}
    }
}

/// Text of YouTube's two text shapes: `{"simpleText": …}` and `{"runs": [{"text": …}]}`.
fn text(v: &Value) -> Option<String> {
    if let Some(s) = v.get("simpleText").and_then(Value::as_str) {
        return Some(s.to_string());
    }
    if let Some(s) = v.get("content").and_then(Value::as_str) {
        return Some(s.to_string());
    }
    let runs = v.get("runs")?.as_array()?;
    let s: String = runs.iter().filter_map(|r| r.get("text").and_then(Value::as_str)).collect();
    (!s.is_empty()).then_some(s)
}

fn first_text(v: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|k| v.get(*k).and_then(text))
}

/// The widest thumbnail up to about 500 px in a `thumbnails`/`sources` list.
fn thumb(list: Option<&Value>) -> Option<String> {
    let list = list?.as_array()?;
    let pick = list
        .iter()
        .filter(|t| t.get("width").and_then(Value::as_u64).unwrap_or(0) <= 520)
        .max_by_key(|t| t.get("width").and_then(Value::as_u64).unwrap_or(0))
        .or_else(|| list.first())?;
    let url = pick.get("url")?.as_str()?;
    Some(if url.starts_with("//") { format!("https:{url}") } else { url.to_string() })
}

/// "1:02:03" → seconds.
fn seconds(s: &str) -> Option<u32> {
    let mut total = 0u32;
    let mut parts = 0;
    for p in s.trim().split(':') {
        total = total.checked_mul(60)?.checked_add(p.trim().parse().ok()?)?;
        parts += 1;
    }
    (parts >= 2).then_some(total)
}

fn browse_id(v: &Value) -> Option<String> {
    let mut id = None;
    find_key(v, "browseId", 0, &mut |b| {
        if id.is_none() {
            id = b.as_str().filter(|s| s.starts_with("UC")).map(str::to_string);
        }
    });
    id
}

/// The classic video shapes (`videoRenderer`, `compactVideoRenderer`, …).
fn video(r: &Value) -> Option<VideoCard> {
    let id = r.get("videoId")?.as_str()?.to_string();
    let title = first_text(r, &["title", "headline"])?;
    let byline = ["ownerText", "shortBylineText", "longBylineText"].iter().find_map(|k| r.get(*k));
    let length = first_text(r, &["lengthText"]);
    let about: Vec<String> = [
        first_text(r, &["shortViewCountText", "viewCountText"]),
        first_text(r, &["publishedTimeText"]),
    ]
    .into_iter()
    .flatten()
    .collect();
    let live = length.is_none()
        && r.get("badges").is_some_and(|b| b.to_string().contains("LIVE"));
    Some(VideoCard {
        thumbnail: thumb(r.get("thumbnail").and_then(|t| t.get("thumbnails")))
            .or_else(|| Some(format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg"))),
        channel_id: byline.and_then(browse_id),
        channel_name: byline.and_then(text),
        channel_avatar: first(r, "channelThumbnailWithLinkRenderer", 0)
            .and_then(|c| c.get("thumbnail"))
            .or_else(|| r.get("channelThumbnail"))
            .and_then(|t| thumb(t.get("thumbnails"))),
        duration_s: length.as_deref().and_then(seconds),
        views: None,
        published: (!about.is_empty()).then(|| about.join(" · ")),
        is_live: live,
        watched: watched_percent(r),
        id,
        title,
    })
}

/// The newer shape used for videos and playlists on most pages.
fn lockup(r: &Value) -> Option<Item> {
    let id = r.get("contentId")?.as_str()?.to_string();
    let kind = r.get("contentType").and_then(Value::as_str).unwrap_or("");
    let meta = r.get("metadata")?.get("lockupMetadataViewModel")?;
    let title = meta.get("title").and_then(text)?;
    let image = r
        .get("contentImage")
        .and_then(|c| c.get("thumbnailViewModel").or_else(|| c.get("collectionThumbnailViewModel")));
    let mut sources = None;
    if let Some(i) = image {
        find_key(i, "sources", 0, &mut |s| {
            if sources.is_none() {
                sources = Some(s.clone());
            }
        });
    }
    // Metadata rows: channel name first, then "views · age".
    let mut parts: Vec<String> = Vec::new();
    find_key(meta, "metadataParts", 0, &mut |p| {
        if let Some(list) = p.as_array() {
            parts.extend(list.iter().filter_map(|x| x.get("text").and_then(text)));
        }
    });
    let channel_name = parts.first().cloned();
    // Stations are YouTube Music mixes: lists of songs, opened like playlists.
    if kind.ends_with("PLAYLIST") || kind.ends_with("PODCAST") || kind.ends_with("ALBUM") || kind.ends_with("STATION") {
        return Some(Item::Playlist(PlaylistCard {
            id,
            name: title,
            thumbnail: thumb(sources.as_ref()),
            channel_name,
            video_count: None,
        }));
    }
    if !kind.ends_with("VIDEO") || id.len() != 11 {
        return None;
    }
    // The duration is the text of a badge over the thumbnail.
    let mut duration = None;
    let mut live = false;
    if let Some(i) = image {
        find_key(i, "text", 0, &mut |t| {
            if let Some(s) = t.as_str() {
                if duration.is_none() {
                    duration = seconds(s);
                }
                live |= s.eq_ignore_ascii_case("live");
            }
        });
    }
    Some(Item::Video(VideoCard {
        thumbnail: thumb(sources.as_ref()).or_else(|| Some(format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg"))),
        channel_id: browse_id(meta),
        channel_name,
        channel_avatar: first(meta, "avatarViewModel", 0)
            .and_then(|a| a.get("image"))
            .and_then(|i| thumb(i.get("sources"))),
        duration_s: duration,
        views: None,
        published: (parts.len() > 1).then(|| parts[1..].join(" · ")),
        is_live: live && duration.is_none(),
        watched: watched_percent(r),
        id,
        title,
    }))
}

fn channel(r: &Value) -> Option<ChannelCard> {
    let id = r.get("channelId")?.as_str()?.to_string();
    Some(ChannelCard {
        name: first_text(r, &["title"])?,
        avatar: thumb(r.get("thumbnail").and_then(|t| t.get("thumbnails"))),
        subscribers: None,
        id,
    })
}

/// Every video, channel and playlist in a page's data, in page order, without ads or Shorts.
pub fn collect(data: &Value) -> Vec<Item> {
    collect_with(data, false)
}

/// As [`collect`]; `shelves` also takes the videos inside shelves (destination pages are
/// made of them; on the home page they are Shorts or news, which are left out).
pub fn collect_with(data: &Value, shelves: bool) -> Vec<Item> {
    let mut out = Vec::new();
    let mut seen = std::collections::HashSet::new();
    walk(data, 0, shelves, &mut out, &mut seen);
    out
}

fn walk(v: &Value, depth: usize, shelves: bool, out: &mut Vec<Item>, seen: &mut std::collections::HashSet<String>) {
    if depth > 48 {
        return;
    }
    match v {
        Value::Array(a) => a.iter().for_each(|c| walk(c, depth + 1, shelves, out, seen)),
        Value::Object(o) => {
            for (k, child) in o {
                let item = match k.as_str() {
                    "richShelfRenderer" if shelves => {
                        walk(child, depth + 1, shelves, out, seen);
                        continue;
                    }
                    // Ads, Shorts shelves and the sidebar/guide are not list content.
                    "adSlotRenderer" | "promotedSparklesWebRenderer" | "promotedVideoRenderer"
                    | "reelShelfRenderer" | "richShelfRenderer" | "shortsLockupViewModel"
                    | "reelItemRenderer" | "guideRenderer" | "topbar" | "frameworkUpdates"
                    | "playerOverlays" | "microformat" => continue,
                    "videoRenderer" | "compactVideoRenderer" | "gridVideoRenderer"
                    | "videoWithContextRenderer" | "playlistVideoRenderer" | "videoCardRenderer" => video(child).map(Item::Video),
                    "lockupViewModel" => lockup(child),
                    "channelRenderer" | "gridChannelRenderer" => channel(child).map(Item::Channel),
                    "backstagePostRenderer" => post(child).map(Item::Post),
                    _ => {
                        walk(child, depth + 1, shelves, out, seen);
                        continue;
                    }
                };
                if let Some(item) = item {
                    let key = match &item {
                        Item::Video(v) => format!("v{}", v.id),
                        Item::Channel(c) => format!("c{}", c.id),
                        Item::Playlist(p) => format!("p{}", p.id),
                        Item::Post(p) => format!("o{}", p.id),
                    };
                    if seen.insert(key) {
                        out.push(item);
                    }
                }
            }
        }
        _ => {}
    }
}


// ------------------------------------------------------------------ posts, chapters, live

/// The widest picture up to `max` px in a `thumbnails` list.
fn picture(list: Option<&Value>, max: u64) -> Option<String> {
    let list = list?.as_array()?;
    let pick = list
        .iter()
        .filter(|t| t.get("width").and_then(Value::as_u64).unwrap_or(0) <= max)
        .max_by_key(|t| t.get("width").and_then(Value::as_u64).unwrap_or(0))
        .or_else(|| list.last())?;
    let url = pick.get("url")?.as_str()?;
    Some(if url.starts_with("//") { format!("https:{url}") } else { url.to_string() })
}

/// A channel post (`backstagePostRenderer`).
fn post(r: &Value) -> Option<PostCard> {
    let id = r.get("postId")?.as_str()?.to_string();
    let attachment = r.get("backstageAttachment");
    let mut images = Vec::new();
    if let Some(a) = attachment {
        find_key(a, "backstageImageRenderer", 0, &mut |img| {
            if let Some(u) = picture(img.get("image").and_then(|i| i.get("thumbnails")), 1080) {
                images.push(u);
            }
        });
    }
    Some(PostCard {
        id,
        author: r.get("authorText").and_then(text),
        author_id: r.get("authorEndpoint").and_then(browse_id),
        author_avatar: thumb(r.get("authorThumbnail").and_then(|t| t.get("thumbnails"))),
        text: r.get("contentText").and_then(text).unwrap_or_default(),
        published: r.get("publishedTimeText").and_then(text),
        likes: r.get("voteCount").and_then(text).filter(|s| !s.trim().is_empty()),
        images,
        video: attachment.and_then(|a| first(a, "videoRenderer", 0)).and_then(video),
    })
}

/// A watch page's chapters, in order.
fn chapters(data: &Value) -> Vec<Chapter> {
    let mut out: Vec<Chapter> = Vec::new();
    fn all<'a>(v: &'a Value, key: &str, depth: usize, out: &mut Vec<&'a Value>) {
        if depth > 60 {
            return;
        }
        match v {
            Value::Object(o) => {
                for (k, c) in o {
                    if k == key {
                        out.push(c);
                    } else {
                        all(c, key, depth + 1, out);
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|c| all(c, key, depth + 1, out)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    all(data, "chapterRenderer", 0, &mut found);
    for c in found {
        if let (Some(title), Some(ms)) = (c.get("title").and_then(text), c.get("timeRangeStartMillis").and_then(Value::as_u64)) {
            out.push(Chapter { title, start: (ms / 1000) as u32 });
        }
    }
    if out.is_empty() {
        // Older pages: the chapter list of the description panel.
        let mut found = Vec::new();
        all(data, "macroMarkersListItemRenderer", 0, &mut found);
        for m in found {
            let start = m.pointer("/onTap/watchEndpoint/startTimeSeconds").and_then(Value::as_u64);
            if let (Some(title), Some(start)) = (m.get("title").and_then(text), start) {
                out.push(Chapter { title, start: start as u32 });
            }
        }
    }
    out.sort_by_key(|c| c.start);
    out.dedup_by_key(|c| c.start);
    out
}

/// The handle that opens a live stream's chat on its watch page.
fn live_chat_token(data: &Value) -> Option<String> {
    let chat = first(data, "liveChatRenderer", 0)?;
    first(chat.get("continuations")?, "continuation", 0)?.as_str().map(str::to_string)
}

/// YouTube's storyboard spec from a player answer.
fn storyboard_spec(player: &Value) -> Option<String> {
    let s = player.get("storyboards")?;
    s.pointer("/playerStoryboardSpecRenderer/spec")
        .or_else(|| s.pointer("/playerLiveStoryboardSpecRenderer/spec"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// The "Not interested" handle of each video in a list that offers one.
pub fn dismiss_tokens(data: &Value) -> Dismiss {
    /// The feedback handle of the menu entry called "Not interested" inside one card.
    fn token(card: &Value, depth: usize) -> Option<String> {
        if depth > 30 {
            return None;
        }
        match card {
            Value::Object(o) => {
                for key in ["listItemViewModel", "menuServiceItemRenderer"] {
                    if let Some(entry) = o.get(key) {
                        let label = entry.get("title").and_then(text).or_else(|| entry.get("text").and_then(text));
                        if label.as_deref() == Some("Not interested") {
                            return first(entry, "feedbackToken", 0).and_then(Value::as_str).map(str::to_string);
                        }
                    }
                }
                o.values().find_map(|c| token(c, depth + 1))
            }
            Value::Array(a) => a.iter().find_map(|c| token(c, depth + 1)),
            _ => None,
        }
    }
    fn walk(v: &Value, depth: usize, out: &mut Dismiss) {
        if depth > 40 {
            return;
        }
        match v {
            Value::Object(o) => {
                for (k, c) in o {
                    let id = match k.as_str() {
                        "lockupViewModel" => c.get("contentId").and_then(Value::as_str),
                        "videoRenderer" | "richItemRenderer" => c.get("videoId").and_then(Value::as_str),
                        _ => None,
                    };
                    match id {
                        Some(id) => {
                            if let Some(t) = token(c, 0) {
                                out.push((id.to_string(), t));
                            }
                        }
                        None => walk(c, depth + 1, out),
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|c| walk(c, depth + 1, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(data, 0, &mut out);
    out
}

// ------------------------------------------------------------------ signed-in page API

/// A signed-in POST to `youtubei/v1/<path>`, answering with YouTube's JSON.
pub async fn post_json(http: &reqwest::Client, cookie: &str, path: &str, mut body: Value) -> Result<Value, String> {
    let auth = sapisid_hash(cookie).ok_or("the saved session has no SAPISID cookie")?;
    body["context"] = serde_json::json!({"client": {"clientName": "WEB", "clientVersion": WEB_VERSION, "hl": "en", "gl": "US"}});
    let resp = http
        .post(format!("https://www.youtube.com/youtubei/v1/{path}?prettyPrint=false"))
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::COOKIE, cookie)
        .header(reqwest::header::AUTHORIZATION, auth)
        .header(reqwest::header::ORIGIN, "https://www.youtube.com")
        .header("X-Origin", "https://www.youtube.com")
        .header("X-Goog-AuthUser", "0")
        .header("X-Youtube-Client-Name", "1")
        .header("X-Youtube-Client-Version", WEB_VERSION)
        .timeout(Duration::from_secs(20))
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("could not reach YouTube: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("YouTube answered HTTP {}", resp.status().as_u16()));
    }
    resp.json().await.map_err(|e| format!("unexpected answer from YouTube: {e}"))
}

/// The account's notifications (the bell), as videos: the title is the notification's text.
pub async fn notifications(http: &reqwest::Client, cookie: &str) -> Result<Page, String> {
    let data = post_json(
        http,
        cookie,
        "notification/get_notification_menu",
        serde_json::json!({"notificationsMenuRequestType": "NOTIFICATIONS_MENU_REQUEST_TYPE_INBOX"}),
    )
    .await?;
    let mut items = Vec::new();
    let mut seen = std::collections::HashSet::new();
    fn all<'a>(v: &'a Value, depth: usize, out: &mut Vec<&'a Value>) {
        if depth > 40 {
            return;
        }
        match v {
            Value::Object(o) => {
                for (k, c) in o {
                    if k == "notificationRenderer" {
                        out.push(c);
                    } else {
                        all(c, depth + 1, out);
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|c| all(c, depth + 1, out)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    all(&data, 0, &mut found);
    for n in found {
        let Some(id) = first(n.get("navigationEndpoint").unwrap_or(&Value::Null), "videoId", 0).and_then(Value::as_str) else {
            continue;
        };
        if id.len() != 11 || !seen.insert(n.get("notificationId").map(|v| v.to_string()).unwrap_or_else(|| id.to_string())) {
            continue;
        }
        items.push(Item::Video(VideoCard {
            id: id.to_string(),
            title: n.get("shortMessage").and_then(text).unwrap_or_default(),
            channel_id: None,
            channel_name: None,
            channel_avatar: thumb(n.get("thumbnail").and_then(|t| t.get("thumbnails"))),
            duration_s: None,
            thumbnail: thumb(n.get("videoThumbnail").and_then(|t| t.get("thumbnails")))
                .or_else(|| Some(format!("https://i.ytimg.com/vi/{id}/mqdefault.jpg"))),
            views: None,
            published: n.get("sentTimeText").and_then(text),
            is_live: false,
            // Unread ones are marked by the app (see the UI); read state is not kept here.
            watched: None,
        }));
    }
    Ok(Page { items, next: None })
}

/// Is the user subscribed to a channel, and the choices of its bell (all, personalized,
/// none) with the handle that sets each one.
/// The subscription state of `channel` itself: a channel page also holds the states of the
/// channels it features, so the entity is picked by its key (protobuf: field 2 = the channel
/// id, then the state kind), not by order.
fn subscription_of(data: &Value, channel: &str) -> Option<bool> {
    use base64::Engine;
    let mut raw = vec![0x12u8, channel.len() as u8];
    raw.extend_from_slice(channel.as_bytes());
    raw.extend_from_slice(&[0x20, 0x33, 0x28, 0x01]);
    let key = base64::engine::general_purpose::STANDARD.encode(&raw);
    let key_url = key.replace('=', "%3D");
    let mutations = data.pointer("/frameworkUpdates/entityBatchUpdate/mutations")?.as_array()?;
    mutations.iter().find_map(|m| {
        let k = m.get("entityKey")?.as_str()?;
        if k != key && k != key_url {
            return None;
        }
        m.pointer("/payload/subscriptionStateEntity/subscribed")?.as_bool()
    })
}

pub async fn channel_state(http: &reqwest::Client, cookie: &str, channel: &str) -> Result<Value, String> {
    let html = get_page(http, Some(cookie), &format!("https://www.youtube.com/channel/{channel}")).await?;
    let data = initial_data(&html).ok_or("YouTube's page had no channel data")?;
    let subscribed = subscription_of(&data, channel).or_else(|| entity(&data, "subscriptionStateEntity", "subscribed").and_then(|v| v.as_bool()));
    let mut bell: Vec<Value> = Vec::new();
    fn walk(v: &Value, label: &str, icon: &str, depth: usize, out: &mut Vec<Value>) {
        if depth > 90 {
            return;
        }
        match v {
            Value::Array(a) => a.iter().for_each(|c| walk(c, label, icon, depth + 1, out)),
            Value::Object(o) => {
                let own = o.get("text").and_then(text).or_else(|| o.get("title").and_then(text));
                let label = own.as_deref().unwrap_or(label);
                let own_icon = o
                    .get("icon")
                    .and_then(|i| i.get("iconType"))
                    .and_then(Value::as_str)
                    .or_else(|| o.get("leadingImage").and_then(|l| l.pointer("/sources/0/clientResource/imageName")).and_then(Value::as_str));
                let icon = own_icon.unwrap_or(icon);
                if let Some(params) = o
                    .get("modifyChannelNotificationPreferenceEndpoint")
                    .and_then(|e| e.get("params"))
                    .and_then(Value::as_str)
                {
                    if !out.iter().any(|b| b["label"] == label) {
                        out.push(serde_json::json!({"label": label, "icon": icon, "params": params}));
                    }
                }
                for c in o.values() {
                    walk(c, label, icon, depth + 1, out);
                }
            }
            _ => {}
        }
    }
    walk(&data, "", "", 0, &mut bell);
    // The bell button shows the current choice by its icon.
    let current = first(&data, "subscriptionNotificationToggleButtonRenderer", 0)
        .and_then(|b| {
            let id = b.get("currentStateId")?.as_u64()?;
            let states = b.get("states")?.as_array()?;
            let state = states.iter().find(|s| s.get("stateId").and_then(Value::as_u64) == Some(id))?;
            first(state, "iconType", 0)?.as_str().map(str::to_string)
        });
    // The header's "4.34M subscribers" and "865 videos", as YouTube writes them.
    let (mut subs_text, mut videos_text) = (None::<String>, None::<String>);
    find_all(&data, "metadataParts", &mut |parts| {
        for p in parts.as_array().into_iter().flatten() {
            if let Some(t) = p.pointer("/text/content").and_then(Value::as_str) {
                if subs_text.is_none() && t.contains("subscriber") {
                    subs_text = Some(t.to_string());
                } else if videos_text.is_none() && (t.ends_with(" videos") || t.ends_with(" video")) {
                    videos_text.get_or_insert_with(|| t.to_string());
                }
            }
        }
    });
    Ok(serde_json::json!({"subscribed": subscribed, "bell": bell, "bell_now": current, "subscribers_text": subs_text, "videos_text": videos_text}))
}

/// Choose a channel's bell setting with a handle from [`channel_state`].
pub async fn set_bell(http: &reqwest::Client, cookie: &str, params: &str) -> Result<(), String> {
    act(http, cookie, "notification/modify_channel_preference", serde_json::json!({"params": params})).await
}

/// The user's playlists, each saying whether it holds the video already.
pub async fn playlist_options(http: &reqwest::Client, cookie: &str, video: &str) -> Result<Value, String> {
    let data = post_json(http, cookie, "playlist/get_add_to_playlist", serde_json::json!({"videoIds": [video]})).await?;
    let mut out = Vec::new();
    find_all(&data, "playlistAddToOptionRenderer", &mut |p| {
        if let Some(id) = p.get("playlistId").and_then(Value::as_str) {
            out.push(serde_json::json!({
                "id": id,
                "title": p.get("title").and_then(text).unwrap_or_default(),
                "contains": p.get("containsSelectedVideos").and_then(Value::as_str) == Some("ALL"),
                "privacy": p.get("privacy").and_then(Value::as_str).unwrap_or(""),
            }));
        }
    });
    Ok(serde_json::json!({"playlists": out}))
}

/// Put a video in a playlist, or take it out.
pub async fn edit_playlist(http: &reqwest::Client, cookie: &str, playlist: &str, video: &str, add: bool) -> Result<(), String> {
    let action = if add {
        serde_json::json!({"action": "ACTION_ADD_VIDEO", "addedVideoId": video})
    } else {
        serde_json::json!({"action": "ACTION_REMOVE_VIDEO_BY_VIDEO_ID", "removedVideoId": video})
    };
    act(http, cookie, "browse/edit_playlist", serde_json::json!({"playlistId": playlist, "actions": [action]})).await
}

/// Make a new playlist (`privacy`: PRIVATE, UNLISTED or PUBLIC), optionally with a first video.
pub async fn create_playlist(http: &reqwest::Client, cookie: &str, title: &str, privacy: &str, video: Option<&str>) -> Result<String, String> {
    let videos: Vec<&str> = video.into_iter().collect();
    let data = post_json(
        http,
        cookie,
        "playlist/create",
        serde_json::json!({"title": title, "privacyStatus": privacy, "videoIds": videos}),
    )
    .await?;
    data.get("playlistId").and_then(Value::as_str).map(str::to_string).ok_or_else(|| "YouTube did not make the playlist".into())
}

/// The watch history page's own buttons: "clear", "pause" or "resume", with their handles.
pub async fn history_actions(http: &reqwest::Client, cookie: &str) -> Result<Value, String> {
    let html = get_page(http, Some(cookie), "https://www.youtube.com/feed/history").await?;
    let data = initial_data(&html).ok_or("YouTube's page had no history data")?;
    let mut found: Vec<(String, String)> = Vec::new();
    fn walk(v: &Value, label: &str, depth: usize, out: &mut Vec<(String, String)>) {
        if depth > 90 {
            return;
        }
        match v {
            Value::Array(a) => a.iter().for_each(|c| walk(c, label, depth + 1, out)),
            Value::Object(o) => {
                let own = o.get("text").and_then(text).or_else(|| o.get("title").and_then(text));
                let label = own.as_deref().unwrap_or(label);
                if let Some(t) = o.get("feedbackEndpoint").and_then(|f| f.get("feedbackToken")).and_then(Value::as_str) {
                    out.push((label.to_string(), t.to_string()));
                }
                for c in o.values() {
                    walk(c, label, depth + 1, out);
                }
            }
            _ => {}
        }
    }
    let side = data.pointer("/contents/twoColumnBrowseResultsRenderer/secondaryContents").unwrap_or(&data);
    walk(side, "", 0, &mut found);
    let mut actions: Vec<Value> = Vec::new();
    for (label, token) in found {
        let l = label.to_lowercase();
        let kind = if l.contains("clear") {
            "clear"
        } else if l.contains("pause") {
            "pause"
        } else if l.contains("turn on") || l.contains("resume") {
            "resume"
        } else {
            continue;
        };
        if !actions.iter().any(|a| a["token"] == token.as_str()) {
            actions.push(serde_json::json!({"kind": kind, "token": token}));
        }
    }
    Ok(serde_json::json!({"actions": actions}))
}

/// Send one of YouTube's feedback handles ("Not interested", clear or pause history).
pub async fn feedback(http: &reqwest::Client, cookie: &str, token: &str) -> Result<(), String> {
    act(http, cookie, "feedback", serde_json::json!({"feedbackTokens": [token]})).await
}

fn find_all(v: &Value, key: &str, hit: &mut dyn FnMut(&Value)) {
    fn go(v: &Value, key: &str, depth: usize, hit: &mut dyn FnMut(&Value)) {
        if depth > 60 {
            return;
        }
        match v {
            Value::Object(o) => {
                for (k, c) in o {
                    if k == key {
                        hit(c);
                    } else {
                        go(c, key, depth + 1, hit);
                    }
                }
            }
            Value::Array(a) => a.iter().for_each(|c| go(c, key, depth + 1, hit)),
            _ => {}
        }
    }
    go(v, key, 0, hit)
}

/// The text of a chat message: words and emoji.
fn message_text(v: &Value) -> String {
    let Some(runs) = v.get("runs").and_then(Value::as_array) else {
        return text(v).unwrap_or_default();
    };
    runs.iter()
        .map(|r| {
            if let Some(t) = r.get("text").and_then(Value::as_str) {
                return t.to_string();
            }
            let e = r.get("emoji");
            // Standard emoji carry the character as their id; channel emoji a short name.
            match e.and_then(|e| e.get("emojiId")).and_then(Value::as_str) {
                Some(id) if id.chars().count() <= 4 => id.to_string(),
                _ => e
                    .and_then(|e| e.get("shortcuts"))
                    .and_then(|s| s.get(0))
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
            }
        })
        .collect()
}

/// New messages of a live stream's chat. Answers `{messages, next, wait_ms}`: ask again
/// with `next` after `wait_ms`.
pub async fn live_chat(http: &reqwest::Client, cookie: Option<&str>, token: &str) -> Result<Value, String> {
    let body = serde_json::json!({
        "context": {"client": {"clientName": "WEB", "clientVersion": WEB_VERSION, "hl": "en", "gl": "US"}},
        "continuation": token,
    });
    let mut req = http
        .post("https://www.youtube.com/youtubei/v1/live_chat/get_live_chat?prettyPrint=false")
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ORIGIN, "https://www.youtube.com")
        .header("X-Origin", "https://www.youtube.com")
        .timeout(Duration::from_secs(20))
        .json(&body);
    if let Some(c) = cookie {
        req = req.header(reqwest::header::COOKIE, c).header("X-Goog-AuthUser", "0");
        if let Some(auth) = sapisid_hash(c) {
            req = req.header(reqwest::header::AUTHORIZATION, auth);
        }
    }
    let resp = req.send().await.map_err(|e| format!("could not reach YouTube: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("YouTube answered HTTP {}", resp.status().as_u16()));
    }
    let data: Value = resp.json().await.map_err(|e| format!("unexpected answer from YouTube: {e}"))?;
    let chat = data.pointer("/continuationContents/liveChatContinuation").ok_or("the chat has ended")?;
    let mut messages = Vec::new();
    for action in chat.get("actions").and_then(Value::as_array).into_iter().flatten() {
        let Some(item) = action.pointer("/addChatItemAction/item") else {
            continue;
        };
        for (kind, paid) in [("liveChatTextMessageRenderer", false), ("liveChatPaidMessageRenderer", true)] {
            if let Some(m) = item.get(kind) {
                messages.push(serde_json::json!({
                    "id": m.get("id").and_then(Value::as_str).unwrap_or(""),
                    "author": m.get("authorName").and_then(text).unwrap_or_default(),
                    "avatar": thumb(m.pointer("/authorPhoto/thumbnails")),
                    "text": m.get("message").map(message_text).unwrap_or_default(),
                    "amount": if paid { m.get("purchaseAmountText").and_then(text) } else { None },
                }));
            }
        }
    }
    let next = chat.get("continuations").and_then(|c| c.get(0));
    let data_of = next.and_then(|n| {
        ["invalidationContinuationData", "timedContinuationData", "reloadContinuationData"]
            .iter()
            .find_map(|k| n.get(*k))
    });
    Ok(serde_json::json!({
        "messages": messages,
        "next": data_of.and_then(|d| d.get("continuation")).and_then(Value::as_str),
        "wait_ms": data_of.and_then(|d| d.get("timeoutMs")).and_then(Value::as_u64).unwrap_or(5000).clamp(1000, 15000),
    }))
}

/// Caption tracks and preview pictures of a video, from the phone apps' player API: the
/// caption links of the web page now need a proof-of-origin token, these do not.
pub async fn player_extras(http: &reqwest::Client, id: &str) -> Result<Value, String> {
    let clients = [
        (
            serde_json::json!({"clientName": "IOS", "clientVersion": "20.10.4", "deviceMake": "Apple", "deviceModel": "iPhone16,2",
                "osName": "iPhone", "osVersion": "18.3.2.22D82", "hl": "en", "gl": "US"}),
            "com.google.ios.youtube/20.10.4 (iPhone16,2; U; CPU iOS 18_3_2 like Mac OS X;)",
        ),
        (
            serde_json::json!({"clientName": "ANDROID_VR", "clientVersion": "1.62.27", "deviceMake": "Oculus", "deviceModel": "Quest 3",
                "androidSdkVersion": 32, "osName": "Android", "osVersion": "12L", "hl": "en", "gl": "US"}),
            "com.google.android.apps.youtube.vr.oculus/1.62.27 (Linux; U; Android 12L; eureka-user Build/SQ3A.220605.009.A1) gzip",
        ),
    ];
    let mut last = String::from("no answer");
    for (client, agent) in clients {
        let resp = http
            .post("https://www.youtube.com/youtubei/v1/player?prettyPrint=false")
            .header(reqwest::header::USER_AGENT, agent)
            .timeout(Duration::from_secs(15))
            .json(&serde_json::json!({"videoId": id, "context": {"client": client}}))
            .send()
            .await;
        let data: Value = match resp {
            Ok(r) if r.status().is_success() => match r.json().await {
                Ok(v) => v,
                Err(e) => {
                    last = e.to_string();
                    continue;
                }
            },
            Ok(r) => {
                last = format!("HTTP {}", r.status().as_u16());
                continue;
            }
            Err(e) => {
                last = e.to_string();
                continue;
            }
        };
        let tracks: Vec<Value> = data
            .pointer("/captions/playerCaptionsTracklistRenderer/captionTracks")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|t| {
                Some(serde_json::json!({
                    "lang": t.get("languageCode")?.as_str()?,
                    "name": t.get("name").and_then(text).unwrap_or_default(),
                    "auto": t.get("kind").and_then(Value::as_str) == Some("asr"),
                    "url": t.get("baseUrl")?.as_str()?,
                    "agent": agent,
                }))
            })
            .collect();
        let storyboard = storyboard_spec(&data);
        if tracks.is_empty() && storyboard.is_none() {
            last = "no captions".into();
            continue;
        }
        return Ok(serde_json::json!({"captions": tracks, "storyboard": storyboard}));
    }
    Err(last)
}

/// One caption track as timed lines: `{cues: [{s, e, t}]}` in seconds.
pub async fn captions(http: &reqwest::Client, url: &str, agent: Option<&str>) -> Result<Value, String> {
    let resp = http
        .get(format!("{url}&fmt=json3"))
        .header(reqwest::header::USER_AGENT, agent.unwrap_or(USER_AGENT))
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("could not load captions: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("YouTube answered HTTP {}", resp.status().as_u16()));
    }
    let body = resp.text().await.map_err(|e| e.to_string())?;
    if body.trim().is_empty() {
        return Err("YouTube sent empty captions".into());
    }
    let data: Value = serde_json::from_str(&body).map_err(|e| format!("unexpected captions: {e}"))?;
    let mut cues = Vec::new();
    for ev in data.get("events").and_then(Value::as_array).into_iter().flatten() {
        let Some(segs) = ev.get("segs").and_then(Value::as_array) else {
            continue;
        };
        let line: String = segs.iter().filter_map(|s| s.get("utf8").and_then(Value::as_str)).collect();
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let start = ev.get("tStartMs").and_then(Value::as_f64).unwrap_or(0.0) / 1000.0;
        let length = ev.get("dDurationMs").and_then(Value::as_f64).unwrap_or(2000.0) / 1000.0;
        cues.push(serde_json::json!({"s": start, "e": start + length, "t": line}));
    }
    Ok(serde_json::json!({"cues": cues}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_page_data() {
        let html = r#"<script nonce="x">var ytInitialData = {"responseContext":{"mainAppWebResponseContext":{"loggedOut":false}},
          "contents":{"richGridRenderer":{"contents":[
            {"richItemRenderer":{"content":{"videoRenderer":{"videoId":"aaaaaaaaaaa","title":{"runs":[{"text":"First"}]},
               "ownerText":{"runs":[{"text":"Chan","navigationEndpoint":{"browseEndpoint":{"browseId":"UC123"}}}]},
               "lengthText":{"simpleText":"1:02:03"},"shortViewCountText":{"simpleText":"1.2M views"},
               "publishedTimeText":{"simpleText":"3 days ago"},
               "thumbnail":{"thumbnails":[{"url":"//i/1.jpg","width":168},{"url":"https://i/2.jpg","width":336},{"url":"https://i/3.jpg","width":1280}]}}}}},
            {"richItemRenderer":{"content":{"adSlotRenderer":{"x":{"videoRenderer":{"videoId":"adadadadada","title":{"simpleText":"Ad"}}}}}}},
            {"richItemRenderer":{"content":{"lockupViewModel":{"contentId":"bbbbbbbbbbb","contentType":"LOCKUP_CONTENT_TYPE_VIDEO",
               "contentImage":{"thumbnailViewModel":{"image":{"sources":[{"url":"https://i/b.jpg","width":336}]},
                 "overlays":[{"thumbnailOverlayBadgeViewModel":{"thumbnailBadges":[{"thumbnailBadgeViewModel":{"text":"4:05"}}]}}]}},
               "metadata":{"lockupMetadataViewModel":{"title":{"content":"Second"},
                 "metadata":{"contentMetadataViewModel":{"metadataRows":[
                   {"metadataParts":[{"text":{"content":"Other"}}]},
                   {"metadataParts":[{"text":{"content":"5K views"}},{"text":{"content":"1 hour ago"}}]}]}}}}}}}},
            {"richSectionRenderer":{"content":{"richShelfRenderer":{"contents":[{"videoRenderer":{"videoId":"shortshorts","title":{"simpleText":"S"}}}]}}}},
            {"channelRenderer":{"channelId":"UC9","title":{"simpleText":"A channel"},"thumbnail":{"thumbnails":[{"url":"//a/c.jpg","width":88}]}}},
            {"videoRenderer":{"videoId":"aaaaaaaaaaa","title":{"simpleText":"Duplicate"}}}
          ]}}};</script><script>other()</script>"#;
        let data = initial_data(html).unwrap();
        assert!(signed_in(&data));
        let items = collect(&data);
        assert_eq!(items.len(), 3, "{items:?}");
        let Item::Video(a) = &items[0] else { panic!() };
        assert_eq!((a.title.as_str(), a.channel_id.as_deref(), a.duration_s), ("First", Some("UC123"), Some(3723)));
        assert_eq!(a.published.as_deref(), Some("1.2M views · 3 days ago"));
        assert_eq!(a.thumbnail.as_deref(), Some("https://i/2.jpg"));
        let Item::Video(b) = &items[1] else { panic!() };
        assert_eq!((b.title.as_str(), b.channel_name.as_deref(), b.duration_s), ("Second", Some("Other"), Some(245)));
        assert_eq!(b.published.as_deref(), Some("5K views · 1 hour ago"));
        assert!(matches!(&items[2], Item::Channel(c) if c.avatar.as_deref() == Some("https://a/c.jpg")));
    }

    #[test]
    fn reads_watch_page() {
        let d: Value = serde_json::json!({"contents": {"twoColumnWatchNextResults": {
            "results": {"results": {"contents": [
                {"videoPrimaryInfoRenderer": {"title": {"runs": [{"text": "A "}, {"text": "song"}]},
                    "viewCount": {"videoViewCountRenderer": {"viewCount": {"simpleText": "1,234 views"}}},
                    "dateText": {"simpleText": "Nov 10, 2026"}}},
                {"videoSecondaryInfoRenderer": {
                    "owner": {"videoOwnerRenderer": {"title": {"runs": [{"text": "Artist",
                        "navigationEndpoint": {"browseEndpoint": {"browseId": "UCabc"}}}]}}},
                    "attributedDescription": {"content": "About it"}}}]}},
            "secondaryResults": {"secondaryResults": {"results": [
                {"compactVideoRenderer": {"videoId": "ccccccccccc", "title": {"simpleText": "Next"}}},
                {"compactVideoRenderer": {"videoId": "aaaaaaaaaaa", "title": {"simpleText": "Itself"}}}]}}}}});
        let p = watch_page(&d, "aaaaaaaaaaa").unwrap();
        assert_eq!((p.video.title.as_str(), p.video.channel_name.as_deref(), p.video.channel_id.as_deref()), ("A song", Some("Artist"), Some("UCabc")));
        assert_eq!(p.video.published.as_deref(), Some("1,234 views · Nov 10, 2026"));
        assert_eq!((p.description.as_str(), p.related.items.len()), ("About it", 1));
    }

    #[test]
    fn finds_what_load_more_needs() {
        let html = r#"ytcfg.set({"INNERTUBE_API_KEY":"k","INNERTUBE_CONTEXT":{"client":{"clientName":"WEB","clientVersion":"2.1"}},"SESSION_INDEX":"1"});"#;
        assert_eq!(json_after(html, "\"INNERTUBE_CONTEXT\":").unwrap()["client"]["clientName"], "WEB");
        assert_eq!(json_after(html, "\"SESSION_INDEX\":").unwrap(), "1");
        let data: Value = serde_json::json!({
            "header": {"continuationItemRenderer": {"continuationEndpoint": {"continuationCommand": {"token": "chips"}}}},
            "contents": [
                {"continuationItemRenderer": {"continuationEndpoint": {"continuationCommand": {"token": "first"}}}},
                {"videoRenderer": {"videoId": "aaaaaaaaaaa", "title": {"simpleText": "A"}}},
                {"continuationItemRenderer": {"continuationEndpoint": {"continuationCommand": {"token": "last"}}}}]});
        assert_eq!(continuation(&data).as_deref(), Some("last"));
        let h = sapisid_hash("a=b; SAPISID=secret; c=d").unwrap();
        assert!(h.starts_with("SAPISIDHASH ") && h.contains('_') && !h.contains("secret"));
        assert!(sapisid_hash("a=b").is_none());
    }

    #[test]
    fn reads_comments() {
        let page: Value = serde_json::json!({"contents": [
            {"itemSectionRenderer": {"sectionIdentifier": "other", "contents": [{"continuationItemRenderer": {"x": {"token": "wrong"}}}]}},
            {"itemSectionRenderer": {"sectionIdentifier": "comment-item-section",
                "contents": [{"continuationItemRenderer": {"continuationEndpoint": {"continuationCommand": {"token": "open"}}}}]}}]});
        assert_eq!(comments_token(&page).as_deref(), Some("open"));
        let answer: Value = serde_json::json!({
            "onResponseReceivedEndpoints": [{"reloadContinuationItemsCommand": {"continuationItems": [
                {"commentThreadRenderer": {"replies": {"commentRepliesRenderer": {"contents": [
                    {"continuationItemRenderer": {"continuationEndpoint": {"continuationCommand": {"token": "replies"}}}}]}}}},
                {"continuationItemRenderer": {"continuationEndpoint": {"continuationCommand": {"token": "page2"}}}}]}}],
            "frameworkUpdates": {"entityBatchUpdate": {"mutations": [
                {"payload": {"commentEntityPayload": {
                    "properties": {"commentId": "c1", "content": {"content": "First!"}, "publishedTime": "2 days ago", "replyLevel": 0},
                    "author": {"channelId": "UC1", "displayName": "@someone", "avatarThumbnailUrl": "https://a/1.jpg", "isCreator": true},
                    "toolbar": {"likeCountNotliked": "1.2K", "replyCount": "3"}}}},
                {"payload": {"commentEntityPayload": {
                    "properties": {"commentId": "c2", "content": {"content": "a reply"}, "replyLevel": 1},
                    "author": {"displayName": "@other"}, "toolbar": {}}}}]}}});
        let (items, next) = read_comments(&answer);
        assert_eq!(next.as_deref(), Some("page2"));
        assert_eq!(items.len(), 1);
        let c = &items[0];
        assert_eq!((c.text.as_str(), c.author.as_deref(), c.likes, c.replies, c.by_owner), ("First!", Some("@someone"), Some(1200), 3, true));
        assert_eq!(count("12"), Some(12));
    }

    #[test]
    fn notices_signed_out_pages() {
        let d: Value = serde_json::json!({"responseContext": {"mainAppWebResponseContext": {"loggedOut": true}}});
        assert!(!signed_in(&d));
        assert_eq!(seconds("3:53"), Some(233));
        assert_eq!(seconds("LIVE"), None);
    }
}
