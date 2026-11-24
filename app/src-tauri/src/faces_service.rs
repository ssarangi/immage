// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! Faces in the background: finds the Catalog's photos with no stored result for the current model,
//! detects and embeds their faces (YuNet and SFace in the engine's face worker, engine/faces.c),
//! stores them (faces_store.rs) and, once the backlog is empty, suggests names and groups the
//! unnamed faces (faces_cluster.rs). It runs only while the pack is installed AND the Faces switch
//! is on (Preferences > AI models; off by default, since embeddings are biometric), on the shared
//! analysis queue so the one Pause and the user's own work take precedence, never more than two
//! photos ahead of the worker. A folder can also be scanned once on request (`scan_folder`).
//!
//! Resumable by construction: a photo's `photos` row is its scan state (current file and model
//! means done), so a restart picks up where it stopped. A photo that cannot be read is retried a few
//! times and then left alone until the file changes (tags_health.rs's Ledger); a systemic failure
//! (a model that will not load) backs the worker off, as Auto-keywords does.
use crate::{
    analysis_image::{self, Shared},
    analysis_queue::{AnalysisQueue, Job},
    engine::Tagged,
    faces_cluster::{self, Params},
    faces_detect as det,
    faces_models::{self as fm, Model, Models},
    faces_store::{FaceStore, NewFace, Unnamed},
    volumes::MountSnapshot,
    frame_output::FrameOutput,
    search_analysis::{library_selection, now_ms, selected, AnalysisProgress, SHARE_QUIET},
    search_cache::SearchCache,
    tags_health::{Gate, Health, Kind, Ledger, Reason, MAX_PHOTO_TRIES},
};
use serde_json::json;
use std::{
    collections::{HashSet, VecDeque},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};

const SCAN_EVERY: Duration = Duration::from_secs(4);
const COUNT_EVERY: Duration = Duration::from_secs(5);
const WANTED_FOR_MS: u64 = 10_000;
const ACTIVE_FOR_MS: u64 = 15_000;
const PAGE: u32 = 256;
const AHEAD: usize = 2;
const UNLOAD_AFTER: Duration = Duration::from_secs(20);
const FOLDER_LIMIT: usize = 100_000;

/// What the page is told, as events `faces-progress` and `faces-changed` (main.rs sends them).
#[derive(Clone, Debug, serde::Serialize, specta::Type)]
pub struct FacesProgress {
    pub scanned: u64,
    pub pending: Option<u32>,
    pub running: u32,
    pub faces_found: u64,
    pub folder_pending: u32,
}
#[derive(Clone, Debug)]
pub enum FacesEvent {
    Progress(FacesProgress),
    /// Faces, suggestions or groups changed: the lists are stale.
    Changed,
}
pub type Emit = Arc<dyn Fn(FacesEvent) + Send + Sync>;

pub struct Progress {
    installed: AtomicBool,
    enabled: AtomicBool,
    pending: AtomicI64,
    running: AtomicUsize,
    last_work_ms: AtomicU64,
    wanted_ms: AtomicU64,
    last_ms: AtomicU64,
    scanned: AtomicU64,
    faces_found: AtomicU64,
    from_file: AtomicU64,
    rendered: AtomicU64,
    mirrored: AtomicU64,
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
            last_ms: AtomicU64::new(0),
            scanned: AtomicU64::new(0),
            faces_found: AtomicU64::new(0),
            from_file: AtomicU64::new(0),
            rendered: AtomicU64::new(0),
            mirrored: AtomicU64::new(0),
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
    health: Arc<Health>,
    ledger: Arc<Ledger>,
    dirty: Arc<AtomicBool>,
    /// A review decision asked for the analysis: it does not wait for a scan in progress.
    urgent: Arc<AtomicBool>,
    folders: Arc<Mutex<VecDeque<Requested>>>,
    store: Arc<FaceStore>,
    wake: Arc<AtomicBool>,
}
/// A folder asked for once: (folder, with subfolders, a network folder is allowed).
pub type Requested = (PathBuf, bool, bool);
const FOLDERS_KEY: &str = "folders";
/// Settings key (a JSON list of folders): the network sources the user allowed Faces to scan. Default none.
pub const NETWORK_SOURCES_KEY: &str = "faces.network_sources";

/// Which paths the scan may touch: a path on a network share is off limits unless it is under a source the
/// user allowed for Faces, or under a folder they asked for explicitly with the network allowed. Judged from the
/// mount table alone: nothing on a share is read, stat'ed or listed to decide.
pub struct NetPolicy {
    snapshot: MountSnapshot,
    allowed: Vec<PathBuf>,
}
impl NetPolicy {
    pub fn new(snapshot: MountSnapshot, runtime: &Path, requested: &[Requested]) -> Self {
        let mut allowed: Vec<PathBuf> = allowed_sources(runtime);
        allowed.extend(requested.iter().filter(|r| r.2).map(|r| r.0.clone()));
        NetPolicy { snapshot, allowed }
    }
    pub fn take(runtime: &Path, requested: &[Requested]) -> Self {
        Self::new(MountSnapshot::take(), runtime, requested)
    }
    pub fn blocked(&self, path: &Path) -> bool {
        self.snapshot.is_network(path) && !self.allowed.iter().any(|r| path.starts_with(r))
    }
}
pub fn allowed_sources(runtime: &Path) -> Vec<PathBuf> {
    crate::settings::read_key(runtime, NETWORK_SOURCES_KEY)
        .and_then(|t| serde_json::from_str::<Vec<String>>(&t).ok())
        .unwrap_or_default()
        .into_iter()
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .collect()
}
/// How many of the library's folders are on a network share the user has not allowed (not scanned).
pub fn skipped_network_sources(runtime: &Path, snapshot: MountSnapshot) -> u32 {
    let policy = NetPolicy::new(snapshot, runtime, &[]);
    library_selection(runtime)["folders"].as_array().into_iter().flatten().filter_map(|v| v.as_str()).filter(|f| policy.blocked(Path::new(f))).count() as u32
}

fn persist_folders(store: &FaceStore, queue: &VecDeque<Requested>) {
    persist_list(store, FOLDERS_KEY, queue);
}
/// Folders ever asked for (kept after their scan finished): a change of model or detection size scans them again.
const KNOWN_KEY: &str = "folders_known";
fn remember_folder(store: &FaceStore, entry: &Requested) {
    let mut known = known_folders(store);
    if !known.contains(entry) {
        known.push_back(entry.clone());
        persist_list(store, KNOWN_KEY, &known);
    }
}
fn known_folders(store: &FaceStore) -> VecDeque<Requested> {
    load_list(store, KNOWN_KEY)
}
/// Puts every known folder back in the queue (those already there stay once). Returns how many were added.
pub fn requeue_known(store: &FaceStore, queue: &mut VecDeque<Requested>) -> usize {
    let before = queue.len();
    let mut known = known_folders(store);
    if known.is_empty() {
        // folders scanned before they were recorded: where the scanned photos are (never a network share, as asked before)
        known = store.scanned_roots().unwrap_or_default().into_iter().map(|p| (PathBuf::from(p), true, false)).filter(|(p, _, _)| p.is_absolute()).collect();
    }
    for k in known {
        if !queue.contains(&k) {
            queue.push_back(k);
        }
    }
    persist_folders(store, queue);
    queue.len() - before
}
fn persist_list(store: &FaceStore, key: &str, queue: &VecDeque<Requested>) {
    let list: Vec<(&str, bool, bool)> = queue.iter().filter_map(|(p, s, n)| p.to_str().map(|p| (p, *s, *n))).collect();
    let _ = store.meta_set(key, &serde_json::to_string(&list).unwrap_or_default());
}
fn load_folders(store: &FaceStore) -> VecDeque<Requested> {
    load_list(store, FOLDERS_KEY)
}

fn load_list(store: &FaceStore, key: &str) -> VecDeque<Requested> {
    // the older form had no network flag: it means not allowed
    let list: Vec<serde_json::Value> = store.meta_get(key).ok().flatten().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    list.iter()
        .filter_map(|e| {
            let e = e.as_array()?;
            Some((PathBuf::from(e.first()?.as_str()?), e.get(1)?.as_bool()?, e.get(2).and_then(serde_json::Value::as_bool).unwrap_or(false)))
        })
        .filter(|(p, _, _)| p.is_absolute())
        .collect()
}

/// Whether People scans only the folders it was given (the setting the page writes: "catalog" or "folder").
pub fn scope_is_folder(runtime: &Path) -> bool {
    crate::settings::read_key(runtime, "faces.scope").as_deref() == Some("folder")
}

