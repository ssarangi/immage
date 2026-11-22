// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! Catalog sources: a folder tree on a VOLUME. A source is identified by the
//! volume it is on (the server and share of a network mount, the file system
//! UUID of a drive), never by the path the volume happens to be mounted at, so
//! a share remounted somewhere else is the same source and relinking is one
//! UPDATE. The mount table is read from `/proc/self/mountinfo`, which says what
//! `/proc/self/mounts` does not: where in the file system a mount is rooted, and
//! the order of stacked mounts, which is how an idle systemd automount
//! (`autofs`) is told from the share behind it once it is mounted.
//!
//! Everything that decides is a pure function of a [`MountTable`], so tests use
//! real mountinfo text instead of the machine's mounts.

use crate::catalog_db::{now_s, CatalogDb};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

// ---- the mount table ------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountEntry {
    pub mount_id: u32,
    pub parent_id: u32,
    /// The directory of the file system this mount shows (a bind mount or a
    /// btrfs subvolume does not show the root).
    pub root: String,
    pub mount_point: String,
    pub options: String,
    pub fs_type: String,
    pub source: String,
    pub super_options: String,
}

impl MountEntry {
    /// An option's value from the mount or super options: `vers` in `vers=3.0`.
    #[allow(dead_code)] // for the speed test's protocol line later
    pub fn option(&self, key: &str) -> Option<&str> {
        self.options.split(',').chain(self.super_options.split(',')).find_map(|o| {
            let (k, v) = o.split_once('=')?;
            (k == key).then_some(v)
        })
    }
}

#[derive(Debug, Default, Clone)]
pub struct MountTable {
    pub entries: Vec<MountEntry>,
}

/// `\040` and the like, as the kernel writes spaces and the like.
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 3 < b.len() && b[i + 1..i + 4].iter().all(|c| (b'0'..=b'7').contains(c)) {
            out.push(u8::from_str_radix(&s[i + 1..i + 4], 8).unwrap_or(b'?'));
            i += 4;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

impl MountTable {
    /// `/proc/self/mountinfo`: `36 35 98:0 /root /mnt rw,noatime master:1 - ext3 /dev/root rw`.
    pub fn parse(text: &str) -> Self {
        let mut entries = Vec::new();
        for line in text.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            let Some(dash) = f.iter().position(|x| *x == "-") else { continue };
            if dash < 6 || f.len() < dash + 3 {
                continue;
            }
            let (Ok(mount_id), Ok(parent_id)) = (f[0].parse(), f[1].parse()) else { continue };
            entries.push(MountEntry {
                mount_id,
                parent_id,
                root: unescape(f[3]),
                mount_point: unescape(f[4]),
                options: f[5].to_string(),
                fs_type: f[dash + 1].to_string(),
                source: unescape(f[dash + 2]),
                super_options: f.get(dash + 3).map(|s| s.to_string()).unwrap_or_default(),
            });
        }
        Self { entries }
    }

    pub fn current() -> Self {
        #[cfg(target_os = "linux")]
        {
            Self::parse(&std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default())
        }
        #[cfg(not(target_os = "linux"))]
        {
            Self::default()
        }
    }

    /// The mounts whose mount point holds `path`, longest first, and among
    /// those at the same point the one mounted last (stacked mounts: the top
    /// one is what a path resolves to).
    pub fn covering(&self, path: &Path) -> Vec<&MountEntry> {
        let mut v: Vec<(usize, usize, &MountEntry)> = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, e)| path.starts_with(&e.mount_point))
            .map(|(i, e)| (e.mount_point.len(), i, e))
            .collect();
        v.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        v.into_iter().map(|(_, _, e)| e).collect()
    }

    /// What the path is on right now.
    pub fn top_at(&self, path: &Path) -> Option<&MountEntry> {
        self.covering(path).into_iter().next()
    }

    /// The real (not autofs) mount at exactly this mount point, topmost.
    pub fn real_at_point(&self, point: &str) -> Option<&MountEntry> {
        self.entries.iter().rev().find(|e| e.mount_point == point && e.fs_type != "autofs")
    }

    /// Every topmost real mount: what each mount point shows now.
    pub fn real_mounts(&self) -> Vec<&MountEntry> {
        let mut seen: HashMap<&str, &MountEntry> = HashMap::new();
        for e in &self.entries {
            if e.fs_type == "autofs" {
                // an autofs below a real mount at the same point is hidden by it, and vice versa
                seen.remove(e.mount_point.as_str());
                continue;
            }
            seen.insert(e.mount_point.as_str(), e);
        }
        // a real mount covered later by an autofs of the same point was removed above; that is right
        let mut v: Vec<&MountEntry> = seen.into_values().collect();
        v.sort_by_key(|e| e.mount_id);
        v
    }

    pub fn idle_autofs_at(&self, path: &Path) -> Option<&MountEntry> {
        self.top_at(path).filter(|e| e.fs_type == "autofs")
    }
}

// ---- volume identity ------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct VolumeId {
    /// Stable across remounts at another path: `smb:host/share`, `nfs:host:/export`, `uuid:...`.
    pub key: String,
    /// `nas`, `drive` or `local`.
    pub kind: String,
    pub fs_type: String,
    pub host: Option<String>,
    /// First 10 hex digits of sha256(host): what logs and support bundles show.
    pub host_hash: Option<String>,
    pub share: Option<String>,
    /// Where this mount sits inside the volume ('' for the volume root).
    pub subpath: String,
    pub network: bool,
}

pub fn host_hash(host: &str) -> String {
    let d = Sha256::digest(host.to_ascii_lowercase().as_bytes());
    d.iter().take(5).map(|b| format!("{b:02x}")).collect()
}

fn components(p: &str) -> Vec<&str> {
    p.split(['/', '\\']).filter(|c| !c.is_empty() && *c != ".").collect()
}

pub fn join_rel(a: &str, b: &str) -> String {
    components(a).into_iter().chain(components(b)).collect::<Vec<_>>().join("/")
}

/// Whether `child` is `parent` or below it, by whole components.
pub fn rel_within(child: &str, parent: &str) -> bool {
    let (c, p) = (components(child), components(parent));
    c.len() >= p.len() && c[..p.len()] == p[..]
}

/// `rel` minus the leading `prefix` components, if it has them.
pub fn rel_strip(rel: &str, prefix: &str) -> Option<String> {
    let (c, p) = (components(rel), components(prefix));
    (c.len() >= p.len() && c[..p.len()] == p[..]).then(|| c[p.len()..].join("/"))
}

