// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The Catalog commands for the page: sources, scans, status. The database is
//! opened on first use from the runtime directory and kept; a scan runs on its
//! own thread and reports through two events:
//!
//! * `catalog://scan-progress` with a `Progress` (at most about 4 a second per scan)
//! * `catalog://scan-finished` with a `ScanSummary` (never dropped)
//!
//! `jobs_active` answers the same thing for a page that missed an event.

use crate::catalog_db::{CatalogDb, FILE, SCHEMA_VERSION};
use crate::catalog_meta_job::{coverage, run_metadata, MetaConfig, MetaProgress};
use crate::catalog_thumb_job::{run_thumbnails, ThumbProgress};
use crate::catalog_thumbs::{self as thumbs, GcReport, Signals, Store, ThumbConfig};
use crate::catalog_scan::{run_scan, Progress, RealFs, ScanConfig, ScanControl, ScanOptions};
use crate::catalog_sources::{
    self as sources, availability, list_sources, system_uuid_of, AddError, AddOptions, Availability, MountTable, RelinkCheck,
    RemovePreview, Source, SourceStatus, SourceUpdate,
};
use crate::typed_catalog::*;
use crate::AppState;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

pub const EVENT_PROGRESS: &str = "catalog://scan-progress";
pub const EVENT_FINISHED: &str = "catalog://scan-finished";
/// A `MetaProgress` (at most about 4 a second per source) and, at the end, a `MetaSummary`.
pub const EVENT_META_PROGRESS: &str = "catalog://metadata-progress";
pub const EVENT_META_FINISHED: &str = "catalog://metadata-finished";
/// A `ThumbProgress` (at most about 4 a second per source) and, at the end, a `ThumbSummary`.
pub const EVENT_THUMB_PROGRESS: &str = "catalog://thumbnails-progress";
pub const EVENT_THUMB_FINISHED: &str = "catalog://thumbnails-finished";

struct Active {
    source_id: i64,
    ctl: Arc<ScanControl>,
    progress: Arc<Mutex<Option<Progress>>>,
}

struct ActiveMeta {
    source_id: i64,
    ctl: Arc<ScanControl>,
    progress: Arc<Mutex<Option<MetaProgress>>>,
}

struct ActiveThumbs {
    source_id: i64,
    ctl: Arc<ScanControl>,
    progress: Arc<Mutex<Option<ThumbProgress>>>,
}

struct Catalog {
    db: Arc<CatalogDb>,
    scans: Mutex<Vec<Active>>,
    metas: Mutex<Vec<ActiveMeta>>,
    thumbs: Mutex<Vec<ActiveThumbs>>,
    /// What the page tells a running thumbnail job: browsing (yield) and the photos on screen.
    signals: Arc<Signals>,
}

static CATALOG: OnceLock<Result<Arc<Catalog>, String>> = OnceLock::new();