impl Control {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
    }
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
    pub fn progress(&self) -> FacesProgress {
        let p = &self.progress;
        FacesProgress {
            scanned: p.scanned.load(Ordering::Acquire),
            pending: u32::try_from(p.pending.load(Ordering::Acquire)).ok(),
            running: p.running.load(Ordering::Acquire) as u32,
            faces_found: p.faces_found.load(Ordering::Acquire),
            folder_pending: self.folders.lock().unwrap().len() as u32,
        }
    }
    /// The last inference time (ms) and where the pictures came from: file, engine render, companion copy.
    pub fn speed(&self) -> (Option<u32>, (u64, u64, u64)) {
        let p = &self.progress;
        let ms = p.last_ms.load(Ordering::Acquire);
        ((ms > 0).then_some(ms as u32), (p.from_file.load(Ordering::Acquire), p.rendered.load(Ordering::Acquire), p.mirrored.load(Ordering::Acquire)))
    }
    /// "blocked", "retrying" or "ok", for the card.
    pub fn health(&self) -> crate::tags_health::Report {
        self.health.report(now_ms())
    }
    pub fn retry(&self) {
        self.health.retry();
        self.ledger.reset();
    }
    /// The model or the detection size changed (or the user asked to reprocess): look for photos to scan now instead of
    /// at the next pass. The folders asked for earlier go back in the queue, the Catalog scan starts from its beginning,
    /// and the pending count and progress are refreshed at once. Nothing is wiped.
    pub fn wake(&self) {
        requeue_known(&self.store, &mut self.folders.lock().unwrap());
        self.wake.store(true, Ordering::Release);
    }
    /// The names and groups are out of date (a review decision, a merge): recompute them soon.
    pub fn request_analysis(&self) {
        self.urgent.store(true, Ordering::Release);
        self.dirty.store(true, Ordering::Release);
    }
    /// Scans one folder once (with its subfolders when asked), in addition to the Catalog, or instead of
    /// it when `faces.scope` is "folder". The request survives a restart.
    pub fn scan_folder(&self, folder: PathBuf, subfolders: bool, allow_network: bool) {
        let mut q = self.folders.lock().unwrap();
        remember_folder(&self.store, &(folder.clone(), subfolders, allow_network));
        if !q.iter().any(|(p, s, n)| *p == folder && *s == subfolders && *n == allow_network) {
            q.push_back((folder, subfolders, allow_network));
            persist_folders(&self.store, &q);
        }
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Service {
    cache: Option<Arc<SearchCache>>,
    models: Arc<Models>,
    store: Arc<FaceStore>,
    queue: Arc<AnalysisQueue>,
    engine: Tagged,
    output: Arc<FrameOutput>,
    shared: Shared,
    runtime: PathBuf,
    cursor: AtomicI64,
    stop: Arc<AtomicBool>,
    progress: Arc<Progress>,
    health: Arc<Health>,
    ledger: Arc<Ledger>,
    dirty: Arc<AtomicBool>,
    urgent: Arc<AtomicBool>,
    wake: Arc<AtomicBool>,
    /// Where the pass over stale stored photos stopped (a path).
    stale_cursor: Mutex<String>,
    folders: Arc<Mutex<VecDeque<Requested>>>,
    inflight: Mutex<HashSet<String>>,
    idle_since: Mutex<Option<Instant>>,
    loaded: AtomicBool,
    emit: Emit,
}

#[allow(clippy::too_many_arguments)]
pub fn start(
    cache: Option<Arc<SearchCache>>,
    models: Arc<Models>,
    store: Arc<FaceStore>,
    queue: Arc<AnalysisQueue>,
    engine: Tagged,
    output: Arc<FrameOutput>,
    runtime: PathBuf,
    emit: Emit,
) -> Control {
    let stop = Arc::new(AtomicBool::new(false));
    let progress = Arc::new(Progress::new());
    let health = Arc::new(Health::default());
    let ledger = Arc::new(Ledger::default());
    let dirty = Arc::new(AtomicBool::new(true));
    let urgent = Arc::new(AtomicBool::new(false));
    let wake = Arc::new(AtomicBool::new(false));
    let folders = Arc::new(Mutex::new(load_folders(&store)));
    let service = Arc::new(Service {
        cache,
        models,
        store: store.clone(),
        queue,
        engine,
        output,
        shared: Shared::new(),
        runtime,
        cursor: AtomicI64::new(0),
        stop: stop.clone(),
        progress: progress.clone(),
        health: health.clone(),
        ledger: ledger.clone(),
        dirty: dirty.clone(),
        urgent: urgent.clone(),
        wake: wake.clone(),
        stale_cursor: Mutex::new(String::new()),
        folders: folders.clone(),
        inflight: Mutex::default(),
        idle_since: Mutex::new(None),
        loaded: AtomicBool::new(false),
        emit,
    });
    std::thread::spawn(move || service.run());
    Control { stop, progress, health, ledger, dirty, urgent, folders, store, wake }
}

/// Maps the face worker's own words to what the worker does about them.
pub fn classify(msg: &str) -> Kind {
    let m = msg.to_ascii_lowercase();
    let has = |s: &str| m.contains(s);
    if has("worker is full") {
        Kind::Transient
    } else if has("model is unavailable") || has("unsupported face model") || has("not installed") || has("changed") || has("failed pinned") {
        Kind::Systemic(Reason::Model)
    } else if has("built without") {
        Kind::Systemic(Reason::NoCpu)
    } else if has("engine write") || has("engine restarted") || has("timed out") || has("closed") || has("exited") || has("broken pipe") {
        Kind::Systemic(Reason::Crash)
    } else if has("invalid face") || has("could not read the face") || has("could not decode") || has("no usable picture") {
        Kind::Photo
    } else {
        Kind::Systemic(Reason::Other)
    }
}

struct Failure {
    kind: Kind,
    msg: String,
}
impl From<String> for Failure {
    fn from(msg: String) -> Self {
        Failure { kind: classify(&msg), msg }
    }
}
impl Failure {
    fn photo(msg: &str) -> Self {
        Failure { kind: Kind::Photo, msg: msg.to_string() }
    }
}

fn is_photo(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|e| e.to_str()) else { return false };
    let ext = ext.to_ascii_lowercase();
    matches!(ext.as_str(), "jpg" | "jpeg" | "png" | "tif" | "tiff" | "webp" | "heic" | "heif") || crate::catalog_meta::is_raw_ext(&ext)
}

/// Every photo under a folder, in path order, up to a limit.
/// `skip` is asked about every folder before it is listed (a network mount inside the folder is not entered).
pub fn photos_under(folder: &Path, limit: usize, recursive: bool, skip: &dyn Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![folder.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<_> = read.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            let p = e.path();
            let Ok(t) = e.file_type() else { continue };
            if t.is_dir() && recursive && !e.file_name().to_string_lossy().starts_with('.') {
                if !skip(&p) {
                    stack.push(p);
                }
            } else if t.is_file() && is_photo(&p) {
                out.push(p);
                if out.len() >= limit {
                    return out;
                }
            }
        }
    }
    out.sort();
    out
}

impl Service {
    /// The stored model version of the recognizer in use now (the setting, while its file is there).
    fn model_version(&self) -> String {
        fm::version_for(&self.runtime, self.models.recognizer(&self.runtime))
    }

    fn failed(&self, path: &str, version: &str, generation: u64, f: Failure) {
        match f.kind {
            Kind::Transient => {}
            Kind::Systemic(reason) => {
                if let Some(line) = self.health.systemic(generation, reason, &f.msg, now_ms()) {
                    eprintln!("{line}");
                }
                fm::unload(&self.engine);
                self.loaded.store(false, Ordering::Release);
            }
            Kind::Photo => {
                let tries = self.ledger.fail(path, version, now_ms());
                if tries >= MAX_PHOTO_TRIES {
                    // remembered, so the photo is not offered again until its file or the model changes
                    let _ = self.store.put_failure(path, version, &self.model_version(), "failed", &f.msg);
                    eprintln!("faces {path}: given up: {}", f.msg);
                }
            }
        }
    }

