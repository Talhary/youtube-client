//! UI-facing data types. These hide rustypipe's types so the shells depend only on core.

use rustypipe::model::{
    ChannelItem, ChannelTag, PlaylistItem, Thumbnail, VideoItem, YouTubeItem,
};
use serde::{Deserialize, Serialize};

/// Opaque handle for the next page of a list. Valid until used once.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Cursor(pub u64);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VideoCard {
    pub id: String,
    pub title: String,
    pub channel_id: Option<String>,
    pub channel_name: Option<String>,
    /// The channel's picture, when the list carries one.
    #[serde(default)]
    pub channel_avatar: Option<String>,
    pub duration_s: Option<u32>,
    pub thumbnail: Option<String>,
    pub views: Option<u64>,
    pub published: Option<String>,
    pub is_live: bool,
    /// How much of it the signed-in user has watched, in percent, when known.
    #[serde(default)]
    pub watched: Option<u8>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelCard {
    pub id: String,
    pub name: String,
    pub avatar: Option<String>,
    pub subscribers: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlaylistCard {
    pub id: String,
    pub name: String,
    pub thumbnail: Option<String>,
    pub channel_name: Option<String>,
    pub video_count: Option<u64>,
}

/// A channel's post (the "Posts" tab, sometimes also in feeds).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PostCard {
    pub id: String,
    pub author: Option<String>,
    pub author_id: Option<String>,
    pub author_avatar: Option<String>,
    pub text: String,
    /// As YouTube words it, e.g. "2 days ago".
    pub published: Option<String>,
    pub likes: Option<String>,
    /// Pictures attached to the post.
    pub images: Vec<String>,
    /// A video attached to the post.
    pub video: Option<VideoCard>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Item {
    Video(VideoCard),
    Channel(ChannelCard),
    Playlist(PlaylistCard),
    Post(PostCard),
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Page {
    pub items: Vec<Item>,
    pub next: Option<Cursor>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelPage {
    pub channel: ChannelCard,
    pub banner: Option<String>,
    pub description: String,
    pub videos: Page,
    /// "@name".
    #[serde(default)]
    pub handle: Option<String>,
    #[serde(default)]
    pub video_count: Option<u64>,
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub has_shorts: bool,
    #[serde(default)]
    pub has_live: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlaylistPage {
    pub playlist: PlaylistCard,
    pub videos: Page,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VideoPage {
    pub video: VideoCard,
    pub description: String,
    pub likes: Option<u32>,
    pub related: Page,
    #[serde(default)]
    pub chapters: Vec<Chapter>,
    /// The channel's subscribers as YouTube words it ("1.2M subscribers").
    #[serde(default)]
    pub subscribers: Option<String>,
}

/// A named part of a video.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Chapter {
    pub title: String,
    /// Where it starts, in seconds.
    pub start: u32,
}

/// One comment under a video.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CommentCard {
    pub id: String,
    pub author: Option<String>,
    pub author_id: Option<String>,
    pub avatar: Option<String>,
    pub text: String,
    /// As YouTube words it, e.g. "3 days ago".
    pub published: String,
    pub likes: Option<u32>,
    pub replies: u32,
    pub pinned: bool,
    /// Written by the video's own channel.
    pub by_owner: bool,
    /// Marked with a heart by the video's channel.
    pub hearted: bool,
    /// Loads this comment's replies (YouTube's page API handle).
    #[serde(default)]
    pub replies_token: Option<String>,
    /// Posts a reply to this comment's thread (signed in only).
    #[serde(default)]
    pub reply_params: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CommentPage {
    pub items: Vec<CommentCard>,
    pub next: Option<Cursor>,
    /// Posts a new comment on the video (signed in only).
    #[serde(default)]
    pub create_params: Option<String>,
}

/// Pick the thumbnail closest to `target` px wide without going far under it.
pub fn best_thumb(thumbs: &[Thumbnail], target: u32) -> Option<String> {
    thumbs
        .iter()
        .min_by_key(|t| {
            if t.width >= target {
                t.width - target
            } else {
                (target - t.width) * 4
            }
        })
        .map(|t| t.url.clone())
}

pub(crate) const THUMB_W: u32 = 360;

impl From<VideoItem> for VideoCard {
    fn from(v: VideoItem) -> Self {
        let (channel_id, channel_name) = split_tag(v.channel.as_ref());
        Self {
            channel_avatar: v.channel.as_ref().and_then(|c| best_thumb(&c.avatar, 88)),
            thumbnail: best_thumb(&v.thumbnail, THUMB_W),
            id: v.id,
            title: v.name,
            channel_id,
            channel_name,
            duration_s: v.duration,
            views: v.view_count,
            published: v.publish_date_txt,
            is_live: v.is_live,
            watched: None,
        }
    }
}

impl From<ChannelItem> for ChannelCard {
    fn from(c: ChannelItem) -> Self {
        Self {
            avatar: best_thumb(&c.avatar, 88),
            id: c.id,
            name: c.name,
            subscribers: c.subscriber_count,
        }
    }
}

impl From<PlaylistItem> for PlaylistCard {
    fn from(p: PlaylistItem) -> Self {
        Self {
            thumbnail: best_thumb(&p.thumbnail, THUMB_W),
            channel_name: p.channel.map(|c| c.name),
            id: p.id,
            name: p.name,
            video_count: p.video_count,
        }
    }
}

impl From<YouTubeItem> for Item {
    fn from(i: YouTubeItem) -> Self {
        match i {
            YouTubeItem::Video(v) => Item::Video(v.into()),
            YouTubeItem::Channel(c) => Item::Channel(c.into()),
            YouTubeItem::Playlist(p) => Item::Playlist(p.into()),
        }
    }
}

fn split_tag(t: Option<&ChannelTag>) -> (Option<String>, Option<String>) {
    match t {
        Some(t) => (Some(t.id.clone()), Some(t.name.clone())),
        None => (None, None),
    }
}

/// "1:02:03" / "4:05".
pub fn fmt_duration(s: u32) -> String {
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// "1.2M views".
pub fn fmt_count(n: u64) -> String {
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => trim(n as f64 / 1e3, "K"),
        1_000_000..=999_999_999 => trim(n as f64 / 1e6, "M"),
        _ => trim(n as f64 / 1e9, "B"),
    }
}

fn trim(v: f64, unit: &str) -> String {
    if v >= 100.0 {
        format!("{v:.0}{unit}")
    } else {
        let s = format!("{v:.1}");
        format!("{}{unit}", s.trim_end_matches(".0"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(fmt_duration(65), "1:05");
        assert_eq!(fmt_duration(3723), "1:02:03");
        assert_eq!(fmt_count(999), "999");
        assert_eq!(fmt_count(1_000), "1K");
        assert_eq!(fmt_count(1_250_000), "1.2M");
        assert_eq!(fmt_count(512_000_000), "512M");
    }
}