fn catalog(app: &AppHandle) -> Result<Arc<Catalog>, String> {
    CATALOG
        .get_or_init(|| {
            let runtime = app.state::<AppState>().runtime.clone();
            CatalogDb::open(&runtime.join(FILE))
                .map(|db| Arc::new(Catalog {
                    db: Arc::new(db),
                    scans: Mutex::new(Vec::new()),
                    metas: Mutex::new(Vec::new()),
                    thumbs: Mutex::new(Vec::new()),
                    signals: Arc::new(Signals::default()),
                }))
                .map_err(|e| e.to_string())
        })
        .clone()
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

/// Every source with where it is now; relinks a volume that was remounted elsewhere.
#[tauri::command]
#[specta::specta]
pub async fn sources_list(app: AppHandle) -> Result<Vec<SourceStatus>, String> {
    blocking(move || {
        let cat = catalog(&app)?;
        sources::reconcile(&cat.db, &MountTable::current(), &system_uuid_of)
    })
    .await
}

/// What a folder is, before it is added.
#[tauri::command]
#[specta::specta]
pub async fn source_detect(app: AppHandle, path: String) -> Result<SourceDetect, String> {
    blocking(move || {
        let cat = catalog(&app)?;
        let p = sources::normalize(Path::new(&path));
        let readable = crate::volumes::answering(&p) && p.is_dir();
        let table = MountTable::current();
        let known = list_sources(&cat.db.read().map_err(|e| e.to_string())?)?;
        let opts = AddOptions { allow_offline: true, ..Default::default() };
        let plan = sources::plan_add(&table, &p, readable, &known, &opts, &system_uuid_of);
        let (planned, overlaps) = match plan {
            Ok(n) => (n, None),
            Err(AddError::Overlap(id)) => {
                let n = sources::plan_add(&table, &p, readable, &[], &opts, &system_uuid_of).map_err(|e| e.to_string())?;
                (n, Some(id))
            }
            Err(e) => return Err(e.to_string()),
        };
        let asleep = planned.pending;
        let protocol = match planned.fs_type.as_str() {
            "cifs" | "smb3" | "smbfs" => Some("SMB".to_string()),
            t if t.starts_with("nfs") => Some("NFS".into()),
            "fuse.sshfs" | "sshfs" => Some("SFTP".into()),
            _ => None,
        };
        let storage = match (planned.kind.as_str(), &protocol) {
            ("nas", Some(pr)) => format!(
                "Network share, {pr} ({})",
                [planned.host.clone(), planned.share.clone()].into_iter().flatten().collect::<Vec<_>>().join(", ")
            ),
            ("nas", None) => "Network share".to_string(),
            ("drive", _) => format!("Drive ({})", planned.fs_type),
            _ => "Local disk".to_string(),
        };
        Ok(SourceDetect {
            storage,
            kind: planned.kind,
            protocol,
            host: planned.host,
            share: planned.share,
            fs_type: planned.fs_type,
            mount_path: planned.mount_path,
            readable,
            asleep,
            overlaps,
        })
    })
    .await
}

/// A few seconds of read-only listing, to tell the user what to expect.
#[tauri::command]
#[specta::specta]
pub async fn source_speed_test(path: String) -> Result<SpeedTest, String> {
    blocking(move || {
        use crate::catalog_scan::{EntryKind, ScanFs};
        let root = PathBuf::from(path);
        let fs = RealFs::new(root.clone());
        let started = std::time::Instant::now();
        let (mut stack, mut entries, mut failed) = (vec![root.clone()], 0u64, 0u64);
        while let Some(d) = stack.pop() {
            if started.elapsed() > Duration::from_secs(3) {
                break;
            }
            match fs.list(&d, Duration::from_secs(5)) {
                Ok(list) => {
                    entries += list.len() as u64;
                    for e in list {
                        if e.kind == EntryKind::Dir && !e.name.to_string_lossy().starts_with('.') {
                            stack.push(d.join(&e.name));
                        }
                    }
                }
                Err(_) => failed += 1,
            }
        }
        let secs = started.elapsed().as_secs_f64().max(0.001);
        let per_s = entries as f64 / secs;
        let verdict = if entries == 0 && failed > 0 {
            "failed"
        } else if per_s >= 2000.0 {
            "fast"
        } else if per_s >= 500.0 {
            "good"
        } else if per_s >= 100.0 {
            "slow"
        } else {
            "very_slow"
        };
        Ok(SpeedTest { listing_per_s: per_s, reading_per_s: None, verdict: verdict.into(), sampled: entries, elapsed_s: secs })
    })
    .await
}

fn add_error(e: AddError) -> String {
    e.to_string()
}

#[tauri::command]
#[specta::specta]
pub async fn source_add(app: AppHandle, args: SourceAdd) -> Result<Source, String> {
    let handle = app.clone();
    let src = blocking(move || {
        let cat = catalog(&app)?;
        let opts = AddOptions {
            name: args.name.filter(|n| !n.trim().is_empty()),
            policy: args.policy,
            include_subfolders: args.include_subfolders,
            allow_offline: args.allow_offline,
            treat_as_network: None,
        };
        sources::add_source(&cat.db, Path::new(&args.path), &opts).map_err(add_error).map(|s| (s, args.index_now))
    })
    .await?;
    if src.1 {
        start_scans(&handle, vec![src.0.id], false)?;
    }
    Ok(src.0)
}

#[tauri::command]
#[specta::specta]
pub async fn source_update(app: AppHandle, id: i64, patch: SourcePatch) -> Result<(), String> {
    blocking(move || {
        let cat = catalog(&app)?;
        let net = match patch.treat_as_network.as_deref() {
            Some("yes") => Some(Some(true)),
            Some("no") => Some(Some(false)),
            Some("auto") => Some(None),
            Some(other) => return Err(format!("treat_as_network must be auto, yes or no, not {other}")),
            None => None,
        };
        sources::update_source(
            &cat.db,
            id,
            SourceUpdate { name: patch.name, policy: patch.policy, enabled: patch.enabled, treat_as_network: net, dir_skip: patch.dir_skip },
        )
    })
    .await
}

/// With `dry_run` only what removing would forget; otherwise removes. Never touches a file.
#[tauri::command]
#[specta::specta]
pub async fn source_remove(app: AppHandle, id: i64, dry_run: bool) -> Result<RemovePreview, String> {
    blocking(move || {
        let cat = catalog(&app)?;
        let preview = sources::remove_preview(&cat.db.read().map_err(|e| e.to_string())?, id)?;
        if !dry_run {
            for a in cat.scans.lock().unwrap().iter().filter(|a| a.source_id == id) {
                a.ctl.cancel();
            }
            sources::remove_source(&cat.db, id)?;
        }
        Ok(preview)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn source_relink(app: AppHandle, id: i64, new_path: String, dry_run: bool) -> Result<RelinkCheck, String> {
    blocking(move || {
        let cat = catalog(&app)?;
        sources::relink(&cat.db, id, Path::new(&new_path), dry_run, &MountTable::current(), &|p| {
            crate::volumes::answering(p) && std::fs::metadata(p).is_ok()
        })
    })
    .await
}

/// Starts scanning the given sources one after another (one NAS at a time);
/// returns at once. Progress arrives as events.
#[tauri::command]
#[specta::specta]
pub async fn scan_start(app: AppHandle, source_ids: Vec<i64>, light: bool) -> Result<(), String> {
    start_scans(&app, source_ids, light)
}

fn start_scans(app: &AppHandle, ids: Vec<i64>, light: bool) -> Result<(), String> {
    let cat = catalog(app)?;
    let app = app.clone();
    std::thread::Builder::new()
        .name("catalog-scan".into())
        .spawn(move || {
            for id in ids {
                scan_one(&app, &cat, id, light);
            }
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn scan_one(app: &AppHandle, cat: &Arc<Catalog>, id: i64, light: bool) {
    let summary = |state: &str, error: Option<String>| ScanSummary {
        source_id: id, job_id: 0, state: state.into(), resumed: false, elapsed_s: 0.0, found: 0, folders: 0, new: 0, changed: 0,
        unchanged: 0, missing: 0, unreadable: 0, incomplete_dirs: vec![], files_per_s_list: 0.0, error,
    };
    let fail = |m: String| {
        let _ = app.emit(EVENT_FINISHED, summary("failed", Some(m)));
    };
    if cat.scans.lock().unwrap().iter().any(|a| a.source_id == id) {
        return fail("A scan of this source is already running.".into());
    }
    let statuses = match sources::reconcile(&cat.db, &MountTable::current(), &system_uuid_of) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let Some(st) = statuses.into_iter().find(|s| s.source.id == id) else { return fail("No such source.".into()) };
    let (src, avail) = (st.source, st.availability);
    // an idle automount is woken by a scan: it is an explicit act
    let avail = match avail {
        Availability::Asleep { mount_point } => {
            let _ = crate::volumes::answering(Path::new(&mount_point));
            let table = MountTable::current();
            availability(&table, &src, &system_uuid_of)
        }
        other => other,
    };
    let abs_root = match avail {
        Availability::Online { abs_root, .. } if Path::new(&abs_root).is_dir() => abs_root,
        _ => {
            let _ = app.emit(EVENT_FINISHED, summary("offline", Some("The source is not reachable.".into())));
            return;
        }
    };
    let network = sources::treats_as_network(&src, crate::volumes::is_network_fs(&src.fs_type));
    let mut cfg = ScanConfig::for_network(network);
    cfg.progress_every = Duration::from_millis(250);
    let opts = ScanOptions {
        root: PathBuf::from(&abs_root),
        include_subfolders: src.include_subfolders,
        dir_skip: src.dir_skip,
        light,
        fresh: false,
    };
    let ctl = Arc::new(ScanControl::default());
    let progress = Arc::new(Mutex::new(None));
    cat.scans.lock().unwrap().push(Active { source_id: id, ctl: ctl.clone(), progress: progress.clone() });
    let fs = RealFs::new(PathBuf::from(&abs_root));
    let result = run_scan(&cat.db, &fs, &src, &opts, &cfg, &ctl, &mut |p| {
        *progress.lock().unwrap() = Some(p.clone());
        let _ = app.emit(EVENT_PROGRESS, p);
    });
    cat.scans.lock().unwrap().retain(|a| a.source_id != id);
    let _ = match result {
        Ok(r) => app.emit(EVENT_FINISHED, ScanSummary::from(&r)),
        Err(e) => app.emit(EVENT_FINISHED, summary("failed", Some(e))),
    };
}

fn for_jobs(cat: &Catalog, job_id: Option<i64>, f: impl Fn(&ScanControl)) {
    for a in cat.scans.lock().unwrap().iter() {
        let this = a.progress.lock().unwrap().as_ref().map(|p| p.job_id);
        if job_id.is_none() || this == job_id {
            f(&a.ctl);
        }
    }
    for a in cat.metas.lock().unwrap().iter() {
        let this = a.progress.lock().unwrap().as_ref().map(|p| p.job_id);
        if job_id.is_none() || this == job_id {
            f(&a.ctl);
        }
    }
    for a in cat.thumbs.lock().unwrap().iter() {
        let this = a.progress.lock().unwrap().as_ref().map(|p| p.job_id);
        if job_id.is_none() || this == job_id {
            f(&a.ctl);
        }
    }
}

/// The catalog's thumbnail store: beside the catalog file.
fn thumb_store(app: &AppHandle) -> Store {
    Store::new(app.state::<AppState>().runtime.join("catalog-thumbs"))
}

/// Makes small thumbnails of the photos of the given sources whose metadata is read
/// (see `catalog_thumb_job`), one source after another, at low priority; each is also
/// linked into the library thumbnail cache so the grid finds it. Progress comes as
/// `catalog://thumbnails-progress`, the end as `catalog://thumbnails-finished`; pause,
/// resume and cancel are `job_pause`, `job_resume` and `job_cancel`.
#[tauri::command]
#[specta::specta]
pub async fn thumbnails_start(app: AppHandle, source_ids: Vec<i64>) -> Result<(), String> {
    let cat = catalog(&app)?;
    std::thread::Builder::new()
        .name("catalog-thumbnails".into())
        .spawn(move || {
            for id in source_ids {
                thumb_one(&app, &cat, id);
            }
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn thumb_one(app: &AppHandle, cat: &Arc<Catalog>, id: i64) {
    let summary = |state: &str, error: Option<String>| ThumbSummary {
        source_id: id, job_id: 0, state: state.into(), resumed: false, elapsed_s: 0.0, total: 0, done: 0, failed: 0, deferred: 0, skipped: 0,
        from_preview: 0, from_exif: 0, from_jpeg: 0, from_engine: 0, linked: 0, reads: 0, bytes_read: 0, thumbnail_bytes: 0, per_s: 0.0,
        p50_ms: 0.0, p95_ms: 0.0, error,
    };
    let fail = |m: String| {
        let _ = app.emit(EVENT_THUMB_FINISHED, summary("failed", Some(m)));
    };
    if cat.thumbs.lock().unwrap().iter().any(|a| a.source_id == id) {
        return fail("The thumbnails of this source are already being made.".into());
    }
    let statuses = match sources::reconcile(&cat.db, &MountTable::current(), &system_uuid_of) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let Some(st) = statuses.into_iter().find(|s| s.source.id == id) else { return fail("No such source.".into()) };
    let (src, avail) = (st.source, st.availability);
    let abs_root = match avail {
        Availability::Online { abs_root, .. } if Path::new(&abs_root).is_dir() => abs_root,
        _ => {
            let _ = app.emit(EVENT_THUMB_FINISHED, summary("offline", Some("The source is not reachable.".into())));
            return;
        }
    };
    let network = sources::treats_as_network(&src, crate::volumes::is_network_fs(&src.fs_type));
    let mut cfg = ThumbConfig::for_network(network);
    cfg.lower_priority = true;
    // the app's engine can render what Rust cannot decode: a hook for it goes in `cfg.engine`
    let ctl = Arc::new(ScanControl::default());
    let progress = Arc::new(Mutex::new(None));
    cat.thumbs.lock().unwrap().push(ActiveThumbs { source_id: id, ctl: ctl.clone(), progress: progress.clone() });
    let fs = RealFs::new(PathBuf::from(&abs_root));
    let store = thumb_store(app);
    let cache = app.state::<crate::cache::Caches>().thumbs.clone();
    let result = run_thumbnails(&cat.db, &fs, &src, Path::new(&abs_root), &store, Some(&cache), &cfg, &ctl, &cat.signals, &mut |p| {
        *progress.lock().unwrap() = Some(p.clone());
        let _ = app.emit(EVENT_THUMB_PROGRESS, p);
    });
    cat.thumbs.lock().unwrap().retain(|a| a.source_id != id);
    let _ = match result {
        Ok(r) => app.emit(EVENT_THUMB_FINISHED, ThumbSummary::from(&r)),
        Err(e) => app.emit(EVENT_THUMB_FINISHED, summary("failed", Some(e))),
    };
}

/// The last progress of every running thumbnail job.
#[tauri::command]
#[specta::specta]
pub async fn thumbnails_jobs_active(app: AppHandle) -> Result<Vec<ThumbProgress>, String> {
    let cat = catalog(&app)?;
    let v = cat.thumbs.lock().unwrap().iter().filter_map(|a| a.progress.lock().unwrap().clone()).collect();
    Ok(v)
}

/// The page says whether the user is browsing: while true a thumbnail job waits.
#[tauri::command]
#[specta::specta]
pub async fn thumbnails_busy(app: AppHandle, busy: bool) -> Result<(), String> {
    let cat = catalog(&app)?;
    cat.signals.set_busy(busy);
    Ok(())
}

/// The photos on screen (absolute paths, the first most wanted): a running thumbnail
/// job makes them next.
#[tauri::command]
#[specta::specta]
pub async fn thumbnails_hint(app: AppHandle, paths: Vec<String>) -> Result<(), String> {
    let cat = catalog(&app)?;
    cat.signals.hint(paths);
    Ok(())
}

/// Removes thumbnail files nothing points to, trims the store to its limit (the
/// photos still present first, the missing ones last) and returns what it did.
#[tauri::command]
#[specta::specta]
pub async fn thumbnails_gc(app: AppHandle) -> Result<GcReport, String> {
    blocking(move || {
        let cat = catalog(&app)?;
        thumbs::gc(&cat.db, &thumb_store(&app))
    })
    .await
}

/// The grid's cache miss: the catalog's thumbnail of the photo at `path`, if it made one
/// for the file as it is (`size`, `mtime_ns`, sidecar stamp `side`). Never opens the
/// catalog: before the page has used it, it answers nothing.
pub fn thumb_fallback(app: &AppHandle, path: &Path, size: u64, mtime_ns: i64, side: &str) -> Option<Vec<u8>> {
    let cat = CATALOG.get()?.as_ref().ok()?.clone();
    let conn = cat.db.read().ok()?;
    let roots: Vec<(i64, PathBuf)> = list_sources(&conn)
        .ok()?
        .into_iter()
        .filter_map(|s| Some((s.id, Path::new(s.mount_path.as_deref()?).join(s.rel_root.trim_start_matches('/')))))
        .collect();
    thumbs::lookup_by_path(&conn, &thumb_store(app), &roots, path, size, mtime_ns, side)
}

/// Reads the headers and sidecars of every photo of the sources that a scan found new or
/// changed (and retries earlier failures), then relinks renamed and moved photos. Progress comes as
/// `catalog://metadata-progress`, the end as `catalog://metadata-finished`. Pause, resume and
/// cancel are `job_pause`, `job_resume` and `job_cancel`, like a scan's.
#[tauri::command]
#[specta::specta]
pub async fn metadata_start(app: AppHandle, source_ids: Vec<i64>) -> Result<(), String> {
    let cat = catalog(&app)?;
    std::thread::Builder::new()
        .name("catalog-metadata".into())
        .spawn(move || {
            for id in source_ids {
                meta_one(&app, &cat, id);
            }
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn meta_one(app: &AppHandle, cat: &Arc<Catalog>, id: i64) {
    let summary = |state: &str, error: Option<String>| MetaSummary {
        source_id: id, job_id: 0, state: state.into(), resumed: false, elapsed_s: 0.0, total: 0, done: 0, sidecar_only: 0, failed: 0,
        unparsed: 0, skipped: 0, reads: 0, escalations: 0, bytes_read: 0, per_s: 0.0, p50_ms: 0.0, p95_ms: 0.0, relinked: 0, ambiguous: 0, error,
    };
    let fail = |m: String| {
        let _ = app.emit(EVENT_META_FINISHED, summary("failed", Some(m)));
    };
    if cat.metas.lock().unwrap().iter().any(|a| a.source_id == id) {
        return fail("The metadata of this source is already being read.".into());
    }
    let statuses = match sources::reconcile(&cat.db, &MountTable::current(), &system_uuid_of) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let Some(st) = statuses.into_iter().find(|s| s.source.id == id) else { return fail("No such source.".into()) };
    let (src, avail) = (st.source, st.availability);
    let abs_root = match avail {
        Availability::Online { abs_root, .. } if Path::new(&abs_root).is_dir() => abs_root,
        _ => {
            let _ = app.emit(EVENT_META_FINISHED, summary("offline", Some("The source is not reachable.".into())));
            return;
        }
    };
    let network = sources::treats_as_network(&src, crate::volumes::is_network_fs(&src.fs_type));
    let ctl = Arc::new(ScanControl::default());
    let progress = Arc::new(Mutex::new(None));
    cat.metas.lock().unwrap().push(ActiveMeta { source_id: id, ctl: ctl.clone(), progress: progress.clone() });
    let fs = RealFs::new(PathBuf::from(&abs_root));
    let result = run_metadata(&cat.db, &fs, &src, &MetaConfig::for_network(network), &ctl, &mut |p| {
        *progress.lock().unwrap() = Some(p.clone());
        let _ = app.emit(EVENT_META_PROGRESS, p);
    });
    cat.metas.lock().unwrap().retain(|a| a.source_id != id);
    let _ = match result {
        Ok(r) => app.emit(EVENT_META_FINISHED, MetaSummary::from(&r)),
        Err(e) => app.emit(EVENT_META_FINISHED, summary("failed", Some(e))),
    };
}

/// The last progress of every running metadata job.
#[tauri::command]
#[specta::specta]
pub async fn metadata_jobs_active(app: AppHandle) -> Result<Vec<MetaProgress>, String> {
    let cat = catalog(&app)?;
    let v = cat.metas.lock().unwrap().iter().filter_map(|a| a.progress.lock().unwrap().clone()).collect();
    Ok(v)
}

/// `job_id` None means every running scan.
#[tauri::command]
#[specta::specta]
pub async fn job_pause(app: AppHandle, job_id: Option<i64>) -> Result<(), String> {
    let cat = catalog(&app)?;
    for_jobs(&cat, job_id, |c| c.pause());
    Ok(())
}

#[tauri::command]
#[specta::specta]
pub async fn job_resume(app: AppHandle, job_id: Option<i64>) -> Result<(), String> {
    let cat = catalog(&app)?;
    for_jobs(&cat, job_id, |c| c.resume());
    Ok(())
}

/// Stops for good: the run is closed `cancelled` and a later scan starts afresh.
#[tauri::command]
#[specta::specta]
pub async fn job_cancel(app: AppHandle, job_id: Option<i64>) -> Result<(), String> {
    let cat = catalog(&app)?;
    for_jobs(&cat, job_id, |c| c.cancel());
    Ok(())
}

/// The last progress of every running scan.
#[tauri::command]
#[specta::specta]
pub async fn jobs_active(app: AppHandle) -> Result<Vec<Progress>, String> {
    let cat = catalog(&app)?;
    let v = cat.scans.lock().unwrap().iter().filter_map(|a| a.progress.lock().unwrap().clone()).collect();
    Ok(v)
}

/// What a rebuild forgets: that any photo's metadata was read. The next metadata run
/// reads every present photo again; sources, photos, ratings in sidecars, thumbnails
/// and the files themselves are untouched. Returns the number of photos queued.
pub fn reset_for_rebuild(db: &CatalogDb) -> Result<i64, String> {
    db.exec(|c| {
        c.execute("UPDATE photos SET meta_state=0 WHERE status='present'", [])
            .map(|n| n as i64)
            .map_err(|e| e.to_string())
    })
}

/// Marks every photo's metadata as not read so the page can index again (scan, metadata,
/// thumbnails). Refused while a job runs. Never opens or changes a photo file.
#[tauri::command]
#[specta::specta]
pub async fn catalog_rebuild(app: AppHandle) -> Result<i64, String> {
    blocking(move || {
        let cat = catalog(&app)?;
        if cat.scans.lock().unwrap().len() + cat.metas.lock().unwrap().len() + cat.thumbs.lock().unwrap().len() > 0 {
            return Err("Wait for the running index to finish, or stop it, before rebuilding.".into());
        }
        reset_for_rebuild(&cat.db)
    })
    .await
}

#[tauri::command]
#[specta::specta]
pub async fn catalog_status(app: AppHandle) -> Result<CatalogStatus, String> {
    blocking(move || {
        let cat = catalog(&app)?;
        let conn = cat.db.read().map_err(|e| e.to_string())?;
        let n = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0);
        let table = MountTable::current();
        let list = list_sources(&conn)?;
        let online = list.iter().filter(|s| matches!(availability(&table, s, &system_uuid_of), Availability::Online { .. })).count() as i64;
        let running = (cat.scans.lock().unwrap().len() + cat.metas.lock().unwrap().len() + cat.thumbs.lock().unwrap().len()) as i64;
        Ok(CatalogStatus {
            path: cat.db.path.to_string_lossy().into_owned(),
            schema: SCHEMA_VERSION,
            bytes: std::fs::metadata(&cat.db.path).map(|m| m.len()).unwrap_or(0),
            photos: n("SELECT COUNT(*) FROM photos WHERE status='present'"),
            missing: n("SELECT COUNT(*) FROM photos WHERE status='missing'"),
            sources: list.len() as i64,
            online,
            offline: list.len() as i64 - online,
            jobs_running: running,
            metadata: coverage(&conn, None).map_err(|e| e.to_string())?,
            thumbnails: thumbs::coverage(&conn, None).map_err(|e| e.to_string())?,
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog_fakefs::{FakeFs, BASE_MTIME};

    #[test]
    fn rebuild_queues_every_present_photo_and_changes_nothing_else() {
        let dir = tempfile::tempdir().unwrap();
        let db = CatalogDb::open(&dir.path().join("catalog.sqlite")).unwrap();
        db.exec(|c| {
            c.execute("INSERT INTO sources(name,kind,volume_key,mount_path,rel_root,created_at) VALUES('S','nas','smb:x/y','/fake','',1)", [])
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
        .unwrap();
        let fs = FakeFs::new();
        for n in ["a.jpg", "b.jpg", "c.jpg"] {
            fs.add_blob(n, vec![0u8; 64], BASE_MTIME);
        }
        let src = sources::get_source(&db.read().unwrap(), 1).unwrap().unwrap();
        let o = ScanOptions { root: fs.root.clone(), include_subfolders: true, dir_skip: false, light: false, fresh: false };
        let mut cfg = ScanConfig::for_network(false);
        cfg.workers = 2;
        run_scan(&db, &fs, &src, &o, &cfg, &ScanControl::default(), &mut |_| {}).unwrap();
        db.exec(|c| c.execute("UPDATE photos SET meta_state=1, rating=4", []).map(|_| ()).map_err(|e| e.to_string())).unwrap();
        assert_eq!(reset_for_rebuild(&db).unwrap(), 3);
        let c = db.read().unwrap();
        let q = |sql: &str| c.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap();
        assert_eq!(q("SELECT COUNT(*) FROM photos WHERE meta_state=0"), 3);
        // what was read stays until the next run replaces it
        assert_eq!(q("SELECT COUNT(*) FROM photos WHERE rating=4"), 3);
        assert_eq!(q("SELECT COUNT(*) FROM sources"), 1);
    }
}