pub fn is_network_class(fs: &str) -> bool {
    crate::volumes::is_network_fs(fs)
}

/// The volume behind a real mount. `uuid_of` maps a device path to its file
/// system UUID (`/dev/disk/by-uuid`); injectable so tests need no devices.
pub fn identity_of(e: &MountEntry, uuid_of: &dyn Fn(&str) -> Option<String>) -> VolumeId {
    let fs = e.fs_type.as_str();
    let network = is_network_class(fs);
    let named = |host: &str, share: Option<String>, subpath: String, key: String| VolumeId {
        key,
        kind: "nas".into(),
        fs_type: fs.to_string(),
        host: Some(host.to_string()),
        host_hash: Some(host_hash(host)),
        share,
        subpath,
        network: true,
    };
    if matches!(fs, "cifs" | "smb3" | "smbfs") {
        // //[user@]host/share[/prefix/path]
        let s = e.source.replace('\\', "/");
        let rest = s.trim_start_matches('/');
        let mut parts = rest.split('/').filter(|p| !p.is_empty());
        let host = parts.next().unwrap_or("").rsplit('@').next().unwrap_or("").to_ascii_lowercase();
        let share = parts.next().unwrap_or("").to_string();
        let subpath = parts.collect::<Vec<_>>().join("/");
        let key = format!("smb:{host}/{}", share.to_ascii_lowercase());
        return named(&host, Some(share), subpath, key);
    }
    if fs.starts_with("nfs") || fs == "fuse.sshfs" || fs == "sshfs" {
        // host:/export (user@host:/path for sshfs)
        let (host, export) = e.source.split_once(':').unwrap_or(("", e.source.as_str()));
        let host = host.rsplit('@').next().unwrap_or("").to_ascii_lowercase();
        let proto = if fs.starts_with("nfs") { "nfs" } else { "sshfs" };
        let key = format!("{proto}:{host}:{}", export.trim_end_matches('/'));
        return named(&host, Some(export.to_string()), String::new(), key);
    }
    if network {
        // rclone, gvfs, mergerfs, ceph...: the source string is the best name there is
        return VolumeId {
            key: format!("{fs}:{}", e.source),
            kind: "nas".into(),
            fs_type: fs.to_string(),
            host: None,
            host_hash: None,
            share: Some(e.source.clone()),
            subpath: String::new(),
            network: true,
        };
    }
    // a local file system: its UUID if the source is a device that has one; a
    // bind mount or subvolume shows `root` of it
    let uuid = if e.source.starts_with("/dev/") { uuid_of(&e.source) } else { None };
    let key = match uuid {
        Some(u) => format!("uuid:{u}"),
        None if e.source.starts_with("/dev/") => format!("dev:{}", e.source),
        None => format!("{fs}:{}:{}", e.source, e.mount_point),
    };
    let mp = e.mount_point.as_str();
    let kind = if mp == "/" || mp.starts_with("/home") {
        "local"
    } else if ["/media", "/run/media", "/mnt", "/Volumes"].iter().any(|p| mp.starts_with(p)) {
        "drive"
    } else {
        "local"
    };
    VolumeId {
        key,
        kind: kind.into(),
        fs_type: fs.to_string(),
        host: None,
        host_hash: None,
        share: None,
        subpath: components(&e.root).join("/"),
        network: false,
    }
}

/// The device's file system UUID from `/dev/disk/by-uuid`.
pub fn system_uuid_of(device: &str) -> Option<String> {
    let real = std::fs::canonicalize(device).ok()?;
    for entry in std::fs::read_dir("/dev/disk/by-uuid").ok()?.flatten() {
        if std::fs::canonicalize(entry.path()).ok().as_deref() == Some(real.as_path()) {
            return Some(entry.file_name().to_string_lossy().into_owned());
        }
    }
    None
}

// ---- a path resolved against the table ------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum Resolved {
    /// On a mounted volume.
    Mounted { entry: MountEntry, id: VolumeId, rel_in_volume: String },
    /// Under an automount with nothing mounted: touching it would mount it.
    Asleep { mount_point: String },
    Unknown,
}

pub fn resolve(table: &MountTable, path: &Path, uuid_of: &dyn Fn(&str) -> Option<String>) -> Resolved {
    let Some(top) = table.top_at(path) else { return Resolved::Unknown };
    if top.fs_type == "autofs" {
        return Resolved::Asleep { mount_point: top.mount_point.clone() };
    }
    let id = identity_of(top, uuid_of);
    let under = path.strip_prefix(&top.mount_point).unwrap_or(Path::new(""));
    let rel = join_rel(&id.subpath, &under.to_string_lossy());
    Resolved::Mounted { entry: top.clone(), id, rel_in_volume: rel }
}

// ---- sources ---------------------------------------------------------------

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct Source {
    pub id: i64,
    pub name: String,
    pub kind: String,
    pub volume_key: String,
    pub fs_type: String,
    pub host: Option<String>,
    pub host_hash: Option<String>,
    pub share: Option<String>,
    pub mount_path: Option<String>,
    pub rel_root: String,
    pub enabled: bool,
    pub treat_as_network: Option<bool>,
    pub scan_policy: String,
    pub include_subfolders: bool,
    pub dir_skip: bool,
    pub state: String,
    pub error: Option<String>,
    pub last_scan_start: Option<i64>,
    pub last_scan_end: Option<i64>,
    pub last_ok: Option<i64>,
    pub created_at: i64,
    pub photos: i64,
    pub missing: i64,
}

const SOURCE_COLS: &str = "s.id,s.name,s.kind,s.volume_key,s.fs_type,s.host,s.host_hash,s.share,s.mount_path,s.rel_root,
    s.enabled,s.treat_as_network,s.scan_policy,s.include_subfolders,s.dir_skip,s.state,s.error,
    s.last_scan_start,s.last_scan_end,s.last_ok,s.created_at,
    (SELECT COUNT(*) FROM photos p WHERE p.source_id=s.id AND p.status='present'),
    (SELECT COUNT(*) FROM photos p WHERE p.source_id=s.id AND p.status='missing')";

