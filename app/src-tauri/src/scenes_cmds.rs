// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The Scenes commands: build a folder's timeline of scenes from the page's list of
//! photos, score frames on request, and remember which frames Scenes accepted. A
//! build reads no photo: capture times come from the page's list, looks from the
//! thumbnails the grid already cached (`DiskCache::read_known`, by path alone, so
//! nothing is statted on a network share), and scores from the cull store's cache.
//! Frames still without a cached thumbnail group by time alone.
//!
//! Progress reaches the page as the `scenes-progress` event, at most four a
//! second: `{ job, stage: "reading" | "scoring", done, total }`.
use crate::cache::{Caches, DiskCache};
use crate::cull_cmds;
use crate::cull::CullBlurRegion;
use crate::cull_cmds::CullPlan;
use crate::cull_store::{CullFailure, CullTarget, Store as CullStore};
use crate::scenes_rules::{self, RuleCounts, Known, RuleScene, Rules, Verdict};
use crate::scenes::{self, Block, Frame, Override, SceneItem, SceneParams};
use crate::scenes_store::{SceneAccepted, Store};
use crate::AppState;
use serde::{Deserialize, Serialize};
use serde_json::json;
use specta::Type;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager, State};

/// The most threads reading, decoding and hashing at once.
const MAX_WORKERS: usize = 4;

fn jobs() -> &'static Mutex<HashMap<u64, Arc<AtomicBool>>> {
    static JOBS: OnceLock<Mutex<HashMap<u64, Arc<AtomicBool>>>> = OnceLock::new();
    JOBS.get_or_init(Default::default)
}

struct JobEntry(u64);

impl Drop for JobEntry {
    fn drop(&mut self) {
        jobs().lock().unwrap_or_else(|e| e.into_inner()).remove(&self.0);
    }
}

fn register(job: u64) -> (Arc<AtomicBool>, JobEntry) {
    let cancel = Arc::new(AtomicBool::new(false));
    jobs().lock().unwrap_or_else(|e| e.into_inner()).insert(job, cancel.clone());
    (cancel, JobEntry(job))
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct ScenesRequest {
    pub items: Vec<SceneItem>,
    #[serde(default)]
    pub params: Option<SceneParams>,
}

/// One scene: indexes into the request's items, in capture order.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct SceneRec {
    pub frames: Vec<u32>,
    /// frames of the scene with no cached thumbnail yet, grouped by time alone
    pub unhashed: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq, Type)]
