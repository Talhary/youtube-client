//! Stream selection (PRD 3.2 and 3.3): quality list, hardware-codec filtering, auto quality,
//! audio choice and output container.

use rustypipe::model::{AudioCodec, AudioStream, VideoCodec, VideoPlayer, VideoStream};
use serde::{Deserialize, Serialize};

use crate::config::{AudioOut, CodecPref};

/// Which video codecs this device decodes in hardware. Filled by the shell
/// (Android `MediaCodecList`, desktop mpv `hwdec` probe).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HwCaps {
    pub h264: bool,
    pub vp9: bool,
    pub av1: bool,
}

impl Default for HwCaps {
    fn default() -> Self {
        Self {
            h264: true,
            vp9: false,
            av1: false,
        }
    }
}


/// One selectable stream, without rustypipe types.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct StreamChoice {
    pub url: String,
    pub itag: u32,
    pub mime: String,
    pub codec: String,
    pub height: u32,
    pub fps: u8,
    pub bitrate: u32,
    pub size: Option<u64>,
    /// HTTP headers the server expects when fetching this URL (from yt-dlp).
    #[serde(default)]
    pub headers: Vec<(String, String)>,
    /// Byte range (inclusive) of the stream's initialization segment, when known.
    #[serde(default)]
    pub init_range: Option<(u64, u64)>,
    /// Byte range (inclusive) of the stream's segment index, when known.
    #[serde(default)]
    pub index_range: Option<(u64, u64)>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct QualityOption {
    pub label: String,
    pub height: u32,
    pub fps: u8,
    pub stream: StreamChoice,
}

pub fn codec_name(c: VideoCodec) -> &'static str {
    match c {
        VideoCodec::Avc1 => "h264",
        VideoCodec::Vp9 => "vp9",
        VideoCodec::Av01 => "av1",
        VideoCodec::Mp4v => "mp4v",
        _ => "unknown",
    }
}

fn audio_codec_name(c: AudioCodec) -> &'static str {
    match c {
        AudioCodec::Mp4a => "aac",
        AudioCodec::Opus => "opus",
        AudioCodec::Ac3 => "ac3",
        AudioCodec::Ec3 => "ec3",
        _ => "unknown",
    }
}

fn video_choice(v: &VideoStream) -> StreamChoice {
    StreamChoice {
        url: v.url.clone(),
        itag: v.itag,
        mime: v.mime.clone(),
        codec: codec_name(v.codec).into(),
        height: v.height,
        fps: v.fps,
        bitrate: v.bitrate,
        size: v.size,
        headers: Vec::new(),
        init_range: None,
        index_range: None,
    }
}

pub fn audio_choice(a: &AudioStream) -> StreamChoice {
    StreamChoice {
        url: a.url.clone(),
        itag: a.itag,
        mime: a.mime.clone(),
        codec: audio_codec_name(a.codec).into(),
        height: 0,
        fps: 0,
        bitrate: a.bitrate,
        size: Some(a.size),
        headers: Vec::new(),
        init_range: None,
        index_range: None,
    }
}

/// Codec preference among hardware-decodable ones: smaller files first.
fn codec_rank(codec: &str, caps: &HwCaps) -> Option<u8> {
    match codec {
        "av1" if caps.av1 => Some(3),
        "vp9" if caps.vp9 => Some(2),
        "h264" if caps.h264 => Some(1),
        _ => None,
    }
}

/// One entry per (height, fps), using the best hardware-decodable codec for that rung.
///
/// Rungs no hardware decoder can play are hidden; that drops 1440p/4K (VP9/AV1 only) on
/// devices without VP9/AV1 hardware. If nothing is hardware-decodable at all, H.264 is
/// offered so the user still gets video.
pub fn qualities(streams: &[VideoStream], caps: &HwCaps) -> Vec<QualityOption> {
    let usable: Vec<StreamChoice> = streams
        .iter()
        .filter(|s| s.drm_track_type.is_none() && !s.hdr)
        .map(video_choice)
        .collect();
    qualities_from(&usable, caps)
}

