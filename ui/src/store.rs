//! Small things the interface remembers between runs: where each video was left, the search
//! history and the player preferences. Kept in one JSON file in the app's config folder.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Instant;

use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    /// Video id → (seconds, length) of where playback stopped.
    pub resume: HashMap<String, (f64, f64)>,
    /// Newest first.
    pub searches: Vec<String>,
    pub search_history_off: bool,
    /// Preferred caption language ("" = captions off).
    pub caption_lang: String,
    pub loop_video: bool,
    pub stable_volume: bool,
    pub ambient_off: bool,
    pub music_mode: bool,
    /// Songs play as video (YouTube Music's Song/Video switch).
    pub music_video: bool,
}

pub struct Store {
    pub saved: Saved,
    file: Option<PathBuf>,
    dirty: Option<Instant>,
}

impl Store {
    pub fn load(dir: Option<PathBuf>) -> Self {
        let file = dir.map(|d| d.join("ui-state.json"));
        let saved = file
            .as_ref()
            .and_then(|f| std::fs::read(f).ok())
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Self { saved, file, dirty: None }
    }

    /// Mark changed; written a moment later (positions change every second).
    pub fn touch(&mut self) {
        self.dirty.get_or_insert_with(Instant::now);
    }

    /// Write if something changed a few seconds ago (or now, with `force`).
    pub fn flush(&mut self, force: bool) {
        let due = self.dirty.is_some_and(|t| force || t.elapsed().as_secs() >= 3);
        if !due {
            return;
        }
        self.dirty = None;
        if let (Some(f), Ok(b)) = (&self.file, serde_json::to_vec(&self.saved)) {
            let _ = streampipe_core::config::write_atomic(f, &b);
        }
    }

    /// Where to start a video: its saved place, unless it was nearly finished.
    pub fn resume_at(&self, id: &str) -> Option<f64> {
        let (at, len) = *self.saved.resume.get(id)?;
        (at > 10.0 && (len <= 0.0 || at < len - 20.0)).then_some(at)
    }

    /// How much of a video was watched, 0–1, for the red bar on thumbnails.
    pub fn watched(&self, id: &str) -> Option<f32> {
        let (at, len) = *self.saved.resume.get(id)?;
        (len > 0.0).then(|| (at / len).clamp(0.0, 1.0) as f32)
    }

    pub fn played(&mut self, id: &str, at: f64, len: f64) {
        if at < 5.0 {
            return;
        }
        if self.saved.resume.len() > 2000 {
            // Forget the oldest half (no order is kept, so any half will do).
            let keep: Vec<String> = self.saved.resume.keys().take(1000).cloned().collect();
            self.saved.resume.retain(|k, _| keep.contains(k));
        }
        self.saved.resume.insert(id.to_string(), (at, len));
        self.touch();
    }

    pub fn searched(&mut self, q: &str) {
        if self.saved.search_history_off {
            return;
        }
        self.saved.searches.retain(|s| !s.eq_ignore_ascii_case(q));
        self.saved.searches.insert(0, q.to_string());
        self.saved.searches.truncate(30);
        self.touch();
    }
}
