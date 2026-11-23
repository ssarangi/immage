// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The auto-cull commands: plan a cull over some photos (analysis, grouping,
//! keepers, rejects, highlights), review and override it, apply it to the
//! library and undo it. The analysis reads the engine's small analysis
//! preview of each photo (never a full RAW decode), on a few worker threads,
//! and keeps what it measured in cull.sqlite (cull_store.rs) so a second run
//! over the same files is quick. The numbers and the planner are cull.rs and
//! cull_plan.rs; writing to the library is cull_apply.rs.
//!
//! Progress reaches the page as the `cull-progress` event:
//! `{ job, stage: "analyzing" | "grouping", done, total, file }`.
use crate::cull::{self, Analysed};
use crate::cull_apply::{self, CullApplyOptions, CullApplyResult, CullChange, Library};
use crate::cull_plan::{self, CullSettings, CullStatus};
use crate::cull_store::{CullDecision, CullFailure, CullSession, CullTarget, Item, CullLibraryState, Store};
use crate::frame_output::FrameOutput;
use crate::AppState;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map};
use specta::Type;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use tauri::{AppHandle, Emitter, Manager, State};

/// The most photos analysed at once: the engine renders each analysis
/// preview itself, so more only queues behind it.
const MAX_WORKERS: usize = 4;

/// Running plans' cancel flags, by the page's job number.
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

/// A photo to cull, and the files that take its decision with it (a RAW's
/// JPEG when the page stacks pairs). Only the first is analysed.
#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct CullScopeItem {
    pub path: String,
    #[serde(default)]
    pub version: i64,
    #[serde(default)]
    pub backing: Vec<CullTarget>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct CullScope {
    /// shown in the History list: a folder's name, "Selection"
    pub label: String,
    pub items: Vec<CullScopeItem>,
}

/// A session with its decisions.
#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct CullPlan {
    pub session: CullSession,
    pub decisions: Vec<CullDecision>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Type)]
#[serde(rename_all = "camelCase")]
pub struct CullOverrideResult {
    pub decision: CullDecision,
    /// library entries rewritten, when the session was already applied
    pub changes: Vec<CullChange>,
}

pub(crate) fn store_path(state: &AppState) -> PathBuf {
    state.runtime.join("cull.sqlite")
}

fn progress(app: &AppHandle, job: u64, stage: &str, done: usize, total: usize, file: &str) {
    let _ = app.emit("cull-progress", json!({ "job": job, "stage": stage, "done": done, "total": total, "file": file }));
}

fn file_name(path: &str) -> String {
    Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path).to_string()
}

/// the analysis preview of one photo as pixels
fn load_preview(app: &AppHandle, target: &CullTarget) -> Result<image::RgbImage, String> {
    let state = app.state::<AppState>();
    state.tagged.mark_active();
    let imgid = state.tagged.cached_photo(&target.path, target.version);
    let tag = state.tagged.tag();
    let output = state._frame_output.clone();
    let out = output.path(&format!("cull-{tag}.jpg"));
    let got = state
        .tagged
        .request(tag, json!({ "cmd": "analysis_preview", "imgid": imgid, "subject": "original", "out": out }))
        .and_then(|_| FrameOutput::read(&out).map_err(|e| e.to_string()));
    output.remove(&out);
    decode_jpeg(&got?)
}

fn decode_jpeg(bytes: &[u8]) -> Result<image::RgbImage, String> {
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes));
    reader.set_format(image::ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(128 * 1024 * 1024);
    reader.limits(limits);
    Ok(reader.decode().map_err(|e| e.to_string())?.to_rgb8())
}

pub(crate) fn analyse_target(app: &AppHandle, store: &Store, target: &CullTarget, size: i64, modified_ms: i64) -> Result<Analysed, String> {
    if let Some((result, hash)) = store.cached(target, size, modified_ms) {
        if let Some(hash) = cull::hash_from_string(&hash) {
            return Ok(Analysed { hash, result });
        }
    }
    let analysed = cull::analyze(&load_preview(app, target)?);
    // a cache that cannot be written only costs the next run its speed
    let _ = store.cache_put(target, size, modified_ms, &cull::hash_to_string(&analysed.hash), &analysed.result);
    Ok(analysed)
}

