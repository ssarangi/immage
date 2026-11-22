// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! Auto-keywords in the background: finds photos of the library that have no result for
//! the current model, runs RAM++ on each in the engine (one serial CPU worker, engine/tags.c),
//! stores the tags (tags_store.rs) and attaches the confident ones as darktable keywords.
//! It runs only while the model is installed AND the "Add keywords automatically" switch is
//! on, on the shared analysis queue (so the one Pause covers it), never more than two photos
//! ahead of the worker, and frees the 1.8 GB model when the backlog is empty.
use crate::{
    analysis_queue::{AnalysisQueue, Job},
    engine::Tagged,
    frame_output::FrameOutput,
    ram_models::{self, Models},
    search_analysis::{library_selection, now_ms, selected, AnalysisProgress, SHARE_QUIET},
    search_cache::SearchCache,
    tags_store::TagStore,
};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicUsize, Ordering},
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
/// The model is freed after this long without a photo to analyse.
const UNLOAD_AFTER: Duration = Duration::from_secs(45);

/// What the status poll reads: memory only, like the Smart search service's.
pub struct Progress {
    installed: AtomicBool,
    enabled: AtomicBool,
    pending: AtomicI64,
    running: AtomicUsize,
    last_work_ms: AtomicU64,
    wanted_ms: AtomicU64,
    /// Milliseconds per photo of the last one analysed (the engine's inference alone).
    last_infer_ms: AtomicU64,
    /// Peak resident memory of the app, in kB, last time a photo finished (Linux).
    last_rss_kb: AtomicU64,
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
}
impl Control {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::Release);
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
    /// The last measured inference time and process memory, for the card's speed line.
    pub fn speed(&self) -> (Option<u32>, Option<u64>) {
        let ms = self.progress.last_infer_ms.load(Ordering::Acquire);
        let kb = self.progress.last_rss_kb.load(Ordering::Acquire);
        ((ms > 0).then_some(ms as u32), (kb > 0).then_some(kb))
    }
}
impl Drop for Control {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Service {
    cache: Arc<SearchCache>,
    models: Arc<Models>,
    store: Arc<TagStore>,
    queue: Arc<AnalysisQueue>,
    engine: Tagged,
    apply: Applier,
    output: Arc<FrameOutput>,
    runtime: PathBuf,
    cursor: AtomicI64,
    stop: Arc<AtomicBool>,
    progress: Arc<Progress>,
    /// Paths handed to the queue and not finished; at most AHEAD.
    inflight: Mutex<HashSet<String>>,
    /// The engine holds the model (a photo was analysed and no unload was sent since).
    loaded: AtomicBool,
    idle_since: Mutex<Option<Instant>>,
}

#[allow(clippy::too_many_arguments)]
pub fn start(
    cache: Option<Arc<SearchCache>>,
    models: Arc<Models>,
    store: Arc<TagStore>,
    queue: Arc<AnalysisQueue>,
    engine: Tagged,
    apply: Applier,
    output: Arc<FrameOutput>,
    runtime: PathBuf,
) -> Control {
    let stop = Arc::new(AtomicBool::new(false));
    let progress = Arc::new(Progress::new());
    if let Some(cache) = cache {
        let service = Arc::new(Service {
            cache,
            models,
            store,
            queue,
            engine,
            apply,
            output,
            runtime,
            cursor: AtomicI64::new(0),
            stop: stop.clone(),
            progress: progress.clone(),
            inflight: Mutex::default(),
            loaded: AtomicBool::new(false),
            idle_since: Mutex::new(None),
        });
        std::thread::spawn(move || service.run());
    }
    Control { stop, progress }
}

impl Service {
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
                if last_scan.is_none_or(|t| t.elapsed() >= SCAN_EVERY) {
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
                last_scan = None;
                last_count = None;
            }
            self.maybe_unload(installed && enabled);
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    // frees the model once there has been nothing to analyse for a while, or the switch is off
    fn maybe_unload(self: &Arc<Self>, active: bool) {
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
            self.loaded.store(false, Ordering::Release);
            let next = self.clone();
            self.queue.submit("tags:unload".into(), move |_| {
                let tag = next.engine.tag();
                let _ = next.engine.request(tag, json!({"cmd":"tags_analyze","unload":true}));
            });
        }
    }

    fn deferred(&self, path: &str) -> bool {
        self.engine.quiet_for() < SHARE_QUIET && crate::volumes::on_network(Path::new(path))
    }

