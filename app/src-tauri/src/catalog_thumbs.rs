// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The Catalog's thumbnail store and everything around it that is not the job
//! itself: configuration, the signals the app sends a running job, the files,
//! garbage collection, and the link into the grid's disk cache.
//!
//! How this meets the grid (notes/catalog-thumbs-notes.md): a made thumbnail is
//! kept in a store of its own, one JPEG per `photos.id` (`<dir>/<id/1024>/<id>.jpg`),
//! so it survives renames, moves (relink keeps the row) and a source going
//! offline. It is then hard-linked into the existing library thumbnail cache
//! (`cache::DiskCache`) under the very name the grid computes for the photo
//! (`cache::named`: path, version 0, file stamp, sidecar stamp, size 240), and
//! the path-only `.last` pointer is set, so `thumb_cached` and `thumb_known`
//! hit with no change to the page. When the cache evicts or a photo moves, the
//! store's copy is linked again without reading the photo.

use crate::cache::{self, DiskCache};
use crate::catalog_db::{self, CatalogDb};
use crate::catalog_sources::Source;
use rusqlite::{params, Connection};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The size the grid asks the cache for (library.ts `THUMB_SIZE`); the stored
/// thumbnail is named for it whatever its pixels measure.
pub const GRID_SIZE: u32 = 240;
/// The long edge of a stored thumbnail unless told otherwise.
pub const DEFAULT_EDGE: u32 = 480;
pub const DEFAULT_QUALITY: u8 = 80;
/// The store's size limit: about 100,000 thumbnails.
pub const DEFAULT_CAP: u64 = 3 * 1024 * 1024 * 1024;
/// Below this much free space nothing is written (the disk cache's rule).
pub const MIN_FREE: u64 = 5 * 1024 * 1024 * 1024;

/// Whether a JPEG with no usable Exif thumbnail may be read whole: the policy
/// knob `read whole JPEGs for thumbnails`. A 19 MB JPEG is 19 MB over the
/// network, against 25 to 400 KB for everything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WholeJpegs {
    /// Never (except files small enough that a second read would cost more).
    No,
    /// Only while the app is not being used for browsing (a CLI run never is).
    Idle,
    Yes,
}

#[allow(dead_code)]
impl WholeJpegs {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "no" | "never" => Some(Self::No),
            "idle" | "when-idle" => Some(Self::Idle),
            "yes" | "always" => Some(Self::Yes),
            _ => None,
        }
    }
    pub fn word(self) -> &'static str {
        match self {
            Self::No => "no",
            Self::Idle => "idle",
            Self::Yes => "yes",
        }
    }
}

/// Provides the engine's thumbnail for a photo no Rust path can decode (HEIC,
/// TIFF, a corrupt preview): the app wires it, the CLI has none. Returns a JPEG.
pub type EngineHook = Arc<dyn Fn(&str) -> Option<Vec<u8>> + Send + Sync>;

#[derive(Clone)]
pub struct ThumbConfig {
    /// Reads at once: 2 on a network share, as the baseline found.
    pub read_workers: usize,
    /// Decoders and resizers: cores minus one.
    pub decode_workers: usize,
    pub read_deadline: Duration,
    pub edge: u32,
    pub quality: u8,
    /// An Exif thumbnail is used only if its long edge is at least this.
    pub min_exif_edge: u32,
    /// The first read of a JPEG, which must hold the Exif block.
    pub exif_head: usize,
    /// A JPEG at most this big is read whole at once, whatever the policy.
    pub small_jpeg: u64,
    /// The largest file or preview read whole.
    pub max_whole: u64,
    pub whole_jpegs: WholeJpegs,
    pub adaptive: bool,
    pub retry_failed: bool,
    /// Link made thumbnails into the grid's cache.
    pub publish: bool,
    /// Lower the CPU and I/O priority of the workers.
    pub lower_priority: bool,
    pub batch_rows: usize,
    pub flush_every: Duration,
    pub progress_every: Duration,
    pub checkpoint_every: Duration,
    pub chunk: usize,
    pub engine: Option<EngineHook>,
}

impl ThumbConfig {
    pub fn for_network(network: bool) -> Self {
        let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
        Self {
            read_workers: if network { 2 } else { 4 },
            decode_workers: cores.saturating_sub(1).max(1),
            read_deadline: Duration::from_secs(5),
            edge: DEFAULT_EDGE,
            quality: DEFAULT_QUALITY,
            min_exif_edge: 256,
            exif_head: 80 * 1024,
            small_jpeg: 768 * 1024,
            max_whole: 48 * 1024 * 1024,
            whole_jpegs: WholeJpegs::Idle,
            adaptive: true,
            retry_failed: true,
            publish: true,
            lower_priority: false,
            batch_rows: 500,
            flush_every: Duration::from_secs(1),
            progress_every: Duration::from_millis(250),
            checkpoint_every: Duration::from_secs(5),
            chunk: 2000,
            engine: None,
        }
    }
}

