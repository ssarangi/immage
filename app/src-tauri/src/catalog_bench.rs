// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! `immage catalog bench`: measures how the catalog's pipelines behave on one
//! folder (a NAS share, a local disk), read-only, and reports aggregates only.
//! It uses the same `ScanFs` / `MetaFs` readers and the same metadata parser as
//! the real jobs, but never opens the user's catalog: the SQLite write cost is
//! measured in a scratch database that is deleted at the end. The report holds
//! counts, sizes, latencies and extensions, never a file or folder name.
//! Output paths inside the measured tree are refused by the caller.

use crate::catalog_meta::{extract, Extract, Span, HEAD_LADDER};
use crate::catalog_meta_job::{MetaFs, READ_DEADLINE};
use crate::catalog_scan::{EntryKind, FsError, RealFs};
use crate::catalog_sources::MountTable;
use rusqlite::{params, Connection};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A preview read is capped: one embedded preview is some hundred KB to a few MB.
const PREVIEW_CAP: u64 = 16 * 1024 * 1024;
const PREVIEW_DEADLINE: Duration = Duration::from_secs(30);
/// This many failures in a row end the metadata phase: the share is not answering.
const ERROR_STORM: usize = 64;
const MAX_CONCURRENCY: usize = 8;

/// What the benchmark needs beyond the scan and metadata readers: a file stat.
pub trait BenchFs: MetaFs {
    /// The size of file `rel` (relative to the root), from a stat.
    fn stat_file(&self, rel: &str, deadline: Duration) -> Result<u64, FsError>;
}

impl BenchFs for RealFs {
    fn stat_file(&self, rel: &str, deadline: Duration) -> Result<u64, FsError> {
        let path = self.root.join(rel);
        self.within(deadline, move || {
            std::fs::metadata(&path).map(|m| m.len()).map_err(|e| crate::catalog_scan::classify_error(&e))
        })?
    }
}

#[derive(Debug, Clone)]
pub struct BenchConfig {
    /// Workers at the start; also the most the controller will raise it to.
    pub concurrency: usize,
    pub max_files: usize,
    pub duration: Duration,
    /// Window p95 above this backs the workers off (never aborts the run).
    pub latency_limit_ms: f64,
    /// How often the controller looks at the latency window.
    pub window: Duration,
    /// How long all workers pause after a backoff.
    pub backoff_pause: Duration,
    /// Width of a throughput bucket.
    pub bucket: Duration,
    /// The resource sampler's period.
    pub sample_every: Duration,
    /// Preview reads per format.
    pub thumb_sample: usize,
    pub stat_sample: usize,
    pub list_deadline: Duration,
    pub read_deadline: Duration,
    /// A local folder for the scratch SQLite database.
    pub scratch: PathBuf,
    pub batch: usize,
}

impl BenchConfig {
    pub fn new(network: bool, scratch: PathBuf) -> Self {
        Self {
            concurrency: if network { 2 } else { 4 },
            max_files: 5000,
            duration: Duration::from_secs(300),
            latency_limit_ms: 500.0,
            window: Duration::from_secs(2),
            backoff_pause: Duration::from_secs(1),
            bucket: Duration::from_secs(10),
            sample_every: Duration::from_secs(1),
            thumb_sample: 30,
            stat_sample: 500,
            list_deadline: Duration::from_secs(12),
            read_deadline: READ_DEADLINE,
            scratch,
            batch: 500,
        }
    }
}

// ---- small helpers ----------------------------------------------------------

fn pct(v: &mut [f64], p: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let i = ((v.len() as f64 - 1.0) * p).round() as usize;
    v[i]
}

fn r1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

fn dist(v: &[f64]) -> Value {
    let mut s = v.to_vec();
    let max = s.iter().cloned().fold(0.0, f64::max);
    json!({ "count": v.len(), "p50": r1(pct(&mut s, 0.5)), "p95": r1(pct(&mut s, 0.95)), "p99": r1(pct(&mut s, 0.99)), "max": r1(max) })
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

/// The kind of an error, without any path or message text.
fn error_kind(e: &FsError) -> String {
    match e {
        FsError::Timeout => "timeout".into(),
        FsError::Denied => "denied".into(),
        FsError::NotFound => "not-found".into(),
        FsError::Io(m) => match m.find("os error ") {
            Some(i) => format!("io (os error {})", m[i + 9..].trim_end_matches(')').trim()),
            None => "io".into(),
        },
    }
}

/// The format a file counts under: the formats the catalog cares about by name.
fn fmt_key(ext: &str) -> &'static str {
    match ext.to_ascii_lowercase().as_str() {
        "cr3" => "cr3",
        "cr2" => "cr2",
        "nef" => "nef",
        "arw" => "arw",
        "raf" => "raf",
        "dng" => "dng",
        "heic" | "heif" | "hif" => "heic",
        "jpg" | "jpeg" | "jfif" => "jpg",
        e if crate::catalog_meta::is_raw_ext(e) => "other-raw",
        _ => "other",
    }
}

fn ext_of(name: &str) -> String {
    name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

// ---- shared run state -------------------------------------------------------

/// How many workers may run, and a pause every worker honours.
/// Ends a phase for every worker when any worker leaves it: a worker parked by
/// the gate would otherwise wait for work that is gone.
struct Over<'a>(&'a AtomicBool);