/// [`qualities`] for streams that did not come from rustypipe (e.g. yt-dlp).
pub fn qualities_from(usable: &[StreamChoice], caps: &HwCaps) -> Vec<QualityOption> {
    let mut best: Vec<(&StreamChoice, u8)> = Vec::new();
    for s in usable {
        let Some(rank) = codec_rank(&s.codec, caps) else {
            continue;
        };
        match best
            .iter_mut()
            .find(|(b, _)| b.height == s.height && b.fps == s.fps)
        {
            Some(slot) if rank > slot.1 || (rank == slot.1 && s.bitrate > slot.0.bitrate) => {
                *slot = (s, rank)
            }
            Some(_) => {}
            None => best.push((s, rank)),
        }
    }
    if best.is_empty() {
        let soft = HwCaps {
            h264: true,
            vp9: false,
            av1: false,
        };
        if *caps != soft {
            return qualities_from(usable, &soft);
        }
    }
    best.sort_by(|a, b| (b.0.height, b.0.fps).cmp(&(a.0.height, a.0.fps)));
    best.into_iter()
        .map(|(s, _)| QualityOption {
            label: if s.fps > 30 {
                format!("{}p{}", s.height, s.fps)
            } else {
                format!("{}p", s.height)
            },
            height: s.height,
            fps: s.fps,
            stream: s.clone(),
        })
        .collect()
}

/// Auto quality never starts below this height when the connection sustains it.
const AUTO_MIN_HEIGHT: u32 = 720;

/// Auto quality: the smallest rung that covers the player's pixel height (at least
/// [`AUTO_MIN_HEIGHT`]) among those whose bitrate the measured throughput sustains with 30%
/// headroom; if none is that tall, the tallest sustainable one. Falls back to the lowest rung.
pub fn pick_auto(
    options: &[QualityOption],
    player_px_height: u32,
    throughput_bps: Option<u64>,
) -> Option<&QualityOption> {
    let target = player_px_height.max(AUTO_MIN_HEIGHT);
    // `options` is sorted tallest first, higher frame rate first within a height.
    let mut fits = options
        .iter()
        .filter(|o| throughput_bps.is_none_or(|t| (o.stream.bitrate as u64) * 13 / 10 <= t));
    let Some(tallest) = fits.next() else {
        return options.last();
    };
    let cover = std::iter::once(tallest)
        .chain(fits)
        .filter(|o| o.height >= target)
        .map(|o| o.height)
        .min();
    match cover {
        Some(h) => options.iter().find(|o| {
            o.height == h && throughput_bps.is_none_or(|t| (o.stream.bitrate as u64) * 13 / 10 <= t)
        }),
        None => Some(tallest),
    }
}

/// Highest rung at or below `max_height`.
pub fn pick_max(options: &[QualityOption], max_height: u32) -> Option<&QualityOption> {
    options
        .iter()
        .find(|o| o.height <= max_height)
        .or_else(|| options.last())
}

/// Best audio stream of the preferred codec, else the best of any. Default-language track wins.
pub fn pick_audio(streams: &[AudioStream], prefer: AudioCodec) -> Option<&AudioStream> {
    let ok = |a: &&AudioStream| {
        a.drm_track_type.is_none()
            && matches!(a.codec, AudioCodec::Mp4a | AudioCodec::Opus)
            && a.track.as_ref().is_none_or(|t| t.is_default)
    };
    let best_of = |codec: Option<AudioCodec>| {
        streams
            .iter()
            .filter(ok)
            .filter(|a| codec.is_none_or(|c| a.codec == c))
            .max_by_key(|a| a.bitrate)
    };
    best_of(Some(prefer)).or_else(|| best_of(None))
}

/// The audio codec that pairs with a video codec in a single container.
pub fn audio_for_video(video_codec: &str) -> AudioCodec {
    match video_codec {
        "h264" => AudioCodec::Mp4a,
        _ => AudioCodec::Opus,
    }
}

/// Output container (PRD 3.3): H.264/AV1 + AAC → mp4; VP9/AV1 + Opus → webm/mkv.
pub fn container(video_codec: &str, audio_codec: &str) -> &'static str {
    match (video_codec, audio_codec) {
        ("h264" | "av1", "aac") => "mp4",
        ("vp9" | "av1", "opus") => "webm",
        _ => "mkv",
    }
}

/// What to fetch for a download.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DownloadPlan {
    pub video: Option<StreamChoice>,
    pub audio: StreamChoice,
    /// Final file extension.
    pub ext: String,
    /// Audio must be re-encoded to MP3 after download.
    pub transcode_mp3: bool,
}

