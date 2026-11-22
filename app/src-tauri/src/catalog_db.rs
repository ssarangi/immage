// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The Catalog database: `catalog.sqlite` in the runtime directory. It lives on
//! LOCAL disk only (WAL does not work on network file systems) and, unlike the
//! search cache, is never disposable: schema changes go through the migration
//! ladder below, which never drops extracted data, and a copy of the file is
//! taken before any step runs on a database that has content.
//!
//! One dedicated writer thread owns the only writable connection and is fed by
//! a bounded channel; directory results are applied in transactions of about
//! [`BATCH_ROWS`] rows. Readers open their own read-only connections.
//! A second process (the CLI beside a running app) cannot open the database for
//! writing: an advisory lock file refuses it with [`CatalogError::Locked`].

use rusqlite::{params, Connection, OpenFlags, OptionalExtension, Transaction};
use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

pub const FILE: &str = "catalog.sqlite";
pub const SCHEMA_VERSION: i64 = 2;
/// Rows (photos, plus one per directory) per write transaction. The NAS
/// baseline recommends 500-2000.
pub const BATCH_ROWS: usize = 1000;
/// Batches the scanner may have queued before it waits for the writer.
const QUEUE_DEPTH: usize = 64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogError {
    /// Another process (a running Immage) holds the database for writing.
    Locked,
    /// Written by a newer Immage; this one must not touch it.
    Newer { found: i64, known: i64 },
    Corrupt(String),
    Other(String),
}

impl fmt::Display for CatalogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Locked => write!(f, "The catalog is open in Immage. Close it first."),
            Self::Newer { found, known } => write!(
                f,
                "The catalog was made by a newer Immage (schema {found}, this one knows {known})."
            ),
            Self::Corrupt(e) => write!(f, "The catalog database is damaged: {e}"),
            Self::Other(e) => write!(f, "{e}"),
        }
    }
}

impl From<rusqlite::Error> for CatalogError {
    fn from(e: rusqlite::Error) -> Self {
        let text = e.to_string();
        let lower = text.to_ascii_lowercase();
        if ["not a database", "malformed", "corrupt"].iter().any(|p| lower.contains(p)) {
            Self::Corrupt(text)
        } else {
            Self::Other(text)
        }
    }
}

pub fn now_s() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs() as i64)
}

// ---- the migration ladder -------------------------------------------------

pub type Step = fn(&Transaction) -> rusqlite::Result<()>;

/// Step N takes the database from user_version N-1 to N. Append only: a step
/// that has shipped is never edited.
pub const LADDER: &[Step] = &[v1, v2_metadata];

fn v1(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        "
        CREATE TABLE meta(key TEXT PRIMARY KEY, value TEXT NOT NULL) WITHOUT ROWID;

        -- a folder tree on a volume. identity is the VOLUME (volume_key), not
        -- the mount path, which is only where it is mounted now
        CREATE TABLE sources(
          id INTEGER PRIMARY KEY,
          name TEXT NOT NULL,
          kind TEXT NOT NULL DEFAULT 'local',          -- nas | drive | local
          volume_key TEXT NOT NULL,
          fs_type TEXT NOT NULL DEFAULT '',
          host TEXT, host_hash TEXT, share TEXT,
          mount_path TEXT,
          rel_root TEXT NOT NULL DEFAULT '',            -- under the volume root, '/' separated
          enabled INTEGER NOT NULL DEFAULT 1,
          treat_as_network INTEGER,                     -- NULL: decide from the file system
          scan_policy TEXT NOT NULL DEFAULT 'manual',   -- manual | launch | every=<hours>h
          include_subfolders INTEGER NOT NULL DEFAULT 1,
          dir_skip INTEGER NOT NULL DEFAULT 0,          -- directory fingerprint skipping; OFF
          state TEXT NOT NULL DEFAULT 'never_indexed',
          error TEXT,
          last_scan_start INTEGER, last_scan_end INTEGER, last_ok INTEGER,
          created_at INTEGER NOT NULL
        );
        CREATE INDEX sources_volume ON sources(volume_key);

        CREATE TABLE dirs(
          id INTEGER PRIMARY KEY,
          source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
          parent_id INTEGER,
          rel_path TEXT NOT NULL,                       -- '' is the source root
          mtime_ns INTEGER,
          entry_count INTEGER,
          scanned_at INTEGER,
          status TEXT NOT NULL DEFAULT 'ok',            -- ok | stalled | denied | error | gone
          error TEXT,
          seen_run INTEGER,
          UNIQUE(source_id, rel_path)
        );
        CREATE INDEX dirs_parent ON dirs(source_id, parent_id);

        CREATE TABLE photos(
          id INTEGER PRIMARY KEY,
          source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
          dir_id INTEGER NOT NULL REFERENCES dirs(id) ON DELETE CASCADE,
          name TEXT NOT NULL,
          rel_path TEXT NOT NULL,
          ext TEXT NOT NULL,                            -- lower case
          size INTEGER NOT NULL,
          mtime_ns INTEGER NOT NULL,
          status TEXT NOT NULL DEFAULT 'present',       -- present | missing
          discovered_at INTEGER NOT NULL,
          seen_at INTEGER NOT NULL,                     -- last time a scan wrote this row
          first_run INTEGER,                            -- scan run that found it
          changed_run INTEGER,                          -- scan run that last changed it
          xmp_kind INTEGER NOT NULL DEFAULT 0,          -- 0 none, 1 name.ext.xmp, 2 name.xmp
          xmp_size INTEGER, xmp_mtime_ns INTEGER,
          -- reserved for the metadata phase; NULL until it has read the file
          meta_state INTEGER NOT NULL DEFAULT 0,        -- 0 not read, 1 read, 2 failed
          taken_at INTEGER, tz_offset INTEGER,
          make TEXT, model TEXT, body_serial TEXT, lens TEXT,
          focal REAL, aperture REAL, iso INTEGER, exposure REAL,
          width INTEGER, height INTEGER, gps_lat REAL, gps_lon REAL,
          rating INTEGER, label INTEGER, rejected INTEGER,
          content_key TEXT, extra_json TEXT,
          UNIQUE(source_id, rel_path)
        );
        CREATE INDEX photos_dir ON photos(dir_id, name);
        CREATE INDEX photos_status ON photos(source_id, status);

        CREATE TABLE scan_runs(
          id INTEGER PRIMARY KEY,
          source_id INTEGER NOT NULL REFERENCES sources(id) ON DELETE CASCADE,
          job_id INTEGER,
          mode TEXT NOT NULL DEFAULT 'full',
          state TEXT NOT NULL,        -- running | complete | incomplete | offline | cancelled | interrupted | failed
          started_at INTEGER NOT NULL, finished_at INTEGER,
          elapsed_ms INTEGER,
          found INTEGER NOT NULL DEFAULT 0, dirs INTEGER NOT NULL DEFAULT 0,
          new INTEGER NOT NULL DEFAULT 0, changed INTEGER NOT NULL DEFAULT 0,
          unchanged INTEGER NOT NULL DEFAULT 0, missing INTEGER NOT NULL DEFAULT 0,
          unreadable INTEGER NOT NULL DEFAULT 0,
          incomplete_json TEXT, error TEXT
        );
        CREATE INDEX scan_runs_source ON scan_runs(source_id, id);

        -- photos a run found gone from a directory it listed; applied only if
        -- the run ends complete, so it survives a kill and a resume
        CREATE TABLE scan_gone(
          run_id INTEGER NOT NULL, photo_id INTEGER NOT NULL,
          PRIMARY KEY(run_id, photo_id)
        ) WITHOUT ROWID;

        -- what a run skipped or could not read: symlinks, names that are not
        -- UTF-8, mount boundaries
        CREATE TABLE scan_notes(
          id INTEGER PRIMARY KEY,
          run_id INTEGER NOT NULL, source_id INTEGER NOT NULL,
          dir_rel TEXT NOT NULL, name TEXT NOT NULL,
          kind TEXT NOT NULL, detail TEXT
        );
        CREATE INDEX scan_notes_source ON scan_notes(source_id, run_id);

        CREATE TABLE jobs(
          id INTEGER PRIMARY KEY,
          kind TEXT NOT NULL,         -- scan
          source_id INTEGER REFERENCES sources(id) ON DELETE CASCADE,
          state TEXT NOT NULL,        -- queued | running | paused | done | failed | cancelled | interrupted
          payload TEXT, checkpoint TEXT, progress TEXT, error TEXT,
          run_id INTEGER,
          created_at INTEGER NOT NULL, started_at INTEGER,
          updated_at INTEGER NOT NULL, finished_at INTEGER
        );
        CREATE INDEX jobs_state ON jobs(state);
        ",
    )?;
    tx.execute("INSERT INTO meta(key,value) VALUES('created_by', ?1)", [env!("CARGO_PKG_VERSION")])?;
    Ok(())
}

