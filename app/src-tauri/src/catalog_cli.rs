// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! `immage catalog ...`: the Catalog from a terminal. Run from `main()` before
//! Tauri starts, so no window or webview is ever created and it works over SSH.
//! Human output goes to stdout when it is the result, progress and logs to
//! stderr, so `--json` stdout is always parseable. Exit codes: 0 success, 1
//! failure, 2 usage, 3 catalog problem (locked, newer schema, corrupt), 4 every
//! requested source unreachable, 5 finished but partial (incomplete folders,
//! unreadable files, problems found), 130 interrupted (checkpoint saved).

use crate::catalog_db::{CatalogDb, CatalogError, FILE, SCHEMA_VERSION};
use crate::catalog_meta_job::{coverage, run_metadata, MetaConfig, MetaProgress, MetaReport};
use crate::catalog_scan::{run_scan, Progress, RealFs, ScanConfig, ScanControl, ScanOptions, ScanReport};
use crate::catalog_sources::{
    self as sources, add_source, availability, find_source, host_hash, list_sources, reconcile, system_uuid_of, AddError,
    AddOptions, Availability, MountTable, Source,
};
use serde_json::{json, Value};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

pub const EXIT_OK: i32 = 0;
pub const EXIT_FAIL: i32 = 1;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_CATALOG: i32 = 3;
pub const EXIT_OFFLINE: i32 = 4;
pub const EXIT_PARTIAL: i32 = 5;
pub const EXIT_INTERRUPTED: i32 = 130;

/// How often a non-terminal progress line is written.
const PLAIN_PROGRESS_EVERY: Duration = Duration::from_secs(10);

/// Called first thing in `main()`: runs the CLI when the first argument is
/// `catalog` and returns its exit code.
pub fn maybe_run() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("catalog") {
        return None;
    }
    Some(run(args.collect(), &mut std::io::stdout(), &mut std::io::stderr()))
}

// ---- where the catalog lives -------------------------------------------------

/// The runtime directory the app keeps its files in: a dev build's checkout
/// `build/app-runtime`, otherwise the app data directory of the bundle
/// identifier (what Tauri's `app_data_dir` is).
pub fn runtime_dir() -> PathBuf {
    if cfg!(debug_assertions) {
        return Path::new(env!("CARGO_MANIFEST_DIR")).join("../../build/app-runtime");
    }
    let home = || std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    #[cfg(target_os = "macos")]
    let base = home().join("Library/Application Support");
    #[cfg(windows)]
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(home);
    #[cfg(all(unix, not(target_os = "macos")))]
    let base = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| home().join(".local/share"));
    base.join("app.immage")
}

pub fn default_db() -> PathBuf {
    runtime_dir().join(FILE)
}

// ---- arguments ---------------------------------------------------------------

#[derive(Default, Debug)]
struct Args {
    db: Option<PathBuf>,
    json: bool,
    quiet: bool,
    verbose: bool,
    yes: bool,
    progress: bool,
    concurrency: Option<usize>,
    flags: Vec<String>,
    values: Vec<(String, String)>,
    positional: Vec<String>,
}

const WITH_VALUE: &[&str] = &["--db", "--catalog", "--concurrency", "--name", "--policy", "--source", "--size", "--first-read"];
const BOOLEAN: &[&str] = &[
    "--json", "--quiet", "--no-color", "--verbose", "--yes", "--progress", "--all", "--light", "--no-thumbnails",
    "--metadata-only", "--no-subfolders", "--index", "--skip-test", "--allow-offline", "--keep-annotations", "--watch",
    "--deep", "--fix", "--help", "--read-only", "--treat-as-network", "--no-treat-as-network", "--dir-skip", "--no-dir-skip",
    "--regenerate", "--missing-only", "--metadata", "--skip-failed", "--no-sidecars", "--no-relink", "--no-adaptive",
];

fn parse_args(raw: &[String]) -> Result<Args, String> {
    let mut a = Args::default();
    let mut it = raw.iter();
    while let Some(arg) = it.next() {
        if let Some(flag) = arg.strip_prefix("--").map(|_| arg.as_str()) {
            let (flag, inline) = match flag.split_once('=') {
                Some((f, v)) if WITH_VALUE.contains(&f) => (f, Some(v.to_string())),
                _ => (flag, None),
            };
            if WITH_VALUE.contains(&flag) {
                let v = match inline {
                    Some(v) => v,
                    None => it.next().cloned().ok_or_else(|| format!("{flag} needs a value"))?,
                };
                match flag {
                    "--db" => a.db = Some(PathBuf::from(v)),
                    "--catalog" => a.db = Some(PathBuf::from(v).join(FILE)),
                    "--concurrency" => {
                        let n: usize = v.parse().map_err(|_| "--concurrency needs a number".to_string())?;
                        if !(1..=64).contains(&n) {
                            return Err("--concurrency must be between 1 and 64".into());
                        }
                        a.concurrency = Some(n);
                    }
                    other => a.values.push((other.to_string(), v)),
                }
            } else if BOOLEAN.contains(&flag) {
                match flag {
                    "--json" => a.json = true,
                    "--quiet" => a.quiet = true,
                    "--verbose" => a.verbose = true,
                    "--yes" => a.yes = true,
                    "--progress" => a.progress = true,
                    "--no-color" => {}
                    other => a.flags.push(other.to_string()),
                }
            } else {
                return Err(format!("Unknown option {flag}. Try: immage catalog --help"));
            }
        } else if arg == "-h" {
            a.flags.push("--help".into());
        } else {
            a.positional.push(arg.clone());
        }
    }
    Ok(a)
}

impl Args {
    fn flag(&self, f: &str) -> bool {
        self.flags.iter().any(|x| x == f)
    }
    fn value(&self, k: &str) -> Option<&str> {
        self.values.iter().rev().find(|(x, _)| x == k).map(|(_, v)| v.as_str())
    }
    fn db_path(&self) -> PathBuf {
        self.db.clone().unwrap_or_else(default_db)
    }
}

// ---- formatting ---------------------------------------------------------------

