// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The Statistics commands. They read the search cache (never the engine) into
//! an in-memory `stats::Index`, rebuilt when the cache changed and the last
//! build is a few seconds old, and answer every request from that index. The
//! cache's mutex is held for one page of rows at a time, so the indexer is not
//! starved; parsing and aggregation run outside it, off the UI thread.
use crate::{
    search_cache::SearchCache,
    stats::{self, Index, Library, Parsed, Photos, Query, RawRow},
    typed_library::LibraryImage,
    typed_stats::*,
    AppState,
};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, Emitter, Manager};

const PAGE: u32 = 2048;
// a changed cache rebuilds the index at most this often, so indexing a folder
// does not turn every poll into a full read
const MIN_AGE: Duration = Duration::from_secs(5);
const GRID_CAP: usize = 20_000;

pub struct Built {
    pub index: Index,
    revision: String,
    at: Instant,
    gen: u64,
}

static SLOT: Mutex<Option<Arc<Built>>> = Mutex::new(None);
static BUILD: Mutex<()> = Mutex::new(());
static GEN: AtomicU64 = AtomicU64::new(0);
static WATCHING: AtomicBool = AtomicBool::new(false);

/// Read every original photo of the cache into an index: pages under a short
/// lock each, parsed by a few worker threads.
pub fn load(cache: &SearchCache) -> Result<Index, String> {
    let workers = std::thread::available_parallelism().map_or(2, |n| n.get()).clamp(1, 8);
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<RawRow>>(workers * 2);
    let rx = Mutex::new(rx);
    let result = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..workers)
            .map(|_| {
                scope.spawn(|| {
                    let mut parsed: Vec<(i64, Parsed)> = Vec::new();
                    let mut pending: Vec<String> = Vec::new();
                    loop {
                        let page = match rx.lock().unwrap().recv() {
                            Ok(p) => p,
                            Err(_) => break,
                        };
                        for row in page {
                            match stats::parse_row(&row) {
                                Some(p) => parsed.push((row.variant_id, p)),
                                None => pending.push(row.path),
                            }
                        }
                    }
                    (parsed, pending)
                })
            })
            .collect();
        let mut after = 0;
        let mut read = Ok(());
        loop {
            match cache.stats_page(after, PAGE) {
                Ok(page) if page.is_empty() => break,
                Ok(page) => {
                    after = page[page.len() - 1].variant_id;
                    if tx.send(page).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    read = Err(e);
                    break;
                }
            }
        }
        drop(tx);
        let mut parsed = Vec::new();
        let mut pending = Vec::new();
        for h in handles {
            let (p, u) = h.join().map_err(|_| "Statistics worker failed".to_string())?;
            parsed.extend(p);
            pending.extend(u);
        }
        read.map(|_| (parsed, pending))
    })?;
    let (mut parsed, pending) = result;
    // id order: the same photo order whatever thread parsed it
    parsed.sort_unstable_by_key(|(id, _)| *id);
    Ok(Index::build(parsed.into_iter().map(|(_, p)| p).collect(), &pending))
}

/// The index for the cache's current state: the last one while it is fresh enough.
pub fn current(cache: &SearchCache) -> Result<Arc<Built>, String> {
    let fresh = |b: &Arc<Built>| b.at.elapsed() < MIN_AGE || cache.revision().is_ok_and(|r| r == b.revision);
    if let Some(b) = SLOT.lock().map_err(|_| "Statistics lock poisoned")?.as_ref().filter(|b| fresh(b)) {
        return Ok(b.clone());
    }
    let _build = BUILD.lock().map_err(|_| "Statistics lock poisoned")?;
    if let Some(b) = SLOT.lock().map_err(|_| "Statistics lock poisoned")?.as_ref().filter(|b| fresh(b)) {
        return Ok(b.clone());
    }
    // read the revision first: a write during the build makes the next call rebuild
    let revision = cache.revision()?;
    let index = load(cache)?;
    let built = Arc::new(Built { index, revision, at: Instant::now(), gen: GEN.fetch_add(1, Ordering::AcqRel) + 1 });
    *SLOT.lock().map_err(|_| "Statistics lock poisoned")? = Some(built.clone());
    Ok(built)
}

fn library(runtime: &Path) -> Library {
    let config = crate::settings::read_key(runtime, "immage.library")
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .unwrap_or_default();
    let paths = |key: &str| {
        config[key]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v.as_str().map(String::from))
            .collect::<Vec<_>>()
    };
    Library { roots: paths("folders"), hidden: paths("hidden") }
}

fn watch(app: &AppHandle, cache: Arc<SearchCache>) {
    if WATCHING.swap(true, Ordering::AcqRel) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let mut last = cache.revision().unwrap_or_default();
        loop {
            std::thread::sleep(Duration::from_secs(2));
            if let Ok(now) = cache.revision() {
                if now != last {
                    last = now;
                    let _ = app.emit("stats_dirty", ());
                }
            }
        }
    });
}