/// The metadata phase (catalog_meta_job.rs): the typed columns the extractor
/// fills, what was read of a sidecar (so a sidecar-only change is re-read
/// alone), the keywords, and the indexes the metadata job and the relink step
/// query. `meta_state` is now 0 pending, 1 done, 2 failed (retried by the next
/// run), 3 unparsed (the extractor cannot read the file; a later engine fallback
/// picks these up). `taken_at` is the written wall-clock time read as UTC, with
/// `tz_offset` (seconds east of UTC) when the file said it.
fn v2_metadata(tx: &Transaction) -> rusqlite::Result<()> {
    tx.execute_batch(
        "
        ALTER TABLE photos ADD COLUMN meta_error TEXT;
        ALTER TABLE photos ADD COLUMN meta_at INTEGER;
        ALTER TABLE photos ADD COLUMN subsec_ms INTEGER;
        ALTER TABLE photos ADD COLUMN orientation INTEGER;
        ALTER TABLE photos ADD COLUMN focal35 REAL;
        ALTER TABLE photos ADD COLUMN exposure_bias REAL;
        ALTER TABLE photos ADD COLUMN raw_width INTEGER;
        ALTER TABLE photos ADD COLUMN raw_height INTEGER;
        ALTER TABLE photos ADD COLUMN preview_offset INTEGER;
        ALTER TABLE photos ADD COLUMN preview_len INTEGER;
        ALTER TABLE photos ADD COLUMN preview_w INTEGER;
        ALTER TABLE photos ADD COLUMN preview_h INTEGER;
        ALTER TABLE photos ADD COLUMN gps_alt REAL;
        ALTER TABLE photos ADD COLUMN software TEXT;
        ALTER TABLE photos ADD COLUMN container TEXT;
        ALTER TABLE photos ADD COLUMN is_raw INTEGER;
        ALTER TABLE photos ADD COLUMN edited INTEGER;
        ALTER TABLE photos ADD COLUMN head_bytes INTEGER;
        ALTER TABLE photos ADD COLUMN xmp_seen_size INTEGER;
        ALTER TABLE photos ADD COLUMN xmp_seen_mtime_ns INTEGER;

        CREATE TABLE keywords(
          photo_id INTEGER NOT NULL REFERENCES photos(id) ON DELETE CASCADE,
          keyword TEXT NOT NULL,
          src TEXT NOT NULL DEFAULT 'xmp',
          PRIMARY KEY(photo_id, keyword, src)
        ) WITHOUT ROWID;
        CREATE INDEX keywords_word ON keywords(keyword, photo_id);

        CREATE INDEX photos_content_key ON photos(source_id, content_key) WHERE content_key IS NOT NULL;
        CREATE INDEX photos_meta_state ON photos(source_id, meta_state) WHERE status='present';
        ",
    )
}

#[allow(dead_code)]
pub struct MigrateReport {
    pub from: i64,
    pub to: i64,
}

fn user_version(conn: &Connection) -> rusqlite::Result<i64> {
    conn.query_row("PRAGMA user_version", [], |r| r.get(0))
}

/// Brings `conn` to the ladder's head, one transaction per step, so a crash
/// leaves the database at a whole version. A database with content is copied
/// first (`<file>.pre-v<N>.bak`, beside it); a newer one is refused.
pub fn migrate(conn: &mut Connection, ladder: &[Step], backup_to: Option<&Path>) -> Result<MigrateReport, CatalogError> {
    let from = user_version(conn)?;
    let head = ladder.len() as i64;
    if from > head {
        return Err(CatalogError::Newer { found: from, known: head });
    }
    if from == head {
        return Ok(MigrateReport { from, to: head });
    }
    if from > 0 {
        if let Some(path) = backup_to {
            let copy = path.with_file_name(format!(
                "{}.pre-v{from}.bak",
                path.file_name().unwrap_or_default().to_string_lossy()
            ));
            if !copy.exists() {
                conn.execute("VACUUM INTO ?1", [copy.to_string_lossy().as_ref()])?;
            }
        }
    }
    for (i, step) in ladder.iter().enumerate().skip(from as usize) {
        let tx = conn.transaction()?;
        step(&tx)?;
        tx.pragma_update(None, "user_version", i as i64 + 1)?;
        tx.commit()?;
    }
    Ok(MigrateReport { from, to: head })
}