pub fn fmt_n(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

pub fn fmt_dur(secs: f64) -> String {
    let s = secs.round().max(0.0) as u64;
    match s {
        0..=59 => format!("{s} s"),
        60..=3599 => format!("{} min {} s", s / 60, s % 60),
        _ => format!("{} h {} min", s / 3600, (s % 3600) / 60),
    }
}

fn fmt_short(secs: u64) -> String {
    if secs < 90 {
        format!("{secs} s")
    } else if secs < 5400 {
        format!("{} min", (secs + 30) / 60)
    } else {
        format!("{:.1} h", secs as f64 / 3600.0)
    }
}

pub fn fmt_ago(ts: Option<i64>) -> String {
    let Some(t) = ts else { return "never".into() };
    let d = (crate::catalog_db::now_s() - t).max(0);
    match d {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", d / 60),
        3600..=86_399 => format!("{} h ago", d / 3600),
        _ => format!("{} d ago", d / 86_400),
    }
}

fn fmt_mb(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else if bytes < 1_000_000 {
        format!("{} KB", bytes.div_ceil(1000))
    } else {
        format!("{:.0} MB", bytes as f64 / 1e6)
    }
}

fn storage_text(s: &Source) -> String {
    match s.kind.as_str() {
        "nas" => {
            let proto = match s.fs_type.as_str() {
                "cifs" | "smb3" | "smbfs" => "SMB",
                t if t.starts_with("nfs") => "NFS",
                "fuse.sshfs" | "sshfs" => "SFTP",
                "autofs" => "Network share, waiting for the automount",
                _ => "network",
            };
            let host = s.host.clone().unwrap_or_default();
            let share = s.share.clone().unwrap_or_default();
            let parts: Vec<&str> = [host.as_str(), share.as_str()].into_iter().filter(|p| !p.is_empty()).collect();
            if proto.starts_with("Network") || parts.is_empty() {
                proto.to_string()
            } else {
                format!("Network share, {proto} ({})", parts.join(", "))
            }
        }
        "drive" => format!("Drive ({})", s.fs_type),
        _ => "Local disk".to_string(),
    }
}

fn volume_text(s: &Source) -> String {
    match (&s.host, &s.share) {
        (Some(h), Some(sh)) => {
            let short = h.split('.').next().unwrap_or(h);
            format!("{short}/{}", sh.trim_matches('/').rsplit('/').next().unwrap_or(sh))
        }
        _ => s.volume_key.clone(),
    }
}

// ---- output ---------------------------------------------------------------------

struct Out<'a> {
    out: &'a mut dyn Write,
    err: &'a mut dyn Write,
    json: bool,
    quiet: bool,
    verbose: bool,
    progress_events: bool,
    tty: bool,
}

impl Out<'_> {
    fn say(&mut self, line: impl AsRef<str>) {
        let _ = writeln!(self.out, "{}", line.as_ref());
    }
    fn warn(&mut self, line: impl AsRef<str>) {
        let _ = writeln!(self.err, "{}", line.as_ref());
    }
    fn data(&mut self, v: &Value) {
        let _ = writeln!(self.out, "{v}");
    }
}

struct ProgressPrinter {
    name: String,
    last: Option<Instant>,
    printed_line: bool,
}

impl ProgressPrinter {
    fn show(&mut self, o: &mut Out, p: &Progress) {
        if o.quiet && !o.progress_events {
            return;
        }
        if o.json {
            if o.progress_events && p.phase != "done" {
                let ev = json!({"event":"progress","source":p.source_id,"phase":"finding","done":p.found,"total":null,
                    "rate":p.rate_per_s.round(),"eta_s":p.eta_s.map(|(a,b)|json!([a,b])),"dirs":p.dirs_done,"errors":p.errors});
                let _ = writeln!(o.err, "{ev}");
            }
            return;
        }
        let line = progress_line(&self.name, p);
        if o.tty {
            let _ = write!(o.err, "\r\x1b[2K{line}");
            let _ = o.err.flush();
            self.printed_line = true;
            if p.phase == "done" {
                self.clear(o);
            }
        } else if p.phase != "done" && self.last.is_none_or(|t| t.elapsed() >= PLAIN_PROGRESS_EVERY) {
            self.last = Some(Instant::now());
            let _ = writeln!(o.err, "{line}");
        }
    }
    fn clear(&mut self, o: &mut Out) {
        if self.printed_line && o.tty {
            let _ = write!(o.err, "\r\x1b[2K");
            let _ = o.err.flush();
            self.printed_line = false;
        }
    }
}

pub fn progress_line(name: &str, p: &Progress) -> String {
    let mut s = format!("{name:<20} finding photos      {} so far", fmt_n(p.found));
    if p.dirs_done > 0 {
        s += &format!(" in {} folders", fmt_n(p.dirs_done));
    }
    if p.rate_per_s >= 1.0 {
        s += &format!(", {}/s", fmt_n(p.rate_per_s as u64));
    }
    if p.paused {
        s += " (paused)";
    } else if let Some((lo, hi)) = p.eta_s {
        s += &format!(", about {} left ({}-{})", fmt_short((lo + hi) / 2), fmt_short(lo), fmt_short(hi));
    }
    if p.errors > 0 {
        s += &format!(", {} folders did not answer", p.errors);
    }
    s
}

// ---- Ctrl-C ----------------------------------------------------------------------

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

#[cfg(unix)]
extern "C" fn on_sigint(_: libc::c_int) {
    INTERRUPTED.store(true, Ordering::SeqCst);
}

fn install_sigint() {
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGINT, on_sigint as *const () as libc::sighandler_t);
        libc::signal(libc::SIGTERM, on_sigint as *const () as libc::sighandler_t);
    }
}

// ---- the commands ------------------------------------------------------------------

const HELP: &str = "\
Usage: immage catalog <command> [options]

Commands:
  list-sources                  List sources
  add-source <path>             Add a folder as a source (--name, --policy launch|every=6h|manual,
                                --no-subfolders, --index, --skip-test, --allow-offline)
  remove-source <id|name>       Forget a source and what the catalog knows of it (--yes)
  scan [<source>...|--all]      Find photos (first index or full check); --light skips unchanged folders;
                                --metadata then reads their metadata
  metadata [<source>...|--all]  Read the headers and sidecars of new and changed photos, then relink renames
  rescan <source>               Same as scan, for a source that is already indexed
  status [<source>] [--watch]   What the catalog holds and what is running
  verify [--deep] [--fix]       Check the catalog's integrity (read only; --fix repairs the catalog only)

