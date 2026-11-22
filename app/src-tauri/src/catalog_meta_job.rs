// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The catalog's metadata job: after a scan, reads the head of every new or
//! changed photo (64 KB first, escalating only when the parser asks), parses it
//! with [`crate::catalog_meta`], reads the small XMP sidecar, and writes the
//! typed columns through the writer thread in batches.
//!
//! The rules, from the NAS baseline (header reads over SMB are latency bound:
//! about 22 ms for 64 KB, 43 ms for 256 KB, no gain past 2 workers):
//!
//! * a few readers per mount ([`MetaConfig::for_network`]: 2), every read under a
//!   5 s deadline; when the p95 latency rises the job backs off to fewer readers
//!   and creeps back up when it calms ([`Adaptive`]);
//! * only photos with `meta_state` 0 (new or changed since the scan) are read,
//!   plus failed ones from an earlier run, plus the sidecar alone of a photo whose
//!   sidecar changed. A photo whose size and mtime are unchanged and is done is
//!   never opened;
//! * a read error is recorded per photo (`failed`, with the reason, retried by
//!   the next run); a file the extractor cannot read is `unparsed` (for a later
//!   engine fallback). Read errors never mark a source offline or a photo
//!   missing: if the volume stops answering the job stops `offline` with its
//!   checkpoint and the failures of that streak are not written;
//! * pause, resume and cancel work like a scan's ([`ScanControl`]); progress is
//!   throttled; a checkpoint every few seconds lets a restart continue.
//!
//! Files are read through [`MetaFs`], which tests replace to inject latency,
//! failures and a mount that goes away.

use crate::catalog_db::{self, CatalogDb};
use crate::catalog_meta::{extract, Extract, Meta, HEAD_LADDER};
use crate::catalog_meta_xmp::{self, Xmp};
use crate::catalog_relink::{relink_source, RelinkReport};
use crate::catalog_scan::{classify_error, FsError, RealFs, ScanControl, ScanFs, CANCEL, RUN};
use crate::catalog_sources::Source;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{sync_channel, RecvTimeoutError, SyncSender};
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

/// Readers on a network mount; the baseline saw no gain past 2.
pub const NETWORK_META_WORKERS: usize = 2;
pub const LOCAL_META_WORKERS: usize = 4;
/// Each read has this long.
pub const READ_DEADLINE: Duration = Duration::from_secs(5);
/// The part of a file's start the content key hashes: always the same size, so
/// the key does not depend on how large a window a run happened to read.
pub const KEY_HASH_BYTES: usize = 64 * 1024;
/// Consecutive read failures before the job asks whether the volume is still there.
const BREAKER: usize = 16;

// ---- reading files ------------------------------------------------------------------

/// What the metadata job needs of a file system, beside [`ScanFs`]'s `online()`.
pub trait MetaFs: ScanFs {
    /// Up to `len` bytes at `offset` of the file `rel` (relative to the source
    /// root), within `deadline`. Shorter at the end of the file.
    fn read_at(&self, rel: &str, offset: u64, len: usize, deadline: Duration) -> Result<Vec<u8>, FsError>;
}

#[cfg(unix)]
fn pread(path: &std::path::Path, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
    use std::os::unix::fs::FileExt;
    #[cfg(target_os = "linux")]
    use std::os::unix::fs::OpenOptionsExt;
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    // reading headers must not rewrite access times of originals (not permitted on files
    // the user does not own: that open fails with EPERM and is retried without)
    #[cfg(target_os = "linux")]
    let f = {
        opts.custom_flags(libc::O_NOATIME);
        match opts.open(path) {
            Ok(f) => f,
            Err(e) if e.raw_os_error() == Some(libc::EPERM) => std::fs::File::open(path)?,
            Err(e) => return Err(e),
        }
    };
    #[cfg(not(target_os = "linux"))]
    let f = opts.open(path)?;
    let mut buf = vec![0u8; len];
    let mut got = 0;
    while got < len {
        match f.read_at(&mut buf[got..], offset + got as u64) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    buf.truncate(got);
    Ok(buf)
}

#[cfg(not(unix))]
fn pread(path: &std::path::Path, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
    use std::os::windows::fs::FileExt;
    let f = std::fs::File::open(path)?;
    let mut buf = vec![0u8; len];
    let mut got = 0;
    while got < len {
        match f.seek_read(&mut buf[got..], offset + got as u64) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(e) => return Err(e),
        }
    }
    buf.truncate(got);
    Ok(buf)
}

impl MetaFs for RealFs {
    fn read_at(&self, rel: &str, offset: u64, len: usize, deadline: Duration) -> Result<Vec<u8>, FsError> {
        let path: PathBuf = self.root.join(rel);
        self.within(deadline, move || pread(&path, offset, len).map_err(|e| classify_error(&e)))?
    }
}

// ---- configuration, control, progress ---------------------------------------------------

#[derive(Debug, Clone)]
pub struct MetaConfig {
    pub workers: usize,
    /// The first read of each file; clamped to at least [`KEY_HASH_BYTES`].
    pub first_read: usize,
    /// The largest window an escalation may read.
    pub max_read: usize,
    pub read_deadline: Duration,
    pub batch_rows: usize,
    pub flush_every: Duration,
    pub progress_every: Duration,
    pub checkpoint_every: Duration,
    /// Back off when latency rises.
    pub adaptive: bool,
    /// Retry photos that failed in an earlier run.
    pub retry_failed: bool,
    pub sidecars: bool,
    /// Relink renames and moves at the end of a complete run.
    pub relink: bool,
    /// Rows loaded from the catalog at a time.
    pub chunk: usize,
}

