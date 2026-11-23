// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The model pack manager: one generic install / verify / repair / remove /
//! cancel / resume pipeline for every optional model pack (CLIP first), with
//! progress events and disk accounting. A pack is a pinned list of files (size
//! and a SHA-256, or a Git blob id for small files), a source and a license.
//! Nothing here ever downloads on its own: `PackStore::start` and
//! `install_blocking` are reached only from an explicit user act (the consent
//! dialog, Resume or Repair; see pack_cmds.rs).
//!
//! Layout under a pack's root (CLIP: `<runtime>/clip-models`):
//!   `<revision>/`            the installed package (the layout CLIP always had)
//!   `.partial-<revision>/`   files kept between attempts: whole verified files
//!                            and `<file>.part` for one cut short (resumed with a
//!                            Range request); kept on Cancel and when offline,
//!                            removed on a failed check or a full disk
//!   `<revision>/.immage-verified.json`  when each file was last checked, with
//!                            its size and mtime, so the state can say "verified"
//!                            without hashing again (extra to the pinned files)
use crate::analysis_settings::AnalysisId;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, serde::Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PackId {
    Clip,
    Ram,
}

// UpdateAvailable and Unsupported are part of the page's contract (the card draws them) and are
// first produced by packs that publish a newer revision or need a CPU feature; none does yet
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PackState {
    NotInstalled,
    Downloading,
    /// Download stopped (cancelled or offline): partial files are kept.
    Stopped,
    Installed,
    NeedsRepair,
    UpdateAvailable,
    Unsupported,
}

#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PackErrorKind {
    Offline,
    DiskFull,
    HashMismatch,
    Unsupported,
    /// Not shown to the user: a cancelled download is a Stopped pack with no error.
    Cancelled,
    Other,
}

#[derive(Clone, Debug, serde::Serialize, specta::Type)]
pub struct PackError {
    pub kind: PackErrorKind,
    pub detail: String,
}
impl PackError {
    fn new(kind: PackErrorKind, detail: impl Into<String>) -> Self {
        PackError { kind, detail: detail.into() }
    }
    fn other(detail: impl Into<String>) -> Self {
        Self::new(PackErrorKind::Other, detail)
    }
    fn cancelled() -> Self {
        Self::new(PackErrorKind::Cancelled, "Model download cancelled")
    }
}
impl std::fmt::Display for PackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.detail)
    }
}
impl From<String> for PackError {
    fn from(detail: String) -> Self {
        PackError::other(detail)
    }
}
impl From<&str> for PackError {
    fn from(detail: &str) -> Self {
        PackError::other(detail)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, serde::Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub enum PackPhase {
    Download,
    Verify,
}

#[derive(Clone, Debug, serde::Serialize, specta::Type)]
pub struct PackProgress {
    pub id: PackId,
    pub done: u64,
    pub total: u64,
    pub bytes_per_sec: f64,
    pub eta_s: Option<u32>,
    pub phase: PackPhase,
}

#[derive(Clone, Debug, serde::Serialize, specta::Type)]
pub struct PackInfo {
    pub id: PackId,
    pub name: String,
    pub license: String,
    pub license_url: String,
    pub source_host: String,
    /// The download size: every file of the pinned package.
    pub bytes: u64,
    pub bytes_on_disk: u64,
    /// What a Resume still has to fetch (the whole package when nothing is kept).
    pub bytes_left: u64,
    pub files: u32,
    pub files_ok: u32,
    pub state: PackState,
    /// Installed, and a check of its files is running (an install made before
    /// checks were recorded is checked once in the background).
    pub checking: bool,
    pub error: Option<PackError>,
    pub version: String,
    pub update_version: Option<String>,
    pub update_bytes: Option<u64>,
    pub verified_at: Option<i64>,
    pub free_bytes: u64,
    pub unsupported_reason: Option<String>,
    pub analyses: Vec<AnalysisId>,
    pub progress: Option<PackProgress>,
    /// Filled in by the command layer from the settings store.
    pub download_consent: bool,
    pub analysis_consent: bool,
    pub folder: String,
}

#[derive(Clone, Debug, serde::Serialize, specta::Type)]
pub struct PackCheck {
    pub files: u32,
    pub files_ok: u32,
    pub failed: Vec<String>,
}

pub enum PackEvent {
    State(PackInfo),
    Progress(PackProgress),
}
type Events = Arc<dyn Fn(PackEvent) + Send + Sync>;

pub struct PackFile {
    pub path: &'static str,
    pub size: u64,
    /// SHA-256 of the file, or the Git blob id when `git_blob`.
    pub digest: &'static str,
    pub git_blob: bool,
}

pub struct PackHooks {
    /// Written into the staged package before it is published (CLIP: the engine's config.json).
    pub prepare: fn(&Path) -> Result<(), String>,
    /// Loads the staged package once, so a package that cannot be used is never published.
    pub validate: fn(&Path) -> Result<(), String>,
}

/// The registry entry of one pack: everything about it that is fixed.
pub struct PackSpec {
    pub id: PackId,
    /// The folder under the runtime directory holding every revision of this pack.
    pub name: &'static str,
    pub license: &'static str,
    pub license_url: &'static str,
    pub license_text: &'static str,
    pub source_host: &'static str,
    /// The revision-pinned address the files are fetched from: `{base_url}/{path}`.
    pub base_url: &'static str,
    /// Tried after `base_url` when it fails (offline, an error status, a hash mismatch): the
    /// same pinned files from another host. Size and digest are the pin's, whichever host answers.
    pub fallback_base_url: Option<&'static str>,
    pub revision: &'static str,
    pub files: &'static [PackFile],
    pub analyses: &'static [AnalysisId],
    pub hooks: PackHooks,
}
impl PackSpec {
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|f| f.size).sum()
    }
}