fn configure(conn: &Connection) -> rusqlite::Result<()> {
    conn.busy_timeout(std::time::Duration::from_secs(10))?;
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

// ---- scan data handed to the writer --------------------------------------

#[derive(Default)]
pub struct Counters {
    pub found: AtomicU64,
    pub new: AtomicU64,
    pub changed: AtomicU64,
    pub unchanged: AtomicU64,
    pub gone: AtomicU64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRec {
    pub name: String,
    pub ext: String,
    pub size: u64,
    pub mtime_ns: i64,
    /// 0 none, 1 `name.ext.xmp`, 2 `name.xmp`, with the sidecar's own stamp.
    pub xmp: Option<(u8, u64, i64)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirStatus {
    Ok,
    Stalled,
    Denied,
    Error,
}

impl DirStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Stalled => "stalled",
            Self::Denied => "denied",
            Self::Error => "error",
        }
    }
}

pub struct DirBatch {
    pub source_id: i64,
    pub run_id: i64,
    pub rel_path: String,
    pub parent_rel: Option<String>,
    pub mtime_ns: Option<i64>,
    pub entry_count: u32,
    pub status: DirStatus,
    pub error: Option<String>,
    pub files: Vec<FileRec>,
    /// The directory was not listed (fingerprint skip): only its `seen_run`
    /// moves; its photos are left as they are.
    pub skipped: bool,
    /// A resumed run may have written this directory's gone list before.
    pub redo: bool,
    pub counters: Arc<Counters>,
}

#[derive(Debug, Clone)]
pub struct Note {
    pub dir_rel: String,
    pub name: String,
    pub kind: &'static str,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunTotals {
    pub found: u64,
    pub dirs: u64,
    pub new: u64,
    pub changed: u64,
    pub unchanged: u64,
    pub missing: u64,
    pub unreadable: u64,
    pub incomplete_dirs: Vec<(String, String, Option<String>)>,
}

pub struct Finish {
    pub source_id: i64,
    pub run_id: i64,
    pub job_id: Option<i64>,
    /// "complete", "incomplete", "offline", "cancelled", "interrupted", "failed".
    pub state: &'static str,
    /// Mark-and-sweep. Only ever true for a state of "complete".
    pub sweep: bool,
    pub error: Option<String>,
}

type Exec = Box<dyn FnOnce(&mut Connection) + Send>;

enum Msg {
    Dir(Box<DirBatch>),
    Notes(i64, i64, Vec<Note>),
    Sync(SyncSender<()>),
    Exec(Exec),
    Stop,
}

impl Msg {
    fn rows(&self) -> usize {
        match self {
            Msg::Dir(d) => d.files.len() + 1,
            Msg::Notes(_, _, n) => n.len(),
            _ => 0,
        }
    }
    fn batchable(&self) -> bool {
        matches!(self, Msg::Dir(_) | Msg::Notes(..))
    }
}

// ---- the handle -----------------------------------------------------------

pub struct CatalogDb {
    pub path: PathBuf,
    tx: Mutex<Option<SyncSender<Msg>>>,
    thread: Mutex<Option<JoinHandle<()>>>,
    failure: Arc<Mutex<Option<String>>>,
    /// Held for the life of the handle; dropping it releases the lock.
    _lock: Option<std::fs::File>,
}

impl CatalogDb {
    /// Opens (creating and migrating if needed) for reading and writing.
    pub fn open(path: &Path) -> Result<Self, CatalogError> {
        Self::open_with(path, LADDER)
    }

    pub fn open_with(path: &Path, ladder: &[Step]) -> Result<Self, CatalogError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| CatalogError::Other(format!("{}: {e}", dir.display())))?;
        }
        let lock = take_lock(path)?;
        let mut conn = Connection::open(path)?;
        configure(&conn)?;
        migrate(&mut conn, ladder, Some(path))?;
        reconcile_on_open(&conn)?;
        let failure = Arc::new(Mutex::new(None));
        let (tx, rx) = sync_channel::<Msg>(QUEUE_DEPTH);
        let fail = failure.clone();
        let thread = std::thread::Builder::new()
            .name("catalog-writer".into())
            .spawn(move || writer(conn, rx, fail))
            .map_err(|e| CatalogError::Other(e.to_string()))?;
        Ok(Self {
            path: path.to_path_buf(),
            tx: Mutex::new(Some(tx)),
            thread: Mutex::new(Some(thread)),
            failure,
            _lock: lock,
        })
    }

    /// A read-only connection: no lock, no migration, no writer. Works beside
    /// a running app. A database that needs migrating is refused.
    pub fn reader(path: &Path) -> Result<Connection, CatalogError> {
        if !path.exists() {
            return Err(CatalogError::Other(format!("No catalog at {}", path.display())));
        }
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
        conn.busy_timeout(std::time::Duration::from_secs(10))?;
        let v = user_version(&conn)?;
        if v > SCHEMA_VERSION {
            return Err(CatalogError::Newer { found: v, known: SCHEMA_VERSION });
        }
        if v < SCHEMA_VERSION {
            return Err(CatalogError::Other("The catalog needs upgrading: run a command that can write first.".into()));
        }
        Ok(conn)
    }

    pub fn read(&self) -> Result<Connection, CatalogError> {
        Self::reader(&self.path)
    }

    fn sender(&self) -> Result<SyncSender<Msg>, String> {
        self.tx.lock().unwrap().clone().ok_or_else(|| "the catalog is closed".to_string())
    }

    /// A write error the writer hit while applying scan batches, if any.
    pub fn failure(&self) -> Option<String> {
        self.failure.lock().unwrap().clone()
    }

    pub fn send_dir(&self, batch: DirBatch) -> Result<(), String> {
        self.sender()?.send(Msg::Dir(Box::new(batch))).map_err(|_| "the catalog writer stopped".to_string())
    }

    pub fn send_notes(&self, source_id: i64, run_id: i64, notes: Vec<Note>) -> Result<(), String> {
        if notes.is_empty() {
            return Ok(());
        }
        self.sender()?.send(Msg::Notes(source_id, run_id, notes)).map_err(|_| "the catalog writer stopped".to_string())
    }

    /// Waits until everything sent before is committed.
    pub fn sync(&self) -> Result<(), String> {
        let (tx, rx) = sync_channel(1);
        self.sender()?.send(Msg::Sync(tx)).map_err(|_| "the catalog writer stopped".to_string())?;
        rx.recv().map_err(|_| "the catalog writer stopped".to_string())
    }

    /// Runs `f` on the writer's connection, in order with the scan batches
    /// before it, and returns its result.
    pub fn exec<T: Send + 'static>(
        &self,
        f: impl FnOnce(&mut Connection) -> Result<T, String> + Send + 'static,
    ) -> Result<T, String> {
        let (tx, rx) = sync_channel(1);
        let job: Exec = Box::new(move |c| {
            let _ = tx.send(f(c));
        });
        self.sender()?.send(Msg::Exec(job)).map_err(|_| "the catalog writer stopped".to_string())?;
        rx.recv().map_err(|_| "the catalog writer stopped".to_string())?
    }

    /// Ends a run: the sweep (only when `finish.sweep`), the run's totals, the
    /// source's state, and the job.
    pub fn finish_run(&self, finish: Finish) -> Result<RunTotals, String> {
        self.exec(move |c| finish_run(c, &finish))
    }

    pub fn close(&self) {
        if let Some(tx) = self.tx.lock().unwrap().take() {
            let _ = tx.send(Msg::Stop);
        }
        if let Some(t) = self.thread.lock().unwrap().take() {
            let _ = t.join();
        }
    }
}

