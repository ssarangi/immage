// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! Catalog discovery: a safe, parallel, resumable walk of a source that records
//! names, sizes and modified times and nothing else (headers are the metadata
//! phase's job). The rules, from the NAS baseline and the plan:
//!
//! * a few directory workers per mount ([`NETWORK_DIR_WORKERS`]), each listing
//!   a directory under a deadline; size and mtime come from the listing's own
//!   `DirEntry` metadata, never a separate stat pass;
//! * a directory that does not answer is recorded `stalled` (or `denied`,
//!   `error`), never as empty, and its photos are left alone;
//! * a run ends `complete` and sweeps (marks what it did not see `missing`) ONLY
//!   if the walk finished with zero errors, the source was verified online at the
//!   start and at the end, and the mount identity did not change. Anything else
//!   is `incomplete`/`offline` and sweeps nothing;
//! * directory symlinks are recorded, not followed; hidden files are skipped;
//!   names that are not UTF-8 are recorded as unreadable;
//! * the walk checkpoints its frontier, so a killed scan resumes where it was.
//!
//! The file system is a trait ([`ScanFs`]) so tests inject latency, stalls,
//! vanishing directories and a mount that flips offline.

use crate::catalog_db::{
    self, CatalogDb, Counters, DirBatch, DirStatus, FileRec, Finish, Note, RunTotals,
};
use crate::catalog_sources::Source;
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// SMB servers limit concurrent requests and the baseline saw no gain past 2.
pub const NETWORK_DIR_WORKERS: usize = 2;
pub const LOCAL_DIR_WORKERS: usize = 4;
/// A stat (the online check) has this long to answer.
pub const STAT_DEADLINE: Duration = Duration::from_secs(5);
/// A directory listing has this long (the baseline: 10-15 s).
pub const LIST_DEADLINE: Duration = Duration::from_secs(12);
/// Listings that never returned at once, per mount, before the walk gives up
/// (each holds a thread in the kernel).
pub const STUCK_LISTINGS_CAP: usize = 4;
const MAX_DEPTH: u32 = 64;

// ---- the file system the scanner sees --------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryKind {
    File,
    Dir,
    Symlink,
    Other,
}

#[derive(Debug, Clone)]
pub struct FsEntry {
    pub name: OsString,
    pub kind: EntryKind,
    /// Of a file. 0 for the rest.
    pub size: u64,
    /// Of a file or directory: nanoseconds since the epoch.
    pub mtime_ns: i64,
    /// The device a directory is on, to notice a mount below the source.
    pub dev: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsError {
    Timeout,
    Denied,
    NotFound,
    Io(String),
}

impl FsError {
    pub fn status(&self) -> DirStatus {
        match self {
            FsError::Timeout => DirStatus::Stalled,
            FsError::Denied => DirStatus::Denied,
            _ => DirStatus::Error,
        }
    }
    pub fn text(&self, deadline: Duration) -> String {
        match self {
            FsError::Timeout => format!("timed out after {} s", deadline.as_secs()),
            FsError::Denied => "permission denied".into(),
            FsError::NotFound => "not found".into(),
            FsError::Io(e) => e.clone(),
        }
    }
}

pub trait ScanFs: Send + Sync {
    /// The entries of a directory, listed within `deadline`. Files that could be
    /// photos or sidecars carry their size and mtime; directories their mtime.
    fn list(&self, dir: &Path, deadline: Duration) -> Result<Vec<FsEntry>, FsError>;
    /// The directory's own mtime and device.
    fn stat_dir(&self, dir: &Path, deadline: Duration) -> Result<(i64, u64), FsError>;
    /// Verified online right now: the root answers within the stat deadline.
    fn online(&self) -> bool;
    /// What the mount is: it must be the same at the end of a walk as at the start.
    fn identity(&self) -> Option<String>;
}

fn nanos(t: std::io::Result<std::time::SystemTime>) -> i64 {
    t.ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as i64)
}

pub(crate) fn classify_error(e: &std::io::Error) -> FsError {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied => FsError::Denied,
        std::io::ErrorKind::NotFound => FsError::NotFound,
        _ => FsError::Io(e.to_string()),
    }
}

/// The real file system under a source root.
pub struct RealFs {
    pub root: PathBuf,
    /// Listings still running after their deadline passed.
    stuck: Arc<AtomicUsize>,
}

impl RealFs {
    pub fn new(root: PathBuf) -> Self {
        Self { root, stuck: Arc::new(AtomicUsize::new(0)) }
    }

