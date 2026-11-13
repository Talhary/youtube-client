//! Local stream server for playback.
//!
//! Players (mpv, Media3) request `http://127.0.0.1:<port>/s/<token>` with ordinary HTTP Range
//! headers. The core answers by fetching upstream exactly the way the download engine does:
//! googlevideo's `range` URL parameter in pieces of at most [`CHUNK`] bytes. Tokens are random
//! 128-bit values, so other local programs cannot guess stream paths.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use futures_util::StreamExt;
use parking_lot::Mutex;
use rand::RngCore;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

/// Upstream request size. Same as the downloader.
pub const CHUNK: u64 = 10 * 1024 * 1024;
/// How many videos' worth of streams stay registered.
const KEEP_GROUPS: usize = 16;

#[derive(Clone)]
struct Source {
    url: String,
    size: u64,
    mime: String,
    headers: Vec<(String, String)>,
}

#[derive(Default)]
struct Registry {
    sources: HashMap<String, Source>,
    /// Group key (video id) → tokens, oldest group first.
    groups: Vec<(String, Vec<String>)>,
}

pub struct StreamProxy {
    port: u16,
    reg: Arc<Mutex<Registry>>,
}

/// One stream to expose.
pub struct ProxyStream {
    pub url: String,
    pub size: u64,
    pub mime: String,
    /// Headers to send upstream (as supplied by the extractor).
    pub headers: Vec<(String, String)>,
}

impl StreamProxy {
    /// Bind to a random loopback port and start serving on the current tokio runtime.
    pub async fn start(http: reqwest::Client) -> std::io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        let reg = Arc::new(Mutex::new(Registry::default()));
        let r2 = reg.clone();
        tokio::spawn(async move {
            loop {
                let Ok((sock, _)) = listener.accept().await else {
                    continue;
                };
                let _ = sock.set_nodelay(true);
                let (reg, http) = (r2.clone(), http.clone());
                tokio::spawn(async move {
                    if let Err(e) = serve(sock, reg, http).await {
                        tracing::debug!("proxy connection ended: {e}");
                    }
                });
            }
        });
        tracing::info!("stream proxy listening on 127.0.0.1:{port}");
        Ok(Self { port, reg })
    }

    /// Register streams for one video and return their local URLs in the same order.
    /// Registering a video again replaces its previous URLs.
    pub fn register(&self, group: &str, streams: Vec<ProxyStream>) -> Vec<String> {
        let mut reg = self.reg.lock();
        // The same streams again (fresh upstream URLs for a video that is playing): the
        // local URLs stay as they are, so a player in the middle of the video carries on.
        // A stream is the same one if its format number and size match; the answers for
        // one video can list the formats in another order, or with more or fewer of them.
        let key = |url: &str, size: u64| {
            let itag = url.split(['?', '&']).find_map(|p| p.strip_prefix("itag=")).unwrap_or("").to_string();
            (itag, size)
        };
        let mut kept: HashMap<(String, u64), String> = HashMap::new();
        if let Some(i) = reg.groups.iter().position(|(g, _)| g == group) {
            let (_, old) = reg.groups.remove(i);
            for t in old {
                if let Some(src) = reg.sources.remove(&t) {
                    kept.insert(key(&src.url, src.size), t);
                }
            }
        }
        while reg.groups.len() >= KEEP_GROUPS {
            let (_, old) = reg.groups.remove(0);
            for t in old {
                reg.sources.remove(&t);
            }
        }
        let mut tokens = Vec::new();
        let mut urls = Vec::new();
        for s in streams {
            let token = kept.remove(&key(&s.url, s.size)).unwrap_or_else(|| {
                let mut b = [0u8; 16];
                rand::rng().fill_bytes(&mut b);
                hex::encode(b)
            });
            urls.push(format!("http://127.0.0.1:{}/s/{token}", self.port));
            reg.sources.insert(
                token.clone(),
                Source {
                    url: s.url,
                    size: s.size,
                    mime: s.mime.split(';').next().unwrap_or("").trim().to_string(),
                    headers: s.headers,
                },
            );
            tokens.push(token);
        }
        reg.groups.push((group.to_string(), tokens));
        urls
    }

    pub fn port(&self) -> u16 {
        self.port
    }
}

