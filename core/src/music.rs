//! YouTube Music, read from its own page API (`music.youtube.com/youtubei/v1`, the WEB_REMIX
//! client), signed in with the app's session when there is one.
//!
//! Every Music page is a list of shelves: a carousel of cards (`musicCarouselShelfRenderer`
//! with `musicTwoRowItemRenderer` or song rows), a list of song rows (`musicShelfRenderer`,
//! `musicPlaylistShelfRenderer`), a grid, buttons (moods and genres) or a block of text.
//! Pages for an artist, an album or a playlist add a header. This module turns all of them
//! into one shape ([`MusicPage`]) so the app needs one renderer.

use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";
const CLIENT_VERSION: &str = "1.20251110.01.00";
const ORIGIN: &str = "https://music.youtube.com";

/// One thing on a Music page.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MusicItem {
    /// "song", "video", "album", "single", "playlist", "artist", "podcast", "episode", "mood".
    pub kind: String,
    /// The video id (songs, videos) or the page to open (`browseId`).
    pub id: String,
    /// Extra handle a page needs (mood and genre pages).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<String>,
    pub title: String,
    /// The grey line under the title, as YouTube words it.
    pub subtitle: String,
    pub artists: Vec<Named>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<Named>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duration: Option<String>,
    /// The list to play for an album or playlist card (its "play" button).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub play_list: Option<String>,
    /// The stripe colour of a mood or genre button (ARGB).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<u32>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub explicit: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Named {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

/// A row of a page.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Shelf {
    pub title: String,
    /// "carousel" (cards side by side), "list" (song rows), "grid", "buttons" or "text".
    pub style: String,
    pub items: Vec<MusicItem>,
    /// The text of a "text" shelf (an artist's description, lyrics).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The page with everything of this shelf ("More").
    #[serde(skip_serializing_if = "Option::is_none")]
    pub more: Option<MusicItem>,
}

/// The top of an artist, album or playlist page.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Header {
    pub title: String,
    pub subtitle: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub second_subtitle: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    pub artists: Vec<Named>,
    /// The channel to subscribe to (artists).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscribed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subscribers: Option<String>,
    /// The endless mix that "Radio" / "Mix" starts.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub radio: Option<String>,
    /// The list to save to the library (albums, playlists).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_id: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MusicPage {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub header: Option<Header>,
    pub shelves: Vec<Shelf>,
    /// True when these shelves continue the last list of the previous page (more rows).
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub append: bool,
    /// The cursor for more (set by the service).
    pub next: Option<u64>,
}

/// What is needed to load more of a page.
#[derive(Clone, Debug)]
pub struct More {
    /// "browse", "search" or "next".
    pub endpoint: &'static str,
    pub token: String,
}

// ------------------------------------------------------------------ requests

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
    let digest = Sha1::digest(format!("{now} {sapisid} {ORIGIN}").as_bytes());
    Some(format!("SAPISIDHASH {now}_{}", hex::encode(digest)))
}

/// A POST to the Music page API.
pub async fn call(http: &reqwest::Client, cookie: Option<&str>, path: &str, mut body: Value) -> Result<Value, String> {
    body["context"] = json!({"client": {"clientName": "WEB_REMIX", "clientVersion": CLIENT_VERSION, "hl": "en", "gl": "US"}});
    let mut req = http
        .post(format!("{ORIGIN}/youtubei/v1/{path}?prettyPrint=false"))
        .header(reqwest::header::USER_AGENT, USER_AGENT)
        .header(reqwest::header::ORIGIN, ORIGIN)
        .header("X-Origin", ORIGIN)
        .header(reqwest::header::ACCEPT_LANGUAGE, "en-US,en;q=0.9")
        .timeout(Duration::from_secs(25))
        .json(&body);
    if let Some(c) = cookie {
        req = req.header(reqwest::header::COOKIE, c).header("X-Goog-AuthUser", "0");
        if let Some(auth) = sapisid_hash(c) {
            req = req.header(reqwest::header::AUTHORIZATION, auth);
        }
    }
    let resp = req.send().await.map_err(|e| format!("could not reach YouTube Music: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("YouTube Music answered HTTP {}", resp.status().as_u16()));
    }
    resp.json().await.map_err(|e| format!("unexpected answer from YouTube Music: {e}"))
}

/// A Music page (`FEmusic_home`, an artist `UC…`, an album `MPRE…`, a playlist `VL…`, …).
pub async fn browse(http: &reqwest::Client, cookie: Option<&str>, id: &str, params: Option<&str>) -> Result<(MusicPage, Option<More>), String> {
    let mut body = json!({"browseId": id});
    if let Some(p) = params {
        body["params"] = json!(p);
    }
    let data = call(http, cookie, "browse", body).await?;
    Ok(read_page(&data))
}

/// The search tabs and their filters.
fn search_params(tab: &str) -> Option<&'static str> {
    Some(match tab {
        "songs" => "EgWKAQIIAWoMEA4QChADEAQQCRAF",
        "videos" => "EgWKAQIQAWoMEA4QChADEAQQCRAF",
        "albums" => "EgWKAQIYAWoMEA4QChADEAQQCRAF",
        "artists" => "EgWKAQIgAWoMEA4QChADEAQQCRAF",
        "playlists" => "EgeKAQQoAEABagwQDhAKEAMQBBAJEAU%3D",
        "podcasts" => "EgWKAQJQAWoQEBAQCRAEEAMQBRAKEBUQEQ%3D%3D",
        _ => return None,
    })
}

