// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The catalog's thumbnail job: after the metadata phase, makes a small JPEG
//! for each photo with the fewest and smallest reads, keeps it in the thumbnail
//! store and links it into the grid's disk cache (see [`crate::catalog_thumbs`]).
//!
//! Where the pixels come from, cheapest first, per photo:
//!
//! 1. **The embedded preview** of a RAW (ARW, DNG, NEF, CR2, CR3, RAF, ...): the
//!    metadata phase stored its offset and length, so one ranged read of that span
//!    (about 400 KB for a Sony ARW) is all the NAS sees. No second read of the head.
//! 2. **A JPEG's Exif thumbnail**, if it is at least 256 px on the long edge: one
//!    read of the first 80 KB.
//! 3. **The JPEG itself**, read whole and decoded, only when the policy allows
//!    ([`WholeJpegs`]: `no`, `idle`, `yes`; the default `idle` reads it when the app
//!    is not being used for browsing) or when the file is small (768 KB). Most
//!    cameras' Exif thumbnails are 160 px, so this is the common JPEG case and the
//!    expensive one: no crate in the tree decodes a JPEG at 1/8 scale, and a 19 MB
//!    JPEG is 19 MB over the network.
//! 4. **The engine's thumbnail** through a hook the app provides, for HEIC, TIFF and
//!    previews that do not decode. The CLI has no engine: those photos are recorded
//!    `deferred` (`engine`) or `failed`, never dropped.
//!
//! Reads and decodes run on separate pools: a few readers per mount (2 on a
//! network share, 5 s deadline, adaptive back-off as in the metadata job) and
//! cores-1 decoders, with a small bounded queue between them. Workers run at low
//! CPU and I/O priority and yield while the app says the user is browsing.
//!
//! A read or decode error is recorded per photo (`failed`, retried after an hour,
//! a day, then a week). Nothing here changes the `sources` or `photos` tables: a
//! volume that stops answering ends the job `offline` with its progress kept, and
//! never marks a photo missing.

use crate::cache::DiskCache;
use crate::catalog_db::{self, CatalogDb};
use crate::catalog_meta_job::{Adaptive, MetaFs};
use crate::catalog_scan::{FsError, ScanControl, CANCEL, RUN};
use crate::catalog_sources::Source;
use crate::catalog_thumb_src::{self as src, Render, Src};
use crate::catalog_thumbs::{self as thumbs, GcReport, Signals, Stamps, Store, StoreError, ThumbConfig, WholeJpegs};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// Consecutive read failures before the job asks whether the volume is still there.
const BREAKER: usize = 16;
pub const JOB_KIND: &str = "thumbnails";

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct ThumbProgress {
    pub source_id: i64,
    pub job_id: i64,
    /// `publishing` (linking what is already made), `making`, then `done`.
    pub phase: String,
    pub total: u64,
    pub done: u64,
    pub failed: u64,
    pub deferred: u64,
    /// Photos with a current thumbnail already (not touched).
    pub skipped: u64,
    pub bytes_read: u64,
    pub elapsed_s: f64,
    pub rate_per_s: f64,
    pub eta_s: Option<(u64, u64)>,
    /// Readers allowed right now (the adaptive limit).
    pub workers: u64,
    pub decoders: u64,
    pub p95_ms: Option<f64>,
    pub paused: bool,
    /// The job is waiting because the app is busy browsing.
    pub yielding: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Checkpoint {
    total: u64,
    done: u64,
    failed: u64,
    deferred: u64,
    bytes: u64,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ThumbReport {
    pub source_id: i64,
    pub job_id: i64,
    /// complete | cancelled | interrupted | offline | disk_full | failed
    pub state: &'static str,
    pub total: u64,
    pub done: u64,
    pub failed: u64,
    pub deferred: u64,
    pub skipped: u64,
    /// Rows a rescan changed while the thumbnail was being made; left pending.
    pub raced: u64,
    /// Thumbnails made from a preview, an Exif thumbnail, a whole JPEG, the engine.
    pub from_preview: u64,
    pub from_exif: u64,
    pub from_jpeg: u64,
    pub from_engine: u64,
    pub linked: u64,
    pub reads: u64,
    pub bytes_read: u64,
    /// What the new thumbnails take on disk.
    pub thumb_bytes: u64,
    pub elapsed: Duration,
    pub per_s: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub gc: Option<GcReport>,
    pub resumed: bool,
    pub error: Option<String>,
}

// ---- the rows ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Work {
    pub id: i64,
    pub rel: String,
    pub ext: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub xmp_kind: u8,
    pub xmp_size: Option<i64>,
    pub xmp_mtime_ns: Option<i64>,
    pub container: Option<String>,
    pub orientation: u8,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub preview_offset: Option<u64>,
    pub preview_len: Option<u64>,
    pub edited: bool,
}

impl Work {
    fn stamps(&self) -> Stamps {
        Stamps { size: self.size, mtime_ns: self.mtime_ns, xmp_kind: self.xmp_kind, xmp_size: self.xmp_size, xmp_mtime_ns: self.xmp_mtime_ns }
    }
    fn is_jpeg(&self) -> bool {
        matches!(self.ext.as_str(), "jpg" | "jpeg" | "jpe" | "jfif") || self.container.as_deref() == Some("jpeg")
    }
}

/// The SQL that selects photos that need a thumbnail: metadata read (or not
/// understood), no thumbnail row or one for another version of the file, a
/// failure whose retry time has come, or a deferral the configuration now lifts.
fn pending_sql(cfg: &ThumbConfig) -> String {
    let now = catalog_db::now_s();
    let retry = if cfg.retry_failed { format!(" OR (t.state=2 AND COALESCE(t.retry_at,0)<={now})") } else { String::new() };
    let policy = if cfg.whole_jpegs != WholeJpegs::No { " OR (t.state=3 AND t.error='policy')" } else { "" };
    let engine = if cfg.engine.is_some() { " OR (t.state=3 AND t.error='engine')" } else { "" };
    format!(
        "p.source_id=?1 AND p.status='present' AND p.meta_state IN (1,3) AND (t.photo_id IS NULL
           OR t.stamp_size IS NOT p.size OR t.stamp_mtime_ns IS NOT p.mtime_ns{retry}{policy}{engine})"
    )
}

