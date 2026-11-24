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
    faces_cluster::{self, Params, SUGGEST_SFACE},
    faces_detect as det,
    faces_models::{self as fm, Model, Models},
    faces_store::{FaceStore, NewFace},
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
    folders: Arc<Mutex<VecDeque<(PathBuf, bool)>>>,
    store: Arc<FaceStore>,
}
const FOLDERS_KEY: &str = "folders";

fn persist_folders(store: &FaceStore, queue: &VecDeque<(PathBuf, bool)>) {
    let list: Vec<(&str, bool)> = queue.iter().filter_map(|(p, s)| p.to_str().map(|p| (p, *s))).collect();
    let _ = store.meta_set(FOLDERS_KEY, &serde_json::to_string(&list).unwrap_or_default());
}
fn load_folders(store: &FaceStore) -> VecDeque<(PathBuf, bool)> {
    let list: Vec<(String, bool)> = store.meta_get(FOLDERS_KEY).ok().flatten().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    list.into_iter().map(|(p, s)| (PathBuf::from(p), s)).filter(|(p, _)| p.is_absolute()).collect()
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
    /// The names and groups are out of date (a review decision, a merge): recompute them soon.
    pub fn request_analysis(&self) {
        self.dirty.store(true, Ordering::Release);
    }
    /// Scans one folder once (with its subfolders when asked), in addition to the Catalog, or instead of
    /// it when `faces.scope` is "folder". The request survives a restart.
    pub fn scan_folder(&self, folder: PathBuf, subfolders: bool) {
        let mut q = self.folders.lock().unwrap();
        if !q.iter().any(|(p, s)| *p == folder && *s == subfolders) {
            q.push_back((folder, subfolders));
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
    folders: Arc<Mutex<VecDeque<(PathBuf, bool)>>>,
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
        folders: folders.clone(),
        inflight: Mutex::default(),
        idle_since: Mutex::new(None),
        loaded: AtomicBool::new(false),
        emit,
    });
    std::thread::spawn(move || service.run());
    Control { stop, progress, health, ledger, dirty, folders, store }
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
pub fn photos_under(folder: &Path, limit: usize, recursive: bool) -> Vec<PathBuf> {
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
                stack.push(p);
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
                    let _ = self.store.put_failure(path, version, fm::MODEL_VERSION, "failed", &f.msg);
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
                if p.count_requested() && last_count.is_none_or(|t| t.elapsed() >= COUNT_EVERY) {
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
            if self.dirty.load(Ordering::Acquire) && self.inflight.lock().unwrap().is_empty() && p.running.load(Ordering::Acquire) == 0 {
                self.dirty.store(false, Ordering::Release);
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
            if last_emit.elapsed() >= Duration::from_secs(2) && (p.running.load(Ordering::Acquire) > 0 || p.count_requested()) {
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
                        && !self.store.is_current(&row.path, &row.original_fingerprint, fm::MODEL_VERSION).ok()?
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
        // an explicitly requested folder first
        let folder = self.folders.lock().unwrap().front().cloned();
        if let Some((folder, recursive)) = folder {
            let mut offered = false;
            for p in photos_under(&folder, FOLDER_LIMIT, recursive) {
                if job.cancelled() || self.stop.load(Ordering::Acquire) {
                    return Ok(());
                }
                let Some(path) = p.to_str().map(str::to_string) else { continue };
                let Some(fp) = self.fingerprint_now(&path) else { continue };
                if self.store.is_current(&path, &fp, fm::MODEL_VERSION)? || self.ledger.skipped(&path, &fp, now_ms()) {
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
                q.retain(|f| *f != (folder.clone(), recursive));
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
                let current = !selected(path, &selection) || self.store.is_current(&a.path, &a.original_fingerprint, fm::MODEL_VERSION)?;
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
        if let Some((img, _)) = self.shared.get(path, fingerprint) {
            self.progress.from_file.fetch_add(1, Ordering::AcqRel);
            return Ok(img);
        }
        if let Some(pic) = analysis_image::picture(Path::new(path)) {
            let img = Arc::new(pic.rgb);
            self.shared.put(path, fingerprint, img.clone());
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
        self.shared.put(path, fingerprint, img.clone());
        self.progress.rendered.fetch_add(1, Ordering::AcqRel);
        Ok(img)
    }

    fn faces_in(&self, root: &Path, img: &image::RgbImage) -> Result<Vec<NewFace>, String> {
        let (faces, ms) = detect_and_embed(&self.engine, &self.runtime, root, img)?;
        self.loaded.store(true, Ordering::Release);
        self.progress.last_ms.store(ms.round() as u64, Ordering::Release);
        Ok(faces)
    }

    fn analyze(&self, path: &str, fingerprint: &str, job: &Job) -> Result<(), Failure> {
        let explicit = !self.folders.lock().unwrap().is_empty();
        if job.cancelled() || self.stop.load(Ordering::Acquire) || !(self.models.enabled() || explicit) {
            return Ok(());
        }
        if !explicit && !scope_is_folder(&self.runtime) && (!selected(Path::new(path), &library_selection(&self.runtime)) || self.deferred(path)) {
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
        let companion = self.store.companion(path, rgb.width(), rgb.height(), fm::MODEL_VERSION)?;
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
        match job.commit(|| self.store.put_scan(path, fingerprint, fm::MODEL_VERSION, rgb.width(), rgb.height(), &faces, companion.as_deref())) {
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
        let (suggestions, groups) = analysis(&self.store.exemplars()?, &self.store.unnamed()?, &Params::default());
        self.store.set_analysis(&suggestions, &groups)
    }
}

/// YuNet, then SFace for each face found; also the milliseconds the engine spent.
pub fn detect_and_embed(engine: &Tagged, runtime: &Path, root: &Path, img: &image::RgbImage) -> Result<(Vec<NewFace>, f64), String> {
    let (input, lb) = det::detector_input(img);
    let (tensors, mut total_ms) = fm::run(engine, runtime, root, Model::Detector, &[1, 3, 640, 640], &input)?;
    let found = det::faces_from_outputs(&fm::detector_outputs(&tensors)?, &lb, img.width(), img.height());
    let mut out = Vec::with_capacity(found.len());
    for f in found {
        let (tensor, aligned) = det::embedding_input(img, &f);
        let (outputs, ms) = fm::run(engine, runtime, root, Model::Recognizer, &[1, 3, 112, 112], &tensor)?;
        total_ms += ms;
        let mut emb = outputs.first().map(|t| t.data.clone()).filter(|d| d.len() == det::EMBED_DIMS).ok_or("The recognizer returned the wrong size of embedding")?;
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

/// The pure part of `reanalyze`: (suggestions, groups) for the unnamed faces given every confirmed face.
pub fn analysis(exemplars: &[(i64, i64, Vec<f32>)], unnamed: &[(i64, Vec<f32>, Vec<i64>)], params: &Params) -> (Vec<(i64, i64, f32)>, Vec<(i64, Vec<i64>)>) {
    let ex: Vec<(i64, &[f32])> = exemplars.iter().map(|(p, _, e)| (*p, e.as_slice())).collect();
    let mut suggestions = Vec::new();
    let mut rest: Vec<usize> = Vec::new();
    for (i, (id, emb, skip)) in unnamed.iter().enumerate() {
        match faces_cluster::suggest(emb, &ex, SUGGEST_SFACE, skip) {
            Some((person, sim)) => suggestions.push((*id, person, sim)),
            None => rest.push(i),
        }
    }
    let embs: Vec<&[f32]> = rest.iter().map(|&i| unnamed[i].1.as_slice()).collect();
    let mut groups = Vec::new();
    for c in faces_cluster::cluster(&embs, params) {
        let rep = rest[faces_cluster::medoid(&embs, &c)];
        groups.push((unnamed[rep].0, c.iter().map(|&k| unnamed[rest[k]].0).collect()));
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
        let unnamed = vec![
            (1, at(0.1), vec![]),   // like person 7
            (2, at(1.5), vec![]),   // like person 8
            (3, at(0.15), vec![7]), // like 7, but the user said it is not 7: not suggested for them
            (4, at(3.0), vec![]),   // a group with 5, 6
            (5, at(3.02), vec![]),
            (6, at(3.04), vec![]),
            (9, at(4.5), vec![]), // alone
        ];
        let (sug, groups) = analysis(&exemplars, &unnamed, &Params::default());
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
        let q: VecDeque<(PathBuf, bool)> = [(PathBuf::from("/a"), true), (PathBuf::from("/b c"), false)].into();
        persist_folders(&store, &q);
        assert_eq!(load_folders(&store), q);
        store.meta_set(FOLDERS_KEY, "not json").unwrap();
        assert!(load_folders(&store).is_empty());
        store.meta_set(FOLDERS_KEY, r#"[["relative",true]]"#).unwrap();
        assert!(load_folders(&store).is_empty(), "only absolute paths are scanned");
        let dir = tempfile::tempdir().unwrap();
        assert!(!scope_is_folder(dir.path()));
        crate::settings::Settings::load(dir.path()).set("faces.scope", "folder").unwrap();
        assert!(scope_is_folder(dir.path()));
        crate::settings::Settings::load(dir.path()).set("faces.scope", "catalog").unwrap();
        assert!(!scope_is_folder(dir.path()));
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
        let names: Vec<String> = photos_under(dir.path(), 100, true).iter().map(|p| p.strip_prefix(dir.path()).unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, ["a.CR3", "b.jpg", "sub/c.png", "sub/deep/e.tif"]);
        assert_eq!(photos_under(dir.path(), 2, true).len(), 2);
        let flat: Vec<_> = photos_under(dir.path(), 100, false).iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
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
        let mut files: Vec<PathBuf> = photos_under(&photos, 20000, true);
        // an even spread over the folder, not its first photos
        let step = (files.len() / limit.max(1)).max(1);
        files = files.into_iter().step_by(step).take(limit).collect();
        let store = FaceStore::from_connection(rusqlite::Connection::open_in_memory().unwrap()).unwrap();
        let (mut photos_done, mut with_faces, mut faces_total, mut engine_ms, mut wall_ms) = (0u32, 0u32, 0u32, 0f64, 0f64);
        let mut per_photo_wall: Vec<f64> = Vec::new();
        for (i, p) in files.iter().enumerate() {
            let t0 = Instant::now();
            let Some(pic) = analysis_image::picture(p) else { continue };
            let (faces, ms) = detect_and_embed(&tagged, &runtime, &root, &pic.rgb).expect("faces");
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
        let (sug, groups) = analysis(&[], &store.unnamed().unwrap(), &Params::default());
        let sizes: Vec<usize> = { let mut s: Vec<usize> = groups.iter().map(|g| g.1.len()).collect(); s.sort_unstable_by(|a, b| b.cmp(a)); s };
        let grouped: usize = sizes.iter().sum();
        per_photo_wall.sort_by(|a, b| a.total_cmp(b));
        let median = per_photo_wall.get(per_photo_wall.len() / 2).copied().unwrap_or(0.0);
        println!("faces real: photos {photos_done}, with faces {with_faces}, faces {faces_total}");
        println!("faces real: wall per photo mean {:.0} ms, median {median:.0} ms (picture decode included), engine inference mean {:.0} ms", wall_ms / f64::from(photos_done.max(1)), engine_ms / f64::from(photos_done.max(1)));
        println!("faces real: groups {}, faces in groups {grouped}, singletons {} ({:.0} %), largest groups {:?}, suggestions {}", groups.len(), faces_total as usize - grouped, 100.0 * (faces_total as usize - grouped) as f64 / f64::from(faces_total.max(1)), &sizes[..sizes.len().min(8)], sug.len());
        assert!(photos_done > 0);
    }
}