    /// Photos of the selected folders with no result for this model, counted a page at a time.
    fn count_backlog(&self) -> Option<u32> {
        let selection = library_selection(&self.runtime);
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
        {
            let mut inflight = self.inflight.lock().unwrap();
            if inflight.len() >= AHEAD || !inflight.insert(path.clone()) {
                return false;
            }
        }
        let next = self.clone();
        let path_copy = path.clone();
        let key = format!("tags:photo:{path}");
        let queued = self.queue.submit(key, move |job| {
            let result = match &fingerprint {
                Some(fp) => next.analyze(&path, fp, &job),
                None => next.reapply(&path, &job),
            };
            if let Err(e) = result {
                eprintln!("auto-keywords {path}: {e}");
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
        let mut after = self.cursor.load(Ordering::Acquire);
        for _ in 0..64 {
            if job.cancelled() || self.stop.load(Ordering::Acquire) {
                return Ok(());
            }
            let rows = self.cache.assets_after(after, 64)?;
            if rows.is_empty() {
                self.cursor.store(0, Ordering::Release);
                return Ok(());
            }
            for row in &rows {
                after = row.id;
                let path = Path::new(&row.path);
                if !selected(path, &selection)
                    || self.store.is_current(&row.path, &row.original_fingerprint, ram_models::MODEL_VERSION)?
                    || (quiet < SHARE_QUIET && crate::volumes::on_network(path))
                {
                    continue;
                }
                if !self.offer(row.path.clone(), Some(row.original_fingerprint.clone())) {
                    // the worker is busy: continue from this photo at the next scan
                    self.cursor.store(row.id - 1, Ordering::Release);
                    return Ok(());
                }
            }
            self.cursor.store(after, Ordering::Release);
        }
        Ok(())
    }

    fn fingerprint_now(&self, path: &str) -> Option<String> {
        let p = Path::new(path);
        if !crate::volumes::answering(p) {
            return None;
        }
        crate::cache::listed_metadata_fingerprints(&Mutex::new(crate::cache::Listed::default()), p, 0).map(|(original, _)| original)
    }

    fn analyze(&self, path: &str, fingerprint: &str, job: &Job) -> Result<(), String> {
        if job.cancelled() || self.stop.load(Ordering::Acquire) || !self.models.enabled() {
            return Ok(());
        }
        if !selected(Path::new(path), &library_selection(&self.runtime)) || self.deferred(path) {
            return Ok(());
        }
        if self.fingerprint_now(path).as_deref() != Some(fingerprint) {
            return Ok(());
        }
        let (root, vocabulary) = self.models.package()?;
        let running = RunGuard::new(&self.progress);
        let imgid = self.engine.cached_photo(path, 0);
        let tag = self.engine.tag();
        let out = self.output.path(&format!("tags-{tag}.jpg"));
        let preview = (|| {
            self.engine.request(tag, json!({"cmd":"analysis_preview","imgid":imgid,"subject":"original","out":out}))?;
            FrameOutput::read(&out).map_err(|e| e.to_string())
        })();
        self.output.remove(&out);
        let preview = preview?;
        if job.cancelled() {
            return Ok(());
        }
        let mut reader = image::ImageReader::new(std::io::Cursor::new(preview));
        reader.set_format(image::ImageFormat::Jpeg);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(1024);
        limits.max_image_height = Some(1024);
        limits.max_alloc = Some(16 * 1024 * 1024);
        reader.limits(limits);
        let rgb = reader.decode().map_err(|e| e.to_string())?.to_rgb8();
        let tensor = ram_models::preprocess(rgb.as_raw(), rgb.width(), rgb.height())?;
        let logits = infer(&self.engine, &self.runtime, &root, &tensor, &self.progress)?;
        self.loaded.store(true, Ordering::Release);
        let found = vocabulary.select(&logits)?;
        if job.cancelled() || self.fingerprint_now(path).as_deref() != Some(fingerprint) {
            return Ok(());
        }
        let stored: Vec<(String, f32)> = found.iter().map(|t| (t.name.clone(), t.confidence)).collect();
        let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64);
        let committed = job.commit(|| self.store.put(path, fingerprint, ram_models::MODEL_VERSION, &stored, secs));
        match committed {
            Some(r) => r?,
            None => return Ok(()),
        }
        running.stored();
        self.progress.last_rss_kb.store(resident_kb(), Ordering::Release);
        self.decide(path, imgid)
    }

    fn reapply(&self, path: &str, job: &Job) -> Result<(), String> {
        if job.cancelled() || !self.models.enabled() {
            return Ok(());
        }
        let imgid = self.engine.cached_photo(path, 0);
        self.decide(path, imgid)
    }

    /// Attaches the keywords at or above the confidence setting and takes off the ones
    /// Immage attached that are now below it. Keywords the user had are never touched.
    fn decide(&self, path: &str, imgid: i64) -> Result<(), String> {
        let pct = self.models.threshold();
        let stored = self.store.tags(path)?;
        let wanted = |c: f32| c * 100.0 >= f32::from(pct);
        let add: Vec<&str> = stored.iter().filter(|t| wanted(t.confidence) && !t.applied && !t.dismissed).map(|t| t.name.as_str()).collect();
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

/// One RAM++ inference in the engine's CPU worker; returns the logits.
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

/// Resident memory of the engine child process in kB (Linux, read from /proc); 0 elsewhere.
/// The engine holds the model, so its figure is the one that matters.
fn resident_kb() -> u64 {
    #[cfg(target_os = "linux")]
    {
        // the engine child is the one holding the model; ask the process table for its children
        let me = std::process::id();
        let mut total = 0u64;
        if let Ok(entries) = std::fs::read_dir("/proc") {
            for entry in entries.flatten() {
                let Some(pid) = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok()) else { continue };
                let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else { continue };
                let after = stat.rsplit(')').next().unwrap_or("");
                let fields: Vec<&str> = after.split_whitespace().collect();
                if fields.get(1).and_then(|p| p.parse::<u32>().ok()) == Some(me) && stat.contains("darktable-engin") {
                    if let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) {
                        total += status
                            .lines()
                            .find_map(|l| l.strip_prefix("VmRSS:"))
                            .and_then(|v| v.split_whitespace().next())
                            .and_then(|v| v.parse::<u64>().ok())
                            .unwrap_or(0);
                    }
                }
            }
        }
        total
    }
    #[cfg(not(target_os = "linux"))]
    {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

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
        let control = start(Some(cache), models.clone(), store.clone(), queue, tagged.clone(), apply.clone(), Arc::new(FrameOutput::new(&runtime)), runtime.clone());

        // 1. switch off: nothing is read, requested or stored
        std::thread::sleep(Duration::from_secs(6));
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