pub fn plan_download(
    player: &VideoPlayer,
    caps: &HwCaps,
    max_height: Option<u32>,
    audio_only: Option<AudioOut>,
) -> Option<DownloadPlan> {
    if let Some(out) = audio_only {
        let prefer = match out {
            AudioOut::Opus => AudioCodec::Opus,
            AudioOut::M4a | AudioOut::Mp3 => AudioCodec::Mp4a,
        };
        let a = pick_audio(&player.audio_streams, prefer)?;
        let native_ext = match a.codec {
            AudioCodec::Opus => "opus",
            _ => "m4a",
        };
        return Some(DownloadPlan {
            video: None,
            audio: audio_choice(a),
            ext: if out == AudioOut::Mp3 { "mp3" } else { native_ext }.into(),
            transcode_mp3: out == AudioOut::Mp3,
        });
    }
    let q = qualities(&player.video_only_streams, caps);
    let v = pick_max(&q, max_height.unwrap_or(u32::MAX))?;
    let a = pick_audio(&player.audio_streams, audio_for_video(&v.stream.codec))?;
    let ac = audio_choice(a);
    Some(DownloadPlan {
        ext: container(&v.stream.codec, &ac.codec).into(),
        video: Some(v.stream.clone()),
        audio: ac,
        transcode_mp3: false,
    })
}

/// Streams for one video, independent of where they came from (yt-dlp or rustypipe).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ResolvedStreams {
    pub title: String,
    pub channel: String,
    pub duration_s: u32,
    /// Video-only streams (any codec).
    pub video: Vec<StreamChoice>,
    pub audio_aac: Option<StreamChoice>,
    pub audio_opus: Option<StreamChoice>,
    /// HLS manifest for livestreams.
    pub hls: Option<String>,
    /// Unix seconds after which the URLs stop working.
    pub valid_until: i64,
    /// "yt-dlp" or a rustypipe client name.
    pub source: String,
    /// Every sound track of a video with dubbed audio (AAC), the original first. Empty when
    /// the video has one track.
    pub audio_tracks: Vec<AudioTrack>,
    /// The loudness-levelled ("stable volume") AAC sound of the main track, when offered.
    pub audio_drc: Option<StreamChoice>,
}

/// One language of a video with dubbed sound.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AudioTrack {
    /// As YouTube names it, e.g. "English (United States) original".
    pub name: String,
    pub original: bool,
    pub stream: StreamChoice,
}