    /// Runs `f` on a helper thread with a deadline. The helper of a call that
    /// times out goes on until the kernel returns it; at most
    /// [`STUCK_LISTINGS_CAP`] such helpers are ever allowed to pile up.
    pub(crate) fn within<T: Send + 'static>(&self, wait: Duration, f: impl FnOnce() -> T + Send + 'static) -> Result<T, FsError> {
        if self.stuck.load(Ordering::Relaxed) >= STUCK_LISTINGS_CAP {
            return Err(FsError::Timeout);
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let timed_out = Arc::new(AtomicBool::new(false));
        let (stuck, flag) = (self.stuck.clone(), timed_out.clone());
        let spawned = std::thread::Builder::new().name("catalog-list".into()).spawn(move || {
            let out = f();
            if flag.load(Ordering::SeqCst) {
                stuck.fetch_sub(1, Ordering::SeqCst);
            }
            let _ = tx.send(out);
        });
        if spawned.is_err() {
            return Err(FsError::Io("cannot start a listing thread".into()));
        }
        match rx.recv_timeout(wait) {
            Ok(v) => Ok(v),
            Err(_) => {
                self.stuck.fetch_add(1, Ordering::SeqCst);
                timed_out.store(true, Ordering::SeqCst);
                // it may have returned between the timeout and the flag
                Err(FsError::Timeout)
            }
        }
    }
}

fn list_now(dir: &Path) -> Result<Vec<FsEntry>, FsError> {
    let rd = std::fs::read_dir(dir).map_err(|e| classify_error(&e))?;
    let mut out = Vec::new();
    for e in rd {
        let e = e.map_err(|e| classify_error(&e))?;
        let Ok(t) = e.file_type() else { continue };
        let name = e.file_name();
        let kind = if t.is_symlink() {
            EntryKind::Symlink
        } else if t.is_dir() {
            EntryKind::Dir
        } else if t.is_file() {
            EntryKind::File
        } else {
            EntryKind::Other
        };
        let (mut size, mut mtime_ns, mut dev) = (0, 0, 0);
        // only what the walk may keep is stat'ed: directories, photos, sidecars. on a
        // network share the listing already carries these, so this is not a round trip
        let wanted = match kind {
            EntryKind::Dir => true,
            EntryKind::File => name.to_str().is_some_and(|n| {
                !n.starts_with('.') && (is_xmp(n) || crate::library_cmds::photo_candidate(Path::new(n)))
            }),
            _ => false,
        };
        if wanted {
            match e.metadata() {
                Ok(m) => {
                    size = if kind == EntryKind::File { m.len() } else { 0 };
                    mtime_ns = nanos(m.modified());
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::MetadataExt;
                        dev = m.dev();
                    }
                }
                // vanished between the listing and the stat: the next scan sees what is there
                Err(_) => continue,
            }
        }
        out.push(FsEntry { name, kind, size, mtime_ns, dev });
    }
    Ok(out)
}

impl ScanFs for RealFs {
    fn list(&self, dir: &Path, deadline: Duration) -> Result<Vec<FsEntry>, FsError> {
        let dir = dir.to_path_buf();
        self.within(deadline, move || list_now(&dir))?
    }

    fn stat_dir(&self, dir: &Path, deadline: Duration) -> Result<(i64, u64), FsError> {
        let dir = dir.to_path_buf();
        self.within(deadline, move || {
            let m = std::fs::metadata(&dir).map_err(|e| classify_error(&e))?;
            #[cfg(unix)]
            let dev = {
                use std::os::unix::fs::MetadataExt;
                m.dev()
            };
            #[cfg(not(unix))]
            let dev = 0;
            Ok((nanos(m.modified()), dev))
        })?
    }

    fn online(&self) -> bool {
        crate::volumes::answering(&self.root) && self.root.is_dir()
    }

    fn identity(&self) -> Option<String> {
        let table = crate::catalog_sources::MountTable::current();
        let e = table.top_at(&self.root)?;
        Some(format!("{}|{}|{}|{}", e.mount_id, e.mount_point, e.fs_type, e.source))
    }
}

// ---- configuration and control ---------------------------------------------

#[derive(Debug, Clone)]
pub struct ScanConfig {
    pub workers: usize,
    pub list_deadline: Duration,
    pub stat_deadline: Duration,
    pub progress_every: Duration,
    pub checkpoint_every: Duration,
    /// Directories between checks that the mount is still there.
    pub online_check_every_dirs: u64,
    pub stuck_cap: usize,
    pub cross_mounts: bool,
}