fn source_row(r: &rusqlite::Row) -> rusqlite::Result<Source> {
    Ok(Source {
        id: r.get(0)?,
        name: r.get(1)?,
        kind: r.get(2)?,
        volume_key: r.get(3)?,
        fs_type: r.get(4)?,
        host: r.get(5)?,
        host_hash: r.get(6)?,
        share: r.get(7)?,
        mount_path: r.get(8)?,
        rel_root: r.get(9)?,
        enabled: r.get::<_, i64>(10)? != 0,
        treat_as_network: r.get::<_, Option<i64>>(11)?.map(|v| v != 0),
        scan_policy: r.get(12)?,
        include_subfolders: r.get::<_, i64>(13)? != 0,
        dir_skip: r.get::<_, i64>(14)? != 0,
        state: r.get(15)?,
        error: r.get(16)?,
        last_scan_start: r.get(17)?,
        last_scan_end: r.get(18)?,
        last_ok: r.get(19)?,
        created_at: r.get(20)?,
        photos: r.get(21)?,
        missing: r.get(22)?,
    })
}

pub fn list_sources(c: &Connection) -> Result<Vec<Source>, String> {
    let mut st = c
        .prepare(&format!("SELECT {SOURCE_COLS} FROM sources s ORDER BY s.id"))
        .map_err(|e| e.to_string())?;
    let rows = st.query_map([], source_row).map_err(|e| e.to_string())?;
    rows.collect::<Result<_, _>>().map_err(|e| e.to_string())
}

pub fn get_source(c: &Connection, id: i64) -> Result<Option<Source>, String> {
    c.query_row(&format!("SELECT {SOURCE_COLS} FROM sources s WHERE s.id=?1"), [id], source_row)
        .optional()
        .map_err(|e| e.to_string())
}

/// A source by id, or by exact name (case-insensitive) when `key` is not a number.
pub fn find_source(c: &Connection, key: &str) -> Result<Option<Source>, String> {
    if let Ok(id) = key.parse::<i64>() {
        if let Some(s) = get_source(c, id)? {
            return Ok(Some(s));
        }
    }
    Ok(list_sources(c)?.into_iter().find(|s| s.name.eq_ignore_ascii_case(key)))
}

/// Whether the source reads a mount as a network: its own override first.
pub fn treats_as_network(s: &Source, fs_is_network: bool) -> bool {
    s.treat_as_network.unwrap_or(fs_is_network)
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, specta::Type)]
#[serde(tag = "state", content = "detail")]
pub enum Availability {
    /// Mounted and reachable: where the source's root is now.
    Online { mount_point: String, abs_root: String },
    /// Behind an idle automount: not known to be away, not touched.
    Asleep { mount_point: String },
    Offline { reason: String },
}

impl Availability {
    pub fn word(&self) -> &'static str {
        match self {
            Availability::Online { .. } => "online",
            Availability::Asleep { .. } => "asleep",
            Availability::Offline { .. } => "offline",
        }
    }
}

/// Where a source is now. Never touches the file system.
pub fn availability(table: &MountTable, s: &Source, uuid_of: &dyn Fn(&str) -> Option<String>) -> Availability {
    if let Some(point) = s.volume_key.strip_prefix("pending:") {
        // added while offline: its identity is learned when something mounts there
        return match table.real_at_point(point) {
            Some(_) => Availability::Online { mount_point: point.to_string(), abs_root: join_path(point, &s.rel_root) },
            None if table.idle_autofs_at(Path::new(point)).is_some() => Availability::Asleep { mount_point: point.to_string() },
            None => Availability::Offline { reason: "not mounted".into() },
        };
    }
    let mut best: Option<(&MountEntry, VolumeId)> = None;
    for e in table.real_mounts() {
        let id = identity_of(e, uuid_of);
        if id.key != s.volume_key || !rel_within(&s.rel_root, &id.subpath) {
            continue;
        }
        let better = match &best {
            None => true,
            Some((b, bid)) => {
                // the mount nearest the source's root, then where it was before
                id.subpath.len() > bid.subpath.len()
                    || (id.subpath.len() == bid.subpath.len() && s.mount_path.as_deref() == Some(e.mount_point.as_str()) && b.mount_point != e.mount_point)
            }
        };
        if better {
            best = Some((e, id));
        }
    }
    if let Some((e, id)) = best {
        let under = rel_strip(&s.rel_root, &id.subpath).unwrap_or_default();
        return Availability::Online { mount_point: e.mount_point.clone(), abs_root: join_path(&e.mount_point, &under) };
    }
    if let Some(old) = &s.mount_path {
        if table.idle_autofs_at(Path::new(old)).is_some() {
            return Availability::Asleep { mount_point: old.clone() };
        }
    }
    // a volume that was mounted at the same path before, with nothing there now
    Availability::Offline { reason: "not mounted".into() }
}

pub fn join_path(base: &str, rel: &str) -> String {
    let mut p = PathBuf::from(base);
    for c in components(rel) {
        p.push(c);
    }
    p.to_string_lossy().into_owned()
}

// ---- adding -----------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AddOptions {
    pub name: Option<String>,
    pub policy: String,
    pub include_subfolders: bool,
    pub allow_offline: bool,
    pub treat_as_network: Option<bool>,
}

impl Default for AddOptions {
    fn default() -> Self {
        Self { name: None, policy: "launch".into(), include_subfolders: true, allow_offline: false, treat_as_network: None }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AddError {
    /// The path is not reachable and `allow_offline` was not given.
    Offline(String),
    /// An existing source (id) already covers part of this one.
    Overlap(i64),
    Usage(String),
    Db(String),
}

impl std::fmt::Display for AddError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Offline(m) | Self::Usage(m) | Self::Db(m) => write!(f, "{m}"),
            Self::Overlap(id) => write!(f, "That folder overlaps source {id}."),
        }
    }
}

/// What `add_source` would store, before the file system is asked anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSource {
    pub volume_key: String,
    pub kind: String,
    pub fs_type: String,
    pub host: Option<String>,
    pub host_hash: Option<String>,
    pub share: Option<String>,
    pub mount_path: String,
    pub rel_root: String,
    pub pending: bool,
}