impl ResolvedStreams {
    pub fn from_player(p: &VideoPlayer) -> Self {
        let audio = |c: AudioCodec| {
            pick_audio(&p.audio_streams, c)
                .filter(|a| a.codec == c)
                .map(audio_choice)
        };
        Self {
            title: p.details.name.clone().unwrap_or_default(),
            channel: p.details.channel_name.clone().unwrap_or_default(),
            duration_s: p.details.duration,
            video: p
                .video_only_streams
                .iter()
                .filter(|s| s.drm_track_type.is_none() && !s.hdr)
                .map(video_choice)
                .collect(),
            audio_aac: audio(AudioCodec::Mp4a),
            audio_opus: audio(AudioCodec::Opus),
            hls: p.hls_manifest_url.clone(),
            valid_until: p.valid_until.unix_timestamp(),
            source: format!("{:?}", p.client_type),
            audio_tracks: Vec::new(),
            audio_drc: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.video.is_empty() && self.audio_aac.is_none() && self.audio_opus.is_none() && self.hls.is_none()
    }

    /// Seconds of validity left.
    pub fn ttl(&self) -> i64 {
        self.valid_until - time::OffsetDateTime::now_utc().unix_timestamp()
    }
}

/// Narrow the hardware capabilities to the user's codec preference.
pub fn apply_codec_pref(caps: HwCaps, pref: CodecPref) -> HwCaps {
    match pref {
        CodecPref::Auto | CodecPref::Av1 => caps,
        CodecPref::H264 => HwCaps { h264: true, vp9: false, av1: false },
        CodecPref::Vp9 => HwCaps { h264: true, vp9: caps.vp9, av1: false },
    }
}

/// Plan a download from resolved streams. Video picks the tallest stream within `max_height`,
/// preferring H.264 at equal height for the widest player compatibility.
pub fn plan_from_resolved(
    r: &ResolvedStreams,
    max_height: Option<u32>,
    audio_only: Option<AudioOut>,
) -> Option<DownloadPlan> {
    if let Some(out) = audio_only {
        let (first, second) = match out {
            AudioOut::Opus => (&r.audio_opus, &r.audio_aac),
            AudioOut::M4a | AudioOut::Mp3 => (&r.audio_aac, &r.audio_opus),
        };
        let a = first.as_ref().or(second.as_ref())?.clone();
        let native = if a.codec == "opus" { "opus" } else { "m4a" };
        return Some(DownloadPlan {
            ext: if out == AudioOut::Mp3 { "mp3" } else { native }.into(),
            transcode_mp3: out == AudioOut::Mp3,
            video: None,
            audio: a,
        });
    }
    let max = max_height.unwrap_or(u32::MAX);
    let compat = |c: &str| match c {
        "h264" => 3,
        "vp9" => 2,
        "av1" => 1,
        _ => 0,
    };
    let v = r
        .video
        .iter()
        .filter(|s| s.height <= max && compat(&s.codec) > 0)
        .max_by_key(|s| (s.height, compat(&s.codec), s.fps, s.bitrate))
        .or_else(|| r.video.iter().filter(|s| compat(&s.codec) > 0).min_by_key(|s| s.height))?
        .clone();
    let a = if v.codec == "h264" {
        r.audio_aac.as_ref().or(r.audio_opus.as_ref())
    } else {
        r.audio_opus.as_ref().or(r.audio_aac.as_ref())
    }?
    .clone();
    Some(DownloadPlan {
        ext: container(&v.codec, &a.codec).into(),
        video: Some(v),
        audio: a,
        transcode_mp3: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opt(h: u32, br: u32) -> QualityOption {
        QualityOption {
            label: format!("{h}p"),
            height: h,
            fps: 30,
            stream: StreamChoice {
                url: String::new(),
                itag: 0,
                mime: String::new(),
                codec: "h264".into(),
                height: h,
                fps: 30,
                bitrate: br,
                size: None,
                headers: Vec::new(),
                init_range: None,
                index_range: None,
            },
        }
    }

    #[test]
    fn auto_respects_height_and_throughput() {
        let o = vec![opt(1080, 4_000_000), opt(720, 2_000_000), opt(360, 500_000)];
        // The smallest rung covering the player, never below 720p when it is sustainable.
        assert_eq!(pick_auto(&o, 800, None).unwrap().height, 1080);
        assert_eq!(pick_auto(&o, 700, None).unwrap().height, 720);
        assert_eq!(pick_auto(&o, 300, None).unwrap().height, 720);
        assert_eq!(pick_auto(&o, 4000, None).unwrap().height, 1080);
        assert_eq!(pick_auto(&o, 2000, Some(3_000_000)).unwrap().height, 720);
        assert_eq!(pick_auto(&o, 2000, Some(100)).unwrap().height, 360);
    }

    #[test]
    fn download_plan_prefers_h264_at_same_height() {
        let mk = |codec: &str, h: u32| StreamChoice {
            url: String::new(), itag: h, mime: String::new(), codec: codec.into(),
            height: h, fps: 30, bitrate: 1, size: None, headers: Vec::new(), init_range: None, index_range: None,
        };
        let r = ResolvedStreams {
            title: String::new(), channel: String::new(), duration_s: 0,
            video: vec![mk("av1", 1080), mk("h264", 1080), mk("vp9", 2160)],
            audio_aac: Some(mk("aac", 0)), audio_opus: Some(mk("opus", 0)),
            hls: None, valid_until: 0, source: String::new(), audio_tracks: Vec::new(), audio_drc: None,
        };
        let p = plan_from_resolved(&r, Some(1080), None).unwrap();
        assert_eq!((p.video.unwrap().codec.as_str(), p.audio.codec.as_str(), p.ext.as_str()), ("h264", "aac", "mp4"));
        let p = plan_from_resolved(&r, None, None).unwrap();
        assert_eq!((p.video.unwrap().height, p.ext.as_str()), (2160, "webm"));
        let p = plan_from_resolved(&r, None, Some(AudioOut::Opus)).unwrap();
        assert_eq!(p.ext, "opus");
    }

    #[test]
    fn containers() {
        assert_eq!(container("h264", "aac"), "mp4");
        assert_eq!(container("av1", "aac"), "mp4");
        assert_eq!(container("vp9", "opus"), "webm");
        assert_eq!(container("vp9", "aac"), "mkv");
    }
}