#[derive(Default)]
struct Run {
    downloading: bool,
    phase: Option<PackPhase>,
    checking: bool,
    /// In-process: the package passed a full check since it was last changed.
    verified: bool,
    error: Option<PackError>,
    started: Option<Instant>,
    started_done: u64,
}

pub struct PackStore {
    spec: &'static PackSpec,
    root: PathBuf,
    run: Mutex<Run>,
    cancelled: AtomicBool,
    done: AtomicU64,
    total: AtomicU64,
    events: Mutex<Option<Events>>,
    last_emit: Mutex<Option<Instant>>,
    /// Test seams: a mock server in place of the pinned host, and a disk size.
    base_override: Mutex<Option<String>>,
    /// Test seam: a plain-http second source, tried after the overridden first.
    fallback_override: Mutex<Option<String>>,
    free_override: Mutex<Option<u64>>,
}

fn now_s() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

// ---- hashing -------------------------------------------------------------

struct Digests {
    sha256: Sha256,
    git: sha1::Sha1,
    size: u64,
}
impl Digests {
    fn new(asset: &PackFile) -> Self {
        let mut git = <sha1::Sha1 as sha1::Digest>::new();
        sha1::Digest::update(&mut git, format!("blob {}\0", asset.size).as_bytes());
        Digests { sha256: Sha256::new(), git, size: 0 }
    }
    fn update(&mut self, bytes: &[u8]) {
        self.size += bytes.len() as u64;
        self.sha256.update(bytes);
        sha1::Digest::update(&mut self.git, bytes);
    }
    fn matches(self, asset: &PackFile) -> bool {
        let actual = if asset.git_blob {
            sha1::Digest::finalize(self.git).iter().map(|b| format!("{b:02x}")).collect::<String>()
        } else {
            format!("{:x}", self.sha256.finalize())
        };
        self.size == asset.size && actual == asset.digest
    }
}

fn is_disk_full(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::StorageFull || matches!(e.raw_os_error(), Some(28) | Some(122) | Some(112))
}
fn read_error(e: std::io::Error, net: bool) -> PackError {
    if e.to_string().contains("cancelled") {
        PackError::cancelled()
    } else if net {
        PackError::new(PackErrorKind::Offline, e.to_string())
    } else {
        PackError::other(e.to_string())
    }
}
fn write_error(e: std::io::Error) -> PackError {
    if is_disk_full(&e) {
        PackError::new(PackErrorKind::DiskFull, e.to_string())
    } else {
        PackError::other(e.to_string())
    }
}

pub struct Cancellable<'a, R> {
    pub reader: &'a mut R,
    pub cancelled: &'a AtomicBool,
}
impl<R: Read> Read for Cancellable<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> std::io::Result<usize> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(std::io::Error::other("Model download cancelled"));
        }
        self.reader.read(output)
    }
}

/// Streams `input` into `output`, hashing as it goes, and fails unless exactly
/// the pinned bytes arrive. `digests` may already hold a resumed prefix. Reads
/// at most one byte beyond the pinned size, even from a dishonest server.
fn copy_digests(
    input: &mut impl Read,
    output: &mut impl Write,
    asset: &PackFile,
    mut digests: Digests,
    net: bool,
    progress: &mut dyn FnMut(u64),
) -> Result<(), PackError> {
    let mut buffer = [0u8; 65536];
    loop {
        let limit = (asset.size.saturating_sub(digests.size).saturating_add(1)).min(buffer.len() as u64) as usize;
        let n = input.read(&mut buffer[..limit]).map_err(|e| read_error(e, net))?;
        if n == 0 {
            break;
        }
        if digests.size + n as u64 > asset.size {
            return Err(PackError::new(PackErrorKind::HashMismatch, format!("{} exceeds pinned size", asset.path)));
        }
        digests.update(&buffer[..n]);
        output.write_all(&buffer[..n]).map_err(write_error)?;
        progress(n as u64);
    }
    if !digests.matches(asset) {
        return Err(PackError::new(PackErrorKind::HashMismatch, format!("{} failed pinned integrity check", asset.path)));
    }
    Ok(())
}
pub fn copy_verified(input: &mut impl Read, output: &mut impl Write, asset: &PackFile) -> Result<(), PackError> {
    copy_digests(input, output, asset, Digests::new(asset), false, &mut |_| {})
}

