//! Thin wrapper over rustypipe: builds the client, maps results to UI models and keeps
//! continuation state behind opaque [`Cursor`]s.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::{Mutex, RwLock};
use rustypipe::client::{ClientType, RustyPipe, RustyPipeQuery};
use rustypipe::model::richtext::ToPlaintext;
use rustypipe::error::{AuthError, Error as RpError};
use rustypipe::model::{
    ChannelItem, PlaylistItem, VideoItem, VideoPlayer, YouTubeItem, paginator::Paginator,
};

use crate::config::Paths;
use crate::model::*;
use crate::secure::{EncryptedCacheStorage, EncryptedFile, Key};

#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    #[error("{0}")]
    Yt(#[from] RpError),
    #[error("this list has no more pages")]
    CursorGone,
    #[error("sign in to see this")]
    NeedsLogin,
}

impl ApiError {
    pub fn is_auth(&self) -> bool {
        matches!(self, ApiError::NeedsLogin)
            || matches!(self, ApiError::Yt(RpError::Auth(AuthError::NoLogin)))
            || matches!(self, ApiError::Yt(RpError::HttpStatus(401, _)))
    }
}

enum Pager {
    Mixed(Paginator<YouTubeItem>),
    Videos(Paginator<VideoItem>),
    Playlists(Paginator<PlaylistItem>),
    Channels(Paginator<ChannelItem>),
    Comments(Paginator<rustypipe::model::Comment>),
}

pub struct Api {
    rp: RustyPipe,
    clients: RwLock<Vec<ClientType>>,
    pagers: Mutex<HashMap<u64, Pager>>,
    /// Comment handles of recently opened videos, by video id.
    comment_pagers: Mutex<HashMap<String, Paginator<rustypipe::model::Comment>>>,
    next_cursor: AtomicU64,
}

pub struct ApiOptions {
    pub key: Key,
    /// Path to the `rustypipe-botguard` binary. `None` searches next to the exe and on PATH.
    pub botguard: Option<PathBuf>,
}

impl Api {
    pub fn new(paths: &Paths, opts: ApiOptions) -> Result<Self, ApiError> {
        let storage = EncryptedCacheStorage(EncryptedFile::new(
            paths.data.join("session.bin"),
            &opts.key,
        ));
        let mut b = RustyPipe::builder()
            .storage(Box::new(storage))
            .no_reporter()
            .timeout(Duration::from_secs(20))
            .n_http_retries(2)
            .po_token_cache()
            .botguard_snapshot_file(paths.cache.join("botguard-snapshot.bin"));
        let botguard = opts.botguard.or_else(|| {
            let exe = std::env::current_exe().ok()?;
            let name = if cfg!(windows) {
                "rustypipe-botguard.exe"
            } else {
                "rustypipe-botguard"
            };
            let p = exe.parent()?.join(name);
            p.exists().then_some(p)
        });
        if let Some(bin) = botguard {
            b = b.botguard_bin(bin);
        }
        let rp = b.build_with_client(crate::service::http_builder())?;
        Ok(Self {
            rp,
            clients: RwLock::new(crate::remote::RemoteConfig::default().client_types()),
            pagers: Mutex::new(HashMap::new()),
            comment_pagers: Mutex::new(HashMap::new()),
            next_cursor: AtomicU64::new(1),
        })
    }

    pub fn rp(&self) -> &RustyPipe {
        &self.rp
    }

    fn q(&self) -> RustyPipeQuery {
        self.rp.query()
    }

    pub fn set_client_order(&self, order: Vec<ClientType>) {
        if !order.is_empty() {
            *self.clients.write() = order;
        }
    }

    pub fn client_order(&self) -> Vec<ClientType> {
        self.clients.read().clone()
    }

    pub async fn botguard_version(&self) -> Option<String> {
        self.rp.version_botguard().await
    }