impl Drop for CatalogDb {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(unix)]
fn take_lock(path: &Path) -> Result<Option<std::fs::File>, CatalogError> {
    use std::os::unix::io::AsRawFd;
    let lock_path = path.with_file_name(format!("{}.lock", path.file_name().unwrap_or_default().to_string_lossy()));
    let f = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| CatalogError::Other(format!("{}: {e}", lock_path.display())))?;
    // flock is per open file description, so two handles in one process also exclude each other
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(CatalogError::Locked);
    }
    Ok(Some(f))
}

#[cfg(not(unix))]
fn take_lock(_path: &Path) -> Result<Option<std::fs::File>, CatalogError> {
    Ok(None)
}

/// Whatever was running when the last process died is not running now. A scan
/// job keeps its checkpoint and becomes `interrupted`, which a new scan of the
/// source resumes; a run that never finished is closed the same way.
fn reconcile_on_open(conn: &Connection) -> rusqlite::Result<()> {
    let now = now_s();
    conn.execute(
        "UPDATE jobs SET state='interrupted', updated_at=?1 WHERE state IN ('running','paused','queued')",
        [now],
    )?;
    conn.execute("UPDATE scan_runs SET state='interrupted' WHERE state='running'", [])?;
    conn.execute(
        "UPDATE sources SET state='interrupted' WHERE state='indexing'",
        [],
    )?;
    Ok(())
}

// ---- the writer -----------------------------------------------------------

fn writer(mut conn: Connection, rx: Receiver<Msg>, failure: Arc<Mutex<Option<String>>>) {
    let mut carry: Option<Msg> = None;
    loop {
        let msg = match carry.take() {
            Some(m) => m,
            None => match rx.recv() {
                Ok(m) => m,
                Err(_) => break,
            },
        };
        match msg {
            Msg::Stop => break,
            Msg::Sync(tx) => {
                let _ = tx.send(());
            }
            Msg::Exec(f) => f(&mut conn),
            first => {
                // a transaction of about BATCH_ROWS rows, ended by a barrier or an empty queue
                let mut rows = first.rows();
                let mut batch = vec![first];
                while rows < BATCH_ROWS {
                    match rx.try_recv() {
                        Ok(m) if m.batchable() => {
                            rows += m.rows();
                            batch.push(m);
                        }
                        Ok(m) => {
                            carry = Some(m);
                            break;
                        }
                        Err(_) => break,
                    }
                }
                if let Err(e) = apply(&mut conn, &batch) {
                    eprintln!("catalog: write failed: {e}");
                    *failure.lock().unwrap() = Some(e.to_string());
                }
            }
        }
    }
    let _ = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)");
}

struct Existing {
    id: i64,
    size: i64,
    mtime_ns: i64,
    status: String,
    xmp_kind: i64,
    xmp_size: Option<i64>,
    xmp_mtime_ns: Option<i64>,
}

fn apply(conn: &mut Connection, batch: &[Msg]) -> rusqlite::Result<()> {
    let tx = conn.transaction()?;
    for m in batch {
        match m {
            Msg::Dir(d) => apply_dir(&tx, d)?,
            Msg::Notes(source, run, notes) => {
                let mut st = tx.prepare_cached(
                    "INSERT INTO scan_notes(run_id,source_id,dir_rel,name,kind,detail) VALUES(?1,?2,?3,?4,?5,?6)",
                )?;
                for n in notes {
                    st.execute(params![run, source, n.dir_rel, n.name, n.kind, n.detail])?;
                }
            }
            _ => {}
        }
    }
    tx.commit()
}