impl Drop for Over<'_> {
    fn drop(&mut self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

struct Gate {
    allowed: AtomicUsize,
    resume_at: Mutex<Option<Instant>>,
}

struct Shared {
    start: Instant,
    stop: AtomicBool,
    interrupted: Box<dyn Fn() -> bool + Send + Sync>,
    gate: Gate,
    /// Latencies since the controller last looked.
    window: Mutex<Vec<f64>>,
    errors: Mutex<BTreeMap<String, u64>>,
    bytes: AtomicU64,
    ops: AtomicU64,
    series: Mutex<BTreeMap<&'static str, Vec<u64>>>,
    bucket: Duration,
}

impl Shared {
    fn done(&self) -> bool {
        self.stop.load(Ordering::Relaxed) || (self.interrupted)()
    }

    /// Waits while this worker is not allowed to run or everyone is paused.
    /// False when the run is over.
    fn pass(&self, idx: usize, over: &AtomicBool) -> bool {
        loop {
            if self.done() || over.load(Ordering::Relaxed) {
                return false;
            }
            let paused = self.gate.resume_at.lock().unwrap().is_some_and(|t| Instant::now() < t);
            if idx < self.gate.allowed.load(Ordering::Relaxed) && !paused {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn bump(&self, name: &'static str, n: u64) {
        let i = (self.start.elapsed().as_secs_f64() / self.bucket.as_secs_f64().max(0.001)) as usize;
        let mut s = self.series.lock().unwrap();
        let v = s.entry(name).or_default();
        if v.len() <= i {
            v.resize(i + 1, 0);
        }
        v[i] += n;
    }

    fn observe(&self, ms: f64) {
        self.window.lock().unwrap().push(ms);
        self.ops.fetch_add(1, Ordering::Relaxed);
    }

    fn error(&self, e: &FsError) {
        *self.errors.lock().unwrap().entry(error_kind(e)).or_default() += 1;
    }
}

#[derive(Default)]
struct Controller {
    backoffs: u64,
    events: Vec<Value>,
    peak_p95: f64,
    min_allowed: usize,
}

fn control(sh: &Shared, cfg: &BenchConfig, max: usize, finished: &AtomicBool) -> Controller {
    let mut c = Controller { min_allowed: max, ..Default::default() };
    let mut calm = 0;
    while !finished.load(Ordering::Relaxed) {
        // sleep in slices so the end of the run is noticed quickly
        let t0 = Instant::now();
        while t0.elapsed() < cfg.window && !finished.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(5).min(cfg.window));
        }
        let mut w = std::mem::take(&mut *sh.window.lock().unwrap());
        if w.len() < 3 {
            continue;
        }
        let p95 = pct(&mut w, 0.95);
        c.peak_p95 = c.peak_p95.max(p95);
        let allowed = sh.gate.allowed.load(Ordering::Relaxed);
        if p95 > cfg.latency_limit_ms {
            let next = (allowed / 2).max(1);
            sh.gate.allowed.store(next, Ordering::Relaxed);
            *sh.gate.resume_at.lock().unwrap() = Some(Instant::now() + cfg.backoff_pause);
            c.backoffs += 1;
            calm = 0;
            if c.events.len() < 200 {
                c.events.push(json!({ "t_s": r1(sh.start.elapsed().as_secs_f64()), "window_p95_ms": r1(p95), "workers": next }));
            }
        } else if p95 < cfg.latency_limit_ms / 2.0 && allowed < max {
            calm += 1;
            if calm >= 3 {
                sh.gate.allowed.store(allowed + 1, Ordering::Relaxed);
                calm = 0;
                if c.events.len() < 200 {
                    c.events.push(json!({ "t_s": r1(sh.start.elapsed().as_secs_f64()), "window_p95_ms": r1(p95), "workers": allowed + 1 }));
                }
            }
        } else {
            calm = 0;
        }
        c.min_allowed = c.min_allowed.min(sh.gate.allowed.load(Ordering::Relaxed));
    }
    c
}

// ---- process and network counters -------------------------------------------

fn cpu_seconds() -> Option<(f64, f64)> {
    #[cfg(unix)]
    {
        // SAFETY: getrusage writes into the zeroed struct we own.
        let mut u: libc::rusage = unsafe { std::mem::zeroed() };
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut u) } != 0 {
            return None;
        }
        let f = |t: libc::timeval| t.tv_sec as f64 + t.tv_usec as f64 / 1e6;
        Some((f(u.ru_utime), f(u.ru_stime)))
    }
    #[cfg(not(unix))]
    {
        None
    }
}

fn rss_bytes() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = s.lines().find(|l| l.starts_with("VmRSS:"))?;
    Some(line.split_whitespace().nth(1)?.parse::<u64>().ok()? * 1024)
}

/// Bytes received on every interface but loopback: everything the machine
/// received, not only this process, so it is an upper bound.
fn net_rx() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/net/dev").ok()?;
    let mut total = 0u64;
    for l in s.lines().skip(2) {
        let (name, rest) = l.split_once(':')?;
        if name.trim() == "lo" {
            continue;
        }
        total += rest.split_whitespace().next()?.parse::<u64>().ok()?;
    }
    Some(total)
}

#[derive(Default)]
struct Resources {
    /// Per bucket: (cpu seconds used, wall seconds, peak rss bytes).
    buckets: BTreeMap<usize, (f64, f64, u64)>,
    peak_rss: u64,
}

fn sample(sh: &Shared, cfg: &BenchConfig, finished: &AtomicBool) -> Resources {
    let mut r = Resources::default();
    let mut last = (Instant::now(), cpu_seconds().map_or(0.0, |(u, s)| u + s));
    while !finished.load(Ordering::Relaxed) {
        let t0 = Instant::now();
        while t0.elapsed() < cfg.sample_every && !finished.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(5).min(cfg.sample_every));
        }
        let now = Instant::now();
        let cpu = cpu_seconds().map_or(0.0, |(u, s)| u + s);
        let rss = rss_bytes().unwrap_or(0);
        let i = (sh.start.elapsed().as_secs_f64() / cfg.bucket.as_secs_f64().max(0.001)) as usize;
        let e = r.buckets.entry(i).or_default();
        e.0 += (cpu - last.1).max(0.0);
        e.1 += now.duration_since(last.0).as_secs_f64();
        e.2 = e.2.max(rss);
        r.peak_rss = r.peak_rss.max(rss);
        last = (now, cpu);
    }
    r
}

// ---- the scratch database ---------------------------------------------------

struct Scratch {
    dir: PathBuf,
    conn: Connection,
    batch_ms: Vec<f64>,
    rows: u64,
    wal_max: u64,
}

