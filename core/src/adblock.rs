//! Ad-blocking rules for the YouTube web player.
//!
//! The rules are uBlock Origin's own: its public filter lists are downloaded from uBlock's
//! servers, the script rules (`##+js(...)`) that apply to YouTube are picked out, and the
//! result is cached on disk as JSON. The player page runs them with our own implementations
//! of the rule types (see the shell's `player-inject.js`). A rule type that script does not
//! know is skipped there and reported.
//!
//! The shell reads [`rules_file`] directly when it builds the page script, so playback never
//! waits for the network: until the first download finishes it gets [`BUILTIN`].

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// uBlock Origin's filter lists (its CDN mirrors, in the order uBlock itself tries them).
const LISTS: &[&[&str]] = &[
    &[
        "https://ublockorigin.github.io/uAssetsCDN/filters/filters.min.txt",
        "https://ublockorigin.pages.dev/filters/filters.min.txt",
    ],
    &[
        "https://ublockorigin.github.io/uAssetsCDN/filters/quick-fixes.min.txt",
        "https://ublockorigin.pages.dev/filters/quick-fixes.min.txt",
    ],
];

/// How often the lists are downloaded again.
const REFRESH: Duration = Duration::from_secs(6 * 3600);

/// The hosts the player page can be on.
const HOSTS: &[&str] = &["www.youtube.com", "m.youtube.com"];

/// One script rule: run scriptlet `name` with `args` on `hosts`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Rule {
    pub hosts: Vec<String>,
    pub name: String,
    pub args: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuleSet {
    /// Unix seconds of the download; 0 for the built-in set.
    pub fetched_at: i64,
    pub rules: Vec<Rule>,
}

/// Used until the first download succeeds (uBlock's YouTube rules as of 2026-11).
pub const BUILTIN: &str = r#"www.youtube.com,m.youtube.com##+js(set, ytInitialPlayerResponse.playerAds, undefined)
www.youtube.com,m.youtube.com##+js(set, ytInitialPlayerResponse.adPlacements, undefined)
www.youtube.com,m.youtube.com##+js(set, ytInitialPlayerResponse.adSlots, undefined)
www.youtube.com,m.youtube.com##+js(set, playerResponse.adPlacements, undefined)
youtube.com##+js(json-prune, playerResponse.adPlacements playerResponse.playerAds playerResponse.adSlots adPlacements playerAds adSlots legacyImportant)
www.youtube.com,m.youtube.com##+js(json-prune-fetch-response, adPlacements adSlots playerResponse.adPlacements playerResponse.adSlots [].playerResponse.adPlacements [].playerResponse.adSlots, , propsToMatch, /\/player(\?|$)/)
www.youtube.com,m.youtube.com##+js(json-prune-xhr-response, adPlacements adSlots playerResponse.adPlacements playerResponse.adSlots [].playerResponse.adPlacements [].playerResponse.adSlots, , propsToMatch, /\/player(?:\?.+)?$/)
www.youtube.com,m.youtube.com##+js(trusted-rpfr, '"adPlacements"', '"no_ads"', player?)
www.youtube.com,m.youtube.com##+js(trusted-rpfr, '"adSlots"', '"no_ads"', player?)
www.youtube.com,m.youtube.com##+js(trusted-rpfr, '"adSlots"', '"no_ads"', /get_watch?)
www.youtube.com,m.youtube.com##+js(nano-stb, [native code], 17000, 0.001)
www.youtube.com,m.youtube.com##+js(trusted-prevent-dom-bypass, Node.prototype.appendChild, fetch)
www.youtube.com,m.youtube.com##+js(trusted-prevent-dom-bypass, Node.prototype.appendChild, Request)
"#;

pub fn rules_file(cache_dir: &Path) -> PathBuf {
    cache_dir.join("adblock-rules.json")
}

/// Does a filter's host entry (`youtube.com`, `www.youtube.com`, …) cover `host`?
fn covers(entry: &str, host: &str) -> bool {
    host == entry || host.ends_with(&format!(".{entry}"))
}