fn apply_dir(tx: &Transaction, d: &DirBatch) -> rusqlite::Result<()> {
    let now = now_s();
    let parent_id: Option<i64> = match &d.parent_rel {
        Some(p) => tx
            .prepare_cached("SELECT id FROM dirs WHERE source_id=?1 AND rel_path=?2")?
            .query_row(params![d.source_id, p], |r| r.get(0))
            .optional()?,
        None => None,
    };
    if d.status == DirStatus::Ok && !d.skipped {
        tx.prepare_cached(
            "INSERT INTO dirs(source_id,parent_id,rel_path,mtime_ns,entry_count,scanned_at,status,error,seen_run)
             VALUES(?1,?2,?3,?4,?5,?6,'ok',NULL,?7)
             ON CONFLICT(source_id,rel_path) DO UPDATE SET parent_id=?2, mtime_ns=?4, entry_count=?5,
               scanned_at=?6, status='ok', error=NULL, seen_run=?7",
        )?
        .execute(params![d.source_id, parent_id, d.rel_path, d.mtime_ns, d.entry_count, now, d.run_id])?;
    } else {
        // a directory that did not answer, or was skipped: what is known of it stays
        tx.prepare_cached(
            "INSERT INTO dirs(source_id,parent_id,rel_path,mtime_ns,entry_count,status,error,seen_run)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8)
             ON CONFLICT(source_id,rel_path) DO UPDATE SET parent_id=COALESCE(?2,parent_id),
               status=?6, error=?7, seen_run=?8",
        )?
        .execute(params![
            d.source_id,
            parent_id,
            d.rel_path,
            d.mtime_ns,
            d.entry_count,
            if d.skipped { "ok" } else { d.status.as_str() },
            d.error,
            d.run_id
        ])?;
    }
    let dir_id: i64 = tx
        .prepare_cached("SELECT id FROM dirs WHERE source_id=?1 AND rel_path=?2")?
        .query_row(params![d.source_id, d.rel_path], |r| r.get(0))?;
    if d.status != DirStatus::Ok || d.skipped {
        if d.skipped {
            let n: i64 = tx
                .prepare_cached("SELECT COUNT(*) FROM photos WHERE dir_id=?1 AND status='present'")?
                .query_row([dir_id], |r| r.get(0))?;
            d.counters.found.fetch_add(n as u64, Ordering::Relaxed);
            d.counters.unchanged.fetch_add(n as u64, Ordering::Relaxed);
        }
        return Ok(());
    }
    if d.redo {
        tx.prepare_cached(
            "DELETE FROM scan_gone WHERE run_id=?1 AND photo_id IN (SELECT id FROM photos WHERE dir_id=?2)",
        )?
        .execute(params![d.run_id, dir_id])?;
    }
    let mut existing: HashMap<String, Existing> = HashMap::new();
    {
        let mut st = tx.prepare_cached(
            "SELECT id,name,size,mtime_ns,status,xmp_kind,xmp_size,xmp_mtime_ns FROM photos WHERE dir_id=?1",
        )?;
        let rows = st.query_map([dir_id], |r| {
            Ok((
                r.get::<_, String>(1)?,
                Existing {
                    id: r.get(0)?,
                    size: r.get(2)?,
                    mtime_ns: r.get(3)?,
                    status: r.get(4)?,
                    xmp_kind: r.get(5)?,
                    xmp_size: r.get(6)?,
                    xmp_mtime_ns: r.get(7)?,
                },
            ))
        })?;
        for row in rows {
            let (name, e) = row?;
            existing.insert(name, e);
        }
    }
    let mut insert = tx.prepare_cached(
        "INSERT INTO photos(source_id,dir_id,name,rel_path,ext,size,mtime_ns,status,discovered_at,seen_at,first_run,changed_run,
                            xmp_kind,xmp_size,xmp_mtime_ns)
         VALUES(?1,?2,?3,?4,?5,?6,?7,'present',?8,?8,?9,?9,?10,?11,?12)",
    )?;
    let mut update = tx.prepare_cached(
        "UPDATE photos SET size=?2, mtime_ns=?3, status='present', seen_at=?4, changed_run=?5,
                xmp_kind=?6, xmp_size=?7, xmp_mtime_ns=?8,
                meta_state=CASE WHEN size<>?2 OR mtime_ns<>?3 THEN 0 ELSE meta_state END
         WHERE id=?1",
    )?;
    let mut gone = tx.prepare_cached("INSERT OR IGNORE INTO scan_gone(run_id,photo_id) VALUES(?1,?2)")?;
    let (mut new, mut changed, mut same) = (0u64, 0u64, 0u64);
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::with_capacity(d.files.len());
    for f in &d.files {
        seen.insert(f.name.as_str());
        let (xk, xs, xm) = f.xmp.map_or((0u8, None, None), |(k, s, m)| (k, Some(s as i64), Some(m)));
        match existing.get(&f.name) {
            None => {
                let rel = if d.rel_path.is_empty() { f.name.clone() } else { format!("{}/{}", d.rel_path, f.name) };
                insert.execute(params![d.source_id, dir_id, f.name, rel, f.ext, f.size as i64, f.mtime_ns, now, d.run_id, xk, xs, xm])?;
                new += 1;
            }
            Some(e) => {
                let differs = e.size != f.size as i64
                    || e.mtime_ns != f.mtime_ns
                    || e.status != "present"
                    || e.xmp_kind != xk as i64
                    || e.xmp_size != xs
                    || e.xmp_mtime_ns != xm;
                if differs {
                    update.execute(params![e.id, f.size as i64, f.mtime_ns, now, d.run_id, xk, xs, xm])?;
                    changed += 1;
                } else {
                    same += 1;
                }
            }
        }
    }
    let mut gone_n = 0u64;
    for (name, e) in &existing {
        if e.status == "present" && !seen.contains(name.as_str()) {
            gone.execute(params![d.run_id, e.id])?;
            gone_n += 1;
        }
    }
    d.counters.found.fetch_add(d.files.len() as u64, Ordering::Relaxed);
    d.counters.new.fetch_add(new, Ordering::Relaxed);
    d.counters.changed.fetch_add(changed, Ordering::Relaxed);
    d.counters.unchanged.fetch_add(same, Ordering::Relaxed);
    d.counters.gone.fetch_add(gone_n, Ordering::Relaxed);
    Ok(())
}

