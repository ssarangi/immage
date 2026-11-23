// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! Auto-keywords in the background: finds photos of the library that have no result for
//! the current model, runs RAM++ on each (in a short-lived helper engine process, engine/tags.c),
//! stores the tags (tags_store.rs) and attaches the confident ones as darktable keywords.
//! It runs only while the model is installed AND the "Add keywords automatically" switch is
//! on, on the shared analysis queue (so the one Pause covers it), never more than two photos
//! ahead of the worker.
//!
//! The 1.8 GB model lives in its own `darktable-engine` process, started by the first photo and
//! ended 20 s after the backlog is empty (or the switch goes off), so its memory goes back to
//! the operating system; it is not started when the Memory level or the free memory cannot
//! hold it (the photo stays pending and the page says why).
//!
//! A photo's picture comes from the file itself (a JPEG, or the preview inside a RAW,
//! analysis_image.rs), decoded once and shared; only a photo with no usable preview is rendered
//! by the main engine. With the burst setting on, a photo that looks like one already analysed in
//! its folder (cull.rs's perceptual hash) takes that photo's tags instead of being analysed.
//! The scan orders the backlog: rated and recent photos first, rejected ones never, ones Auto
//! Cull marked blurry last.
use crate::{
    analysis_image::Shared,
    analysis_queue::{AnalysisQueue, Job},
    engine::{Engine, Tagged},
    frame_output::FrameOutput,
    memory_budget,
    ram_models::{self, Models},
    search_analysis::{library_selection, now_ms, selected, AnalysisProgress, SHARE_QUIET},
    search_cache::{AssetMeta, SearchCache},
    tags_health::{self, Gate, Health, Kind, Ledger, Reason},
    tags_store::{Origin, TagStore},
};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicU8, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

/// Sends one `tags_apply` request to the engine's main loop (main.rs builds it).
pub type Applier = Arc<dyn Fn(Value) -> Result<Value, String> + Send + Sync>;

const SCAN_EVERY: Duration = Duration::from_secs(4);
const COUNT_EVERY: Duration = Duration::from_secs(5);
const WANTED_FOR_MS: u64 = 10_000;
const ACTIVE_FOR_MS: u64 = 15_000;
const PAGE: u32 = 256;
/// Photos handed to the queue at once; the engine worker takes one at a time.
const AHEAD: usize = 2;
/// Photos looked at in one scan, to put the most wanted first.
const WINDOW: usize = 1500;
/// The helper process is ended after this long without a photo to analyse.
const UNLOAD_AFTER: Duration = Duration::from_secs(20);
/// What the model needs while loaded, in megabytes (measured: 2.8 to 3.3 GB resident, plus a picture).
pub const NEED_MB: u64 = 3600;

/// Why the helper was not started (0 means it was, or no photo asked).
pub const BLOCK_NONE: u8 = 0;
pub const BLOCK_LEVEL: u8 = 1;
pub const BLOCK_FREE: u8 = 2;

/// What the status poll reads: memory only, like the Smart search service's.
pub struct Progress {
    installed: AtomicBool,
    enabled: AtomicBool,
    pending: AtomicI64,
    running: AtomicUsize,
    last_work_ms: AtomicU64,
    wanted_ms: AtomicU64,
    /// Milliseconds of the last inference.
    last_infer_ms: AtomicU64,
    /// Resident memory of the helper process, kB, last time a photo finished.
    last_rss_kb: AtomicU64,
    blocked: AtomicU8,
    /// Photos analysed from their own file / rendered by the engine / given a burst mate's tags.
    from_file: AtomicU64,
    rendered: AtomicU64,
    copied: AtomicU64,
}
impl Progress {
    fn new() -> Self {
        Progress {
            installed: AtomicBool::new(false),
            enabled: AtomicBool::new(false),
            pending: AtomicI64::new(-1),
            running: AtomicUsize::new(0),
            last_work_ms: AtomicU64::new(0),
            wanted_ms: AtomicU64::new(0),
            last_infer_ms: AtomicU64::new(0),
            last_rss_kb: AtomicU64::new(0),
            blocked: AtomicU8::new(BLOCK_NONE),
            from_file: AtomicU64::new(0),
            rendered: AtomicU64::new(0),
            copied: AtomicU64::new(0),
        }
    }
    fn count_requested(&self) -> bool {
        let wanted = self.wanted_ms.load(Ordering::Acquire);
        wanted != 0 && now_ms().saturating_sub(wanted) <= WANTED_FOR_MS
    }
}

struct RunGuard(Arc<Progress>);
impl RunGuard {
    fn new(p: &Arc<Progress>) -> Self {
        p.running.fetch_add(1, Ordering::AcqRel);
        RunGuard(p.clone())
    }
    fn stored(&self) {
        self.0.last_work_ms.store(now_ms(), Ordering::Release);
    }
}
impl Drop for RunGuard {
    fn drop(&mut self) {
        self.0.running.fetch_sub(1, Ordering::AcqRel);
    }
}