/// Parse a `Range` header against a resource of `size` bytes. `Ok(None)` = whole resource.
fn parse_range(h: Option<&str>, size: u64) -> Result<Option<(u64, u64)>, ()> {
    let Some(h) = h else { return Ok(None) };
    let spec = h.trim().strip_prefix("bytes=").ok_or(())?;
    // Multiple ranges are not supported; serve the first.
    let spec = spec.split(',').next().unwrap_or("").trim();
    let (a, b) = spec.split_once('-').ok_or(())?;
    let (start, end) = if a.is_empty() {
        let n: u64 = b.parse().map_err(|_| ())?;
        if n == 0 {
            return Err(());
        }
        (size.saturating_sub(n), size - 1)
    } else {
        let start: u64 = a.parse().map_err(|_| ())?;
        let end = if b.is_empty() {
            size - 1
        } else {
            b.parse::<u64>().map_err(|_| ())?.min(size - 1)
        };
        (start, end)
    };
    if size == 0 || start > end || start >= size {
        return Err(());
    }
    Ok(Some((start, end)))
}

fn upstream(url: &str, start: u64, end: u64) -> String {
    let sep = if url.contains('?') { '&' } else { '?' };
    format!("{url}{sep}range={start}-{end}")
}

/// How much to ask upstream for in one request, by position in the stream.
///
/// The start of a stream is fetched in small pieces: a just-resolved address may serve only
/// the first minute or so of the video and refuses any request that reaches past it, so a
/// large first request would be refused whole. Small pieces let playback begin at once; by
/// the time the larger ones are due, the full address has replaced it.
fn piece(pos: u64) -> u64 {
    if pos < 1024 * 1024 {
        256 * 1024
    } else if pos < 8 * 1024 * 1024 {
        1024 * 1024
    } else {
        CHUNK
    }
}

async fn fetch(
    http: &reqwest::Client,
    src: &Source,
    start: u64,
    end: u64,
) -> Result<reqwest::Response, u16> {
    let mut last = 502;
    for attempt in 0..3u32 {
        let mut req = http
            .get(upstream(&src.url, start, end))
            .timeout(Duration::from_secs(60));
        for (k, v) in &src.headers {
            req = req.header(k.as_str(), v.as_str());
        }
        match req.send().await
        {
            Ok(r) if r.status().is_success() => return Ok(r),
            Ok(r) if r.status().as_u16() == 403 || r.status().as_u16() == 404 => {
                tracing::warn!("proxy upstream: HTTP {} for bytes {start}-{end} of {}", r.status().as_u16(), src.url.split('?').next().unwrap_or(""));
                return Err(r.status().as_u16());
            }
            Ok(r) => last = r.status().as_u16(),
            Err(e) => tracing::warn!("proxy upstream: {e}"),
        }
        tokio::time::sleep(Duration::from_millis(400 << attempt)).await;
    }
    Err(last)
}

/// Fetch a piece; if googlevideo refuses it, the address may be a preview that the full
/// answer replaces any moment (the core resolves the full streams right after the quick ones,
/// keeping the same local address for the same file). Wait for that and continue on it, so
/// the player never sees the refusal. Answers the response and the source it came from.
async fn fetch_waiting(
    http: &reqwest::Client,
    reg: &Arc<Mutex<Registry>>,
    token: &str,
    src: &Source,
    start: u64,
    end: u64,
) -> Result<(reqwest::Response, Source), u16> {
    match fetch(http, src, start, end).await {
        Err(403) => {
            let until = std::time::Instant::now() + Duration::from_secs(15);
            while std::time::Instant::now() < until {
                tokio::time::sleep(Duration::from_millis(300)).await;
                let now = reg.lock().sources.get(token).cloned();
                if let Some(next) = now.filter(|n| n.url != src.url) {
                    tracing::info!("proxy: a refused stream was replaced; continuing from byte {start} on the new address");
                    return fetch(http, &next, start, end).await.map(|r| (r, next));
                }
            }
            Err(403)
        }
        other => other.map(|r| (r, src.clone())),
    }
}

