//! Pool of persistent yt-dlp worker processes (bundled Python + `worker.py`).
//!
//! Starting yt-dlp costs ~2.5 s per run (Python start-up, imports, fresh connections). Workers
//! stay loaded and take JSON requests over stdin/stdout, so each video only pays for the
//! YouTube requests themselves. Several workers let pre-loads run while the user browses.
//!
//! The first video in a fresh worker is still slow (cold connections, client config, Deno and
//! botguard start-up: ~4 s against ~1 s afterwards), so each worker runs a warm-up request
//! whenever it is idle and has not resolved anything yet.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout};
use tokio::sync::{Mutex, Semaphore, mpsc, oneshot};

#[derive(Clone)]
pub struct Spawn {
    pub python: PathBuf,
    pub script: PathBuf,
    pub env: Vec<(String, String)>,
}

struct Job {
    req: Value,
    timeout: Duration,
    reply: oneshot::Sender<Result<Value, String>>,
}

type Queues = (mpsc::UnboundedReceiver<Job>, mpsc::UnboundedReceiver<Job>);

/// Builds the warm-up request (called at warm-up time so it carries the current cookies).
pub type WarmReq = Arc<dyn Fn() -> Value + Send + Sync>;

pub struct Pool {
    /// Playback requests: always served first.
    high: mpsc::UnboundedSender<Job>,
    /// Everything else (lists, pre-loads).
    tx: mpsc::UnboundedSender<Job>,
    generation: Arc<AtomicU64>,
    next_id: Arc<AtomicU64>,
}

impl Pool {
    /// Start `workers` worker tasks; the first `warm` spawn their process immediately.
    /// With `warm_req`, every worker starts its process and warms it up while idle.
    pub fn start(spawn: Spawn, workers: usize, warm: usize, warm_req: Option<WarmReq>) -> Arc<Self> {
        let (high, high_rx) = mpsc::unbounded_channel::<Job>();
        let (tx, rx) = mpsc::unbounded_channel::<Job>();
        let rx = Arc::new(Mutex::new((high_rx, rx)));
        let generation = Arc::new(AtomicU64::new(0));
        let next_id = Arc::new(AtomicU64::new(1));
        // One worker always stays free for real requests while the others warm up.
        let warm_slots = Arc::new(Semaphore::new(workers.saturating_sub(1).max(1)));
        for i in 0..workers {
            let w = Worker {
                proc: None,
                spawn: spawn.clone(),
                warmed: false,
            };
            let ctx = LoopCtx {
                rx: rx.clone(),
                generation: generation.clone(),
                next_id: next_id.clone(),
                warm_req: warm_req.clone(),
                warm_slots: warm_slots.clone(),
            };
            tokio::spawn(async move { worker_loop(w, ctx, i < warm).await });
        }
        Arc::new(Self {
            high,
            tx,
            generation,
            next_id,
        })
    }

    pub async fn call(&self, req: Value, timeout: Duration) -> Result<Value, String> {
        self.call_with(req, timeout, false).await
    }

    /// `urgent` jumps ahead of every queued non-urgent request.
    pub async fn call_with(&self, mut req: Value, timeout: Duration, urgent: bool) -> Result<Value, String> {
        req["id"] = Value::from(self.next_id.fetch_add(1, Ordering::Relaxed));
        let (reply, rx) = oneshot::channel();
        let q = if urgent { &self.high } else { &self.tx };
        q.send(Job { req, timeout, reply })
            .map_err(|_| "yt-dlp workers stopped".to_string())?;
        rx.await.map_err(|_| "yt-dlp worker stopped".to_string())?
    }