#[serde(rename_all = "camelCase")]
pub struct SceneBlock {
    /// milliseconds since 1970 as if the camera's clock were UTC; None for the no-time block
    pub start_ms: Option<i64>,
    pub end_ms: Option<i64>,
    pub no_time: bool,
    /// some frame's time is its file's modified time, not the camera's
    pub approx: bool,
    pub scenes: Vec<SceneRec>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct ScenesBuild {
    pub blocks: Vec<SceneBlock>,
    /// each item's cached quality (0-1), in the request's order, None while unscored
    pub quality: Vec<Option<f64>>,
    /// each item's cached sharpest region, in the request's order, None while unscored
    pub regions: Vec<Option<CullBlurRegion>>,
    /// the photos are on a network volume
    pub network: bool,
    pub hashed: u32,
    pub unhashed: u32,
    pub reused: u32,
    pub millis: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct SceneScore {
    pub path: String,
    pub version: i64,
    pub quality: Option<f64>,
    pub sharp_region: Option<CullBlurRegion>,
    pub error: Option<String>,
}

fn scenes_path(state: &AppState) -> PathBuf {
    state.runtime.join("scenes.sqlite")
}

/// Emits at most four progress events a second, and the last one always.
struct Throttle {
    last: AtomicU64,
    start: std::time::Instant,
}

impl Throttle {
    fn new() -> Self {
        Self { last: AtomicU64::new(0), start: std::time::Instant::now() }
    }
    fn due(&self, done: usize, total: usize) -> bool {
        let ms = self.start.elapsed().as_millis() as u64;
        if done >= total || ms >= self.last.load(Ordering::Relaxed) + 250 {
            self.last.store(ms, Ordering::Relaxed);
            return true;
        }
        false
    }
}

/// Where the cached thumbnail of an item is read.
pub type ThumbReader<'a> = &'a (dyn Fn(&SceneItem) -> Option<Vec<u8>> + Sync);

/// The timeline of `request`, without tauri: hashes (from `read`'s thumbnails, kept
/// in the store at `store_path`), the sweep, the stored overrides and the cached
/// qualities from the cull store at `cull_path`. None when cancelled.
pub fn build(
    request: &ScenesRequest, read: ThumbReader, store_path: &Path, cull_path: Option<&Path>, workers: usize, cancel: &AtomicBool,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<Option<ScenesBuild>, String> {
    let started = std::time::Instant::now();
    let items = &request.items;
    let total = items.len();
    let step = |done: usize| progress(done, total);
    let Some(hashed) = scenes::hash_all(items, read, store_path, workers, cancel, &step) else { return Ok(None) };
    let mut folders: HashMap<&Path, u32> = HashMap::new();
    let frames: Vec<Frame> = items
        .iter()
        .zip(hashed.hashes)
        .map(|(item, hash)| {
            let dir = Path::new(&item.path).parent().unwrap_or_else(|| Path::new(""));
            let next = folders.len() as u32;
            let folder = *folders.entry(dir).or_insert(next);
            Frame { t_ms: scenes::parse_taken(&item.taken_at), name: item.name.clone(), hash, folder }
        })
        .collect();
    // user splits and merges, by the frame's identity
    let index: HashMap<(&str, i64), usize> = items.iter().enumerate().map(|(i, item)| ((item.path.as_str(), item.version), i)).collect();
    let overrides: Vec<Override> = Store::open(store_path)
        .and_then(|s| s.overrides())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(path, version, action)| {
            let i = *index.get(&(path.as_str(), version))?;
            match action.as_str() {
                "split" => Some(Override::Split(i)),
                "merge" => Some(Override::Merge(i)),
                _ => None,
            }
        })
        .collect();
    let grouped: Vec<Block> = scenes::group(&frames, &request.params.unwrap_or_default(), &overrides);
    let mut unhashed_total = 0u32;
    let blocks: Vec<SceneBlock> = grouped
        .into_iter()
        .map(|block| {
            let times: Vec<i64> = block.scenes.iter().flatten().filter_map(|&i| frames[i].t_ms).collect();
            let scenes = block
                .scenes
                .iter()
                .map(|scene| {
                    let unhashed = scene.iter().filter(|&&i| frames[i].hash.is_none()).count() as u32;
                    unhashed_total += unhashed;
                    SceneRec { frames: scene.iter().map(|&i| i as u32).collect(), unhashed }
                })
                .collect();
            let approx = block.scenes.iter().flatten().any(|&i| items[i].from_file && frames[i].t_ms.is_some());
            SceneBlock { start_ms: times.iter().min().copied(), end_ms: times.iter().max().copied(), no_time: block.no_time, approx, scenes }
        })
        .collect();
    let analyses = match cull_path.and_then(|p| CullStore::open(p).ok()) {
        Some(store) => {
            let keys: Vec<(&str, i64)> = items.iter().map(|i| (i.path.as_str(), i.version)).collect();
            store.analyses(&keys)
        }
        None => vec![None; total],
    };
    let quality = analyses.iter().map(|a| a.as_ref().map(|a| a.quality_score)).collect();
    let regions = analyses.iter().map(|a| a.as_ref().and_then(|a| a.sharp_region)).collect();
    let network = items.first().is_some_and(|i| crate::volumes::on_network(Path::new(&i.path)));
    Ok(Some(ScenesBuild {
        blocks,
        quality,
        regions,
        network,
        hashed: (total as u32).saturating_sub(unhashed_total.min(total as u32)),
        unhashed: unhashed_total,
        reused: hashed.reused as u32,
        millis: started.elapsed().as_millis() as u32,
    }))
}

fn emit(app: &AppHandle, job: u64, stage: &str, done: usize, total: usize) {
    let _ = app.emit("scenes-progress", json!({ "job": job, "stage": stage, "done": done, "total": total }));
}

fn thumb_reader(thumbs: Arc<DiskCache>) -> impl Fn(&SceneItem) -> Option<Vec<u8>> + Sync {
    move |item| thumbs.read_known(Path::new(&item.path), item.version, scenes::THUMB_SIZE)
}

/// Builds the scenes of a folder's photos: groups them by capture time and look
/// (from cached thumbnails only), and reports the scores already measured. `job` is
/// the page's number for it, for `scenes_cancel` and the `scenes-progress` events.
/// A cancelled build fails with "cancelled".
#[tauri::command]
#[specta::specta]
pub(crate) async fn scenes_build(app: AppHandle, caches: State<'_, Caches>, job: u64, request: ScenesRequest) -> Result<ScenesBuild, String> {
    let (cancel, _entry) = register(job);
    let thumbs = caches.thumbs.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let workers = (crate::engine::thumb_threads(&state.runtime) as usize).clamp(2, MAX_WORKERS);
        let (store, cull) = (scenes_path(&state), cull_cmds::store_path(&state));
        let throttle = Throttle::new();
        let read = thumb_reader(thumbs);
        let total = request.items.len();
        emit(&app, job, "reading", 0, total);
        let progress = |done: usize, total: usize| {
            if throttle.due(done, total) {
                emit(&app, job, "reading", done, total);
            }
        };
        build(&request, &read, &store, Some(&cull), workers, &cancel, &progress)?.ok_or_else(|| "cancelled".to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Stops a running `scenes_build` or `scenes_score`.
#[tauri::command]
#[specta::specta]
pub(crate) fn scenes_cancel(job: u64) {
    if let Some(flag) = jobs().lock().unwrap_or_else(|e| e.into_inner()).get(&job) {
        flag.store(true, Ordering::SeqCst);
    }
}

/// The frames Scenes accepted (the page shows the green label as accepted only for these).
#[tauri::command]
#[specta::specta]
pub(crate) async fn scenes_accepted(state: State<'_, AppState>) -> Result<Vec<SceneAccepted>, String> {
    let path = scenes_path(&state);
    tauri::async_runtime::spawn_blocking(move || Store::open(&path)?.accepted_all()).await.map_err(|e| e.to_string())?
}

/// Remembers, or forgets, that Scenes accepted a frame.
#[tauri::command]
#[specta::specta]
pub(crate) async fn scenes_set_accepted(state: State<'_, AppState>, path: String, version: Option<i64>, accepted: bool) -> Result<(), String> {
    let store = scenes_path(&state);
    tauri::async_runtime::spawn_blocking(move || Store::open(&store)?.set_accepted(&path, version.unwrap_or(0), accepted))
        .await
        .map_err(|e| e.to_string())?
}

/// Scores frames the cull store has not measured, from the engine's small analysis
/// preview of each (the same measurement as Auto Cull, kept in the same cache, so
/// either finds the other's). Reports every frame's score; a frame that could not
/// be read has an `error`. Stopped by `scenes_cancel`, keeping what it measured.
#[tauri::command]
#[specta::specta]
pub(crate) async fn scenes_score(app: AppHandle, job: u64, items: Vec<CullTarget>) -> Result<Vec<SceneScore>, String> {
    let (cancel, _entry) = register(job);
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let path = cull_cmds::store_path(&state);
        let total = items.len();
        let next = AtomicUsize::new(0);
        let done = AtomicUsize::new(0);
        let throttle = Throttle::new();
        let out: Mutex<Vec<(usize, SceneScore)>> = Mutex::new(Vec::new());
        // the engine renders each analysis preview on its own thread, so two workers keep it busy
        std::thread::scope(|scope| {
            for _ in 0..2.min(total.max(1)) {
                scope.spawn(|| {
                    let store = CullStore::open(&path).ok();
                    loop {
                        let i = next.fetch_add(1, Ordering::SeqCst);
                        if i >= total || cancel.load(Ordering::SeqCst) {
                            return;
                        }
                        let target = &items[i];
                        let outcome = (|| {
                            let (size, modified) = crate::cull_apply::file_identity(Path::new(&target.path)).ok_or("the file is missing or unreadable")?;
                            let analysed = match &store {
                                Some(store) => cull_cmds::analyse_target(&app, store, target, size, modified)?,
                                None => return Err("the cull store could not be opened".to_string()),
                            };
                            Ok::<_, String>((analysed.result.quality_score, analysed.result.sharp_region))
                        })();
                        let (quality, sharp_region, error) = match outcome {
                            Ok((q, region)) => (Some(q), region, None),
                            Err(e) => (None, None, Some(e)),
                        };
                        let finished = done.fetch_add(1, Ordering::SeqCst) + 1;
                        if throttle.due(finished, total) {
                            emit(&app, job, "scoring", finished, total);
                        }
                        out.lock().unwrap_or_else(|e| e.into_inner()).push((i, SceneScore { path: target.path.clone(), version: target.version, quality, sharp_region, error }));
                    }
                });
            }
        });
        let mut all = out.into_inner().unwrap_or_else(|e| e.into_inner());
        all.sort_by_key(|(i, _)| *i);
        Ok(all.into_iter().map(|(_, s)| s).collect())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Remembers a split or a merge the user made, so every later grouping replays it. `action` is
/// "split" (a scene starts at this frame) or "merge" (the frame's scene joins the next scene of its
/// block). The page builds the scenes again afterwards.
#[tauri::command]
#[specta::specta]
pub(crate) async fn scenes_override(state: State<'_, AppState>, path: String, version: Option<i64>, action: String) -> Result<(), String> {
    if action != "split" && action != "merge" {
        return Err(format!("unknown scene action {action}"));
    }
    let store = scenes_path(&state);
    tauri::async_runtime::spawn_blocking(move || Store::open(&store)?.override_put(&path, version.unwrap_or(0), &action))
        .await
        .map_err(|e| e.to_string())?
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct RulesRequest {
    pub scenes: Vec<RuleScene>,
    pub rules: Rules,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct RulesPreview {
    pub counts: RuleCounts,
    /// every frame's verdict, in the request's scene and frame order
    pub verdicts: Vec<Vec<Verdict>>,
}

/// What the keep rules would do to the scenes, without writing anything.
#[tauri::command]
#[specta::specta]
pub(crate) async fn scenes_rules_preview(request: RulesRequest) -> Result<RulesPreview, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let verdicts = scenes_rules::evaluate(&request.scenes, &request.rules);
        RulesPreview { counts: scenes_rules::counts(&verdicts), verdicts }
    })
    .await
    .map_err(|e| e.to_string())
}

/// Records what the keep rules would do as a planned cull session (the same shape as Auto Cull's), changing no
/// photo: `cull_apply` writes it and `cull_undo` takes it back. Frames whose file is gone are listed in the
/// session's failed photos. Nothing is recorded when the rules leave nothing to do.
#[tauri::command]
#[specta::specta]
pub(crate) async fn scenes_rules_plan(app: AppHandle, label: String, request: RulesRequest) -> Result<CullPlan, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let verdicts = scenes_rules::evaluate(&request.scenes, &request.rules);
        let state = app.state::<AppState>();
        let mut store = CullStore::open(&cull_cmds::store_path(&state))?;
        let keys: Vec<(&str, i64)> = request.scenes.iter().flat_map(|s| &s.frames).map(|f| (f.path.as_str(), f.version)).collect();
        let mut cached: HashMap<(String, i64), crate::cull::CullAnalysis> = HashMap::new();
        for ((path, version), analysis) in keys.iter().zip(store.analyses(&keys)) {
            if let Some(a) = analysis {
                cached.insert((path.to_string(), *version), a);
            }
        }
        let known = |frame: &scenes_rules::RuleFrame| {
            let (size, modified_ms) = crate::cull_apply::file_identity(Path::new(&frame.path))?;
            Some(Known { analysis: cached.get(&(frame.path.clone(), frame.version)).cloned(), size, modified_ms })
        };
        let (items, failed) = scenes_rules::plan_items(&request.scenes, &verdicts, &known);
        if items.is_empty() {
            return Err("the rules leave nothing to keep or reject".to_string());
        }
        let failed: Vec<CullFailure> = failed.into_iter().map(|(path, error)| CullFailure { path, error }).collect();
        let (session, decisions) = store.create_session(&label, &crate::cull_plan::CullSettings::default(), items, failed)?;
        Ok(CullPlan { session, decisions })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenes::tests::{at, jpeg};

    fn item(name: &str, hms: &str) -> SceneItem {
        SceneItem { path: format!("/fake/{name}"), version: 0, name: name.into(), taken_at: format!("2026:03:05 {hms}"), from_file: false }
    }

    /// thumbnails from a table of seeds: the item's name picks its picture
    fn reader(seeds: HashMap<String, u32>) -> impl Fn(&SceneItem) -> Option<Vec<u8>> + Sync {
        move |item| seeds.get(&item.name).map(|&s| jpeg(s))
    }

    #[test]
    fn a_build_groups_by_time_and_look_and_a_second_build_decodes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("scenes.sqlite");
        let items = vec![item("a", "07:10:00"), item("b", "07:10:02"), item("c", "07:10:04"), item("d", "07:40:00"), item("e", "07:40:01")];
        let seeds: HashMap<String, u32> = [("a", 1), ("b", 1), ("c", 2), ("d", 3)].iter().map(|(n, s)| (n.to_string(), *s)).collect();
        let request = ScenesRequest { items, params: None };
        let cancel = AtomicBool::new(false);
        let first = build(&request, &reader(seeds.clone()), &store, None, 2, &cancel, &|_, _| {}).unwrap().unwrap();
        assert_eq!(first.blocks.len(), 2);
        assert_eq!(first.blocks[0].scenes.iter().map(|s| s.frames.clone()).collect::<Vec<_>>(), vec![vec![0, 1], vec![2]]);
        // e has no thumbnail: grouped by time with d, and counted
        assert_eq!(first.blocks[1].scenes[0].frames, vec![3, 4]);
        assert_eq!(first.blocks[1].scenes[0].unhashed, 1);
        assert_eq!((first.hashed, first.unhashed, first.reused), (4, 1, 0));
        assert_eq!(first.blocks[0].start_ms, Some(at("07:10:00")));
        let second = build(&request, &reader(seeds), &store, None, 2, &cancel, &|_, _| {}).unwrap().unwrap();
        assert_eq!(second.reused, 4);
        assert_eq!(second.blocks, first.blocks);
    }

    #[test]
    fn stored_overrides_are_replayed_by_frame_identity() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("scenes.sqlite");
        let items = vec![item("a", "07:10:00"), item("b", "07:10:02"), item("c", "07:10:04")];
        Store::open(&store).unwrap().override_put("/fake/c", 0, "split").unwrap();
        let seeds: HashMap<String, u32> = [("a", 1), ("b", 1), ("c", 1)].iter().map(|(n, s)| (n.to_string(), *s)).collect();
        let request = ScenesRequest { items, params: None };
        let got = build(&request, &reader(seeds), &store, None, 1, &AtomicBool::new(false), &|_, _| {}).unwrap().unwrap();
        assert_eq!(got.blocks[0].scenes.len(), 2);
    }

    #[test]
    fn a_split_then_a_merge_replays_in_the_order_they_were_made() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("scenes.sqlite");
        let items = vec![item("a", "07:10:00"), item("b", "07:10:02"), item("c", "07:10:04"), item("d", "07:10:06")];
        let seeds: HashMap<String, u32> = ["a", "b", "c", "d"].iter().map(|n| (n.to_string(), 1)).collect();
        let request = ScenesRequest { items, params: None };
        let groups = |store: &Path| {
            let got = build(&request, &reader(seeds.clone()), store, None, 1, &AtomicBool::new(false), &|_, _| {}).unwrap().unwrap();
            got.blocks[0].scenes.iter().map(|s| s.frames.clone()).collect::<Vec<_>>()
        };
        assert_eq!(groups(&store), vec![vec![0, 1, 2, 3]]);
        let overrides = Store::open(&store).unwrap();
        overrides.override_put("/fake/c", 0, "split").unwrap();
        assert_eq!(groups(&store), vec![vec![0, 1], vec![2, 3]]);
        // joining them again, made a moment later, undoes the split
        std::thread::sleep(std::time::Duration::from_millis(5));
        overrides.override_put("/fake/a", 0, "merge").unwrap();
        assert_eq!(groups(&store), vec![vec![0, 1, 2, 3]]);
    }