/// What the app tells a running job: whether the user is browsing (the job
/// yields) and which photos are on screen (they go first).
#[derive(Default)]
pub struct Signals {
    busy: AtomicBool,
    hints: Mutex<Vec<String>>,
    generation: AtomicU64,
}

impl Signals {
    pub fn set_busy(&self, busy: bool) {
        self.busy.store(busy, Ordering::SeqCst);
    }
    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::SeqCst)
    }
    /// Absolute paths of the photos the user is looking at, most wanted first.
    pub fn hint(&self, paths: Vec<String>) {
        *self.hints.lock().unwrap() = paths;
        self.generation.fetch_add(1, Ordering::SeqCst);
    }
    pub(crate) fn changed(&self, seen: &mut u64) -> Option<Vec<String>> {
        let g = self.generation.load(Ordering::SeqCst);
        if g == *seen {
            return None;
        }
        *seen = g;
        Some(self.hints.lock().unwrap().clone())
    }
}

/// Nice 10 and the idlest best-effort I/O class for the calling thread, where
/// the system allows it. Failure is ignored.
pub fn lower_priority() {
    #[cfg(target_os = "linux")]
    // SAFETY: plain system calls on the calling thread with integer arguments
    unsafe {
        let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
        libc::setpriority(libc::PRIO_PROCESS, tid, 10);
        // ioprio_set(IOPRIO_WHO_PROCESS, tid, best effort class, level 7)
        libc::syscall(libc::SYS_ioprio_set, 1 as libc::c_long, tid as libc::c_long, ((2 << 13) | 7) as libc::c_long);
    }
}

// ---- the files --------------------------------------------------------------------------

#[derive(Debug)]
pub enum StoreError {
    /// Below the free-space floor: nothing is written.
    Full,
    Io(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StoreError::Full => write!(f, "the disk is nearly full"),
            StoreError::Io(e) => write!(f, "{e}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct StoredFile {
    pub id: i64,
    pub size: u64,
    pub time: SystemTime,
    pub path: PathBuf,
}

/// One JPEG per photo id. Writes are atomic (a part file, then a rename), so a
/// reader or a crash never sees half a thumbnail.
#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
    pub cap: u64,
    pub min_free: u64,
}

#[allow(dead_code)]
impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, cap: DEFAULT_CAP, min_free: MIN_FREE }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn path(&self, id: i64) -> PathBuf {
        self.dir.join((id >> 10).to_string()).join(format!("{id}.jpg"))
    }

    pub fn exists(&self, id: i64) -> bool {
        self.path(id).is_file()
    }

    pub fn read(&self, id: i64) -> Option<Vec<u8>> {
        std::fs::read(self.path(id)).ok()
    }

    pub fn remove(&self, id: i64) {
        let _ = std::fs::remove_file(self.path(id));
    }

    pub fn write(&self, id: i64, bytes: &[u8]) -> Result<(), StoreError> {
        self.write_inner(id, bytes, None)
    }

    fn write_inner(&self, id: i64, bytes: &[u8], crash_after: Option<usize>) -> Result<(), StoreError> {
        let path = self.path(id);
        let dir = path.parent().unwrap_or(&self.dir);
        std::fs::create_dir_all(dir).map_err(|e| StoreError::Io(e.to_string()))?;
        if cache::free_space(dir).is_some_and(|f| f < self.min_free) {
            return Err(StoreError::Full);
        }
        let part = dir.join(format!(".{id}.part"));
        let cut = crash_after.map_or(bytes.len(), |n| n.min(bytes.len()));
        std::fs::write(&part, &bytes[..cut]).map_err(|e| StoreError::Io(e.to_string()))?;
        if crash_after.is_some() {
            // a process killed here leaves the part file and nothing else
            return Err(StoreError::Io("simulated crash".into()));
        }
        std::fs::rename(&part, &path).map_err(|e| {
            let _ = std::fs::remove_file(&part);
            StoreError::Io(e.to_string())
        })
    }

    #[cfg(test)]
    pub(crate) fn write_crashing(&self, id: i64, bytes: &[u8], after: usize) -> Result<(), StoreError> {
        self.write_inner(id, bytes, Some(after))
    }

    /// Every thumbnail file, and the part files of writes that never finished.
    pub fn scan(&self) -> (Vec<StoredFile>, Vec<PathBuf>) {
        let (mut files, mut parts) = (Vec::new(), Vec::new());
        let Ok(top) = std::fs::read_dir(&self.dir) else { return (files, parts) };
        for shard in top.flatten() {
            let Ok(list) = std::fs::read_dir(shard.path()) else { continue };
            for f in list.flatten() {
                let name = f.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') && name.ends_with(".part") {
                    parts.push(f.path());
                } else if let Some(id) = name.strip_suffix(".jpg").and_then(|s| s.parse::<i64>().ok()) {
                    if let Ok(m) = f.metadata() {
                        files.push(StoredFile { id, size: m.len(), time: m.modified().unwrap_or(UNIX_EPOCH), path: f.path() });
                    }
                }
            }
        }
        (files, parts)
    }

    pub fn usage(&self) -> u64 {
        self.scan().0.iter().map(|f| f.size).sum()
    }
}