impl ScanConfig {
    pub fn for_network(network: bool) -> Self {
        Self {
            workers: if network { NETWORK_DIR_WORKERS } else { LOCAL_DIR_WORKERS },
            list_deadline: LIST_DEADLINE,
            stat_deadline: STAT_DEADLINE,
            progress_every: Duration::from_millis(500),
            checkpoint_every: Duration::from_secs(5),
            online_check_every_dirs: 32,
            stuck_cap: STUCK_LISTINGS_CAP,
            cross_mounts: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// The source's root as a path now.
    pub root: PathBuf,
    pub include_subfolders: bool,
    /// Skip directories whose mtime is as stored. A per-source flag that is OFF:
    /// the DS220's directory-mtime test has not passed.
    pub dir_skip: bool,
    /// The user asked for a light scan: skip unchanged directories this time.
    pub light: bool,
    /// Ignore an interrupted run and start a new one.
    pub fresh: bool,
}

pub(crate) const RUN: u8 = 0;
pub(crate) const CANCEL: u8 = 1;
pub(crate) const INTERRUPT: u8 = 2;

/// Pause, resume and cancel from another thread.
#[derive(Default)]
pub struct ScanControl {
    paused: AtomicBool,
    stop: AtomicU8,
    gate: Mutex<()>,
    cv: Condvar,
}

impl ScanControl {
    pub fn pause(&self) {
        self.paused.store(true, Ordering::SeqCst);
    }
    pub fn resume(&self) {
        self.paused.store(false, Ordering::SeqCst);
        let _g = self.gate.lock().unwrap();
        self.cv.notify_all();
    }
    /// Stops for good: the run is closed `cancelled` and does not resume.
    pub fn cancel(&self) {
        self.stop.store(CANCEL, Ordering::SeqCst);
        self.resume();
    }
    /// Stops with the checkpoint kept: a later scan resumes (Ctrl-C).
    pub fn interrupt(&self) {
        self.stop.store(INTERRUPT, Ordering::SeqCst);
        self.resume();
    }
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::SeqCst)
    }
    pub(crate) fn stopped(&self) -> u8 {
        self.stop.load(Ordering::SeqCst)
    }
    /// Blocks while paused; false once the scan was stopped.
    pub(crate) fn proceed(&self) -> bool {
        let mut g = self.gate.lock().unwrap();
        while self.is_paused() && self.stopped() == RUN {
            g = self.cv.wait_timeout(g, Duration::from_millis(200)).unwrap().0;
        }
        self.stopped() == RUN
    }
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct Progress {
    pub source_id: i64,
    pub job_id: i64,
    pub run_id: i64,
    /// `finding` while walking, then `done`.
    pub phase: String,
    pub dirs_done: u64,
    pub dirs_queued: u64,
    pub found: u64,
    pub new: u64,
    pub changed: u64,
    pub unchanged: u64,
    pub errors: u64,
    pub elapsed_s: f64,
    pub rate_per_s: f64,
    /// A range (low, high) in seconds, once the rate is steady and the last
    /// scan's size is known; never while paused.
    pub eta_s: Option<(u64, u64)>,
    pub current_dir: Option<String>,
    pub paused: bool,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ScanReport {
    pub source_id: i64,
    pub job_id: i64,
    pub run_id: i64,
    /// complete | incomplete | offline | cancelled | interrupted | failed
    pub state: &'static str,
    pub totals: RunTotals,
    pub dirs_listed: u64,
    pub files_listed: u64,
    pub elapsed: Duration,
    pub files_per_s: f64,
    pub resumed: bool,
    pub error: Option<String>,
}

// ---- the walk ----------------------------------------------------------------

#[derive(Clone, Debug)]
struct Item {
    rel: String,
    depth: u32,
    mtime_ns: Option<i64>,
    dev: u64,
}

#[derive(Default)]
struct Queue {
    items: Vec<Item>,
    in_flight: HashMap<usize, Item>,
}

struct Known {
    mtime_ns: Option<i64>,
    children: Vec<String>,
}

#[derive(Debug, Clone)]
enum Reason {
    Stopped(u8),
    Offline(String),
    Stuck,
    Failed(String),
}

struct Walk<'a> {
    db: &'a CatalogDb,
    fs: &'a dyn ScanFs,
    cfg: &'a ScanConfig,
    opts: &'a ScanOptions,
    source_id: i64,
    run_id: i64,
    redo: bool,
    ctl: &'a ScanControl,
    counters: Arc<Counters>,
    queue: Mutex<Queue>,
    wake: Condvar,
    abort: Mutex<Option<Reason>>,
    exited: AtomicUsize,
    dirs_done: AtomicU64,
    files_listed: AtomicU64,
    errors: AtomicU64,
    stalls: AtomicU64,
    root_dev: Mutex<Option<u64>>,
    known: Option<HashMap<String, Known>>,
    current: Mutex<Option<String>>,
}

impl Walk<'_> {
    fn abs(&self, rel: &str) -> PathBuf {
        let mut p = self.opts.root.clone();
        for c in rel.split('/').filter(|c| !c.is_empty()) {
            p.push(c);
        }
        p
    }