Options:
  --db <file>        The catalog file (default: the app's runtime folder)
  --catalog <dir>    The folder holding catalog.sqlite
  --json             Machine output on stdout
  --progress         With --json, progress events as NDJSON on stderr
  --quiet  --verbose --yes --concurrency <n>

Exit codes: 0 ok, 1 failure, 2 usage, 3 catalog problem, 4 source unreachable,
5 finished but partial, 130 interrupted (safe to rerun).

Example: immage catalog add-source /mnt/photos/2023 --index
";

const HELP_ADD: &str = "\
Usage: immage catalog add-source <path> [--name N] [--policy launch|every=6h|manual]
                                 [--no-subfolders] [--index] [--skip-test] [--allow-offline]
Adds a folder as a source and prints how it was detected. Offline folders exit 4 unless --allow-offline.
Example: immage catalog add-source /mnt/synology/Photos_Backup/2023 --name Japan --index
";

const HELP_METADATA: &str = "\
Usage: immage catalog metadata [<source>...|--all] [--concurrency N] [--first-read KB] [--skip-failed]
                               [--no-sidecars] [--no-relink] [--no-adaptive]
Reads the first 64 KB of each photo the scan found new or changed (more only when the format needs it),
and the small XMP sidecar, and records camera, lens, exposure, time, size, GPS, rating, labels and keywords
in the catalog. Photos already read and unchanged are not opened. Read errors are recorded per photo
(failed, retried by the next run); files the reader does not understand are marked unparsed. Nothing is
written to the volume, and a read error never marks a source offline or a photo missing. Afterwards photos
that were renamed or moved inside the source are matched to their old entries (keeping tags and analyses).
Readers: 2 on a network share, 4 on a local drive. Ctrl-C keeps the progress; run it again to continue.
Example: immage catalog metadata 2 --json
";

const HELP_SCAN: &str = "\
Usage: immage catalog scan [<source>...|--all] [--light] [--concurrency N] [--metadata]
Lists the source's folders and records every photo's name, size and modified time. Nothing is read
from the files and nothing is changed on the volume. Photos that are gone are marked missing only
when the whole walk finished without a problem and the volume was online throughout.
Example: immage catalog scan 2 --json
";

pub fn run(raw: Vec<String>, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let args = match parse_args(&raw) {
        Ok(a) => a,
        Err(e) => {
            let _ = writeln!(err, "{e}");
            return EXIT_USAGE;
        }
    };
    let mut o = Out {
        out,
        err,
        json: args.json,
        quiet: args.quiet,
        verbose: args.verbose,
        progress_events: args.progress,
        tty: std::io::stderr().is_terminal() && !args.json,
    };
    let Some(cmd) = args.positional.first().cloned() else {
        o.say(HELP);
        return if args.flag("--help") { EXIT_OK } else { EXIT_USAGE };
    };
    if args.flag("--help") {
        o.say(match cmd.as_str() {
            "add-source" => HELP_ADD,
            "scan" | "rescan" => HELP_SCAN,
            "metadata" => HELP_METADATA,
            _ => HELP,
        });
        return EXIT_OK;
    }
    match cmd.as_str() {
        "list-sources" | "ls" => cmd_list(&args, &mut o),
        "add-source" => cmd_add(&args, &mut o),
        "remove-source" => cmd_remove(&args, &mut o),
        "scan" => cmd_scan(&args, &mut o, false),
        "rescan" => cmd_scan(&args, &mut o, true),
        "metadata" => cmd_metadata(&args, &mut o),
        "status" => cmd_status(&args, &mut o),
        "verify" => cmd_verify(&args, &mut o),
        "thumbnails" | "rebuild" | "backup" | "restore" => {
            o.warn(format!("immage catalog {cmd} is not available in this version yet."));
            EXIT_FAIL
        }
        other => {
            o.warn(format!("Unknown command {other}. Try: immage catalog --help"));
            EXIT_USAGE
        }
    }
}

fn catalog_error(o: &mut Out, e: &CatalogError) -> i32 {
    o.warn(e.to_string());
    match e {
        CatalogError::Locked | CatalogError::Newer { .. } | CatalogError::Corrupt(_) => EXIT_CATALOG,
        CatalogError::Other(_) => EXIT_FAIL,
    }
}

fn open_rw(args: &Args, o: &mut Out) -> Result<CatalogDb, i32> {
    CatalogDb::open(&args.db_path()).map_err(|e| catalog_error(o, &e))
}

fn availability_word(table: &MountTable, s: &Source) -> Availability {
    availability(table, s, &system_uuid_of)
}

/// Where the source is, and whether its folder is really there (the volume
/// being mounted is not enough). An idle automount is not touched.
fn checked_availability(table: &MountTable, s: &Source) -> Availability {
    match availability_word(table, s) {
        Availability::Online { abs_root, .. }
            if !(crate::volumes::answering(Path::new(&abs_root)) && Path::new(&abs_root).is_dir()) =>
        {
            Availability::Offline { reason: "folder not found".into() }
        }
        other => other,
    }
}

fn state_word(s: &Source, a: &Availability) -> String {
    match a {
        Availability::Offline { .. } => "offline".into(),
        Availability::Asleep { .. } => "asleep".into(),
        Availability::Online { .. } => s.state.replace('_', "-"),
    }
}

fn source_json(s: &Source, a: &Availability) -> Value {
    json!({
        "id": s.id, "name": s.name, "kind": s.kind,
        "volume": { "key": s.volume_key, "host_hash": s.host_hash, "share": s.share, "fs_type": s.fs_type, "mount_path": s.mount_path },
        "rel_root": s.rel_root, "enabled": s.enabled, "policy": s.scan_policy,
        "include_subfolders": s.include_subfolders, "treat_as_network": s.treat_as_network, "dir_skip": s.dir_skip,
        "state": state_word(s, a), "availability": a.word(),
        "photos": s.photos, "missing": s.missing, "last_scan_start": s.last_scan_start, "last_scan_end": s.last_scan_end, "last_ok": s.last_ok,
    })
}

fn cmd_list(args: &Args, o: &mut Out) -> i32 {
    let conn = match CatalogDb::reader(&args.db_path()) {
        Ok(c) => c,
        Err(e) => return catalog_error(o, &e),
    };
    let list = match list_sources(&conn) {
        Ok(l) => l,
        Err(e) => {
            o.warn(e);
            return EXIT_FAIL;
        }
    };
    let table = MountTable::current();
    let rows: Vec<(Source, Availability)> = list.into_iter().map(|s| { let a = checked_availability(&table, &s); (s, a) }).collect();
    if o.json {
        let v: Vec<Value> = rows.iter().map(|(s, a)| source_json(s, a)).collect();
        o.data(&Value::Array(v));
        return EXIT_OK;
    }
    if rows.is_empty() {
        o.say("No sources yet. Add one with: immage catalog add-source <path>");
        return EXIT_OK;
    }
    let w = rows.iter().map(|(s, _)| s.name.chars().count()).max().unwrap_or(4).clamp(4, 28);
    let vw = rows.iter().map(|(s, _)| volume_text(s).chars().count()).max().unwrap_or(6).clamp(6, 24);
    o.say(format!("{:<3} {:<w$} {:<vw$} {:<13} {:>9}  {:<13} {}", "ID", "NAME", "VOLUME", "STATE", "PHOTOS", "LAST CHECK", "POLICY"));
    for (s, a) in &rows {
        let name: String = s.name.chars().take(w).collect();
        o.say(format!(
            "{:<3} {:<w$} {:<vw$} {:<13} {:>9}  {:<13} {}",
            s.id,
            name,
            volume_text(s).chars().take(vw).collect::<String>(),
            state_word(s, a),
            fmt_n(s.photos as u64),
            fmt_ago(s.last_scan_end),
            s.scan_policy
        ));
    }
    EXIT_OK
}

fn parse_policy(v: &str) -> Option<String> {
    match v {
        "launch" | "on-launch" => Some("launch".into()),
        "manual" => Some("manual".into()),
        _ => {
            let h = v.strip_prefix("every=")?.strip_suffix('h')?;
            let n: u32 = h.parse().ok()?;
            (n > 0 && n <= 24 * 30).then(|| format!("every={n}h"))
        }
    }
}

/// A short listing-speed test: walks folders for a few seconds, read only.
fn speed_test(root: &Path, max: Duration) -> (u64, f64, f64) {
    use crate::catalog_scan::ScanFs;
    let fs = RealFs::new(root.to_path_buf());
    let started = Instant::now();
    let mut stack = vec![root.to_path_buf()];
    let mut entries = 0u64;
    while let Some(d) = stack.pop() {
        if started.elapsed() > max {
            break;
        }
        if let Ok(list) = fs.list(&d, Duration::from_secs(5)) {
            entries += list.len() as u64;
            for e in list {
                if e.kind == crate::catalog_scan::EntryKind::Dir && !e.name.to_string_lossy().starts_with('.') {
                    stack.push(d.join(&e.name));
                }
            }
        }
    }
    let secs = started.elapsed().as_secs_f64().max(0.001);
    (entries, entries as f64 / secs, secs)
}

fn verdict(per_s: f64) -> &'static str {
    match per_s {
        x if x >= 2000.0 => "fast",
        x if x >= 500.0 => "good",
        x if x >= 100.0 => "slow",
        _ => "very slow",
    }
}