    fn page_comments(&self, p: Paginator<rustypipe::model::Comment>) -> CommentPage {
        let items = p
            .items
            .iter()
            .map(|c| CommentCard {
                id: c.id.clone(),
                author: c.author.as_ref().map(|a| a.name.clone()),
                author_id: c.author.as_ref().map(|a| a.id.clone()),
                avatar: c.author.as_ref().and_then(|a| best_thumb(&a.avatar, 88)),
                text: c.text.to_plaintext(),
                published: c.publish_date_txt.clone(),
                likes: c.like_count,
                replies: c.reply_count,
                pinned: c.pinned,
                by_owner: c.by_owner,
                hearted: c.hearted,
                replies_token: None,
                reply_params: None,
            })
            .collect();
        let more = !p.is_exhausted();
        CommentPage {
            items,
            next: self.park(Pager::Comments(p), more),
            create_params: None,
        }
    }

    /// The first page of a video's top comments. Loading the video's details first leaves
    /// the comment handle ready, so this is then a single request.
    pub async fn comments(&self, id: &str) -> Result<CommentPage, ApiError> {
        let known = self.comment_pagers.lock().get(id).cloned();
        let pager = match known {
            Some(p) => p,
            None => self.q().video_details(id).await?.top_comments,
        };
        Ok(match pager.next(&self.q()).await? {
            Some(p) => self.page_comments(p),
            None => CommentPage::default(),
        })
    }

    /// The page of comments after `cursor` (single use).
    pub async fn more_comments(&self, cursor: Cursor) -> Result<CommentPage, ApiError> {
        let pager = self.pagers.lock().remove(&cursor.0).ok_or(ApiError::CursorGone)?;
        match pager {
            Pager::Comments(p) => Ok(match p.next(&self.q()).await? {
                Some(n) => self.page_comments(n),
                None => CommentPage::default(),
            }),
            _ => Err(ApiError::CursorGone),
        }
    }

    fn park(&self, pager: Pager, has_more: bool) -> Option<Cursor> {
        if !has_more {
            return None;
        }
        let id = self.next_cursor.fetch_add(1, Ordering::Relaxed);
        self.pagers.lock().insert(id, pager);
        Some(Cursor(id))
    }

    fn page_mixed(&self, p: Paginator<YouTubeItem>) -> Page {
        let items = p.items.iter().cloned().map(Item::from).collect();
        let more = !p.is_exhausted();
        Page {
            items,
            next: self.park(Pager::Mixed(p), more),
        }
    }

    fn page_videos(&self, p: Paginator<VideoItem>) -> Page {
        let items = p
            .items
            .iter()
            .cloned()
            .map(|v| Item::Video(v.into()))
            .collect();
        let more = !p.is_exhausted();
        Page {
            items,
            next: self.park(Pager::Videos(p), more),
        }
    }

    fn page_playlists(&self, p: Paginator<PlaylistItem>) -> Page {
        let items = p
            .items
            .iter()
            .cloned()
            .map(|v| Item::Playlist(v.into()))
            .collect();
        let more = !p.is_exhausted();
        Page {
            items,
            next: self.park(Pager::Playlists(p), more),
        }
    }

    fn page_channels(&self, p: Paginator<ChannelItem>) -> Page {
        let items = p
            .items
            .iter()
            .cloned()
            .map(|v| Item::Channel(v.into()))
            .collect();
        let more = !p.is_exhausted();
        Page {
            items,
            next: self.park(Pager::Channels(p), more),
        }
    }

    /// Fetch the page after `cursor`. Each cursor is single-use.
    pub async fn more(&self, cursor: Cursor) -> Result<Page, ApiError> {
        let pager = self
            .pagers
            .lock()
            .remove(&cursor.0)
            .ok_or(ApiError::CursorGone)?;
        let q = self.q();
        Ok(match pager {
            Pager::Mixed(p) => match p.next(&q).await? {
                Some(n) => self.page_mixed(n),
                None => Page::default(),
            },
            Pager::Videos(p) => match p.next(&q).await? {
                Some(n) => self.page_videos(n),
                None => Page::default(),
            },
            Pager::Playlists(p) => match p.next(&q).await? {
                Some(n) => self.page_playlists(n),
                None => Page::default(),
            },
            Pager::Channels(p) => match p.next(&q).await? {
                Some(n) => self.page_channels(n),
                None => Page::default(),
            },
            // Comments have their own page type (see `more_comments`).
            Pager::Comments(_) => return Err(ApiError::CursorGone),
        })
    }