fn coverage(app: &AppHandle, built: Option<&Built>, scoped: Option<&stats::Scoped>, lib: &Library) -> Coverage {
    let state = app.state::<AppState>();
    let paused = state.analysis_queue.status().paused;
    let pending = state.search_analysis.status(paused).pending.unwrap_or(0);
    let (indexed, total, per_folder) = match (built, scoped) {
        (Some(b), Some(s)) => stats::scope_counts(&b.index, s),
        _ => state
            .search_cache
            .as_ref()
            .and_then(|c| c.counts().ok())
            .map_or((0, 0, vec![]), |c| (c.metadata as u32, c.variants as u32, vec![])),
    };
    let mut pending_roots: Vec<PendingRoot> = match built {
        Some(b) => lib
            .roots
            .iter()
            .map(|root| PendingRoot {
                folder: root.clone(),
                pending: per_folder
                    .iter()
                    .filter(|(f, _)| stats::folder_under(&b.index, *f, root))
                    .map(|(_, n)| n)
                    .sum(),
            })
            .filter(|r| r.pending > 0)
            .collect(),
        None => vec![],
    };
    pending_roots.sort_by(|a, b| b.pending.cmp(&a.pending).then(a.folder.cmp(&b.folder)));
    pending_roots.truncate(5);
    let unindexed = total.saturating_sub(indexed);
    let state = if lib.roots.is_empty() {
        CoverageState::NoRoots
    } else if indexed == 0 && unindexed == 0 {
        CoverageState::Empty
    } else if paused && (pending > 0 || unindexed > 0) {
        CoverageState::Paused
    } else if pending > 0 || unindexed > 0 {
        CoverageState::Indexing
    } else {
        CoverageState::Ready
    };
    Coverage {
        indexed,
        total: (!lib.roots.is_empty()).then_some(total),
        pending,
        roots: lib.roots.len() as u32,
        pending_roots,
        paused,
        max_age_sec: built.map_or(0, |b| b.at.elapsed().as_secs() as u32),
        state,
    }
}

struct Ctx {
    built: Arc<Built>,
    lib: Library,
    tagged: crate::engine::Tagged,
}
fn context(app: &AppHandle) -> Result<Ctx, String> {
    let state = app.state::<AppState>();
    let cache = state.search_cache.clone().ok_or("Local metadata cache is unavailable")?;
    watch(app, cache.clone());
    Ok(Ctx { built: current(&cache)?, lib: library(&state.runtime), tagged: state.tagged.clone() })
}

fn hash_request(req: &StatsRequest) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut r = req.clone();
    r.etag = None;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(&r).unwrap_or_default().hash(&mut h);
    h.finish()
}

#[derive(Serialize, Deserialize)]
struct Cursor {
    scope: StatsScope,
    filter: StatsFilter,
    offset: usize,
    gen: u64,
}

fn answer(app: &AppHandle, req: StatsRequest) -> Result<StatsReply, String> {
    let cx = context(app)?;
    let etag = format!("{}-{:x}", cx.built.gen, hash_request(&req));
    if req.etag.as_deref() == Some(etag.as_str()) {
        return Ok(StatsReply::NotModified(NotModified { not_modified: true }));
    }
    let scoped = stats::resolve_scope(&cx.built.index, &req.scope, &cx.lib)?;
    let q = Query::new(&cx.built.index, &scoped, &req.filter);
    let id = |path: &str| cx.tagged.cached_photo(path, 0).to_string();
    let photos = Photos { index: &cx.built.index, id: &id };
    let s = stats::snapshot(&q, &photos, &stats::Wanted { sections: &req.sections, places: req.places.clone() });
    let mut top = s.top;
    if let Some(t) = top.as_mut() {
        if (t.items.len() as u32) < t.total {
            t.page.next = encode(&Cursor { scope: req.scope.clone(), filter: req.filter.clone(), offset: t.items.len(), gen: cx.built.gen })?.into();
        }
    }
    Ok(StatsReply::Full(Box::new(StatsResponse {
        etag,
        generated_at: chrono::Utc::now().to_rfc3339(),
        coverage: coverage(app, Some(&cx.built), Some(&scoped), &cx.lib),
        defs: stats::defs(),
        overview: s.overview,
        cameras: s.cameras,
        lenses: s.lenses,
        exposure: s.exposure,
        time: s.time,
        places: s.places,
        top,
    })))
}
fn encode(c: &Cursor) -> Result<String, String> {
    serde_json::to_string(c).map_err(|e| e.to_string())
}

/// The statistics of a scope under a filter. `sections` picks the blocks; an
/// etag still current answers `{ notModified: true }`.
#[tauri::command]
#[specta::specta]
pub async fn stats_query(app: AppHandle, req: StatsRequest) -> Result<StatsReply, String> {
    tauri::async_runtime::spawn_blocking(move || answer(&app, req))
        .await
        .map_err(|e| e.to_string())?
}