    /// Restart every worker before its next job (after updating yt-dlp).
    pub fn restart(&self) {
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
}

struct Proc {
    _child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Proc {
    fn spawn(s: &Spawn) -> Result<Self, String> {
        let mut cmd = tokio::process::Command::new(&s.python);
        cmd.arg("-u")
            .arg(&s.script)
            .envs(s.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .env("PYTHONIOENCODING", "utf-8")
            .env("PYTHONDONTWRITEBYTECODE", "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        {
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let mut child = cmd.spawn().map_err(|e| format!("could not start yt-dlp worker: {e}"))?;
        let stdin = child.stdin.take().ok_or("worker has no stdin")?;
        let stdout = BufReader::new(child.stdout.take().ok_or("worker has no stdout")?);
        Ok(Self {
            _child: child,
            stdin,
            stdout,
        })
    }

    async fn ask(&mut self, req: &Value) -> Result<Value, String> {
        let id = req["id"].clone();
        let mut line = serde_json::to_string(req).map_err(|e| e.to_string())?;
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .await
            .map_err(|e| format!("yt-dlp worker: {e}"))?;
        self.stdin.flush().await.map_err(|e| e.to_string())?;
        loop {
            let mut buf = String::new();
            let n = self
                .stdout
                .read_line(&mut buf)
                .await
                .map_err(|e| format!("yt-dlp worker: {e}"))?;
            if n == 0 {
                return Err("yt-dlp worker exited".into());
            }
            let Ok(reply) = serde_json::from_str::<Value>(&buf) else {
                continue;
            };
            if reply["id"] != id {
                continue;
            }
            if reply["ok"].as_bool() == Some(true) {
                if let Some(secs) = reply["secs"].as_f64() {
                    tracing::info!("yt-dlp worker {} in {secs:.1}s", req["op"].as_str().unwrap_or("?"));
                }
                return Ok(reply["result"].clone());
            }
            return Err(reply["error"].as_str().unwrap_or("yt-dlp failed").to_string());
        }
    }
}

struct Worker {
    proc: Option<Proc>,
    spawn: Spawn,
    /// This process has already paid its start-up costs (warm-up or a resolved video).
    warmed: bool,
}

impl Worker {
    async fn ask(&mut self, req: &Value, timeout: Duration) -> Result<Value, String> {
        if self.proc.is_none() {
            self.proc = Some(Proc::spawn(&self.spawn)?);
            self.warmed = false;
        }
        let p = self.proc.as_mut().unwrap();
        match tokio::time::timeout(timeout, p.ask(req)).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => {
                // Protocol or process failure: start a fresh worker next time.
                if e.contains("worker") {
                    self.proc = None;
                }
                Err(e)
            }
            Err(_) => {
                self.proc = None;
                Err("yt-dlp took too long".into())
            }
        }
    }
}

struct LoopCtx {
    rx: Arc<Mutex<Queues>>,
    generation: Arc<AtomicU64>,
    next_id: Arc<AtomicU64>,
    warm_req: Option<WarmReq>,
    warm_slots: Arc<Semaphore>,
}

async fn next_job(rx: &Mutex<Queues>) -> Option<Job> {
    let mut q = rx.lock().await;
    let (high, low) = &mut *q;
    tokio::select! {
        biased;
        j = high.recv() => j,
        j = low.recv() => j,
    }
}

async fn worker_loop(mut w: Worker, ctx: LoopCtx, warm: bool) {
    let mut seen_gen = ctx.generation.load(Ordering::SeqCst);
    if warm {
        w.proc = Proc::spawn(&w.spawn).map_err(|e| tracing::warn!("{e}")).ok();
    }
    // Warm-ups left for this worker to try; a worker that cannot start stops trying.
    let mut warm_tries = 3u32;
    loop {
        let job = match &ctx.warm_req {
            // Not warmed up yet: take a waiting request if there is one, otherwise warm up.
            Some(make) if !w.warmed && warm_tries > 0 => {
                let waited = tokio::select! {
                    biased;
                    j = next_job(&ctx.rx) => Ok(j),
                    permit = ctx.warm_slots.acquire() => Err(permit),
                };
                match waited {
                    Ok(j) => j,
                    Err(_permit) => {
                        let mut req = make();
                        req["id"] = Value::from(ctx.next_id.fetch_add(1, Ordering::Relaxed));
                        let started = std::time::Instant::now();
                        let r = w.ask(&req, Duration::from_secs(60)).await;
                        // One warm-up per process, whatever it found; a process that died
                        // gets another go, a few times at most.
                        w.warmed = w.proc.is_some();
                        if w.warmed {
                            warm_tries = 3;
                        } else {
                            warm_tries -= 1;
                        }
                        tracing::info!(
                            "yt-dlp worker warm-up in {:.1}s ({})",
                            started.elapsed().as_secs_f32(),
                            match &r {
                                Ok(v) if v["ok"].as_bool() == Some(true) => "ok".to_string(),
                                Ok(_) => "no streams".to_string(),
                                Err(e) => e.clone(),
                            }
                        );
                        continue;
                    }
                }
            }
            _ => next_job(&ctx.rx).await,
        };
        let Some(job) = job else { break };
        let g = ctx.generation.load(Ordering::SeqCst);
        if g != seen_gen {
            w.proc = None;
            seen_gen = g;
        }
        let result = w.ask(&job.req, job.timeout).await;
        if result.is_ok() && job.req["op"] == "resolve" && job.req["full"].as_bool() != Some(true) {
            w.warmed = true;
        }
        let _ = job.reply.send(result);
    }
}