    /// Forget a cursor the UI no longer needs (e.g. after navigating away).
    pub fn drop_cursor(&self, cursor: Cursor) {
        self.pagers.lock().remove(&cursor.0);
    }

    pub async fn search(&self, query: &str) -> Result<Page, ApiError> {
        let r = self.q().search::<YouTubeItem, _>(query).await?;
        Ok(self.page_mixed(r.items))
    }

    pub async fn suggestions(&self, query: &str) -> Result<Vec<String>, ApiError> {
        Ok(self.q().search_suggestion(query).await?)
    }

    /// Signed-out home feed. YouTube's trending page is not available in every region,
    /// so this falls back to a popular-music search.
    pub async fn home(&self) -> Result<Page, ApiError> {
        match self.q().trending().await {
            Ok(v) if !v.is_empty() => Ok(Page {
                items: v.into_iter().map(|v| Item::Video(v.into())).collect(),
                next: None,
            }),
            Ok(_) | Err(_) => self.search("popular music videos").await,
        }
    }

    pub async fn channel(&self, id: &str) -> Result<ChannelPage, ApiError> {
        let c = self.q().channel_videos(id).await?;
        Ok(ChannelPage {
            channel: ChannelCard {
                avatar: best_thumb(&c.avatar, 88),
                id: c.id,
                name: c.name,
                subscribers: c.subscriber_count,
            },
            banner: best_thumb(&c.banner, 1280),
            description: c.description,
            videos: self.page_videos(c.content),
            handle: c.handle,
            video_count: c.video_count,
            verified: c.verification.verified(),
            has_shorts: c.has_shorts,
            has_live: c.has_live,
        })
    }

    /// A channel's Shorts ("shorts") or live streams ("live").
    pub async fn channel_tab(&self, id: &str, tab: &str) -> Result<Page, ApiError> {
        let tab = match tab {
            "shorts" => rustypipe::param::ChannelVideoTab::Shorts,
            "live" => rustypipe::param::ChannelVideoTab::Live,
            _ => rustypipe::param::ChannelVideoTab::Videos,
        };
        let c = self.q().channel_videos_tab(id, tab).await?;
        Ok(self.page_videos(c.content))
    }

    /// What a channel's "About" says.
    pub async fn channel_about(&self, id: &str) -> Result<serde_json::Value, ApiError> {
        let i = self.q().channel_info(id).await?;
        Ok(serde_json::json!({
            "description": i.description,
            "subscribers": i.subscriber_count,
            "videos": i.video_count,
            "views": i.view_count,
            "joined": i.create_date.map(|d| format!("{} {}, {}", d.month(), d.day(), d.year())),
            "country": i.country.map(|c| c.name().to_string()),
            "url": i.url,
            "links": i.links.iter().map(|(name, url)| serde_json::json!({"name": name, "url": url})).collect::<Vec<_>>(),
        }))
    }