// ---- garbage collection -------------------------------------------------------------------

#[derive(Debug, Clone, Default, Serialize, specta::Type)]
pub struct GcReport {
    /// Part files of writes that never finished.
    pub partials: u64,
    /// Files whose photo is gone from the catalog (source removed, relink merged).
    pub orphans: u64,
    /// Rows whose file is not there: made again by the next run.
    pub rows_without_file: u64,
    /// Thumbnails dropped to stay under the size limit, of photos that are present
    /// (made again later) and of photos that are missing (not recoverable).
    pub evicted_present: u64,
    pub evicted_missing: u64,
    pub bytes_freed: u64,
    pub bytes_after: u64,
}

/// Part files older than this are leftovers, not a write in progress.
const PART_AGE: Duration = Duration::from_secs(600);

/// Brings the store and the `thumbs` table into agreement and under the limit:
///
/// 1. part files of crashed writes go;
/// 2. a file with no row (its photo was deleted by a source removal or merged by
///    a relink) goes; a row whose file is gone is dropped, so it is made again;
/// 3. over the limit, the least recently made thumbnails of photos that are
///    present go first (the photo can be read again), and only then those of
///    missing photos (an offline NAS, or a deleted photo), oldest first, down to
///    90% of the limit.
pub fn gc(db: &CatalogDb, store: &Store) -> Result<GcReport, String> {
    let mut rep = GcReport::default();
    let (files, parts) = store.scan();
    for p in parts {
        let old = std::fs::metadata(&p).and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_none_or(|a| a > PART_AGE);
        if old && std::fs::remove_file(&p).is_ok() {
            rep.partials += 1;
        }
    }
    let rows: HashMap<i64, (bool, i64)> = {
        let c = db.read().map_err(|e| e.to_string())?;
        let mut st = c.prepare("SELECT t.photo_id, p.status='present', t.used_at FROM thumbs t JOIN photos p ON p.id=t.photo_id WHERE t.state=1").map_err(|e| e.to_string())?;
        let v = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, (r.get::<_, bool>(1)?, r.get::<_, i64>(2)?)))).map_err(|e| e.to_string())?;
        v.flatten().collect()
    };
    let mut keep: Vec<(&StoredFile, bool, i64)> = Vec::new();
    for f in &files {
        match rows.get(&f.id) {
            Some((present, used)) => keep.push((f, *present, *used)),
            None => {
                if std::fs::remove_file(&f.path).is_ok() {
                    rep.orphans += 1;
                    rep.bytes_freed += f.size;
                }
            }
        }
    }
    let have: HashSet<i64> = keep.iter().map(|(f, ..)| f.id).collect();
    let mut drop_rows: Vec<i64> = rows.keys().copied().filter(|id| !have.contains(id)).collect();
    rep.rows_without_file = drop_rows.len() as u64;
    let mut total: u64 = keep.iter().map(|(f, ..)| f.size).sum();
    if total > store.cap {
        let target = store.cap / 10 * 9;
        // present photos first, then missing; within each the least recently used
        keep.sort_by_key(|(f, present, used)| (!*present, *used, f.time));
        for (f, present, _) in keep {
            if total <= target {
                break;
            }
            if std::fs::remove_file(&f.path).is_ok() {
                total -= f.size;
                rep.bytes_freed += f.size;
                drop_rows.push(f.id);
                if present {
                    rep.evicted_present += 1;
                } else {
                    rep.evicted_missing += 1;
                }
            }
        }
    }
    rep.bytes_after = total;
    if !drop_rows.is_empty() {
        db.exec(move |c| {
            let tx = c.transaction().map_err(|e| e.to_string())?;
            for id in drop_rows {
                tx.execute("DELETE FROM thumbs WHERE photo_id=?1", [id]).map_err(|e| e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())
        })?;
    }
    Ok(rep)
}