/// Photo ids to do, the newest folders first (folders are usually named by date,
/// and photos of one folder sit together on the volume).
fn pending_ids(c: &Connection, sid: i64, cfg: &ThumbConfig) -> rusqlite::Result<VecDeque<i64>> {
    let sql = format!(
        "SELECT p.id FROM photos p LEFT JOIN thumbs t ON t.photo_id=p.id JOIN dirs d ON d.id=p.dir_id WHERE {} ORDER BY d.rel_path DESC, p.name ASC",
        pending_sql(cfg)
    );
    let mut st = c.prepare(&sql)?;
    let v = st.query_map([sid], |r| r.get::<_, i64>(0))?;
    v.collect()
}

fn load_work(c: &Connection, ids: &[i64]) -> rusqlite::Result<Vec<Work>> {
    let mut st = c.prepare_cached(
        "SELECT id, rel_path, ext, size, mtime_ns, xmp_kind, xmp_size, xmp_mtime_ns, container, orientation, width, height,
                preview_offset, preview_len, COALESCE(edited,0) FROM photos WHERE id=?1",
    )?;
    let mut by_id: HashMap<i64, Work> = HashMap::new();
    for id in ids {
        let w = st
            .query_row([id], |r| {
                Ok(Work {
                    id: r.get(0)?,
                    rel: r.get(1)?,
                    ext: r.get(2)?,
                    size: r.get::<_, i64>(3)?.max(0) as u64,
                    mtime_ns: r.get(4)?,
                    xmp_kind: r.get::<_, i64>(5)? as u8,
                    xmp_size: r.get(6)?,
                    xmp_mtime_ns: r.get(7)?,
                    container: r.get(8)?,
                    orientation: r.get::<_, Option<i64>>(9)?.map_or(1, |o| o.clamp(1, 8) as u8),
                    width: r.get::<_, Option<i64>>(10)?.map(|v| v.max(0) as u32),
                    height: r.get::<_, Option<i64>>(11)?.map(|v| v.max(0) as u32),
                    preview_offset: r.get::<_, Option<i64>>(12)?.map(|v| v.max(0) as u64),
                    preview_len: r.get::<_, Option<i64>>(13)?.map(|v| v.max(0) as u64),
                    edited: r.get::<_, i64>(14)? == 1,
                })
            })
            .optional()?;
        if let Some(w) = w {
            by_id.insert(w.id, w);
        }
    }
    Ok(ids.iter().filter_map(|i| by_id.remove(i)).collect())
}

// ---- reading -------------------------------------------------------------------------------

struct Stats {
    reads: AtomicU64,
    bytes: AtomicU64,
    consecutive_failures: AtomicUsize,
    consecutive_timeouts: AtomicUsize,
    lat_all: Mutex<Vec<f32>>,
    made: [AtomicU64; 4],
    thumb_bytes: AtomicU64,
    linked: AtomicU64,
}