pub async fn search(http: &reqwest::Client, cookie: Option<&str>, query: &str, tab: &str) -> Result<(MusicPage, Option<More>), String> {
    let mut body = json!({"query": query});
    if let Some(p) = search_params(tab) {
        body["params"] = json!(p);
    }
    let data = call(http, cookie, "search", body).await?;
    let (mut page, more) = read_page(&data);
    page.header = None;
    Ok((page, more.map(|m| More { endpoint: "search", ..m })))
}

/// More of a page, a search or a queue.
pub async fn more(http: &reqwest::Client, cookie: Option<&str>, m: &More) -> Result<(MusicPage, Option<More>), String> {
    if m.endpoint == "next" {
        let data = call(http, cookie, "next", json!({"continuation": m.token, "isAudioOnly": true})).await?;
        let (items, next) = read_queue(&data);
        let page = MusicPage { shelves: vec![Shelf { title: String::new(), style: "list".into(), items, ..Default::default() }], append: true, ..Default::default() };
        return Ok((page, next));
    }
    let data = call(http, cookie, m.endpoint, json!({"continuation": m.token})).await?;
    let (page, next) = read_page(&data);
    Ok((page, next.map(|n| More { endpoint: m.endpoint, ..n })))
}

/// What plays after a song: its radio (an endless mix) or the rest of a list, plus the
/// handles of the song's lyrics and related pages.
pub async fn queue(http: &reqwest::Client, cookie: Option<&str>, video: Option<&str>, list: Option<&str>) -> Result<(Value, Option<More>), String> {
    let mut body = json!({"isAudioOnly": true, "enablePersistentPlaylistPanel": true, "tunerSettingValue": "AUTOMIX_SETTING_NORMAL"});
    if let Some(v) = video {
        body["videoId"] = json!(v);
    }
    match (list, video) {
        (Some(l), _) => body["playlistId"] = json!(l),
        (None, Some(v)) => body["playlistId"] = json!(format!("RDAMVM{v}")),
        (None, None) => return Err("nothing to play".into()),
    }
    let data = call(http, cookie, "next", body).await?;
    let (items, next) = read_queue(&data);
    let mut lyrics = None;
    let mut related = None;
    find_all(&data, "tabRenderer", &mut |t| {
        let page = t.pointer("/endpoint/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType").and_then(Value::as_str);
        let id = t.pointer("/endpoint/browseEndpoint/browseId").and_then(Value::as_str).map(str::to_string);
        match page {
            Some("MUSIC_PAGE_TYPE_TRACK_LYRICS") => lyrics = id,
            Some("MUSIC_PAGE_TYPE_TRACK_RELATED") => related = id,
            _ => {}
        }
    });
    Ok((json!({"items": items, "lyrics": lyrics, "related": related}), next))
}

/// A song's lyrics: `{text, source}`, or null text when it has none.
pub async fn lyrics(http: &reqwest::Client, cookie: Option<&str>, id: &str) -> Result<Value, String> {
    let data = call(http, cookie, "browse", json!({"browseId": id})).await?;
    let mut out = json!({"text": null, "source": null});
    find_all(&data, "musicDescriptionShelfRenderer", &mut |d| {
        if out["text"].is_null() {
            out["text"] = json!(d.get("description").and_then(text));
            out["source"] = json!(d.get("footer").and_then(text));
        }
    });
    Ok(out)
}

/// Save an album or playlist to the library, or take it out.
pub async fn save(http: &reqwest::Client, cookie: &str, list: &str, on: bool) -> Result<(), String> {
    let path = if on { "like/like" } else { "like/removelike" };
    call(http, Some(cookie), path, json!({"target": {"playlistId": list}})).await.map(|_| ())
}

