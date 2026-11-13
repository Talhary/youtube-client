//! Update check for GitHub builds (PRD 5.1). Store builds compile this out.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::remote::REPO;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct UpdateInfo {
    pub version: String,
    pub url: String,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    html_url: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
}

pub async fn check(http: &reqwest::Client, current: &str) -> Option<UpdateInfo> {
    let r: Release = http
        .get(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(8))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json()
        .await
        .ok()?;
    if r.draft || r.prerelease {
        return None;
    }
    newer(&r.tag_name, current).then(|| UpdateInfo {
        version: r.tag_name.trim_start_matches('v').to_string(),
        url: r.html_url,
    })
}

fn newer(tag: &str, current: &str) -> bool {
    match (
        semver::Version::parse(tag.trim_start_matches('v')),
        semver::Version::parse(current),
    ) {
        (Ok(a), Ok(b)) => a > b,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn compares() {
        assert!(super::newer("v0.2.0", "0.1.9"));
        assert!(!super::newer("v0.1.0", "0.1.0"));
        assert!(!super::newer("garbage", "0.1.0"));
    }
}