async fn serve(sock: TcpStream, reg: Arc<Mutex<Registry>>, http: reqwest::Client) -> std::io::Result<()> {
    let (r, mut w) = sock.into_split();
    let mut reader = BufReader::new(r);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).await? == 0 {
            return Ok(());
        }
        let mut parts = line.split_whitespace();
        let method = parts.next().unwrap_or("").to_string();
        let path = parts.next().unwrap_or("").to_string();
        let mut range = None;
        let mut close = false;
        loop {
            let mut h = String::new();
            if reader.read_line(&mut h).await? == 0 {
                return Ok(());
            }
            let h = h.trim_end();
            if h.is_empty() {
                break;
            }
            if let Some((k, v)) = h.split_once(':') {
                match k.trim().to_ascii_lowercase().as_str() {
                    "range" => range = Some(v.trim().to_string()),
                    "connection" => close = v.trim().eq_ignore_ascii_case("close"),
                    _ => {}
                }
            }
        }

        let token = path.strip_prefix("/s/").unwrap_or("").to_string();
        let src = reg.lock().sources.get(&token).cloned();
        let Some(src) = src else {
            w.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").await?;
            continue;
        };
        if method != "GET" && method != "HEAD" {
            w.write_all(b"HTTP/1.1 405 Method Not Allowed\r\nContent-Length: 0\r\n\r\n").await?;
            continue;
        }
        let (start, end, partial) = match parse_range(range.as_deref(), src.size) {
            Ok(Some((s, e))) => (s, e, true),
            Ok(None) => (0, src.size - 1, false),
            Err(()) => {
                let msg = format!(
                    "HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{}\r\nContent-Length: 0\r\n\r\n",
                    src.size
                );
                w.write_all(msg.as_bytes()).await?;
                continue;
            }
        };

        // Open the first upstream piece before answering, so failures map to real status codes.
        let first = if method == "GET" {
            match fetch_waiting(&http, &reg, &token, &src, start, (start + piece(start) - 1).min(end)).await {
                Ok((r, _)) => Some(r),
                Err(code) => {
                    let reason = if code == 403 { "Forbidden" } else { "Bad Gateway" };
                    let code = if code == 403 { 403 } else { 502 };
                    tracing::warn!("proxy: upstream refused stream ({code})");
                    w.write_all(format!("HTTP/1.1 {code} {reason}\r\nContent-Length: 0\r\n\r\n").as_bytes())
                        .await?;
                    continue;
                }
            }
        } else {
            None
        };

        let len = end - start + 1;
        let mut head = if partial {
            format!(
                "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{end}/{}\r\n",
                src.size
            )
        } else {
            "HTTP/1.1 200 OK\r\n".to_string()
        };
        head.push_str(&format!(
            "Content-Length: {len}\r\nAccept-Ranges: bytes\r\nContent-Type: {}\r\nCache-Control: no-store\r\n\r\n",
            if src.mime.is_empty() { "application/octet-stream" } else { &src.mime }
        ));
        w.write_all(head.as_bytes()).await?;
        let Some(mut resp) = first else { continue };

        let mut pos = start;
        loop {
            let piece_end = (pos + piece(pos) - 1).min(end);
            let mut got = 0u64;
            let mut body = resp.bytes_stream();
            while let Some(b) = body.next().await {
                let b = b.map_err(std::io::Error::other)?;
                got += b.len() as u64;
                // A write error means the player went away (seek or stop): just stop.
                w.write_all(&b).await?;
            }
            if got != piece_end - pos + 1 {
                return Err(std::io::Error::other("short upstream read"));
            }
            pos = piece_end + 1;
            if pos > end {
                break;
            }
            // The upstream address may have been replaced while this answer was being sent
            // (a preview swapped for the full stream): each piece uses the current one.
            let now = reg.lock().sources.get(&token).cloned().unwrap_or_else(|| src.clone());
            resp = fetch_waiting(&http, &reg, &token, &now, pos, (pos + piece(pos) - 1).min(end))
                .await
                .map(|(r, _)| r)
                .map_err(|c| std::io::Error::other(format!("upstream {c}")))?;
        }
        w.flush().await?;
        if close {
            return Ok(());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges() {
        assert_eq!(parse_range(None, 100), Ok(None));
        assert_eq!(parse_range(Some("bytes=0-"), 100), Ok(Some((0, 99))));
        assert_eq!(parse_range(Some("bytes=10-19"), 100), Ok(Some((10, 19))));
        assert_eq!(parse_range(Some("bytes=90-500"), 100), Ok(Some((90, 99))));
        assert_eq!(parse_range(Some("bytes=-10"), 100), Ok(Some((90, 99))));
        assert!(parse_range(Some("bytes=100-"), 100).is_err());
        assert!(parse_range(Some("items=1-2"), 100).is_err());
    }

    /// A fake upstream that honours the `range=a-b` query parameter, behind the proxy.
    #[tokio::test]
    async fn serves_ranges_across_pieces() {
        let data: Arc<Vec<u8>> = Arc::new((0..(CHUNK as usize * 2 + 1234)).map(|i| (i % 251) as u8).collect());
        let up = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let up_port = up.local_addr().unwrap().port();
        let d2 = data.clone();
        tokio::spawn(async move {
            loop {
                let (s, _) = up.accept().await.unwrap();
                let d = d2.clone();
                tokio::spawn(async move {
                    let (r, mut w) = s.into_split();
                    let mut r = BufReader::new(r);
                    let mut line = String::new();
                    r.read_line(&mut line).await.unwrap();
                    loop {
                        let mut h = String::new();
                        r.read_line(&mut h).await.unwrap();
                        if h.trim().is_empty() {
                            break;
                        }
                    }
                    let q = line.split("range=").nth(1).unwrap().split_whitespace().next().unwrap();
                    let (a, b) = q.split_once('-').unwrap();
                    let (a, b): (usize, usize) = (a.parse().unwrap(), b.parse().unwrap());
                    let body = &d[a..=b];
                    w.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).as_bytes()).await.unwrap();
                    w.write_all(body).await.unwrap();
                });
            }
        });

        let http = reqwest::Client::new();
        let proxy = StreamProxy::start(http.clone()).await.unwrap();
        let urls = proxy.register(
            "vid",
            vec![ProxyStream {
                url: format!("http://127.0.0.1:{up_port}/videoplayback?x=1"),
                size: data.len() as u64,
                mime: "video/mp4; codecs=\"avc1\"".into(),
                headers: vec![("X-Test".into(), "1".into())],
            }],
        );
        // Whole file (spans three upstream pieces).
        let all = http.get(&urls[0]).send().await.unwrap();
        assert_eq!(all.status(), 200);
        assert_eq!(all.headers()["content-type"], "video/mp4");
        assert_eq!(&all.bytes().await.unwrap()[..], &data[..]);
        // A range crossing a piece boundary.
        let s = CHUNK as usize - 10;
        let r = http.get(&urls[0]).header("Range", format!("bytes={s}-{}", s + 19)).send().await.unwrap();
        assert_eq!(r.status(), 206);
        assert_eq!(&r.bytes().await.unwrap()[..], &data[s..s + 20]);
        // Unknown token.
        let bad = http.get(format!("http://127.0.0.1:{}/s/nope", proxy.port())).send().await.unwrap();
        assert_eq!(bad.status(), 404);
        // Re-registering the group invalidates old URLs.
        proxy.register("vid", vec![]);
        assert_eq!(http.get(&urls[0]).send().await.unwrap().status(), 404);
    }
}