pub struct Control {
    stop: Arc<AtomicBool>,
    progress: Arc<Progress>,
    helper: Arc<Helper>,
    health: Arc<Health>,
    ledger: Arc<Ledger>,
}
impl Control {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
        self.helper.end();
    }
    /// For the status command: memory only. Asking also keeps the backlog counted for a while.
    pub fn status(&self, paused: bool) -> AnalysisProgress {
        let p = &self.progress;
        let now = now_ms();
        p.wanted_ms.store(now, Ordering::Release);
        let running = p.running.load(Ordering::Acquire) as u32;
        let last = p.last_work_ms.load(Ordering::Acquire);
        AnalysisProgress {
            installed: p.installed.load(Ordering::Acquire),
            enabled: p.enabled.load(Ordering::Acquire),
            pending: u32::try_from(p.pending.load(Ordering::Acquire)).ok(),
            running,
            active: running > 0 || (last != 0 && now.saturating_sub(last) < ACTIVE_FOR_MS),
            paused,
        }
    }
    /// The last measured inference time and the helper's memory, for the card's speed line.
    pub fn speed(&self) -> (Option<u32>, Option<u64>) {
        let ms = self.progress.last_infer_ms.load(Ordering::Acquire);
        let kb = self.progress.last_rss_kb.load(Ordering::Acquire);
        ((ms > 0).then_some(ms as u32), (kb > 0).then_some(kb))
    }
    /// Why photos are waiting without being analysed: the Memory level is too low, or too little is free.
    pub fn blocked(&self) -> u8 {
        self.progress.blocked.load(Ordering::Acquire)
    }
    /// Whether analysis is working, waiting to retry or blocked, and why (what the card shows).
    pub fn health(&self) -> tags_health::Report {
        self.health.report(now_ms())
    }
    /// Photos left alone after failing on their own.
    pub fn failed(&self) -> u32 {
        self.ledger.count(now_ms())
    }
    /// The user's Retry: forgets the failures so the worker tries again at once.
    pub fn retry(&self) {
        self.health.retry();
        self.ledger.reset();
        self.helper.end();
    }
    #[cfg(test)]
    /// The helper process's id while it runs (it holds the model), 0 otherwise.
    pub fn helper_pid(&self) -> u32 {
        if self.helper.running() { self.helper.pid.load(Ordering::Acquire) } else { 0 }
    }
    #[cfg(test)]
    /// Photos analysed from their file, rendered by the engine, and given a burst mate's tags.
    pub fn sources(&self) -> (u64, u64, u64) {
        let p = &self.progress;
        (p.from_file.load(Ordering::Acquire), p.rendered.load(Ordering::Acquire), p.copied.load(Ordering::Acquire))
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The helper `darktable-engine` that holds the model. Started by the first photo that needs it,
/// ended by `end` (a quit and exit), so its memory returns to the operating system.
pub struct Helper {
    bin: PathBuf,
    dir: PathBuf,
    engine: Mutex<Option<Engine>>,
    pid: AtomicU32,
}
impl Helper {
    fn new(bin: PathBuf, runtime: &Path) -> Self {
        Helper { bin, dir: runtime.join("tags-engine"), engine: Mutex::new(None), pid: AtomicU32::new(0) }
    }
    fn running(&self) -> bool {
        self.engine.lock().unwrap().is_some()
    }
    fn tagged(&self) -> Result<Tagged, String> {
        let mut engine = self.engine.lock().unwrap();
        if engine.is_none() {
            let started = Engine::spawn(&self.bin, &self.dir, &self.dir.join("models")).map_err(|e| format!("cannot start the keyword engine: {e}"))?;
            self.pid.store(started.pid().unwrap_or(0), Ordering::Release);
            *engine = Some(started);
        }
        Ok(engine.as_ref().unwrap().tagged())
    }
    /// The helper's resident memory in kB (Linux), 0 when it is not running or elsewhere.
    pub fn rss_kb(&self) -> u64 {
        rss_of(self.pid.load(Ordering::Acquire))
    }
    pub fn end(&self) {
        // dropping the engine quits the process
        let taken = self.engine.lock().unwrap().take();
        drop(taken);
        self.pid.store(0, Ordering::Release);
    }
}

/// Resident memory of a process in kB from /proc (Linux); 0 for none, or on other systems.
pub fn rss_of(pid: u32) -> u64 {
    if pid == 0 || !cfg!(target_os = "linux") {
        return 0;
    }
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|s| s.lines().find_map(|l| l.strip_prefix("VmRSS:").and_then(|v| v.split_whitespace().next()).and_then(|v| v.parse().ok())))
        .unwrap_or(0)
}

/// Whether the model can be loaded now: the Memory level's budget must hold it, and so must what is free.
pub fn memory_gate(total_mb: u64, available_mb: u64, level: &str) -> u8 {
    if memory_budget::compute(total_mb, level).budget_mb < NEED_MB {
        BLOCK_LEVEL
    } else if available_mb < NEED_MB {
        BLOCK_FREE
    } else {
        BLOCK_NONE
    }
}
fn memory_now(runtime: &Path) -> u8 {
    use sysinfo::{MemoryRefreshKind, RefreshKind, System};
    let sys = System::new_with_specifics(RefreshKind::nothing().with_memory(MemoryRefreshKind::nothing().with_ram()));
    memory_gate(sys.total_memory() >> 20, sys.available_memory() >> 20, &crate::engine::resource_level(runtime))
}

struct Service {
    cache: Arc<SearchCache>,
    models: Arc<Models>,
    store: Arc<TagStore>,
    queue: Arc<AnalysisQueue>,
    engine: Tagged,
    helper: Arc<Helper>,
    apply: Applier,
    output: Arc<FrameOutput>,
    shared: Shared,
    runtime: PathBuf,
    cursor: AtomicI64,
    stop: Arc<AtomicBool>,
    progress: Arc<Progress>,
    health: Arc<Health>,
    ledger: Arc<Ledger>,
    /// Paths handed to the queue and not finished; at most AHEAD.
    inflight: Mutex<HashSet<String>>,
    idle_since: Mutex<Option<Instant>>,
}

#[allow(clippy::too_many_arguments)]
pub fn start(
    cache: Option<Arc<SearchCache>>,
    models: Arc<Models>,
    store: Arc<TagStore>,
    queue: Arc<AnalysisQueue>,
    engine: Tagged,
    engine_bin: PathBuf,
    apply: Applier,
    output: Arc<FrameOutput>,
    runtime: PathBuf,
) -> Control {
    let stop = Arc::new(AtomicBool::new(false));
    let progress = Arc::new(Progress::new());
    let helper = Arc::new(Helper::new(engine_bin, &runtime));
    let health = Arc::new(Health::default());
    let ledger = Arc::new(Ledger::default());
    if let Some(cache) = cache {
        let service = Arc::new(Service {
            cache,
            models,
            store,
            queue,
            engine,
            helper: helper.clone(),
            apply,
            output,
            shared: Shared::new(),
            runtime,
            cursor: AtomicI64::new(0),
            stop: stop.clone(),
            progress: progress.clone(),
            health: health.clone(),
            ledger: ledger.clone(),
            inflight: Mutex::default(),
            idle_since: Mutex::new(None),
        });
        std::thread::spawn(move || service.run());
    }
    Control { stop, progress, helper, health, ledger }
}