fn verify_file(path: &Path, asset: &PackFile) -> Result<(), PackError> {
    let meta = std::fs::symlink_metadata(path)
        .map_err(|e| PackError::new(PackErrorKind::HashMismatch, format!("Missing model asset {}: {e}", asset.path)))?;
    if !meta.is_file() || meta.len() != asset.size {
        return Err(PackError::new(PackErrorKind::HashMismatch, format!("Invalid model asset {}", asset.path)));
    }
    let mut file = std::fs::File::open(path).map_err(|e| PackError::other(e.to_string()))?;
    copy_verified(&mut file, &mut std::io::sink(), asset)
}
pub fn verify_package(root: &Path, assets: &[PackFile]) -> Result<(), PackError> {
    for asset in assets {
        verify_file(&root.join(asset.path), asset)?;
    }
    Ok(())
}

// ---- disk ----------------------------------------------------------------

fn dir_bytes(dir: &Path) -> u64 {
    let Ok(read) = std::fs::read_dir(dir) else { return 0 };
    read.flatten()
        .map(|e| match e.metadata() {
            Ok(m) if m.is_dir() => dir_bytes(&e.path()),
            Ok(m) => m.len(),
            Err(_) => 0,
        })
        .sum()
}
type Stamp = (String, u64, u128);
fn stamps(root: &Path, assets: &[PackFile]) -> Option<Vec<Stamp>> {
    assets
        .iter()
        .map(|a| {
            let m = std::fs::symlink_metadata(root.join(a.path)).ok()?;
            if !m.is_file() || m.len() != a.size {
                return None;
            }
            let t = m.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_nanos();
            Some((a.path.to_string(), m.len(), t))
        })
        .collect()
}