/// Split scriptlet arguments the way uBlock does: commas separate, `\,` is a literal comma,
/// and an argument wrapped in quotes or backticks keeps its commas.
fn split_args(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&',') => {
                cur.push(',');
                chars.next();
            }
            '\\' if quote.is_some() && chars.peek().is_some_and(|n| Some(*n) == quote) => {
                cur.push(chars.next().unwrap());
            }
            '\'' | '"' | '`' if quote.is_none() && cur.trim().is_empty() => {
                quote = Some(c);
                cur.clear();
                cur.push(c);
            }
            c if Some(c) == quote => {
                // Closing quote only if what follows is a separator or the end.
                let mut look = chars.clone();
                let mut next = look.next();
                while next == Some(' ') {
                    next = look.next();
                }
                if next.is_none() || next == Some(',') {
                    quote = None;
                    cur.remove(0);
                    out.push(std::mem::take(&mut cur));
                    // Skip to just past the separator.
                    while let Some(n) = chars.next() {
                        if n == ',' {
                            break;
                        }
                    }
                    if chars.peek().is_none() {
                        return out;
                    }
                    continue;
                }
                cur.push(c);
            }
            ',' if quote.is_none() => out.push(std::mem::take(&mut cur).trim().to_string()),
            c => cur.push(c),
        }
    }
    let last = cur.trim().to_string();
    out.push(match quote {
        // An unterminated quote was a literal character after all.
        Some(_) => last,
        None => last,
    });
    out
}

/// Pick the script rules that apply to the player hosts out of filter-list text.
/// Exception filters (`#@#+js(...)`) remove the matching rule for the hosts they name.
pub fn parse(lists: &str) -> Vec<Rule> {
    let mut rules: Vec<Rule> = Vec::new();
    let mut exceptions: Vec<Rule> = Vec::new();
    for line in lists.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('!') || !line.contains("youtube.com") {
            continue;
        }
        let (domains, body, exception) = if let Some((d, b)) = line.split_once("#@#+js(") {
            (d, b, true)
        } else if let Some((d, b)) = line.split_once("##+js(") {
            (d, b, false)
        } else {
            continue;
        };
        let Some(body) = body.strip_suffix(')') else { continue };
        let entries: Vec<&str> = domains.split(',').map(str::trim).filter(|d| !d.is_empty()).collect();
        let hosts: Vec<String> = HOSTS
            .iter()
            .filter(|h| {
                entries.iter().any(|e| !e.starts_with('~') && covers(e, h))
                    && !entries.iter().any(|e| e.strip_prefix('~').is_some_and(|e| covers(e, h)))
            })
            .map(|h| h.to_string())
            .collect();
        if hosts.is_empty() {
            continue;
        }
        let mut args = split_args(body);
        if args.is_empty() || args[0].is_empty() {
            continue;
        }
        let name = args.remove(0).trim_end_matches(".js").to_string();
        let rule = Rule { hosts, name, args };
        if exception {
            exceptions.push(rule);
        } else if !rules.contains(&rule) {
            rules.push(rule);
        }
    }
    for ex in exceptions {
        for r in rules.iter_mut().filter(|r| r.name == ex.name && r.args == ex.args) {
            r.hosts.retain(|h| !ex.hosts.contains(h));
        }
    }
    rules.retain(|r| !r.hosts.is_empty());
    rules
}

async fn download(http: &reqwest::Client, mirrors: &[&str]) -> Option<String> {
    for url in mirrors {
        let r = http.get(*url).timeout(Duration::from_secs(60)).send().await;
        match r {
            Ok(r) if r.status().is_success() => match r.text().await {
                Ok(t) if t.len() > 1000 => return Some(t),
                _ => {}
            },
            Ok(r) => tracing::warn!("adblock list {url}: HTTP {}", r.status()),
            Err(e) => tracing::warn!("adblock list {url}: {e}"),
        }
    }
    None
}

fn cached(cache_dir: &Path) -> Option<RuleSet> {
    serde_json::from_slice(&std::fs::read(rules_file(cache_dir)).ok()?).ok()
}