// ---- linking into the grid's cache ---------------------------------------------------------------

/// What the grid's cache name for a photo needs from the catalog row.
#[derive(Debug, Clone)]
pub struct Stamps {
    pub size: u64,
    pub mtime_ns: i64,
    pub xmp_kind: u8,
    pub xmp_size: Option<i64>,
    pub xmp_mtime_ns: Option<i64>,
}

/// The name under which the grid looks the photo's thumbnail up (version 0, the
/// grid size): the same hash `cache::listed_thumb_name` computes from a listing.
/// Only a `name.ext.xmp` sidecar counts, as in the cache.
pub fn cache_name(abs_path: &Path, s: &Stamps) -> String {
    let file = cache::stamp_of(s.size, s.mtime_ns.max(0) as u128);
    let side = match (s.xmp_kind, s.xmp_size, s.xmp_mtime_ns) {
        (1, Some(sz), Some(mt)) => cache::stamp_of(sz.max(0) as u64, mt.max(0) as u128),
        _ => "-".into(),
    };
    cache::named(abs_path, 0, GRID_SIZE, (file, side))
}

/// Links the store's thumbnail of `id` into the cache; the name it took, or None
/// when the photo has no cache entry to make (an edited one: the engine renders
/// those) or the cache did not take it.
pub fn publish_one(store: &Store, cache: &DiskCache, id: i64, abs_path: &Path, s: &Stamps, edited: bool) -> Option<String> {
    if edited {
        return None;
    }
    let name = cache_name(abs_path, s);
    if !cache.contains(&name) && !cache.adopt_file(&name, &store.path(id)) {
        return None;
    }
    cache.set_known(abs_path, 0, GRID_SIZE, &name);
    Some(name)
}

#[derive(Debug, Clone, Default, Serialize, specta::Type)]
pub struct PublishReport {
    pub linked: u64,
    pub already: u64,
    pub skipped_edited: u64,
    pub no_file: u64,
}

/// Links every made thumbnail of `source` that the cache does not hold under the
/// name the grid would ask for now: after a rename or move (the path changed), a
/// cache eviction or clearing. Reads nothing from the photos.
pub fn publish_source(db: &CatalogDb, store: &Store, cache: &DiskCache, source: &Source, abs_root: &Path) -> Result<PublishReport, String> {
    let mut rep = PublishReport::default();
    let rows: Vec<(i64, String, Stamps, bool, Option<String>)> = {
        let c = db.read().map_err(|e| e.to_string())?;
        let mut st = c
            .prepare(
                "SELECT p.id, p.rel_path, p.size, p.mtime_ns, p.xmp_kind, p.xmp_size, p.xmp_mtime_ns, COALESCE(p.edited,0), t.published
                 FROM thumbs t JOIN photos p ON p.id=t.photo_id
                 WHERE p.source_id=?1 AND t.state=1 AND t.stamp_size=p.size AND t.stamp_mtime_ns=p.mtime_ns",
            )
            .map_err(|e| e.to_string())?;
        let v = st
            .query_map([source.id], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    Stamps { size: r.get::<_, i64>(2)?.max(0) as u64, mtime_ns: r.get(3)?, xmp_kind: r.get::<_, i64>(4)? as u8, xmp_size: r.get(5)?, xmp_mtime_ns: r.get(6)? },
                    r.get::<_, i64>(7)? == 1,
                    r.get(8)?,
                ))
            })
            .map_err(|e| e.to_string())?;
        v.flatten().collect()
    };
    let mut updates: Vec<(i64, Option<String>)> = Vec::new();
    for (id, rel, stamps, edited, published) in rows {
        if edited {
            rep.skipped_edited += 1;
            continue;
        }
        if !store.exists(id) {
            rep.no_file += 1;
            continue;
        }
        let abs = abs_root.join(&rel);
        let want = cache_name(&abs, &stamps);
        if published.as_deref() == Some(want.as_str()) && cache.contains(&want) {
            rep.already += 1;
            continue;
        }
        match publish_one(store, cache, id, &abs, &stamps, false) {
            Some(name) => {
                rep.linked += 1;
                updates.push((id, Some(name)));
            }
            None => rep.no_file += 1,
        }
    }
    if !updates.is_empty() {
        db.exec(move |c| {
            let tx = c.transaction().map_err(|e| e.to_string())?;
            for (id, name) in updates {
                tx.execute("UPDATE thumbs SET published=?2, used_at=?3 WHERE photo_id=?1", params![id, name, catalog_db::now_s()]).map_err(|e| e.to_string())?;
            }
            tx.commit().map_err(|e| e.to_string())
        })?;
    }
    Ok(rep)
}