/// Words to complete a search with.
pub async fn suggest(http: &reqwest::Client, cookie: Option<&str>, query: &str) -> Result<Vec<String>, String> {
    let data = call(http, cookie, "music/get_search_suggestions", json!({"input": query})).await?;
    let mut out = Vec::new();
    find_all(&data, "searchSuggestionRenderer", &mut |s| {
        if let Some(t) = s.get("suggestion").and_then(text) {
            out.push(t);
        }
    });
    Ok(out)
}

// ------------------------------------------------------------------ reading

fn text(v: &Value) -> Option<String> {
    if let Some(s) = v.get("simpleText").and_then(Value::as_str) {
        return Some(s.to_string());
    }
    let runs = v.get("runs")?.as_array()?;
    let s: String = runs.iter().filter_map(|r| r.get("text").and_then(Value::as_str)).collect();
    (!s.is_empty()).then_some(s)
}

fn find_all(v: &Value, key: &str, hit: &mut dyn FnMut(&Value)) {
    fn go(v: &Value, key: &str, depth: usize, hit: &mut dyn FnMut(&Value)) {
        if depth > 70 {
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

fn first<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    fn go<'a>(v: &'a Value, key: &str, depth: usize) -> Option<&'a Value> {
        if depth > 70 {
            return None;
        }
        match v {
            Value::Object(o) => o.get(key).or_else(|| o.values().find_map(|c| go(c, key, depth + 1))),
            Value::Array(a) => a.iter().find_map(|c| go(c, key, depth + 1)),
            _ => None,
        }
    }
    go(v, key, 0)
}

/// The largest picture of a `thumbnails` list, asked for at 544 px where the host allows it.
fn picture(v: Option<&Value>) -> Option<String> {
    let list = v?.as_array()?;
    let best = list.iter().max_by_key(|t| t.get("width").and_then(Value::as_u64).unwrap_or(0))?;
    let url = best.get("url")?.as_str()?;
    let url = if url.starts_with("//") { format!("https:{url}") } else { url.to_string() };
    // Album art and artist pictures are served at any size; small ones look blurry.
    if url.contains("googleusercontent.com") || url.contains("ggpht.com") {
        if let Some(at) = url.rfind("=w") {
            return Some(format!("{}=w544-h544-l90-rj", &url[..at]));
        }
        if let Some(at) = url.rfind("=s") {
            return Some(format!("{}=s544", &url[..at]));
        }
    }
    Some(url)
}

fn thumbnail_of(r: &Value) -> Option<String> {
    let t = r
        .get("thumbnail")
        .or_else(|| r.get("thumbnailRenderer"))
        .or_else(|| r.get("foregroundThumbnail"))?;
    picture(first(t, "thumbnails"))
}

fn page_type(endpoint: &Value) -> Option<&str> {
    endpoint
        .pointer("/browseEndpoint/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType")
        .and_then(Value::as_str)
}

fn kind_of_page(page: &str) -> &'static str {
    match page {
        "MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL" | "MUSIC_PAGE_TYPE_LIBRARY_ARTIST" => "artist",
        "MUSIC_PAGE_TYPE_ALBUM" | "MUSIC_PAGE_TYPE_AUDIOBOOK" => "album",
        "MUSIC_PAGE_TYPE_PODCAST_SHOW_DETAIL_PAGE" => "podcast",
        "MUSIC_PAGE_TYPE_NON_MUSIC_AUDIO_TRACK_PAGE" => "episode",
        _ => "playlist",
    }
}

fn video_kind(endpoint: &Value) -> &'static str {
    match endpoint
        .pointer("/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType")
        .and_then(Value::as_str)
    {
        Some("MUSIC_VIDEO_TYPE_ATV") | Some("MUSIC_VIDEO_TYPE_PRIVATELY_OWNED_TRACK") => "song",
        Some("MUSIC_VIDEO_TYPE_PODCAST_EPISODE") => "episode",
        _ => "video",
    }
}