/// Download the lists and rewrite the rules file. Keeps the old file when a list cannot be
/// fetched or the result looks broken (no rules at all).
pub async fn refresh(http: &reqwest::Client, cache_dir: &Path) -> Result<usize, String> {
    let mut text = String::new();
    for mirrors in LISTS {
        let t = download(http, mirrors).await.ok_or("could not download uBlock's filter lists")?;
        text.push_str(&t);
        text.push('\n');
    }
    let rules = parse(&text);
    if rules.is_empty() {
        return Err("uBlock's lists contained no YouTube rules".into());
    }
    let set = RuleSet {
        fetched_at: time::OffsetDateTime::now_utc().unix_timestamp(),
        rules,
    };
    let bytes = serde_json::to_vec(&set).map_err(|e| e.to_string())?;
    crate::config::write_atomic(&rules_file(cache_dir), &bytes).map_err(|e| e.to_string())?;
    Ok(set.rules.len())
}

/// Write the built-in rules if there is no rules file yet, then keep the file up to date for
/// as long as the app runs.
pub async fn keep_fresh(http: reqwest::Client, cache_dir: PathBuf) {
    if cached(&cache_dir).is_none() {
        let set = RuleSet { fetched_at: 0, rules: parse(BUILTIN) };
        if let Ok(b) = serde_json::to_vec(&set) {
            let _ = crate::config::write_atomic(&rules_file(&cache_dir), &b);
        }
    }
    loop {
        let age = cached(&cache_dir)
            .map(|s| time::OffsetDateTime::now_utc().unix_timestamp() - s.fetched_at)
            .unwrap_or(i64::MAX);
        if age >= REFRESH.as_secs() as i64 {
            match refresh(&http, &cache_dir).await {
                Ok(n) => tracing::info!("adblock: {n} YouTube rules from uBlock's lists"),
                Err(e) => tracing::warn!("adblock: {e}; keeping the previous rules"),
            }
        }
        tokio::time::sleep(Duration::from_secs(1800)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_arguments() {
        assert_eq!(split_args("set, a.b, undefined"), ["set", "a.b", "undefined"]);
        assert_eq!(
            split_args(r#"trusted-rpfr, '"adPlacements"', '"no_ads"', player?"#),
            ["trusted-rpfr", "\"adPlacements\"", "\"no_ads\"", "player?"]
        );
        assert_eq!(split_args(r"x, /a\,b/, , c"), ["x", "/a,b/", "", "c"]);
        assert_eq!(split_args("nano-stb, [native code], 17000, 0.001").len(), 4);
    }

    #[test]
    fn picks_youtube_rules() {
        let lists = "! comment\n\
            example.com##+js(set, a, 1)\n\
            youtube.com##+js(json-prune, playerResponse.adPlacements adSlots)\n\
            m.youtube.com,music.youtube.com##+js(set, x.y, undefined)\n\
            tv.youtube.com##+js(set, tv, 1)\n\
            youtube.com,~m.youtube.com##+js(nano-stb, [native code], 17000, 0.001)\n\
            www.youtube.com##.some-cosmetic-filter\n\
            www.youtube.com##+js(rmnt, script, bad)\n\
            www.youtube.com#@#+js(rmnt, script, bad)\n";
        let r = parse(lists);
        assert_eq!(r.len(), 3);
        assert_eq!(r[0].name, "json-prune");
        assert_eq!(r[0].hosts, ["www.youtube.com", "m.youtube.com"]);
        assert_eq!(r[0].args, ["playerResponse.adPlacements adSlots"]);
        assert_eq!(r[1].hosts, ["m.youtube.com"]);
        assert_eq!((r[2].name.as_str(), r[2].hosts.len()), ("nano-stb", 1));
    }

    #[test]
    fn builtin_rules_parse() {
        let r = parse(BUILTIN);
        assert!(r.len() >= 12);
        assert!(r.iter().all(|r| !r.hosts.is_empty()));
    }
}