/// The pure part of adding: which volume, where on it. `reachable` says the
/// caller has seen the folder answer (a path under an automount is reachable
/// once the automount has been woken, which then shows in `table`).
pub fn plan_add(
    table: &MountTable,
    path: &Path,
    reachable: bool,
    known: &[Source],
    opts: &AddOptions,
    uuid_of: &dyn Fn(&str) -> Option<String>,
) -> Result<NewSource, AddError> {
    if !path.is_absolute() {
        return Err(AddError::Usage("Give the folder's full path.".into()));
    }
    let planned = match resolve(table, path, uuid_of) {
        Resolved::Mounted { entry, id, rel_in_volume } => NewSource {
            volume_key: id.key,
            kind: id.kind,
            fs_type: id.fs_type,
            host: id.host,
            host_hash: id.host_hash,
            share: id.share,
            mount_path: entry.mount_point,
            rel_root: rel_in_volume,
            pending: false,
        },
        Resolved::Asleep { mount_point } => {
            // nothing is mounted, so which volume is behind it is not known:
            // the source waits, and reconcile() gives it its identity once
            // something mounts there. its rel_root is relative to the mount point
            let rel = path.strip_prefix(&mount_point).map(|p| join_rel("", &p.to_string_lossy())).unwrap_or_default();
            NewSource {
                volume_key: format!("pending:{mount_point}"),
                kind: "nas".into(),
                fs_type: "autofs".into(),
                host: None,
                host_hash: None,
                share: None,
                mount_path: mount_point,
                rel_root: rel,
                pending: true,
            }
        }
        Resolved::Unknown => return Err(AddError::Usage(format!("{} is not on any known mount.", path.display()))),
    };
    if !reachable && !opts.allow_offline {
        return Err(AddError::Offline(format!(
            "{} is not reachable. Use --allow-offline to add it anyway.",
            path.display()
        )));
    }
    for s in known {
        let same_volume = s.volume_key == planned.volume_key
            || (planned.pending && s.mount_path.as_deref() == Some(planned.mount_path.as_str()))
            || (s.volume_key.starts_with("pending:") && s.mount_path.as_deref() == Some(planned.mount_path.as_str()));
        if !same_volume {
            continue;
        }
        let (new_in_old, old_in_new) = (rel_within(&planned.rel_root, &s.rel_root), rel_within(&s.rel_root, &planned.rel_root));
        let equal = new_in_old && old_in_new;
        // a source that does not go into subfolders leaves a deeper folder to another
        let overlaps = equal
            || (new_in_old && s.include_subfolders)
            || (old_in_new && opts.include_subfolders);
        if overlaps {
            return Err(AddError::Overlap(s.id));
        }
    }
    Ok(planned)
}

fn default_name(path: &Path, rel_root: &str, share: &Option<String>) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .or_else(|| components(rel_root).last().map(|s| s.to_string()))
        .or_else(|| share.clone())
        .unwrap_or_else(|| "Source".into())
}

/// Adds a source for `path`. The folder is asked (with a deadline) whether it
/// answers; an idle automount is woken for it, because adding is an explicit act.
pub fn add_source(db: &CatalogDb, path: &Path, opts: &AddOptions) -> Result<Source, AddError> {
    let norm = normalize(path);
    let mut table = MountTable::current();
    let mut reachable = false;
    // the file system's real answer, at most a few seconds
    if crate::volumes::answering(&norm) && norm.is_dir() {
        reachable = true;
        // the automount may just have mounted
        table = MountTable::current();
    }
    let norm = if reachable { std::fs::canonicalize(&norm).unwrap_or(norm) } else { norm };
    let conn = db.read().map_err(|e| AddError::Db(e.to_string()))?;
    let known = list_sources(&conn).map_err(AddError::Db)?;
    let planned = plan_add(&table, &norm, reachable, &known, opts, &system_uuid_of)?;
    let name = opts.name.clone().unwrap_or_else(|| default_name(&norm, &planned.rel_root, &planned.share));
    if name.trim().is_empty() {
        return Err(AddError::Usage("A source needs a name.".into()));
    }
    let (opts, p) = (opts.clone(), planned);
    let id = db
        .exec(move |c| {
            c.execute(
                "INSERT INTO sources(name,kind,volume_key,fs_type,host,host_hash,share,mount_path,rel_root,scan_policy,
                                     include_subfolders,treat_as_network,state,created_at)
                 VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'never_indexed',?13)",
                params![
                    name,
                    p.kind,
                    p.volume_key,
                    p.fs_type,
                    p.host,
                    p.host_hash,
                    p.share,
                    p.mount_path,
                    p.rel_root,
                    opts.policy,
                    opts.include_subfolders as i64,
                    opts.treat_as_network.map(|b| b as i64),
                    now_s()
                ],
            )
            .map_err(|e| e.to_string())?;
            Ok(c.last_insert_rowid())
        })
        .map_err(AddError::Db)?;
    let conn = db.read().map_err(|e| AddError::Db(e.to_string()))?;
    get_source(&conn, id).map_err(AddError::Db)?.ok_or_else(|| AddError::Db("the new source vanished".into()))
}

/// An absolute path with `.` and `..` resolved lexically.
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[derive(Debug, Clone, Default)]
pub struct SourceUpdate {
    pub name: Option<String>,
    pub policy: Option<String>,
    pub enabled: Option<bool>,
    /// `Some(None)` clears the override.
    pub treat_as_network: Option<Option<bool>>,
    pub dir_skip: Option<bool>,
}