/// How wanted a photo is: rated and recent first, ones Auto Cull marked blurry last.
/// Rejected photos never get this far.
pub fn priority(metadata_json: Option<&str>, capture_ns: Option<i64>, blurry: bool) -> Option<(u8, i64, i64)> {
    let meta: Value = metadata_json.and_then(|m| serde_json::from_str(m).ok()).unwrap_or(Value::Null);
    if meta["rejected"].as_bool() == Some(true) {
        return None;
    }
    let rating = meta["rating"].as_i64().unwrap_or(0).clamp(0, 5);
    Some((u8::from(!blurry), rating, capture_ns.unwrap_or(0)))
}

/// Paths a finished Auto Cull marked blurry (and the user has not overridden), read from cull.sqlite.
fn blurry_paths(runtime: &Path) -> HashSet<String> {
    let path = runtime.join("cull.sqlite");
    if !path.is_file() {
        return HashSet::new();
    }
    let Ok(db) = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY) else { return HashSet::new() };
    let Ok(mut q) = db.prepare("SELECT DISTINCT path FROM cull_decisions WHERE reason='blurry' AND COALESCE(final_status,proposed)='reject'") else {
        return HashSet::new();
    };
    q.query_map([], |r| r.get::<_, String>(0)).map(|rows| rows.flatten().collect()).unwrap_or_default()
}

/// Why one photo's job failed, and whether it is the photo's fault.
pub struct Failure {
    kind: Kind,
    msg: String,
}
impl From<String> for Failure {
    fn from(msg: String) -> Self {
        Failure { kind: Kind::Photo, msg }
    }
}
impl From<&str> for Failure {
    fn from(msg: &str) -> Self {
        msg.to_string().into()
    }
}
impl Failure {
    fn model(msg: String) -> Self {
        Failure { kind: Kind::Systemic(Reason::Model), msg }
    }
    fn helper(msg: String) -> Self {
        Failure { kind: tags_health::classify_helper(&msg), msg }
    }
}

/// Records a failed job: a systemic one backs the worker off (and ends the helper, so the next
/// try starts a clean one after the wait); a photo's own one leaves that photo alone. Each is
/// logged once, not on every retry.
#[allow(clippy::too_many_arguments)]
fn record_failure(health: &Health, ledger: &Ledger, end_helper: &dyn Fn(), path: &str, key: &str, version: &str, generation: u64, f: Failure, now: u64) {
    match f.kind {
        Kind::Transient => {}
        Kind::Systemic(reason) => {
            if let Some(line) = health.systemic(generation, reason, &f.msg, now) {
                eprintln!("{line}");
            }
            end_helper();
        }
        Kind::Photo => {
            let tries = ledger.fail(key, version, now);
            if tries == 1 || tries >= tags_health::MAX_PHOTO_TRIES {
                eprintln!("auto-keywords {path} (try {tries}{}): {}", if tries >= tags_health::MAX_PHOTO_TRIES { ", given up" } else { "" }, f.msg);
            }
        }
    }
}

/// Whether a photo may be handed to the worker now: not one left alone after its own failures,
/// and (for an analysis, which needs the model) not while the worker backs off or is blocked.
fn admit(health: &Health, ledger: &Ledger, key: &str, version: &str, needs_model: bool, now: u64) -> bool {
    !ledger.skipped(key, version, now) && (!needs_model || matches!(health.gate(now), Gate::Go))
}

impl Service {
    /// Records a failed job (see `record_failure`).
    fn failed(&self, path: &str, key: &str, version: &str, generation: u64, f: Failure) {
        record_failure(&self.health, &self.ledger, &|| self.helper.end(), path, key, version, generation, f, now_ms());
    }