fn cmd_add(args: &Args, o: &mut Out) -> i32 {
    let Some(path) = args.positional.get(1) else {
        o.warn("Give the folder to add: immage catalog add-source <path>");
        return EXIT_USAGE;
    };
    let policy = match args.value("--policy") {
        Some(p) => match parse_policy(p) {
            Some(p) => p,
            None => {
                o.warn("--policy must be launch, manual or every=<hours>h (for example every=6h)");
                return EXIT_USAGE;
            }
        },
        None => "launch".into(),
    };
    let opts = AddOptions {
        name: args.value("--name").map(str::to_string),
        policy,
        include_subfolders: !args.flag("--no-subfolders"),
        allow_offline: args.flag("--allow-offline"),
        treat_as_network: if args.flag("--treat-as-network") { Some(true) } else if args.flag("--no-treat-as-network") { Some(false) } else { None },
    };
    let db = match open_rw(args, o) {
        Ok(d) => d,
        Err(c) => return c,
    };
    let src = match add_source(&db, Path::new(path), &opts) {
        Ok(s) => s,
        Err(AddError::Offline(m)) => {
            o.warn(m);
            return EXIT_OFFLINE;
        }
        Err(AddError::Overlap(id)) => {
            o.warn(format!("That folder overlaps source {id}. Remove it first or pick another folder."));
            return EXIT_USAGE;
        }
        Err(AddError::Usage(m)) => {
            o.warn(m);
            return EXIT_USAGE;
        }
        Err(AddError::Db(m)) => {
            o.warn(m);
            return EXIT_FAIL;
        }
    };
    let table = MountTable::current();
    let a = availability_word(&table, &src);
    // mounted is not enough: the folder itself has to be there
    let reachable = matches!(&a, Availability::Online { abs_root, .. }
        if crate::volumes::answering(Path::new(abs_root)) && Path::new(abs_root).is_dir());
    let mut test: Option<(u64, f64, f64)> = None;
    if reachable && !args.flag("--skip-test") && !o.json {
        if let Availability::Online { abs_root, .. } = &a {
            test = Some(speed_test(Path::new(abs_root), Duration::from_secs(3)));
        }
    }
    if o.json {
        o.data(&json!({ "source": source_json(&src, &a), "storage": storage_text(&src),
            "added_offline": !reachable }));
    } else {
        o.say(format!("Folder   {path}"));
        o.say(format!("Storage  {}", storage_text(&src)));
        match test {
            Some((n, per_s, secs)) if n > 0 => o.say(format!(
                "Test     listing {} files/s: {} (read only, {} s); reading photos is not measured yet",
                fmt_n(per_s as u64), verdict(per_s), secs.round() as u64
            )),
            Some(_) => o.say("Test     the folder is empty"),
            None if !reachable => o.say("Test     skipped: not reachable"),
            None => o.say("Test     skipped"),
        }
        if reachable {
            o.say(format!(
                "Added source {} \"{}\". Policy: {}. Immage never changes your files.\nRun: immage catalog scan {}",
                src.id,
                src.name,
                match src.scan_policy.as_str() { "launch" => "on launch".to_string(), p => p.to_string() },
                src.id
            ));
        } else {
            o.say(format!("Added source {} \"{}\". Will index when it appears.", src.id, src.name));
        }
    }
    if args.flag("--index") && reachable {
        drop(db);
        let mut a2 = Args { db: args.db.clone(), json: args.json, quiet: args.quiet, yes: args.yes, ..Default::default() };
        a2.positional = vec!["scan".into(), src.id.to_string()];
        return cmd_scan(&a2, o, false);
    }
    EXIT_OK
}