    /// Search with YouTube's filters: `date` hour/day/week/month/year, `kind`
    /// video/channel/playlist, `length` short/medium/long, `sort` rating/date/views.
    pub async fn search_filtered(&self, query: &str, f: &SearchOptions) -> Result<Page, ApiError> {
        use rustypipe::param::search_filter::{ItemType, Length, Order, SearchFilter, UploadDate};
        let mut filter = SearchFilter::new();
        filter = filter.date_opt(match f.date.as_deref() {
            Some("hour") => Some(UploadDate::Hour),
            Some("day") => Some(UploadDate::Day),
            Some("week") => Some(UploadDate::Week),
            Some("month") => Some(UploadDate::Month),
            Some("year") => Some(UploadDate::Year),
            _ => None,
        });
        filter = filter.item_type_opt(match f.kind.as_deref() {
            Some("video") => Some(ItemType::Video),
            Some("channel") => Some(ItemType::Channel),
            Some("playlist") => Some(ItemType::Playlist),
            _ => None,
        });
        filter = filter.length_opt(match f.length.as_deref() {
            Some("short") => Some(Length::Short),
            Some("medium") => Some(Length::Medium),
            Some("long") => Some(Length::Long),
            _ => None,
        });
        filter = filter.sort_opt(match f.sort.as_deref() {
            Some("rating") => Some(Order::Rating),
            Some("date") => Some(Order::Date),
            Some("views") => Some(Order::Views),
            _ => None,
        });
        if f.live {
            filter = filter.feature(rustypipe::param::search_filter::Feature::IsLive);
        }
        let r = self.q().search_filter::<YouTubeItem, _>(query, &filter).await?;
        Ok(self.page_mixed(r.items))
    }

    pub async fn channel_playlists(&self, id: &str) -> Result<Page, ApiError> {
        let c = self.q().channel_playlists(id).await?;
        Ok(self.page_playlists(c.content))
    }

    pub async fn playlist(&self, id: &str) -> Result<PlaylistPage, ApiError> {
        let p = self.q().playlist(id).await?;
        Ok(PlaylistPage {
            playlist: PlaylistCard {
                thumbnail: best_thumb(&p.thumbnail, THUMB_W),
                id: p.id,
                name: p.name,
                channel_name: p.channel.map(|c| c.name),
                video_count: Some(p.video_count),
            },
            videos: self.page_videos(p.videos),
        })
    }

    /// Every video id in a playlist, following all continuations (for playlist downloads).
    pub async fn playlist_all(&self, id: &str) -> Result<Vec<VideoCard>, ApiError> {
        let q = self.q();
        let mut p = q.playlist(id).await?;
        p.videos.extend_all(&q).await?;
        Ok(p.videos.items.into_iter().map(VideoCard::from).collect())
    }

    pub async fn video(&self, id: &str) -> Result<VideoPage, ApiError> {
        let d = self.q().video_details(id).await?;
        {
            // Keep the handle for this video's comments for when the user opens them.
            let mut known = self.comment_pagers.lock();
            if known.len() >= 30 {
                known.clear();
            }
            known.insert(id.to_string(), d.top_comments.clone());
        }
        let subscribers = d.channel.subscriber_count;
        let card = VideoCard {
            channel_avatar: best_thumb(&d.channel.avatar, 88),
            id: d.id,
            title: d.name,
            channel_id: Some(d.channel.id),
            channel_name: Some(d.channel.name),
            duration_s: None,
            thumbnail: None,
            views: Some(d.view_count),
            published: d.publish_date_txt,
            is_live: d.is_live,
            watched: None,
        };
        Ok(VideoPage {
            video: card,
            description: d.description.to_plaintext(),
            likes: d.like_count,
            chapters: d.chapters.iter().map(|c| Chapter { title: c.name.clone(), start: c.position }).collect(),
            subscribers: subscribers.map(|n| format!("{} subscribers", fmt_count(n))),
            related: self.page_videos(d.recommended),
        })
    }

    /// Resolve playable streams, walking the client order. `skip` excludes a client that
    /// just produced a 403 so the retry uses the next one.
    pub async fn player(&self, id: &str, skip: Option<ClientType>) -> Result<VideoPlayer, ApiError> {
        let order: Vec<ClientType> = self
            .client_order()
            .into_iter()
            .filter(|c| Some(*c) != skip)
            .collect();
        // Walk the order ourselves: rustypipe's own fallback stops at the first
        // deobfuscation error, which is the most common way a client breaks.
        let q = self.q();
        let mut last = None;
        for c in order {
            match q.player_from_client(id, c).await {
                Ok(p) => return Ok(p),
                Err(e @ RpError::Extraction(rustypipe::error::ExtractionError::Unavailable { .. })) => {
                    return Err(e.into());
                }
                Err(e) => {
                    tracing::warn!("player via {c:?} failed: {e}");
                    last = Some(e);
                }
            }
        }
        Err(last
            .map(ApiError::from)
            .unwrap_or_else(|| ApiError::Yt(RpError::Other("no player clients configured".into()))))
    }