    fn run(self: Arc<Self>) {
        let mut last_scan: Option<Instant> = None;
        let mut last_count: Option<Instant> = None;
        while !self.stop.load(Ordering::Acquire) {
            let p = &self.progress;
            // nothing is read, hashed or requested while the model is missing or the switch is off
            let installed = self.models.installed();
            let enabled = self.models.enabled();
            p.installed.store(installed, Ordering::Release);
            p.enabled.store(enabled, Ordering::Release);
            if installed && enabled {
                // no scan while the worker is backing off or blocked: nothing could be analysed
                if matches!(self.health.gate(now_ms()), Gate::Go) && last_scan.is_none_or(|t| t.elapsed() >= SCAN_EVERY) {
                    last_scan = Some(Instant::now());
                    let next = self.clone();
                    self.queue.submit("tags:scan".into(), move |job| {
                        if !job.cancelled() && !next.stop.load(Ordering::Acquire) {
                            if let Err(e) = next.scan(&job) {
                                eprintln!("auto-keywords: {e}");
                            }
                        }
                    });
                }
                if p.count_requested() && last_count.is_none_or(|t| t.elapsed() >= COUNT_EVERY) {
                    last_count = Some(Instant::now());
                    if let Some(n) = self.count_backlog() {
                        p.pending.store(n as i64, Ordering::Release);
                    }
                }
            } else {
                p.pending.store(-1, Ordering::Release);
                p.blocked.store(BLOCK_NONE, Ordering::Release);
                last_scan = None;
                last_count = None;
            }
            self.maybe_end_helper(installed && enabled);
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    // ends the helper process once there has been nothing to analyse for a while, or the switch is off
    fn maybe_end_helper(&self, active: bool) {
        if !self.helper.running() {
            return;
        }
        let busy = !self.inflight.lock().unwrap().is_empty();
        let mut idle = self.idle_since.lock().unwrap();
        if busy {
            *idle = None;
            return;
        }
        let since = *idle.get_or_insert_with(Instant::now);
        if !active || since.elapsed() >= UNLOAD_AFTER {
            *idle = None;
            drop(idle);
            self.helper.end();
        }
    }

    fn deferred(&self, path: &str) -> bool {
        self.engine.quiet_for() < SHARE_QUIET && crate::volumes::on_network(Path::new(path))
    }

    /// Photos of the selected folders with no result for this model, counted a page at a time.
    fn count_backlog(&self) -> Option<u32> {
        let selection = library_selection(&self.runtime);
        let now = now_ms();
        let (mut after, mut total) = (0i64, 0u32);
        loop {
            if self.stop.load(Ordering::Acquire) {
                return None;
            }
            let rows = self.cache.assets_after(after, PAGE).ok()?;
            let end = (rows.len() as u32) < PAGE;
            after = rows.last().map_or(after, |r| r.id);
            for row in rows {
                if selected(Path::new(&row.path), &selection)
                    && !self.store.is_current(&row.path, &row.original_fingerprint, ram_models::MODEL_VERSION).ok()?
                    && !self.ledger.skipped(&row.path, &row.original_fingerprint, now)
                {
                    total = total.saturating_add(1);
                }
            }
            if end {
                return Some(total);
            }
        }
    }

    fn offer(self: &Arc<Self>, path: String, fingerprint: Option<String>) -> bool {
        let key = if fingerprint.is_some() { path.clone() } else { format!("reapply:{path}") };
        let version = fingerprint.clone().unwrap_or_default();
        if !admit(&self.health, &self.ledger, &key, &version, fingerprint.is_some(), now_ms()) {
            // not an end of the walk: the next photos are still offered, so one bad photo starves none
            return true;
        }
        {
            let mut inflight = self.inflight.lock().unwrap();
            if inflight.len() >= AHEAD || !inflight.insert(path.clone()) {
                return false;
            }
        }
        let next = self.clone();
        let path_copy = path.clone();
        let job_key = format!("tags:photo:{path}");
        let queued = self.queue.submit(job_key, move |job| {
            let generation = next.health.generation();
            let result = match &fingerprint {
                Some(fp) => next.analyze(&path, fp, &job),
                None => next.reapply(&path, &job),
            };
            match result {
                Ok(()) => {}
                Err(f) => next.failed(&path, &key, &version, generation, f),
            }
            next.inflight.lock().unwrap().remove(&path);
        });
        if !queued {
            self.inflight.lock().unwrap().remove(&path_copy);
        }
        queued
    }

    fn scan(self: &Arc<Self>, job: &Job) -> Result<(), String> {
        let selection = library_selection(&self.runtime);
        let quiet = self.engine.quiet_for();
        let pct = self.models.threshold();
        // photos whose keywords were decided at another confidence are re-decided from the
        // stored tags first: no inference, so a change of the setting takes effect at once
        for path in self.store.needing_reapply(pct, 8)? {
            if job.cancelled() || !self.offer(path, None) {
                return Ok(());
            }
        }
        let blurry = blurry_paths(&self.runtime);
        let start = self.cursor.load(Ordering::Acquire);
        let (mut after, mut lead, mut leading) = (start, start, true);
        let mut wanted: Vec<((u8, i64, i64), AssetMeta)> = Vec::new();
        let mut end = false;
        while wanted.len() < WINDOW && !end {
            if job.cancelled() || self.stop.load(Ordering::Acquire) {
                return Ok(());
            }
            let rows = self.cache.assets_after_meta(after, PAGE)?;
            end = (rows.len() as u32) < PAGE;
            for m in rows {
                after = m.asset.id;
                let a = &m.asset;
                let path = Path::new(&a.path);
                let current = !selected(path, &selection) || self.store.is_current(&a.path, &a.original_fingerprint, ram_models::MODEL_VERSION)?;
                if current {
                    if leading {
                        lead = a.id;
                    }
                    continue;
                }
                leading = false;
                if quiet < SHARE_QUIET && crate::volumes::on_network(path) {
                    continue;
                }
                if let Some(p) = priority(m.metadata_json.as_deref(), m.capture_time_ns, blurry.contains(&a.path)) {
                    wanted.push((p, m));
                }
            }
        }
        if wanted.is_empty() {
            self.cursor.store(if end { 0 } else { lead }, Ordering::Release);
            return Ok(());
        }
        wanted.sort_by(|a, b| b.0.cmp(&a.0));
        for (_, m) in wanted {
            if job.cancelled() || !self.offer(m.asset.path, Some(m.asset.original_fingerprint)) {
                break;
            }
        }
        self.cursor.store(lead, Ordering::Release);
        Ok(())
    }

    fn fingerprint_now(&self, path: &str) -> Option<String> {
        let p = Path::new(path);
        if !crate::volumes::answering(p) {
            return None;
        }
        crate::cache::listed_metadata_fingerprints(&Mutex::new(crate::cache::Listed::default()), p, 0).map(|(original, _)| original)
    }

    /// The photo's small picture: from its own file when it has a usable one, else rendered by
    /// the main engine (and kept for the next analysis of this photo).
    fn picture(&self, path: &str, fingerprint: &str, imgid: &dyn Fn() -> i64) -> Result<Arc<image::RgbImage>, String> {
        if let Some((img, _)) = self.shared.get(path, fingerprint) {
            self.progress.from_file.fetch_add(1, Ordering::AcqRel);
            return Ok(img);
        }
        let tag = self.engine.tag();
        let out = self.output.path(&format!("tags-{tag}.jpg"));
        let preview = (|| {
            self.engine.request(tag, json!({"cmd":"analysis_preview","imgid":imgid(),"subject":"original","out":out}))?;
            FrameOutput::read(&out).map_err(|e| e.to_string())
        })();
        self.output.remove(&out);
        let mut reader = image::ImageReader::new(std::io::Cursor::new(preview?));
        reader.set_format(image::ImageFormat::Jpeg);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(1024);
        limits.max_image_height = Some(1024);
        limits.max_alloc = Some(16 * 1024 * 1024);
        reader.limits(limits);
        let img = Arc::new(reader.decode().map_err(|e| e.to_string())?.to_rgb8());
        self.shared.put(path, fingerprint, img.clone());
        self.progress.rendered.fetch_add(1, Ordering::AcqRel);
        Ok(img)
    }

    /// The analysed photo of the same folder this one looks like (a burst), if any.
    fn burst_mate(&self, path: &str, hash: &str) -> Option<String> {
        let mine = crate::cull::hash_from_string(hash)?;
        let limit = crate::cull_plan::CullSettings::default().similarity_threshold;
        let dir = Path::new(path).parent()?.to_str()?;
        let mut best: Option<(u32, String)> = None;
        for (p, h) in self.store.representatives(dir, ram_models::MODEL_VERSION).ok()? {
            if p == path {
                continue;
            }
            if let Some(d) = crate::cull::hash_from_string(&h).map(|o| mine.dist(&o)).filter(|d| *d <= limit) {
                if best.as_ref().is_none_or(|(n, _)| d < *n) {
                    best = Some((d, p));
                }
            }
        }
        best.map(|(_, p)| p)
    }

    fn analyze(&self, path: &str, fingerprint: &str, job: &Job) -> Result<(), Failure> {
        if job.cancelled() || self.stop.load(Ordering::Acquire) || !self.models.enabled() {
            return Ok(());
        }
        if !selected(Path::new(path), &library_selection(&self.runtime)) || self.deferred(path) {
            return Ok(());
        }
        if self.fingerprint_now(path).as_deref() != Some(fingerprint) {
            return Ok(());
        }
        if !matches!(self.health.gate(now_ms()), Gate::Go) {
            return Ok(());
        }
        let (root, vocabulary) = self.models.package().map_err(Failure::model)?;
        let running = RunGuard::new(&self.progress);
        let imgid = std::cell::Cell::new(0i64);
        let imgid_of = || {
            if imgid.get() == 0 {
                imgid.set(self.engine.cached_photo(path, 0));
            }
            imgid.get()
        };
        let rgb = self.picture(path, fingerprint, &imgid_of)?;
        if job.cancelled() {
            return Ok(());
        }
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        // the near-duplicate hash and quality of this picture: found a burst mate, or kept for later ones
        let analysed = crate::cull::analyze(&rgb);
        let hash = crate::cull::hash_to_string(&analysed.hash);
        if self.models.burst() {
            if let Some(mate) = self.burst_mate(path, &hash) {
                let tags: Vec<(String, f32)> =
                    self.store.tags(&mate)?.into_iter().filter(|t| t.confidence > 0.0).map(|t| (t.name, t.confidence)).collect();
                let origin = Origin { hash: Some(&hash), quality: Some(analysed.result.quality_score), propagated_from: Some(&mate) };
                if let Some(true) = job.commit(|| self.store.put_with(path, fingerprint, ram_models::MODEL_VERSION, &tags, secs, &origin)).transpose()? {
                    self.progress.copied.fetch_add(1, Ordering::AcqRel);
                    running.stored();
                    self.ledger.clear(path);
                    return Ok(self.decide(path, imgid_of())?);
                }
                return Ok(());
            }
        }
        // the model's own process, if the memory allows it
        if !self.helper.running() {
            let block = memory_now(&self.runtime);
            self.progress.blocked.store(block, Ordering::Release);
            if block != BLOCK_NONE {
                return Ok(());
            }
        }
        self.progress.blocked.store(BLOCK_NONE, Ordering::Release);
        let helper = self.helper.tagged().map_err(Failure::helper)?;
        let tensor = ram_models::preprocess(rgb.as_raw(), rgb.width(), rgb.height())?;
        let logits = infer(&helper, &self.runtime, &root, &tensor, &self.progress).map_err(Failure::helper)?;
        if let Some(line) = self.health.ok() {
            eprintln!("{line}");
        }
        let found = vocabulary.select(&logits).map_err(Failure::model)?;
        if job.cancelled() || self.fingerprint_now(path).as_deref() != Some(fingerprint) {
            return Ok(());
        }
        let stored: Vec<(String, f32)> = found.iter().map(|t| (t.name.clone(), t.confidence)).collect();
        let origin = Origin { hash: Some(&hash), quality: Some(analysed.result.quality_score), propagated_from: None };
        match job.commit(|| self.store.put_with(path, fingerprint, ram_models::MODEL_VERSION, &stored, secs, &origin)) {
            Some(r) => r.map(|_| ())?,
            None => return Ok(()),
        }
        running.stored();
        self.ledger.clear(path);
        self.progress.last_rss_kb.store(self.helper.rss_kb(), Ordering::Release);
        Ok(self.decide(path, imgid_of())?)
    }

    fn reapply(&self, path: &str, job: &Job) -> Result<(), Failure> {
        if job.cancelled() || !self.models.enabled() {
            return Ok(());
        }
        let imgid = self.engine.cached_photo(path, 0);
        Ok(self.decide(path, imgid)?)
    }

    /// Attaches the keywords at or above the confidence setting and takes off the ones
    /// Immage attached that are now below it. Keywords the user had are never touched.
    fn decide(&self, path: &str, imgid: i64) -> Result<(), String> {
        let pct = self.models.threshold();
        let stored = self.store.tags(path)?;
        let wanted = |c: f32| c * 100.0 >= f32::from(pct);
        let add: Vec<&str> = stored.iter().filter(|t| wanted(t.confidence) && !t.applied && !t.dismissed && !t.kept).map(|t| t.name.as_str()).collect();
        let remove: Vec<&str> = stored.iter().filter(|t| !wanted(t.confidence) && t.applied).map(|t| t.name.as_str()).collect();
        if add.is_empty() && remove.is_empty() {
            return self.store.record_applied(path, pct, &[], &[]);
        }
        let reply = (self.apply)(json!({"cmd":"tags_apply","imgid":imgid,"add":add,"remove":remove}))?;
        let names = |key: &str| -> Vec<String> {
            reply[key].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(str::to_string)).collect()
        };
        self.store.record_applied(path, pct, &names("added"), &names("removed"))
    }
}

/// One RAM++ inference in the helper engine's CPU worker; returns the logits.
fn infer(engine: &Tagged, runtime: &Path, root: &Path, tensor: &[f32], progress: &Progress) -> Result<Vec<f32>, String> {
    if tensor.len() != 3 * 384 * 384 || tensor.iter().any(|n| !n.is_finite()) {
        return Err("Invalid RAM++ tensor".into());
    }
    let dir = runtime.join("tags-inputs");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(format!("{}.bin", uuid::Uuid::new_v4()));
    let mut options = std::fs::OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(|e| e.to_string())?;
    let bytes: Vec<u8> = tensor.iter().flat_map(|v| v.to_le_bytes()).collect();
    let written = file.write_all(&bytes).and_then(|()| file.flush());
    drop(file);
    let reply = written
        .map_err(|e| e.to_string())
        .and_then(|()| engine.request(engine.tag(), json!({"cmd":"tags_analyze","model_root":root,"input":path})));
    let _ = std::fs::remove_file(&path);
    let reply = reply?;
    if reply["ok"].as_bool() != Some(true) {
        return Err(reply["error"].as_str().unwrap_or("RAM++ inference failed").into());
    }
    if let Some(ms) = reply["ms"].as_f64() {
        progress.last_infer_ms.store(ms.round() as u64, Ordering::Release);
    }
    reply["logits"]
        .as_array()
        .filter(|v| v.len() == ram_models::CLASSES)
        .ok_or("RAM++ returned the wrong number of logits")?
        .iter()
        .map(|v| v.as_f64().filter(|n| n.is_finite()).map(|n| n as f32).ok_or_else(|| "RAM++ returned an invalid logit".to_string()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn the_backlog_is_ordered_by_stars_then_recency_with_blurry_last_and_rejected_never() {
        let meta = |rating: i64, rejected: bool| Some(format!("{{\"rating\":{rating},\"rejected\":{rejected}}}"));
        let p = |m: Option<String>, t: Option<i64>, blurry| priority(m.as_deref(), t, blurry);
        assert_eq!(p(meta(0, true), Some(5), false), None, "rejected photos are never analysed");
        let five_old = p(meta(5, false), Some(1), false).unwrap();
        let five_new = p(meta(5, false), Some(9), false).unwrap();
        let unrated_new = p(None, Some(99), false).unwrap();
        let blurry_five = p(meta(5, false), Some(99), true).unwrap();
        let mut order = vec![("unrated_new", unrated_new), ("blurry_five", blurry_five), ("five_old", five_old), ("five_new", five_new)];
        order.sort_by(|a, b| b.1.cmp(&a.1));
        assert_eq!(order.iter().map(|o| o.0).collect::<Vec<_>>(), ["five_new", "five_old", "unrated_new", "blurry_five"]);
        // a record that is missing or garbled counts as unrated, not as an error
        assert_eq!(p(Some("not json".into()), None, false), Some((1, 0, 0)));
    }

    #[test]
    fn blurry_photos_are_read_from_the_cull_database_unless_the_user_overrode_them() {
        let dir = tempfile::tempdir().unwrap();
        assert!(blurry_paths(dir.path()).is_empty(), "no cull database, no blurry photos");
        let db = rusqlite::Connection::open(dir.path().join("cull.sqlite")).unwrap();
        db.execute_batch(
            "CREATE TABLE cull_decisions(path TEXT, reason TEXT, proposed TEXT, final_status TEXT);
             INSERT INTO cull_decisions VALUES('/a.jpg','blurry','reject',NULL),('/b.jpg','blurry','reject','keep'),
               ('/c.jpg','unique','keep',NULL),('/d.jpg','duplicateOf','reject',NULL);",
        )
        .unwrap();
        drop(db);
        let got = blurry_paths(dir.path());
        assert!(got.contains("/a.jpg") && got.len() == 1, "{got:?}");
    }

    #[test]
    fn the_model_is_not_started_when_the_memory_level_or_free_memory_cannot_hold_it() {
        // 16 GB, default level: a budget of 8 GB, 10 GB free
        assert_eq!(memory_gate(16384, 10_000, "default"), BLOCK_NONE);
        // the small level leaves 2 GB of a 16 GB machine: refused by the level, whatever is free
        assert_eq!(memory_gate(16384, 12_000, "small"), BLOCK_LEVEL);
        // the level allows it but the machine is busy
        assert_eq!(memory_gate(16384, 2_000, "default"), BLOCK_FREE);
        // a 4 GB machine cannot hold it at any level
        assert_eq!(memory_gate(4096, 3_000, "large"), BLOCK_LEVEL);
        assert_eq!(rss_of(0), 0);
    }

    /// A worker run on a fake clock and a fake helper, through the same `admit` and `record_failure`
    /// the service uses: `fake(photo, helper_spawns)` is what the engine answers. (The service itself
    /// needs the 1.8 GB model's checksummed files, so its own run is the ignored real test.)
    struct Sim {
        health: Health,
        ledger: Ledger,
        now: u64,
        /// helper processes started: one per analysis while none runs
        spawns: u32,
        running: std::cell::Cell<bool>,
        attempts: std::collections::HashMap<String, u32>,
        done: Vec<String>,
    }
    impl Sim {
        fn new() -> Self {
            Sim { health: Health::default(), ledger: Ledger::default(), now: 1_000, spawns: 0, running: false.into(), attempts: Default::default(), done: vec![] }
        }
        /// One scan: offers every photo not yet done, two at a time like the service
        fn pass(&mut self, photos: &[&str], fake: &dyn Fn(&str) -> Result<(), String>) {
            let mut offered = 0;
            for p in photos {
                if self.done.iter().any(|d| d == p) || !admit(&self.health, &self.ledger, p, "v1", true, self.now) {
                    continue;
                }
                if offered == AHEAD {
                    break;
                }
                offered += 1;
                let generation = self.health.generation();
                if !self.running.get() {
                    self.spawns += 1;
                    self.running.set(true);
                }
                *self.attempts.entry(p.to_string()).or_default() += 1;
                match fake(p) {
                    Ok(()) => {
                        self.health.ok();
                        self.ledger.clear(p);
                        self.done.push(p.to_string());
                    }
                    Err(msg) => {
                        let running = &self.running;
                        record_failure(&self.health, &self.ledger, &|| running.set(false), p, p, "v1", generation, Failure::helper(msg), self.now);
                    }
                }
            }
        }
        /// The worker's loop for `secs` seconds, one scan every 4 s
        fn run(&mut self, secs: u64, photos: &[&str], fake: &dyn Fn(&str) -> Result<(), String>) {
            for _ in 0..secs / 4 {
                if matches!(self.health.gate(self.now), Gate::Go) {
                    self.pass(photos, fake);
                }
                self.now += 4_000;
            }
        }
    }

    #[test]
    fn a_missing_runtime_backs_off_then_blocks_without_respawning_in_a_loop() {
        let mut sim = Sim::new();
        let photos = ["/a.jpg", "/b.jpg", "/c.jpg"];
        let missing = |_: &str| Err("RAM++ model is unavailable; verify the Auto-keywords model".to_string());
        sim.run(3 * 3600, &photos, &missing);
        // 4 systemic failures (the waits are 5 s, 30 s and 5 min), then nothing for the next hours
        assert_eq!(sim.attempts.values().sum::<u32>(), tags_health::MAX_SYSTEMIC);
        assert_eq!(sim.spawns, tags_health::MAX_SYSTEMIC, "one fresh helper per try, none in a loop");
        let r = sim.health.report(sim.now);
        assert_eq!((r.state, r.reason), ("blocked", Some("model")));
        assert!(r.detail.contains("unavailable"));
        // the user fixes the runtime and presses Retry: everything is analysed
        sim.health.retry();
        sim.ledger.reset();
        sim.run(60, &photos, &|_| Ok(()));
        assert_eq!(sim.done.len(), 3);
        assert_eq!(sim.health.report(sim.now).state, "ok");
    }

    #[test]
    fn an_unreadable_photo_is_tried_a_few_times_and_never_holds_up_the_others() {
        let mut sim = Sim::new();
        let photos = ["/bad.jpg", "/b.jpg", "/c.jpg", "/d.jpg", "/e.jpg"];
        let fake = |p: &str| if p == "/bad.jpg" { Err("Invalid RAM++ tensor values".to_string()) } else { Ok(()) };
        sim.run(6 * 3600, &photos, &fake);
        assert_eq!(sim.done.len(), 4, "every good photo was analysed: {:?}", sim.done);
        assert_eq!(sim.attempts["/bad.jpg"], tags_health::MAX_PHOTO_TRIES, "tried a few times, then left alone");
        assert_eq!(sim.ledger.count(sim.now), 1, "the card can say one photo failed");
        assert_eq!(sim.health.report(sim.now).state, "ok", "a photo's own failure is not the worker's");
        assert_eq!(sim.spawns, 1, "the helper was not restarted for it");
    }

    #[test]
    fn a_transient_busy_reply_counts_as_nothing() {
        let (h, l) = (Health::default(), Ledger::default());
        record_failure(&h, &l, &|| {}, "/a.jpg", "/a.jpg", "v1", h.generation(), Failure::helper("RAM++ worker is full or stopping".into()), 0);
        assert_eq!(h.report(0).state, "ok");
        assert_eq!(l.count(0), 0);
    }

    fn wait(what: &str, limit: Duration, mut done: impl FnMut() -> bool) {
        let start = Instant::now();
        while !done() {
            assert!(start.elapsed() < limit, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    /// The whole feature on the real engine and the real model, on a disposable profile and the
    /// photos in IMMAGE_TEST_PHOTOS only: nothing runs while the switch is off; turning it on
    /// analyses every photo, attaches keywords (also in the XMP sidecar), the confidence setting
    /// re-decides stored results without inference, and the bulk removal takes off only what the
    /// analysis added while a keyword of the user's own stays.
    /// IMMAGE_RAM_BASE=<runtime>/ram-models IMMAGE_TEST_ENGINE=<build>/bin/darktable-engine
    /// IMMAGE_TEST_ORT=<libonnxruntime.so> IMMAGE_TEST_PHOTOS=<folder of jpegs>
    ///   cargo test opt_in_real_keywords_service -- --ignored --nocapture
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs the installed RAM++ pack, the built engine and ONNX Runtime"]
    fn opt_in_real_keywords_service() {
        let var = |k: &str| PathBuf::from(std::env::var_os(k).unwrap_or_else(|| panic!("set {k}")));
        let (base, engine_path, ort, photos) = (var("IMMAGE_RAM_BASE"), var("IMMAGE_TEST_ENGINE"), var("IMMAGE_TEST_ORT"), var("IMMAGE_TEST_PHOTOS"));
        let temp = tempfile::tempdir().unwrap();
        let runtime = temp.path().join("runtime");
        let library = temp.path().join("library");
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::create_dir_all(&library).unwrap();
        let mut paths = Vec::new();
        for entry in std::fs::read_dir(&photos).unwrap().flatten() {
            if entry.path().extension().is_some_and(|e| e == "jpg") {
                let to = library.join(entry.file_name());
                std::fs::copy(entry.path(), &to).unwrap();
                paths.push(to);
            }
        }
        paths.sort();
        assert!(!paths.is_empty());
        let settings = crate::settings::Settings::load(&runtime);
        settings.set("immage.library", &json!({"folders": [library]}).to_string()).unwrap();
        let cache = Arc::new(SearchCache::open(&runtime.join("search-cache.sqlite")).unwrap());
        for p in &paths {
            let (original, edited) = crate::cache::listed_metadata_fingerprints(&Mutex::new(crate::cache::Listed::default()), p, 0).unwrap();
            cache.upsert_variant(p.to_str().unwrap(), "", &original, &edited, None, None, None).unwrap();
        }
        // the user's own keyword on the first photo, which no removal may take
        use std::os::unix::fs::PermissionsExt;
        let wrapper = runtime.join("engine-with-test-ort");
        std::fs::write(&wrapper, format!("#!/bin/sh\nexec '{}' \"$@\" --conf 'plugins/ai/ort_library_path={}'\n", engine_path.display(), ort.display())).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let engine = Arc::new(Mutex::new(crate::engine::Engine::spawn(&wrapper, &runtime, &runtime).expect("start the isolated engine")));
        let tagged = engine.lock().unwrap().tagged();
        let apply: Applier = {
            let engine = engine.clone();
            Arc::new(move |req| engine.lock().unwrap().request(req))
        };
        let models = Arc::new(Models::new(base));
        assert!(models.installed(), "the pack is not installed under IMMAGE_RAM_BASE");
        let store = Arc::new(TagStore::open(&temp.path().join("tags.sqlite")).unwrap());
        let queue = Arc::new(AnalysisQueue::new(2, 256));
        let control = start(Some(cache), models.clone(), store.clone(), queue, tagged.clone(), wrapper.clone(), apply.clone(), Arc::new(FrameOutput::new(&runtime)), runtime.clone());

        let main_pid = engine.lock().unwrap().pid().unwrap();
        let main_before = rss_of(main_pid) / 1024;
        // 1. switch off: nothing is read, requested or stored
        std::thread::sleep(Duration::from_secs(6));
        assert_eq!(control.helper_pid(), 0, "the model's process started with the switch off");
        assert_eq!(store.totals().unwrap().photos, 0, "analysis ran with the switch off");
        assert!(!runtime.join("tags-inputs").exists(), "a tensor was written with the switch off");
        assert!(!control.status(false).enabled);

        // 2. switch on
        let started = Instant::now();
        models.enabled_flag().store(true, Ordering::Release);
        wait("every photo analysed", Duration::from_secs(600), || store.totals().unwrap().photos as usize == paths.len());
        let total = started.elapsed();
        wait("keywords decided", Duration::from_secs(60), || paths.iter().all(|p| store.applied_pct(p.to_str().unwrap()).unwrap() == Some(70)));
        let at_70 = store.totals().unwrap();
        assert!(at_70.applied > 0, "no keyword was added at 70%");
        let mut xmp_checked = 0;
        for p in &paths {
            let applied: Vec<_> = store.tags(p.to_str().unwrap()).unwrap().into_iter().filter(|t| t.applied).collect();
            assert!(applied.iter().all(|t| t.confidence >= 0.7));
            let xmp = std::fs::read_to_string(format!("{}.xmp", p.display())).unwrap_or_default();
            for t in &applied {
                assert!(xmp.contains(&t.name), "{} is not in the XMP of {}", t.name, p.display());
                xmp_checked += 1;
            }
        }
        assert!(xmp_checked > 0);
        let (infer_ms, rss_kb) = control.speed();
        let during = rss_of(control.helper_pid()) / 1024;
        let (from_file, rendered, copied) = control.sources();
        println!("sources: {from_file} from the file, {rendered} rendered, {copied} copied from a burst mate; helper RSS during {during} MB, main engine {main_before} MB before / {} MB now", rss_of(main_pid) / 1024);
        // the helper ends itself when the backlog is empty: the model's memory goes back to the system
        let helper_pid = control.helper_pid();
        assert_ne!(helper_pid, 0);
        wait("the helper process to end", Duration::from_secs(60), || control.helper_pid() == 0);
        wait("its process to exit", Duration::from_secs(20), || !std::path::Path::new(&format!("/proc/{helper_pid}")).exists());
        println!("helper RSS after: process gone (was {during} MB); main engine {} MB", rss_of(main_pid) / 1024);
        println!("{} photos in {:.1} s ({:.1} s each incl. preview and load); last inference {:?} ms; engine memory {:?} MB; {} keywords on {} photos at 70%",
                 paths.len(), total.as_secs_f64(), total.as_secs_f64() / paths.len() as f64, infer_ms, rss_kb.map(|k| k / 1024), at_70.applied, at_70.keyworded);

        // 3. a higher confidence takes keywords off without new inference; a lower one adds them
        models.threshold_flag().store(95, Ordering::Release);
        wait("re-decided at 95%", Duration::from_secs(60), || paths.iter().all(|p| store.applied_pct(p.to_str().unwrap()).unwrap() == Some(95)));
        let at_95 = store.totals().unwrap();
        assert!(at_95.applied <= at_70.applied);
        models.threshold_flag().store(40, Ordering::Release);
        wait("re-decided at 40%", Duration::from_secs(60), || paths.iter().all(|p| store.applied_pct(p.to_str().unwrap()).unwrap() == Some(40)));
        let at_40 = store.totals().unwrap();
        assert!(at_40.applied >= at_70.applied, "{} at 40% vs {} at 70%", at_40.applied, at_70.applied);
        println!("keywords: {} at 95%, {} at 70%, {} at 40%", at_95.applied, at_70.applied, at_40.applied);

        // 4. the user's own keyword survives the bulk removal; the AI ones go, also from the XMP
        let first = paths[0].to_str().unwrap();
        let imgid = tagged.cached_photo(first, 0);
        let mine = apply(json!({"cmd":"tags_apply","imgid":imgid,"add":["my own keyword"]})).unwrap();
        assert_eq!(mine["added"], json!(["my own keyword"]));
        models.enabled_flag().store(false, Ordering::Release);
        std::thread::sleep(Duration::from_secs(2));
        let report = crate::tags_cmds::remove_applied(&store, &tagged, &apply, &[]).unwrap();
        assert_eq!(report.failed, 0);
        assert_eq!(report.keywords, at_40.applied);
        assert_eq!(store.totals().unwrap().applied, 0);
        for p in &paths {
            let xmp = std::fs::read_to_string(format!("{}.xmp", p.display())).unwrap_or_default();
            for t in store.tags(p.to_str().unwrap()).unwrap() {
                assert!(!xmp.contains(&format!(">{}<", t.name)), "{} stayed in the XMP", t.name);
            }
        }
        let xmp = std::fs::read_to_string(format!("{first}.xmp")).unwrap();
        assert!(xmp.contains("my own keyword"), "the user's keyword was taken off");
        // a removed keyword is not added again when the confidence changes
        models.enabled_flag().store(true, Ordering::Release);
        models.threshold_flag().store(60, Ordering::Release);
        wait("re-decided at 60%", Duration::from_secs(60), || paths.iter().all(|p| store.applied_pct(p.to_str().unwrap()).unwrap() == Some(60)));
        assert_eq!(store.totals().unwrap().applied, 0, "a removed AI keyword came back");
        control.stop();
        println!("PASS");
    }
}