enum Fetch {
    Ready(Vec<u8>, Src),
    /// Nothing in Rust can decode it: the engine hook, or `deferred`.
    NeedEngine,
    Defer(&'static str),
    Fail(String, bool),
}

enum Outcome {
    Made { w: Work, src: Src, width: u32, height: u32, bytes: u64, published: Option<String> },
    Failed { w: Work, reason: String, read_error: bool },
    Deferred { w: Work, why: &'static str },
    /// Not recorded: the work stopped before it could be done (the disk is full).
    Dropped,
}

struct Ready {
    w: Work,
    bytes: Vec<u8>,
    src: Src,
}

fn is_read_error(e: &FsError) -> bool {
    matches!(e, FsError::Timeout | FsError::Io(_))
}

struct Ctx<'a> {
    fs: &'a dyn MetaFs,
    cfg: &'a ThumbConfig,
    sig: &'a Signals,
    stats: &'a Stats,
    adaptive: &'a Mutex<Adaptive>,
}

impl Ctx<'_> {
    fn read(&self, rel: &str, off: u64, len: usize, deadline: Duration, stop: &dyn Fn() -> bool) -> Result<Vec<u8>, FsError> {
        let mut tries = 0;
        loop {
            let t = Instant::now();
            let r = self.fs.read_at(rel, off, len, deadline);
            let ms = t.elapsed().as_secs_f64() * 1000.0;
            self.stats.reads.fetch_add(1, Ordering::Relaxed);
            let lat = if matches!(r, Err(FsError::Timeout)) { deadline.as_secs_f64() * 1000.0 } else { ms };
            self.adaptive.lock().unwrap().record(lat);
            self.stats.lat_all.lock().unwrap().push(lat as f32);
            match &r {
                Ok(b) => {
                    self.stats.bytes.fetch_add(b.len() as u64, Ordering::Relaxed);
                    self.stats.consecutive_failures.store(0, Ordering::Relaxed);
                    self.stats.consecutive_timeouts.store(0, Ordering::Relaxed);
                    return r;
                }
                Err(FsError::Timeout | FsError::Io(_)) if tries == 0 && !stop() => {
                    tries += 1;
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(e) => {
                    self.stats.consecutive_failures.fetch_add(1, Ordering::Relaxed);
                    if matches!(e, FsError::Timeout) {
                        self.stats.consecutive_timeouts.fetch_add(1, Ordering::Relaxed);
                    } else {
                        self.stats.consecutive_timeouts.store(0, Ordering::Relaxed);
                    }
                    return r;
                }
            }
        }
    }

    /// A deadline that grows with the size: the base for latency plus time to move
    /// the bytes at a slow 2 MB/s.
    fn deadline_for(&self, len: u64) -> Duration {
        self.cfg.read_deadline + Duration::from_millis(len * 1000 / (2 * 1024 * 1024))
    }

    fn whole_allowed(&self, len: u64) -> bool {
        len <= self.cfg.max_whole
            && match self.cfg.whole_jpegs {
                WholeJpegs::No => false,
                WholeJpegs::Idle => !self.sig.is_busy(),
                WholeJpegs::Yes => true,
            }
    }

    fn fetch(&self, w: &Work, stop: &dyn Fn() -> bool) -> Fetch {
        let fail = |e: FsError| Fetch::Fail(e.text(self.cfg.read_deadline), is_read_error(&e));
        // 1. the embedded preview: one ranged read of exactly its span
        if let (Some(off), Some(len)) = (w.preview_offset, w.preview_len) {
            if len > 0 {
                if off.saturating_add(len) > w.size {
                    return Fetch::Fail("the preview lies outside the file".into(), false);
                }
                if len > 8 * 1024 * 1024 && !self.whole_allowed(len) {
                    return Fetch::Defer("policy");
                }
                if len > self.cfg.max_whole {
                    return Fetch::Defer("engine");
                }
                return match self.read(&w.rel, off, len as usize, self.deadline_for(len), stop) {
                    Ok(b) if b.len() as u64 == len && b.starts_with(&[0xFF, 0xD8]) => Fetch::Ready(b, Src::Preview),
                    Ok(b) if b.len() as u64 != len => Fetch::Fail(format!("short read of the preview ({} of {len} bytes)", b.len()), false),
                    Ok(_) => Fetch::Fail("the preview is not a JPEG".into(), false),
                    Err(e) => fail(e),
                };
            }
        }
        if !w.is_jpeg() {
            return Fetch::NeedEngine;
        }
        // small JPEGs: one read of everything, whatever the policy
        let first = if w.size <= self.cfg.small_jpeg { w.size } else { self.cfg.exif_head as u64 }.min(w.size) as usize;
        let head = match self.read(&w.rel, 0, first, self.deadline_for(first as u64), stop) {
            Ok(b) => b,
            Err(e) => return fail(e),
        };
        if head.len() < first {
            return Fetch::Fail(format!("short read ({} of {first} bytes)", head.len()), false);
        }
        // 2. the Exif thumbnail, if it is big enough to be worth showing
        if let Some(t) = src::exif_thumbnail(&head) {
            if t.width.max(t.height) >= self.cfg.min_exif_edge {
                return Fetch::Ready(head[t.offset..t.offset + t.len].to_vec(), Src::Exif);
            }
        }
        if head.len() as u64 >= w.size {
            return Fetch::Ready(head, Src::Jpeg);
        }
        // 3. the whole JPEG, if the policy allows
        if !self.whole_allowed(w.size) {
            return Fetch::Defer("policy");
        }
        let rest = w.size - head.len() as u64;
        match self.read(&w.rel, head.len() as u64, rest as usize, self.deadline_for(rest), stop) {
            Ok(r) if r.len() as u64 == rest => {
                let mut all = head;
                all.extend_from_slice(&r);
                Fetch::Ready(all, Src::Jpeg)
            }
            Ok(r) => Fetch::Fail(format!("short read ({} of {rest} bytes)", r.len()), false),
            Err(e) => fail(e),
        }
    }
}

// ---- the run ------------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Reason {
    Offline,
    Full,
    Failed(String),
}

