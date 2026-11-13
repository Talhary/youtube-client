//! Thumbnails off the UI thread (PRD 3.2): fetch and decode in tokio tasks, keep an LRU in
//! RAM (~64 MB decoded) and on disk (~200 MB encoded).

use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use lru::LruCache;
use parking_lot::Mutex;
use sha2::{Digest, Sha256};

pub const MEM_BUDGET: usize = 64 * 1024 * 1024;
pub const DISK_BUDGET: u64 = 200 * 1024 * 1024;
/// Thumbnails wider than this are downscaled before caching; cards never show them larger.
const MAX_W: u32 = 640;

#[derive(Debug)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

pub struct Thumbs {
    http: reqwest::Client,
    dir: PathBuf,
    mem: Mutex<Mem>,
    writes: AtomicU32,
}

struct Mem {
    lru: LruCache<String, Arc<Rgba>>,
    bytes: usize,
}

impl Thumbs {
    pub fn new(http: reqwest::Client, cache_dir: PathBuf) -> Self {
        let dir = cache_dir.join("thumbs");
        let _ = std::fs::create_dir_all(&dir);
        Self {
            http,
            dir,
            mem: Mutex::new(Mem {
                lru: LruCache::new(NonZeroUsize::new(4096).unwrap()),
                bytes: 0,
            }),
            writes: AtomicU32::new(0),
        }
    }

    fn disk_path(&self, url: &str) -> PathBuf {
        self.dir.join(hex::encode(&Sha256::digest(url.as_bytes())[..16]))
    }

    pub async fn get(&self, url: &str) -> Result<Arc<Rgba>, String> {
        if let Some(img) = self.mem.lock().lru.get(url) {
            return Ok(img.clone());
        }
        let path = self.disk_path(url);
        let bytes = match tokio::fs::read(&path).await {
            Ok(b) => {
                // Touch so disk eviction sees it as recently used.
                let _ = filetime_touch(&path);
                b
            }
            Err(_) => {
                let b = self
                    .http
                    .get(url)
                    .send()
                    .await
                    .and_then(|r| r.error_for_status())
                    .map_err(|e| e.to_string())?
                    .bytes()
                    .await
                    .map_err(|e| e.to_string())?
                    .to_vec();
                let _ = tokio::fs::write(&path, &b).await;
                if self.writes.fetch_add(1, Ordering::Relaxed) % 64 == 0 {
                    let dir = self.dir.clone();
                    tokio::task::spawn_blocking(move || evict_disk(&dir, DISK_BUDGET));
                }
                b
            }
        };
        let img = tokio::task::spawn_blocking(move || decode(&bytes))
            .await
            .map_err(|e| e.to_string())??;
        let img = Arc::new(img);
        self.insert(url.to_string(), img.clone());
        Ok(img)
    }

    /// Drop every cached thumbnail (memory and disk). Returns the bytes freed on disk.
    pub fn clear(&self) -> u64 {
        {
            let mut m = self.mem.lock();
            m.lru.clear();
            m.bytes = 0;
        }
        let mut freed = 0;
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let len = e.metadata().map(|m| m.len()).unwrap_or(0);
                if std::fs::remove_file(e.path()).is_ok() {
                    freed += len;
                }
            }
        }
        freed
    }

    fn insert(&self, url: String, img: Arc<Rgba>) {
        let mut m = self.mem.lock();
        m.bytes += img.pixels.len();
        if let Some(old) = m.lru.put(url, img) {
            m.bytes -= old.pixels.len();
        }
        while m.bytes > MEM_BUDGET {
            match m.lru.pop_lru() {
                Some((_, v)) => m.bytes -= v.pixels.len(),
                None => break,
            }
        }
    }
}

fn decode(bytes: &[u8]) -> Result<Rgba, String> {
    let img = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    let img = if img.width() > MAX_W {
        let h = img.height() * MAX_W / img.width();
        img.resize(MAX_W, h, image::imageops::FilterType::Triangle)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    Ok(Rgba {
        width: rgba.width(),
        height: rgba.height(),
        pixels: rgba.into_raw(),
    })
}

fn filetime_touch(path: &std::path::Path) -> std::io::Result<()> {
    let f = std::fs::OpenOptions::new().append(true).open(path)?;
    f.set_modified(std::time::SystemTime::now())
}

/// Delete least-recently-used files until the directory fits the budget.
fn evict_disk(dir: &std::path::Path, budget: u64) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            Some((m.modified().ok()?, m.len(), e.path()))
        })
        .collect();
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= budget {
        return;
    }
    files.sort_by_key(|f| f.0);
    for (_, len, path) in files {
        if total <= budget {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_eviction_keeps_newest() {
        let d = tempfile::tempdir().unwrap();
        for i in 0..5 {
            let p = d.path().join(format!("f{i}"));
            std::fs::write(&p, vec![0u8; 100]).unwrap();
            let t = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1000 + i);
            std::fs::OpenOptions::new()
                .append(true)
                .open(&p)
                .unwrap()
                .set_modified(t)
                .unwrap();
        }
        evict_disk(d.path(), 250);
        let left: Vec<_> = std::fs::read_dir(d.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(left.len(), 2);
        assert!(left.contains(&"f4".to_string()) && left.contains(&"f3".to_string()));
    }

    #[test]
    fn mem_budget_evicts() {
        let t = Thumbs::new(reqwest::Client::new(), tempfile::tempdir().unwrap().keep());
        let big = MEM_BUDGET / 3 + 1;
        for i in 0..4 {
            t.insert(
                format!("u{i}"),
                Arc::new(Rgba {
                    width: 1,
                    height: 1,
                    pixels: vec![0; big],
                }),
            );
        }
        let m = t.mem.lock();
        assert!(m.bytes <= MEM_BUDGET);
        assert_eq!(m.lru.len(), 2);
    }
}