/// Artists and album named in a line of runs, and the line's text.
fn read_runs(runs: &[&Value], item: &mut MusicItem) -> Vec<String> {
    let mut parts = Vec::new();
    for column in runs {
        let Some(list) = column.get("runs").and_then(Value::as_array) else {
            continue;
        };
        let mut line = String::new();
        for r in list {
            let t = r.get("text").and_then(Value::as_str).unwrap_or("");
            line.push_str(t);
            let nav = r.get("navigationEndpoint");
            let id = nav.and_then(|n| n.pointer("/browseEndpoint/browseId")).and_then(Value::as_str);
            match nav.and_then(page_type) {
                Some("MUSIC_PAGE_TYPE_ARTIST") | Some("MUSIC_PAGE_TYPE_USER_CHANNEL") => {
                    item.artists.push(Named { name: t.to_string(), id: id.map(str::to_string) })
                }
                Some("MUSIC_PAGE_TYPE_ALBUM") => item.album = Some(Named { name: t.to_string(), id: id.map(str::to_string) }),
                _ => {}
            }
            if item.duration.is_none() && is_duration(t) {
                item.duration = Some(t.to_string());
            }
        }
        if !line.trim().is_empty() {
            parts.push(line);
        }
    }
    parts
}

fn is_duration(s: &str) -> bool {
    let parts: Vec<&str> = s.split(':').collect();
    (2..=3).contains(&parts.len()) && parts.iter().all(|p| !p.is_empty() && p.len() <= 2 && p.chars().all(|c| c.is_ascii_digit()))
}

fn explicit(r: &Value) -> bool {
    r.get("badges").is_some_and(|b| b.to_string().contains("MUSIC_EXPLICIT_BADGE")) || r.get("subtitleBadges").is_some_and(|b| b.to_string().contains("MUSIC_EXPLICIT_BADGE"))
}