    #[test]
    fn a_cancelled_build_returns_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let request = ScenesRequest { items: vec![item("a", "07:10:00")], params: None };
        let got = build(&request, &reader(HashMap::new()), &dir.path().join("s.sqlite"), None, 1, &AtomicBool::new(true), &|_, _| {}).unwrap();
        assert!(got.is_none());
    }

    #[test]
    fn photos_with_a_file_time_are_marked_approximate_and_no_time_goes_last() {
        let dir = tempfile::tempdir().unwrap();
        let mut from_file = item("a", "07:10:00");
        from_file.from_file = true;
        let mut none = item("z", "07:10:00");
        none.taken_at.clear();
        let request = ScenesRequest { items: vec![none, from_file], params: None };
        let got = build(&request, &reader(HashMap::new()), &dir.path().join("s.sqlite"), None, 1, &AtomicBool::new(false), &|_, _| {}).unwrap().unwrap();
        assert!(got.blocks[0].approx && !got.blocks[0].no_time);
        assert!(got.blocks[1].no_time && got.blocks[1].start_ms.is_none());
    }

    #[test]
    fn scores_already_measured_come_with_the_build() {
        use crate::cull::CullAnalysis;
        let dir = tempfile::tempdir().unwrap();
        let cull_path = dir.path().join("cull.sqlite");
        let cull = CullStore::open(&cull_path).unwrap();
        let analysis = CullAnalysis {
            quality_score: 0.84, sharpness_metric: 1.0, center_focus_metric: 1.0, subject_focus_metric: None, subject_composition_metric: None,
            subject_edge_contact_ratio: None, exposure_metric: 1.0, width: 10, height: 10, blurry_region: None,
            sharp_region: Some(CullBlurRegion { x: 0.5, y: 0.25, width: 0.375, height: 0.375 }),
        };
        cull.cache_put(&CullTarget { path: "/fake/a".into(), version: 0 }, 1, 1, "h", &analysis).unwrap();
        let request = ScenesRequest { items: vec![item("a", "07:10:00"), item("b", "07:10:01")], params: None };
        let got = build(&request, &reader(HashMap::new()), &dir.path().join("s.sqlite"), Some(&cull_path), 1, &AtomicBool::new(false), &|_, _| {}).unwrap().unwrap();
        assert_eq!(got.quality, vec![Some(0.84), None]);
        assert_eq!(got.regions[0].map(|r| (r.x, r.y)), Some((0.5, 0.25)));
        assert!(got.regions[1].is_none());
    }

    /// 20,000 photos with cached thumbnails: cold (every thumbnail decoded and hashed) and warm (hashes
    /// reused). Slow in a debug build, so it runs on request: cargo test --release scenes_cmds -- --ignored --nocapture
    #[test]
    #[ignore]
    fn twenty_thousand_photos_build_within_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("scenes.sqlite");
        let pictures: Vec<Vec<u8>> = (0..40).map(jpeg).collect();
        let items: Vec<SceneItem> = (0..20_000usize)
            .map(|i| {
                let t = at("00:00:00") + (i as i64 / 8) * 100_000 + (i as i64 / 400) * 3_000_000 + (i as i64 % 8) * 400;
                let when = chrono::DateTime::from_timestamp_millis(t).unwrap().format("%Y:%m:%d %H:%M:%S%.3f").to_string();
                SceneItem { path: format!("/fake/{i}.jpg"), version: 0, name: format!("{i}.jpg"), taken_at: when, from_file: false }
            })
            .collect();
        let request = ScenesRequest { items, params: None };
        let read = |item: &SceneItem| item.name.trim_end_matches(".jpg").parse::<usize>().ok().map(|i| pictures[(i / 8) % 40].clone());
        let cancel = AtomicBool::new(false);
        let cold = build(&request, &read, &store, None, 4, &cancel, &|_, _| {}).unwrap().unwrap();
        let warm = build(&request, &read, &store, None, 4, &cancel, &|_, _| {}).unwrap().unwrap();
        eprintln!("SCENES_20K cold_ms={} warm_ms={} blocks={} scenes={}", cold.millis, warm.millis, cold.blocks.len(), cold.blocks.iter().map(|b| b.scenes.len()).sum::<usize>());
        assert_eq!(warm.reused, 20_000);
        assert!(cold.millis < 10_000, "cold build {} ms", cold.millis);
        assert!(warm.millis < 3_000, "warm build {} ms", warm.millis);
    }
}