impl Scratch {
    fn open(base: &Path) -> Result<Self, String> {
        let dir = base.join(format!("immage-bench-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())));
        std::fs::create_dir_all(&dir).map_err(|e| format!("cannot create the scratch folder: {e}"))?;
        let conn = Connection::open(dir.join("bench.sqlite")).map_err(|e| e.to_string())?;
        conn.execute_batch(
            "PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;
             CREATE TABLE photos(id INTEGER PRIMARY KEY, rel TEXT NOT NULL, ext TEXT, size INTEGER);
             CREATE TABLE meta(photo_id INTEGER PRIMARY KEY, container TEXT, make TEXT, model TEXT, lens TEXT, taken_at INTEGER,
                               width INTEGER, height INTEGER, iso INTEGER, preview_off INTEGER, preview_len INTEGER);",
        )
        .map_err(|e| e.to_string())?;
        Ok(Self { dir, conn, batch_ms: Vec::new(), rows: 0, wal_max: 0 })
    }

    fn timed(&mut self, rows: u64, f: impl FnOnce(&rusqlite::Transaction) -> rusqlite::Result<()>) {
        let t = Instant::now();
        let ok = (|| {
            let tx = self.conn.transaction()?;
            f(&tx)?;
            tx.commit()
        })();
        if ok.is_ok() {
            self.batch_ms.push(ms(t.elapsed()));
            self.rows += rows;
            let wal = std::fs::metadata(self.dir.join("bench.sqlite-wal")).map_or(0, |m| m.len());
            self.wal_max = self.wal_max.max(wal);
        }
    }

    fn photos(&mut self, rows: &[(String, String, u64)]) {
        self.timed(rows.len() as u64, |tx| {
            let mut st = tx.prepare_cached("INSERT INTO photos(rel, ext, size) VALUES (?1, ?2, ?3)")?;
            for (rel, ext, size) in rows {
                st.execute(params![rel, ext, *size as i64])?;
            }
            Ok(())
        });
    }

    fn metas(&mut self, rows: &[(i64, MetaRow)]) {
        self.timed(rows.len() as u64, |tx| {
            let mut st = tx.prepare_cached("INSERT OR REPLACE INTO meta VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)")?;
            for (id, m) in rows {
                st.execute(params![id, m.container, m.make, m.model, m.lens, m.taken_at, m.width, m.height, m.iso, m.off, m.len])?;
            }
            Ok(())
        });
    }

    fn finish(self) -> Value {
        let t = Instant::now();
        let _ = self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
        let checkpoint = ms(t.elapsed());
        let db = std::fs::metadata(self.dir.join("bench.sqlite")).map_or(0, |m| m.len());
        let out = json!({
            "batches": dist(&self.batch_ms), "rows": self.rows, "rows_per_batch_target": null,
            "wal_max_bytes": self.wal_max, "db_bytes": db, "checkpoint_ms": r1(checkpoint),
            "total_write_ms": r1(self.batch_ms.iter().sum::<f64>()),
        });
        drop(self.conn);
        let _ = std::fs::remove_dir_all(&self.dir);
        out
    }
}

struct MetaRow {
    container: String,
    make: Option<String>,
    model: Option<String>,
    lens: Option<String>,
    taken_at: Option<i64>,
    width: Option<u32>,
    height: Option<u32>,
    iso: Option<u32>,
    off: Option<i64>,
    len: Option<i64>,
}

// ---- the phases ----------------------------------------------------------------

struct PhotoRef {
    rel: String,
    ext: String,
    size: u64,
}

#[derive(Default)]
struct ExtInfo {
    count: u64,
    sizes: Vec<f64>,
}

#[derive(Default)]
struct Discovery {
    dirs: u64,
    entries: u64,
    xmp: u64,
    other_files: u64,
    skipped: u64,
    list_ms: Vec<f64>,
    /// (entries in the directory, ms)
    by_size: Vec<(usize, f64)>,
    entries_per_dir: Vec<f64>,
    exts: BTreeMap<String, ExtInfo>,
    photos: Vec<PhotoRef>,
    complete: bool,
    elapsed: Duration,
    dir_samples: Vec<String>,
}

fn discover<F: BenchFs>(fs: &F, root: &Path, cfg: &BenchConfig, sh: &Shared, until: Instant, workers: usize) -> Discovery {
    struct Q {
        todo: VecDeque<String>,
        busy: usize,
        d: Discovery,
        capped: bool,
        cut: bool,
    }
    let q = Mutex::new(Q { todo: VecDeque::from([String::new()]), busy: 0, d: Discovery::default(), capped: false, cut: false });
    let t0 = Instant::now();
    let over = AtomicBool::new(false);
    std::thread::scope(|s| {
        for idx in 0..workers {
            let q = &q;
            let over = &over;
            s.spawn(move || {
                let _end = Over(over);
                loop {
                if !sh.pass(idx, over) || Instant::now() >= until {
                    // parked workers leave when the walk is over; only a stop or the time limit cuts it
                    if sh.done() || Instant::now() >= until {
                        q.lock().unwrap().cut = true;
                    }
                    return;
                }
                let rel = {
                    let mut g = q.lock().unwrap();
                    if g.d.photos.len() >= cfg.max_files {
                        g.capped = true;
                        return;
                    }
                    match g.todo.pop_front() {
                        Some(r) => {
                            g.busy += 1;
                            r
                        }
                        None if g.busy == 0 => return,
                        None => {
                            drop(g);
                            std::thread::sleep(Duration::from_millis(2));
                            continue;
                        }
                    }
                };
                let path = if rel.is_empty() { root.to_path_buf() } else { root.join(&rel) };
                let t = Instant::now();
                let r = fs.list(&path, cfg.list_deadline);
                let took = ms(t.elapsed());
                sh.observe(took);
                let mut g = q.lock().unwrap();
                g.busy -= 1;
                match r {
                    Err(e) => sh.error(&e),
                    Ok(list) => {
                        g.d.dirs += 1;
                        g.d.entries += list.len() as u64;
                        g.d.list_ms.push(took);
                        g.d.by_size.push((list.len(), took));
                        g.d.entries_per_dir.push(list.len() as f64);
                        sh.bump("dirs", 1);
                        sh.bump("entries", list.len() as u64);
                        if g.d.dir_samples.len() < 100 {
                            g.d.dir_samples.push(rel.clone());
                        }
                        for e in list {
                            let Some(name) = e.name.to_str() else {
                                g.d.skipped += 1;
                                continue;
                            };
                            if name.starts_with('.') {
                                g.d.skipped += 1;
                                continue;
                            }
                            let child = if rel.is_empty() { name.to_string() } else { format!("{rel}/{name}") };
                            match e.kind {
                                EntryKind::Dir => g.todo.push_back(child),
                                EntryKind::File if name.to_ascii_lowercase().ends_with(".xmp") => g.d.xmp += 1,
                                EntryKind::File if crate::library_cmds::photo_candidate(Path::new(name)) => {
                                    let ext = ext_of(name);
                                    let info = g.d.exts.entry(fmt_key(&ext).to_string()).or_default();
                                    info.count += 1;
                                    info.sizes.push(e.size as f64);
                                    g.d.photos.push(PhotoRef { rel: child, ext, size: e.size });
                                }
                                EntryKind::File => g.d.other_files += 1,
                                _ => g.d.skipped += 1,
                            }
                        }
                    }
                }
                }
            });
        }
    });
    let mut g = q.into_inner().unwrap();
    g.d.complete = g.todo.is_empty() && !g.capped && !g.cut;
    g.d.photos.truncate(cfg.max_files);
    g.d.elapsed = t0.elapsed();
    g.d
}

#[derive(Default)]
struct Fmt {
    files: u64,
    read_ms: Vec<f64>,
    final_bytes: Vec<f64>,
    asked: Vec<f64>,
    /// Files that finished at the 64 KB, 256 KB, 1 MB step.
    ladder: [u64; 3],
    retried: u64,
    done: u64,
    truncated: u64,
    unsupported: u64,
    corrupt: u64,
    read_errors: u64,
    bytes: u64,
    previews: u64,
}

struct MetaOut {
    fmts: BTreeMap<&'static str, Fmt>,
    previews: Vec<(&'static str, String, Span)>,
    stopped: Option<&'static str>,
    elapsed: Duration,
    files: u64,
}

fn read_metadata<F: BenchFs>(fs: &F, cfg: &BenchConfig, sh: &Shared, photos: &[PhotoRef], until: Instant, scratch: &Mutex<Scratch>, workers: usize) -> MetaOut {
    struct St {
        fmts: BTreeMap<&'static str, Fmt>,
        previews: Vec<(&'static str, String, Span)>,
        per_fmt_prev: BTreeMap<&'static str, usize>,
        rows: Vec<(i64, MetaRow)>,
        stopped: Option<&'static str>,
        files: u64,
    }
    let st = Mutex::new(St { fmts: BTreeMap::new(), previews: Vec::new(), per_fmt_prev: BTreeMap::new(), rows: Vec::new(), stopped: None, files: 0 });
    let next = AtomicUsize::new(0);
    let streak = AtomicUsize::new(0);
    let t0 = Instant::now();
    let over = AtomicBool::new(false);
    std::thread::scope(|s| {
        for idx in 0..workers {
            let (st, next, streak) = (&st, &next, &streak);
            let over = &over;
            s.spawn(move || {
                let _end = Over(over);
                loop {
                if !sh.pass(idx, over) {
                    return;
                }
                if Instant::now() >= until {
                    st.lock().unwrap().stopped.get_or_insert("duration");
                    return;
                }
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(p) = photos.get(i) else { return };
                let key = fmt_key(&p.ext);
                let mut f = Fmt { files: 1, ..Default::default() };
                let mut step = 0;
                let mut len = (HEAD_LADDER[0] as u64).min(p.size.max(1)) as usize;
                let mut result: Option<crate::catalog_meta::Meta> = None;
                let mut failed = false;
                loop {
                    let t = Instant::now();
                    let r = fs.read_at(&p.rel, 0, len, cfg.read_deadline);
                    let took = ms(t.elapsed());
                    let buf = match r {
                        Ok(b) => b,
                        Err(e) => {
                            sh.error(&e);
                            f.read_errors += 1;
                            failed = true;
                            if streak.fetch_add(1, Ordering::Relaxed) + 1 >= ERROR_STORM {
                                st.lock().unwrap().stopped.get_or_insert("error-storm");
                                sh.stop.store(true, Ordering::Relaxed);
                            }
                            break;
                        }
                    };
                    streak.store(0, Ordering::Relaxed);
                    sh.observe(took);
                    f.read_ms.push(took);
                    f.bytes += buf.len() as u64;
                    sh.bytes.fetch_add(buf.len() as u64, Ordering::Relaxed);
                    sh.bump("bytes", buf.len() as u64);
                    f.final_bytes = vec![buf.len() as f64];
                    match extract(&buf, p.size, &p.ext) {
                        Extract::Done(m) => {
                            f.done = 1;
                            f.ladder[step] = 1;
                            result = Some(*m);
                            break;
                        }
                        Extract::NeedMore { bytes, partial } => {
                            f.asked = vec![bytes as f64];
                            let nx = HEAD_LADDER.iter().enumerate().skip(step + 1).find(|(_, n)| **n as u64 >= bytes);
                            match nx {
                                Some((j, n)) if (buf.len() as u64) < p.size && buf.len() >= HEAD_LADDER[step].min(p.size as usize) => {
                                    f.retried += 1;
                                    step = j;
                                    len = (*n as u64).min(p.size) as usize;
                                }
                                _ => {
                                    // past the ladder, or nothing more in the file
                                    f.truncated = 1;
                                    f.ladder[step] = 1;
                                    result = Some(*partial);
                                    break;
                                }
                            }
                        }
                        Extract::Unsupported => {
                            f.unsupported = 1;
                            f.ladder[step] = 1;
                            break;
                        }
                        Extract::Corrupt(_) => {
                            f.corrupt = 1;
                            f.ladder[step] = 1;
                            break;
                        }
                    }
                }
                sh.bump("files", 1);
                let mut g = st.lock().unwrap();
                g.files += 1;
                let mut flush = None;
                if let Some(m) = &result {
                    if let Some(sp) = m.preview {
                        f.previews = 1;
                        let n = g.per_fmt_prev.entry(key).or_default();
                        if *n < cfg.thumb_sample {
                            *n += 1;
                            g.previews.push((key, p.rel.clone(), sp));
                        }
                    }
                    g.rows.push((
                        i as i64 + 1,
                        MetaRow {
                            container: m.container.into(),
                            make: m.make.clone(),
                            model: m.model.clone(),
                            lens: m.lens.clone(),
                            taken_at: m.taken_at,
                            width: m.width,
                            height: m.height,
                            iso: m.iso,
                            off: m.preview.map(|s| s.offset as i64),
                            len: m.preview.map(|s| s.len as i64),
                        },
                    ));
                    if g.rows.len() >= cfg.batch {
                        flush = Some(std::mem::take(&mut g.rows));
                    }
                }
                let _ = failed;
                let e = g.fmts.entry(key).or_default();
                e.files += 1;
                e.read_ms.extend(f.read_ms);
                e.final_bytes.extend(f.final_bytes);
                e.asked.extend(f.asked);
                for k in 0..3 {
                    e.ladder[k] += f.ladder[k];
                }
                e.retried += f.retried;
                e.done += f.done;
                e.truncated += f.truncated;
                e.unsupported += f.unsupported;
                e.corrupt += f.corrupt;
                e.read_errors += f.read_errors;
                e.bytes += f.bytes;
                e.previews += f.previews;
                drop(g);
                if let Some(rows) = flush {
                    scratch.lock().unwrap().metas(&rows);
                }
                }
            });
        }
    });
    let g = st.into_inner().unwrap();
    if !g.rows.is_empty() {
        scratch.lock().unwrap().metas(&g.rows);
    }
    MetaOut { fmts: g.fmts, previews: g.previews, stopped: g.stopped, elapsed: t0.elapsed(), files: g.files }
}

/// Reads each sampled embedded preview in one ranged read: the thumbnail step's
/// whole network cost for a RAW.
fn read_previews<F: BenchFs>(fs: &F, sh: &Shared, previews: &[(&'static str, String, Span)], until: Instant, workers: usize) -> Value {
    #[derive(Default)]
    struct P {
        ms: Vec<f64>,
        bytes: Vec<f64>,
        jpeg: u64,
        err: u64,
    }
    let by: Mutex<BTreeMap<&'static str, P>> = Mutex::new(BTreeMap::new());
    let next = AtomicUsize::new(0);
    let over = AtomicBool::new(false);
    std::thread::scope(|s| {
        for idx in 0..workers {
            let (by, next) = (&by, &next);
            let over = &over;
            s.spawn(move || {
                let _end = Over(over);
                loop {
                if !sh.pass(idx, over) || Instant::now() >= until {
                    return;
                }
                let Some((key, rel, sp)) = previews.get(next.fetch_add(1, Ordering::Relaxed)) else { return };
                let t = Instant::now();
                let r = fs.read_at(rel, sp.offset, sp.len.min(PREVIEW_CAP) as usize, PREVIEW_DEADLINE);
                let took = ms(t.elapsed());
                let mut g = by.lock().unwrap();
                let p = g.entry(key).or_default();
                match r {
                    Ok(b) => {
                        sh.observe(took);
                        sh.bytes.fetch_add(b.len() as u64, Ordering::Relaxed);
                        sh.bump("bytes", b.len() as u64);
                        p.ms.push(took);
                        p.bytes.push(b.len() as f64);
                        if b.starts_with(&[0xFF, 0xD8]) {
                            p.jpeg += 1;
                        }
                    }
                    Err(e) => {
                        sh.error(&e);
                        p.err += 1;
                    }
                }
                }
            });
        }
    });
    let mut out = Map::new();
    for (k, mut p) in by.into_inner().unwrap() {
        let total: f64 = p.bytes.iter().sum();
        out.insert(
            k.into(),
            json!({ "reads": p.ms.len(), "errors": p.err, "valid_jpeg": p.jpeg, "latency_ms": dist(&p.ms), "bytes": dist(&p.bytes),
                    "total_bytes": total as u64, "ms_per_mb": if total > 0.0 { r1(p.ms.iter().sum::<f64>() / (total / 1e6)) } else { 0.0 } }),
        );
        let _ = pct(&mut p.ms, 0.5);
    }
    Value::Object(out)
}

// ---- mount facts ---------------------------------------------------------------

/// Options worth reporting; anything else (user names, addresses, credentials)
/// stays out of the report.
const OPTION_KEYS: &[&str] = &[
    "vers", "cache", "rsize", "wsize", "actimeo", "acregmax", "acdirmax", "acregmin", "acdirmin", "echo_interval", "soft", "hard", "nounix",
    "noserverino", "serverino", "mfsymlinks", "seal", "sec", "nosharesock", "multichannel", "max_channels", "signing", "noperm", "noatime",
    "relatime", "ro", "rw", "nolock", "proto", "timeo", "retrans", "nfsvers", "bsize", "nobrl", "nocase", "handlecache", "closetimeo",
];

pub fn mount_facts(table: &MountTable, root: &Path) -> Value {
    let Some(e) = table.top_at(root) else {
        return json!({ "found": false });
    };
    let mut opts = Map::new();
    for o in e.options.split(',').chain(e.super_options.split(',')) {
        let (k, v) = o.split_once('=').map_or((o, None), |(k, v)| (k, Some(v)));
        if OPTION_KEYS.contains(&k) {
            opts.insert(k.into(), v.map_or(Value::Bool(true), |v| Value::String(v.into())));
        }
    }
    let (host, share) = if crate::catalog_sources::is_network_class(&e.fs_type) {
        let s = e.source.trim_start_matches('/');
        match s.split_once('/').or_else(|| s.split_once(':')) {
            Some((h, sh)) => (Some(h.to_string()), Some(sh.trim_start_matches('/').to_string())),
            None => (None, Some(s.to_string())),
        }
    } else {
        (None, None)
    };
    json!({
        "found": true, "fs_type": e.fs_type, "host": host, "share": share, "mounted_at": e.mount_point,
        "network": crate::catalog_sources::is_network_class(&e.fs_type), "options": opts,
        "autofs_below": table.idle_autofs_at(root).is_some(),
    })
}

// ---- the run ----------------------------------------------------------------------

pub fn run_bench<F: BenchFs>(fs: &F, root: &Path, mount: Value, cfg: &BenchConfig, interrupted: Box<dyn Fn() -> bool + Send + Sync>) -> Result<Value, String> {
    let workers = cfg.concurrency.clamp(1, MAX_CONCURRENCY);
    let started = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let start = Instant::now();
    let end = start + cfg.duration;
    let sh = Shared {
        start,
        stop: AtomicBool::new(false),
        interrupted,
        gate: Gate { allowed: AtomicUsize::new(workers), resume_at: Mutex::new(None) },
        window: Mutex::new(Vec::new()),
        errors: Mutex::new(BTreeMap::new()),
        bytes: AtomicU64::new(0),
        ops: AtomicU64::new(0),
        series: Mutex::new(BTreeMap::new()),
        bucket: cfg.bucket,
    };
    let scratch = Mutex::new(Scratch::open(&cfg.scratch)?);
    let rx0 = net_rx();
    let cpu0 = cpu_seconds();
    let finished = AtomicBool::new(false);
    let frac = |f: f64| start + Duration::from_secs_f64(cfg.duration.as_secs_f64() * f);

    let (ctl, res, disc, stat, meta, prev) = std::thread::scope(|s| {
        let c = s.spawn(|| control(&sh, cfg, workers, &finished));
        let r = s.spawn(|| sample(&sh, cfg, &finished));

        let disc = discover(fs, root, cfg, &sh, frac(0.4), workers);
        {
            let rows: Vec<(String, String, u64)> = disc.photos.iter().map(|p| (p.rel.clone(), p.ext.clone(), p.size)).collect();
            let mut sc = scratch.lock().unwrap();
            for chunk in rows.chunks(cfg.batch.max(1)) {
                sc.photos(chunk);
            }
        }

        // stat latency: files singly, then directories (sequential: one op at a time)
        let (mut fstat, mut dstat, mut ferr) = (Vec::new(), Vec::new(), 0u64);
        let step = (disc.photos.len() / cfg.stat_sample.max(1)).max(1);
        for p in disc.photos.iter().step_by(step).take(cfg.stat_sample) {
            if sh.done() || Instant::now() >= frac(0.5) {
                break;
            }
            let t = Instant::now();
            match fs.stat_file(&p.rel, cfg.read_deadline) {
                Ok(_) => {
                    let m = ms(t.elapsed());
                    fstat.push(m);
                    sh.observe(m);
                }
                Err(e) => {
                    ferr += 1;
                    sh.error(&e);
                }
            }
        }
        for d in disc.dir_samples.iter().take(100) {
            if sh.done() || Instant::now() >= frac(0.55) {
                break;
            }
            let path = if d.is_empty() { root.to_path_buf() } else { root.join(d) };
            let t = Instant::now();
            match fs.stat_dir(&path, cfg.read_deadline) {
                Ok(_) => dstat.push(ms(t.elapsed())),
                Err(e) => sh.error(&e),
            }
        }
        let stat = json!({ "file": dist(&fstat), "file_errors": ferr, "dir": dist(&dstat),
                           "note": "files were listed seconds earlier, so an SMB attribute cache (actimeo) may answer some of these without a round trip" });

        let meta = read_metadata(fs, cfg, &sh, &disc.photos, frac(0.85), &scratch, workers);
        let prev = read_previews(fs, &sh, &meta.previews, end, workers.min(2));
        finished.store(true, Ordering::Relaxed);
        let (ctl, res) = (c.join().unwrap(), r.join().unwrap());
        (ctl, res, disc, stat, meta, prev)
    });

    let wall = start.elapsed();
    let cpu1 = cpu_seconds();
    let rx1 = net_rx();
    let sql = scratch.into_inner().unwrap().finish();
    let series = sh.series.lock().unwrap().clone();
    let nb = series.values().map(|v| v.len()).max().unwrap_or(0).max(res.buckets.keys().max().map_or(0, |k| k + 1));
    let get = |name: &str, i: usize| series.get(name).and_then(|v| v.get(i)).copied().unwrap_or(0);
    let bw = cfg.bucket.as_secs_f64();
    let timeline: Vec<Value> = (0..nb)
        .map(|i| {
            let (cpu, wallb, rss) = res.buckets.get(&i).copied().unwrap_or_default();
            json!({ "t_s": i as f64 * bw, "dirs_per_s": r1(get("dirs", i) as f64 / bw), "entries_per_s": r1(get("entries", i) as f64 / bw),
                    "files_per_s": r1(get("files", i) as f64 / bw), "mb_per_s": r1(get("bytes", i) as f64 / 1e6 / bw),
                    "cpu_pct": if wallb > 0.0 { r1(cpu / wallb * 100.0) } else { 0.0 }, "rss_mb": r1(rss as f64 / 1e6) })
        })
        .collect();

    let mut sizes_by_ext = Map::new();
    let photos_n: u64 = disc.exts.values().map(|e| e.count).sum();
    let mut disc = disc;
    for (k, mut v) in std::mem::take(&mut disc.exts) {
        let max = v.sizes.iter().cloned().fold(0.0, f64::max);
        sizes_by_ext.insert(k, json!({ "count": v.count, "size_p50": pct(&mut v.sizes, 0.5) as u64, "size_p95": pct(&mut v.sizes, 0.95) as u64, "size_max": max as u64 }));
    }
    // listing latency by directory size
    let mut bands: BTreeMap<&str, Vec<f64>> = BTreeMap::new();
    for (n, m) in &disc.by_size {
        let b = match *n {
            0..=100 => "up_to_100",
            101..=1000 => "101_to_1000",
            _ => "over_1000",
        };
        bands.entry(b).or_default().push(*m);
    }
    let bands: Map<String, Value> = bands.into_iter().map(|(k, v)| (k.to_string(), dist(&v))).collect();
    let epd = disc.entries_per_dir.clone();
    let secs = disc.elapsed.as_secs_f64().max(0.001);

    let mut fmts = Map::new();
    let mut ladder = [0u64; 3];
    let (mut total_files, mut total_fail) = (0u64, 0u64);
    for (k, f) in &meta.fmts {
        for i in 0..3 {
            ladder[i] += f.ladder[i];
        }
        total_files += f.files;
        total_fail += f.corrupt + f.unsupported + f.read_errors;
        fmts.insert(
            k.to_string(),
            json!({
                "files": f.files, "parsed": f.done, "truncated_after_ladder": f.truncated, "retried": f.retried,
                "ladder_finished_at": { "64k": f.ladder[0], "256k": f.ladder[1], "1m": f.ladder[2] },
                "parse_failures": { "unsupported": f.unsupported, "corrupt": f.corrupt, "read_errors": f.read_errors },
                "read_latency_ms": dist(&f.read_ms), "size_read_bytes": dist(&f.final_bytes), "bytes_asked_by_parser": dist(&f.asked),
                "bytes_read_total": f.bytes, "files_with_preview": f.previews,
            }),
        );
    }
    let errors = sh.errors.lock().unwrap().clone();
    let timeouts = errors.get("timeout").copied().unwrap_or(0);
    let err_total: u64 = errors.values().sum();
    let stopped = if (sh.interrupted)() { "interrupted" } else { meta.stopped.unwrap_or("finished") };
    let user = cpu1.zip(cpu0).map(|(a, b)| a.0 - b.0);
    let sys = cpu1.zip(cpu0).map(|(a, b)| a.1 - b.1);

    Ok(json!({
        "kind": "immage-catalog-bench", "version": 1, "started_unix": started, "wall_s": r1(wall.as_secs_f64()),
        "config": { "concurrency": workers, "max_files": cfg.max_files, "duration_s": cfg.duration.as_secs(), "latency_limit_ms": cfg.latency_limit_ms,
                    "bucket_s": cfg.bucket.as_secs(), "preview_sample_per_format": cfg.thumb_sample, "stat_sample": cfg.stat_sample },
        "environment": { "os": std::env::consts::OS, "arch": std::env::consts::ARCH, "cpus": std::thread::available_parallelism().map_or(0, |n| n.get()),
                         "immage": env!("CARGO_PKG_VERSION"), "debug_build": cfg!(debug_assertions), "mount": mount },
        "discovery": {
            "dirs": disc.dirs, "entries": disc.entries, "photos": photos_n, "xmp_sidecars": disc.xmp, "other_files": disc.other_files,
            "skipped_hidden_or_special": disc.skipped, "elapsed_s": r1(secs), "dirs_per_s": r1(disc.dirs as f64 / secs), "entries_per_s": r1(disc.entries as f64 / secs),
            "walk_complete": disc.complete, "list_latency_ms": dist(&disc.list_ms), "list_latency_by_dir_size_ms": bands,
            "entries_per_dir": dist(&epd), "by_format": sizes_by_ext,
        },
        "stat": stat,
        "metadata": {
            "files": meta.files, "elapsed_s": r1(meta.elapsed.as_secs_f64()), "files_per_s": r1(meta.files as f64 / meta.elapsed.as_secs_f64().max(0.001)),
            "ladder_finished_at": { "64k": ladder[0], "256k": ladder[1], "1m": ladder[2] },
            "files_total": total_files, "parse_failures_total": total_fail, "by_format": fmts,
        },
        "thumbnails": { "note": "one ranged read of the embedded preview, as the thumbnail step does; JPEGs without a preview are not read whole here", "by_format": prev },
        "concurrency": { "start": workers, "final": sh.gate.allowed.load(Ordering::Relaxed), "lowest": ctl.min_allowed, "backoffs": ctl.backoffs, "events": ctl.events },
        "safety": { "latency_limit_ms": cfg.latency_limit_ms, "window_p95_peak_ms": r1(ctl.peak_p95), "backoffs": ctl.backoffs, "ended": stopped },
        "timeline": timeline,
        "sqlite": sql,
        "process": { "cpu_user_s": user.map(r1), "cpu_system_s": sys.map(r1), "avg_cpu_pct": user.zip(sys).map(|(u, s)| r1((u + s) / wall.as_secs_f64().max(0.001) * 100.0)),
                     "peak_rss_mb": r1(res.peak_rss as f64 / 1e6) },
        "network": { "payload_bytes_read": sh.bytes.load(Ordering::Relaxed), "system_rx_bytes_delta": rx1.zip(rx0).map(|(a, b)| a.saturating_sub(b)),
                     "note": "payload counts the bytes asked for by head and preview reads; the system figure is every non-loopback interface and includes other traffic and SMB overhead" },
        "errors": { "total": err_total, "timeouts": timeouts, "by_kind": errors },
    }))
}

// ---- the readable report -----------------------------------------------------------

pub fn render_report(v: &Value) -> String {
    let f = |x: &Value| match x {
        Value::Number(n) => n.as_f64().map_or("-".into(), |f| if f.fract() == 0.0 { format!("{}", f as i64) } else { format!("{f:.1}") }),
        Value::Null => "-".into(),
        o => o.to_string().trim_matches('"').to_string(),
    };
    let d = |x: &Value| format!("p50 {} / p95 {} / p99 {} / max {} ms (n={})", f(&x["p50"]), f(&x["p95"]), f(&x["p99"]), f(&x["max"]), f(&x["count"]));
    let mb = |x: &Value| x.as_f64().map_or("-".into(), |b| format!("{:.1} MB", b / 1e6));
    let mut s = String::new();
    let mut w = |l: String| {
        s.push_str(&l);
        s.push('\n');
    };
    let e = &v["environment"];
    w("Immage catalog benchmark (read-only, aggregates only)".into());
    w(format!("run {} s, {} {} ({} cpus), build {}{}", f(&v["wall_s"]), f(&e["os"]), f(&e["arch"]), f(&e["cpus"]), f(&e["immage"]), if e["debug_build"] == true { "  *** DEBUG BUILD: numbers are not representative ***" } else { "" }));
    let m = &e["mount"];
    if m["found"] == true {
        w(format!("mount: {} host={} share={} at {}", f(&m["fs_type"]), f(&m["host"]), f(&m["share"]), f(&m["mounted_at"])));
        w(format!("options: {}", f(&m["options"])));
    } else {
        w("mount: not found in mountinfo".into());
    }
    let c = &v["config"];
    w(format!("config: concurrency {}, max files {}, duration {} s, latency limit {} ms", f(&c["concurrency"]), f(&c["max_files"]), f(&c["duration_s"]), f(&c["latency_limit_ms"])));
    let x = &v["discovery"];
    w(String::new());
    w("DISCOVERY".into());
    w(format!("  {} dirs, {} entries, {} photos, {} xmp in {} s ({}; hidden/special skipped {})", f(&x["dirs"]), f(&x["entries"]), f(&x["photos"]), f(&x["xmp_sidecars"]), f(&x["elapsed_s"]), if x["walk_complete"] == true { "walk complete" } else { "walk cut off" }, f(&x["skipped_hidden_or_special"])));
    w(format!("  rate: {} dirs/s, {} entries/s", f(&x["dirs_per_s"]), f(&x["entries_per_s"])));
    w(format!("  per-dir list latency: {}", d(&x["list_latency_ms"])));
    if let Some(b) = x["list_latency_by_dir_size_ms"].as_object() {
        for (k, val) in b {
            w(format!("    {k} entries: {}", d(val)));
        }
    }
    w(format!("  entries per dir: p50 {} p95 {} max {}", f(&x["entries_per_dir"]["p50"]), f(&x["entries_per_dir"]["p95"]), f(&x["entries_per_dir"]["max"])));
    if let Some(b) = x["by_format"].as_object() {
        for (k, val) in b {
            w(format!("    {k:<10} {:>7} files, size p50 {} p95 {} max {}", f(&val["count"]), mb(&val["size_p50"]), mb(&val["size_p95"]), mb(&val["size_max"])));
        }
    }
    w(String::new());
    w("STAT".into());
    w(format!("  file: {}", d(&v["stat"]["file"])));
    w(format!("  dir:  {}", d(&v["stat"]["dir"])));
    let md = &v["metadata"];
    w(String::new());
    w("METADATA HEAD READS (64 KB, then 256 KB, then 1 MB when the parser asks)".into());
    w(format!("  {} files in {} s = {} files/s; finished at 64k/256k/1m: {}/{}/{}; parse failures {}", f(&md["files"]), f(&md["elapsed_s"]), f(&md["files_per_s"]), f(&md["ladder_finished_at"]["64k"]), f(&md["ladder_finished_at"]["256k"]), f(&md["ladder_finished_at"]["1m"]), f(&md["parse_failures_total"])));
    if let Some(b) = md["by_format"].as_object() {
        for (k, val) in b {
            w(format!("  {k}: {} files, parsed {}, retried {}, fail unsupported/corrupt/read {}/{}/{}", f(&val["files"]), f(&val["parsed"]), f(&val["retried"]), f(&val["parse_failures"]["unsupported"]), f(&val["parse_failures"]["corrupt"]), f(&val["parse_failures"]["read_errors"])));
            w(format!("      read latency: {}", d(&val["read_latency_ms"])));
            w(format!("      bytes read per file p50 {} p95 {}; parser asked p95 {}; ladder 64k/256k/1m {}/{}/{}", f(&val["size_read_bytes"]["p50"]), f(&val["size_read_bytes"]["p95"]), f(&val["bytes_asked_by_parser"]["p95"]), f(&val["ladder_finished_at"]["64k"]), f(&val["ladder_finished_at"]["256k"]), f(&val["ladder_finished_at"]["1m"])));
        }
    }
    w(String::new());
    w("THUMBNAIL SOURCE (embedded preview, one ranged read)".into());
    if let Some(b) = v["thumbnails"]["by_format"].as_object() {
        for (k, val) in b {
            w(format!("  {k}: {} reads, {} valid JPEG, {} errors, {}; bytes p50 {} p95 {}; {} ms/MB", f(&val["reads"]), f(&val["valid_jpeg"]), f(&val["errors"]), d(&val["latency_ms"]), f(&val["bytes"]["p50"]), f(&val["bytes"]["p95"]), f(&val["ms_per_mb"])));
        }
    }
    let cc = &v["concurrency"];
    w(String::new());
    w("CONCURRENCY AND SAFETY".into());
    w(format!("  workers start {} lowest {} final {}; backoffs {}; peak window p95 {} ms (limit {} ms); ended: {}", f(&cc["start"]), f(&cc["lowest"]), f(&cc["final"]), f(&cc["backoffs"]), f(&v["safety"]["window_p95_peak_ms"]), f(&v["safety"]["latency_limit_ms"]), f(&v["safety"]["ended"])));
    w(String::new());
    w("THROUGHPUT OVER TIME".into());
    w("  t(s)  dirs/s  entries/s  files/s   MB/s  cpu%  rss MB".into());
    if let Some(t) = v["timeline"].as_array() {
        for r in t {
            w(format!("  {:>4} {:>7} {:>10} {:>8} {:>6} {:>5} {:>7}", f(&r["t_s"]), f(&r["dirs_per_s"]), f(&r["entries_per_s"]), f(&r["files_per_s"]), f(&r["mb_per_s"]), f(&r["cpu_pct"]), f(&r["rss_mb"])));
        }
    }
    let q = &v["sqlite"];
    w(String::new());
    w("SQLITE (scratch database, deleted)".into());
    w(format!("  {} rows in {} batches: {}; max WAL {}; db {}; checkpoint {} ms", f(&q["rows"]), f(&q["batches"]["count"]), d(&q["batches"]), mb(&q["wal_max_bytes"]), mb(&q["db_bytes"]), f(&q["checkpoint_ms"])));
    let p = &v["process"];
    let n = &v["network"];
    w(String::new());
    w("PROCESS AND NETWORK".into());
    w(format!("  cpu user {} s, system {} s, average {} %; peak RSS {} MB", f(&p["cpu_user_s"]), f(&p["cpu_system_s"]), f(&p["avg_cpu_pct"]), f(&p["peak_rss_mb"])));
    w(format!("  payload read {}; machine-wide rx during the run {}", mb(&n["payload_bytes_read"]), mb(&n["system_rx_bytes_delta"])));
    w(format!("  errors {} (timeouts {}): {}", f(&v["errors"]["total"]), f(&v["errors"]["timeouts"]), f(&v["errors"]["by_kind"])));
    s
}

#[cfg(test)]
#[path = "catalog_bench_tests.rs"]
mod tests;