fn confirm(args: &Args, o: &mut Out, question: &str) -> bool {
    if args.yes {
        return true;
    }
    if !std::io::stdin().is_terminal() {
        o.warn(format!("{question} Use --yes to confirm."));
        return false;
    }
    let _ = write!(o.err, "{question} [y/N] ");
    let _ = o.err.flush();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).is_ok() && matches!(line.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn cmd_remove(args: &Args, o: &mut Out) -> i32 {
    let Some(key) = args.positional.get(1) else {
        o.warn("Give the source: immage catalog remove-source <id|name>");
        return EXIT_USAGE;
    };
    let db = match open_rw(args, o) {
        Ok(d) => d,
        Err(c) => return c,
    };
    let conn = match db.read() {
        Ok(c) => c,
        Err(e) => return catalog_error(o, &e),
    };
    let src = match find_source(&conn, key) {
        Ok(Some(s)) => s,
        Ok(None) => {
            o.warn(format!("No source \"{key}\"."));
            return EXIT_USAGE;
        }
        Err(e) => {
            o.warn(e);
            return EXIT_FAIL;
        }
    };
    let prev = sources::remove_preview(&conn, src.id).unwrap_or(sources::RemovePreview { photos: 0, dirs: 0 });
    let q = format!(
        "Remove source {} \"{}\"? The catalog forgets {} photos in {} folders. Your files are not touched.",
        src.id, src.name, fmt_n(prev.photos as u64), fmt_n(prev.dirs as u64)
    );
    if !confirm(args, o, &q) {
        return EXIT_USAGE;
    }
    match sources::remove_source(&db, src.id) {
        Ok(()) => {
            if o.json {
                o.data(&json!({"removed": src.id, "photos": prev.photos, "dirs": prev.dirs}));
            } else {
                o.say(format!("Removed source {} \"{}\".", src.id, src.name));
            }
            EXIT_OK
        }
        Err(e) => {
            o.warn(e);
            EXIT_FAIL
        }
    }
}

fn report_json(src: &Source, r: &ScanReport) -> Value {
    json!({
        "source": src.id, "name": src.name, "state": r.state,
        "found": r.totals.found, "read": 0, "new": r.totals.new, "changed": r.totals.changed,
        "unchanged": r.totals.unchanged, "missing": r.totals.missing, "unreadable": r.totals.unreadable,
        "folders": r.totals.dirs,
        "incomplete_dirs": r.totals.incomplete_dirs.iter().map(|d| json!({"folder": d.0, "status": d.1, "error": d.2})).collect::<Vec<_>>(),
        "elapsed_s": r.elapsed.as_secs(), "resumed": r.resumed,
        "files_per_s": { "list": r.files_per_s.round(), "read": null },
        "error": r.error,
    })
}

/// Wakes an idle automount (a scan is an explicit act) and returns where the source is now.
fn wake_and_resolve(db: &CatalogDb, src: &Source) -> Availability {
    let table = MountTable::current();
    let a = availability_word(&table, src);
    if let Availability::Asleep { mount_point } = &a {
        let _ = crate::volumes::answering(Path::new(mount_point));
        let table = MountTable::current();
        let statuses = reconcile(db, &table, &system_uuid_of).unwrap_or_default();
        return statuses.into_iter().find(|s| s.source.id == src.id).map(|s| s.availability).unwrap_or(a);
    }
    let statuses = reconcile(db, &table, &system_uuid_of).unwrap_or_default();
    statuses.into_iter().find(|s| s.source.id == src.id).map(|s| s.availability).unwrap_or(a)
}

fn cmd_scan(args: &Args, o: &mut Out, alias: bool) -> i32 {
    let db = match open_rw(args, o) {
        Ok(d) => d,
        Err(c) => return c,
    };
    let all = match db.read().and_then(|c| list_sources(&c).map_err(CatalogError::Other)) {
        Ok(l) => l,
        Err(e) => return catalog_error(o, &e),
    };
    let keys: Vec<&String> = args.positional.iter().skip(1).collect();
    let chosen: Vec<Source> = if args.flag("--all") {
        all.iter().filter(|s| s.enabled).cloned().collect()
    } else if keys.is_empty() {
        o.warn(if alias { "Give the source to rescan: immage catalog rescan <source>" } else { "Give a source (id or name) or --all." });
        return EXIT_USAGE;
    } else {
        let mut v = Vec::new();
        for k in keys {
            match all.iter().find(|s| s.id.to_string() == *k).or_else(|| all.iter().find(|s| s.name.eq_ignore_ascii_case(k))) {
                Some(s) => v.push(s.clone()),
                None => {
                    o.warn(format!("No source \"{k}\"."));
                    return EXIT_USAGE;
                }
            }
        }
        v
    };
    if chosen.is_empty() {
        o.warn("There are no sources to scan.");
        return EXIT_OK;
    }
    if args.flag("--no-thumbnails") {
        o.warn("--no-thumbnails: thumbnails are not part of this version yet.");
    }
    let chain_metadata = args.flag("--metadata") || args.flag("--metadata-only");
    install_sigint();
    let (mut offline, mut partial, mut interrupted) = (0usize, false, false);
    for src in &chosen {
        if INTERRUPTED.load(Ordering::SeqCst) {
            interrupted = true;
            break;
        }
        let a = wake_and_resolve(&db, src);
        let a = match a {
            Availability::Online { abs_root, .. } if !Path::new(&abs_root).is_dir() => {
                Availability::Offline { reason: "folder not found".into() }
            }
            other => other,
        };
        let Availability::Online { abs_root, .. } = &a else {
            offline += 1;
            if o.json {
                o.data(&json!({"source": src.id, "name": src.name, "state": "offline", "found": 0, "new": 0, "changed": 0, "missing": 0,
                    "unreadable": 0, "incomplete_dirs": [], "elapsed_s": 0}));
            } else {
                o.say(format!("{}: not reachable ({}). Nothing was changed.", src.name, a.word()));
            }
            continue;
        };
        let fs_network = crate::volumes::is_network_fs(&src.fs_type);
        let network = sources::treats_as_network(src, fs_network);
        let mut cfg = ScanConfig::for_network(network);
        if let Some(n) = args.concurrency {
            cfg.workers = n;
        }
        let opts = ScanOptions {
            root: PathBuf::from(abs_root),
            include_subfolders: src.include_subfolders,
            dir_skip: src.dir_skip,
            light: args.flag("--light"),
            fresh: false,
        };
        let fs = RealFs::new(PathBuf::from(abs_root));
        let ctl = ScanControl::default();
        let mut printer = ProgressPrinter { name: src.name.chars().take(20).collect(), last: None, printed_line: false };
        let done = AtomicBool::new(false);
        let result = std::thread::scope(|sc| {
            sc.spawn(|| {
                while !done.load(Ordering::SeqCst) {
                    if INTERRUPTED.load(Ordering::SeqCst) {
                        ctl.interrupt();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            });
            let r = run_scan(&db, &fs, src, &opts, &cfg, &ctl, &mut |p| printer.show(o, p));
            done.store(true, Ordering::SeqCst);
            r
        });
        printer.clear(o);
        let report = match result {
            Ok(r) => r,
            Err(e) => {
                o.warn(format!("{}: {e}", src.name));
                return EXIT_FAIL;
            }
        };
        match report.state {
            "complete" => {
                if report.totals.unreadable > 0 {
                    partial = true;
                }
            }
            "interrupted" => interrupted = true,
            "offline" => offline += 1,
            "cancelled" => interrupted = true,
            _ => partial = true,
        }
        if o.json {
            o.data(&report_json(src, &report));
        } else {
            print_summary(o, src, &report);
        }
        // the metadata of what this scan found, while the volume is known to answer
        if chain_metadata && matches!(report.state, "complete" | "incomplete") && !INTERRUPTED.load(Ordering::SeqCst) {
            match meta_source(&db, args, o, src, abs_root) {
                MetaEnd::Ok => {}
                MetaEnd::Partial => partial = true,
                MetaEnd::Interrupted => interrupted = true,
                MetaEnd::Offline => offline += 1,
                MetaEnd::Failed => return EXIT_FAIL,
            }
        }
    }
    let n = chosen.len();
    if interrupted {
        o.warn("Interrupted. Progress is saved; run the same command again to continue.");
        EXIT_INTERRUPTED
    } else if offline == n {
        EXIT_OFFLINE
    } else if partial || offline > 0 {
        EXIT_PARTIAL
    } else {
        EXIT_OK
    }
}

fn print_summary(o: &mut Out, src: &Source, r: &ScanReport) {
    let t = &r.totals;
    let secs = r.elapsed.as_secs_f64();
    match r.state {
        "complete" | "incomplete" => {
            o.say(format!(
                "{}: checked {} photos in {}, {} new, {} changed, {} missing.",
                src.name, fmt_n(t.found), fmt_dur(secs), fmt_n(t.new), fmt_n(t.changed), fmt_n(t.missing)
            ));
            if r.resumed {
                o.say(format!("{}: continued an earlier scan.", src.name));
            }
            if t.unreadable > 0 {
                o.say(format!("{}: {} files had names Immage could not read. They were skipped.", src.name, fmt_n(t.unreadable)));
            }
            if r.state == "incomplete" {
                for (dir, status, err) in t.incomplete_dirs.iter().take(10) {
                    let what = err.clone().unwrap_or_else(|| status.clone());
                    let shown = if dir.is_empty() { "(the source's own folder)" } else { dir.as_str() };
                    o.say(format!("incomplete: {shown} ({what}); nothing marked missing"));
                }
                if t.incomplete_dirs.len() > 10 {
                    o.say(format!("... and {} more", t.incomplete_dirs.len() - 10));
                }
                if t.incomplete_dirs.is_empty() {
                    if let Some(e) = &r.error {
                        o.say(format!("incomplete: {e}; nothing marked missing"));
                    }
                }
            }
        }
        "offline" => o.say(format!(
            "{}: the volume went away during the scan ({} photos seen so far). Nothing was marked missing; run it again to continue.",
            src.name, fmt_n(r.files_listed)
        )),
        "interrupted" | "cancelled" => o.say(format!("{}: stopped after {} photos.", src.name, fmt_n(r.files_listed))),
        _ => o.say(format!("{}: failed: {}", src.name, r.error.clone().unwrap_or_default())),
    }
    if o.verbose {
        o.warn(format!("{}: {} folders listed, {:.0} files/s", src.name, fmt_n(r.dirs_listed), r.files_per_s));
    }
}

// ---- metadata ---------------------------------------------------------------------------

enum MetaEnd {
    Ok,
    /// Some photos failed to read.
    Partial,
    Interrupted,
    Offline,
    Failed,
}

pub fn meta_progress_line(name: &str, p: &MetaProgress) -> String {
    let read = p.done + p.failed + p.unparsed;
    let mut s = format!("{name:<20} reading metadata {} of {}", fmt_n(read), fmt_n(p.total));
    if p.phase == "linking" {
        return format!("{name:<20} matching renamed photos");
    }
    if p.rate_per_s >= 1.0 {
        s += &format!(", {}/s", fmt_n(p.rate_per_s as u64));
    }
    if p.paused {
        s += " (paused)";
    } else if let Some((lo, hi)) = p.eta_s {
        s += &format!(", about {} left ({}-{})", fmt_short((lo + hi) / 2), fmt_short(lo), fmt_short(hi));
    }
    if p.failed > 0 {
        s += &format!(", {} failed", p.failed);
    }
    s
}

fn meta_json(src: &Source, r: &MetaReport) -> Value {
    json!({
        "source": src.id, "name": src.name, "state": r.state, "resumed": r.resumed,
        "metadata": {
            "total": r.total, "done": r.done, "sidecar_only": r.sidecar_only, "failed": r.failed, "unparsed": r.unparsed,
            "skipped": r.skipped, "raced": r.raced, "reads": r.reads, "escalations": r.escalations, "bytes_read": r.bytes_read,
            "elapsed_s": (r.elapsed.as_secs_f64() * 100.0).round() / 100.0, "per_s": r.per_s.round(),
            "p50_ms": (r.p50_ms * 10.0).round() / 10.0, "p95_ms": (r.p95_ms * 10.0).round() / 10.0,
            "relinked": r.relink.relinked, "ambiguous": r.relink.ambiguous,
        },
        "error": r.error,
    })
}

fn print_meta_summary(o: &mut Out, src: &Source, r: &MetaReport) {
    let secs = r.elapsed.as_secs_f64();
    match r.state {
        "complete" => {
            if r.total == 0 {
                o.say(format!("{}: metadata is up to date ({} photos already read).", src.name, fmt_n(r.skipped)));
            } else {
                o.say(format!(
                    "{}: read {} photos in {} ({}/s), {} failed, {} not understood.",
                    src.name, fmt_n(r.done + r.sidecar_only), fmt_dur(secs), fmt_n(r.per_s as u64), fmt_n(r.failed), fmt_n(r.unparsed)
                ));
            }
            if r.relink.relinked > 0 {
                o.say(format!("{}: {} renamed or moved photos were matched to their old entries.", src.name, fmt_n(r.relink.relinked)));
            }
            if r.relink.ambiguous > 0 {
                o.say(format!("{}: {} renamed photos could not be matched (identical copies); left as they are.", src.name, fmt_n(r.relink.ambiguous)));
            }
            if r.failed > 0 {
                o.say(format!("{}: {} photos could not be read; they are tried again next time.", src.name, fmt_n(r.failed)));
            }
        }
        "offline" => o.say(format!("{}: the volume stopped answering after {} photos. Nothing was marked failed or missing; run it again to continue.", src.name, fmt_n(r.done))),
        "interrupted" | "cancelled" => o.say(format!("{}: stopped after {} photos.", src.name, fmt_n(r.done))),
        _ => o.say(format!("{}: failed: {}", src.name, r.error.clone().unwrap_or_default())),
    }
    if o.verbose {
        o.warn(format!(
            "{}: {} reads ({} escalated), {} read, p50 {:.1} ms, p95 {:.1} ms",
            src.name, fmt_n(r.reads), fmt_n(r.escalations), fmt_mb(r.bytes_read), r.p50_ms, r.p95_ms
        ));
    }
}

/// Runs the metadata job for one online source and prints its result.
fn meta_source(db: &CatalogDb, args: &Args, o: &mut Out, src: &Source, abs_root: &str) -> MetaEnd {
    let network = sources::treats_as_network(src, crate::volumes::is_network_fs(&src.fs_type));
    let mut cfg = MetaConfig::for_network(network);
    if let Some(n) = args.concurrency {
        cfg.workers = n;
    }
    if let Some(kb) = args.value("--first-read").and_then(|v| v.parse::<usize>().ok()) {
        cfg.first_read = (kb * 1024).clamp(64 * 1024, 1024 * 1024);
        cfg.max_read = cfg.max_read.max(cfg.first_read);
    }
    cfg.retry_failed = !args.flag("--skip-failed");
    cfg.sidecars = !args.flag("--no-sidecars");
    cfg.relink = !args.flag("--no-relink");
    cfg.adaptive = !args.flag("--no-adaptive");
    let fs = RealFs::new(PathBuf::from(abs_root));
    let ctl = ScanControl::default();
    install_sigint();
    let name: String = src.name.chars().take(20).collect();
    let (mut last, mut printed) = (None::<Instant>, false);
    let done = AtomicBool::new(false);
    let result = std::thread::scope(|sc| {
        sc.spawn(|| {
            while !done.load(Ordering::SeqCst) {
                if INTERRUPTED.load(Ordering::SeqCst) {
                    ctl.interrupt();
                    break;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        });
        let r = run_metadata(db, &fs, src, &cfg, &ctl, &mut |p: &MetaProgress| {
            if o.quiet && !o.progress_events {
                return;
            }
            if o.json {
                if o.progress_events && p.phase != "done" {
                    let ev = json!({"event":"progress","source":p.source_id,"phase":p.phase,"done":p.done + p.failed + p.unparsed,"total":p.total,
                        "rate":p.rate_per_s.round(),"eta_s":p.eta_s.map(|(a,b)|json!([a,b])),"failed":p.failed,"workers":p.workers});
                    let _ = writeln!(o.err, "{ev}");
                }
                return;
            }
            let line = meta_progress_line(&name, p);
            if o.tty {
                let _ = write!(o.err, "\r\x1b[2K{line}");
                let _ = o.err.flush();
                printed = true;
                if p.phase == "done" {
                    let _ = write!(o.err, "\r\x1b[2K");
                    printed = false;
                }
            } else if p.phase != "done" && last.is_none_or(|t| t.elapsed() >= PLAIN_PROGRESS_EVERY) {
                last = Some(Instant::now());
                let _ = writeln!(o.err, "{line}");
            }
        });
        done.store(true, Ordering::SeqCst);
        r
    });
    if printed && o.tty {
        let _ = write!(o.err, "\r\x1b[2K");
    }
    let report = match result {
        Ok(r) => r,
        Err(e) => {
            o.warn(format!("{}: {e}", src.name));
            return MetaEnd::Failed;
        }
    };
    if o.json {
        o.data(&meta_json(src, &report));
    } else {
        print_meta_summary(o, src, &report);
    }
    match report.state {
        "complete" if report.failed > 0 => MetaEnd::Partial,
        "complete" => MetaEnd::Ok,
        "interrupted" | "cancelled" => MetaEnd::Interrupted,
        "offline" => MetaEnd::Offline,
        _ => MetaEnd::Failed,
    }
}

fn cmd_metadata(args: &Args, o: &mut Out) -> i32 {
    let db = match open_rw(args, o) {
        Ok(d) => d,
        Err(c) => return c,
    };
    let all = match db.read().and_then(|c| list_sources(&c).map_err(CatalogError::Other)) {
        Ok(l) => l,
        Err(e) => return catalog_error(o, &e),
    };
    let keys: Vec<&String> = args.positional.iter().skip(1).collect();
    let chosen: Vec<Source> = if args.flag("--all") {
        all.iter().filter(|s| s.enabled).cloned().collect()
    } else if keys.is_empty() {
        o.warn("Give a source (id or name) or --all.");
        return EXIT_USAGE;
    } else {
        let mut v = Vec::new();
        for k in keys {
            match all.iter().find(|s| s.id.to_string() == *k).or_else(|| all.iter().find(|s| s.name.eq_ignore_ascii_case(k))) {
                Some(s) => v.push(s.clone()),
                None => {
                    o.warn(format!("No source \"{k}\"."));
                    return EXIT_USAGE;
                }
            }
        }
        v
    };
    if chosen.is_empty() {
        o.warn("There are no sources.");
        return EXIT_OK;
    }
    let (mut offline, mut partial, mut interrupted) = (0usize, false, false);
    for src in &chosen {
        if INTERRUPTED.load(Ordering::SeqCst) {
            interrupted = true;
            break;
        }
        let a = match wake_and_resolve(&db, src) {
            Availability::Online { abs_root, .. } if !Path::new(&abs_root).is_dir() => Availability::Offline { reason: "folder not found".into() },
            other => other,
        };
        let Availability::Online { abs_root, .. } = &a else {
            offline += 1;
            if o.json {
                o.data(&json!({"source": src.id, "name": src.name, "state": "offline", "metadata": null}));
            } else {
                o.say(format!("{}: not reachable ({}). Nothing was read or changed.", src.name, a.word()));
            }
            continue;
        };
        match meta_source(&db, args, o, src, abs_root) {
            MetaEnd::Ok => {}
            MetaEnd::Partial => partial = true,
            MetaEnd::Interrupted => interrupted = true,
            MetaEnd::Offline => offline += 1,
            MetaEnd::Failed => return EXIT_FAIL,
        }
    }
    if interrupted {
        o.warn("Interrupted. Progress is saved; run the same command again to continue.");
        EXIT_INTERRUPTED
    } else if offline == chosen.len() {
        EXIT_OFFLINE
    } else if partial || offline > 0 {
        EXIT_PARTIAL
    } else {
        EXIT_OK
    }
}

fn cmd_status(args: &Args, o: &mut Out) -> i32 {
    let watch = args.flag("--watch");
    loop {
        let code = status_once(args, o);
        if !watch || code == EXIT_CATALOG {
            return code;
        }
        std::thread::sleep(Duration::from_secs(2));
        if o.tty {
            let _ = write!(o.out, "\x1b[2J\x1b[H");
        } else {
            o.say("");
        }
    }
}

fn cov_json(c: &crate::catalog_meta_job::MetaCoverage) -> Value {
    json!({"done": c.done, "pending": c.pending, "failed": c.failed, "unparsed": c.unparsed})
}

fn status_once(args: &Args, o: &mut Out) -> i32 {
    let path = args.db_path();
    let conn = match CatalogDb::reader(&path) {
        Ok(c) => c,
        Err(e) => return catalog_error(o, &e),
    };
    let sources_all = match list_sources(&conn) {
        Ok(l) => l,
        Err(e) => {
            o.warn(e);
            return EXIT_CATALOG;
        }
    };
    let focus = args.positional.get(1).and_then(|k| sources_all.iter().find(|s| s.id.to_string() == *k || s.name.eq_ignore_ascii_case(k)).cloned());
    if let (Some(k), None) = (args.positional.get(1), &focus) {
        o.warn(format!("No source \"{k}\"."));
        return EXIT_USAGE;
    }
    let table = MountTable::current();
    let avail: Vec<(Source, Availability)> = sources_all.iter().map(|s| (s.clone(), checked_availability(&table, s))).collect();
    let n = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0);
    let photos = n("SELECT COUNT(*) FROM photos WHERE status='present'");
    let missing = n("SELECT COUNT(*) FROM photos WHERE status='missing'");
    let running = n("SELECT COUNT(*) FROM jobs WHERE state='running'");
    let queued = n("SELECT COUNT(*) FROM jobs WHERE state IN ('queued','paused')");
    let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let schema: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap_or(0);
    let online = avail.iter().filter(|(_, a)| matches!(a, Availability::Online { .. })).count();
    let offline = avail.len() - online;
    // running jobs' last progress, and anything that needs attention
    let mut running_lines = Vec::new();
    if let Ok(mut st) = conn.prepare("SELECT source_id, progress FROM jobs WHERE state='running'") {
        if let Ok(rows) = st.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?))) {
            for (sid, prog) in rows.flatten() {
                if let (Some(s), Some(p)) = (sources_all.iter().find(|s| s.id == sid), prog.and_then(|p| serde_json::from_str::<Value>(&p).ok())) {
                    running_lines.push(format!(
                        "{}  finding photos  {} so far, {}/s",
                        s.name, fmt_n(p["found"].as_u64().unwrap_or(0)), fmt_n(p["rate_per_s"].as_f64().unwrap_or(0.0) as u64)
                    ));
                }
            }
        }
    }
    let cov = coverage(&conn, focus.as_ref().map(|f| f.id)).unwrap_or_default();
    let mut attention: Vec<String> = Vec::new();
    for (s, a) in &avail {
        if focus.as_ref().is_some_and(|f| f.id != s.id) {
            continue;
        }
        if s.state == "attention" {
            let dirs: Vec<String> = conn
                .prepare("SELECT rel_path FROM dirs WHERE source_id=?1 AND status IN ('stalled','denied','error') ORDER BY rel_path LIMIT 3")
                .ok()
                .map(|mut st| st.query_map([s.id], |r| r.get::<_, String>(0)).map(|rows| rows.flatten().collect()).unwrap_or_default())
                .unwrap_or_default();
            attention.push(format!(
                "{}: some folders did not answer{}",
                s.name,
                if dirs.is_empty() { String::new() } else { format!(" ({})", dirs.iter().map(|d| if d.is_empty() { "." } else { d.as_str() }).collect::<Vec<_>>().join(", ")) }
            ));
        } else if s.state == "paused" {
            attention.push(format!("{}: a scan was interrupted; run scan to continue", s.name));
        }
        if matches!(a, Availability::Offline { .. }) {
            attention.push(format!("{}: offline", s.name));
        }
    }
    if o.json {
        let v = json!({
            "catalog": path.to_string_lossy(), "schema": schema, "bytes": size, "photos": photos, "missing": missing,
            "sources": { "total": avail.len(), "online": online, "offline": offline },
            "jobs": { "running": running, "queued": queued },
            "metadata": cov_json(&cov),
            "sources_list": avail.iter().filter(|(s, _)| focus.as_ref().is_none_or(|f| f.id == s.id)).map(|(s, a)| {
                let mut v = source_json(s, a);
                v["metadata"] = cov_json(&coverage(&conn, Some(s.id)).unwrap_or_default());
                v
            }).collect::<Vec<_>>(),
            "running": running_lines, "attention": attention,
        });
        o.data(&v);
    } else {
        o.say(format!("Catalog  {}   schema {schema}   {}   {} photos", path.display(), fmt_mb(size), fmt_n(photos as u64)));
        o.say(format!("Sources  {} ({online} online, {offline} offline)   Jobs: {running} running, {queued} queued", avail.len()));
        if missing > 0 {
            o.say(format!("Missing  {} photos are marked missing (kept in the catalog)", fmt_n(missing as u64)));
        }
        o.say(format!(
            "Metadata {} read, {} waiting, {} failed, {} not understood",
            fmt_n(cov.done as u64), fmt_n(cov.pending as u64), fmt_n(cov.failed as u64), fmt_n(cov.unparsed as u64)
        ));
        for l in &running_lines {
            o.say(format!("Running  {l}"));
        }
        for a in &attention {
            o.say(format!("Attention {a}"));
        }
        if let Some(f) = &focus {
            if let Some((_, a)) = avail.iter().find(|(s, _)| s.id == f.id) {
                o.say(format!("Source {} \"{}\": {} photos, last check {}, {}", f.id, f.name, fmt_n(f.photos as u64), fmt_ago(f.last_scan_end), state_word(f, a)));
            }
        }
    }
    let _ = SCHEMA_VERSION;
    if attention.is_empty() { EXIT_OK } else { EXIT_PARTIAL }
}

fn cmd_verify(args: &Args, o: &mut Out) -> i32 {
    let fix = args.flag("--fix");
    let path = args.db_path();
    let db = if fix {
        match open_rw(args, o) {
            Ok(d) => Some(d),
            Err(c) => return c,
        }
    } else {
        None
    };
    let conn = match &db {
        Some(d) => d.read(),
        None => CatalogDb::reader(&path),
    };
    let conn = match conn {
        Ok(c) => c,
        Err(e) => return catalog_error(o, &e),
    };
    let mut checks: Vec<(String, bool, String, bool)> = Vec::new(); // name, ok, detail, fixable
    let integrity: String = conn.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap_or_else(|e| e.to_string());
    let sound = integrity == "ok";
    checks.push(("database integrity".into(), sound, if sound { String::new() } else { integrity }, false));
    if !sound {
        return report_verify(o, checks, 0, EXIT_CATALOG);
    }
    let fk: i64 = conn.prepare("PRAGMA foreign_key_check").and_then(|mut s| s.query_map([], |_| Ok(())).map(|r| r.count() as i64)).unwrap_or(0);
    checks.push(("photos and folders belong to a source".into(), fk == 0, format!("{fk} rows without a parent"), false));
    let n = |sql: &str| conn.query_row(sql, [], |r| r.get::<_, i64>(0)).unwrap_or(0);
    let stale_gone = n("SELECT COUNT(*) FROM scan_gone WHERE run_id NOT IN (SELECT id FROM scan_runs WHERE state='running')");
    checks.push(("no leftover scan bookkeeping".into(), stale_gone == 0, format!("{stale_gone} stale rows"), true));
    let orphan_notes = n("SELECT COUNT(*) FROM scan_notes WHERE source_id NOT IN (SELECT id FROM sources)");
    checks.push(("no notes of removed sources".into(), orphan_notes == 0, format!("{orphan_notes} orphan notes"), true));
    let stuck = n("SELECT COUNT(*) FROM jobs WHERE state IN ('running','paused')");
    checks.push(("no scan stuck as running".into(), stuck == 0, format!("{stuck} jobs look running (the app may be scanning)"), false));
    let dir_mismatch = n("SELECT COUNT(*) FROM photos p JOIN dirs d ON d.id=p.dir_id WHERE d.source_id<>p.source_id");
    checks.push(("photos sit in their source's folders".into(), dir_mismatch == 0, format!("{dir_mismatch} photos in another source's folder"), false));
    if args.flag("--deep") {
        // re-check what the catalog recorded against the files: reads the volume (stat only)
        let table = MountTable::current();
        let mut mismatched = 0u64;
        let mut checked = 0u64;
        let only = args.value("--source").map(str::to_string);
        for s in list_sources(&conn).unwrap_or_default() {
            if only.as_ref().is_some_and(|k| *k != s.id.to_string() && !s.name.eq_ignore_ascii_case(k)) {
                continue;
            }
            let Availability::Online { abs_root, .. } = availability_word(&table, &s) else { continue };
            let Ok(mut st) = conn.prepare("SELECT rel_path,size,mtime_ns FROM photos WHERE source_id=?1 AND status='present'") else { continue };
            let Ok(rows) = st.query_map([s.id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?, r.get::<_, i64>(2)?))) else { continue };
            for (rel, size, mtime) in rows.flatten() {
                checked += 1;
                let p = sources::join_path(&abs_root, &rel);
                match std::fs::metadata(&p) {
                    Ok(m) => {
                        let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as i64);
                        if m.len() as i64 != size || t != mtime {
                            mismatched += 1;
                        }
                    }
                    Err(_) => mismatched += 1,
                }
            }
        }
        checks.push((format!("files match the catalog ({} checked)", fmt_n(checked)), mismatched == 0, format!("{mismatched} differ; scan to update"), false));
    }
    let mut fixed = 0;
    if let Some(db) = &db {
        if stale_gone > 0 || orphan_notes > 0 {
            let r = db.exec(|c| {
                let a = c.execute("DELETE FROM scan_gone WHERE run_id NOT IN (SELECT id FROM scan_runs WHERE state='running')", []).map_err(|e| e.to_string())?;
                let b = c.execute("DELETE FROM scan_notes WHERE source_id NOT IN (SELECT id FROM sources)", []).map_err(|e| e.to_string())?;
                Ok(a + b)
            });
            if r.is_ok() {
                fixed = (stale_gone > 0) as i64 + (orphan_notes > 0) as i64;
                for c in checks.iter_mut().filter(|c| c.3) {
                    c.1 = true;
                    c.2 = "repaired".into();
                }
            }
        }
    }
    let problems = checks.iter().filter(|c| !c.1).count();
    report_verify(o, checks, fixed, if problems > 0 { EXIT_PARTIAL } else { EXIT_OK })
}

fn report_verify(o: &mut Out, checks: Vec<(String, bool, String, bool)>, fixed: i64, code: i32) -> i32 {
    let problems = checks.iter().filter(|c| !c.1).count();
    if o.json {
        let v: Vec<Value> = checks.iter().map(|c| json!({"check": c.0, "ok": c.1, "detail": c.2, "fixable": c.3})).collect();
        o.data(&json!({"checks": v, "problems": problems, "fixed": fixed}));
        return code;
    }
    for (name, ok, detail, _) in &checks {
        if *ok {
            o.say(format!("ok       {name}"));
        } else {
            o.say(format!("problem  {name}: {detail}"));
        }
    }
    if problems == 0 {
        o.say(if fixed > 0 { format!("Found no problems left, fixed {fixed}.") } else { "Found no problems.".to_string() });
    } else {
        o.say(format!("Found {problems} problems, fixed {fixed}."));
    }
    code
}

// keep host_hash in the public surface for logs
#[allow(dead_code)]
pub fn log_host(host: &str) -> String {
    host_hash(host)
}

#[cfg(test)]
#[path = "catalog_cli_tests.rs"]
mod tests;