pub fn update_source(db: &CatalogDb, id: i64, u: SourceUpdate) -> Result<(), String> {
    db.exec(move |c| {
        let tx = c.transaction().map_err(|e| e.to_string())?;
        if let Some(n) = &u.name {
            tx.execute("UPDATE sources SET name=?2 WHERE id=?1", params![id, n]).map_err(|e| e.to_string())?;
        }
        if let Some(p) = &u.policy {
            tx.execute("UPDATE sources SET scan_policy=?2 WHERE id=?1", params![id, p]).map_err(|e| e.to_string())?;
        }
        if let Some(e) = u.enabled {
            tx.execute("UPDATE sources SET enabled=?2 WHERE id=?1", params![id, e as i64]).map_err(|e| e.to_string())?;
        }
        if let Some(t) = u.treat_as_network {
            tx.execute("UPDATE sources SET treat_as_network=?2 WHERE id=?1", params![id, t.map(|b| b as i64)])
                .map_err(|e| e.to_string())?;
        }
        if let Some(d) = u.dir_skip {
            tx.execute("UPDATE sources SET dir_skip=?2 WHERE id=?1", params![id, d as i64]).map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())
    })
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct RemovePreview {
    pub photos: i64,
    pub dirs: i64,
}

pub fn remove_preview(c: &Connection, id: i64) -> Result<RemovePreview, String> {
    let n = |sql: &str| c.query_row(sql, [id], |r| r.get::<_, i64>(0)).map_err(|e| e.to_string());
    Ok(RemovePreview {
        photos: n("SELECT COUNT(*) FROM photos WHERE source_id=?1")?,
        dirs: n("SELECT COUNT(*) FROM dirs WHERE source_id=?1")?,
    })
}

/// Removes the source and, by cascade, everything the catalog knows under it.
/// Never touches a file. (There are no annotations in schema v1 to keep.)
pub fn remove_source(db: &CatalogDb, id: i64) -> Result<(), String> {
    let done = db.exec(move |c| {
        // scan_gone and scan_notes carry no foreign key (they are run-scoped)
        let tx = c.transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM scan_gone WHERE run_id IN (SELECT id FROM scan_runs WHERE source_id=?1)", [id]).map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM scan_notes WHERE source_id=?1", [id]).map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM sources WHERE id=?1", [id]).map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())
    });
    db.touch_shape(id);
    done
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct SourceStatus {
    pub source: Source,
    pub availability: Availability,
}

/// Brings every source up to date with the mount table: a pending source whose
/// volume has appeared learns its identity; a volume mounted somewhere else is
/// relinked with one UPDATE. Returns each source's availability.
pub fn reconcile(db: &CatalogDb, table: &MountTable, uuid_of: &dyn Fn(&str) -> Option<String>) -> Result<Vec<SourceStatus>, String> {
    let sources = list_sources(&db.read().map_err(|e| e.to_string())?)?;
    let mut out = Vec::new();
    for s in sources {
        let mut s = s;
        if let Some(point) = s.volume_key.strip_prefix("pending:").map(str::to_string) {
            if let Some(e) = table.real_at_point(&point) {
                let id = identity_of(e, uuid_of);
                let rel = join_rel(&id.subpath, &s.rel_root);
                let (sid, key, kind, fs, host, hash, share, rel2) =
                    (s.id, id.key.clone(), id.kind.clone(), id.fs_type.clone(), id.host.clone(), id.host_hash.clone(), id.share.clone(), rel.clone());
                db.exec(move |c| {
                    c.execute(
                        "UPDATE sources SET volume_key=?2, kind=?3, fs_type=?4, host=?5, host_hash=?6, share=?7, rel_root=?8 WHERE id=?1",
                        params![sid, key, kind, fs, host, hash, share, rel2],
                    )
                    .map(|_| ())
                    .map_err(|e| e.to_string())
                })?;
                s.volume_key = id.key;
                s.kind = id.kind;
                s.fs_type = id.fs_type;
                s.host = id.host;
                s.host_hash = id.host_hash;
                s.share = id.share;
                s.rel_root = rel;
            }
        }
        let a = availability(table, &s, uuid_of);
        if let Availability::Online { mount_point, .. } = &a {
            if s.mount_path.as_deref() != Some(mount_point.as_str()) {
                // remounted elsewhere: one UPDATE for the volume
                let (key, mp) = (s.volume_key.clone(), mount_point.clone());
                db.exec(move |c| {
                    c.execute("UPDATE sources SET mount_path=?2 WHERE volume_key=?1 AND mount_path IS NOT ?2", params![key, mp])
                        .map(|_| ())
                        .map_err(|e| e.to_string())
                })?;
                s.mount_path = Some(mount_point.clone());
            }
        }
        out.push(SourceStatus { source: s, availability: a });
    }
    Ok(out)
}

/// Explicit relink: point a source at `new_path`. With `dry_run` only checks
/// that up to `SAMPLE` known photos exist there. `exists` is the file check.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct RelinkCheck {
    pub sample_matched: i64,
    pub of: i64,
}

const RELINK_SAMPLE: i64 = 20;