struct Measured {
    item: CullScopeItem,
    size: i64,
    modified_ms: i64,
    analysed: Analysed,
}

/// Analyses every item on a few threads, in the scope's order. Stops early,
/// with None, when `cancel` is set.
fn analyse_all(app: &AppHandle, job: u64, items: &[CullScopeItem], cancel: &AtomicBool) -> Option<(Vec<Measured>, Vec<CullFailure>)> {
    let state = app.state::<AppState>();
    let path = store_path(&state);
    let workers = (crate::engine::thumb_threads(&state.runtime) as usize).clamp(1, MAX_WORKERS).min(items.len().max(1));
    let next = AtomicUsize::new(0);
    let done = AtomicUsize::new(0);
    let results: Mutex<Vec<(usize, Result<Measured, CullFailure>)>> = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                // a connection of its own: SQLite connections are not shared
                let store = Store::open(&path).ok();
                loop {
                    let index = next.fetch_add(1, Ordering::SeqCst);
                    if index >= items.len() || cancel.load(Ordering::SeqCst) {
                        return;
                    }
                    let item = &items[index];
                    let target = CullTarget { path: item.path.clone(), version: item.version };
                    let outcome = (|| {
                        let (size, modified_ms) = cull_apply::file_identity(Path::new(&item.path)).ok_or("the file is missing or unreadable")?;
                        let analysed = match &store {
                            Some(store) => analyse_target(app, store, &target, size, modified_ms)?,
                            None => cull::analyze(&load_preview(app, &target)?),
                        };
                        Ok::<_, String>(Measured { item: item.clone(), size, modified_ms, analysed })
                    })();
                    let finished = done.fetch_add(1, Ordering::SeqCst) + 1;
                    progress(app, job, "analyzing", finished, items.len(), &file_name(&item.path));
                    results.lock().unwrap().push((index, outcome.map_err(|error| CullFailure { path: item.path.clone(), error })));
                }
            });
        }
    });
    if cancel.load(Ordering::SeqCst) {
        return None;
    }
    let mut all = results.into_inner().unwrap();
    all.sort_by_key(|(index, _)| *index);
    let (mut measured, mut failed) = (Vec::new(), Vec::new());
    for (_, outcome) in all {
        match outcome {
            Ok(m) => measured.push(m),
            Err(f) => failed.push(f),
        }
    }
    Some((measured, failed))
}