struct Queue {
    items: VecDeque<Work>,
    closed: bool,
}

struct Run<'a> {
    ctx: Ctx<'a>,
    ctl: &'a ScanControl,
    store: &'a Store,
    cache: Option<&'a DiskCache>,
    abs_root: &'a Path,
    queue: Mutex<Queue>,
    wake: Condvar,
    abort: Mutex<Option<Reason>>,
    active: Mutex<usize>,
    gate: Condvar,
    allowed: AtomicUsize,
}

impl Run<'_> {
    fn set_abort(&self, r: Reason) {
        let mut a = self.abort.lock().unwrap();
        if a.is_none() {
            *a = Some(r);
        }
        drop(a);
        self.wake.notify_all();
        self.gate.notify_all();
    }

    fn aborted(&self) -> bool {
        self.abort.lock().unwrap().is_some()
    }

    fn stopped(&self) -> bool {
        self.aborted() || self.ctl.stopped() != RUN
    }

    fn take(&self) -> Option<Work> {
        let mut q = self.queue.lock().unwrap();
        loop {
            if self.stopped() {
                return None;
            }
            if let Some(w) = q.items.pop_front() {
                self.wake.notify_all();
                return Some(w);
            }
            if q.closed {
                return None;
            }
            q = self.wake.wait_timeout(q, Duration::from_millis(50)).unwrap().0;
        }
    }

    fn enter(&self) -> bool {
        let mut a = self.active.lock().unwrap();
        while *a >= self.allowed.load(Ordering::SeqCst) {
            if self.stopped() {
                return false;
            }
            a = self.gate.wait_timeout(a, Duration::from_millis(50)).unwrap().0;
        }
        *a += 1;
        true
    }

    fn leave(&self) {
        *self.active.lock().unwrap() -= 1;
        self.gate.notify_all();
    }

    /// Waits while the user is browsing, unless the job was stopped.
    fn yield_to_browsing(&self) {
        while self.ctx.sig.is_busy() && !self.stopped() {
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Sends to a bounded queue without blocking past a stop: decoders leave when the job is
    /// stopped, so a reader waiting on a full queue would never be released.
    fn send_unless_stopped(&self, tx: &SyncSender<Ready>, mut v: Ready) -> bool {
        loop {
            match tx.try_send(v) {
                Ok(()) => return true,
                Err(TrySendError::Full(back)) => {
                    if self.stopped() {
                        return false;
                    }
                    v = back;
                    std::thread::sleep(Duration::from_millis(2));
                }
                Err(TrySendError::Disconnected(_)) => return false,
            }
        }
    }

    fn reader(&self, dec: SyncSender<Ready>, res: SyncSender<Outcome>) {
        if self.ctx.cfg.lower_priority {
            thumbs::lower_priority();
        }
        loop {
            if !self.ctl.proceed() {
                break;
            }
            self.yield_to_browsing();
            let Some(w) = self.take() else { break };
            if !self.enter() {
                break;
            }
            let f = self.ctx.fetch(&w, &|| self.stopped());
            self.leave();
            let sent = match f {
                Fetch::Ready(bytes, s) => self.send_unless_stopped(&dec, Ready { w, bytes, src: s }),
                Fetch::NeedEngine if self.ctx.cfg.engine.is_some() => self.send_unless_stopped(&dec, Ready { w, bytes: Vec::new(), src: Src::Engine }),
                Fetch::NeedEngine => res.send(Outcome::Deferred { w, why: "engine" }).is_ok(),
                Fetch::Defer(why) => res.send(Outcome::Deferred { w, why }).is_ok(),
                Fetch::Fail(reason, read_error) => res.send(Outcome::Failed { w, reason, read_error }).is_ok(),
            };
            if !sent {
                break;
            }
        }
    }

    fn decoder(&self, rx: &Mutex<Receiver<Ready>>, res: SyncSender<Outcome>) {
        if self.ctx.cfg.lower_priority {
            thumbs::lower_priority();
        }
        loop {
            let got = rx.lock().unwrap().recv_timeout(Duration::from_millis(50));
            match got {
                Ok(job) => {
                    if res.send(self.make(job)).is_err() {
                        break;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {
                    if self.stopped() {
                        break;
                    }
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    }

    fn make(&self, job: Ready) -> Outcome {
        let Ready { w, bytes, src: from } = job;
        let cfg = self.ctx.cfg;
        let own = Render { edge: cfg.edge, quality: cfg.quality, orientation: w.orientation, main: w.width.zip(w.height), own_pixels: from == Src::Jpeg };
        let abs = self.abs_root.join(&w.rel);
        let mut from = from;
        let mut result = if from == Src::Engine { Err("no engine thumbnail".to_string()) } else { src::render(&bytes, &own) };
        if result.is_err() {
            // a preview that does not decode, or a format only the engine reads
            if let Some(hook) = &cfg.engine {
                if let Some(b) = hook(&abs.to_string_lossy()) {
                    result = src::render(&b, &Render { orientation: 1, main: None, own_pixels: true, ..own });
                    from = Src::Engine;
                }
            }
        }
        let t = match result {
            Ok(t) => t,
            Err(reason) => return Outcome::Failed { w, reason, read_error: false },
        };
        if let Err(e) = self.store.write(w.id, &t.jpeg) {
            return match e {
                StoreError::Full => {
                    self.set_abort(Reason::Full);
                    Outcome::Dropped
                }
                StoreError::Io(m) => Outcome::Failed { w, reason: format!("store: {m}"), read_error: false },
            };
        }
        let published = self.cache.and_then(|c| thumbs::publish_one(self.store, c, w.id, &abs, &w.stamps(), w.edited));
        let st = self.ctx.stats;
        st.made[from as usize].fetch_add(1, Ordering::Relaxed);
        st.thumb_bytes.fetch_add(t.jpeg.len() as u64, Ordering::Relaxed);
        if published.is_some() {
            st.linked.fetch_add(1, Ordering::Relaxed);
        }
        Outcome::Made { w, src: from, width: t.width, height: t.height, bytes: t.jpeg.len() as u64, published }
    }
}

#[derive(Default)]
struct Written {
    done: u64,
    failed: u64,
    deferred: u64,
    raced: u64,
}

/// One transaction. A thumbnail is written only if the photo still has the size and
/// mtime it had when it was read; a rescan that changed it leaves the row to the next run.
fn write_batch(c: &mut Connection, batch: Vec<Outcome>) -> Result<Written, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let mut out = Written::default();
    let tx = c.transaction().map_err(e)?;
    let now = catalog_db::now_s();
    {
        let mut ok = tx
            .prepare_cached(
                "INSERT INTO thumbs(photo_id,state,src,w,h,bytes,stamp_size,stamp_mtime_ns,made_at,used_at,tries,error,retry_at,published)
                 SELECT ?1,1,?2,?3,?4,?5,?6,?7,?8,?8,0,NULL,NULL,?9 WHERE EXISTS(SELECT 1 FROM photos WHERE id=?1 AND size=?6 AND mtime_ns=?7)
                 ON CONFLICT(photo_id) DO UPDATE SET state=1, src=excluded.src, w=excluded.w, h=excluded.h, bytes=excluded.bytes,
                   stamp_size=excluded.stamp_size, stamp_mtime_ns=excluded.stamp_mtime_ns, made_at=excluded.made_at, used_at=excluded.used_at,
                   tries=0, error=NULL, retry_at=NULL, published=excluded.published",
            )
            .map_err(e)?;
        let mut bad = tx
            .prepare_cached(
                "INSERT INTO thumbs(photo_id,state,src,w,h,bytes,stamp_size,stamp_mtime_ns,made_at,used_at,tries,error,retry_at,published)
                 SELECT ?1,?2,NULL,NULL,NULL,NULL,?3,?4,?5,?5,?7,?6,?8,NULL WHERE EXISTS(SELECT 1 FROM photos WHERE id=?1 AND size=?3 AND mtime_ns=?4)
                 ON CONFLICT(photo_id) DO UPDATE SET state=excluded.state, error=excluded.error, stamp_size=excluded.stamp_size,
                   stamp_mtime_ns=excluded.stamp_mtime_ns, used_at=excluded.used_at,
                   tries=CASE WHEN excluded.state=2 THEN thumbs.tries+1 ELSE 0 END,
                   retry_at=CASE WHEN excluded.state=2 THEN ?5 + (CASE thumbs.tries WHEN 0 THEN 3600 WHEN 1 THEN 86400 ELSE 604800 END) ELSE NULL END",
            )
            .map_err(e)?;
        for o in batch {
            match o {
                Outcome::Made { w, src, width, height, bytes, published } => {
                    let n = ok
                        .execute(params![w.id, src.word(), width, height, bytes as i64, w.size as i64, w.mtime_ns, now, published])
                        .map_err(e)?;
                    if n == 0 { out.raced += 1 } else { out.done += 1 }
                }
                Outcome::Failed { w, reason, .. } => {
                    let n = bad.execute(params![w.id, 2, w.size as i64, w.mtime_ns, now, reason, 1, now + 3600]).map_err(e)?;
                    if n == 0 { out.raced += 1 } else { out.failed += 1 }
                }
                Outcome::Dropped => {}
                Outcome::Deferred { w, why } => {
                    let n = bad.execute(params![w.id, 3, w.size as i64, w.mtime_ns, now, why, 0, Option::<i64>::None]).map_err(e)?;
                    if n == 0 { out.raced += 1 } else { out.deferred += 1 }
                }
            }
        }
    }
    tx.commit().map_err(e)?;
    Ok(out)
}

fn percentile(v: &[f32], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    s[((s.len() - 1) as f64 * p).round() as usize] as f64
}

/// Makes the thumbnail of every photo of `source` that needs one. Blocks until the
/// job ends, is stopped through `ctl`, or the volume goes away. `on_progress` is
/// called from the calling thread, throttled to `cfg.progress_every`. `abs_root` is
/// where the source is mounted now (the cache names use the photo's absolute path).
#[allow(clippy::too_many_arguments)]
pub fn run_thumbnails(
    db: &CatalogDb,
    fs: &dyn MetaFs,
    source: &Source,
    abs_root: &Path,
    store: &Store,
    cache: Option<&DiskCache>,
    cfg: &ThumbConfig,
    ctl: &ScanControl,
    sig: &Signals,
    on_progress: &mut dyn FnMut(&ThumbProgress),
) -> Result<ThumbReport, String> {
    let started = Instant::now();
    let sid = source.id;
    // linking into the grid's cache is the job's to switch off
    let cache = if cfg.publish { cache } else { None };
    let blank = |state: &'static str, error: Option<String>| ThumbReport {
        source_id: sid, job_id: 0, state, total: 0, done: 0, failed: 0, deferred: 0, skipped: 0, raced: 0, from_preview: 0, from_exif: 0,
        from_jpeg: 0, from_engine: 0, linked: 0, reads: 0, bytes_read: 0, thumb_bytes: 0, elapsed: started.elapsed(), per_s: 0.0, p50_ms: 0.0,
        p95_ms: 0.0, gc: None, resumed: false, error,
    };
    if !fs.online() {
        return Ok(blank("offline", Some("the source is not reachable".into())));
    }
    let prior = {
        let c = db.read().map_err(|e| e.to_string())?;
        c.query_row(
            "SELECT id, checkpoint FROM jobs WHERE source_id=?1 AND kind=?2 AND state='interrupted' ORDER BY id DESC LIMIT 1",
            params![sid, JOB_KIND],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
    };
    let (job_id, resumed, cp) = match prior {
        Some((id, cp)) => {
            let cp: Checkpoint = cp.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
            db.exec(move |c| c.execute("UPDATE jobs SET state='running', updated_at=?2 WHERE id=?1", params![id, catalog_db::now_s()]).map(|_| ()).map_err(|e| e.to_string()))?;
            (id, true, cp)
        }
        None => {
            let id = db.exec(move |c| {
                let id = catalog_db::create_job(c, JOB_KIND, sid, "{}").map_err(|e| e.to_string())?;
                c.execute("UPDATE jobs SET state='running', started_at=?2, updated_at=?2 WHERE id=?1", params![id, catalog_db::now_s()]).map_err(|e| e.to_string())?;
                Ok(id)
            })?;
            (id, false, Checkpoint::default())
        }
    };
    let skipped = {
        let c = db.read().map_err(|e| e.to_string())?;
        thumbs::coverage(&c, Some(sid)).map_err(|e| e.to_string())?.done.max(0) as u64
    };
    let mut ids = {
        let c = db.read().map_err(|e| e.to_string())?;
        pending_ids(&c, sid, cfg).map_err(|e| e.to_string())?
    };
    let total = ids.len() as u64 + cp.done + cp.failed + cp.deferred;
    let workers = cfg.read_workers.max(1);
    let decoders = cfg.decode_workers.max(1);
    let stats = Stats {
        reads: AtomicU64::new(0),
        bytes: AtomicU64::new(cp.bytes),
        consecutive_failures: AtomicUsize::new(0),
        consecutive_timeouts: AtomicUsize::new(0),
        lat_all: Mutex::new(Vec::new()),
        made: Default::default(),
        thumb_bytes: AtomicU64::new(0),
        linked: AtomicU64::new(0),
    };
    let adaptive = Mutex::new(Adaptive::new(workers));
    let run = Run {
        ctx: Ctx { fs, cfg, sig, stats: &stats, adaptive: &adaptive },
        ctl,
        store,
        cache,
        abs_root,
        queue: Mutex::new(Queue { items: VecDeque::new(), closed: false }),
        wake: Condvar::new(),
        abort: Mutex::new(None),
        active: Mutex::new(0),
        gate: Condvar::new(),
        allowed: AtomicUsize::new(workers),
    };
    let mut counts = Written { done: cp.done, failed: cp.failed, deferred: cp.deferred, raced: 0 };
    let mut buffer: Vec<Outcome> = Vec::new();
    let (mut last_flush, mut last_progress, mut last_checkpoint, mut last_tick) =
        (Instant::now(), Instant::now() - cfg.progress_every, Instant::now(), Instant::now());
    let mut samples: VecDeque<(Instant, u64)> = VecDeque::new();
    let mut breaker_at = 0usize;
    let mut hints_seen = 0u64;
    let mut exhausted = ids.is_empty();
    let reader_db = db.read().map_err(|e| e.to_string())?;
    let snapshot = |counts: &Written, paused: bool, phase: &str, samples: &VecDeque<(Instant, u64)>| -> ThumbProgress {
        let done_now = counts.done + counts.failed + counts.deferred;
        let elapsed = started.elapsed().as_secs_f64();
        let rate = match samples.front() {
            Some((t, n)) if Instant::now() > *t => (done_now.saturating_sub(*n)) as f64 / (Instant::now() - *t).as_secs_f64(),
            _ => 0.0,
        };
        let eta_s = (!paused && rate > 1.0 && elapsed > 2.0 && total > done_now).then(|| {
            let left = (total - done_now) as f64 / rate;
            ((left * 0.7) as u64, (left * 1.6).ceil() as u64)
        });
        ThumbProgress {
            source_id: sid, job_id, phase: phase.into(), total, done: counts.done, failed: counts.failed, deferred: counts.deferred, skipped,
            bytes_read: stats.bytes.load(Ordering::Relaxed), elapsed_s: elapsed, rate_per_s: rate, eta_s,
            workers: run.allowed.load(Ordering::SeqCst) as u64, decoders: decoders as u64, p95_ms: adaptive.lock().unwrap().p95(), paused,
            yielding: sig.is_busy(),
        }
    };
    let save = |counts: &Written, state: &str, prog: &ThumbProgress| -> Result<(), String> {
        let cp = Checkpoint { total, done: counts.done, failed: counts.failed, deferred: counts.deferred, bytes: prog.bytes_read };
        let (cp, pr, st) = (serde_json::to_string(&cp).map_err(|e| e.to_string())?, serde_json::to_string(prog).map_err(|e| e.to_string())?, state.to_string());
        db.exec(move |c| catalog_db::save_checkpoint(c, job_id, &cp, &pr, &st).map_err(|e| e.to_string()))
    };
    // thumbnails made earlier whose cache link is missing (a move, an eviction)
    let mut linked_before = 0u64;
    if cfg.publish {
        if let Some(cache) = cache {
            on_progress(&snapshot(&counts, false, "publishing", &samples));
            linked_before = thumbs::publish_source(db, store, cache, source, abs_root)?.linked;
        }
    }
    let flush = |buffer: &mut Vec<Outcome>, counts: &mut Written| -> Result<(), String> {
        if buffer.is_empty() {
            return Ok(());
        }
        let mut batch = std::mem::take(buffer);
        // a read error counts only if the volume still answers
        if batch.iter().any(|o| matches!(o, Outcome::Failed { read_error: true, .. })) && (!fs.online() || matches!(*run.abort.lock().unwrap(), Some(Reason::Offline))) {
            run.set_abort(Reason::Offline);
            batch.retain(|o| !matches!(o, Outcome::Failed { read_error: true, .. }));
        }
        let w = db.exec(move |c| write_batch(c, batch))?;
        counts.done += w.done;
        counts.failed += w.failed;
        counts.deferred += w.deferred;
        counts.raced += w.raced;
        Ok(())
    };
    let mut write_error: Option<String> = None;
    let (dec_tx, dec_rx) = sync_channel::<Ready>(decoders * 2 + 2);
    let dec_rx = Mutex::new(dec_rx);
    let (res_tx, res_rx) = sync_channel::<Outcome>(cfg.batch_rows.max(16));
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let (d, r, run) = (dec_tx.clone(), res_tx.clone(), &run);
            scope.spawn(move || run.reader(d, r));
        }
        for _ in 0..decoders {
            let (r, run, rx) = (res_tx.clone(), &run, &dec_rx);
            scope.spawn(move || run.decoder(rx, r));
        }
        drop(dec_tx);
        drop(res_tx);
        loop {
            // what the app is looking at goes first
            if let Some(paths) = sig.changed(&mut hints_seen) {
                let mut want: Vec<i64> = Vec::new();
                for p in paths {
                    if let Ok(rel) = Path::new(&p).strip_prefix(abs_root) {
                        let rel = rel.to_string_lossy().replace('\\', "/");
                        if let Ok(Some(id)) = reader_db.query_row("SELECT id FROM photos WHERE source_id=?1 AND rel_path=?2", params![sid, rel], |r| r.get::<_, i64>(0)).optional() {
                            want.push(id);
                        }
                    }
                }
                let mut q = run.queue.lock().unwrap();
                for id in want.into_iter().rev() {
                    if let Some(pos) = q.items.iter().position(|w| w.id == id) {
                        let w = q.items.remove(pos).unwrap();
                        q.items.push_front(w);
                    } else if let Some(pos) = ids.iter().position(|i| *i == id) {
                        ids.remove(pos);
                        if let Ok(mut v) = load_work(&reader_db, &[id]) {
                            if let Some(w) = v.pop() {
                                q.items.push_front(w);
                            }
                        }
                    }
                }
                drop(q);
                run.wake.notify_all();
            }
            // keep the queue filled from the catalog
            if !exhausted && run.queue.lock().unwrap().items.len() < cfg.chunk / 2 {
                let n = cfg.chunk.min(ids.len());
                let take: Vec<i64> = ids.drain(..n).collect();
                match load_work(&reader_db, &take) {
                    Ok(chunk) => run.queue.lock().unwrap().items.extend(chunk),
                    Err(e) => run.set_abort(Reason::Failed(e.to_string())),
                }
                if ids.is_empty() {
                    exhausted = true;
                    run.queue.lock().unwrap().closed = true;
                }
                run.wake.notify_all();
            }
            if exhausted && !run.queue.lock().unwrap().closed {
                run.queue.lock().unwrap().closed = true;
                run.wake.notify_all();
            }
            match res_rx.recv_timeout(Duration::from_millis(50)) {
                Ok(o) => {
                    buffer.push(o);
                    while let Ok(o) = res_rx.try_recv() {
                        buffer.push(o);
                        if buffer.len() >= cfg.batch_rows {
                            break;
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    if let Err(e) = flush(&mut buffer, &mut counts) {
                        write_error = Some(e);
                    }
                    break;
                }
            }
            let now = Instant::now();
            if buffer.len() >= cfg.batch_rows || (!buffer.is_empty() && now.duration_since(last_flush) >= cfg.flush_every) {
                last_flush = now;
                if let Err(e) = flush(&mut buffer, &mut counts) {
                    write_error = Some(e.clone());
                    run.set_abort(Reason::Failed(e));
                }
            }
            if let Some(e) = db.failure() {
                run.set_abort(Reason::Failed(e));
            }
            let fails = stats.consecutive_failures.load(Ordering::Relaxed);
            if fails >= BREAKER && fails >= breaker_at + BREAKER {
                breaker_at = fails;
                if !fs.online() || stats.consecutive_timeouts.load(Ordering::Relaxed) >= BREAKER {
                    run.set_abort(Reason::Offline);
                }
            }
            if fails == 0 {
                breaker_at = 0;
            }
            if cfg.adaptive && now.duration_since(last_tick) >= Duration::from_millis(500) {
                last_tick = now;
                let n = adaptive.lock().unwrap().tick(now);
                if n != run.allowed.load(Ordering::SeqCst) {
                    run.allowed.store(n, Ordering::SeqCst);
                    run.gate.notify_all();
                }
            }
            if samples.back().is_none_or(|(t, _)| now.duration_since(*t) >= Duration::from_millis(500)) {
                samples.push_back((now, counts.done + counts.failed + counts.deferred));
                while samples.len() > 12 {
                    samples.pop_front();
                }
            }
            let paused = ctl.is_paused();
            if now.duration_since(last_progress) >= cfg.progress_every {
                last_progress = now;
                on_progress(&snapshot(&counts, paused, "making", &samples));
            }
            if now.duration_since(last_checkpoint) >= cfg.checkpoint_every {
                last_checkpoint = now;
                let _ = save(&counts, if paused { "paused" } else { "running" }, &snapshot(&counts, paused, "making", &samples));
            }
        }
    });
    drop(reader_db);
    db.sync()?;
    let abort = run.abort.lock().unwrap().clone();
    let mut error = write_error;
    let state: &'static str = match &abort {
        Some(Reason::Failed(m)) => {
            error = Some(m.clone());
            "failed"
        }
        Some(Reason::Offline) => {
            error = Some("the source stopped answering".into());
            "offline"
        }
        Some(Reason::Full) => {
            error = Some("the disk is nearly full".into());
            "disk_full"
        }
        None => match ctl.stopped() {
            RUN => "complete",
            CANCEL => "cancelled",
            _ => "interrupted",
        },
    };
    let last = snapshot(&counts, false, "making", &samples);
    if matches!(state, "interrupted" | "offline" | "disk_full") {
        save(&counts, "interrupted", &last)?;
    } else {
        let (st, err) = (if state == "complete" { "done" } else if state == "cancelled" { "cancelled" } else { "failed" }, error.clone());
        let pr = serde_json::to_string(&last).map_err(|e| e.to_string())?;
        db.exec(move |c| {
            c.execute(
                "UPDATE jobs SET state=?2, error=?3, progress=?4, checkpoint=NULL, updated_at=?5, finished_at=?5 WHERE id=?1",
                params![job_id, st, err, pr, catalog_db::now_s()],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })?;
    }
    // the store back under its limit, and without files nothing points to
    let gc = if matches!(state, "complete" | "cancelled" | "interrupted") { thumbs::gc(db, store).ok() } else { None };
    let lat = stats.lat_all.lock().unwrap().clone();
    let elapsed = started.elapsed();
    let mut fin = snapshot(&counts, false, "done", &samples);
    fin.phase = "done".into();
    on_progress(&fin);
    let processed = counts.done + counts.failed + counts.deferred - (cp.done + cp.failed + cp.deferred);
    let made = |s: Src| stats.made[s as usize].load(Ordering::Relaxed);
    Ok(ThumbReport {
        source_id: sid,
        job_id,
        state,
        total,
        done: counts.done,
        failed: counts.failed,
        deferred: counts.deferred,
        skipped,
        raced: counts.raced,
        from_preview: made(Src::Preview),
        from_exif: made(Src::Exif),
        from_jpeg: made(Src::Jpeg),
        from_engine: made(Src::Engine),
        linked: stats.linked.load(Ordering::Relaxed) + linked_before,
        reads: stats.reads.load(Ordering::Relaxed),
        bytes_read: stats.bytes.load(Ordering::Relaxed),
        thumb_bytes: stats.thumb_bytes.load(Ordering::Relaxed),
        elapsed,
        per_s: processed as f64 / elapsed.as_secs_f64().max(0.001),
        p50_ms: percentile(&lat, 0.5),
        p95_ms: percentile(&lat, 0.95),
        gc,
        resumed,
        error,
    })
}
