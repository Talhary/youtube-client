//! "Play on TV" over DIAL: find TVs with the YouTube app on the local network and ask one
//! to play a video (what the phone app's Tv.java does, for the desktop).

use std::collections::BTreeSet;
use std::net::UdpSocket;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const ASK: &str = "M-SEARCH * HTTP/1.1\r\nHOST: 239.255.255.250:1900\r\nMAN: \"ssdp:discover\"\r\nMX: 2\r\nST: urn:dial-multiscreen-org:service:dial:1\r\n\r\n";

/// The device descriptions that answered an SSDP search (about three seconds).
fn locations() -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else {
        return out;
    };
    let _ = socket.set_read_timeout(Some(Duration::from_millis(700)));
    let until = Instant::now() + Duration::from_millis(3200);
    let mut sent = 0;
    let mut buf = [0u8; 2048];
    while Instant::now() < until {
        if sent < 3 {
            let _ = socket.send_to(ASK.as_bytes(), "239.255.255.250:1900");
            sent += 1;
        }
        if let Ok((n, _)) = socket.recv_from(&mut buf) {
            let answer = String::from_utf8_lossy(&buf[..n]);
            for line in answer.split("\r\n") {
                if let Some((k, v)) = line.split_once(':') {
                    if k.trim().eq_ignore_ascii_case("location") {
                        out.insert(v.trim().to_string());
                    }
                }
            }
        }
    }
    out
}

/// TVs on the network: `{"devices": [{"name", "app_url"}]}`.
pub async fn discover(http: &reqwest::Client) -> Result<Value, String> {
    let found = tokio::task::spawn_blocking(locations).await.map_err(|e| e.to_string())?;
    let mut devices = Vec::new();
    for location in found {
        let Ok(resp) = http.get(&location).timeout(Duration::from_millis(2500)).send().await else {
            continue;
        };
        let Some(app) = resp.headers().get("Application-URL").and_then(|v| v.to_str().ok()).map(str::to_string) else {
            continue;
        };
        let xml = resp.text().await.unwrap_or_default();
        let name = xml
            .split("<friendlyName>")
            .nth(1)
            .and_then(|r| r.split('<').next())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| reqwest::Url::parse(&location).ok().and_then(|u| u.host_str().map(str::to_string)))
            .unwrap_or_else(|| "TV".into());
        let app_url = if app.ends_with('/') { app } else { format!("{app}/") };
        devices.push(json!({"name": name, "app_url": app_url}));
    }
    Ok(json!({"devices": devices}))
}

/// Ask the TV's YouTube app to play `video` from `at` seconds.
pub async fn launch(http: &reqwest::Client, app_url: &str, video: &str, at: u32) -> Result<Value, String> {
    let body = if at > 0 { format!("v={video}&t={at}") } else { format!("v={video}") };
    let resp = http
        .post(format!("{app_url}YouTube"))
        .header(reqwest::header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(body)
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .map_err(|e| format!("could not reach the TV: {e}"))?;
    let code = resp.status().as_u16();
    if code == 200 || code == 201 {
        Ok(json!({"ok": true}))
    } else {
        Err(format!("the TV answered {code}"))
    }
}