    fn set_abort(&self, r: Reason) {
        let mut a = self.abort.lock().unwrap();
        if a.is_none() {
            *a = Some(r);
        }
        drop(a);
        self.wake.notify_all();
    }

    fn aborted(&self) -> bool {
        self.abort.lock().unwrap().is_some()
    }

    /// The next directory, or None when the walk is over (or stopped).
    fn take(&self, id: usize) -> Option<Item> {
        let mut q = self.queue.lock().unwrap();
        loop {
            if self.aborted() {
                return None;
            }
            if let Some(item) = q.items.pop() {
                q.in_flight.insert(id, item.clone());
                return Some(item);
            }
            if q.in_flight.is_empty() {
                self.wake.notify_all();
                return None;
            }
            q = self.wake.wait_timeout(q, Duration::from_millis(100)).unwrap().0;
        }
    }

    /// Puts an unfinished directory back, so a checkpoint still has it.
    fn give_back(&self, id: usize) {
        let mut q = self.queue.lock().unwrap();
        if let Some(item) = q.in_flight.remove(&id) {
            q.items.push(item);
        }
        self.wake.notify_all();
    }

    /// Children go on the queue and the directory off the in-flight list in one step.
    fn finish(&self, id: usize, children: Vec<Item>) {
        let mut q = self.queue.lock().unwrap();
        q.items.extend(children);
        q.in_flight.remove(&id);
        drop(q);
        self.wake.notify_all();
    }

    fn frontier(&self) -> Vec<Item> {
        let q = self.queue.lock().unwrap();
        q.items.iter().chain(q.in_flight.values()).cloned().collect()
    }

    fn worker(&self, id: usize) {
        while let Some(item) = self.take(id) {
            if !self.ctl.proceed() {
                self.give_back(id);
                self.set_abort(Reason::Stopped(self.ctl.stopped()));
                break;
            }
            let n = self.dirs_done.load(Ordering::Relaxed);
            if self.cfg.online_check_every_dirs > 0 && n.is_multiple_of(self.cfg.online_check_every_dirs) && !self.fs.online() {
                self.give_back(id);
                self.set_abort(Reason::Offline("the source stopped answering during the scan".into()));
                break;
            }
            *self.current.lock().unwrap() = Some(item.rel.clone());
            let children = self.process(&item);
            self.finish(id, children);
        }
        self.exited.fetch_add(1, Ordering::SeqCst);
        self.wake.notify_all();
    }

    fn send(&self, b: DirBatch) {
        if let Err(e) = self.db.send_dir(b) {
            self.set_abort(Reason::Failed(e));
        }
    }

    fn batch(&self, rel: &str, mtime_ns: Option<i64>, status: DirStatus, error: Option<String>) -> DirBatch {
        DirBatch {
            source_id: self.source_id,
            run_id: self.run_id,
            rel_path: rel.to_string(),
            parent_rel: parent_of(rel),
            mtime_ns,
            entry_count: 0,
            status,
            error,
            files: Vec::new(),
            skipped: false,
            redo: self.redo,
            counters: self.counters.clone(),
        }
    }