/// A song or list row (`musicResponsiveListItemRenderer`).
fn row(r: &Value) -> Option<MusicItem> {
    let mut item = MusicItem { thumbnail: thumbnail_of(r), explicit: explicit(r), ..Default::default() };
    let columns: Vec<&Value> = r
        .get("flexColumns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| c.get("musicResponsiveListItemFlexColumnRenderer").and_then(|f| f.get("text")))
        .collect();
    let title = columns.first().and_then(|c| text(c))?;
    let title_nav = columns.first().and_then(|c| c.pointer("/runs/0/navigationEndpoint"));
    let video = r
        .pointer("/playlistItemData/videoId")
        .or_else(|| title_nav.and_then(|n| n.pointer("/watchEndpoint/videoId")))
        .or_else(|| r.pointer("/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint/videoId"))
        .and_then(Value::as_str);
    let mut lines = read_runs(&columns[1.min(columns.len())..], &mut item);
    if let Some(fixed) = r.get("fixedColumns").and_then(Value::as_array) {
        for f in fixed {
            if let Some(t) = f.pointer("/musicResponsiveListItemFixedColumnRenderer/text").and_then(text) {
                if is_duration(&t) {
                    item.duration = Some(t);
                } else {
                    lines.push(t);
                }
            }
        }
    }
    item.title = title;
    item.subtitle = lines.join(" • ");
    if let Some(v) = video {
        item.id = v.to_string();
        let nav = title_nav
            .or_else(|| r.pointer("/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint"))
            .unwrap_or(&Value::Null);
        item.kind = video_kind(nav).to_string();
        if item.kind == "video" && nav.get("watchEndpoint").is_some() && nav.pointer("/watchEndpoint/watchEndpointMusicSupportedConfigs").is_none() {
            // Album tracks name no type; they are songs.
            item.kind = "song".into();
        }
        return Some(item);
    }
    let nav = r.get("navigationEndpoint")?;
    item.id = nav.pointer("/browseEndpoint/browseId")?.as_str()?.to_string();
    item.kind = kind_of_page(page_type(nav).unwrap_or("")).to_string();
    Some(item)
}

/// A card (`musicTwoRowItemRenderer`).
fn card(r: &Value) -> Option<MusicItem> {
    let mut item = MusicItem { thumbnail: thumbnail_of(r), explicit: explicit(r), ..Default::default() };
    item.title = r.get("title").and_then(text)?;
    let lines = read_runs(&[r.get("subtitle").unwrap_or(&Value::Null)], &mut item);
    item.subtitle = lines.join(" • ");
    let nav = r.get("navigationEndpoint")?;
    if let Some(v) = nav.pointer("/watchEndpoint/videoId").and_then(Value::as_str) {
        item.id = v.to_string();
        item.kind = video_kind(nav).to_string();
        return Some(item);
    }
    item.id = nav.pointer("/browseEndpoint/browseId")?.as_str()?.to_string();
    let page = page_type(nav).unwrap_or("");
    item.kind = kind_of_page(page).to_string();
    if item.kind == "album" && item.subtitle.starts_with("Single") {
        item.kind = "single".into();
    }
    item.play_list = first(r.get("thumbnailOverlay").unwrap_or(&Value::Null), "playlistId")
        .and_then(Value::as_str)
        .map(str::to_string);
    Some(item)
}

/// A mood or genre button.
fn button(r: &Value) -> Option<MusicItem> {
    let nav = r.pointer("/clickCommand/browseEndpoint")?;
    Some(MusicItem {
        kind: "mood".into(),
        id: nav.get("browseId")?.as_str()?.to_string(),
        params: nav.get("params").and_then(Value::as_str).map(str::to_string),
        title: r.get("buttonText").and_then(text)?,
        color: r.pointer("/solid/leftStripeColor").and_then(Value::as_u64).map(|c| c as u32),
        ..Default::default()
    })
}

/// A queue entry (`playlistPanelVideoRenderer`).
fn queued(r: &Value) -> Option<MusicItem> {
    let mut item = MusicItem { thumbnail: thumbnail_of(r), ..Default::default() };
    item.id = r.get("videoId")?.as_str()?.to_string();
    item.title = r.get("title").and_then(text)?;
    let byline = r.get("longBylineText").unwrap_or(&Value::Null);
    let lines = read_runs(&[byline], &mut item);
    item.subtitle = lines.join(" • ");
    item.duration = r.get("lengthText").and_then(text);
    item.kind = r.get("navigationEndpoint").map(video_kind).unwrap_or("song").to_string();
    Some(item)
}

fn items_of(list: &[Value]) -> Vec<MusicItem> {
    list.iter()
        .filter_map(|i| {
            if let Some(r) = i.get("musicResponsiveListItemRenderer") {
                row(r)
            } else if let Some(r) = i.get("musicTwoRowItemRenderer") {
                card(r)
            } else if let Some(r) = i.get("musicNavigationButtonRenderer") {
                button(r)
            } else if let Some(r) = i.get("playlistPanelVideoRenderer") {
                queued(r)
            } else if let Some(r) = i.get("musicMultiRowListItemRenderer") {
                // Podcast episodes.
                let mut item = MusicItem { kind: "episode".into(), thumbnail: thumbnail_of(r), ..Default::default() };
                item.title = r.get("title").and_then(text)?;
                item.subtitle = r.get("subtitle").and_then(text).unwrap_or_default();
                item.id = first(r, "videoId")?.as_str()?.to_string();
                Some(item)
            } else {
                None
            }
        })
        .collect()
}

fn more_link(header: &Value) -> Option<MusicItem> {
    let nav = header.pointer("/moreContentButton/buttonRenderer/navigationEndpoint/browseEndpoint")?;
    Some(MusicItem {
        kind: "page".into(),
        id: nav.get("browseId")?.as_str()?.to_string(),
        params: nav.get("params").and_then(Value::as_str).map(str::to_string),
        title: header.get("title").and_then(text).unwrap_or_default(),
        ..Default::default()
    })
}

/// One section of a page as a shelf.
fn shelf(key: &str, v: &Value) -> Option<Shelf> {
    match key {
        "musicCarouselShelfRenderer" => {
            let h = v.pointer("/header/musicCarouselShelfBasicHeaderRenderer");
            let items = items_of(v.get("contents")?.as_array()?);
            let rows = items.iter().all(|i| matches!(i.kind.as_str(), "song" | "video" | "episode"))
                && v.get("contents")?.as_array()?.first().is_some_and(|c| c.get("musicResponsiveListItemRenderer").is_some());
            let buttons = items.iter().all(|i| i.kind == "mood");
            Some(Shelf {
                title: h.and_then(|h| h.get("title")).and_then(text).unwrap_or_default(),
                // Quick picks and similar are song rows laid out in pages of four.
                style: if buttons { "buttons" } else if rows { "pages" } else { "carousel" }.into(),
                more: h.and_then(more_link),
                items,
                text: None,
            })
        }
        "musicShelfRenderer" | "musicPlaylistShelfRenderer" => Some(Shelf {
            title: v.get("title").and_then(text).unwrap_or_default(),
            style: "list".into(),
            more: v.pointer("/bottomEndpoint/browseEndpoint").and_then(|b| {
                Some(MusicItem {
                    kind: "page".into(),
                    id: b.get("browseId")?.as_str()?.to_string(),
                    params: b.get("params").and_then(Value::as_str).map(str::to_string),
                    title: v.get("title").and_then(text).unwrap_or_default(),
                    ..Default::default()
                })
            }),
            items: items_of(v.get("contents").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])),
            text: None,
        }),
        "gridRenderer" => {
            let items = items_of(v.get("items")?.as_array()?);
            Some(Shelf {
                title: v.pointer("/header/gridHeaderRenderer/title").and_then(text).unwrap_or_default(),
                style: if items.iter().all(|i| i.kind == "mood") { "buttons" } else { "grid" }.into(),
                items,
                text: None,
                more: None,
            })
        }
        "musicDescriptionShelfRenderer" => Some(Shelf {
            title: v.get("header").and_then(text).unwrap_or_else(|| "About".into()),
            style: "text".into(),
            text: v.get("description").and_then(text),
            items: Vec::new(),
            more: None,
        }),
        "musicCardShelfRenderer" => {
            // The top result of a search.
            let mut item = MusicItem { thumbnail: thumbnail_of(v), ..Default::default() };
            item.title = v.get("title").and_then(text)?;
            let lines = read_runs(&[v.get("subtitle").unwrap_or(&Value::Null)], &mut item);
            item.subtitle = lines.join(" • ");
            let nav = v.pointer("/title/runs/0/navigationEndpoint").or_else(|| v.get("onTap"))?;
            if let Some(id) = nav.pointer("/watchEndpoint/videoId").and_then(Value::as_str) {
                item.id = id.to_string();
                item.kind = video_kind(nav).to_string();
            } else {
                item.id = nav.pointer("/browseEndpoint/browseId")?.as_str()?.to_string();
                item.kind = kind_of_page(page_type(nav).unwrap_or("")).to_string();
            }
            let mut items = vec![item];
            items.extend(items_of(v.get("contents").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])));
            Some(Shelf { title: "Top result".into(), style: "list".into(), items, text: None, more: None })
        }
        _ => None,
    }
}