pub fn relink(
    db: &CatalogDb,
    id: i64,
    new_path: &Path,
    dry_run: bool,
    table: &MountTable,
    exists: &dyn Fn(&Path) -> bool,
) -> Result<RelinkCheck, String> {
    let conn = db.read().map_err(|e| e.to_string())?;
    let _src = get_source(&conn, id)?.ok_or("no such source")?;
    let new_path = normalize(new_path);
    let resolved = match resolve(table, &new_path, &system_uuid_of) {
        Resolved::Mounted { entry, id, rel_in_volume } => (entry, id, rel_in_volume),
        _ => return Err("The new folder is not on a mounted volume.".into()),
    };
    let mut st = conn
        .prepare("SELECT rel_path FROM photos WHERE source_id=?1 AND status='present' ORDER BY id LIMIT ?2")
        .map_err(|e| e.to_string())?;
    let paths: Vec<String> = st
        .query_map(params![id, RELINK_SAMPLE], |r| r.get(0))
        .map_err(|e| e.to_string())?
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let matched = paths.iter().filter(|p| exists(&new_path.join(p))).count() as i64;
    let check = RelinkCheck { sample_matched: matched, of: paths.len() as i64 };
    if !dry_run {
        let (entry, vid, rel) = resolved;
        db.exec(move |c| {
            c.execute(
                "UPDATE sources SET volume_key=?2, kind=?3, fs_type=?4, host=?5, host_hash=?6, share=?7, mount_path=?8, rel_root=?9 WHERE id=?1",
                params![id, vid.key, vid.kind, vid.fs_type, vid.host, vid.host_hash, vid.share, entry.mount_point, rel],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
        })?;
    }
    Ok(check)
}

#[cfg(test)]
mod tests {
    use super::*;

    // this owner's stack: an idle automount, then the same with the share mounted
    const OWNER_IDLE: &str = "\
22 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw,errors=remount-ro
41 22 0:38 / /mnt/synology/Photos_Backup rw,relatime shared:90 - autofs systemd-1 rw,fd=44,pgrp=1,timeout=0,minproto=5,maxproto=5,direct,pipe_ino=19873
";
    const OWNER_MOUNTED: &str = "\
22 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw,errors=remount-ro
41 22 0:38 / /mnt/synology/Photos_Backup rw,relatime shared:90 - autofs systemd-1 rw,fd=44,pgrp=1,timeout=0,minproto=5,maxproto=5,direct,pipe_ino=19873
912 41 0:83 / /mnt/synology/Photos_Backup rw,relatime shared:640 - cifs //SynologyDS220.local/Photos/Photos_Backup rw,vers=3.0,cache=strict,username=me,uid=1000,noforceuid,gid=1000,noforcegid,addr=192.168.1.20,file_mode=0755,dir_mode=0755,soft,nounix,mapposix,symlink=native,rsize=4194304,wsize=4194304,bsize=1048576,echo_interval=60,actimeo=1,x-systemd.automount
";
    const OWNER_REMOUNTED_ELSEWHERE: &str = "\
22 1 259:2 / / rw,relatime shared:1 - ext4 /dev/nvme0n1p2 rw,errors=remount-ro
955 22 0:90 / /media/nas/photos rw,relatime - cifs //synologyds220.local/Photos/Photos_Backup rw,vers=3.0
";
    const MIXED: &str = "\
22 1 259:2 / / rw,relatime - ext4 /dev/nvme0n1p2 rw
23 22 259:1 / /boot/efi rw,relatime - vfat /dev/nvme0n1p1 rw
30 22 8:17 / /run/media/me/EOS_DIGITAL rw,nosuid,nodev - exfat /dev/sdb1 rw
31 22 0:50 / /mnt/nfs\\040photos rw - nfs4 NAS.local:/volume1/photos rw,vers=4.1
32 22 0:51 /@home /home rw - btrfs /dev/nvme0n1p3 rw,subvol=/@home
33 22 0:52 / /mnt/pool rw - fuse.mergerfs a:b rw
34 22 0:53 / /run/user/1000/gvfs rw - fuse.gvfsd-fuse gvfsd-fuse rw
";

    fn no_uuid(_: &str) -> Option<String> {
        None
    }
    fn uuids(dev: &str) -> Option<String> {
        match dev {
            "/dev/sdb1" => Some("7C3E-1A09".into()),
            "/dev/nvme0n1p3" => Some("aaaa-bbbb".into()),
            _ => None,
        }
    }

    #[test]
    fn parses_mountinfo_including_optional_fields_and_escapes() {
        let t = MountTable::parse(MIXED);
        assert_eq!(t.entries.len(), 7);
        let nfs = t.entries.iter().find(|e| e.fs_type == "nfs4").unwrap();
        assert_eq!(nfs.mount_point, "/mnt/nfs photos");
        assert_eq!(nfs.source, "NAS.local:/volume1/photos");
        let owner = MountTable::parse(OWNER_MOUNTED);
        let cifs = owner.entries.iter().find(|e| e.fs_type == "cifs").unwrap();
        assert_eq!(cifs.option("vers"), Some("3.0"));
        assert_eq!(cifs.option("rsize"), Some("4194304"));
        assert_eq!(cifs.option("actimeo"), Some("1"));
        assert_eq!(cifs.options, "rw,relatime", "optional fields (shared:640) are not options");
        // garbage lines are skipped
        assert_eq!(MountTable::parse("nonsense\n\n- -\n").entries.len(), 0);
    }

    #[test]
    fn owners_stack_idle_then_mounted() {
        let p = Path::new("/mnt/synology/Photos_Backup/2023/Japan");
        let idle = MountTable::parse(OWNER_IDLE);
        // idle-out: only autofs is present
        assert_eq!(resolve(&idle, p, &no_uuid), Resolved::Asleep { mount_point: "/mnt/synology/Photos_Backup".into() });
        assert!(idle.idle_autofs_at(p).is_some());
        assert!(idle.real_mounts().iter().all(|e| e.mount_point == "/"));
        // mounted: the cifs entry stacked above the autofs one wins
        let up = MountTable::parse(OWNER_MOUNTED);
        let Resolved::Mounted { entry, id, rel_in_volume } = resolve(&up, p, &no_uuid) else { panic!("not mounted") };
        assert_eq!(entry.fs_type, "cifs");
        assert_eq!(id.key, "smb:synologyds220.local/photos");
        assert_eq!(id.share.as_deref(), Some("Photos"));
        assert_eq!(id.subpath, "Photos_Backup");
        assert_eq!(id.host.as_deref(), Some("synologyds220.local"));
        assert_eq!(id.host_hash.as_deref().map(str::len), Some(10));
        assert!(id.network);
        assert_eq!(id.kind, "nas");
        assert_eq!(rel_in_volume, "Photos_Backup/2023/Japan");
        assert_eq!(up.real_mounts().len(), 2);
        // idle-out again after the mount went away (autofs timeout): back to asleep
        assert!(matches!(resolve(&idle, p, &no_uuid), Resolved::Asleep { .. }));
    }

    // this machine's own table (read from /proc only; nothing is touched)
    #[cfg(target_os = "linux")]
    #[test]
    fn this_machines_mountinfo_parses() {
        let t = MountTable::current();
        assert!(!t.entries.is_empty());
        assert!(t.top_at(Path::new("/")).is_some());
        let Resolved::Mounted { id, .. } = resolve(&t, Path::new("/"), &system_uuid_of) else { panic!("/ is not mounted?") };
        assert!(!id.key.is_empty());
    }

    #[test]
    fn host_hash_is_stable_and_case_blind() {
        assert_eq!(host_hash("SynologyDS220.local"), host_hash("synologyds220.local"));
        assert_ne!(host_hash("a"), host_hash("b"));
        assert!(!host_hash("nas").contains("nas"));
    }

    fn src(id: i64, key: &str, rel: &str, mount: &str) -> Source {
        Source {
            id,
            name: format!("s{id}"),
            kind: "nas".into(),
            volume_key: key.into(),
            fs_type: "cifs".into(),
            host: None,
            host_hash: None,
            share: None,
            mount_path: Some(mount.into()),
            rel_root: rel.into(),
            enabled: true,
            treat_as_network: None,
            scan_policy: "manual".into(),
            include_subfolders: true,
            dir_skip: false,
            state: "never_indexed".into(),
            error: None,
            last_scan_start: None,
            last_scan_end: None,
            last_ok: None,
            created_at: 0,
            photos: 0,
            missing: 0,
        }
    }

    #[test]
    fn a_remount_at_another_path_is_the_same_volume_and_relinks() {
        let before = MountTable::parse(OWNER_MOUNTED);
        let p = Path::new("/mnt/synology/Photos_Backup/2023 Japan");
        let new = plan_add(&before, p, true, &[], &AddOptions::default(), &no_uuid).unwrap();
        assert_eq!(new.volume_key, "smb:synologyds220.local/photos");
        assert_eq!(new.rel_root, "Photos_Backup/2023 Japan");
        assert_eq!(new.mount_path, "/mnt/synology/Photos_Backup");
        let mut s = src(1, &new.volume_key, &new.rel_root, &new.mount_path);
        let a = availability(&before, &s, &no_uuid);
        assert_eq!(a, Availability::Online { mount_point: "/mnt/synology/Photos_Backup".into(), abs_root: "/mnt/synology/Photos_Backup/2023 Japan".into() });
        // idle-out: asleep, not offline
        let idle = MountTable::parse(OWNER_IDLE);
        assert_eq!(availability(&idle, &s, &no_uuid), Availability::Asleep { mount_point: "/mnt/synology/Photos_Backup".into() });
        // the share comes back mounted at the volume's own root of a different path, with another subpath
        let other = MountTable::parse(OWNER_REMOUNTED_ELSEWHERE);
        let a = availability(&other, &s, &no_uuid);
        assert_eq!(a, Availability::Online { mount_point: "/media/nas/photos".into(), abs_root: "/media/nas/photos/2023 Japan".into() });
        s.mount_path = Some("/media/nas/photos".into());
        // nothing mounted at all
        let none = MountTable::parse("22 1 259:2 / / rw - ext4 /dev/nvme0n1p2 rw\n");
        assert!(matches!(availability(&none, &s, &no_uuid), Availability::Offline { .. }));
        // a different share is another volume
        let wrong = MountTable::parse("22 1 259:2 / / rw - ext4 /dev/x rw\n9 22 0:9 / /mnt/x rw - cifs //synologyds220.local/Video rw,vers=3.0\n");
        assert!(matches!(availability(&wrong, &s, &no_uuid), Availability::Offline { .. }));
    }

    #[test]
    fn a_mount_that_does_not_contain_the_root_is_not_it() {
        // the share is back, but mounted only at a different folder of it
        let t = MountTable::parse("9 1 0:9 / /mnt/v rw - cifs //nas/Photos/Videos rw\n");
        let s = src(1, "smb:nas/photos", "Photos_Backup/a", "/mnt/old");
        assert!(matches!(availability(&t, &s, &no_uuid), Availability::Offline { .. }));
        let t2 = MountTable::parse("9 1 0:9 / /mnt/v rw - cifs //nas/Photos rw\n");
        let a = availability(&t2, &s, &no_uuid);
        assert_eq!(a, Availability::Online { mount_point: "/mnt/v".into(), abs_root: "/mnt/v/Photos_Backup/a".into() });
    }

    #[test]
    fn identities_of_drives_nfs_subvolumes_and_pools() {
        let t = MountTable::parse(MIXED);
        let id = |p: &str| match resolve(&t, Path::new(p), &uuids) {
            Resolved::Mounted { id, rel_in_volume, .. } => (id, rel_in_volume),
            other => panic!("{other:?}"),
        };
        let (sd, rel) = id("/run/media/me/EOS_DIGITAL/DCIM/100");
        assert_eq!(sd.key, "uuid:7C3E-1A09");
        assert_eq!(sd.kind, "drive");
        assert_eq!(rel, "DCIM/100");
        let (nfs, _) = id("/mnt/nfs photos/2024");
        assert_eq!(nfs.key, "nfs:nas.local:/volume1/photos");
        assert!(nfs.network);
        let (home, rel) = id("/home/me/Pictures");
        assert_eq!(home.key, "uuid:aaaa-bbbb");
        assert_eq!(home.subpath, "@home", "a btrfs subvolume shows its own root");
        assert_eq!(rel, "@home/me/Pictures");
        assert_eq!(home.kind, "local");
        assert!(id("/mnt/pool/x").0.network, "mergerfs is treated as a network");
        assert!(id("/run/user/1000/gvfs/x").0.network);
        // without a uuid the device name stands in
        let Resolved::Mounted { id, .. } = resolve(&t, Path::new("/run/media/me/EOS_DIGITAL"), &no_uuid) else { panic!() };
        assert_eq!(id.key, "dev:/dev/sdb1");
    }

    #[test]
    fn treat_as_network_override_wins() {
        let mut s = src(1, "k", "", "/m");
        assert!(treats_as_network(&s, true));
        assert!(!treats_as_network(&s, false));
        s.treat_as_network = Some(true);
        assert!(treats_as_network(&s, false));
        s.treat_as_network = Some(false);
        assert!(!treats_as_network(&s, true));
    }

    #[test]
    fn adding_while_offline_needs_the_flag_and_is_pending_until_it_mounts() {
        let idle = MountTable::parse(OWNER_IDLE);
        let p = Path::new("/mnt/synology/Photos_Backup/Archive");
        let e = plan_add(&idle, p, false, &[], &AddOptions::default(), &no_uuid).unwrap_err();
        assert!(matches!(e, AddError::Offline(_)));
        let opts = AddOptions { allow_offline: true, ..Default::default() };
        let n = plan_add(&idle, p, false, &[], &opts, &no_uuid).unwrap();
        assert!(n.pending);
        assert_eq!(n.volume_key, "pending:/mnt/synology/Photos_Backup");
        assert_eq!(n.rel_root, "Archive");
        // pending source: asleep while idle, online (with its identity to learn) once mounted
        let s = src(1, &n.volume_key, &n.rel_root, &n.mount_path);
        assert!(matches!(availability(&idle, &s, &no_uuid), Availability::Asleep { .. }));
        let up = MountTable::parse(OWNER_MOUNTED);
        assert!(matches!(availability(&up, &s, &no_uuid), Availability::Online { .. }));
        // a path under no known mount at all is a usage error
        let none = MountTable::parse("");
        assert!(matches!(plan_add(&none, p, true, &[], &opts, &no_uuid), Err(AddError::Usage(_))));
        assert!(matches!(plan_add(&idle, Path::new("rel/path"), true, &[], &opts, &no_uuid), Err(AddError::Usage(_))));
    }

    #[test]
    fn overlapping_sources_are_refused_by_component() {
        let up = MountTable::parse(OWNER_MOUNTED);
        let key = "smb:synologyds220.local/photos";
        let known = vec![src(7, key, "Photos_Backup/Archive", "/mnt/synology/Photos_Backup")];
        let add = |p: &str, sub: bool| {
            plan_add(&up, Path::new(p), true, &known, &AddOptions { include_subfolders: sub, ..Default::default() }, &no_uuid)
        };
        assert_eq!(add("/mnt/synology/Photos_Backup/Archive", true).unwrap_err(), AddError::Overlap(7));
        assert_eq!(add("/mnt/synology/Photos_Backup/Archive/2012", true).unwrap_err(), AddError::Overlap(7));
        assert_eq!(add("/mnt/synology/Photos_Backup", true).unwrap_err(), AddError::Overlap(7));
        // a sibling with a similar prefix is not nested
        assert!(add("/mnt/synology/Photos_Backup/Archive 2020", true).is_ok());
        // a new non-recursive parent leaves the existing folder below it alone
        assert!(add("/mnt/synology/Photos_Backup", false).is_ok());
    }

    #[test]
    fn non_recursive_sources_do_not_overlap_their_subfolders() {
        let up = MountTable::parse(OWNER_MOUNTED);
        let key = "smb:synologyds220.local/photos";
        let mut parent = src(3, key, "Photos_Backup", "/mnt/synology/Photos_Backup");
        parent.include_subfolders = false;
        let r = plan_add(&up, Path::new("/mnt/synology/Photos_Backup/2023"), true, &[parent], &AddOptions::default(), &no_uuid);
        assert!(r.is_ok());
        // and a new non-recursive parent over an existing child
        let child = src(4, key, "Photos_Backup/2023", "/mnt/synology/Photos_Backup");
        let opts = AddOptions { include_subfolders: false, ..Default::default() };
        assert!(plan_add(&up, Path::new("/mnt/synology/Photos_Backup"), true, &[child], &opts, &no_uuid).is_ok());
    }

    #[test]
    fn crud_relink_and_adoption_against_a_database() {
        let dir = tempfile::tempdir().unwrap();
        let db = CatalogDb::open(&dir.path().join("catalog.sqlite")).unwrap();
        // two sources on the same volume, one pending
        let ids: Vec<i64> = db
            .exec(|c| {
                c.execute(
                    "INSERT INTO sources(name,kind,volume_key,fs_type,mount_path,rel_root,created_at) VALUES('A','nas','smb:synologyds220.local/photos','cifs','/mnt/old','Photos_Backup/A',1)",
                    [],
                )
                .unwrap();
                c.execute(
                    "INSERT INTO sources(name,kind,volume_key,fs_type,mount_path,rel_root,created_at) VALUES('B','nas','smb:synologyds220.local/photos','cifs','/mnt/old','Photos_Backup/B',1)",
                    [],
                )
                .unwrap();
                c.execute(
                    "INSERT INTO sources(name,kind,volume_key,fs_type,mount_path,rel_root,created_at) VALUES('P','nas','pending:/mnt/synology/Photos_Backup','autofs','/mnt/synology/Photos_Backup','Later',1)",
                    [],
                )
                .unwrap();
                Ok(vec![1, 2, 3])
            })
            .unwrap();
        assert_eq!(ids.len(), 3);
        let table = MountTable::parse(OWNER_REMOUNTED_ELSEWHERE);
        let status = reconcile(&db, &table, &no_uuid).unwrap();
        assert_eq!(status[0].availability.word(), "online");
        assert_eq!(status[1].source.mount_path.as_deref(), Some("/media/nas/photos"), "relinked");
        assert_eq!(status[2].availability.word(), "offline", "the pending mount point has nothing");
        let c = db.read().unwrap();
        assert_eq!(get_source(&c, 1).unwrap().unwrap().mount_path.as_deref(), Some("/media/nas/photos"));
        assert_eq!(get_source(&c, 2).unwrap().unwrap().mount_path.as_deref(), Some("/media/nas/photos"));
        // the pending one adopts the identity when its mount point shows a share
        let up = MountTable::parse(OWNER_MOUNTED);
        let status = reconcile(&db, &up, &no_uuid).unwrap();
        let p = &status[2];
        assert_eq!(p.source.volume_key, "smb:synologyds220.local/photos");
        assert_eq!(p.source.rel_root, "Photos_Backup/Later");
        assert_eq!(p.availability.word(), "online");
        // update, find by name, remove
        update_source(&db, 1, SourceUpdate { name: Some("Renamed".into()), enabled: Some(false), treat_as_network: Some(Some(false)), ..Default::default() }).unwrap();
        let c = db.read().unwrap();
        let s = find_source(&c, "renamed").unwrap().unwrap();
        assert!(!s.enabled);
        assert_eq!(s.treat_as_network, Some(false));
        assert_eq!(remove_preview(&c, 1).unwrap().photos, 0);
        remove_source(&db, 1).unwrap();
        assert!(get_source(&db.read().unwrap(), 1).unwrap().is_none());
    }

    #[test]
    fn add_source_on_a_real_local_folder() {
        let dir = tempfile::tempdir().unwrap();
        let photos = dir.path().join("Photos");
        std::fs::create_dir_all(&photos).unwrap();
        let db = CatalogDb::open(&dir.path().join("catalog.sqlite")).unwrap();
        let s = add_source(&db, &photos, &AddOptions { name: Some("Mine".into()), ..Default::default() }).unwrap();
        assert_eq!(s.name, "Mine");
        assert!(!s.volume_key.is_empty());
        assert!(s.mount_path.is_some());
        let again = add_source(&db, &photos.join("sub").join(".."), &AddOptions::default());
        assert!(matches!(again, Err(AddError::Overlap(_))), "{again:?}");
        let missing = add_source(&db, &dir.path().join("nope"), &AddOptions::default());
        assert!(matches!(missing, Err(AddError::Offline(_))), "{missing:?}");
    }

    #[test]
    fn relink_checks_a_sample_then_applies() {
        let dir = tempfile::tempdir().unwrap();
        let db = CatalogDb::open(&dir.path().join("catalog.sqlite")).unwrap();
        db.exec(|c| {
            c.execute("INSERT INTO sources(name,kind,volume_key,mount_path,rel_root,created_at) VALUES('A','nas','smb:x/y','/mnt/old','',1)", []).unwrap();
            c.execute("INSERT INTO dirs(source_id,rel_path) VALUES(1,'')", []).unwrap();
            for n in ["a.jpg", "b.jpg"] {
                c.execute("INSERT INTO photos(source_id,dir_id,name,rel_path,ext,size,mtime_ns,discovered_at,seen_at) VALUES(1,1,?1,?1,'jpg',1,1,1,1)", [n]).unwrap();
            }
            Ok(())
        })
        .unwrap();
        let table = MountTable::parse("9 1 0:9 / /mnt/new rw - cifs //nas/y rw\n");
        let only_a = |p: &Path| p.ends_with("a.jpg");
        let dry = relink(&db, 1, Path::new("/mnt/new"), true, &table, &only_a).unwrap();
        assert_eq!((dry.sample_matched, dry.of), (1, 2));
        assert_eq!(get_source(&db.read().unwrap(), 1).unwrap().unwrap().mount_path.as_deref(), Some("/mnt/old"));
        relink(&db, 1, Path::new("/mnt/new"), false, &table, &only_a).unwrap();
        assert_eq!(get_source(&db.read().unwrap(), 1).unwrap().unwrap().mount_path.as_deref(), Some("/mnt/new"));
    }
}