    fn process(&self, item: &Item) -> Vec<Item> {
        let abs = self.abs(&item.rel);
        let mut mtime = item.mtime_ns;
        // fingerprint skipping: a directory whose mtime is as stored is not listed
        if let Some(known) = &self.known {
            if mtime.is_none() {
                mtime = self.fs.stat_dir(&abs, self.cfg.stat_deadline).ok().map(|s| s.0);
            }
            if let (Some(m), Some(k)) = (mtime, known.get(&item.rel)) {
                if k.mtime_ns == Some(m) {
                    let mut b = self.batch(&item.rel, Some(m), DirStatus::Ok, None);
                    b.skipped = true;
                    self.send(b);
                    self.dirs_done.fetch_add(1, Ordering::Relaxed);
                    return k
                        .children
                        .iter()
                        .map(|rel| Item { rel: rel.clone(), depth: item.depth + 1, mtime_ns: None, dev: item.dev })
                        .collect();
                }
            }
        }
        let entries = match self.fs.list(&abs, self.cfg.list_deadline) {
            Ok(e) => e,
            Err(e) => {
                self.errors.fetch_add(1, Ordering::Relaxed);
                let mut b = self.batch(&item.rel, mtime, e.status(), Some(e.text(self.cfg.list_deadline)));
                b.mtime_ns = None;
                self.send(b);
                self.dirs_done.fetch_add(1, Ordering::Relaxed);
                if e == FsError::Timeout && self.stalls.fetch_add(1, Ordering::Relaxed) + 1 >= self.cfg.stuck_cap as u64 {
                    self.set_abort(Reason::Stuck);
                }
                return Vec::new();
            }
        };
        if item.rel.is_empty() {
            *self.root_dev.lock().unwrap() = Some(item.dev);
            // a mount that vanished leaves an empty directory behind: an empty root
            // above photos we know of is not trusted
            if entries.is_empty() {
                let known: i64 = self
                    .db
                    .read()
                    .ok()
                    .and_then(|c| {
                        c.query_row(
                            "SELECT COUNT(*) FROM photos WHERE source_id=?1 AND status='present'",
                            [self.source_id],
                            |r| r.get(0),
                        )
                        .ok()
                    })
                    .unwrap_or(0);
                if known > 0 {
                    self.errors.fetch_add(1, Ordering::Relaxed);
                    let msg = format!("the folder listed as empty but {known} photos are known in it");
                    self.send(self.batch("", mtime, DirStatus::Error, Some(msg)));
                    self.dirs_done.fetch_add(1, Ordering::Relaxed);
                    return Vec::new();
                }
            }
        }
        let root_dev = self.root_dev.lock().unwrap().unwrap_or(item.dev);
        let mut notes: Vec<Note> = Vec::new();
        let mut files: Vec<(String, u64, i64)> = Vec::new();
        let mut xmps: HashMap<String, (u64, i64)> = HashMap::new();
        let mut children = Vec::new();
        let count = entries.len();
        for e in entries {
            let Some(name) = e.name.to_str() else {
                if looks_like_photo_bytes(&e.name) || e.kind == EntryKind::Dir {
                    notes.push(Note {
                        dir_rel: item.rel.clone(),
                        name: e.name.to_string_lossy().into_owned(),
                        kind: "non_utf8",
                        detail: Some("the name is not valid UTF-8".into()),
                    });
                }
                continue;
            };
            if name.starts_with('.') {
                continue;
            }
            match e.kind {
                EntryKind::Symlink => notes.push(Note {
                    dir_rel: item.rel.clone(),
                    name: name.to_string(),
                    kind: "symlink",
                    detail: Some("not followed".into()),
                }),
                EntryKind::Dir if self.opts.include_subfolders => {
                    let rel = join(&item.rel, name);
                    if item.depth + 1 > MAX_DEPTH {
                        notes.push(Note { dir_rel: item.rel.clone(), name: name.into(), kind: "too_deep", detail: None });
                    } else if !self.cfg.cross_mounts && e.dev != 0 && root_dev != 0 && e.dev != root_dev {
                        notes.push(Note {
                            dir_rel: item.rel.clone(),
                            name: name.into(),
                            kind: "mount_boundary",
                            detail: Some("another file system, not entered".into()),
                        });
                    } else {
                        children.push(Item { rel, depth: item.depth + 1, mtime_ns: Some(e.mtime_ns), dev: e.dev });
                    }
                }
                EntryKind::File => {
                    if is_xmp(name) {
                        xmps.insert(name.to_ascii_lowercase(), (e.size, e.mtime_ns));
                    } else if crate::library_cmds::photo_candidate(Path::new(name)) {
                        files.push((name.to_string(), e.size, e.mtime_ns));
                    }
                }
                _ => {}
            }
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));
        let recs: Vec<FileRec> = files
            .into_iter()
            .map(|(name, size, mtime_ns)| {
                let ext = name.rsplit_once('.').map_or(String::new(), |(_, x)| x.to_ascii_lowercase());
                let xmp = sidecar_of(&name, &xmps);
                FileRec { name, ext, size, mtime_ns, xmp }
            })
            .collect();
        self.files_listed.fetch_add(recs.len() as u64, Ordering::Relaxed);
        let mut b = self.batch(&item.rel, mtime, DirStatus::Ok, None);
        b.entry_count = count as u32;
        b.files = recs;
        // the directory's row goes first: its children are queued after this
        self.send(b);
        if !notes.is_empty() {
            if let Err(e) = self.db.send_notes(self.source_id, self.run_id, notes) {
                self.set_abort(Reason::Failed(e));
            }
        }
        self.dirs_done.fetch_add(1, Ordering::Relaxed);
        children
    }
}