/// Plans a cull over `scope`: measures each photo, groups look-alikes,
/// decides keepers, rejects and highlights, and records it as a session
/// without changing any photo. `job` is the page's number for it, for
/// `cull_cancel` and the `cull-progress` events. A cancelled plan stores
/// nothing and fails with "cancelled".
#[tauri::command]
#[specta::specta]
pub(crate) async fn cull_plan(app: AppHandle, job: u64, scope: CullScope, settings: CullSettings) -> Result<CullPlan, String> {
    if scope.items.is_empty() {
        return Err("no photos to cull".into());
    }
    let cancel = Arc::new(AtomicBool::new(false));
    jobs().lock().unwrap().insert(job, cancel.clone());
    let _entry = JobEntry(job);
    tauri::async_runtime::spawn_blocking(move || {
        let total = scope.items.len();
        progress(&app, job, "analyzing", 0, total, "");
        let (measured, failed) = analyse_all(&app, job, &scope.items, &cancel).ok_or("cancelled")?;
        progress(&app, job, "grouping", total, total, "");
        let inputs: Vec<cull_plan::Input> = measured
            .iter()
            .map(|m| cull_plan::Input { path: m.item.path.clone(), analysis: m.analysed.result.clone(), hash: m.analysed.hash.clone() })
            .collect();
        let items: Vec<Item> = cull_plan::plan(&inputs, &settings)
            .into_iter()
            .zip(measured)
            .map(|(planned, m)| Item {
                target: CullTarget { path: m.item.path.clone(), version: m.item.version },
                backing: m.item.backing,
                size: m.size,
                modified_ms: m.modified_ms,
                planned,
            })
            .collect();
        if cancel.load(Ordering::SeqCst) {
            return Err("cancelled".to_string());
        }
        let state = app.state::<AppState>();
        let mut store = Store::open(&store_path(&state))?;
        let (session, decisions) = store.create_session(&scope.label, &settings, items, failed)?;
        Ok(CullPlan { session, decisions })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Stops a running `cull_plan`: the photo being measured finishes, nothing
/// is recorded.
#[tauri::command]
#[specta::specta]
pub(crate) fn cull_cancel(job: u64) {
    if let Some(flag) = jobs().lock().unwrap().get(&job) {
        flag.store(true, Ordering::SeqCst);
    }
}

/// Past sessions, newest first, without their decisions.
#[tauri::command]
#[specta::specta]
pub(crate) async fn cull_sessions(state: State<'_, AppState>) -> Result<Vec<CullSession>, String> {
    let path = store_path(&state);
    blocking(move || Store::open(&path)?.sessions()).await
}

/// One session with its decisions, to review it again.
#[tauri::command]
#[specta::specta]
pub(crate) async fn cull_session(state: State<'_, AppState>, session_id: i64) -> Result<Option<CullPlan>, String> {
    let path = store_path(&state);
    blocking(move || {
        let store = Store::open(&path)?;
        Ok(match store.session(session_id)? {
            Some(session) => Some(CullPlan { decisions: store.decisions(session_id)?, session }),
            None => None,
        })
    })
    .await
}

/// Forgets a session. Photos it already changed stay as they are; undo first
/// to take that back.
#[tauri::command]
#[specta::specta]
pub(crate) async fn cull_delete_session(state: State<'_, AppState>, session_id: i64) -> Result<bool, String> {
    let path = store_path(&state);
    blocking(move || Store::open(&path)?.delete_session(session_id)).await
}

/// Forgets every photo's cached measurements, so the next cull measures
/// them again.
#[tauri::command]
#[specta::specta]
pub(crate) async fn cull_cache_clear(state: State<'_, AppState>) -> Result<(), String> {
    let path = store_path(&state);
    blocking(move || Store::open(&path)?.cache_clear().map(|_| ())).await
}

/// The user's call on one photo: Keep or Reject, and with `makeBest` the
/// mark of its set's best frame. In a session already applied the library
/// follows at once (the photo goes back as it was, then is flagged again for
/// a reject); in a planned one only the plan changes.
#[tauri::command]
#[specta::specta]
pub(crate) async fn cull_override(app: AppHandle, decision_id: i64, status: CullStatus, make_best: Option<bool>) -> Result<CullOverrideResult, String> {
    blocking(move || {
        let path = store_path(&app.state::<AppState>());
        let mut store = Store::open(&path)?;
        let before = store.decision(decision_id)?.ok_or("no such decision")?;
        let session = store.session(before.session_id)?.ok_or("no such cull session")?;
        store.override_decision(decision_id, status, make_best.unwrap_or(false), "user")?;
        let changes = if session.state == crate::cull_store::CullSessionState::Applied {
            cull_apply::override_applied(&store, decision_id, &EngineLibrary { app: app.clone() })?
        } else {
            Vec::new()
        };
        let decision = store.decision(decision_id)?.ok_or("no such decision")?;
        Ok(CullOverrideResult { decision, changes })
    })
    .await
}

/// Writes a planned session to the library: rejects are flagged in place
/// (no file is moved or deleted) and, if asked, highlights get a rating and
/// a color label. Photos whose file changed since the analysis are skipped
/// and listed. A session applies once; `cull_undo` reverses it.
#[tauri::command]
#[specta::specta]
pub(crate) async fn cull_apply(app: AppHandle, session_id: i64, options: Option<CullApplyOptions>) -> Result<CullApplyResult, String> {
    blocking(move || {
        let store = Store::open(&store_path(&app.state::<AppState>()))?;
        let events = app.clone();
        let last = std::sync::Mutex::new(std::time::Instant::now() - std::time::Duration::from_secs(1));
        // the page shows "n of m" and treats a long silence as a stall, so it hears every 100 ms at most
        let report = move |done: usize, total: usize| {
            let mut at = last.lock().unwrap();
            if done == total || at.elapsed() >= std::time::Duration::from_millis(100) {
                *at = std::time::Instant::now();
                progress(&events, session_id as u64, "apply", done, total, "");
            }
        };
        cull_apply::apply_session_with(&store, session_id, &options.unwrap_or_default(), &EngineLibrary { app }, &report)
    })
    .await
}

/// Puts every photo an applied session changed back as it was. A photo that
/// has been changed since the cull is left alone and listed.
#[tauri::command]
#[specta::specta]
pub(crate) async fn cull_undo(app: AppHandle, session_id: i64) -> Result<CullApplyResult, String> {
    blocking(move || {
        let store = Store::open(&store_path(&app.state::<AppState>()))?;
        cull_apply::undo_session(&store, session_id, &EngineLibrary { app })
    })
    .await
}

async fn blocking<T: Send + 'static>(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work).await.map_err(|e| e.to_string())?
}

/// darktable's library, through the same `meta` and `meta_set` the page's own
/// rating, reject and label controls use, so changes reach the sidecars and
/// the search metadata. Called on a blocking thread.
struct EngineLibrary {
    app: AppHandle,
}

impl Library for EngineLibrary {
    fn read(&self, target: &CullTarget) -> Result<CullLibraryState, String> {
        let state = self.app.state::<AppState>();
        let imgid = state.tagged.cached_photo(&target.path, target.version);
        let reply = tauri::async_runtime::block_on(crate::library_cmds::meta(state, imgid))?;
        Ok(CullLibraryState { rating: reply.image.rating, rejected: reply.image.rejected, labels: reply.image.labels })
    }

    fn write(&self, target: &CullTarget, wanted: &CullLibraryState) -> Result<(), String> {
        let state = self.app.state::<AppState>();
        let imgid = state.tagged.cached_photo(&target.path, target.version);
        let mut fields = Map::new();
        if wanted.rejected {
            fields.insert("rejected".into(), json!(true));
        } else {
            // a rating also clears the reject flag
            fields.insert("rating".into(), json!(wanted.rating));
            fields.insert("rejected".into(), json!(false));
        }
        fields.insert("labels".into(), json!(wanted.labels));
        tauri::async_runtime::block_on(crate::library_cmds::meta_set(state, imgid, fields))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analysis_previews_decode_as_jpeg_only() {
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgb8(crate::cull::tests::sharp_scene(64, 48, 0))
            .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Jpeg)
            .unwrap();
        assert_eq!(decode_jpeg(&bytes).unwrap().dimensions(), (64, 48));
        assert!(decode_jpeg(b"not an image").is_err());
    }

    #[test]
    fn cancel_flags_follow_their_job() {
        let flag = Arc::new(AtomicBool::new(false));
        jobs().lock().unwrap().insert(9_000_001, flag.clone());
        {
            let _entry = JobEntry(9_000_001);
            cull_cancel(9_000_001);
            assert!(flag.load(Ordering::SeqCst));
        }
        assert!(!jobs().lock().unwrap().contains_key(&9_000_001));
        cull_cancel(9_000_001); // no such job: nothing happens
    }

    #[test]
    fn scope_items_default_their_version_and_backing() {
        let scope: CullScope = serde_json::from_str(r#"{"label":"Roll","items":[{"path":"/a.arw"}]}"#).unwrap();
        assert_eq!((scope.items[0].version, scope.items[0].backing.len()), (0, 0));
    }
}