impl PackStore {
    pub fn new(spec: &'static PackSpec, root: PathBuf) -> Self {
        PackStore {
            spec,
            root,
            run: Mutex::new(Run::default()),
            cancelled: AtomicBool::new(false),
            done: AtomicU64::new(0),
            total: AtomicU64::new(0),
            events: Mutex::new(None),
            last_emit: Mutex::new(None),
            base_override: Mutex::new(None),
            fallback_override: Mutex::new(None),
            free_override: Mutex::new(None),
        }
    }
    pub fn spec(&self) -> &'static PackSpec {
        self.spec
    }
    pub fn final_dir(&self) -> PathBuf {
        self.root.join(self.spec.revision)
    }
    fn partial_dir(&self) -> PathBuf {
        self.root.join(format!(".partial-{}", self.spec.revision))
    }
    fn marker(&self) -> PathBuf {
        self.final_dir().join(".immage-verified.json")
    }
    #[cfg(test)]
    pub fn cancel_flag(&self) -> &AtomicBool {
        &self.cancelled
    }
    pub fn cancel(&self) {
        // taken so a cancel is ordered against publication (see run_install)
        let _run = self.run.lock().unwrap();
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn set_events(&self, events: Events) {
        *self.events.lock().unwrap() = Some(events);
    }
    fn emit(&self, event: PackEvent) {
        let events = self.events.lock().unwrap().clone();
        if let Some(f) = events {
            f(event);
        }
    }
    fn emit_state(&self) {
        if self.events.lock().unwrap().is_some() {
            self.emit(PackEvent::State(self.info()));
        }
    }
    fn free(&self) -> u64 {
        if let Some(n) = *self.free_override.lock().unwrap() {
            return n;
        }
        let mut probe = self.root.as_path();
        while !probe.exists() {
            match probe.parent() {
                Some(p) => probe = p,
                None => break,
            }
        }
        crate::cache::free_space(probe).unwrap_or(u64::MAX)
    }
    pub fn is_downloading(&self) -> bool {
        self.run.lock().unwrap().downloading
    }
    pub fn set_verified(&self, verified: bool) {
        self.run.lock().unwrap().verified = verified;
    }
    /// The package passed a full check: remembered in memory and on disk.
    pub fn record_verified(&self) {
        self.set_verified(true);
        self.write_marker();
    }
    pub fn verified(&self) -> bool {
        self.run.lock().unwrap().verified
    }
    pub fn error(&self) -> Option<PackError> {
        self.run.lock().unwrap().error.clone()
    }

    /// The installed package directory exists (CLIP's long-standing test; a
    /// full check is `check_files`).
    pub fn installed(&self) -> bool {
        self.final_dir().is_dir()
    }

    fn read_marker(&self) -> Option<(i64, Vec<Stamp>)> {
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(self.marker()).ok()?).ok()?;
        let at = value.get("at")?.as_i64()?;
        let list = value
            .get("files")?
            .as_array()?
            .iter()
            .map(|f| {
                Some((
                    f.get("path")?.as_str()?.to_string(),
                    f.get("size")?.as_u64()?,
                    f.get("mtime")?.as_str()?.parse::<u128>().ok()?,
                ))
            })
            .collect::<Option<Vec<_>>>()?;
        Some((at, list))
    }
    fn write_marker(&self) {
        let Some(list) = stamps(&self.final_dir(), self.spec.files) else { return };
        let files: Vec<_> = list
            .iter()
            .map(|(p, s, m)| serde_json::json!({"path": p, "size": s, "mtime": m.to_string()}))
            .collect();
        let _ = std::fs::write(self.marker(), serde_json::json!({"at": now_s(), "files": files}).to_string());
    }
    /// When the files last passed a check, if they are still the ones checked.
    fn verified_at(&self) -> Option<i64> {
        let (at, saved) = self.read_marker()?;
        (stamps(&self.final_dir(), self.spec.files)? == saved).then_some(at)
    }

    fn files_present(&self, root: &Path) -> u32 {
        self.spec
            .files
            .iter()
            .filter(|a| std::fs::symlink_metadata(root.join(a.path)).is_ok_and(|m| m.is_file() && m.len() == a.size))
            .count() as u32
    }
    fn partial_bytes(&self) -> u64 {
        let dir = self.partial_dir();
        self.spec
            .files
            .iter()
            .map(|a| {
                let f = dir.join(a.path);
                let whole = std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
                let mut part = f.into_os_string();
                part.push(".part");
                whole.max(0) + std::fs::metadata(PathBuf::from(part)).map(|m| m.len()).unwrap_or(0)
            })
            .sum()
    }

    /// The pack as the page sees it. Cheap: sizes and mtimes, never a hash.
    pub fn info(&self) -> PackInfo {
        let spec = self.spec;
        let run = self.run.lock().unwrap();
        let (downloading, checking, error, phase) = (run.downloading, run.checking, run.error.clone(), run.phase);
        let started = run.started;
        let started_done = run.started_done;
        drop(run);
        let final_dir = self.final_dir();
        let has_final = final_dir.is_dir();
        let present = self.files_present(&final_dir);
        let partial = self.partial_bytes();
        let verified_at = if has_final { self.verified_at() } else { None };
        let total = spec.bytes();
        let state = if downloading {
            PackState::Downloading
        } else if has_final {
            if present as usize != spec.files.len() || error.as_ref().is_some_and(|e| e.kind == PackErrorKind::HashMismatch) {
                PackState::NeedsRepair
            } else {
                PackState::Installed
            }
        } else if partial > 0 {
            PackState::Stopped
        } else if error.as_ref().is_some_and(|e| e.kind == PackErrorKind::HashMismatch) {
            PackState::NeedsRepair
        } else {
            PackState::NotInstalled
        };
        let error = match (&state, error) {
            (PackState::NeedsRepair, None) => Some(PackError::new(
                PackErrorKind::HashMismatch,
                "Some model files are missing or have the wrong size.",
            )),
            (PackState::Installed, Some(e)) if e.kind != PackErrorKind::HashMismatch => None,
            (_, Some(e)) if e.kind == PackErrorKind::Cancelled => None,
            (_, e) => e,
        };
        let progress = downloading.then(|| {
            let done = self.done.load(Ordering::Acquire);
            let tot = self.total.load(Ordering::Acquire).max(1);
            let elapsed = started.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
            let rate = if elapsed > 0.5 { done.saturating_sub(started_done) as f64 / elapsed } else { 0.0 };
            PackProgress {
                id: spec.id,
                done,
                total: tot,
                bytes_per_sec: rate,
                eta_s: (rate > 1.0).then(|| (tot.saturating_sub(done) as f64 / rate).ceil() as u32),
                phase: phase.unwrap_or(PackPhase::Download),
            }
        });
        let on_disk = (if has_final { dir_bytes(&final_dir) } else { 0 }) + dir_bytes(&self.partial_dir());
        PackInfo {
            id: spec.id,
            name: spec.name.to_string(),
            license: spec.license.to_string(),
            license_url: spec.license_url.to_string(),
            source_host: spec.source_host.to_string(),
            bytes: total,
            bytes_on_disk: on_disk,
            bytes_left: if has_final { 0 } else { total.saturating_sub(partial) },
            files: spec.files.len() as u32,
            files_ok: present,
            state,
            checking,
            error,
            version: spec.revision.to_string(),
            update_version: None,
            update_bytes: None,
            verified_at,
            free_bytes: self.free(),
            unsupported_reason: None,
            analyses: spec.analyses.to_vec(),
            progress,
            download_consent: false,
            analysis_consent: false,
            folder: self.root.to_string_lossy().into_owned(),
        }
    }

    // ---- install ---------------------------------------------------------

    fn begin(&self) -> Result<(), String> {
        let mut run = self.run.lock().unwrap();
        if run.downloading {
            return Err("Model download is already running".into());
        }
        if run.checking {
            return Err("The model files are being checked".into());
        }
        self.cancelled.store(false, Ordering::Release);
        run.downloading = true;
        run.verified = false;
        run.error = None;
        run.phase = Some(PackPhase::Download);
        run.started = Some(Instant::now());
        self.total.store(self.spec.bytes(), Ordering::Release);
        Ok(())
    }
    fn finish(&self, result: &Result<(), PackError>) {
        let mut run = self.run.lock().unwrap();
        run.downloading = false;
        run.phase = None;
        run.verified = result.is_ok();
        run.error = match result {
            Err(e) if e.kind != PackErrorKind::Cancelled => Some(e.clone()),
            _ => None,
        };
    }

    /// Runs the whole install on the calling thread. The caller has the user's consent.
    pub fn install_blocking(&self) -> Result<(), PackError> {
        self.begin()?;
        let result = self.run_http();
        self.finish(&result);
        self.emit_state();
        result
    }
    /// Starts the install on a thread of its own and returns at once with the
    /// pack in Downloading. A second start while one runs is refused.
    pub fn start(self: &Arc<Self>) -> Result<(), String> {
        self.begin()?;
        let store = Arc::clone(self);
        std::thread::spawn(move || {
            let result = store.run_http();
            store.finish(&result);
            store.emit_state();
        });
        self.emit_state();
        Ok(())
    }

    fn run_http(&self) -> Result<(), PackError> {
        let mut fetch = |asset: &PackFile, dest: &Path| self.http_fetch(asset, dest);
        self.run_install(self.spec.files, &mut fetch, true)
    }

    /// The pipeline, with the transport as a closure (and no pack-specific validation, which
    /// needs the real files; the tests of the pipeline itself use this): reuses a valid install,
    /// else fills the partial folder file by file (skipping files that already
    /// verify), checks the whole package, publishes it with one rename.
    #[cfg(test)]
    pub fn install_with(
        &self,
        assets: &[PackFile],
        mut fetch: impl FnMut(&PackFile, &Path) -> Result<(), PackError>,
    ) -> Result<(), PackError> {
        self.run_install(assets, &mut fetch, false)
    }

    fn run_install(
        &self,
        assets: &[PackFile],
        fetch: &mut dyn FnMut(&PackFile, &Path) -> Result<(), PackError>,
        validate: bool,
    ) -> Result<(), PackError> {
        let spec = self.spec;
        let final_dir = self.final_dir();
        if verify_package(&final_dir, assets).is_ok() {
            (spec.hooks.prepare)(&final_dir)?;
            self.write_marker();
            return Ok(());
        }
        std::fs::create_dir_all(&self.root).map_err(|e| PackError::other(e.to_string()))?;
        let staging = self.partial_dir();
        std::fs::create_dir_all(&staging).map_err(|e| PackError::other(e.to_string()))?;
        // leftovers of the old per-attempt staging folders
        if let Ok(read) = std::fs::read_dir(&self.root) {
            for e in read.flatten() {
                if e.file_name().to_string_lossy().starts_with(".download-") {
                    let _ = std::fs::remove_dir_all(e.path());
                }
            }
        }
        let result = self.fill_and_publish(assets, &staging, fetch, validate);
        match &result {
            Ok(()) => {}
            // a stopped download keeps what it has, to be resumed
            Err(e) if matches!(e.kind, PackErrorKind::Cancelled | PackErrorKind::Offline) => {}
            // the failed file is already gone (HashMismatch); the others stay for the next try
            Err(e) if e.kind == PackErrorKind::HashMismatch => {}
            // a full disk must not leave a hidden gigabyte behind; an unknown failure starts clean
            Err(_) => {
                let _ = std::fs::remove_dir_all(&staging);
            }
        }
        result
    }

    fn fill_and_publish(
        &self,
        assets: &[PackFile],
        staging: &Path,
        fetch: &mut dyn FnMut(&PackFile, &Path) -> Result<(), PackError>,
        validate: bool,
    ) -> Result<(), PackError> {
        let spec = self.spec;
        let final_dir = self.final_dir();
        let total: u64 = assets.iter().map(|a| a.size).sum();
        self.total.store(total, Ordering::Release);
        // a repair keeps the files of the damaged install that still verify
        if final_dir.is_dir() {
            for a in assets {
                let dest = staging.join(a.path);
                if !dest.exists() && verify_file(&final_dir.join(a.path), a).is_ok() {
                    if let Some(parent) = dest.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    let src = final_dir.join(a.path);
                    if std::fs::hard_link(&src, &dest).is_err() {
                        let _ = std::fs::copy(&src, &dest);
                    }
                }
            }
        }
        // enough room for what is still to fetch, before any network access
        let have: u64 = assets
            .iter()
            .map(|a| {
                let f = staging.join(a.path);
                let mut part = f.clone().into_os_string();
                part.push(".part");
                std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0) + std::fs::metadata(PathBuf::from(part)).map(|m| m.len()).unwrap_or(0)
            })
            .sum();
        let need = total.saturating_sub(have);
        let free = self.free();
        if free < need {
            return Err(PackError::new(
                PackErrorKind::DiskFull,
                format!("Needs {need} bytes free, this disk has {free}."),
            ));
        }
        let mut done = 0u64;
        for a in assets {
            if self.cancelled.load(Ordering::Acquire) {
                return Err(PackError::cancelled());
            }
            let dest = staging.join(a.path);
            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent).map_err(|e| PackError::other(e.to_string()))?;
            }
            if dest.exists() {
                if verify_file(&dest, a).is_ok() {
                    done += a.size;
                    self.done.store(done, Ordering::Release);
                    continue;
                }
                let _ = std::fs::remove_file(&dest);
            }
            let mut part = dest.clone().into_os_string();
            part.push(".part");
            let have_part = std::fs::metadata(PathBuf::from(part)).map(|m| m.len()).unwrap_or(0);
            self.done.store(done + have_part, Ordering::Release);
            {
                let mut run = self.run.lock().unwrap();
                run.started_done = done + have_part;
                run.started = Some(Instant::now());
            }
            fetch(a, &dest)?;
            done += a.size;
            self.done.store(done, Ordering::Release);
        }
        self.run.lock().unwrap().phase = Some(PackPhase::Verify);
        // Independently verify staged files, including their size, before publication.
        for a in assets {
            if let Err(e) = verify_file(&staging.join(a.path), a) {
                let _ = std::fs::remove_file(staging.join(a.path));
                return Err(e);
            }
        }
        if validate {
            (spec.hooks.validate)(staging)?;
        }
        // Serialize cancellation with publication; a cancelled package cannot appear later.
        let _publication = self.run.lock().unwrap();
        if self.cancelled.load(Ordering::Acquire) {
            return Err(PackError::cancelled());
        }
        (spec.hooks.prepare)(staging)?;
        let mut aside = None;
        if final_dir.exists() {
            // Never merge mixed revisions: the damaged folder moves aside, and goes once the new one is in place.
            let to = self.root.join(format!(".invalid-{}", uuid::Uuid::new_v4()));
            std::fs::rename(&final_dir, &to).map_err(|e| PackError::other(e.to_string()))?;
            aside = Some(to);
        }
        if let Err(e) = std::fs::rename(staging, &final_dir) {
            if let Some(to) = &aside {
                let _ = std::fs::rename(to, &final_dir);
            }
            return Err(PackError::other(e.to_string()));
        }
        if let Some(to) = aside {
            let _ = std::fs::remove_dir_all(to);
        }
        drop(_publication);
        self.write_marker();
        Ok(())
    }

    fn http_fetch(&self, asset: &PackFile, dest: &Path) -> Result<(), PackError> {
        let override_base = self.base_override.lock().unwrap().clone();
        let mut bases: Vec<String> = vec![override_base.clone().unwrap_or_else(|| self.spec.base_url.to_string())];
        match &override_base {
            None => bases.extend(self.spec.fallback_base_url.map(str::to_string)),
            Some(_) => bases.extend(self.fallback_override.lock().unwrap().clone()),
        }
        let mut last = None;
        for base in &bases {
            match self.http_fetch_from(base, override_base.is_some(), asset, dest) {
                Ok(()) => return Ok(()),
                // these end the install whichever host would answer
                Err(e) if matches!(e.kind, PackErrorKind::Cancelled | PackErrorKind::DiskFull) => return Err(e),
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| PackError::other("no source for the pack")))
    }

    fn http_fetch_from(&self, base: &str, plain_http: bool, asset: &PackFile, dest: &Path) -> Result<(), PackError> {
        let url = format!("{base}/{}", asset.path);
        // Blocking Response::read applies this per read, bounding a stalled
        // transfer without imposing a minute-long whole-model deadline.
        let client = reqwest::blocking::Client::builder()
            .https_only(!plain_http)
            .connect_timeout(Duration::from_secs(30))
            .timeout(Duration::from_secs(60))
            .build()
            .map_err(|e| PackError::other(e.to_string()))?;
        let mut part = dest.to_path_buf().into_os_string();
        part.push(".part");
        let part = PathBuf::from(part);
        for _attempt in 0..2 {
            let mut resume = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
            if resume > asset.size {
                let _ = std::fs::remove_file(&part);
                resume = 0;
            }
            let mut digests = Digests::new(asset);
            if resume > 0 {
                let mut f = std::fs::File::open(&part).map_err(|e| PackError::other(e.to_string()))?;
                let mut buf = [0u8; 65536];
                loop {
                    let n = f.read(&mut buf).map_err(|e| PackError::other(e.to_string()))?;
                    if n == 0 {
                        break;
                    }
                    digests.update(&buf[..n]);
                }
            }
            let mut file;
            let mut response;
            if resume == asset.size {
                // a complete file that was never checked: hash it, no request
                response = None;
                file = None;
            } else {
                let mut request = client.get(&url);
                if resume > 0 {
                    request = request.header("Range", format!("bytes={resume}-"));
                }
                let r = request.send().map_err(|e| {
                    PackError::new(PackErrorKind::Offline, e.to_string())
                })?;
                let status = r.status();
                if status.as_u16() == 416 {
                    let _ = std::fs::remove_file(&part);
                    continue;
                }
                if !status.is_success() {
                    let kind = if status.is_server_error() { PackErrorKind::Offline } else { PackErrorKind::Other };
                    return Err(PackError::new(kind, format!("{} answered {status}", self.spec.source_host)));
                }
                let partial = status.as_u16() == 206;
                if partial {
                    let ok = r
                        .headers()
                        .get("content-range")
                        .and_then(|v| v.to_str().ok())
                        .is_some_and(|v| v.starts_with(&format!("bytes {resume}-")));
                    if !ok {
                        let _ = std::fs::remove_file(&part);
                        continue;
                    }
                } else if resume > 0 {
                    // the server ignored the range: start the file again
                    self.done.fetch_sub(resume.min(self.done.load(Ordering::Acquire)), Ordering::AcqRel);
                    resume = 0;
                    digests = Digests::new(asset);
                }
                let expect = asset.size - resume;
                if r.content_length().is_some_and(|len| len != expect) {
                    return Err(PackError::other(format!("Unexpected length for {}", asset.path)));
                }
                let mut options = std::fs::OpenOptions::new();
                options.write(true);
                if resume > 0 {
                    options.append(true);
                } else {
                    options.create(true).truncate(true);
                }
                file = Some(options.open(&part).map_err(write_error)?);
                response = Some(r);
            }
            let outcome = match (&mut response, &mut file) {
                (Some(r), Some(f)) => {
                    let mut bar = |n: u64| self.bump(n);
                    let result = copy_digests(&mut Cancellable { reader: r, cancelled: &self.cancelled }, f, asset, digests, true, &mut bar);
                    let _ = f.sync_all();
                    result
                }
                _ => {
                    let mut empty: &[u8] = &[];
                    copy_digests(&mut empty, &mut std::io::sink(), asset, digests, false, &mut |_| {})
                }
            };
            return match outcome {
                Ok(()) => std::fs::rename(&part, dest).map_err(|e| PackError::other(e.to_string())),
                Err(e) if e.kind == PackErrorKind::HashMismatch => {
                    // never kept: it would fail the same way on every resume
                    let _ = std::fs::remove_file(&part);
                    Err(e)
                }
                Err(e) => Err(e),
            };
        }
        Err(PackError::other(format!("{} could not be resumed", asset.path)))
    }
    fn bump(&self, n: u64) {
        self.done.fetch_add(n, Ordering::AcqRel);
        let due = {
            let mut last = self.last_emit.lock().unwrap();
            let due = last.is_none_or(|t| t.elapsed() >= Duration::from_millis(250));
            if due {
                *last = Some(Instant::now());
            }
            due
        };
        if due && self.events.lock().unwrap().is_some() {
            if let Some(p) = self.info_progress() {
                self.emit(PackEvent::Progress(p));
            }
        }
    }
    fn info_progress(&self) -> Option<PackProgress> {
        let run = self.run.lock().unwrap();
        if !run.downloading {
            return None;
        }
        let (started, from, phase) = (run.started, run.started_done, run.phase);
        drop(run);
        let done = self.done.load(Ordering::Acquire);
        let total = self.total.load(Ordering::Acquire).max(1);
        let elapsed = started.map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
        let rate = if elapsed > 0.5 { done.saturating_sub(from) as f64 / elapsed } else { 0.0 };
        Some(PackProgress {
            id: self.spec.id,
            done,
            total,
            bytes_per_sec: rate,
            eta_s: (rate > 1.0).then(|| (total.saturating_sub(done) as f64 / rate).ceil() as u32),
            phase: phase.unwrap_or(PackPhase::Download),
        })
    }

    // ---- check, remove ---------------------------------------------------

    /// Hashes every installed file. Records the time on success; on a failure the
    /// pack is Needs repair (nothing is deleted, and nothing downloads).
    pub fn check_files(&self) -> Result<PackCheck, String> {
        {
            let mut run = self.run.lock().unwrap();
            if run.downloading {
                return Err("The model is downloading".into());
            }
            if run.checking {
                return Err("The model files are already being checked".into());
            }
            run.checking = true;
        }
        self.emit_state();
        let dir = self.final_dir();
        let mut failed = Vec::new();
        let mut ok = 0u32;
        let mut last_error = None;
        for a in self.spec.files {
            match verify_file(&dir.join(a.path), a) {
                Ok(()) => ok += 1,
                Err(e) => {
                    failed.push(a.path.to_string());
                    last_error = Some(e);
                }
            }
        }
        {
            let mut run = self.run.lock().unwrap();
            run.checking = false;
            if failed.is_empty() {
                run.verified = true;
                run.error = None;
            } else {
                run.verified = false;
                run.error = last_error.map(|e| PackError::new(
                    PackErrorKind::HashMismatch,
                    format!("{} of {} files failed their check: {}", failed.len(), self.spec.files.len(), e.detail),
                ));
            }
        }
        if failed.is_empty() {
            self.write_marker();
        } else {
            let _ = std::fs::remove_file(self.marker());
        }
        self.emit_state();
        Ok(PackCheck { files: self.spec.files.len() as u32, files_ok: ok, failed })
    }

    /// An install with no record of a check (made before checks were recorded)
    /// is checked once, in the background, so it can say "verified".
    pub fn ensure_checked(self: &Arc<Self>) {
        let wanted = {
            let run = self.run.lock().unwrap();
            !run.downloading && !run.checking && run.error.is_none()
        };
        if wanted
            && self.final_dir().is_dir()
            && self.files_present(&self.final_dir()) as usize == self.spec.files.len()
            && self.verified_at().is_none()
        {
            let store = Arc::clone(self);
            std::thread::spawn(move || {
                let _ = store.check_files();
            });
        }
    }

    /// Deletes the installed package and the kept partial files, or only the
    /// partial files. Refused while a download or a check runs.
    pub fn remove(&self, partial_only: bool) -> Result<(), String> {
        let mut run = self.run.lock().unwrap();
        if run.downloading || run.checking {
            return Err("Cancel the download before removing the model".into());
        }
        if !partial_only {
            let dir = self.final_dir();
            if dir.exists() {
                std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?;
            }
            let _ = std::fs::remove_file(self.marker());
            run.verified = false;
        }
        run.error = None;
        drop(run);
        let _ = std::fs::remove_dir_all(self.partial_dir());
        if let Ok(read) = std::fs::read_dir(&self.root) {
            for e in read.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if n.starts_with(".invalid-") || n.starts_with(".download-") {
                    let _ = std::fs::remove_dir_all(e.path());
                }
            }
        }
        self.emit_state();
        Ok(())
    }

    #[cfg(test)]
    pub fn set_test_seams(&self, base: Option<String>, free: Option<u64>) {
        *self.base_override.lock().unwrap() = base;
        *self.free_override.lock().unwrap() = free;
    }
    #[cfg(test)]
    pub fn set_test_fallback(&self, base: Option<String>) {
        *self.fallback_override.lock().unwrap() = base;
    }
}

/// Every pack, in the order the page lists them.
pub struct PackManager {
    packs: Vec<Arc<PackStore>>,
}
impl PackManager {
    pub fn new(packs: Vec<Arc<PackStore>>) -> Self {
        PackManager { packs }
    }
    pub fn get(&self, id: PackId) -> Option<&Arc<PackStore>> {
        self.packs.iter().find(|p| p.spec.id == id)
    }
    pub fn list(&self) -> Vec<PackInfo> {
        self.packs.iter().map(|p| p.info()).collect()
    }
    pub fn stores(&self) -> &[Arc<PackStore>] {
        &self.packs
    }
    pub fn set_events(&self, events: Events) {
        for p in &self.packs {
            p.set_events(events.clone());
        }
    }
    pub fn cancel_all(&self) {
        for p in &self.packs {
            p.cancel();
        }
    }
}

#[cfg(test)]
mod tests;