/// One map cell's side panel.
#[tauri::command]
#[specta::specta]
pub async fn stats_place(app: AppHandle, key: String, filter: StatsFilter, scope: StatsScope) -> Result<PlaceDetail, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let cx = context(&app)?;
        let scoped = stats::resolve_scope(&cx.built.index, &scope, &cx.lib)?;
        let q = Query::new(&cx.built.index, &scoped, &filter);
        let id = |path: &str| cx.tagged.cached_photo(path, 0).to_string();
        stats::place_detail(&q, &Photos { index: &cx.built.index, id: &id }, &key)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The next page of top images from a cursor `stats_query` or this command gave.
#[tauri::command]
#[specta::specta]
pub async fn stats_top_page(app: AppHandle, cursor: String) -> Result<TopPageReply, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let c: Cursor = serde_json::from_str(&cursor).map_err(|_| "Invalid statistics cursor")?;
        let cx = context(&app)?;
        if cx.built.gen != c.gen {
            return Err("stats-changed: the statistics changed; reload them".into());
        }
        let scoped = stats::resolve_scope(&cx.built.index, &c.scope, &cx.lib)?;
        let q = Query::new(&cx.built.index, &scoped, &c.filter);
        let (total, rows) = stats::top_rows(&q, c.offset, stats::TOP_PAGE);
        let id = |path: &str| cx.tagged.cached_photo(path, 0).to_string();
        let photos = Photos { index: &cx.built.index, id: &id };
        let offset = c.offset + rows.len();
        let next = if (offset as u32) < total {
            Some(encode(&Cursor { offset, ..c })?)
        } else {
            None
        };
        Ok(TopPageReply { items: rows.iter().map(|i| photos.get(*i)).collect(), next })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Handles of every photo the scope and filter match, for "Open N in grid".
#[tauri::command]
#[specta::specta]
pub async fn stats_open_in_grid(app: AppHandle, scope: StatsScope, filter: StatsFilter) -> Result<GridIds, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let cx = context(&app)?;
        let scoped = stats::resolve_scope(&cx.built.index, &scope, &cx.lib)?;
        let q = Query::new(&cx.built.index, &scoped, &filter);
        let (rows, complete) = stats::grid_rows(&q, GRID_CAP);
        let ids = rows
            .iter()
            .map(|i| cx.tagged.cached_photo(&cx.built.index.rows[*i as usize].path, 0).to_string())
            .collect();
        Ok(GridIds { ids, complete })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The grid rows for handles `stats_open_in_grid` gave: the same `LibraryImage` rows local search
/// returns, built from the cache's stored metadata, so the grid and the photo-open path treat them
/// alike (the handle stays the row's `imgid`; opening it imports the photo and reads its sidecar).
/// Handles the session does not know, and photos the cache has no metadata for, are left out.
pub fn grid_entries(
    cache: &SearchCache,
    ids: &[String],
    resolve: &dyn Fn(i64) -> Option<crate::library_ids::Photo>,
) -> Result<Vec<LibraryImage>, String> {
    if ids.len() > GRID_CAP {
        return Err(format!("Too many photos for the grid ({} > {GRID_CAP})", ids.len()));
    }
    let known: Vec<(i64, crate::library_ids::Photo)> = ids
        .iter()
        .filter_map(|id| id.parse::<i64>().ok())
        .filter_map(|id| resolve(id).filter(|p| p.version == 0).map(|p| (id, p)))
        .collect();
    let paths: Vec<&str> = known.iter().map(|(_, p)| p.path.as_str()).collect();
    let metadata = cache.original_metadata(&paths)?;
    Ok(known
        .iter()
        .zip(metadata)
        .filter_map(|((id, photo), raw)| {
            let mut image = crate::local_search::cached_image(&raw?, &photo.path, 0)?;
            image.imgid = *id;
            Some(image)
        })
        .collect())
}

/// The grid's entries for handles from `stats_open_in_grid` (at most 20,000).
#[tauri::command]
#[specta::specta]
pub async fn stats_grid_entries(app: AppHandle, ids: Vec<String>) -> Result<Vec<LibraryImage>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let cache = state.search_cache.clone().ok_or("Local metadata cache is unavailable")?;
        let tagged = state.tagged.clone();
        grid_entries(&cache, &ids, &|id| tagged.known_photo(id))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Cheap and polled while indexing: library-wide counts without building the index.
#[tauri::command]
#[specta::specta]
pub async fn stats_coverage(app: AppHandle) -> Result<Coverage, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        let lib = library(&state.runtime);
        let built = SLOT.lock().map_err(|_| "Statistics lock poisoned")?.clone();
        let scoped = built.as_ref().and_then(|b| stats::resolve_scope(&b.index, &StatsScope::Library, &lib).ok());
        Ok(coverage(&app, built.as_deref(), scoped.as_ref(), &lib))
    })
    .await
    .map_err(|e| e.to_string())?
}
