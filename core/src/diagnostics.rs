//! Logging and the one-tap diagnostics export (PRD 5.1). Recent log lines are kept in a ring
//! buffer; the export redacts tokens, cookies and signed URL parameters.

use std::collections::VecDeque;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use parking_lot::Mutex;
use regex::Regex;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

const MAX_LINES: usize = 3000;

#[derive(Clone, Default)]
pub struct RingLog(Arc<Mutex<VecDeque<String>>>);

impl RingLog {
    pub fn lines(&self) -> Vec<String> {
        self.0.lock().iter().cloned().collect()
    }
}

pub struct RingWriter(RingLog, Vec<u8>);

impl Write for RingWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.1.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for RingWriter {
    fn drop(&mut self) {
        let s = String::from_utf8_lossy(&self.1);
        let mut q = (self.0).0.lock();
        for line in s.lines() {
            if q.len() >= MAX_LINES {
                q.pop_front();
            }
            q.push_back(line.to_string());
        }
    }
}

impl<'a> MakeWriter<'a> for RingLog {
    type Writer = RingWriter;
    fn make_writer(&'a self) -> Self::Writer {
        RingWriter(self.clone(), Vec::new())
    }
}

static LOG: OnceLock<RingLog> = OnceLock::new();

/// Install the global subscriber once. Safe to call repeatedly.
pub fn init() -> RingLog {
    LOG.get_or_init(|| {
        let ring = RingLog::default();
        let filter = || {
            EnvFilter::try_from_env("STREAMPIPE_LOG")
                .unwrap_or_else(|_| EnvFilter::new("info,rustypipe=info"))
        };
        let ring_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(ring.clone())
            .with_filter(filter());
        let stderr_layer = tracing_subscriber::fmt::layer()
            .with_writer(std::io::stderr)
            .with_filter(filter());
        let _ = tracing_subscriber::registry()
            .with(ring_layer)
            .with(stderr_layer)
            .try_init();
        ring
    })
    .clone()
}

/// Mask secrets: OAuth tokens, cookies, auth headers, PO tokens and URL signatures.
pub fn redact(s: &str) -> String {
    static RES: OnceLock<Vec<Regex>> = OnceLock::new();
    let res = RES.get_or_init(|| {
        [
            r#"(?i)("?(?:access_token|refresh_token|device_code|po_token|visitor_data|client_secret)"?\s*[:=]\s*"?)[^"&,\s}]+"#,
            r"(?i)((?:authorization|cookie|set-cookie)\s*[:=]\s*)[^\n]+",
            r"(?i)\b((?:SAPISID|__Secure-[0-9A-Za-z]+PSID[A-Za-z]*|SID|HSID|SSID|APISID|LOGIN_INFO)=)[^;\s]+",
            r"([?&](?:sig|lsig|pot|n|signature|ip|ei)=)[^&\s]+",
        ]
        .iter()
        .map(|r| Regex::new(r).unwrap())
        .collect()
    });
    res.iter().fold(s.to_string(), |acc, re| {
        re.replace_all(&acc, "${1}[redacted]").into_owned()
    })
}

/// Write recent logs plus a header to `path`.
pub fn export(path: &Path, header: &str) -> std::io::Result<()> {
    let ring = init();
    let mut out = String::new();
    out.push_str(header);
    out.push_str("\n---\n");
    for line in ring.lines() {
        out.push_str(&redact(&line));
        out.push('\n');
    }
    crate::config::write_atomic(path, out.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_secrets() {
        let s = r#"{"access_token":"ya29.abc","refresh_token": "1//xyz"} Cookie: SID=foo; HSID=bar
GET https://rr1.googlevideo.com/videoplayback?expire=1&sig=AAA&pot=BBB&itag=18"#;
        let r = redact(s);
        for leak in ["ya29.abc", "1//xyz", "SID=foo", "AAA", "BBB"] {
            assert!(!r.contains(leak), "{leak} leaked in {r}");
        }
        assert!(r.contains("itag=18"));
    }
}
