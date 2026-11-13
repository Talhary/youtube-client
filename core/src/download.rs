//! Download engine (PRD 3.3).
//!
//! - Streams are fetched in ~10 MB pieces with googlevideo's `range` URL parameter.
//! - A global semaphore caps connections across every download (default 8, max 16); each
//!   stream runs up to `chunks_per_stream` pieces; `parallel_videos` videos run at once.
//! - Each piece retries with exponential backoff and jitter (1 s → 30 s, 5 tries). A 403
//!   re-resolves the stream URLs and resumes.
//! - Output is preallocated, pieces are written at their offsets, and a sidecar JSON records
//!   finished pieces, so downloads survive restarts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use futures_util::{StreamExt, TryStreamExt};
use parking_lot::{Mutex, RwLock};
use rand::Rng;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::Semaphore;

use crate::config::{AudioOut, MAX_CONNECTIONS_CAP, write_atomic};
use crate::mux::Muxer;
use crate::streams::{DownloadPlan, ResolvedStreams, StreamChoice, plan_from_resolved};

pub const CHUNK_SIZE: u64 = 10 * 1024 * 1024;
const MAX_TRIES: u32 = 5;

pub type JobId = u64;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum DownloadKind {
    Video { max_height: Option<u32> },
    Audio(AudioOut),
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DownloadRequest {
    pub video_id: String,
    pub title: String,
    pub kind: DownloadKind,
    /// The playlist this download came from, if it was queued as part of one.
    #[serde(default)]
    pub group: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum JobStatus {
    Queued,
    Resolving,
    Downloading,
    Muxing,
    Paused,
    Done(PathBuf),
    Failed(String),
    Cancelled,
}

impl JobStatus {
    pub fn is_active(&self) -> bool {
        matches!(
            self,
            JobStatus::Queued | JobStatus::Resolving | JobStatus::Downloading | JobStatus::Muxing
        )
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum DownloadEvent {
    Added {
        id: JobId,
        req: DownloadRequest,
        status: JobStatus,
    },
    Status {
        id: JobId,
        status: JobStatus,
    },
    Progress {
        id: JobId,
        done: u64,
        total: u64,
        bytes_per_s: u64,
    },
    Removed {
        id: JobId,
    },
}

#[derive(Debug, thiserror::Error)]
enum DlError {
    #[error("stream URL expired or was rejected (403)")]
    Forbidden,
    #[error("HTTP {0}")]
    Status(u16),
    #[error("network: {0}")]
    Net(String),
    #[error("disk: {0}")]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Fatal(String),
}

/// A semaphore whose limit can be raised or lowered at runtime.
struct DynSemaphore {
    sem: Arc<Semaphore>,
    limit: AtomicUsize,
}

impl DynSemaphore {
    fn new(n: usize) -> Self {
        Self {
            sem: Arc::new(Semaphore::new(n)),
            limit: AtomicUsize::new(n),
        }
    }

    fn set(&self, n: usize) {
        let old = self.limit.swap(n, Ordering::SeqCst);
        if n > old {
            self.sem.add_permits(n - old);
        } else if n < old {
            let sem = self.sem.clone();
            let k = (old - n) as u32;
            tokio::spawn(async move {
                if let Ok(p) = sem.acquire_many(k).await {
                    p.forget();
                }
            });
        }
    }

    async fn acquire(&self) -> tokio::sync::SemaphorePermit<'_> {
        self.sem.acquire().await.expect("semaphore closed")
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_connections: u8,
    pub chunks_per_stream: u8,
    pub parallel_videos: u8,
}

struct Job {
    req: DownloadRequest,
    status: JobStatus,
    task: Option<tokio::task::AbortHandle>,
}

#[derive(Serialize, Deserialize)]
struct SavedJob {
    id: JobId,
    req: DownloadRequest,
    status: JobStatus,
}

type Emit = Arc<dyn Fn(DownloadEvent) + Send + Sync>;

/// Resolves a video id to streams. The flag asks for fresh URLs (after a 403), bypassing caches.
pub type ResolveFn = Arc<
    dyn Fn(String, bool) -> futures_util::future::BoxFuture<'static, Result<ResolvedStreams, String>>
        + Send
        + Sync,
>;

pub struct Downloads {
    inner: Arc<Inner>,
}

struct Inner {
    http: reqwest::Client,
    resolve: ResolveFn,
    muxer: Arc<dyn Muxer>,
    conns: DynSemaphore,
    videos: DynSemaphore,
    chunks_per_stream: AtomicUsize,
    jobs: Mutex<BTreeMap<JobId, Job>>,
    next_id: AtomicU64,
    out_dir: RwLock<PathBuf>,
    queue_file: PathBuf,
    emit: Emit,
}

impl Downloads {
    pub fn new(
        http: reqwest::Client,
        resolve: ResolveFn,
        muxer: Arc<dyn Muxer>,
        limits: Limits,
        out_dir: PathBuf,
        data_dir: &Path,
        emit: Emit,
    ) -> Self {
        let inner = Arc::new(Inner {
            http,
            resolve,
            muxer,
            conns: DynSemaphore::new(limits.max_connections.clamp(1, MAX_CONNECTIONS_CAP).into()),
            videos: DynSemaphore::new(limits.parallel_videos.max(1).into()),
            chunks_per_stream: AtomicUsize::new(limits.chunks_per_stream.max(1).into()),
            jobs: Mutex::new(BTreeMap::new()),
            next_id: AtomicU64::new(1),
            out_dir: RwLock::new(out_dir),
            queue_file: data_dir.join("downloads.json"),
            emit,
        });
        Self { inner }
    }

    /// Reload the persisted queue and resume unfinished jobs. Call once at startup.
    pub fn restore(&self) {
        let saved: Vec<SavedJob> = std::fs::read(&self.inner.queue_file)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        let max = saved.iter().map(|j| j.id).max().unwrap_or(0);
        self.inner.next_id.fetch_max(max + 1, Ordering::SeqCst);
        for j in saved {
            if self.inner.jobs.lock().contains_key(&j.id) {
                continue;
            }
            let status = if j.status.is_active() {
                JobStatus::Queued
            } else {
                j.status
            };
            (self.inner.emit)(DownloadEvent::Added {
                id: j.id,
                req: j.req.clone(),
                status: status.clone(),
            });
            let resume = status == JobStatus::Queued;
            self.inner.jobs.lock().insert(
                j.id,
                Job {
                    req: j.req,
                    status,
                    task: None,
                },
            );
            if resume {
                spawn_job(&self.inner, j.id);
            }
        }
    }

    pub fn set_limits(&self, l: Limits) {
        let i = &self.inner;
        i.conns.set(l.max_connections.clamp(1, MAX_CONNECTIONS_CAP).into());
        i.videos.set(l.parallel_videos.max(1).into());
        i.chunks_per_stream
            .store(l.chunks_per_stream.max(1).into(), Ordering::SeqCst);
    }

    pub fn set_out_dir(&self, dir: PathBuf) {
        *self.inner.out_dir.write() = dir;
    }


    pub fn enqueue(&self, req: DownloadRequest) -> JobId {
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        (self.inner.emit)(DownloadEvent::Added {
            id,
            req: req.clone(),
            status: JobStatus::Queued,
        });
        self.inner.jobs.lock().insert(
            id,
            Job {
                req,
                status: JobStatus::Queued,
                task: None,
            },
        );
        persist(&self.inner);
        spawn_job(&self.inner, id);
        id
    }

    pub fn pause(&self, id: JobId) {
        let mut jobs = self.inner.jobs.lock();
        if let Some(j) = jobs.get_mut(&id) {
            if j.status.is_active() {
                if let Some(t) = j.task.take() {
                    t.abort();
                }
                j.status = JobStatus::Paused;
                (self.inner.emit)(DownloadEvent::Status {
                    id,
                    status: JobStatus::Paused,
                });
            }
        }
        drop(jobs);
        persist(&self.inner);
    }

    pub fn resume(&self, id: JobId) {
        let ok = {
            let mut jobs = self.inner.jobs.lock();
            match jobs.get_mut(&id) {
                Some(j) if matches!(j.status, JobStatus::Paused | JobStatus::Failed(_)) => {
                    j.status = JobStatus::Queued;
                    true
                }
                _ => false,
            }
        };
        if ok {
            set_status(&self.inner, id, JobStatus::Queued);
            spawn_job(&self.inner, id);
        }
    }

    /// Stop a job and delete its partial files.
    pub fn cancel(&self, id: JobId) {
        if let Some(j) = self.inner.jobs.lock().get_mut(&id) {
            if let Some(t) = j.task.take() {
                t.abort();
            }
            j.status = JobStatus::Cancelled;
        }
        remove_partials(&self.inner, id);
        (self.inner.emit)(DownloadEvent::Status {
            id,
            status: JobStatus::Cancelled,
        });
        persist(&self.inner);
    }

    /// Drop finished, failed and cancelled jobs from the list.
    pub fn clear_finished(&self) {
        let removed: Vec<JobId> = {
            let mut jobs = self.inner.jobs.lock();
            let ids: Vec<JobId> = jobs
                .iter()
                .filter(|(_, j)| {
                    matches!(
                        j.status,
                        JobStatus::Done(_) | JobStatus::Cancelled | JobStatus::Failed(_)
                    )
                })
                .map(|(id, _)| *id)
                .collect();
            for id in &ids {
                jobs.remove(id);
            }
            ids
        };
        for id in removed {
            remove_partials(&self.inner, id);
            (self.inner.emit)(DownloadEvent::Removed { id });
        }
        persist(&self.inner);
    }
}

fn partial_dir(inner: &Inner) -> PathBuf {
    inner.out_dir.read().join(".streampipe-partial")
}

fn remove_partials(inner: &Inner, id: JobId) {
    let prefix = format!("{id}-");
    if let Ok(rd) = std::fs::read_dir(partial_dir(inner)) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

fn persist(inner: &Inner) {
    let saved: Vec<SavedJob> = inner
        .jobs
        .lock()
        .iter()
        .filter(|(_, j)| !matches!(j.status, JobStatus::Cancelled))
        .map(|(id, j)| SavedJob {
            id: *id,
            req: j.req.clone(),
            status: j.status.clone(),
        })
        .collect();
    if let Ok(b) = serde_json::to_vec_pretty(&saved) {
        let _ = write_atomic(&inner.queue_file, &b);
    }
}

fn set_status(inner: &Inner, id: JobId, status: JobStatus) {
    if let Some(j) = inner.jobs.lock().get_mut(&id) {
        j.status = status.clone();
    }
    (inner.emit)(DownloadEvent::Status { id, status });
}

fn spawn_job(inner: &Arc<Inner>, id: JobId) {
    let i2 = inner.clone();
    let handle = tokio::spawn(async move {
        let req = match i2.jobs.lock().get(&id) {
            Some(j) => j.req.clone(),
            None => return,
        };
        let result = run_job(&i2, id, &req).await;
        let status = match result {
            Ok(path) => JobStatus::Done(path),
            Err(e) => JobStatus::Failed(e),
        };
        set_status(&i2, id, status);
        if let Some(j) = i2.jobs.lock().get_mut(&id) {
            j.task = None;
        }
        persist(&i2);
    });
    if let Some(j) = inner.jobs.lock().get_mut(&id) {
        j.task = Some(handle.abort_handle());
    }
}

/// Shared, refreshable URLs for one job's streams.
struct Urls {
    video: RwLock<Option<String>>,
    audio: RwLock<String>,
    video_headers: RwLock<Vec<(String, String)>>,
    audio_headers: RwLock<Vec<(String, String)>>,
    generation: AtomicU64,
    lock: tokio::sync::Mutex<()>,
}

struct JobCtx<'a> {
    inner: &'a Inner,
    id: JobId,
    req: &'a DownloadRequest,
    plan: DownloadPlan,
    urls: Urls,
    done: AtomicU64,
    total: u64,
}

impl JobCtx<'_> {
    /// Re-resolve stream URLs after a 403. Only the first caller per generation hits the
    /// network; the rest wait and reuse the result.
    async fn reresolve(&self, seen_gen: u64) -> Result<(), DlError> {
        let _g = self.urls.lock.lock().await;
        if self.urls.generation.load(Ordering::SeqCst) != seen_gen {
            return Ok(());
        }
        tracing::info!(job = self.id, "403: re-resolving stream URLs");
        let fresh = (self.inner.resolve)(self.req.video_id.clone(), true)
            .await
            .map_err(DlError::Fatal)?;
        if let Some(v) = &self.plan.video {
            let n = fresh
                .video
                .iter()
                .find(|s| s.itag == v.itag)
                .ok_or_else(|| DlError::Fatal("video format no longer offered".into()))?;
            *self.urls.video.write() = Some(n.url.clone());
            *self.urls.video_headers.write() = n.headers.clone();
        }
        let a = [&fresh.audio_aac, &fresh.audio_opus]
            .into_iter()
            .flatten()
            .find(|s| s.itag == self.plan.audio.itag)
            .ok_or_else(|| DlError::Fatal("audio format no longer offered".into()))?;
        *self.urls.audio.write() = a.url.clone();
        *self.urls.audio_headers.write() = a.headers.clone();
        self.urls.generation.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

async fn run_job(inner: &Arc<Inner>, id: JobId, req: &DownloadRequest) -> Result<PathBuf, String> {
    let _slot = inner.videos.acquire().await;
    set_status(inner, id, JobStatus::Resolving);

    let resolved = (inner.resolve)(req.video_id.clone(), false).await?;
    let plan = match &req.kind {
        DownloadKind::Video { max_height } => plan_from_resolved(&resolved, *max_height, None),
        DownloadKind::Audio(out) => plan_from_resolved(&resolved, None, Some(*out)),
    }
    .ok_or("no downloadable streams for this video")?;
    if (plan.video.is_some() || plan.transcode_mp3 || plan.ext == "opus") && !inner.muxer.available()
    {
        return Err("FFmpeg is needed for this download. Install it from Settings.".into());
    }

    let out_dir = inner.out_dir.read().clone();
    let part_dir = out_dir.join(".streampipe-partial");
    tokio::fs::create_dir_all(&part_dir)
        .await
        .map_err(|e| format!("cannot create {}: {e}", part_dir.display()))?;

    let mut streams: Vec<(StreamChoice, PathBuf)> = Vec::new();
    if let Some(v) = &plan.video {
        streams.push((v.clone(), part_dir.join(format!("{id}-{}.part", v.itag))));
    }
    streams.push((
        plan.audio.clone(),
        part_dir.join(format!("{id}-{}.part", plan.audio.itag)),
    ));

    // Sizes: prefer the API value, then the `clen` URL parameter, then a HEAD request.
    let mut sizes = Vec::new();
    for (s, _) in &streams {
        sizes.push(stream_size(&inner.http, s).await.map_err(|e| e.to_string())?);
    }
    let total: u64 = sizes.iter().sum();

    // Free space: the streams plus the muxed copy, plus 50 MB slack.
    let copies = if plan.video.is_some() || plan.transcode_mp3 || plan.ext == "opus" {
        2
    } else {
        1
    };
    let needed = total * copies + 50 * 1024 * 1024;
    if let Ok(free) = fs4::available_space(&out_dir) {
        if free < needed {
            return Err(format!(
                "not enough disk space: need {} MB, {} MB free",
                needed >> 20,
                free >> 20
            ));
        }
    }

    let ctx = JobCtx {
        inner,
        id,
        req,
        urls: Urls {
            video: RwLock::new(plan.video.as_ref().map(|v| v.url.clone())),
            audio: RwLock::new(plan.audio.url.clone()),
            video_headers: RwLock::new(plan.video.as_ref().map(|v| v.headers.clone()).unwrap_or_default()),
            audio_headers: RwLock::new(plan.audio.headers.clone()),
            generation: AtomicU64::new(0),
            lock: tokio::sync::Mutex::new(()),
        },
        plan: plan.clone(),
        done: AtomicU64::new(0),
        total,
    };

    set_status(inner, id, JobStatus::Downloading);
    let ticker = progress_ticker(&ctx);
    let downloads = futures_util::future::try_join_all(
        streams
            .iter()
            .zip(&sizes)
            .map(|((s, path), size)| fetch_stream(&ctx, s.height > 0, s.itag, path, *size)),
    );
    let r = tokio::select! {
        r = downloads => r,
        _ = ticker => unreachable!(),
    };
    r.map_err(|e| e.to_string())?;
    (inner.emit)(DownloadEvent::Progress {
        id,
        done: total,
        total,
        bytes_per_s: 0,
    });

    // Mux / remux / transcode into the final file.
    set_status(inner, id, JobStatus::Muxing);
    let final_path = unique_path(&out_dir, &sanitize(&req.title), &plan.ext);
    let muxer = inner.muxer.clone();
    let audio_part = streams.last().unwrap().1.clone();
    let video_part = plan.video.as_ref().map(|_| streams[0].1.clone());
    let fp = final_path.clone();
    let transcode = plan.transcode_mp3;
    let ext = plan.ext.clone();
    tokio::task::spawn_blocking(move || match video_part {
        Some(v) => muxer.mux(&v, &audio_part, &fp),
        None if transcode => muxer.to_mp3(&audio_part, &fp),
        None if ext == "opus" || muxer.remuxes_m4a() => muxer.remux_audio(&audio_part, &fp),
        // Same volume (the partial dir sits inside the output dir), so this is a move.
        None => std::fs::rename(&audio_part, &fp).map_err(|e| e.to_string()),
    })
    .await
    .map_err(|e| e.to_string())??;

    remove_partials(inner, id);
    tracing::info!(job = id, path = %final_path.display(), "download finished");
    Ok(final_path)
}

async fn progress_ticker(ctx: &JobCtx<'_>) {
    let mut last = (Instant::now(), ctx.done.load(Ordering::Relaxed));
    let mut rate = 0f64;
    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let now = Instant::now();
        let done = ctx.done.load(Ordering::Relaxed);
        let inst = done.saturating_sub(last.1) as f64 / now.duration_since(last.0).as_secs_f64();
        rate = if rate == 0.0 { inst } else { rate * 0.7 + inst * 0.3 };
        last = (now, done);
        (ctx.inner.emit)(DownloadEvent::Progress {
            id: ctx.id,
            done,
            total: ctx.total,
            bytes_per_s: rate as u64,
        });
    }
}

#[derive(Serialize, Deserialize, PartialEq)]
struct ChunkState {
    itag: u32,
    size: u64,
    chunk: u64,
    done: Vec<bool>,
}

async fn fetch_stream(
    ctx: &JobCtx<'_>,
    is_video: bool,
    itag: u32,
    path: &Path,
    size: u64,
) -> Result<(), DlError> {
    let sidecar = path.with_extension("json");
    let n = size.div_ceil(CHUNK_SIZE) as usize;
    let fresh = ChunkState {
        itag,
        size,
        chunk: CHUNK_SIZE,
        done: vec![false; n],
    };
    let state: ChunkState = tokio::fs::read(&sidecar)
        .await
        .ok()
        .and_then(|b| serde_json::from_slice::<ChunkState>(&b).ok())
        .filter(|s| s.itag == itag && s.size == size && s.chunk == CHUNK_SIZE && s.done.len() == n)
        .filter(|_| path.exists())
        .unwrap_or(fresh);

    // Preallocate.
    {
        let f = tokio::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(path)
            .await?;
        if f.metadata().await?.len() != size {
            f.set_len(size).await?;
        }
    }

    let already: u64 = state
        .done
        .iter()
        .enumerate()
        .filter(|(_, d)| **d)
        .map(|(i, _)| chunk_len(i, size))
        .sum();
    ctx.done.fetch_add(already, Ordering::Relaxed);

    let state = Mutex::new(state);
    let pending: Vec<usize> = state
        .lock()
        .done
        .iter()
        .enumerate()
        .filter(|(_, d)| !**d)
        .map(|(i, _)| i)
        .collect();

    let par = ctx.inner.chunks_per_stream.load(Ordering::SeqCst).max(1);
    futures_util::stream::iter(pending)
        .map(|i| fetch_chunk_retrying(ctx, is_video, path, i, size))
        .buffer_unordered(par)
        .map_ok(|i| {
            let bytes = {
                let mut st = state.lock();
                st.done[i] = true;
                serde_json::to_vec(&*st).unwrap_or_default()
            };
            let _ = write_atomic(&sidecar, &bytes);
        })
        .try_collect::<Vec<()>>()
        .await?;

    // Verify against content length.
    let len = tokio::fs::metadata(path).await?.len();
    if len != size || state.lock().done.iter().any(|d| !d) {
        return Err(DlError::Fatal(format!(
            "size check failed for itag {itag}: {len} of {size} bytes"
        )));
    }
    Ok(())
}

fn chunk_len(i: usize, size: u64) -> u64 {
    let start = i as u64 * CHUNK_SIZE;
    (size - start).min(CHUNK_SIZE)
}

fn backoff(attempt: u32) -> Duration {
    let base = (1u64 << attempt.min(5)).min(30) as f64;
    let jitter = rand::rng().random_range(0.5..1.5);
    Duration::from_secs_f64((base * jitter).min(30.0))
}

async fn fetch_chunk_retrying(
    ctx: &JobCtx<'_>,
    is_video: bool,
    path: &Path,
    i: usize,
    size: u64,
) -> Result<usize, DlError> {
    let start = i as u64 * CHUNK_SIZE;
    let end = start + chunk_len(i, size) - 1;
    let mut last_err = DlError::Fatal("no attempts made".into());
    for attempt in 0..MAX_TRIES {
        let generation = ctx.urls.generation.load(Ordering::SeqCst);
        let (url, headers) = if is_video {
            (
                ctx.urls.video.read().clone().unwrap_or_default(),
                ctx.urls.video_headers.read().clone(),
            )
        } else {
            (ctx.urls.audio.read().clone(), ctx.urls.audio_headers.read().clone())
        };
        let permit = ctx.inner.conns.acquire().await;
        let mut got = 0u64;
        let r = fetch_range(ctx, &url, &headers, path, start, end, &mut got).await;
        drop(permit);
        match r {
            Ok(()) => return Ok(i),
            Err(e) => {
                ctx.done.fetch_sub(got, Ordering::Relaxed);
                tracing::warn!(job = ctx.id, chunk = i, attempt, "chunk failed: {e}");
                match e {
                    DlError::Forbidden => ctx.reresolve(generation).await?,
                    DlError::Fatal(_) => return Err(e),
                    DlError::Io(ref io) if io.kind() == std::io::ErrorKind::StorageFull => {
                        return Err(e);
                    }
                    _ => tokio::time::sleep(backoff(attempt)).await,
                }
                last_err = e;
            }
        }
    }
    Err(last_err)
}

async fn fetch_range(
    ctx: &JobCtx<'_>,
    url: &str,
    headers: &[(String, String)],
    path: &Path,
    start: u64,
    end: u64,
    got: &mut u64,
) -> Result<(), DlError> {
    let sep = if url.contains('?') { '&' } else { '?' };
    let mut req = ctx
        .inner
        .http
        .get(format!("{url}{sep}range={start}-{end}"))
        .timeout(Duration::from_secs(60));
    for (k, v) in headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let resp = req
        .send()
        .await
        .map_err(|e| DlError::Net(e.to_string()))?;
    match resp.status().as_u16() {
        200 | 206 => {}
        403 => return Err(DlError::Forbidden),
        s => return Err(DlError::Status(s)),
    }
    let mut f = tokio::fs::OpenOptions::new().write(true).open(path).await?;
    f.seek(std::io::SeekFrom::Start(start)).await?;
    let mut body = resp.bytes_stream();
    let expected = end - start + 1;
    while let Some(chunk) = body.next().await {
        let chunk = chunk.map_err(|e| DlError::Net(e.to_string()))?;
        if *got + chunk.len() as u64 > expected {
            return Err(DlError::Net("server sent more bytes than requested".into()));
        }
        f.write_all(&chunk).await?;
        *got += chunk.len() as u64;
        ctx.done.fetch_add(chunk.len() as u64, Ordering::Relaxed);
    }
    f.flush().await?;
    if *got != expected {
        return Err(DlError::Net(format!("short read: {got} of {expected} bytes")));
    }
    Ok(())
}

async fn stream_size(http: &reqwest::Client, s: &StreamChoice) -> Result<u64, DlError> {
    if let Some(n) = s.size.filter(|n| *n > 0) {
        return Ok(n);
    }
    if let Some(n) = clen(&s.url) {
        return Ok(n);
    }
    let mut req = http.head(&s.url);
    for (k, v) in &s.headers {
        req = req.header(k.as_str(), v.as_str());
    }
    let r = req
        .send()
        .await
        .map_err(|e| DlError::Net(e.to_string()))?;
    if r.status().as_u16() == 403 {
        return Err(DlError::Forbidden);
    }
    r.content_length()
        .filter(|n| *n > 0)
        .ok_or_else(|| DlError::Fatal("stream size unknown".into()))
}

pub(crate) fn clen(url: &str) -> Option<u64> {
    url.split(['?', '&'])
        .find_map(|kv| kv.strip_prefix("clen="))
        .and_then(|v| v.parse().ok())
}

/// Make a title safe as a file name on Windows, Linux and Android.
pub fn sanitize(title: &str) -> String {
    let s: String = title
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let s = s.trim().trim_end_matches(['.', ' ']).to_string();
    let s: String = s.chars().take(150).collect();
    let reserved = ["CON", "PRN", "AUX", "NUL", "COM1", "LPT1"];
    if s.is_empty() || reserved.contains(&s.to_ascii_uppercase().as_str()) {
        format!("video_{s}")
    } else {
        s
    }
}

fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let p = dir.join(format!("{stem}.{ext}"));
    if !p.exists() {
        return p;
    }
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}).{ext}")))
        .find(|p| !p.exists())
        .unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes() {
        assert_eq!(sanitize("a/b:c?"), "a_b_c_");
        assert_eq!(sanitize("  trailing.  "), "trailing");
        assert_eq!(sanitize("con"), "video_con");
    }

    #[test]
    fn chunks() {
        assert_eq!(chunk_len(0, 25 * 1024 * 1024), CHUNK_SIZE);
        assert_eq!(chunk_len(2, 25 * 1024 * 1024), 5 * 1024 * 1024);
        assert_eq!(clen("https://x/videoplayback?a=1&clen=12345&b=2"), Some(12345));
    }

    #[test]
    fn backoff_bounds() {
        for a in 0..10 {
            let d = backoff(a);
            assert!(d <= Duration::from_secs(30) && d >= Duration::from_millis(500));
        }
    }
}