    fn run(self: Arc<Self>) {
        let mut last_scan: Option<Instant> = None;
        let mut last_count: Option<Instant> = None;
        let mut last_emit = Instant::now();
        while !self.stop.load(Ordering::Acquire) {
            let p = &self.progress;
            let installed = self.models.installed();
            let enabled = self.models.enabled();
            let explicit = !self.folders.lock().unwrap().is_empty();
            let woken = self.wake.swap(false, Ordering::AcqRel);
            if woken {
                self.cursor.store(0, Ordering::Release);
                self.stale_cursor.lock().unwrap().clear();
                last_scan = None;
                last_count = None;
            }
            p.installed.store(installed, Ordering::Release);
            p.enabled.store(enabled, Ordering::Release);
            if installed && (enabled || explicit) {
                if matches!(self.health.gate(now_ms()), Gate::Go) && last_scan.is_none_or(|t| t.elapsed() >= SCAN_EVERY) {
                    last_scan = Some(Instant::now());
                    let next = self.clone();
                    self.queue.submit("faces:scan".into(), move |job| {
                        if !job.cancelled() && !next.stop.load(Ordering::Acquire) {
                            if let Err(e) = next.scan(&job) {
                                eprintln!("faces: {e}");
                            }
                        }
                    });
                }
                if (woken || p.count_requested()) && last_count.is_none_or(|t| t.elapsed() >= COUNT_EVERY) {
                    last_count = Some(Instant::now());
                    if let Some(n) = self.count_backlog() {
                        p.pending.store(n as i64, Ordering::Release);
                    }
                }
            } else {
                p.pending.store(-1, Ordering::Release);
                last_scan = None;
                last_count = None;
            }
            // names and groups follow the faces once nothing is being scanned, and after a review decision
            let idle = self.inflight.lock().unwrap().is_empty() && p.running.load(Ordering::Acquire) == 0;
            if self.dirty.load(Ordering::Acquire) && (idle || self.urgent.load(Ordering::Acquire)) {
                self.dirty.store(false, Ordering::Release);
                self.urgent.store(false, Ordering::Release);
                let next = self.clone();
                if !self.queue.submit("faces:analyze".into(), move |job| {
                    if !job.cancelled() {
                        match next.reanalyze() {
                            Ok(()) => (next.emit)(FacesEvent::Changed),
                            Err(e) => eprintln!("faces: {e}"),
                        }
                    }
                }) {
                    self.dirty.store(true, Ordering::Release);
                }
            }
            self.maybe_unload(installed && (enabled || explicit));
            if woken || (last_emit.elapsed() >= Duration::from_secs(2) && (p.running.load(Ordering::Acquire) > 0 || p.count_requested())) {
                last_emit = Instant::now();
                (self.emit)(FacesEvent::Progress(FacesProgress {
                    scanned: p.scanned.load(Ordering::Acquire),
                    pending: u32::try_from(p.pending.load(Ordering::Acquire)).ok(),
                    running: p.running.load(Ordering::Acquire) as u32,
                    faces_found: p.faces_found.load(Ordering::Acquire),
                    folder_pending: self.folders.lock().unwrap().len() as u32,
                }));
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    // frees the models in the engine after a quiet spell
    fn maybe_unload(&self, active: bool) {
        if !self.loaded.load(Ordering::Acquire) {
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
            fm::unload(&self.engine);
            self.loaded.store(false, Ordering::Release);
        }
    }

    fn deferred(&self, path: &str) -> bool {
        self.engine.quiet_for() < SHARE_QUIET && crate::volumes::on_network(Path::new(path))
    }

    fn count_backlog(&self) -> Option<u32> {
        let selection = library_selection(&self.runtime);
        let now = now_ms();
        let (mut after, mut total) = (0i64, 0u32);
        let policy = NetPolicy::take(&self.runtime, &[]);
        if let Some(cache) = self.cache.as_ref().filter(|_| !scope_is_folder(&self.runtime)) {
            loop {
                if self.stop.load(Ordering::Acquire) {
                    return None;
                }
                let rows = cache.assets_after(after, PAGE).ok()?;
                let end = (rows.len() as u32) < PAGE;
                after = rows.last().map_or(after, |r| r.id);
                for row in rows {
                    if self.models.enabled()
                        && selected(Path::new(&row.path), &selection)
                        && !policy.blocked(Path::new(&row.path))
                        && !self.store.is_current(&row.path, &row.original_fingerprint, &self.model_version()).ok()?
                        && !self.store.knows(&row.path).ok()?
                        && !self.ledger.skipped(&row.path, &row.original_fingerprint, now)
                    {
                        total = total.saturating_add(1);
                    }
                }
                if end {
                    break;
                }
            }
        }
        let version = self.model_version();
        // stored photos to scan again are counted from the store (the Catalog loop above leaves out the ones it knows)
        total = total.saturating_add(self.store.stale_total(&version).ok()?);
        // a folder still to be looked at counts one: its new photos are only found by walking it (its stale ones are counted above)
        total = total.saturating_add(self.folders.lock().unwrap().len() as u32);
        Some(total)
    }

    fn offer(self: &Arc<Self>, path: String, fingerprint: String) -> bool {
        if self.ledger.skipped(&path, &fingerprint, now_ms()) || !matches!(self.health.gate(now_ms()), Gate::Go) {
            return true;
        }
        {
            let mut inflight = self.inflight.lock().unwrap();
            if inflight.len() >= AHEAD || !inflight.insert(path.clone()) {
                return false;
            }
        }
        let next = self.clone();
        let copy = path.clone();
        let queued = self.queue.submit(format!("faces:photo:{path}"), move |job| {
            let generation = next.health.generation();
            if let Err(f) = next.analyze(&path, &fingerprint, &job) {
                next.failed(&path, &fingerprint, generation, f);
            }
            next.inflight.lock().unwrap().remove(&path);
        });
        if !queued {
            self.inflight.lock().unwrap().remove(&copy);
        }
        queued
    }

    fn scan(self: &Arc<Self>, job: &Job) -> Result<(), String> {
        let selection = library_selection(&self.runtime);
        let quiet = self.engine.quiet_for();
        // photos the store already has that are stale (another model or detection size, or marked by a reprocess), straight
        // from its rows: independent of the scope, the folders asked for and the Catalog. A share nobody allowed is left alone.
        {
            let version = self.model_version();
            let requested: Vec<Requested> = self.folders.lock().unwrap().iter().cloned().collect();
            let policy = NetPolicy::take(&self.runtime, &requested);
            let mut after = self.stale_cursor.lock().unwrap().clone();
            let batch = self.store.stale_after(&after, &version, 64)?;
            if batch.is_empty() {
                after.clear();
            }
            for path in &batch {
                if job.cancelled() || self.stop.load(Ordering::Acquire) {
                    return Ok(());
                }
                after = path.clone();
                if policy.blocked(Path::new(path)) {
                    continue;
                }
                // a file that is gone is skipped (and stays stale), the rest is offered
                let Some(fp) = self.fingerprint_now(path) else { continue };
                if !self.offer(path.clone(), fp) {
                    // the worker is full: come back to this one
                    *self.stale_cursor.lock().unwrap() = after.clone();
                    return Ok(());
                }
            }
            *self.stale_cursor.lock().unwrap() = after;
        }
        // an explicitly requested folder first
        let folder = self.folders.lock().unwrap().front().cloned();
        if let Some((folder, recursive, network)) = folder {
            let policy = NetPolicy::take(&self.runtime, &self.folders.lock().unwrap().iter().cloned().collect::<Vec<_>>());
            // a share nobody allowed is dropped without being listed
            if policy.blocked(&folder) {
                let mut q = self.folders.lock().unwrap();
                q.retain(|f| *f != (folder.clone(), recursive, network));
                persist_folders(&self.store, &q);
                return Ok(());
            }
            let mut offered = false;
            for p in photos_under(&folder, FOLDER_LIMIT, recursive, &|d| policy.blocked(d)) {
                if job.cancelled() || self.stop.load(Ordering::Acquire) {
                    return Ok(());
                }
                let Some(path) = p.to_str().map(str::to_string) else { continue };
                if policy.blocked(&p) {
                    continue;
                }
                let Some(fp) = self.fingerprint_now(&path) else { continue };
                if self.store.is_current(&path, &fp, &self.model_version())? || self.ledger.skipped(&path, &fp, now_ms()) {
                    continue;
                }
                offered = true;
                if !self.offer(path, fp) {
                    return Ok(());
                }
            }
            // done once nothing is left to offer and the photos handed to the worker are stored
            if !offered && self.inflight.lock().unwrap().is_empty() {
                let mut q = self.folders.lock().unwrap();
                remember_folder(&self.store, &(folder.clone(), recursive, network));
                q.retain(|f| *f != (folder.clone(), recursive, network));
                persist_folders(&self.store, &q);
            }
            return Ok(());
        }
        if scope_is_folder(&self.runtime) {
            return Ok(());
        }
        let Some(cache) = &self.cache else { return Ok(()) };
        if !self.models.enabled() {
            return Ok(());
        }
        let policy = NetPolicy::take(&self.runtime, &[]);
        let start = self.cursor.load(Ordering::Acquire);
        let (mut after, mut lead, mut leading) = (start, start, true);
        loop {
            if job.cancelled() || self.stop.load(Ordering::Acquire) {
                return Ok(());
            }
            let rows = cache.assets_after(after, PAGE)?;
            let end = (rows.len() as u32) < PAGE;
            for a in rows {
                after = a.id;
                let path = Path::new(&a.path);
                // a network source nobody allowed is skipped on the path alone: nothing is stat'ed
                let current = !selected(path, &selection) || policy.blocked(path) || self.store.is_current(&a.path, &a.original_fingerprint, &self.model_version())?;
                if current || (quiet < SHARE_QUIET && crate::volumes::on_network(path)) {
                    if leading {
                        lead = a.id;
                    }
                    continue;
                }
                leading = false;
                if job.cancelled() || !self.offer(a.path, a.original_fingerprint) {
                    self.cursor.store(lead, Ordering::Release);
                    return Ok(());
                }
            }
            if end {
                self.cursor.store(0, Ordering::Release);
                return Ok(());
            }
        }
    }

    fn fingerprint_now(&self, path: &str) -> Option<String> {
        let p = Path::new(path);
        if !crate::volumes::answering(p) {
            return None;
        }
        crate::cache::listed_metadata_fingerprints(&Mutex::new(crate::cache::Listed::default()), p, 0).map(|(original, _)| original)
    }

    /// The photo's small picture: from its own file when it has a usable one, else rendered by the engine.
    fn picture(&self, path: &str, fingerprint: &str) -> Result<Arc<image::RgbImage>, String> {
        // the picture at this detection size: a change of size must not hit a picture made at another
        let key = format!("{fingerprint}:{}", fm::wanted_detect_size(&self.runtime));
        if let Some((img, _)) = self.shared.get(path, &key) {
            self.progress.from_file.fetch_add(1, Ordering::AcqRel);
            return Ok(img);
        }
        if let Some(pic) = analysis_image::picture_at(Path::new(path), fm::wanted_detect_size(&self.runtime)) {
            let img = Arc::new(pic.rgb);
            self.shared.put(path, &key, img.clone());
            self.progress.from_file.fetch_add(1, Ordering::AcqRel);
            return Ok(img);
        }
        let tag = self.engine.tag();
        let out = self.output.path(&format!("faces-{tag}.jpg"));
        let imgid = self.engine.cached_photo(path, 0);
        let preview = (|| {
            self.engine.request(tag, json!({"cmd":"analysis_preview","imgid":imgid,"subject":"original","out":out}))?;
            FrameOutput::read(&out).map_err(|e| e.to_string())
        })();
        self.output.remove(&out);
        let mut reader = image::ImageReader::new(std::io::Cursor::new(preview?));
        reader.set_format(image::ImageFormat::Jpeg);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(analysis_image::EDGE);
        limits.max_image_height = Some(analysis_image::EDGE);
        limits.max_alloc = Some(16 * 1024 * 1024);
        reader.limits(limits);
        let img = Arc::new(reader.decode().map_err(|e| format!("could not decode the picture: {e}"))?.to_rgb8());
        self.shared.put(path, &key, img.clone());
        self.progress.rendered.fetch_add(1, Ordering::AcqRel);
        Ok(img)
    }

    fn faces_in(&self, root: &Path, img: &image::RgbImage) -> Result<Vec<NewFace>, String> {
        let arcface = (self.models.recognizer(&self.runtime) == fm::Recognizer::Arcface).then(|| self.models.arcface()).flatten();
        let (faces, ms) = detect_and_embed_with(&self.engine, &self.runtime, root, arcface.as_deref(), img, fm::wanted_detect_size(&self.runtime) > 1024)?;
        self.loaded.store(true, Ordering::Release);
        self.progress.last_ms.store(ms.round() as u64, Ordering::Release);
        Ok(faces)
    }

    fn analyze(&self, path: &str, fingerprint: &str, job: &Job) -> Result<(), Failure> {
        let explicit = !self.folders.lock().unwrap().is_empty();
        if job.cancelled() || self.stop.load(Ordering::Acquire) || !(self.models.enabled() || explicit) {
            return Ok(());
        }
        // before anything touches the file: a network share nobody allowed is never read
        let requested: Vec<Requested> = self.folders.lock().unwrap().iter().cloned().collect();
        if NetPolicy::take(&self.runtime, &requested).blocked(Path::new(path)) {
            return Ok(());
        }
        if !explicit && !scope_is_folder(&self.runtime) && !self.store.knows(path).unwrap_or(false) && (!selected(Path::new(path), &library_selection(&self.runtime)) || self.deferred(path)) {
            return Ok(());
        }
        if self.fingerprint_now(path).as_deref() != Some(fingerprint) || !matches!(self.health.gate(now_ms()), Gate::Go) {
            return Ok(());
        }
        let root = self.models.package().map_err(|e| Failure::from(format!("not installed: {e}")))?;
        let running = RunGuard::new(&self.progress);
        let rgb = self.picture(path, fingerprint).map_err(|e| Failure { kind: classify(&e), msg: e })?;
        if job.cancelled() {
            return Ok(());
        }
        // a RAW and its JPG are one picture: the second file takes the first's faces
        let companion = self.store.companion(path, rgb.width(), rgb.height(), &self.model_version())?;
        let faces = match &companion {
            Some(primary) => {
                self.progress.mirrored.fetch_add(1, Ordering::AcqRel);
                self.store.new_faces_of(primary)?
            }
            None => self.faces_in(&root, &rgb)?,
        };
        if let Some(line) = self.health.ok() {
            eprintln!("{line}");
        }
        if job.cancelled() || self.fingerprint_now(path).as_deref() != Some(fingerprint) {
            return Ok(());
        }
        let n = faces.len() as u64;
        match job.commit(|| self.store.put_scan(path, fingerprint, &self.model_version(), rgb.width(), rgb.height(), &faces, companion.as_deref())) {
            Some(r) => r.map(|_| ())?,
            None => return Ok(()),
        }
        running.stored();
        self.ledger.clear(path);
        self.progress.scanned.fetch_add(1, Ordering::AcqRel);
        self.progress.faces_found.fetch_add(n, Ordering::AcqRel);
        self.dirty.store(true, Ordering::Release);
        if n > 0 {
            (self.emit)(FacesEvent::Changed);
        }
        Ok(())
    }

    /// Suggests a person for each unnamed face that resembles one, then groups the rest.
    fn reanalyze(&self) -> Result<(), String> {
        let r = self.models.recognizer(&self.runtime);
        let model = fm::version_for(&self.runtime, r);
        let decisions = Decisions { joined: self.store.face_links()?, apart: self.store.groups_not_same()? };
        let (suggestions, groups) = analysis(&self.store.exemplars(&model)?, &self.store.unnamed(&model)?, &fm::thresholds_for(r, fm::wanted_strictness(&self.runtime)), &Params::default(), &decisions);
        self.store.set_analysis(&suggestions, &groups)
    }
}

/// YuNet, then the recognizer for each face found (SFace, or ArcFace from `arcface`'s folder); also the
/// milliseconds the engine spent.
pub fn detect_and_embed(engine: &Tagged, runtime: &Path, root: &Path, arcface: Option<&Path>, img: &image::RgbImage) -> Result<(Vec<NewFace>, f64), String> {
    detect_and_embed_with(engine, runtime, root, arcface, img, false)
}

/// The faces of a picture. The detector sees 640 px, so a picture is first squeezed whole into it; with `tiled`
/// the picture is also looked at in 640 px squares at its own scale, which finds the small faces of group shots.
pub fn detect_faces(engine: &Tagged, runtime: &Path, root: &Path, img: &image::RgbImage, tiled: bool) -> Result<(Vec<det::Face>, f64), String> {
    let (input, lb) = det::detector_input(img);
    let (tensors, mut total_ms) = fm::run(engine, runtime, root, Model::Detector, &[1, 3, 640, 640], &input)?;
    let mut found = det::faces_from_outputs(&fm::detector_outputs(&tensors)?, &lb, img.width(), img.height());
    if tiled {
        for tile in det::tile_rects(img.width(), img.height(), 160) {
            let sub = image::imageops::crop_imm(img, tile.0, tile.1, tile.2, tile.3).to_image();
            let (input, lb) = det::detector_input(&sub);
            let (tensors, ms) = fm::run(engine, runtime, root, Model::Detector, &[1, 3, 640, 640], &input)?;
            total_ms += ms;
            for f in det::faces_from_outputs(&fm::detector_outputs(&tensors)?, &lb, sub.width(), sub.height()) {
                found.extend(det::face_from_tile(&f, tile, img.width(), img.height()).filter(|g| det::valid(g, img.width(), img.height())));
            }
        }
        found = det::nms(found);
    }
    Ok((found, total_ms))
}

pub fn detect_and_embed_with(engine: &Tagged, runtime: &Path, root: &Path, arcface: Option<&Path>, img: &image::RgbImage, tiled: bool) -> Result<(Vec<NewFace>, f64), String> {
    let (found, mut total_ms) = detect_faces(engine, runtime, root, img, tiled)?;
    let mut out = Vec::with_capacity(found.len());
    for f in found {
        let (tensor, aligned) = det::embedding_input(img, &f);
        let (outputs, ms, dims) = match arcface {
            Some(arc_root) => {
                let (o, ms) = fm::run(engine, runtime, arc_root, Model::Arcface, &[1, 3, 112, 112], &det::arcface_input(&tensor))?;
                (o, ms, fm::ARCFACE_DIMS)
            }
            None => {
                let (o, ms) = fm::run(engine, runtime, root, Model::Recognizer, &[1, 3, 112, 112], &tensor)?;
                (o, ms, det::EMBED_DIMS)
            }
        };
        total_ms += ms;
        let mut emb = outputs.first().map(|t| t.data.clone()).filter(|d| d.len() == dims).ok_or("The recognizer returned the wrong size of embedding")?;
        if !det::l2_normalise(&mut emb) {
            return Err("invalid face embedding".into());
        }
        let mut lm = [0f32; 10];
        for (i, p) in f.lm.iter().enumerate() {
            lm[2 * i] = p.0;
            lm[2 * i + 1] = p.1;
        }
        out.push(NewFace {
            x: f.x,
            y: f.y,
            w: f.w,
            h: f.h,
            score: f.score,
            lm,
            frontal: det::frontal_score(&f),
            roll: det::roll_degrees(&f, img.width(), img.height()),
            aligned,
            emb,
        });
    }
    Ok((out, total_ms))
}

/// A face may start a group when it is big enough to be told apart, sharp enough for the detector to be sure
/// and turned enough toward the camera. Others only join a group that exists (faces_cluster::group_faces).
pub const SEED_MIN_PX: f32 = 48.0;
pub const SEED_MIN_SCORE: f32 = 0.75;
pub const SEED_MIN_FRONTAL: f32 = 0.35;
pub fn is_seed(f: &Unnamed) -> bool {
    f.px >= SEED_MIN_PX && f.score >= SEED_MIN_SCORE && f.frontal >= SEED_MIN_FRONTAL
}

/// What the user decided about pairs of faces: `joined` pairs end up in one group, `apart` pairs never do.
#[derive(Default, Clone, Debug)]
pub struct Decisions {
    pub joined: Vec<(i64, i64)>,
    pub apart: Vec<(i64, i64)>,
}

/// The pure part of `reanalyze`: (suggestions, groups) for the unnamed faces given every confirmed face.
/// A suggestion is never a decision: the page shows it for the user to confirm or reject.
pub fn analysis(exemplars: &[(i64, i64, Vec<f32>)], unnamed: &[Unnamed], th: &fm::Thresholds, params: &Params, decisions: &Decisions) -> (Vec<(i64, i64, f32)>, Vec<(i64, Vec<i64>)>) {
    let ex: Vec<(i64, &[f32])> = exemplars.iter().map(|(p, _, e)| (*p, e.as_slice())).collect();
    let mut suggestions = Vec::new();
    let mut rest: Vec<usize> = Vec::new();
    for (i, f) in unnamed.iter().enumerate() {
        match faces_cluster::suggest(&f.emb, &ex, th.suggest, &f.skip) {
            Some((person, sim)) => suggestions.push((f.id, person, sim)),
            None => rest.push(i),
        }
    }
    let embs: Vec<&[f32]> = rest.iter().map(|&i| unnamed[i].emb.as_slice()).collect();
    let seed: Vec<bool> = rest.iter().map(|&i| is_seed(&unnamed[i])).collect();
    let at: std::collections::HashMap<i64, usize> = rest.iter().enumerate().map(|(k, &i)| (unnamed[i].id, k)).collect();
    let pairs = |p: &[(i64, i64)]| -> Vec<(usize, usize)> { p.iter().filter_map(|(a, b)| Some((*at.get(a)?, *at.get(b)?))).collect() };
    let mut clusters = faces_cluster::group_faces(&embs, &seed, th, params, &pairs(&decisions.apart));
    // a group that holds both faces of a pair the user said are different people is split between them
    for (x, y) in pairs(&decisions.apart) {
        if let Some(g) = clusters.iter().position(|g| g.contains(&x) && g.contains(&y)) {
            let members = std::mem::take(&mut clusters[g]);
            let (mut cx, mut cy) = (embs[x].to_vec(), embs[y].to_vec());
            let mut sides = (Vec::new(), Vec::new());
            for _ in 0..3 {
                sides = (Vec::new(), Vec::new());
                for &m in &members {
                    if m == x || (m != y && det::cosine(embs[m], &cx) >= det::cosine(embs[m], &cy)) {
                        sides.0.push(m);
                    } else {
                        sides.1.push(m);
                    }
                }
                let mean = |s: &Vec<usize>| {
                    let mut v = vec![0f32; cx.len()];
                    for &m in s {
                        v.iter_mut().zip(embs[m]).for_each(|(a, b)| *a += b);
                    }
                    det::l2_normalise(&mut v);
                    v
                };
                (cx, cy) = (mean(&sides.0), mean(&sides.1));
            }
            clusters[g] = sides.0;
            clusters.push(sides.1);
        }
    }
    clusters.retain(|g| g.len() >= params.min_size);
    // groups the user joined stay joined, and a loose face joined to a group goes into it
    for (a, b) in pairs(&decisions.joined) {
        let find = |c: &Vec<Vec<usize>>, x: usize| c.iter().position(|g| g.contains(&x));
        match (find(&clusters, a), find(&clusters, b)) {
            (Some(x), Some(y)) if x != y => {
                let moved = std::mem::take(&mut clusters[y]);
                clusters[x].extend(moved);
            }
            (Some(x), None) => clusters[x].push(b),
            (None, Some(y)) => clusters[y].push(a),
            _ => {}
        }
        clusters.retain(|g| !g.is_empty());
    }
    let mut groups = Vec::new();
    for c in clusters {
        let rep = rest[faces_cluster::medoid(&embs, &c)];
        groups.push((unnamed[rep].id, c.iter().map(|&k| unnamed[rest[k]].id).collect()));
    }
    (suggestions, groups)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit(v: Vec<f32>) -> Vec<f32> {
        let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        v.into_iter().map(|x| x / n).collect()
    }
    fn at(angle: f32) -> Vec<f32> {
        let mut v = vec![0.0; 128];
        v[0] = angle.cos();
        v[1] = angle.sin();
        unit(v)
    }

    #[test]
    fn analysis_names_faces_like_a_person_and_groups_the_rest() {
        let exemplars = vec![(7, 100, at(0.0)), (8, 101, at(1.6))];
        let u = |id: i64, emb: Vec<f32>, skip: Vec<i64>| Unnamed { id, emb, skip, px: 120.0, score: 0.9, frontal: 0.9 };
        let unnamed = vec![
            u(1, at(0.1), vec![]),   // like person 7
            u(2, at(1.5), vec![]),   // like person 8
            u(3, at(0.15), vec![7]), // like 7, but the user said it is not 7: not suggested for them
            u(4, at(3.0), vec![]),   // a group with 5, 6
            u(5, at(3.02), vec![]),
            u(6, at(3.04), vec![]),
            u(9, at(4.5), vec![]), // alone
        ];
        let (sug, groups) = analysis(&exemplars, &unnamed, &fm::thresholds(fm::Recognizer::Sface), &Params::default(), &Decisions::default());
        let by: std::collections::HashMap<i64, i64> = sug.iter().map(|s| (s.0, s.1)).collect();
        assert_eq!((by[&1], by[&2]), (7, 8));
        assert!(!by.contains_key(&3) || by[&3] != 7);
        assert_eq!(groups.len(), 1, "{groups:?}");
        let mut members = groups[0].1.clone();
        members.sort();
        assert!(members.starts_with(&[4, 5, 6]) && !members.contains(&9));
        assert!(members.contains(&groups[0].0), "the representative is one of the members");
    }

    #[test]
    fn requested_folders_survive_a_restart_and_the_scope_setting_is_read() {
        let store = FaceStore::from_connection(rusqlite::Connection::open_in_memory().unwrap()).unwrap();
        assert!(load_folders(&store).is_empty());
        let q: VecDeque<Requested> = [(PathBuf::from("/a"), true, false), (PathBuf::from("/b c"), false, true)].into();
        persist_folders(&store, &q);
        assert_eq!(load_folders(&store), q);
        store.meta_set(FOLDERS_KEY, "not json").unwrap();
        assert!(load_folders(&store).is_empty());
        store.meta_set(FOLDERS_KEY, r#"[["/old",true]]"#).unwrap();
        assert_eq!(load_folders(&store), VecDeque::from([(PathBuf::from("/old"), true, false)]), "an older entry means no network");
        store.meta_set(FOLDERS_KEY, r#"[["relative",true,false]]"#).unwrap();
        assert!(load_folders(&store).is_empty(), "only absolute paths are scanned");
        let dir = tempfile::tempdir().unwrap();
        assert!(!scope_is_folder(dir.path()));
        crate::settings::Settings::load(dir.path()).set("faces.scope", "folder").unwrap();
        assert!(scope_is_folder(dir.path()));
        crate::settings::Settings::load(dir.path()).set("faces.scope", "catalog").unwrap();
        assert!(!scope_is_folder(dir.path()));
    }

    const TABLE: &str = "/dev/sda2 / ext4 rw 0 0\n//nas/photos /mnt/synology cifs rw 0 0\n/dev/sdb1 /mnt/disk ext4 rw 0 0\nserver:/x /srv/nfs nfs4 rw 0 0\n";

    #[test]
    fn network_paths_are_blocked_unless_allowed_and_nothing_is_touched_to_decide() {
        let dir = tempfile::tempdir().unwrap();
        let none: Vec<Requested> = vec![];
        let p = NetPolicy::new(MountSnapshot::from_table(TABLE), dir.path(), &none);
        // none of these paths exists: the decision comes from the mount table alone
        assert!(p.blocked(Path::new("/mnt/synology/photos/a.jpg")));
        assert!(p.blocked(Path::new("/srv/nfs/a.jpg")));
        assert!(!p.blocked(Path::new("/mnt/disk/a.jpg")), "a local disk under /mnt is fine");
        assert!(!p.blocked(Path::new("/home/me/a.jpg")));
        // an explicit request with the network allowed opens that folder only
        let asked = vec![(PathBuf::from("/mnt/synology/trip"), true, true), (PathBuf::from("/srv/nfs"), true, false)];
        let p = NetPolicy::new(MountSnapshot::from_table(TABLE), dir.path(), &asked);
        assert!(!p.blocked(Path::new("/mnt/synology/trip/a.jpg")));
        assert!(p.blocked(Path::new("/mnt/synology/other/a.jpg")));
        assert!(p.blocked(Path::new("/srv/nfs/a.jpg")), "asked for without the network flag: still blocked");
        // a source the user allowed in settings
        crate::settings::Settings::load(dir.path()).set(NETWORK_SOURCES_KEY, r#"["/mnt/synology/photos","relative"]"#).unwrap();
        let p = NetPolicy::new(MountSnapshot::from_table(TABLE), dir.path(), &none);
        assert!(!p.blocked(Path::new("/mnt/synology/photos/2024/a.jpg")));
        assert!(p.blocked(Path::new("/mnt/synology/video/a.mp4")));
        // under /mnt with no table entry at all it may be a share: blocked
        let p = NetPolicy::new(MountSnapshot::from_table("/dev/sda2 /home ext4 rw 0 0\n"), dir.path(), &none);
        assert!(p.blocked(Path::new("/mnt/unknown/a.jpg")));
        // an unreadable table clears nothing
        let unreadable = MountSnapshot::take();
        let _ = unreadable.is_network(Path::new("/"));
    }

    #[test]
    fn skipped_network_sources_counts_library_folders_that_are_blocked() {
        let dir = tempfile::tempdir().unwrap();
        crate::settings::Settings::load(dir.path()).set("immage.library", r#"{"folders":["/home/me/pics","/mnt/synology/photos","/srv/nfs"]}"#).unwrap();
        assert_eq!(skipped_network_sources(dir.path(), MountSnapshot::from_table(TABLE)), 2);
        crate::settings::Settings::load(dir.path()).set(NETWORK_SOURCES_KEY, r#"["/srv/nfs"]"#).unwrap();
        assert_eq!(skipped_network_sources(dir.path(), MountSnapshot::from_table(TABLE)), 1);
    }

    #[test]
    fn a_folder_walk_does_not_enter_a_blocked_subfolder() {
        let dir = tempfile::tempdir().unwrap();
        for f in ["a.jpg", "net/b.jpg", "local/c.jpg"] {
            let p = dir.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        }
        let names: Vec<String> = photos_under(dir.path(), 100, true, &|d| d.ends_with("net")).iter().map(|p| p.strip_prefix(dir.path()).unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["a.jpg", "local/c.jpg"]);
    }

    #[test]
    fn only_big_sharp_frontal_faces_seed_a_group() {
        let f = |px: f32, score: f32, frontal: f32| Unnamed { id: 1, emb: vec![], skip: vec![], px, score, frontal };
        assert!(is_seed(&f(95.0, 0.93, 0.88)));
        assert!(!is_seed(&f(30.0, 0.93, 0.88)), "too small");
        assert!(!is_seed(&f(95.0, 0.6, 0.88)), "the detector is unsure");
        assert!(!is_seed(&f(95.0, 0.93, 0.1)), "turned away");
    }

    #[test]
    fn naming_a_face_names_the_alike_faces_and_leaves_poor_unknown_faces_loose() {
        // person 7 was just confirmed on one face: the faces that look like them are suggested at once
        let exemplars = vec![(7, 100, at(0.0))];
        let u = |id: i64, a: f32, px: f32| Unnamed { id, emb: at(a), skip: vec![], px, score: 0.9, frontal: 0.9 };
        let unnamed = vec![u(1, 0.2, 100.0), u(2, 0.25, 30.0), u(3, 2.0, 100.0), u(4, 2.05, 100.0), u(5, 2.1, 20.0), u(6, 4.0, 20.0)];
        let (sug, groups) = analysis(&exemplars, &unnamed, &fm::thresholds(fm::Recognizer::Sface), &Params::default(), &Decisions::default());
        let named: Vec<i64> = sug.iter().map(|s| s.0).collect();
        assert_eq!(named, [1, 2], "alike faces are suggested whatever their size");
        assert_eq!(groups.len(), 1);
        let mut m = groups[0].1.clone();
        m.sort();
        assert_eq!(m, [3, 4, 5], "the small face joins the group; the lone small face starts none");
    }

    #[test]
    fn joined_groups_stay_joined_and_apart_groups_stay_apart() {
        let u = |id: i64, a: f32| Unnamed { id, emb: at(a), skip: vec![], px: 120.0, score: 0.9, frontal: 0.9 };
        let th = fm::thresholds(fm::Recognizer::Sface);
        // two groups far apart (angle 0 and 1.5), and two alike groups (3.0 and 3.05)
        let unnamed = vec![u(1, 0.0), u(2, 0.01), u(3, 1.5), u(4, 1.51), u(5, 3.0), u(6, 3.01), u(7, 3.05), u(8, 3.06)];
        let none = analysis(&[], &unnamed, &th, &Params::default(), &Decisions::default()).1;
        let sizes = |g: &Vec<(i64, Vec<i64>)>| { let mut s: Vec<usize> = g.iter().map(|x| x.1.len()).collect(); s.sort(); s };
        assert_eq!(sizes(&none), [2, 2, 4], "alike faces group on their own");
        // the user joined the far-apart groups: they stay one group
        let joined = analysis(&[], &unnamed, &th, &Params::default(), &Decisions { joined: vec![(1, 3)], apart: vec![] }).1;
        assert_eq!(sizes(&joined), [4, 4]);
        // the user said the alike groups (5,6) and (7,8) are not the same person: not merged by the merge pass
        let apart = analysis(&[], &unnamed, &th, &Params::default(), &Decisions { joined: vec![], apart: vec![(5, 7)] }).1;
        assert!(apart.iter().all(|g| !(g.1.contains(&5) && g.1.contains(&7))), "{apart:?}");
        assert_eq!(sizes(&apart), [2, 2, 2, 2], "the four alike faces fall into the two pairs");
    }

    #[test]
    fn a_switch_puts_the_known_folders_back_and_counts_the_stale_photos() {
        let store = FaceStore::from_connection(rusqlite::Connection::open_in_memory().unwrap()).unwrap();
        let face = |x: f32| crate::faces_store::NewFace { x, y: 0.1, w: 0.1, h: 0.1, score: 0.9, lm: [0.0; 10], frontal: 0.9, roll: 0.0, aligned: true, emb: vec![1.0; 4] };
        let old = fm::version_for(Path::new("/nonexistent"), fm::Recognizer::Sface);
        for p in ["/pics/a/1.jpg", "/pics/a/2.jpg", "/pics/a/sub/3.jpg", "/pics/b/4.jpg", "/other/5.jpg"] {
            store.put_scan(p, "fp", &old, 100, 100, &[face(0.1)], None).unwrap();
        }
        let new = fm::model_version(fm::Recognizer::Arcface);
        // nothing was recorded about folders: they are where the scanned photos are
        assert_eq!(store.scanned_roots().unwrap(), ["/other", "/pics/a", "/pics/b"]);
        let mut queue = VecDeque::new();
        assert_eq!(requeue_known(&store, &mut queue), 3);
        assert_eq!(requeue_known(&store, &mut queue), 0, "a folder already queued is not queued twice");
        assert!(queue.iter().all(|(_, recursive, network)| *recursive && !*network));
        // after the switch every photo of those folders is stale
        assert_eq!(store.stale_under("/pics/a", new).unwrap(), 3);
        assert_eq!(store.stale_under("/pics", new).unwrap(), 4);
        assert_eq!(store.stale_under("/pics/a", &old).unwrap(), 0, "the same version is not stale");
        assert_eq!(store.stale_under("/pic", new).unwrap(), 0, "a folder name is not a prefix match");
        store.mark_all_stale().unwrap();
        assert_eq!(store.stale_under("/pics/a", &old).unwrap(), 3, "a marked photo is stale at any version");
        // folders asked for are remembered once their scan is done
        let mut q2 = VecDeque::new();
        let store2 = FaceStore::from_connection(rusqlite::Connection::open_in_memory().unwrap()).unwrap();
        remember_folder(&store2, &(PathBuf::from("/x"), false, true));
        remember_folder(&store2, &(PathBuf::from("/x"), false, true));
        assert_eq!(requeue_known(&store2, &mut q2), 1);
        assert_eq!(q2[0], (PathBuf::from("/x"), false, true));
        assert_eq!(load_folders(&store2).len(), 1, "and the queue is kept across a restart");
    }

    #[test]
    fn stale_stored_photos_are_queued_from_their_rows_whatever_the_folders() {
        let store = FaceStore::from_connection(rusqlite::Connection::open_in_memory().unwrap()).unwrap();
        let face = |x: f32| crate::faces_store::NewFace { x, y: 0.1, w: 0.1, h: 0.1, score: 0.9, lm: [0.0; 10], frontal: 0.9, roll: 0.0, aligned: true, emb: vec![1.0; 4] };
        let rt = Path::new("/nonexistent");
        let sface = fm::version_for(rt, fm::Recognizer::Sface);
        let arc = fm::model_version(fm::Recognizer::Arcface);
        for i in 0..7 {
            store.put_scan(&format!("/p/{i}.jpg"), "fp", &sface, 100, 100, &[face(0.1)], None).unwrap();
        }
        store.meta_set(FOLDERS_KEY, "[]").unwrap();
        assert!(load_folders(&store).is_empty(), "the folders asked for are empty, as in the owner's store");
        assert_eq!(store.stale_total(&sface).unwrap(), 0);
        assert!(store.stale_after("", &sface, 100).unwrap().is_empty(), "nothing is stale at the version they were made with");
        // after a recognizer switch every stored photo is stale: 7 queued, by path, in pages
        assert_eq!(store.stale_total(&arc).unwrap(), 7);
        let first = store.stale_after("", &arc, 4).unwrap();
        let second = store.stale_after(first.last().unwrap(), &arc, 4).unwrap();
        assert_eq!((first.len(), second.len()), (4, 3));
        assert_eq!(first.iter().chain(&second).cloned().collect::<Vec<_>>(), (0..7).map(|i| format!("/p/{i}.jpg")).collect::<Vec<_>>());
        assert!(store.stale_after("/p/6.jpg", &arc, 4).unwrap().is_empty(), "the pass ends and starts over");
        // a marked photo is stale at any version, and known photos are known
        store.mark_all_stale().unwrap();
        assert_eq!(store.stale_total(&sface).unwrap(), 7);
        assert!(store.knows("/p/3.jpg").unwrap() && !store.knows("/p/99.jpg").unwrap());
        // and a switch also puts the scanned roots in the folders list that is saved
        let mut q = VecDeque::new();
        assert_eq!(requeue_known(&store, &mut q), 1);
        assert_eq!(load_folders(&store), q, "meta.folders now holds the folders that were scanned");
    }

    #[test]
    fn faces_failures_are_classified_for_the_worker() {
        assert_eq!(classify("face worker is full or stopping"), Kind::Transient);
        assert!(matches!(classify("face model is unavailable; verify the Faces model pack"), Kind::Systemic(Reason::Model)));
        assert!(matches!(classify("This engine was built without CPU ONNX support"), Kind::Systemic(Reason::NoCpu)));
        assert!(matches!(classify("engine restarted"), Kind::Systemic(Reason::Crash)));
        assert!(matches!(classify("Invalid face tensor values"), Kind::Photo));
        assert!(matches!(classify("something odd"), Kind::Systemic(Reason::Other)));
    }

    #[test]
    fn a_folder_scan_lists_photos_in_order_and_skips_hidden_folders() {
        let dir = tempfile::tempdir().unwrap();
        for f in ["b.jpg", "a.CR3", "notes.txt", "sub/c.png", ".hidden/d.jpg", "sub/deep/e.tif"] {
            let p = dir.path().join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, b"x").unwrap();
        }
        let names: Vec<String> = photos_under(dir.path(), 100, true, &|_| false).iter().map(|p| p.strip_prefix(dir.path()).unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["a.CR3", "b.jpg", "sub/c.png", "sub/deep/e.tif"]);
        assert_eq!(photos_under(dir.path(), 2, true, &|_| false).len(), 2);
        let flat: Vec<_> = photos_under(dir.path(), 100, false, &|_| false).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(flat, ["a.CR3", "b.jpg"], "without subfolders only the folder itself");
    }

    /// Real YuNet and SFace through the real engine over a folder of photos (read only), printing
    /// counts and timings, never pictures or embeddings:
    /// IMMAGE_FACES_ROOT=<folder with models/ and engine/> IMMAGE_TEST_ENGINE=<darktable-engine>
    /// IMMAGE_TEST_ORT=<libonnxruntime.so> IMMAGE_TEST_PHOTOS=<folder> [IMMAGE_FACES_LIMIT=60]
    ///   cargo test opt_in_real_faces -- --ignored --nocapture
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs the Faces models, the built engine, ONNX Runtime and photos"]
    fn opt_in_real_faces() {
        let var = |k: &str| PathBuf::from(std::env::var_os(k).unwrap_or_else(|| panic!("set {k}")));
        let (root, engine_path, ort, photos) = (var("IMMAGE_FACES_ROOT"), var("IMMAGE_TEST_ENGINE"), var("IMMAGE_TEST_ORT"), var("IMMAGE_TEST_PHOTOS"));
        let limit: usize = std::env::var("IMMAGE_FACES_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(60);
        let temp = tempfile::tempdir().unwrap();
        let runtime = temp.path().join("runtime");
        std::fs::create_dir_all(&runtime).unwrap();
        fm::write_manifests(&root, &[("detector", "immage-yunet"), ("recognizer", "immage-sface")]).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let wrapper = runtime.join("engine-with-test-ort");
        std::fs::write(&wrapper, format!("#!/bin/sh\nexec '{}' \"$@\" --conf 'plugins/ai/ort_library_path={}'\n", engine_path.display(), ort.display())).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let engine = crate::engine::Engine::spawn(&wrapper, &runtime, &runtime).expect("start the isolated engine");
        let tagged = engine.tagged();
        let arcface = std::env::var_os("IMMAGE_ARCFACE_ROOT").map(PathBuf::from);
        if let Some(a) = &arcface {
            fm::write_manifests(a, &[("arcface", "immage-arcface")]).unwrap();
        }
        let mut files: Vec<PathBuf> = photos_under(&photos, 20000, true, &|_| false);
        // an even spread over the folder, not its first photos
        let step = (files.len() / limit.max(1)).max(1);
        files = files.into_iter().step_by(step).take(limit).collect();
        let store = FaceStore::from_connection(rusqlite::Connection::open_in_memory().unwrap()).unwrap();
        let (mut photos_done, mut with_faces, mut faces_total, mut engine_ms, mut wall_ms) = (0u32, 0u32, 0u32, 0f64, 0f64);
        let mut per_photo_wall: Vec<f64> = Vec::new();
        for (i, p) in files.iter().enumerate() {
            let t0 = Instant::now();
            let Some(pic) = analysis_image::picture(p) else { continue };
            let (faces, ms) = detect_and_embed(&tagged, &runtime, &root, arcface.as_deref(), &pic.rgb).expect("faces");
            let wall = t0.elapsed().as_secs_f64() * 1000.0;
            if i > 0 {
                per_photo_wall.push(wall);
            }
            engine_ms += ms;
            wall_ms += wall;
            photos_done += 1;
            faces_total += faces.len() as u32;
            with_faces += u32::from(!faces.is_empty());
            assert!(faces.iter().all(|f| (f.emb.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-3 && f.x >= 0.0 && f.x + f.w <= 1.0001));
            store.put_scan(&format!("/synthetic/{i}.jpg"), "fp", fm::MODEL_VERSION, pic.rgb.width(), pic.rgb.height(), &faces, None).unwrap();
        }
        let recognizer = if arcface.is_some() { fm::Recognizer::Arcface } else { fm::Recognizer::Sface };
        let (sug, groups) = analysis(&[], &store.unnamed(fm::MODEL_VERSION).unwrap(), &fm::thresholds(recognizer), &Params::default(), &Decisions::default());
        let sizes: Vec<usize> = { let mut s: Vec<usize> = groups.iter().map(|g| g.1.len()).collect(); s.sort_unstable_by(|a, b| b.cmp(a)); s };
        let grouped: usize = sizes.iter().sum();
        per_photo_wall.sort_by(|a, b| a.total_cmp(b));
        let median = per_photo_wall.get(per_photo_wall.len() / 2).copied().unwrap_or(0.0);
        println!("faces real: photos {photos_done}, with faces {with_faces}, faces {faces_total}");
        println!("faces real: wall per photo mean {:.0} ms, median {median:.0} ms (picture decode included), engine inference mean {:.0} ms", wall_ms / f64::from(photos_done.max(1)), engine_ms / f64::from(photos_done.max(1)));
        println!("faces real: groups {}, faces in groups {grouped}, singletons {} ({:.0} %), largest groups {:?}, suggestions {}", groups.len(), faces_total as usize - grouped, 100.0 * (faces_total as usize - grouped) as f64 / f64::from(faces_total.max(1)), &sizes[..sizes.len().min(8)], sug.len());
        assert!(photos_done > 0);
    }

    /// Regroups a COPY of a faces.sqlite with the current pipeline, ignoring names (as a fresh scan would),
    /// and prints counts only: IMMAGE_FACES_DB=<copy of faces.sqlite>
    ///   cargo test opt_in_regroup_store -- --ignored --nocapture
    #[test]
    #[ignore = "needs a copy of a faces.sqlite"]
    fn opt_in_regroup_store() {
        let path = std::env::var("IMMAGE_FACES_DB").expect("set IMMAGE_FACES_DB");
        let db = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let mut q = db
            .prepare("SELECT f.id,f.emb,MAX(f.w*p.width,f.h*p.height),f.score,f.frontal,f.person_id FROM faces f JOIN photos p ON p.path=f.path WHERE f.state<>'rejected' ORDER BY f.id")
            .unwrap();
        let rows: Vec<(Unnamed, bool)> = q
            .query_map([], |r| {
                Ok((Unnamed { id: r.get(0)?, emb: crate::faces_store::emb_from(&r.get::<_, Vec<u8>>(1)?), skip: vec![], px: r.get::<_, f64>(2)? as f32, score: r.get::<_, f64>(3)? as f32, frontal: r.get::<_, f64>(4)? as f32 }, r.get::<_, Option<i64>>(5)?.is_some()))
            })
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let named: std::collections::HashSet<i64> = rows.iter().filter(|r| r.1).map(|r| r.0.id).collect();
        let unnamed: Vec<Unnamed> = rows.into_iter().map(|r| r.0).collect();
        let started = Instant::now();
        let (_, groups) = analysis(&[], &unnamed, &fm::thresholds(fm::Recognizer::Sface), &Params::default(), &Decisions::default());
        let sizes: Vec<usize> = { let mut s: Vec<usize> = groups.iter().map(|g| g.1.len()).collect(); s.sort_unstable_by(|a, b| b.cmp(a)); s };
        let grouped: usize = sizes.iter().sum();
        println!("regroup: faces {}, groups {} {:?}, loose {}, took {} ms", unnamed.len(), groups.len(), sizes, unnamed.len() - grouped, started.elapsed().as_millis());
        // the owner's own naming (one person, 88 faces confirmed) as a reference: how the named faces fall into groups
        let mut in_group: Vec<usize> = groups.iter().map(|g| g.1.iter().filter(|f| named.contains(f)).count()).filter(|n| *n > 0).collect();
        in_group.sort_unstable_by(|a, b| b.cmp(a));
        println!("regroup: the {} confirmed faces of the one named person fall into groups of {:?}", named.len(), in_group);
    }

    /// How many faces each detection size finds, on a read-only sample (counts and timings only):
    /// IMMAGE_FACES_ROOT, IMMAGE_TEST_ENGINE, IMMAGE_TEST_ORT, IMMAGE_TEST_PHOTOS, [IMMAGE_FACES_LIMIT=40]
    ///   cargo test opt_in_detect_sizes -- --ignored --nocapture
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "needs the Faces models, the built engine, ONNX Runtime and photos"]
    fn opt_in_detect_sizes() {
        let var = |k: &str| PathBuf::from(std::env::var_os(k).unwrap_or_else(|| panic!("set {k}")));
        let (root, engine_path, ort, photos) = (var("IMMAGE_FACES_ROOT"), var("IMMAGE_TEST_ENGINE"), var("IMMAGE_TEST_ORT"), var("IMMAGE_TEST_PHOTOS"));
        let limit: usize = std::env::var("IMMAGE_FACES_LIMIT").ok().and_then(|v| v.parse().ok()).unwrap_or(40);
        let temp = tempfile::tempdir().unwrap();
        let runtime = temp.path().join("runtime");
        std::fs::create_dir_all(&runtime).unwrap();
        fm::write_manifests(&root, &[("detector", "immage-yunet"), ("recognizer", "immage-sface")]).unwrap();
        use std::os::unix::fs::PermissionsExt;
        let wrapper = runtime.join("engine-with-test-ort");
        std::fs::write(&wrapper, format!("#!/bin/sh\nexec '{}' \"$@\" --conf 'plugins/ai/ort_library_path={}'\n", engine_path.display(), ort.display())).unwrap();
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
        let engine = crate::engine::Engine::spawn(&wrapper, &runtime, &runtime).expect("start the isolated engine");
        let tagged = engine.tagged();
        let mut files: Vec<PathBuf> = photos_under(&photos, 20000, true, &|_| false).into_iter().filter(|p| !p.to_string_lossy().contains("Processed")).collect();
        let step = (files.len() / limit.max(1)).max(1);
        files = files.into_iter().step_by(step).take(limit).collect();
        // (edge, tiled)
        let variants = [(1024u32, false), (1600, false), (1600, true), (2048, true)];
        let mut found = vec![0usize; variants.len()];
        let mut ms = vec![0f64; variants.len()];
        let (mut small, mut photos_done, mut new_vs_base) = (0usize, 0usize, vec![0usize; variants.len()]);
        for p in &files {
            let Some(base_pic) = analysis_image::picture_at(p, 1024) else { continue };
            photos_done += 1;
            let mut base: Vec<det::Face> = Vec::new();
            for (k, (edge, tiled)) in variants.iter().enumerate() {
                let Some(pic) = analysis_image::picture_at(p, *edge) else { continue };
                let t0 = Instant::now();
                let (faces, _) = detect_faces(&tagged, &runtime, &root, &pic.rgb, *tiled).expect("detect");
                ms[k] += t0.elapsed().as_secs_f64() * 1000.0;
                found[k] += faces.len();
                if k == 0 {
                    small += faces.iter().filter(|f| (f.w * base_pic.rgb.width() as f32) < 48.0).count();
                    base = faces.clone();
                } else {
                    new_vs_base[k] += faces.iter().filter(|f| base.iter().all(|b| det::iou(b, f) < 0.3)).count();
                }
            }
        }
        println!("detect sizes: photos {photos_done}");
        for (k, (edge, tiled)) in variants.iter().enumerate() {
            println!("detect sizes: {edge} px{}: faces {}, not found at 1024 {}, detection ms per photo {:.0}", if *tiled { " tiled" } else { "" }, found[k], new_vs_base[k], ms[k] / photos_done.max(1) as f64);
        }
        println!("detect sizes: faces at 1024 narrower than 48 px (of the 1024 picture): {small}");
    }
}