fn read_header(data: &Value) -> Option<Header> {
    let (key, h) = ["musicImmersiveHeaderRenderer", "musicVisualHeaderRenderer", "musicResponsiveHeaderRenderer", "musicDetailHeaderRenderer", "musicHeaderRenderer"]
        .iter()
        .find_map(|k| first(data, k).map(|h| (*k, h)))?;
    let mut scratch = MusicItem::default();
    let mut header = Header { title: h.get("title").and_then(text)?, ..Default::default() };
    header.subtitle = read_runs(&[h.get("subtitle").or_else(|| h.get("straplineTextOne")).unwrap_or(&Value::Null)], &mut scratch).join(" • ");
    header.artists = scratch.artists;
    header.second_subtitle = h.get("secondSubtitle").and_then(text);
    header.description = h
        .get("description")
        .and_then(|d| text(d).or_else(|| first(d, "description").and_then(text)))
        .filter(|s| !s.is_empty());
    header.thumbnail = thumbnail_of(h).or_else(|| picture(first(h.get("thumbnail").unwrap_or(&Value::Null), "thumbnails")));
    if let Some(s) = first(h, "subscribeButtonRenderer") {
        header.channel_id = s.get("channelId").and_then(Value::as_str).map(str::to_string);
        header.subscribed = s.get("subscribed").and_then(Value::as_bool);
        header.subscribers = s.get("longSubscriberCountText").or_else(|| s.get("subscriberCountText")).and_then(text);
    }
    header.radio = h
        .pointer("/startRadioButton/buttonRenderer/navigationEndpoint/watchPlaylistEndpoint/playlistId")
        .or_else(|| h.pointer("/startRadioButton/buttonRenderer/navigationEndpoint/watchEndpoint/playlistId"))
        .and_then(Value::as_str)
        .map(str::to_string);
    if key != "musicImmersiveHeaderRenderer" && key != "musicVisualHeaderRenderer" {
        // Albums and playlists: the list behind their play button, and the one to save.
        let mut lists = Vec::new();
        find_all(h, "playlistId", &mut |p| {
            if let Some(s) = p.as_str() {
                lists.push(s.to_string());
            }
        });
        header.list_id = lists.into_iter().find(|l| !l.starts_with("RDAMPL") && !l.starts_with("RDAO"));
    }
    if header.list_id.is_none() {
        header.list_id = data
            .pointer("/microformat/microformatDataRenderer/urlCanonical")
            .and_then(Value::as_str)
            .and_then(|u| u.split("list=").nth(1))
            .map(|s| s.split('&').next().unwrap_or(s).to_string());
    }
    Some(header)
}

/// The continuation handle of a page's last list (`nextContinuationData`, or a
/// `continuationItemRenderer` at the end of a list).
fn next_token(v: &Value) -> Option<String> {
    let mut found = None;
    find_all(v, "nextContinuationData", &mut |n| {
        if let Some(t) = n.get("continuation").and_then(Value::as_str) {
            found = Some(t.to_string());
        }
    });
    if found.is_none() {
        find_all(v, "continuationItemRenderer", &mut |c| {
            if let Some(t) = first(c, "token").and_then(Value::as_str) {
                found = Some(t.to_string());
            }
        });
    }
    found
}