impl MetaConfig {
    pub fn for_network(network: bool) -> Self {
        Self {
            workers: if network { NETWORK_META_WORKERS } else { LOCAL_META_WORKERS },
            first_read: HEAD_LADDER[0],
            max_read: HEAD_LADDER[2],
            read_deadline: READ_DEADLINE,
            batch_rows: 500,
            flush_every: Duration::from_secs(1),
            progress_every: Duration::from_millis(250),
            checkpoint_every: Duration::from_secs(5),
            adaptive: true,
            retry_failed: true,
            sidecars: true,
            relink: true,
            chunk: 2000,
        }
    }

    fn steps(&self) -> Vec<usize> {
        let first = self.first_read.max(KEY_HASH_BYTES);
        let mut v = vec![first];
        v.extend(HEAD_LADDER.iter().copied().filter(|n| *n > first && *n <= self.max_read));
        v
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct MetaProgress {
    pub source_id: i64,
    pub job_id: i64,
    /// `reading` while it runs, `linking` while renames are relinked, then `done`.
    pub phase: String,
    /// Photos to read in this job (including those already done, on a resume).
    pub total: u64,
    pub done: u64,
    pub failed: u64,
    pub unparsed: u64,
    /// Photos left alone because they are unchanged and already read.
    pub skipped: u64,
    pub bytes_read: u64,
    pub elapsed_s: f64,
    pub rate_per_s: f64,
    pub eta_s: Option<(u64, u64)>,
    /// Readers allowed right now (the adaptive limit).
    pub workers: u64,
    pub p95_ms: Option<f64>,
    pub paused: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct MetaCheckpoint {
    after_id: i64,
    total: u64,
    done: u64,
    failed: u64,
    unparsed: u64,
    bytes: u64,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct MetaReport {
    pub source_id: i64,
    pub job_id: i64,
    /// complete | cancelled | interrupted | offline | failed
    pub state: &'static str,
    pub total: u64,
    pub done: u64,
    pub sidecar_only: u64,
    pub failed: u64,
    pub unparsed: u64,
    pub skipped: u64,
    /// Rows a rescan changed while they were being read; left pending.
    pub raced: u64,
    pub reads: u64,
    pub escalations: u64,
    pub bytes_read: u64,
    pub elapsed: Duration,
    pub per_s: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub relink: RelinkReport,
    pub resumed: bool,
    pub error: Option<String>,
}

// ---- adaptive concurrency ------------------------------------------------------------------

/// Backs the number of readers off when read latency rises and raises it again
/// when it calms. Pure: time and latencies are fed in, so tests can drive it.
pub struct Adaptive {
    max: usize,
    allowed: usize,
    window: VecDeque<f64>,
    baseline: Option<f64>,
    last_change: Option<Instant>,
    calm_since: Option<Instant>,
}

impl Adaptive {
    /// A back-off needs the p95 to exceed this much, whatever the baseline.
    pub const FLOOR_MS: f64 = 100.0;
    const WINDOW: usize = 64;

    pub fn new(max: usize) -> Self {
        Self { max: max.max(1), allowed: max.max(1), window: VecDeque::new(), baseline: None, last_change: None, calm_since: None }
    }

    #[cfg(test)]
    pub fn allowed(&self) -> usize {
        self.allowed
    }

    pub fn record(&mut self, ms: f64) {
        if self.window.len() >= Self::WINDOW {
            self.window.pop_front();
        }
        self.window.push_back(ms);
    }

    fn percentile(&self, p: f64) -> Option<f64> {
        if self.window.is_empty() {
            return None;
        }
        let mut v: Vec<f64> = self.window.iter().copied().collect();
        v.sort_by(|a, b| a.total_cmp(b));
        Some(v[((v.len() - 1) as f64 * p).round() as usize])
    }

    pub fn p95(&self) -> Option<f64> {
        self.percentile(0.95)
    }

    /// Looks at the window and returns the number of readers to allow.
    pub fn tick(&mut self, now: Instant) -> usize {
        if self.window.len() < 16 {
            return self.allowed;
        }
        let (p50, p95) = (self.percentile(0.5).unwrap_or(0.0), self.percentile(0.95).unwrap_or(0.0));
        // the best median seen is what this volume costs when it is not struggling
        if self.window.len() >= 32 {
            self.baseline = Some(self.baseline.map_or(p50, |b| b.min(p50)));
        }
        let base = self.baseline.unwrap_or(p50).max(0.01);
        let hold = self.last_change.is_some_and(|t| now.duration_since(t) < Duration::from_secs(5));
        if p95 > Self::FLOOR_MS.max(6.0 * base) {
            self.calm_since = None;
            if !hold && self.allowed > 1 {
                self.allowed -= 1;
                self.last_change = Some(now);
                // measure the new state, not the old one
                self.window.clear();
            }
        } else if p95 < 3.0 * base.max(Self::FLOOR_MS / 6.0) {
            let since = *self.calm_since.get_or_insert(now);
            if self.allowed < self.max && now.duration_since(since) >= Duration::from_secs(10) && !hold {
                self.allowed += 1;
                self.last_change = Some(now);
                self.calm_since = Some(now);
            }
        } else {
            self.calm_since = None;
        }
        self.allowed
    }
}

// ---- the rows ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct Work {
    id: i64,
    rel: String,
    ext: String,
    size: u64,
    mtime_ns: i64,
    xmp_kind: u8,
    xmp_size: Option<i64>,
    xmp_mtime_ns: Option<i64>,
    /// Read the head (a new, changed or failed photo) rather than the sidecar alone.
    full: bool,
}

#[derive(Debug, Clone, Default)]
struct Row {
    taken_at: Option<i64>,
    subsec_ms: Option<i64>,
    tz_offset: Option<i64>,
    make: Option<String>,
    model: Option<String>,
    body_serial: Option<String>,
    lens: Option<String>,
    focal: Option<f64>,
    focal35: Option<f64>,
    aperture: Option<f64>,
    iso: Option<i64>,
    exposure: Option<f64>,
    exposure_bias: Option<f64>,
    width: Option<i64>,
    height: Option<i64>,
    raw_width: Option<i64>,
    raw_height: Option<i64>,
    orientation: Option<i64>,
    preview_offset: Option<i64>,
    preview_len: Option<i64>,
    preview_w: Option<i64>,
    preview_h: Option<i64>,
    gps_lat: Option<f64>,
    gps_lon: Option<f64>,
    gps_alt: Option<f64>,
    software: Option<String>,
    container: Option<String>,
    is_raw: Option<i64>,
    rating: Option<i64>,
    label: Option<i64>,
    rejected: Option<i64>,
    edited: Option<i64>,
    content_key: Option<String>,
    extra_json: Option<String>,
    head_bytes: i64,
}

/// The XMP-derived columns: the sidecar's values win over the file's own XMP.
fn apply_xmp(row: &mut Row, x: &Xmp) {
    if let Some(r) = x.rating {
        if r < 0 {
            row.rating = Some(0);
            row.rejected = Some(1);
        } else {
            row.rating = Some(r.min(5) as i64);
            row.rejected = Some(0);
        }
    }
    if let Some(l) = x.labels {
        row.label = Some(l as i64);
    }
    if let Some(e) = x.edited {
        row.edited = Some(e as i64);
    }
}

fn content_key(head: &[u8], w: &Work, taken_at: Option<i64>, serial: Option<&str>) -> String {
    let mut h = Sha256::new();
    h.update(w.size.to_le_bytes());
    h.update(w.mtime_ns.to_le_bytes());
    h.update(&head[..head.len().min(KEY_HASH_BYTES)]);
    h.update(taken_at.unwrap_or(i64::MIN).to_le_bytes());
    h.update(serial.unwrap_or("").as_bytes());
    h.finalize().iter().take(16).map(|b| format!("{b:02x}")).collect()
}

fn row_from(meta: &Meta, key: String, head_bytes: usize) -> Row {
    let mut r = Row {
        taken_at: meta.taken_at,
        subsec_ms: meta.subsec_ms.map(i64::from),
        tz_offset: meta.tz_offset.map(i64::from),
        make: meta.make.clone(),
        model: meta.model.clone(),
        body_serial: meta.body_serial.clone(),
        lens: meta.lens.clone(),
        focal: meta.focal,
        focal35: meta.focal35,
        aperture: meta.aperture,
        iso: meta.iso.map(i64::from),
        exposure: meta.exposure,
        exposure_bias: meta.bias,
        width: meta.width.map(i64::from),
        height: meta.height.map(i64::from),
        raw_width: meta.raw_width.map(i64::from),
        raw_height: meta.raw_height.map(i64::from),
        orientation: meta.orientation.map(i64::from),
        preview_offset: meta.preview.map(|p| p.offset as i64),
        preview_len: meta.preview.map(|p| p.len as i64),
        preview_w: meta.preview.and_then(|p| p.width).map(i64::from),
        preview_h: meta.preview.and_then(|p| p.height).map(i64::from),
        gps_lat: meta.gps.as_ref().map(|g| g.lat),
        gps_lon: meta.gps.as_ref().map(|g| g.lon),
        gps_alt: meta.gps.as_ref().and_then(|g| g.alt),
        software: meta.software.clone(),
        container: Some(meta.container.to_string()),
        is_raw: Some(meta.is_raw as i64),
        content_key: Some(key),
        head_bytes: head_bytes as i64,
        ..Default::default()
    };
    if meta.truncated {
        r.extra_json = Some(r#"{"truncated":true}"#.into());
    }
    if let Some(x) = &meta.xmp {
        apply_xmp(&mut r, x);
    }
    r
}

enum Sidecar {
    /// No sidecar file (the scan saw one that is gone now) or nothing in it.
    Absent,
    Parsed(Box<Xmp>),
    TooBig,
    /// A read error: leave the sidecar to the next run.
    Error(String),
}

fn sidecar_names(rel: &str, kind: u8) -> Vec<String> {
    let base = if kind == 1 {
        rel.to_string()
    } else {
        let name_at = rel.rfind('/').map_or(0, |i| i + 1);
        match rel[name_at..].rfind('.') {
            Some(i) => rel[..name_at + i].to_string(),
            None => rel.to_string(),
        }
    };
    vec![format!("{base}.xmp"), format!("{base}.XMP")]
}

fn read_sidecar(fs: &dyn MetaFs, w: &Work, deadline: Duration) -> Sidecar {
    for name in sidecar_names(&w.rel, w.xmp_kind) {
        match fs.read_at(&name, 0, catalog_meta_xmp::MAX_SIDECAR as usize + 1, deadline) {
            Ok(b) if b.len() as u64 > catalog_meta_xmp::MAX_SIDECAR => return Sidecar::TooBig,
            Ok(b) => return catalog_meta_xmp::parse(&b).map_or(Sidecar::Absent, |x| Sidecar::Parsed(Box::new(x))),
            Err(FsError::NotFound) => continue,
            Err(e) => return Sidecar::Error(e.text(deadline)),
        }
    }
    Sidecar::Absent
}

#[allow(clippy::large_enum_variant)]
enum Outcome {
    Full { w: Work, row: Box<Row>, keywords: Vec<String>, sidecar: SidecarWrite, unparsed: Option<String> },
    Side { w: Work, x: SidecarWrite },
    /// `read_error`: the read itself failed (as opposed to a file that is not a photo).
    Failed { w: Work, reason: String, read_error: bool },
}

/// What to write of a sidecar.
#[derive(Default)]
struct SidecarWrite {
    /// Record the sidecar as read (the signature the scan saw).
    seen: bool,
    xmp: Option<Box<Xmp>>,
    note: Option<&'static str>,
}

// ---- processing one photo ----------------------------------------------------------------

struct Stats {
    reads: AtomicU64,
    escalations: AtomicU64,
    bytes: AtomicU64,
    consecutive_failures: AtomicUsize,
    consecutive_timeouts: AtomicUsize,
    lat_all: Mutex<Vec<f32>>,
}

#[allow(clippy::too_many_arguments)]
fn read_retry(fs: &dyn MetaFs, rel: &str, off: u64, len: usize, cfg: &MetaConfig, st: &Stats, adaptive: &Mutex<Adaptive>, stop: &dyn Fn() -> bool) -> Result<Vec<u8>, FsError> {
    let mut tries = 0;
    loop {
        let t = Instant::now();
        let r = fs.read_at(rel, off, len, cfg.read_deadline);
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        st.reads.fetch_add(1, Ordering::Relaxed);
        // a read that timed out counts for its whole deadline
        let lat = if matches!(r, Err(FsError::Timeout)) { cfg.read_deadline.as_secs_f64() * 1000.0 } else { ms };
        adaptive.lock().unwrap().record(lat);
        st.lat_all.lock().unwrap().push(lat as f32);
        match &r {
            Ok(b) => {
                st.bytes.fetch_add(b.len() as u64, Ordering::Relaxed);
                st.consecutive_failures.store(0, Ordering::Relaxed);
                st.consecutive_timeouts.store(0, Ordering::Relaxed);
                return r;
            }
            Err(FsError::Timeout | FsError::Io(_)) if tries == 0 && !stop() => {
                tries += 1;
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                st.consecutive_failures.fetch_add(1, Ordering::Relaxed);
                if matches!(e, FsError::Timeout) {
                    st.consecutive_timeouts.fetch_add(1, Ordering::Relaxed);
                } else {
                    st.consecutive_timeouts.store(0, Ordering::Relaxed);
                }
                return r;
            }
        }
    }
}

fn is_read_error(e: &FsError) -> bool {
    matches!(e, FsError::Timeout | FsError::Io(_))
}

fn process(fs: &dyn MetaFs, w: Work, cfg: &MetaConfig, steps: &[usize], st: &Stats, adaptive: &Mutex<Adaptive>, stop: &dyn Fn() -> bool) -> Outcome {
    if !w.full {
        let x = match read_sidecar(fs, &w, cfg.read_deadline) {
            Sidecar::Error(e) => return Outcome::Failed { w, reason: format!("sidecar: {e}"), read_error: true },
            Sidecar::Parsed(x) => SidecarWrite { seen: true, xmp: Some(x), note: None },
            Sidecar::TooBig => SidecarWrite { seen: true, xmp: None, note: Some("xmp_too_big") },
            Sidecar::Absent => SidecarWrite { seen: true, xmp: None, note: None },
        };
        return Outcome::Side { w, x };
    }
    let mut buf = match read_retry(fs, &w.rel, 0, steps[0], cfg, st, adaptive, stop) {
        Ok(b) => b,
        Err(e) => return Outcome::Failed { reason: e.text(cfg.read_deadline), read_error: is_read_error(&e), w },
    };
    let mut meta: Option<Meta> = None;
    let mut unparsed: Option<String> = None;
    let mut step = 0;
    loop {
        match extract(&buf, w.size, &w.ext) {
            Extract::Done(m) => {
                meta = Some(*m);
                break;
            }
            Extract::NeedMore { bytes, partial } => {
                // the next ladder step that covers the ask
                let next = steps.iter().enumerate().skip(step + 1).find(|(_, n)| **n as u64 >= bytes);
                match next {
                    // the file is shorter than the window we already have: nothing more exists
                    Some(_) if (buf.len() as u64) >= w.size || buf.len() < steps[step] => {
                        let mut m = *partial;
                        m.truncated = true;
                        meta = Some(m);
                        break;
                    }
                    Some((i, n)) => {
                        st.escalations.fetch_add(1, Ordering::Relaxed);
                        // only the part not read yet
                        match read_retry(fs, &w.rel, buf.len() as u64, n - buf.len(), cfg, st, adaptive, stop) {
                            Ok(more) => {
                                buf.extend_from_slice(&more);
                                step = i;
                            }
                            Err(e) => return Outcome::Failed { reason: e.text(cfg.read_deadline), read_error: is_read_error(&e), w },
                        }
                    }
                    // asked for more than the ladder allows: keep what was found
                    None => {
                        let mut m = *partial;
                        m.truncated = true;
                        meta = Some(m);
                        break;
                    }
                }
            }
            Extract::Unsupported => {
                unparsed = Some("unsupported format".into());
                break;
            }
            Extract::Corrupt(e) => {
                unparsed = Some(e);
                break;
            }
        }
    }
    let head_bytes = buf.len();
    let mut row = match &meta {
        Some(m) => row_from(m, content_key(&buf, &w, m.taken_at, m.body_serial.as_deref()), head_bytes),
        None => Row { content_key: Some(content_key(&buf, &w, None, None)), head_bytes: head_bytes as i64, ..Default::default() },
    };
    let mut keywords: Vec<String> = Vec::new();
    // the file's own XMP first; the sidecar overrides it below
    if let Some(x) = meta.as_ref().and_then(|m| m.xmp.as_ref()) {
        keywords.clone_from(&x.keywords);
    }
    let mut side = SidecarWrite::default();
    if cfg.sidecars && w.xmp_kind != 0 {
        match read_sidecar(fs, &w, cfg.read_deadline) {
            Sidecar::Parsed(x) => {
                apply_xmp(&mut row, &x);
                side = SidecarWrite { seen: true, xmp: Some(x), note: None };
            }
            Sidecar::TooBig => side = SidecarWrite { seen: true, xmp: None, note: Some("xmp_too_big") },
            Sidecar::Absent => side.seen = true,
            // the photo itself was read: its sidecar is simply tried again next run
            Sidecar::Error(_) => {}
        }
    } else if w.xmp_kind == 0 {
        side.seen = true;
    }
    if let Some(note) = side.note {
        let mut o: serde_json::Map<String, serde_json::Value> = row.extra_json.as_deref().and_then(|s| serde_json::from_str(s).ok()).unwrap_or_default();
        o.insert(note.into(), true.into());
        row.extra_json = Some(serde_json::Value::Object(o).to_string());
    }
    keywords.truncate(1000);
    Outcome::Full { w, row: Box::new(row), keywords, sidecar: side, unparsed }
}

// ---- writing a batch --------------------------------------------------------------------

#[derive(Default, Debug, Clone, Copy)]
struct Written {
    done: u64,
    sidecar_only: u64,
    failed: u64,
    unparsed: u64,
    raced: u64,
}

fn write_batch(c: &mut Connection, batch: Vec<Outcome>) -> Result<Written, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let now = catalog_db::now_s();
    let mut out = Written::default();
    let tx = c.transaction().map_err(e)?;
    {
        let mut full = tx
            .prepare_cached(
                "UPDATE photos SET meta_state=?2, meta_error=?3, meta_at=?4, taken_at=?5, subsec_ms=?6, tz_offset=?7, make=?8, model=?9,
                   body_serial=?10, lens=?11, focal=?12, focal35=?13, aperture=?14, iso=?15, exposure=?16, exposure_bias=?17, width=?18,
                   height=?19, raw_width=?20, raw_height=?21, orientation=?22, preview_offset=?23, preview_len=?24, preview_w=?25,
                   preview_h=?26, gps_lat=?27, gps_lon=?28, gps_alt=?29, software=?30, container=?31, is_raw=?32, rating=?33, label=?34,
                   rejected=?35, edited=?36, content_key=?37, extra_json=?38, head_bytes=?39, xmp_seen_size=?40, xmp_seen_mtime_ns=?41
                 WHERE id=?1 AND size=?42 AND mtime_ns=?43",
            )
            .map_err(e)?;
        let mut side = tx
            .prepare_cached(
                "UPDATE photos SET rating=COALESCE(?2,rating), label=COALESCE(?3,label), rejected=COALESCE(?4,rejected), edited=COALESCE(?5,edited),
                   xmp_seen_size=?6, xmp_seen_mtime_ns=?7, meta_at=?8 WHERE id=?1 AND size=?9 AND mtime_ns=?10",
            )
            .map_err(e)?;
        let mut fail = tx
            .prepare_cached("UPDATE photos SET meta_state=?2, meta_error=?3, meta_at=?4 WHERE id=?1 AND size=?5 AND mtime_ns=?6")
            .map_err(e)?;
        let mut del_kw = tx.prepare_cached("DELETE FROM keywords WHERE photo_id=?1 AND src=?2").map_err(e)?;
        let mut add_kw = tx.prepare_cached("INSERT OR IGNORE INTO keywords(photo_id,keyword,src) VALUES(?1,?2,?3)").map_err(e)?;
        for o in batch {
            match o {
                Outcome::Full { w, row, keywords, sidecar, unparsed } => {
                    let state = if unparsed.is_some() { 3 } else { 1 };
                    let (seen_size, seen_mtime) = if sidecar.seen { (w.xmp_size, w.xmp_mtime_ns) } else { (None, None) };
                    let r = &*row;
                    let n = full
                        .execute(params![
                            w.id, state, unparsed, now, r.taken_at, r.subsec_ms, r.tz_offset, r.make, r.model, r.body_serial, r.lens, r.focal,
                            r.focal35, r.aperture, r.iso, r.exposure, r.exposure_bias, r.width, r.height, r.raw_width, r.raw_height,
                            r.orientation, r.preview_offset, r.preview_len, r.preview_w, r.preview_h, r.gps_lat, r.gps_lon, r.gps_alt,
                            r.software, r.container, r.is_raw, r.rating, r.label, r.rejected, r.edited, r.content_key, r.extra_json,
                            r.head_bytes, seen_size, seen_mtime, w.size as i64, w.mtime_ns
                        ])
                        .map_err(e)?;
                    if n == 0 {
                        out.raced += 1;
                        continue;
                    }
                    if unparsed.is_some() {
                        out.unparsed += 1;
                    } else {
                        out.done += 1;
                    }
                    del_kw.execute(params![w.id, "xmp"]).map_err(e)?;
                    for k in keywords.iter().take(1000) {
                        add_kw.execute(params![w.id, k, "xmp"]).map_err(e)?;
                    }
                    if let Some(x) = &sidecar.xmp {
                        del_kw.execute(params![w.id, "sidecar"]).map_err(e)?;
                        for k in &x.keywords {
                            add_kw.execute(params![w.id, k, "sidecar"]).map_err(e)?;
                        }
                    }
                }
                Outcome::Side { w, x } => {
                    let mut r = Row::default();
                    if let Some(xmp) = &x.xmp {
                        apply_xmp(&mut r, xmp);
                    }
                    let n = side
                        .execute(params![w.id, r.rating, r.label, r.rejected, r.edited, w.xmp_size, w.xmp_mtime_ns, now, w.size as i64, w.mtime_ns])
                        .map_err(e)?;
                    if n == 0 {
                        out.raced += 1;
                        continue;
                    }
                    out.sidecar_only += 1;
                    if let Some(xmp) = &x.xmp {
                        del_kw.execute(params![w.id, "sidecar"]).map_err(e)?;
                        for k in &xmp.keywords {
                            add_kw.execute(params![w.id, k, "sidecar"]).map_err(e)?;
                        }
                    }
                }
                Outcome::Failed { w, reason, .. } => {
                    // a photo that was already done keeps its data when only its sidecar failed
                    if w.full {
                        let n = fail.execute(params![w.id, 2, reason, now, w.size as i64, w.mtime_ns]).map_err(e)?;
                        if n == 0 {
                            out.raced += 1;
                            continue;
                        }
                    }
                    out.failed += 1;
                }
            }
        }
    }
    tx.commit().map_err(e)?;
    Ok(out)
}

// ---- the run ------------------------------------------------------------------------------

#[derive(Debug, Clone)]
enum Reason {
    Offline,
    Failed(String),
}

struct Queue {
    items: VecDeque<Work>,
    closed: bool,
}

struct Run<'a> {
    fs: &'a dyn MetaFs,
    cfg: &'a MetaConfig,
    steps: Vec<usize>,
    ctl: &'a ScanControl,
    queue: Mutex<Queue>,
    wake: Condvar,
    abort: Mutex<Option<Reason>>,
    stats: Stats,
    adaptive: Mutex<Adaptive>,
    /// Readers currently inside a read.
    active: Mutex<usize>,
    gate: Condvar,
    allowed: AtomicUsize,
    exited: AtomicUsize,
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

    /// Waits for a reader slot under the adaptive limit.
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

    fn worker(&self, tx: SyncSender<Outcome>) {
        loop {
            if !self.ctl.proceed() {
                break;
            }
            let Some(w) = self.take() else { break };
            if !self.enter() {
                break;
            }
            let o = process(self.fs, w, self.cfg, &self.steps, &self.stats, &self.adaptive, &|| self.stopped());
            self.leave();
            if tx.send(o).is_err() {
                break;
            }
        }
        self.exited.fetch_add(1, Ordering::SeqCst);
    }
}

fn pending_sql(retry_failed: bool, sidecars: bool) -> String {
    let side = if sidecars {
        " OR (meta_state IN (1,3) AND ((xmp_kind<>0 AND (xmp_seen_size IS NOT xmp_size OR xmp_seen_mtime_ns IS NOT xmp_mtime_ns))))"
    } else {
        ""
    };
    let gone = if sidecars { " OR (meta_state=1 AND xmp_kind=0 AND xmp_seen_size IS NOT NULL)" } else { "" };
    format!("(meta_state=0{}{side}{gone})", if retry_failed { " OR meta_state=2" } else { "" })
}

fn count_pending(c: &Connection, sid: i64, cfg: &MetaConfig) -> rusqlite::Result<(u64, u64)> {
    let sql = format!("SELECT COUNT(*) FROM photos WHERE source_id=?1 AND status='present' AND {}", pending_sql(cfg.retry_failed, cfg.sidecars));
    let pending: i64 = c.query_row(&sql, [sid], |r| r.get(0))?;
    let all: i64 = c.query_row("SELECT COUNT(*) FROM photos WHERE source_id=?1 AND status='present'", [sid], |r| r.get(0))?;
    Ok((pending as u64, (all - pending).max(0) as u64))
}

fn load_chunk(c: &Connection, sid: i64, after: i64, cfg: &MetaConfig) -> rusqlite::Result<Vec<Work>> {
    let sql = format!(
        "SELECT id, rel_path, ext, size, mtime_ns, xmp_kind, xmp_size, xmp_mtime_ns, meta_state FROM photos
         WHERE source_id=?1 AND status='present' AND id>?2 AND {} ORDER BY id LIMIT ?3",
        pending_sql(cfg.retry_failed, cfg.sidecars)
    );
    let mut st = c.prepare_cached(&sql)?;
    let rows = st.query_map(params![sid, after, cfg.chunk as i64], |r| {
        let state: i64 = r.get(8)?;
        let kind: i64 = r.get(5)?;
        Ok(Work {
            id: r.get(0)?,
            rel: r.get(1)?,
            ext: r.get(2)?,
            size: r.get::<_, i64>(3)?.max(0) as u64,
            mtime_ns: r.get(4)?,
            xmp_kind: kind as u8,
            xmp_size: r.get(6)?,
            xmp_mtime_ns: r.get(7)?,
            // a done photo is here only for its sidecar, unless the sidecar is gone
            full: state == 0 || state == 2 || kind == 0,
        })
    })?;
    rows.collect()
}

fn percentile(v: &[f32], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    s[((s.len() - 1) as f64 * p).round() as usize] as f64
}

/// Reads the metadata of every pending photo of `source`. Blocks until the
/// job ends, is stopped through `ctl`, or the volume goes away. `on_progress` is
/// called from the calling thread, throttled to `cfg.progress_every`.
pub fn run_metadata(
    db: &CatalogDb,
    fs: &dyn MetaFs,
    source: &Source,
    cfg: &MetaConfig,
    ctl: &ScanControl,
    on_progress: &mut dyn FnMut(&MetaProgress),
) -> Result<MetaReport, String> {
    let started = Instant::now();
    let sid = source.id;
    let blank = |state: &'static str, error: Option<String>| MetaReport {
        source_id: sid,
        job_id: 0,
        state,
        total: 0,
        done: 0,
        sidecar_only: 0,
        failed: 0,
        unparsed: 0,
        skipped: 0,
        raced: 0,
        reads: 0,
        escalations: 0,
        bytes_read: 0,
        elapsed: started.elapsed(),
        per_s: 0.0,
        p50_ms: 0.0,
        p95_ms: 0.0,
        relink: RelinkReport::default(),
        resumed: false,
        error,
    };
    // nothing is read, and nothing is recorded, of a volume that does not answer
    if !fs.online() {
        return Ok(blank("offline", Some("the source is not reachable".into())));
    }
    // continue an interrupted metadata job of this source, or start one
    let prior = {
        let c = db.read().map_err(|e| e.to_string())?;
        c.query_row(
            "SELECT id, checkpoint FROM jobs WHERE source_id=?1 AND kind='metadata' AND state='interrupted' ORDER BY id DESC LIMIT 1",
            [sid],
            |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?
    };
    let (job_id, resumed, cp) = match prior {
        Some((id, cp)) => {
            let cp: MetaCheckpoint = cp.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
            db.exec(move |c| {
                c.execute("UPDATE jobs SET state='running', updated_at=?2 WHERE id=?1", params![id, catalog_db::now_s()]).map(|_| ()).map_err(|e| e.to_string())
            })?;
            (id, true, cp)
        }
        None => {
            let id = db.exec(move |c| {
                let id = catalog_db::create_job(c, "metadata", sid, "{}").map_err(|e| e.to_string())?;
                c.execute("UPDATE jobs SET state='running', started_at=?2, updated_at=?2 WHERE id=?1", params![id, catalog_db::now_s()]).map_err(|e| e.to_string())?;
                Ok(id)
            })?;
            (id, false, MetaCheckpoint::default())
        }
    };
    let (pending, skipped) = {
        let c = db.read().map_err(|e| e.to_string())?;
        count_pending(&c, sid, cfg).map_err(|e| e.to_string())?
    };
    let total = pending + cp.done + cp.failed + cp.unparsed;
    let workers = cfg.workers.max(1);
    let run = Run {
        fs,
        cfg,
        steps: cfg.steps(),
        ctl,
        queue: Mutex::new(Queue { items: VecDeque::new(), closed: false }),
        wake: Condvar::new(),
        abort: Mutex::new(None),
        stats: Stats {
            reads: AtomicU64::new(0),
            escalations: AtomicU64::new(0),
            bytes: AtomicU64::new(cp.bytes),
            consecutive_failures: AtomicUsize::new(0),
            consecutive_timeouts: AtomicUsize::new(0),
            lat_all: Mutex::new(Vec::new()),
        },
        adaptive: Mutex::new(Adaptive::new(workers)),
        active: Mutex::new(0),
        gate: Condvar::new(),
        allowed: AtomicUsize::new(workers),
        exited: AtomicUsize::new(0),
    };
    let mut after_id = cp.after_id;
    let mut exhausted = false;
    let mut counts = Written { done: cp.done, failed: cp.failed, unparsed: cp.unparsed, ..Default::default() };
    let mut sidecar_only = 0u64;
    let mut raced = 0u64;
    let mut buffer: Vec<Outcome> = Vec::new();
    let mut last_flush = Instant::now();
    let mut last_progress = Instant::now() - cfg.progress_every;
    let mut last_checkpoint = Instant::now();
    let mut last_tick = Instant::now();
    let mut samples: VecDeque<(Instant, u64)> = VecDeque::new();
    let mut breaker_at = 0usize;
    let reader = db.read().map_err(|e| e.to_string())?;
    let (tx, rx) = sync_channel::<Outcome>(cfg.batch_rows.max(16));
    let snapshot = |counts: &Written, paused: bool, phase: &str, samples: &VecDeque<(Instant, u64)>| -> MetaProgress {
        let done_now = counts.done + counts.failed + counts.unparsed;
        let elapsed = started.elapsed().as_secs_f64();
        let rate = match samples.front() {
            Some((t, n)) if Instant::now() > *t => (done_now.saturating_sub(*n)) as f64 / (Instant::now() - *t).as_secs_f64(),
            _ => 0.0,
        };
        let eta_s = (!paused && rate > 1.0 && elapsed > 2.0 && total > done_now).then(|| {
            let left = (total - done_now) as f64 / rate;
            ((left * 0.7) as u64, (left * 1.6).ceil() as u64)
        });
        MetaProgress {
            source_id: sid,
            job_id,
            phase: phase.into(),
            total,
            done: counts.done,
            failed: counts.failed,
            unparsed: counts.unparsed,
            skipped,
            bytes_read: run.stats.bytes.load(Ordering::Relaxed),
            elapsed_s: elapsed,
            rate_per_s: rate,
            eta_s,
            workers: run.allowed.load(Ordering::SeqCst) as u64,
            p95_ms: run.adaptive.lock().unwrap().p95(),
            paused,
        }
    };
    let save = |counts: &Written, after: i64, state: &str, prog: &MetaProgress| -> Result<(), String> {
        let cp = MetaCheckpoint { after_id: after, total, done: counts.done, failed: counts.failed, unparsed: counts.unparsed, bytes: prog.bytes_read };
        let (cp, pr, st) = (serde_json::to_string(&cp).map_err(|e| e.to_string())?, serde_json::to_string(prog).map_err(|e| e.to_string())?, state.to_string());
        db.exec(move |c| catalog_db::save_checkpoint(c, job_id, &cp, &pr, &st).map_err(|e| e.to_string()))
    };
    // a failed read counts only if the volume still answers; otherwise the streak was the mount going away
    let flush = |buffer: &mut Vec<Outcome>, counts: &mut Written, sidecar_only: &mut u64, raced: &mut u64| -> Result<(), String> {
        if buffer.is_empty() {
            return Ok(());
        }
        let mut batch = std::mem::take(buffer);
        if batch.iter().any(|o| matches!(o, Outcome::Failed { read_error: true, .. })) && (!fs.online() || matches!(*run.abort.lock().unwrap(), Some(Reason::Offline))) {
            run.set_abort(Reason::Offline);
            batch.retain(|o| !matches!(o, Outcome::Failed { read_error: true, .. }));
        }
        let written = db.exec(move |c| write_batch(c, batch))?;
        counts.done += written.done;
        counts.failed += written.failed;
        counts.unparsed += written.unparsed;
        *sidecar_only += written.sidecar_only;
        *raced += written.raced;
        Ok(())
    };

    let mut write_error: Option<String> = None;
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let txc = tx.clone();
            let r = &run;
            scope.spawn(move || r.worker(txc));
        }
        drop(tx);
        loop {
            // keep the queue filled from the catalog
            if !exhausted {
                let low = run.queue.lock().unwrap().items.len() < cfg.chunk / 2;
                if low {
                    match load_chunk(&reader, sid, after_id, cfg) {
                        Ok(chunk) => {
                            if chunk.is_empty() {
                                exhausted = true;
                                run.queue.lock().unwrap().closed = true;
                            } else {
                                after_id = chunk.last().map_or(after_id, |w| w.id);
                                run.queue.lock().unwrap().items.extend(chunk);
                            }
                            run.wake.notify_all();
                        }
                        Err(e) => run.set_abort(Reason::Failed(e.to_string())),
                    }
                }
            }
            match rx.recv_timeout(Duration::from_millis(50)) {
                Ok(o) => {
                    buffer.push(o);
                    while let Ok(o) = rx.try_recv() {
                        buffer.push(o);
                        if buffer.len() >= cfg.batch_rows {
                            break;
                        }
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    if let Err(e) = flush(&mut buffer, &mut counts, &mut sidecar_only, &mut raced) {
                        write_error = Some(e);
                    }
                    break;
                }
            }
            let now = Instant::now();
            if buffer.len() >= cfg.batch_rows || (!buffer.is_empty() && now.duration_since(last_flush) >= cfg.flush_every) {
                last_flush = now;
                if let Err(e) = flush(&mut buffer, &mut counts, &mut sidecar_only, &mut raced) {
                    write_error = Some(e);
                    run.set_abort(Reason::Failed(write_error.clone().unwrap_or_default()));
                }
            }
            if let Some(e) = db.failure() {
                run.set_abort(Reason::Failed(e));
            }
            // too many failures in a row: is the volume still there?
            let fails = run.stats.consecutive_failures.load(Ordering::Relaxed);
            if fails >= BREAKER && fails >= breaker_at + BREAKER {
                breaker_at = fails;
                // a mount that answers a stat but not reads is as good as gone: stop, write nothing about it
                if !fs.online() || run.stats.consecutive_timeouts.load(Ordering::Relaxed) >= BREAKER {
                    run.set_abort(Reason::Offline);
                }
            }
            if fails == 0 {
                breaker_at = 0;
            }
            if cfg.adaptive && now.duration_since(last_tick) >= Duration::from_millis(500) {
                last_tick = now;
                let n = run.adaptive.lock().unwrap().tick(now);
                if n != run.allowed.load(Ordering::SeqCst) {
                    run.allowed.store(n, Ordering::SeqCst);
                    run.gate.notify_all();
                }
            }
            if samples.back().is_none_or(|(t, _)| now.duration_since(*t) >= Duration::from_millis(500)) {
                samples.push_back((now, counts.done + counts.failed + counts.unparsed));
                while samples.len() > 12 {
                    samples.pop_front();
                }
            }
            let paused = ctl.is_paused();
            if now.duration_since(last_progress) >= cfg.progress_every {
                last_progress = now;
                on_progress(&snapshot(&counts, paused, "reading", &samples));
            }
            if now.duration_since(last_checkpoint) >= cfg.checkpoint_every {
                last_checkpoint = now;
                // everything before `after_id` that was queued but not yet written is picked up again:
                // the pending query, not the checkpoint, decides what is left
                let _ = save(&counts, cp.after_id, if paused { "paused" } else { "running" }, &snapshot(&counts, paused, "reading", &samples));
            }
        }
    });
    drop(reader);
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
        None => match ctl.stopped() {
            RUN => "complete",
            CANCEL => "cancelled",
            _ => "interrupted",
        },
    };
    let last = snapshot(&counts, false, "reading", &samples);
    // a resumable stop keeps the checkpoint; the pending query finds what is left
    if matches!(state, "interrupted" | "offline") {
        save(&counts, cp.after_id, "interrupted", &last)?;
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
    let mut relinked = RelinkReport::default();
    if state == "complete" && cfg.relink {
        on_progress(&snapshot(&counts, false, "linking", &samples));
        relinked = relink_source(db, sid)?;
    }
    let lat = run.stats.lat_all.lock().unwrap().clone();
    let elapsed = started.elapsed();
    let mut fin = snapshot(&counts, false, "done", &samples);
    fin.phase = "done".into();
    on_progress(&fin);
    let processed = counts.done + counts.failed + counts.unparsed - (cp.done + cp.failed + cp.unparsed) + sidecar_only;
    Ok(MetaReport {
        source_id: sid,
        job_id,
        state,
        total,
        done: counts.done,
        sidecar_only,
        failed: counts.failed,
        unparsed: counts.unparsed,
        skipped,
        raced,
        reads: run.stats.reads.load(Ordering::Relaxed),
        escalations: run.stats.escalations.load(Ordering::Relaxed),
        bytes_read: run.stats.bytes.load(Ordering::Relaxed),
        elapsed,
        per_s: processed as f64 / elapsed.as_secs_f64().max(0.001),
        p50_ms: percentile(&lat, 0.5),
        p95_ms: percentile(&lat, 0.95),
        relink: relinked,
        resumed,
        error,
    })
}

/// How many photos of each state a catalog (or one source) holds.
#[derive(Debug, Clone, Default, Serialize, specta::Type)]
pub struct MetaCoverage {
    pub done: i64,
    pub pending: i64,
    pub failed: i64,
    pub unparsed: i64,
}

pub fn coverage(c: &Connection, source_id: Option<i64>) -> rusqlite::Result<MetaCoverage> {
    let (mut done, mut pending, mut failed, mut unparsed) = (0, 0, 0, 0);
    let mut st = c.prepare(
        "SELECT meta_state, COUNT(*) FROM photos WHERE status='present' AND (?1 IS NULL OR source_id=?1) GROUP BY meta_state",
    )?;
    let rows = st.query_map([source_id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
    for row in rows {
        let (s, n) = row?;
        match s {
            0 => pending = n,
            1 => done = n,
            2 => failed = n,
            _ => unparsed += n,
        }
    }
    Ok(MetaCoverage { done, pending, failed, unparsed })
}

#[cfg(test)]
#[path = "catalog_meta_job_tests.rs"]
mod tests;