    /// Recent uploads from many channels via their public RSS feeds, newest first.
    /// This is how the subscriptions feed is built when YouTube's own feed page can't be read.
    pub async fn rss_feed(&self, channels: &[ChannelCard], limit: usize) -> Page {
        use futures_util::StreamExt;
        let q = self.q();
        let mut videos: Vec<(time::OffsetDateTime, VideoCard)> = futures_util::stream::iter(channels.iter().cloned())
            .map(|c| {
                let q = q.clone();
                async move {
                    match q.channel_rss(&c.id).await {
                        Ok(feed) => feed
                            .videos
                            .into_iter()
                            .map(|v| {
                                let card = VideoCard {
                                    channel_avatar: c.avatar.clone(),
                                    thumbnail: Some(format!("https://i.ytimg.com/vi/{}/mqdefault.jpg", v.id)),
                                    id: v.id,
                                    title: v.name,
                                    channel_id: Some(c.id.clone()),
                                    channel_name: Some(feed.name.clone()),
                                    duration_s: None,
                                    views: Some(v.view_count).filter(|n| *n > 0),
                                    published: Some(ago(v.publish_date)),
                                    is_live: false,
                                    watched: None,
                                };
                                (v.publish_date, card)
                            })
                            .collect::<Vec<_>>(),
                        Err(e) => {
                            tracing::debug!("rss for {}: {e}", c.id);
                            Vec::new()
                        }
                    }
                }
            })
            .buffer_unordered(12)
            .flat_map(futures_util::stream::iter)
            .collect()
            .await;
        videos.sort_by(|a, b| b.0.cmp(&a.0));
        videos.truncate(limit);
        Page {
            items: videos.into_iter().map(|(_, v)| Item::Video(v)).collect(),
            next: None,
        }
    }

    // ---- signed-in data ----

    pub async fn subscription_feed(&self) -> Result<Page, ApiError> {
        let p = self.q().authenticated().subscription_feed().await?;
        Ok(self.page_videos(p))
    }

    pub async fn subscriptions(&self) -> Result<Page, ApiError> {
        let p = self.q().authenticated().subscriptions().await?;
        Ok(self.page_channels(p))
    }

    pub async fn history(&self) -> Result<Page, ApiError> {
        let p = self.q().authenticated().history().await?;
        let items = p
            .items
            .iter()
            .cloned()
            .map(|h| Item::Video(h.item.into()))
            .collect();
        Ok(Page { items, next: None })
    }

    pub async fn saved_playlists(&self) -> Result<Page, ApiError> {
        let p = self.q().authenticated().saved_playlists().await?;
        Ok(self.page_playlists(p))
    }
}

/// Search filters, as the UI names them. `None` = any.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct SearchOptions {
    #[serde(default)]
    pub date: Option<String>,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub length: Option<String>,
    #[serde(default)]
    pub sort: Option<String>,
    #[serde(default)]
    pub live: bool,
}

impl SearchOptions {
    pub fn is_empty(&self) -> bool {
        self.date.is_none() && self.kind.is_none() && self.length.is_none() && self.sort.is_none() && !self.live
    }
}

/// "3 days ago" style text.
pub fn ago(t: time::OffsetDateTime) -> String {
    let secs = (time::OffsetDateTime::now_utc() - t).whole_seconds().max(0);
    let (n, unit) = match secs {
        0..=59 => return "just now".into(),
        60..=3599 => (secs / 60, "minute"),
        3600..=86_399 => (secs / 3600, "hour"),
        86_400..=604_799 => (secs / 86_400, "day"),
        604_800..=2_629_799 => (secs / 604_800, "week"),
        2_629_800..=31_557_599 => (secs / 2_629_800, "month"),
        _ => (secs / 31_557_600, "year"),
    };
    format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" })
}