/// Any Music page answer (a page, more of a page, a search) as shelves.
pub fn read_page(data: &Value) -> (MusicPage, Option<More>) {
    let mut page = MusicPage { header: read_header(data), ..Default::default() };
    // More rows of one list.
    if let Some(c) = data.get("continuationContents") {
        for key in ["musicShelfContinuation", "musicPlaylistShelfContinuation", "gridContinuation"] {
            if let Some(list) = c.get(key) {
                let items = items_of(list.get("contents").or_else(|| list.get("items")).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]));
                page.shelves.push(Shelf { style: if key == "gridContinuation" { "grid" } else { "list" }.into(), items, ..Default::default() });
                page.append = true;
                let next = next_token(list).map(|token| More { endpoint: "browse", token });
                return (page, next);
            }
        }
        if let Some(list) = c.get("sectionListContinuation") {
            for s in list.get("contents").and_then(Value::as_array).into_iter().flatten() {
                if let Some((k, v)) = s.as_object().and_then(|o| o.iter().next()) {
                    page.shelves.extend(shelf(k, v));
                }
            }
            let next = list.get("continuations").and_then(next_token).map(|token| More { endpoint: "browse", token });
            return (page, next);
        }
    }
    // Newer "load more" answers.
    if let Some(actions) = data.get("onResponseReceivedActions").and_then(Value::as_array) {
        for a in actions {
            if let Some(items) = a.pointer("/appendContinuationItemsAction/continuationItems").and_then(Value::as_array) {
                page.shelves.push(Shelf { style: "list".into(), items: items_of(items), ..Default::default() });
                page.append = true;
                let next = next_token(&Value::Array(items.clone())).map(|token| More { endpoint: "browse", token });
                return (page, next);
            }
        }
    }
    // A whole page: every section list (album and playlist pages have two: the header's
    // and the tracks').
    let mut lists = Vec::new();
    find_all(data, "sectionListRenderer", &mut |l| lists.push(l.clone()));
    let mut next = None;
    for list in &lists {
        for s in list.get("contents").and_then(Value::as_array).into_iter().flatten() {
            if let Some((k, v)) = s.as_object().and_then(|o| o.iter().next()) {
                if let Some(sh) = shelf(k, v) {
                    if sh.style == "list" && sh.items.is_empty() && sh.title.is_empty() {
                        continue;
                    }
                    if let Some(t) = next_token(v) {
                        next = Some(t);
                    }
                    page.shelves.push(sh);
                }
            }
        }
        if let Some(t) = list.get("continuations").and_then(next_token) {
            next = Some(t);
        }
    }
    // Album tracks carry no picture and often no artist: they are the album's.
    if let Some(h) = &page.header {
        for sh in &mut page.shelves {
            if sh.style != "list" {
                continue;
            }
            for i in &mut sh.items {
                if i.thumbnail.is_none() {
                    i.thumbnail = h.thumbnail.clone();
                }
                if i.artists.is_empty() && !h.artists.is_empty() {
                    i.artists = h.artists.clone();
                    if i.subtitle.is_empty() {
                        i.subtitle = h.artists.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ");
                    }
                }
            }
        }
    }
    (page, next.map(|token| More { endpoint: "browse", token }))
}