/// The grid's lookup in the catalog, for a cache miss: the thumbnail of the photo at
/// `path` if the catalog has made one for the file as it is. `size` and `mtime_ns`
/// are the file's stamp and `side` its sidecar's (as `cache::listed_fingerprints`
/// gives them): all must equal what the catalog recorded, so a photo edited since
/// (its sidecar changed) is left to the engine. `sources` are the catalog's
/// sources with where each is mounted now.
pub fn lookup_by_path(c: &Connection, store: &Store, sources: &[(i64, PathBuf)], path: &Path, size: u64, mtime_ns: i64, side: &str) -> Option<Vec<u8>> {
    for (sid, root) in sources {
        let Ok(rel) = path.strip_prefix(root) else { continue };
        let rel = rel.to_string_lossy().replace('\\', "/");
        let row = c
            .query_row(
                "SELECT p.id, p.xmp_kind, p.xmp_size, p.xmp_mtime_ns FROM photos p JOIN thumbs t ON t.photo_id=p.id
                 WHERE p.source_id=?1 AND p.rel_path=?2 AND t.state=1 AND t.stamp_size=?3 AND t.stamp_mtime_ns=?4 AND COALESCE(p.edited,0)<>1",
                params![sid, rel, size as i64, mtime_ns],
                |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?, r.get::<_, Option<i64>>(2)?, r.get::<_, Option<i64>>(3)?)),
            )
            .ok();
        if let Some((id, kind, xs, xm)) = row {
            let mine = match (kind, xs, xm) {
                (1, Some(a), Some(b)) => cache::stamp_of(a.max(0) as u64, b.max(0) as u128),
                _ => "-".into(),
            };
            if mine == side {
                return store.read(id);
            }
        }
    }
    None
}

// ---- coverage -----------------------------------------------------------------------------------

/// How many photos have a thumbnail.
#[derive(Debug, Clone, Default, Serialize, specta::Type)]
pub struct ThumbCoverage {
    /// Present photos with a current thumbnail.
    pub done: i64,
    /// Present photos still to do (metadata read, no current thumbnail).
    pub pending: i64,
    pub failed: i64,
    /// Waiting for the engine, or for a policy that allows reading a whole JPEG.
    pub deferred: i64,
    /// Thumbnails kept for photos that are missing (offline or deleted).
    pub kept_missing: i64,
    /// What the thumbnails take on disk.
    pub bytes: i64,
}

pub fn coverage(c: &Connection, source_id: Option<i64>) -> rusqlite::Result<ThumbCoverage> {
    let one = |sql: &str| c.query_row(sql, [source_id], |r| r.get::<_, i64>(0));
    let stale = "(t.stamp_size IS NOT p.size OR t.stamp_mtime_ns IS NOT p.mtime_ns)";
    let done = one(&format!("SELECT COUNT(*) FROM thumbs t JOIN photos p ON p.id=t.photo_id WHERE p.status='present' AND t.state=1 AND NOT {stale} AND (?1 IS NULL OR p.source_id=?1)"))?;
    let failed = one(&format!("SELECT COUNT(*) FROM thumbs t JOIN photos p ON p.id=t.photo_id WHERE p.status='present' AND t.state=2 AND NOT {stale} AND (?1 IS NULL OR p.source_id=?1)"))?;
    let deferred = one(&format!("SELECT COUNT(*) FROM thumbs t JOIN photos p ON p.id=t.photo_id WHERE p.status='present' AND t.state=3 AND NOT {stale} AND (?1 IS NULL OR p.source_id=?1)"))?;
    let all = one("SELECT COUNT(*) FROM photos p WHERE p.status='present' AND p.meta_state IN (1,3) AND (?1 IS NULL OR p.source_id=?1)")?;
    let kept_missing = one("SELECT COUNT(*) FROM thumbs t JOIN photos p ON p.id=t.photo_id WHERE p.status='missing' AND t.state=1 AND (?1 IS NULL OR p.source_id=?1)")?;
    let bytes = one("SELECT COALESCE(SUM(t.bytes),0) FROM thumbs t JOIN photos p ON p.id=t.photo_id WHERE t.state=1 AND (?1 IS NULL OR p.source_id=?1)")?;
    Ok(ThumbCoverage { done, pending: (all - done - failed - deferred).max(0), failed, deferred, kept_missing, bytes })
}

#[cfg(test)]
#[path = "catalog_thumb_tests.rs"]
mod tests;