fn parent_of(rel: &str) -> Option<String> {
    if rel.is_empty() {
        None
    } else {
        Some(rel.rsplit_once('/').map_or(String::new(), |(p, _)| p.to_string()))
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

fn is_xmp(name: &str) -> bool {
    name.len() > 4 && name[name.len() - 4..].eq_ignore_ascii_case(".xmp")
}

#[cfg(unix)]
fn looks_like_photo_bytes(name: &std::ffi::OsStr) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let b = name.as_bytes();
    b.iter().rposition(|c| *c == b'.').is_some_and(|i| {
        std::str::from_utf8(&b[i + 1..]).is_ok_and(|ext| {
            crate::library_cmds::photo_candidate(Path::new(&format!("x.{ext}"))) || ext.eq_ignore_ascii_case("xmp")
        })
    })
}

#[cfg(not(unix))]
fn looks_like_photo_bytes(_name: &std::ffi::OsStr) -> bool {
    true
}

/// `name.ext.xmp` (darktable) first, then `name.xmp` (Lightroom and others).
fn sidecar_of(name: &str, xmps: &HashMap<String, (u64, i64)>) -> Option<(u8, u64, i64)> {
    if xmps.is_empty() {
        return None;
    }
    let lower = name.to_ascii_lowercase();
    if let Some(&(s, m)) = xmps.get(&format!("{lower}.xmp")) {
        return Some((1, s, m));
    }
    let stem = lower.rsplit_once('.').map_or(lower.as_str(), |(s, _)| s);
    xmps.get(&format!("{stem}.xmp")).map(|&(s, m)| (2, s, m))
}

// ---- running a scan ----------------------------------------------------------

#[derive(Serialize, serde::Deserialize, Default)]
struct Checkpoint {
    frontier: Vec<(String, u32)>,
    dirs_done: u64,
    files_listed: u64,
    identity: Option<String>,
}

fn io<T>(r: Result<T, impl std::fmt::Display>) -> Result<T, String> {
    r.map_err(|e| e.to_string())
}

/// Walks `source` and records what it finds. Blocks until the walk ends, is
/// stopped through `ctl`, or the source goes away. `on_progress` is called from
/// the calling thread, throttled to `cfg.progress_every`.
pub fn run_scan(
    db: &CatalogDb,
    fs: &dyn ScanFs,
    source: &Source,
    opts: &ScanOptions,
    cfg: &ScanConfig,
    ctl: &ScanControl,
    on_progress: &mut dyn FnMut(&Progress),
) -> Result<ScanReport, String> {
    let started = Instant::now();
    let sid = source.id;
    // verified online before anything is created or touched
    if !fs.online() {
        io(db.exec(move |c| {
            c.execute("UPDATE sources SET state='offline' WHERE id=?1", [sid]).map(|_| ()).map_err(|e| e.to_string())
        }))?;
        return Ok(ScanReport {
            source_id: sid,
            job_id: 0,
            run_id: 0,
            state: "offline",
            totals: RunTotals::default(),
            dirs_listed: 0,
            files_listed: 0,
            elapsed: started.elapsed(),
            files_per_s: 0.0,
            resumed: false,
            error: Some("the source is not reachable".into()),
        });
    }
    let identity0 = fs.identity();
    // an interrupted run of this source continues, if it is the same mount
    let mode = if opts.light { "light" } else { "full" };
    let prior = io(db.read()).and_then(|c| io(catalog_db::interrupted_job(&c, sid)))?;
    let prior = prior.filter(|_| !opts.fresh).and_then(|j| {
        let cp: Checkpoint = serde_json::from_str(j.checkpoint.as_deref()?).ok()?;
        (cp.identity == identity0).then_some((j, cp))
    });
    let (job_id, run_id, resumed, cp) = match prior {
        Some((j, cp)) => {
            let run = j.run_id.unwrap_or(0);
            io(db.exec(move |c| catalog_db::resume_run(c, j.id, run, sid).map_err(|e| e.to_string())))?;
            (j.id, run, true, cp)
        }
        None => {
            let payload = serde_json::json!({"light": opts.light}).to_string();
            let (j, r) = io(db.exec(move |c| {
                let j = catalog_db::create_job(c, "scan", sid, &payload).map_err(|e| e.to_string())?;
                let r = catalog_db::start_run(c, sid, j, mode).map_err(|e| e.to_string())?;
                Ok((j, r))
            }))?;
            (j, r, false, Checkpoint::default())
        }
    };
    // what the last scan found, for the ETA, and each directory's fingerprint for skipping
    let (expected, known) = {
        let c = io(db.read())?;
        let expected: Option<u64> = c
            .query_row(
                "SELECT found FROM scan_runs WHERE source_id=?1 AND state='complete' ORDER BY id DESC LIMIT 1",
                [sid],
                |r| r.get::<_, i64>(0),
            )
            .ok()
            .map(|n| n as u64)
            .filter(|n| *n > 0);
        let known = if opts.dir_skip || opts.light { Some(load_known(&c, sid)?) } else { None };
        (expected, known)
    };
    let counters = Arc::new(Counters::default());
    let walk = Walk {
        db,
        fs,
        cfg,
        opts,
        source_id: sid,
        run_id,
        redo: resumed,
        ctl,
        counters: counters.clone(),
        queue: Mutex::new(Queue::default()),
        wake: Condvar::new(),
        abort: Mutex::new(None),
        exited: AtomicUsize::new(0),
        dirs_done: AtomicU64::new(cp.dirs_done),
        files_listed: AtomicU64::new(cp.files_listed),
        errors: AtomicU64::new(0),
        stalls: AtomicU64::new(0),
        root_dev: Mutex::new(None),
        known,
        current: Mutex::new(None),
    };
    {
        let mut q = walk.queue.lock().unwrap();
        if resumed && !cp.frontier.is_empty() {
            q.items = cp.frontier.iter().map(|(rel, depth)| Item { rel: rel.clone(), depth: *depth, mtime_ns: None, dev: 0 }).collect();
            // directories that failed before the stop (a mount that went away fails
            // everything in flight) are tried again; the rest were done
            let c = io(db.read())?;
            let mut st = c
                .prepare("SELECT rel_path FROM dirs WHERE source_id=?1 AND seen_run=?2 AND status IN ('stalled','denied','error')")
                .map_err(|e| e.to_string())?;
            let failed: Vec<String> = st
                .query_map([sid, run_id], |r| r.get(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<_, _>>()
                .map_err(|e| e.to_string())?;
            for rel in failed {
                if !q.items.iter().any(|i| i.rel == rel) {
                    let depth = rel.split('/').filter(|c| !c.is_empty()).count() as u32;
                    q.items.push(Item { rel, depth, mtime_ns: None, dev: 0 });
                }
            }
        } else {
            let (mtime, dev) = match fs.stat_dir(&opts.root, cfg.stat_deadline) {
                Ok((m, d)) => (Some(m), d),
                Err(_) => (None, 0),
            };
            q.items.push(Item { rel: String::new(), depth: 0, mtime_ns: mtime, dev });
        }
    }
    let mut last_progress = Instant::now() - cfg.progress_every;
    let mut last_checkpoint = Instant::now();
    let samples: std::cell::RefCell<VecDeque<(Instant, u64)>> = std::cell::RefCell::new(VecDeque::new());
    let snapshot = |paused: bool| -> Progress {
        let elapsed = started.elapsed().as_secs_f64();
        let listed = walk.files_listed.load(Ordering::Relaxed);
        let now = Instant::now();
        let rate = match samples.borrow().front() {
            Some((t, n)) if now > *t => (listed.saturating_sub(*n)) as f64 / (now - *t).as_secs_f64(),
            _ => 0.0,
        };
        let eta_s = match expected {
            Some(total) if !paused && rate > 5.0 && elapsed > 3.0 && total > listed => {
                let left = (total - listed) as f64 / rate;
                Some(((left * 0.7) as u64, (left * 1.6).ceil() as u64))
            }
            _ => None,
        };
        let q = walk.queue.lock().unwrap();
        Progress {
            source_id: sid,
            job_id,
            run_id,
            phase: "finding".into(),
            dirs_done: walk.dirs_done.load(Ordering::Relaxed),
            dirs_queued: (q.items.len() + q.in_flight.len()) as u64,
            found: listed,
            new: counters.new.load(Ordering::Relaxed),
            changed: counters.changed.load(Ordering::Relaxed),
            unchanged: counters.unchanged.load(Ordering::Relaxed),
            errors: walk.errors.load(Ordering::Relaxed),
            elapsed_s: elapsed,
            rate_per_s: rate,
            eta_s,
            current_dir: walk.current.lock().unwrap().clone(),
            paused,
        }
    };
    let save = |state: &str, prog: &Progress| -> Result<(), String> {
        let frontier = walk.frontier();
        let cp = Checkpoint {
            frontier: frontier.into_iter().map(|i| (i.rel, i.depth)).collect(),
            dirs_done: prog.dirs_done,
            files_listed: prog.found,
            identity: identity0.clone(),
        };
        let (cp, pr, st) = (serde_json::to_string(&cp).map_err(|e| e.to_string())?, serde_json::to_string(prog).map_err(|e| e.to_string())?, state.to_string());
        db.exec(move |c| catalog_db::save_checkpoint(c, job_id, &cp, &pr, &st).map_err(|e| e.to_string()))
    };

    let workers = cfg.workers.max(1);
    std::thread::scope(|scope| {
        for id in 0..workers {
            let w = &walk;
            scope.spawn(move || w.worker(id));
        }
        // the monitor: progress, checkpoints, the writer's health
        loop {
            {
                let q = walk.queue.lock().unwrap();
                let (_q, _) = walk.wake.wait_timeout(q, Duration::from_millis(100)).unwrap();
            }
            if let Some(e) = db.failure() {
                walk.set_abort(Reason::Failed(e));
            }
            let now = Instant::now();
            {
                let mut s = samples.borrow_mut();
                if s.back().is_none_or(|(t, _)| now.duration_since(*t) >= Duration::from_millis(500)) {
                    s.push_back((now, walk.files_listed.load(Ordering::Relaxed)));
                    while s.len() > 12 {
                        s.pop_front();
                    }
                }
            }
            let paused = ctl.is_paused();
            if now.duration_since(last_progress) >= cfg.progress_every {
                last_progress = now;
                on_progress(&snapshot(paused));
            }
            if now.duration_since(last_checkpoint) >= cfg.checkpoint_every {
                last_checkpoint = now;
                let _ = save(if paused { "paused" } else { "running" }, &snapshot(paused));
            }
            if walk.exited.load(Ordering::SeqCst) >= workers {
                break;
            }
        }
    });

    // how it ended. everything the walk sent is committed first, so the
    // database (not a counter) says whether any directory failed: that also
    // covers a run resumed after a kill
    db.sync()?;
    let reason = walk.abort.lock().unwrap().clone();
    let final_progress = snapshot(false);
    let mut error: Option<String> = None;
    let state: &'static str = match &reason {
        Some(Reason::Stopped(CANCEL)) => "cancelled",
        Some(Reason::Stopped(_)) => "interrupted",
        Some(Reason::Offline(m)) => {
            error = Some(m.clone());
            "offline"
        }
        Some(Reason::Stuck) => {
            error = Some("too many directories did not answer".into());
            "incomplete"
        }
        Some(Reason::Failed(m)) => {
            error = Some(m.clone());
            "failed"
        }
        None => {
            // zero errors, still online, same mount: only then is the walk the whole truth
            let identity1 = fs.identity();
            let failed_dirs: i64 = io(db.read())?
                .query_row(
                    "SELECT COUNT(*) FROM dirs WHERE source_id=?1 AND seen_run=?2 AND status IN ('stalled','denied','error')",
                    [sid, run_id],
                    |r| r.get(0),
                )
                .map_err(|e| e.to_string())?;
            if !fs.online() || identity1 != identity0 {
                error = Some("the source went away or changed during the scan".into());
                "offline"
            } else if failed_dirs > 0 {
                "incomplete"
            } else {
                "complete"
            }
        }
    };
    if matches!(state, "interrupted" | "offline") {
        // a resumable stop keeps where it was
        save("interrupted", &final_progress)?;
    }
    let sweep = state == "complete";
    let totals = db.finish_run(Finish { source_id: sid, run_id, job_id: Some(job_id), state, sweep, error: error.clone() })?;
    let elapsed = started.elapsed();
    let files_listed = walk.files_listed.load(Ordering::Relaxed);
    let mut last = final_progress;
    last.phase = "done".into();
    on_progress(&last);
    Ok(ScanReport {
        source_id: sid,
        job_id,
        run_id,
        state,
        totals,
        dirs_listed: walk.dirs_done.load(Ordering::Relaxed),
        files_listed,
        elapsed,
        files_per_s: files_listed as f64 / elapsed.as_secs_f64().max(0.001),
        resumed,
        error,
    })
}

fn load_known(c: &rusqlite::Connection, source_id: i64) -> Result<HashMap<String, Known>, String> {
    let mut st = c
        .prepare("SELECT rel_path, mtime_ns, status FROM dirs WHERE source_id=?1 AND status='ok'")
        .map_err(|e| e.to_string())?;
    let rows = st
        .query_map([source_id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<i64>>(1)?)))
        .map_err(|e| e.to_string())?;
    let mut map: HashMap<String, Known> = HashMap::new();
    for row in rows {
        let (rel, m) = row.map_err(|e| e.to_string())?;
        map.insert(rel, Known { mtime_ns: m, children: Vec::new() });
    }
    let rels: Vec<String> = map.keys().filter(|r| !r.is_empty()).cloned().collect();
    for rel in rels {
        let parent = parent_of(&rel).unwrap_or_default();
        if let Some(p) = map.get_mut(&parent) {
            p.children.push(rel);
        }
    }
    Ok(map)
}

#[cfg(test)]
#[path = "catalog_scan_tests.rs"]
mod tests;