pub fn read_queue(data: &Value) -> (Vec<MusicItem>, Option<More>) {
    let mut items = Vec::new();
    find_all(data, "playlistPanelRenderer", &mut |p| {
        if items.is_empty() {
            items = items_of(p.get("contents").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[]));
        }
    });
    if items.is_empty() {
        if let Some(c) = data.pointer("/continuationContents/playlistPanelContinuation/contents").and_then(Value::as_array) {
            items = items_of(c);
        }
    }
    let mut next = None;
    find_all(data, "nextRadioContinuationData", &mut |n| {
        if let Some(t) = n.get("continuation").and_then(Value::as_str) {
            next = Some(t.to_string());
        }
    });
    if next.is_none() {
        find_all(data, "nextContinuationData", &mut |n| {
            if let Some(t) = n.get("continuation").and_then(Value::as_str) {
                next = Some(t.to_string());
            }
        });
    }
    (items, next.map(|token| More { endpoint: "next", token }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_rows_cards_and_buttons() {
        let data = json!({"contents": {"singleColumnBrowseResultsRenderer": {"tabs": [{"tabRenderer": {"content": {"sectionListRenderer": {"contents": [
            {"musicCarouselShelfRenderer": {
                "header": {"musicCarouselShelfBasicHeaderRenderer": {"title": {"runs": [{"text": "Albums"}]}}},
                "contents": [{"musicTwoRowItemRenderer": {
                    "thumbnailRenderer": {"musicThumbnailRenderer": {"thumbnail": {"thumbnails": [{"url": "https://lh3.googleusercontent.com/x=w226-h226-l90-rj", "width": 226}]}}},
                    "title": {"runs": [{"text": "Discovery"}]},
                    "subtitle": {"runs": [{"text": "Album"}, {"text": " • "}, {"text": "2001"}]},
                    "navigationEndpoint": {"browseEndpoint": {"browseId": "MPREb_x", "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ALBUM"}}}}}}]}},
            {"musicShelfRenderer": {"title": {"runs": [{"text": "Songs"}]}, "contents": [{"musicResponsiveListItemRenderer": {
                "flexColumns": [
                    {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [{"text": "One More Time", "navigationEndpoint": {"watchEndpoint": {"videoId": "FGBhQbmPwH8",
                        "watchEndpointMusicSupportedConfigs": {"watchEndpointMusicConfig": {"musicVideoType": "MUSIC_VIDEO_TYPE_ATV"}}}}}]}}},
                    {"musicResponsiveListItemFlexColumnRenderer": {"text": {"runs": [
                        {"text": "Daft Punk", "navigationEndpoint": {"browseEndpoint": {"browseId": "UCdp", "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ARTIST"}}}}},
                        {"text": " • "},
                        {"text": "Discovery", "navigationEndpoint": {"browseEndpoint": {"browseId": "MPREb_x", "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ALBUM"}}}}},
                        {"text": " • "}, {"text": "5:21"}]}}}],
                "playlistItemData": {"videoId": "FGBhQbmPwH8"}}}]}},
            {"musicCarouselShelfRenderer": {"contents": [{"musicNavigationButtonRenderer": {"buttonText": {"runs": [{"text": "Chill"}]},
                "solid": {"leftStripeColor": 4294961024u64}, "clickCommand": {"browseEndpoint": {"browseId": "FEmusic_moods_and_genres_category", "params": "p"}}}}]}}
        ], "continuations": [{"nextContinuationData": {"continuation": "more"}}]}}}}]}}});
        let (page, more) = read_page(&data);
        assert_eq!(more.unwrap().token, "more");
        assert_eq!(page.shelves.len(), 3);
        let album = &page.shelves[0].items[0];
        assert_eq!((album.kind.as_str(), album.id.as_str(), page.shelves[0].style.as_str()), ("album", "MPREb_x", "carousel"));
        assert_eq!(album.thumbnail.as_deref(), Some("https://lh3.googleusercontent.com/x=w544-h544-l90-rj"));
        let song = &page.shelves[1].items[0];
        assert_eq!((song.kind.as_str(), song.id.as_str(), song.duration.as_deref()), ("song", "FGBhQbmPwH8", Some("5:21")));
        assert_eq!(song.artists[0].id.as_deref(), Some("UCdp"));
        assert_eq!(song.album.as_ref().unwrap().name, "Discovery");
        let mood = &page.shelves[2].items[0];
        assert_eq!((mood.kind.as_str(), mood.params.as_deref(), page.shelves[2].style.as_str()), ("mood", Some("p"), "buttons"));
    }

    #[test]
    fn reads_a_queue() {
        let data = json!({"contents": {"x": {"playlistPanelRenderer": {"contents": [{"playlistPanelVideoRenderer": {
            "videoId": "aaaaaaaaaaa", "title": {"runs": [{"text": "Song"}]}, "lengthText": {"runs": [{"text": "3:10"}]},
            "longBylineText": {"runs": [{"text": "Artist", "navigationEndpoint": {"browseEndpoint": {"browseId": "UCa",
                "browseEndpointContextSupportedConfigs": {"browseEndpointContextMusicConfig": {"pageType": "MUSIC_PAGE_TYPE_ARTIST"}}}}}]},
            "navigationEndpoint": {"watchEndpoint": {"videoId": "aaaaaaaaaaa", "watchEndpointMusicSupportedConfigs": {"watchEndpointMusicConfig": {"musicVideoType": "MUSIC_VIDEO_TYPE_ATV"}}}}}}],
            "continuations": [{"nextRadioContinuationData": {"continuation": "radio"}}]}}}});
        let (items, next) = read_queue(&data);
        assert_eq!((items[0].kind.as_str(), items[0].duration.as_deref(), items[0].artists[0].name.as_str()), ("song", Some("3:10"), "Artist"));
        assert_eq!(next.unwrap().token, "radio");
    }
}