fn finish_run(conn: &mut Connection, f: &Finish) -> Result<RunTotals, String> {
    let e = |e: rusqlite::Error| e.to_string();
    let now = now_s();
    let tx = conn.transaction().map_err(e)?;
    let mut missing = 0u64;
    if f.sweep && f.state == "complete" {
        // photos a listed directory no longer has
        missing += tx
            .execute(
                "UPDATE photos SET status='missing', changed_run=?1, seen_at=?2
                 WHERE status='present' AND id IN (SELECT photo_id FROM scan_gone WHERE run_id=?1)",
                params![f.run_id, now],
            )
            .map_err(e)? as u64;
        // directories not seen this run are gone, and so are their photos
        tx.execute(
            "UPDATE dirs SET status='gone' WHERE source_id=?1 AND (seen_run IS NULL OR seen_run<>?2) AND status<>'gone'",
            params![f.source_id, f.run_id],
        )
        .map_err(e)?;
        missing += tx
            .execute(
                "UPDATE photos SET status='missing', changed_run=?2, seen_at=?3
                 WHERE source_id=?1 AND status='present' AND dir_id IN (SELECT id FROM dirs WHERE source_id=?1 AND status='gone')",
                params![f.source_id, f.run_id, now],
            )
            .map_err(e)? as u64;
    }
    tx.execute("DELETE FROM scan_gone WHERE run_id=?1", [f.run_id]).map_err(e)?;
    let count = |sql: &str, p: &[&dyn rusqlite::ToSql]| -> Result<u64, String> {
        tx.query_row(sql, p, |r| r.get::<_, i64>(0)).map(|n| n as u64).map_err(|x| x.to_string())
    };
    let new = count("SELECT COUNT(*) FROM photos WHERE source_id=?1 AND first_run=?2", &[&f.source_id, &f.run_id])?;
    let changed = count(
        "SELECT COUNT(*) FROM photos WHERE source_id=?1 AND changed_run=?2 AND first_run<>?2 AND status='present'",
        &[&f.source_id, &f.run_id],
    )?;
    let found = count("SELECT COUNT(*) FROM photos WHERE source_id=?1 AND status='present'", &[&f.source_id])?;
    let dirs = count("SELECT COUNT(*) FROM dirs WHERE source_id=?1 AND status<>'gone' AND seen_run=?2", &[&f.source_id, &f.run_id])?;
    let unreadable = count("SELECT COUNT(*) FROM scan_notes WHERE source_id=?1 AND run_id=?2 AND kind IN ('non_utf8','denied')", &[&f.source_id, &f.run_id])?;
    let incomplete_dirs: Vec<(String, String, Option<String>)> = {
        let mut st = tx
            .prepare("SELECT rel_path,status,error FROM dirs WHERE source_id=?1 AND seen_run=?2 AND status NOT IN ('ok','gone') ORDER BY rel_path")
            .map_err(e)?;
        let rows = st
            .query_map(params![f.source_id, f.run_id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .map_err(e)?;
        rows.collect::<Result<_, _>>().map_err(e)?
    };
    let unchanged = found.saturating_sub(new + changed);
    let started: i64 = tx
        .query_row("SELECT started_at FROM scan_runs WHERE id=?1", [f.run_id], |r| r.get(0))
        .map_err(e)?;
    let incomplete_json = serde_json::to_string(&incomplete_dirs.iter().map(|d| &d.0).collect::<Vec<_>>()).ok();
    tx.execute(
        "UPDATE scan_runs SET state=?2, finished_at=?3, elapsed_ms=(?3-started_at)*1000, found=?4, dirs=?5, new=?6,
                changed=?7, unchanged=?8, missing=?9, unreadable=?10, incomplete_json=?11, error=?12 WHERE id=?1",
        params![f.run_id, f.state, now, found as i64, dirs as i64, new as i64, changed as i64, unchanged as i64, missing as i64, unreadable as i64, incomplete_json, f.error],
    )
    .map_err(e)?;
    let _ = started;
    if f.state == "complete" {
        // only the newest notes are worth showing
        tx.execute("DELETE FROM scan_notes WHERE source_id=?1 AND run_id<>?2", params![f.source_id, f.run_id]).map_err(e)?;
    }
    let source_state = match f.state {
        "complete" => "online",
        "incomplete" => "attention",
        "offline" => "offline",
        "interrupted" | "cancelled" => "paused",
        _ => "attention",
    };
    tx.execute(
        "UPDATE sources SET state=?2, error=?3, last_scan_end=?4,
           last_ok=CASE WHEN ?5 THEN ?4 ELSE last_ok END WHERE id=?1",
        params![f.source_id, source_state, f.error, now, f.state == "complete"],
    )
    .map_err(e)?;
    if let Some(job) = f.job_id {
        let state = match f.state {
            "complete" | "incomplete" => "done",
            "cancelled" => "cancelled",
            // the source went away: what was done is kept, and a later scan continues it
            "interrupted" | "offline" => "interrupted",
            _ => "failed",
        };
        // an interrupted job keeps its checkpoint; the others no longer need it
        tx.execute(
            "UPDATE jobs SET state=?2, error=?3, updated_at=?4,
               checkpoint=CASE WHEN ?2='interrupted' THEN checkpoint ELSE NULL END,
               finished_at=CASE WHEN ?2='interrupted' THEN NULL ELSE ?4 END WHERE id=?1",
            params![job, state, f.error, now],
        )
        .map_err(e)?;
    }
    tx.commit().map_err(e)?;
    Ok(RunTotals { found, dirs, new, changed, unchanged, missing, unreadable, incomplete_dirs })
}

// ---- jobs and runs (called through `exec`) --------------------------------

pub fn create_job(c: &Connection, kind: &str, source_id: i64, payload: &str) -> rusqlite::Result<i64> {
    let now = now_s();
    c.execute(
        "INSERT INTO jobs(kind,source_id,state,payload,created_at,updated_at) VALUES(?1,?2,'queued',?3,?4,?4)",
        params![kind, source_id, payload, now],
    )?;
    Ok(c.last_insert_rowid())
}

pub fn start_run(c: &Connection, source_id: i64, job_id: i64, mode: &str) -> rusqlite::Result<i64> {
    let now = now_s();
    c.execute(
        "INSERT INTO scan_runs(source_id,job_id,mode,state,started_at) VALUES(?1,?2,?3,'running',?4)",
        params![source_id, job_id, mode, now],
    )?;
    let run = c.last_insert_rowid();
    c.execute("UPDATE jobs SET state='running', run_id=?2, started_at=COALESCE(started_at,?3), updated_at=?3 WHERE id=?1", params![job_id, run, now])?;
    c.execute("UPDATE sources SET state='indexing', error=NULL, last_scan_start=?2 WHERE id=?1", params![source_id, now])?;
    Ok(run)
}

/// Re-opens an interrupted run for a resume: same run id, so the work already
/// committed counts.
pub fn resume_run(c: &Connection, job_id: i64, run_id: i64, source_id: i64) -> rusqlite::Result<()> {
    let now = now_s();
    c.execute("UPDATE scan_runs SET state='running' WHERE id=?1", [run_id])?;
    c.execute("UPDATE jobs SET state='running', updated_at=?2 WHERE id=?1", params![job_id, now])?;
    c.execute("UPDATE sources SET state='indexing', error=NULL WHERE id=?1", [source_id])?;
    Ok(())
}

pub fn save_checkpoint(c: &Connection, job_id: i64, checkpoint: &str, progress: &str, state: &str) -> rusqlite::Result<()> {
    c.execute(
        "UPDATE jobs SET checkpoint=?2, progress=?3, state=?4, updated_at=?5 WHERE id=?1",
        params![job_id, checkpoint, progress, state, now_s()],
    )?;
    Ok(())
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct JobRow {
    pub id: i64,
    pub source_id: i64,
    pub state: String,
    pub payload: Option<String>,
    pub checkpoint: Option<String>,
    pub run_id: Option<i64>,
}

pub fn interrupted_job(c: &Connection, source_id: i64) -> rusqlite::Result<Option<JobRow>> {
    c.query_row(
        "SELECT id,source_id,state,payload,checkpoint,run_id FROM jobs
         WHERE source_id=?1 AND kind='scan' AND state='interrupted' AND run_id IS NOT NULL ORDER BY id DESC LIMIT 1",
        [source_id],
        |r| {
            Ok(JobRow {
                id: r.get(0)?,
                source_id: r.get(1)?,
                state: r.get(2)?,
                payload: r.get(3)?,
                checkpoint: r.get(4)?,
                run_id: r.get(5)?,
            })
        },
    )
    .optional()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    fn count(c: &Connection, table: &str) -> i64 {
        c.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0)).unwrap()
    }

    #[test]
    fn creates_the_schema_with_wal_and_foreign_keys() {
        let dir = temp();
        let db = CatalogDb::open(&dir.path().join(FILE)).unwrap();
        let c = db.read().unwrap();
        assert_eq!(user_version(&c).unwrap(), SCHEMA_VERSION);
        let mode: String = c.query_row("PRAGMA journal_mode", [], |r| r.get(0)).unwrap();
        assert_eq!(mode, "wal");
        for t in ["meta", "sources", "dirs", "photos", "jobs", "scan_runs", "scan_gone", "scan_notes"] {
            assert_eq!(count(&c, t), if t == "meta" { 1 } else { 0 }, "{t}");
        }
        let fk = db.exec(|c| Ok(c.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0)).unwrap())).unwrap();
        assert_eq!(fk, 1);
        let sync = db.exec(|c| Ok(c.query_row("PRAGMA synchronous", [], |r| r.get::<_, i64>(0)).unwrap())).unwrap();
        assert_eq!(sync, 1, "NORMAL");
    }

    // a test ladder: the real v1, then two invented steps that change data
    fn v2(tx: &Transaction) -> rusqlite::Result<()> {
        tx.execute_batch("ALTER TABLE sources ADD COLUMN note TEXT; UPDATE sources SET note='v2';")
    }
    fn v3(tx: &Transaction) -> rusqlite::Result<()> {
        tx.execute_batch("CREATE TABLE extra(a INTEGER); INSERT INTO extra VALUES(3); UPDATE sources SET note=note||'+v3';")
    }
    fn v3_broken(tx: &Transaction) -> rusqlite::Result<()> {
        tx.execute_batch("CREATE TABLE half(a INTEGER); INSERT INTO nonexistent VALUES(1);")
    }

    #[test]
    fn migrates_from_empty_and_from_each_previous_version() {
        let ladder: &[Step] = &[v1, v2, v3];
        // empty -> head
        let mut c = Connection::open_in_memory().unwrap();
        let r = migrate(&mut c, ladder, None).unwrap();
        assert_eq!((r.from, r.to), (0, 3));
        assert_eq!(user_version(&c).unwrap(), 3);
        // from each older version, with a source row that must survive
        for start in 1..3usize {
            let dir = temp();
            let path = dir.path().join(FILE);
            let mut c = Connection::open(&path).unwrap();
            migrate(&mut c, &ladder[..start], None).unwrap();
            c.execute("INSERT INTO sources(name,volume_key,created_at) VALUES('keep','vol',1)", []).unwrap();
            c.execute("INSERT INTO meta(key,value) VALUES('probe','1')", []).unwrap();
            let r = migrate(&mut c, ladder, Some(&path)).unwrap();
            assert_eq!((r.from, r.to), (start as i64, 3));
            let name: String = c.query_row("SELECT name FROM sources", [], |r| r.get(0)).unwrap();
            assert_eq!(name, "keep");
            let note: Option<String> = c.query_row("SELECT note FROM sources", [], |r| r.get(0)).unwrap();
            // v2 ran on the existing row only when we started below it
            assert_eq!(note.is_some(), start == 1);
            // a copy of the old database was taken first
            assert!(dir.path().join(format!("{FILE}.pre-v{start}.bak")).exists());
            // and the head again is a no-op
            let again = migrate(&mut c, ladder, Some(&path)).unwrap();
            assert_eq!((again.from, again.to), (3, 3));
        }
    }

    #[test]
    fn a_failing_step_leaves_the_previous_version_whole() {
        let ladder: &[Step] = &[v1, v2, v3_broken];
        let mut c = Connection::open_in_memory().unwrap();
        assert!(migrate(&mut c, ladder, None).is_err());
        assert_eq!(user_version(&c).unwrap(), 2);
        let half: i64 = c.query_row("SELECT COUNT(*) FROM sqlite_master WHERE name='half'", [], |r| r.get(0)).unwrap();
        assert_eq!(half, 0, "the failed step rolled back");
    }

    #[test]
    fn refuses_a_newer_schema_and_keeps_it_unchanged() {
        let dir = temp();
        let path = dir.path().join(FILE);
        {
            let c = Connection::open(&path).unwrap();
            c.pragma_update(None, "user_version", SCHEMA_VERSION + 4).unwrap();
        }
        assert!(matches!(CatalogDb::open(&path), Err(CatalogError::Newer { .. })));
        assert!(matches!(CatalogDb::reader(&path), Err(CatalogError::Newer { .. })));
        let c = Connection::open(&path).unwrap();
        assert_eq!(user_version(&c).unwrap(), SCHEMA_VERSION + 4);
    }

    #[test]
    fn a_second_writer_is_refused_but_readers_work() {
        let dir = temp();
        let path = dir.path().join(FILE);
        let db = CatalogDb::open(&path).unwrap();
        assert_eq!(CatalogDb::open(&path).err(), Some(CatalogError::Locked));
        assert!(CatalogDb::reader(&path).is_ok());
        db.close();
        drop(db);
        assert!(CatalogDb::open(&path).is_ok());
    }

    #[test]
    fn a_garbage_file_is_reported_as_corrupt() {
        let dir = temp();
        let path = dir.path().join(FILE);
        std::fs::write(&path, vec![0x41u8; 5000]).unwrap();
        assert!(matches!(CatalogDb::open(&path), Err(CatalogError::Corrupt(_))));
    }

    fn add_source(db: &CatalogDb) -> i64 {
        db.exec(|c| {
            c.execute("INSERT INTO sources(name,volume_key,created_at) VALUES('s','v',1)", []).map_err(|e| e.to_string())?;
            Ok(c.last_insert_rowid())
        })
        .unwrap()
    }

    fn rec(name: &str, size: u64) -> FileRec {
        FileRec { name: name.into(), ext: "jpg".into(), size, mtime_ns: 1_000, xmp: None }
    }

    fn batch(source: i64, run: i64, rel: &str, parent: Option<&str>, files: Vec<FileRec>, counters: &Arc<Counters>) -> DirBatch {
        DirBatch {
            source_id: source,
            run_id: run,
            rel_path: rel.into(),
            parent_rel: parent.map(Into::into),
            mtime_ns: Some(5),
            entry_count: files.len() as u32,
            status: DirStatus::Ok,
            error: None,
            files,
            skipped: false,
            redo: false,
            counters: counters.clone(),
        }
    }

    #[test]
    fn writer_classifies_new_changed_unchanged_and_sweeps_only_when_told() {
        let dir = temp();
        let db = CatalogDb::open(&dir.path().join(FILE)).unwrap();
        let s = add_source(&db);
        let (job, run) = db.exec(move |c| {
            let j = create_job(c, "scan", s, "{}").map_err(|e| e.to_string())?;
            Ok((j, start_run(c, s, j, "full").map_err(|e| e.to_string())?))
        }).unwrap();
        let k = Arc::new(Counters::default());
        db.send_dir(batch(s, run, "", None, vec![rec("a.jpg", 1), rec("b.jpg", 2)], &k)).unwrap();
        db.send_dir(batch(s, run, "sub", Some(""), vec![rec("c.jpg", 3)], &k)).unwrap();
        let t = db.finish_run(Finish { source_id: s, run_id: run, job_id: Some(job), state: "complete", sweep: true, error: None }).unwrap();
        assert_eq!((t.found, t.new, t.changed, t.missing, t.dirs), (3, 3, 0, 0, 2));
        // second run: b changed, c gone from its directory, d new
        let run2 = db.exec(move |c| { let j = create_job(c, "scan", s, "{}").unwrap(); Ok(start_run(c, s, j, "full").unwrap()) }).unwrap();
        let k2 = Arc::new(Counters::default());
        db.send_dir(batch(s, run2, "", None, vec![rec("a.jpg", 1), rec("b.jpg", 99), rec("d.jpg", 4)], &k2)).unwrap();
        db.send_dir(batch(s, run2, "sub", Some(""), vec![], &k2)).unwrap();
        db.sync().unwrap();
        // an incomplete run sweeps nothing
        let t = db.finish_run(Finish { source_id: s, run_id: run2, job_id: None, state: "incomplete", sweep: false, error: None }).unwrap();
        assert_eq!((t.new, t.changed, t.missing), (1, 1, 0));
        let c = db.read().unwrap();
        let present: i64 = c.query_row("SELECT COUNT(*) FROM photos WHERE status='present'", [], |r| r.get(0)).unwrap();
        assert_eq!(present, 4, "c.jpg is still present: no sweep");
        // run 3 sees the same and completes: now c goes missing
        let run3 = db.exec(move |c| { let j = create_job(c, "scan", s, "{}").unwrap(); Ok(start_run(c, s, j, "full").unwrap()) }).unwrap();
        let k3 = Arc::new(Counters::default());
        db.send_dir(batch(s, run3, "", None, vec![rec("a.jpg", 1), rec("b.jpg", 99), rec("d.jpg", 4)], &k3)).unwrap();
        db.send_dir(batch(s, run3, "sub", Some(""), vec![], &k3)).unwrap();
        let t = db.finish_run(Finish { source_id: s, run_id: run3, job_id: None, state: "complete", sweep: true, error: None }).unwrap();
        assert_eq!((t.missing, t.new, t.changed, t.found), (1, 0, 0, 3));
    }

    #[test]
    fn a_big_burst_is_committed_in_batches() {
        let dir = temp();
        let db = CatalogDb::open(&dir.path().join(FILE)).unwrap();
        let s = add_source(&db);
        let run = db.exec(move |c| { let j = create_job(c, "scan", s, "{}").unwrap(); Ok(start_run(c, s, j, "full").unwrap()) }).unwrap();
        let k = Arc::new(Counters::default());
        for d in 0..50 {
            let files = (0..100).map(|i| rec(&format!("{i}.jpg"), i)).collect();
            db.send_dir(batch(s, run, &format!("d{d}"), None, files, &k)).unwrap();
        }
        db.sync().unwrap();
        assert!(db.failure().is_none());
        assert_eq!(k.new.load(Ordering::Relaxed), 5000);
        assert_eq!(count(&db.read().unwrap(), "photos"), 5000);
    }

    #[test]
    fn open_reconciles_jobs_left_running() {
        let dir = temp();
        let path = dir.path().join(FILE);
        {
            let db = CatalogDb::open(&path).unwrap();
            let s = add_source(&db);
            db.exec(move |c| {
                let j = create_job(c, "scan", s, "{}").unwrap();
                let run = start_run(c, s, j, "full").unwrap();
                save_checkpoint(c, j, "{\"frontier\":[\"\"]}", "{}", "running").unwrap();
                let _ = run;
                Ok(())
            }).unwrap();
        }
        let db = CatalogDb::open(&path).unwrap();
        let c = db.read().unwrap();
        let (js, rs, ss): (String, String, String) = c
            .query_row("SELECT j.state, r.state, s.state FROM jobs j, scan_runs r, sources s", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap();
        assert_eq!((js.as_str(), rs.as_str(), ss.as_str()), ("interrupted", "interrupted", "interrupted"));
        let job = interrupted_job(&c, 1).unwrap().unwrap();
        assert!(job.checkpoint.unwrap().contains("frontier"));
    }
}
