//! Remote config: a JSON file in the GitHub repo that can switch client order,
//! disable device login or lower concurrency without a new release (PRD 5.1).

use std::path::Path;
use std::time::Duration;

use rustypipe::client::ClientType;
use serde::{Deserialize, Serialize};

/// GitHub `owner/repo` for remote config and update checks.
pub const REPO: &str = "Talhary/youtube-client";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RemoteConfig {
    /// Player client fallback order, e.g. `["desktop", "tv", "mobile"]`.
    pub client_order: Vec<String>,
    /// Kill switch for device login; `false` hides it even if the user enabled it.
    pub device_login: bool,
    /// Upper bound applied on top of the user's connection setting.
    pub max_connections: Option<u8>,
    /// Optional banner text shown to every user.
    pub message: Option<String>,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            client_order: vec!["ios".into(), "desktop".into(), "tv".into(), "mobile".into()],
            device_login: true,
            max_connections: None,
            message: None,
        }
    }
}

impl RemoteConfig {
    pub fn client_types(&self) -> Vec<ClientType> {
        let v: Vec<ClientType> = self
            .client_order
            .iter()
            .filter_map(|s| parse_client(s))
            .collect();
        if v.is_empty() {
            vec![ClientType::Ios, ClientType::Desktop, ClientType::Tv, ClientType::Mobile]
        } else {
            v
        }
    }

    /// Fetch the live config, falling back to the last cached copy, then defaults.
    pub async fn fetch(http: &reqwest::Client, cache_dir: &Path) -> Self {
        let cache = cache_dir.join("remote-config.json");
        let url = format!("https://raw.githubusercontent.com/{REPO}/main/remote-config.json");
        let live = async {
            let r = http
                .get(&url)
                .timeout(Duration::from_secs(5))
                .send()
                .await
                .ok()?
                .error_for_status()
                .ok()?;
            let bytes = r.bytes().await.ok()?;
            let cfg: RemoteConfig = serde_json::from_slice(&bytes).ok()?;
            let _ = crate::config::write_atomic(&cache, &bytes);
            Some(cfg)
        }
        .await;
        if let Some(cfg) = live {
            return cfg;
        }
        std::fs::read(&cache)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
}

pub fn parse_client(s: &str) -> Option<ClientType> {
    Some(match s.to_ascii_lowercase().as_str() {
        "desktop" | "web" => ClientType::Desktop,
        "tv" => ClientType::Tv,
        "mobile" | "mweb" => ClientType::Mobile,
        "android" => ClientType::Android,
        "ios" => ClientType::Ios,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_order_starts_with_ios() {
        assert_eq!(
            RemoteConfig::default().client_types(),
            vec![ClientType::Ios, ClientType::Desktop, ClientType::Tv, ClientType::Mobile]
        );
    }

    #[test]
    fn unknown_clients_fall_back() {
        let c = RemoteConfig {
            client_order: vec!["nope".into()],
            ..Default::default()
        };
        assert_eq!(c.client_types().len(), 4);
    }
}
