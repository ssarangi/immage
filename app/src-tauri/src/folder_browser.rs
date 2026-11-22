// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// the in-app folder browser's two read-only commands: the places to start
// from (home, mounted drives and shares, the folders already added) and the
// subfolders of one folder. Only directory names leave this file: no file
// names, no sizes, no content. A drive that does not answer is reported, not
// waited on: the listing runs on a worker with a timeout, and a drive
// volumes::answering() has found stuck is not touched at all.

use serde::Serialize;
use std::ffi::OsString;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::volumes;

// how long one folder's listing is waited for
const LIST_WAIT: Duration = Duration::from_secs(5);

#[derive(Serialize, Clone, Debug, PartialEq, specta::Type)]
pub struct BrowserPlace {
    // home | pictures | desktop | documents | root | drive | added
    pub kind: String,
    pub label: String,
    pub path: String,
    // the server of a network share
    pub host: Option<String>,
    pub network: bool,
    // false when the drive is not answering
    pub online: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq, specta::Type)]
pub struct BrowserDir {
    pub name: String,
    pub path: String,
    pub hidden: bool,
}

#[derive(Serialize, Clone, Debug, PartialEq, specta::Type)]
pub struct BrowserListing {
    pub path: String,
    pub parent: Option<String>,
    pub dirs: Vec<BrowserDir>,
    // offline | timeout | missing | notdir | denied | other
    pub error: Option<String>,
}

fn place(kind: &str, label: String, path: &Path) -> BrowserPlace {
    BrowserPlace { kind: kind.into(), label, path: path.to_string_lossy().into_owned(), host: None, network: false, online: true }
}

// the well-known folders of a home folder that exist
fn home_places(home: &Path, exists: impl Fn(&Path) -> bool) -> Vec<BrowserPlace> {
    let mut out = vec![place("home", home.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(), home)];
    for (kind, name) in [("pictures", "Pictures"), ("desktop", "Desktop"), ("documents", "Documents")] {
        let p = home.join(name);
        if exists(&p) { out.push(place(kind, name.into(), &p)); }
    }
    out
}

// (source, mount point, file system) of the mounts worth offering
#[cfg(any(target_os = "linux", test))]
fn drive_mounts(table: &str) -> Vec<(String, String, String)> {
    const PSEUDO: &[&str] = &[
        "proc", "sysfs", "devtmpfs", "devpts", "tmpfs", "cgroup", "cgroup2", "securityfs", "debugfs", "tracefs",
        "configfs", "fusectl", "pstore", "bpf", "mqueue", "hugetlbfs", "binfmt_misc", "efivarfs", "overlay",
        "squashfs", "nsfs", "ramfs", "rpc_pipefs", "fuse.portal", "fuse.gvfsd-fuse", "fuse.snapfuse", "autofs.pseudo",
    ];
    let mut out: Vec<(String, String, String)> = Vec::new();
    for line in table.lines() {
        let mut f = line.split_whitespace();
        let (Some(src), Some(at), Some(fs)) = (f.next(), f.next(), f.next()) else { continue };
        let (src, at) = (volumes::unescape(src), volumes::unescape(at));
        if at == "/" || PSEUDO.contains(&fs) { continue; }
        let under = ["/mnt/", "/media/", "/run/media/"].iter().any(|p| at.starts_with(p));
        if !(under || volumes::is_network_fs(fs)) { continue; }
        // an automount and the share mounted over it are one entry; the later one wins
        out.retain(|(_, m, _)| *m != at);
        out.push((src, at, fs.to_string()));
    }
    out
}

#[cfg(target_os = "linux")]
fn drives() -> Vec<BrowserPlace> {
    let table = std::fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    drive_mounts(&table).into_iter().map(|(src, at, fs)| {
        let network = volumes::is_network_fs(&fs) || fs == "autofs";
        let mut p = place("drive", Path::new(&at).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| at.clone()),
                          Path::new(&at));
        p.network = network;
        p.host = if network { volumes::host_of(&src) } else { None };
        p
    }).collect()
}

#[cfg(target_os = "macos")]
fn drives() -> Vec<BrowserPlace> {
    let Ok(rd) = std::fs::read_dir("/Volumes") else { return Vec::new() };
    let mut out: Vec<BrowserPlace> = rd.flatten().map(|e| {
        let path = e.path();
        let mut p = place("drive", e.file_name().to_string_lossy().into_owned(), &path);
        p.network = volumes::on_network(&path);
        p
    }).collect();
    out.sort_by(|a, b| a.label.cmp(&b.label));
    out
}

#[cfg(windows)]
fn drives() -> Vec<BrowserPlace> {
    ('A'..='Z').filter_map(|c| {
        let root = format!("{c}:\\");
        // GetLogicalDrives would be exact; a mapped drive that is gone is caught by the probe below
        Path::new(&root).exists().then(|| {
            let mut p = place("drive", format!("{c}:"), Path::new(&root));
            p.network = volumes::on_network(Path::new(&root));
            p
        })
    }).collect()
}

// each drive asked whether it answers, side by side: one dead share costs
// one wait, not one per drive. a mount that is an idle automount is not
// asked, as asking would mount it
fn probe(places: Vec<BrowserPlace>) -> Vec<BrowserPlace> {
    std::thread::scope(|s| {
        let jobs: Vec<_> = places.into_iter().map(|mut p| {
            s.spawn(move || {
                if p.kind == "drive" && !volumes::autofs_idle(Path::new(&p.path)) {
                    p.online = volumes::answering(Path::new(&p.path));
                }
                p
            })
        }).collect();
        jobs.into_iter().filter_map(|j| j.join().ok()).collect()
    })
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from).filter(|p| p.is_absolute())
}

// where to start: home and its usual folders, the root, the drives, then
// the folders already added (they are not asked: they are listed when opened)
#[tauri::command]
#[specta::specta]
pub(crate) async fn folder_places(added: Vec<String>) -> Vec<BrowserPlace> {
    tauri::async_runtime::spawn_blocking(move || {
        let mut out = Vec::new();
        if let Some(home) = home_dir() {
            // home's own folders are not asked about: their drive is home's
            out.extend(home_places(&home, |p| p.is_dir()));
        }
        #[cfg(not(windows))]
        out.push(place("root", "/".into(), Path::new("/")));
        out.extend(probe(drives()));
        for a in added {
            let name = Path::new(&a).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| a.clone());
            out.push(place("added", name, Path::new(&a)));
        }
        out
    }).await.unwrap_or_default()
}

fn listing(path: &str, dirs: Vec<BrowserDir>, error: Option<&str>) -> BrowserListing {
    let parent = Path::new(path).parent().map(|p| p.to_string_lossy().into_owned()).filter(|p| !p.is_empty());
    BrowserListing { path: path.into(), parent, dirs, error: error.map(str::to_string) }
}

fn error_code(e: &io::Error) -> &'static str {
    match e.kind() {
        io::ErrorKind::NotFound => "missing",
        io::ErrorKind::PermissionDenied => "denied",
        io::ErrorKind::NotADirectory => "notdir",
        _ => "other",
    }
}

// the names of the folders in a directory: the reader's entries are
// (name, is a directory); a link is followed by the reader, on its worker
type Reader = Box<dyn FnOnce() -> io::Result<Vec<(OsString, bool)>> + Send>;

fn read_dirs(dir: PathBuf) -> Reader {
    Box::new(move || {
        let mut out = Vec::new();
        for e in std::fs::read_dir(&dir)?.flatten() {
            let is_dir = match e.file_type() {
                Ok(t) if t.is_dir() => true,
                Ok(t) if t.is_symlink() => std::fs::metadata(e.path()).map(|m| m.is_dir()).unwrap_or(false),
                _ => false,
            };
            if is_dir { out.push((e.file_name(), true)); }
        }
        Ok(out)
    })
}

// the listing of `path`, or a timeout when the reader has not returned in
// `wait`; the worker goes on and its late answer is dropped
fn list_with(path: &str, wait: Duration, reader: Reader) -> BrowserListing {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || { let _ = tx.send(reader()); });
    match rx.recv_timeout(wait) {
        Err(_) => listing(path, Vec::new(), Some("timeout")),
        Ok(Err(e)) => listing(path, Vec::new(), Some(error_code(&e))),
        Ok(Ok(entries)) => {
            let mut dirs: Vec<BrowserDir> = entries.into_iter().filter(|(_, d)| *d).map(|(n, _)| {
                let name = n.to_string_lossy().into_owned();
                BrowserDir { path: Path::new(path).join(&n).to_string_lossy().into_owned(), hidden: name.starts_with('.'), name }
            }).collect();
            dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then_with(|| a.name.cmp(&b.name)));
            listing(path, dirs, None)
        }
    }
}

// "~" and "~/x" are the home folder; a path that is not absolute is not one
fn expand(path: &str) -> Option<String> {
    let p = path.trim();
    let p = match p.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\') =>
            format!("{}{rest}", home_dir()?.to_string_lossy()),
        _ => p.to_string(),
    };
    Path::new(&p).is_absolute().then_some(p)
}

#[tauri::command]
#[specta::specta]
pub(crate) async fn list_subdirectories(path: String) -> BrowserListing {
    tauri::async_runtime::spawn_blocking(move || {
        let Some(path) = expand(&path) else { return listing(path.trim(), Vec::new(), Some("missing")) };
        // a drive that does not answer is not read: its read would hold for the kernel's timeout
        if !volumes::answering(Path::new(&path)) { return listing(&path, Vec::new(), Some("offline")); }
        list_with(&path, LIST_WAIT, read_dirs(PathBuf::from(&path)))
    }).await.unwrap_or_else(|_| BrowserListing { path: String::new(), parent: None, dirs: Vec::new(), error: Some("other".into()) })
}

#[cfg(test)]
mod tests {
    use super::*;

    const TABLE: &str = "\
/dev/nvme0n1p2 / ext4 rw 0 0
proc /proc proc rw 0 0
tmpfs /run/user/1000 tmpfs rw 0 0
/dev/sda1 /media/me/USB\\040Stick vfat rw 0 0
systemd-1 /mnt/nas/Photos autofs rw 0 0
//nas.local/Photos /mnt/nas/Photos cifs rw 0 0
nas:/export /srv/data nfs4 rw 0 0
gvfsd-fuse /run/user/1000/gvfs fuse.gvfsd-fuse rw 0 0
/dev/sdb1 /data ext4 rw 0 0
";

    #[test]
    fn mounts_keep_drives_and_shares_only() {
        let m = drive_mounts(TABLE);
        let at: Vec<&str> = m.iter().map(|(_, a, _)| a.as_str()).collect();
        assert_eq!(at, ["/media/me/USB Stick", "/mnt/nas/Photos", "/srv/data"]);
        // the share replaced its automount
        assert_eq!(m[1].2, "cifs");
    }

    #[test]
    fn home_places_list_only_existing_folders() {
        let p = home_places(Path::new("/home/me"), |p| p.ends_with("Pictures"));
        let kinds: Vec<&str> = p.iter().map(|p| p.kind.as_str()).collect();
        assert_eq!(kinds, ["home", "pictures"]);
        assert_eq!(p[0].label, "me");
    }

    #[test]
    fn lists_only_folders_sorted_and_marks_hidden() {
        let t = tempfile::tempdir().unwrap();
        for d in ["beta", "Alpha", ".cache"] { std::fs::create_dir(t.path().join(d)).unwrap(); }
        std::fs::write(t.path().join("photo.jpg"), b"x").unwrap();
        let path = t.path().to_string_lossy().into_owned();
        let l = list_with(&path, Duration::from_secs(5), read_dirs(t.path().to_path_buf()));
        assert_eq!(l.error, None);
        let names: Vec<&str> = l.dirs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, [".cache", "Alpha", "beta"]);
        assert!(l.dirs[0].hidden && !l.dirs[1].hidden);
        assert_eq!(l.parent.as_deref(), t.path().parent().and_then(|p| p.to_str()));
        assert!(l.dirs.iter().all(|d| Path::new(&d.path).starts_with(t.path())));
    }

    #[test]
    fn missing_folder_and_file_are_errors() {
        let t = tempfile::tempdir().unwrap();
        let gone = t.path().join("gone");
        let l = list_with(gone.to_str().unwrap(), Duration::from_secs(5), read_dirs(gone.clone()));
        assert_eq!(l.error.as_deref(), Some("missing"));
        let f = t.path().join("f");
        std::fs::write(&f, b"x").unwrap();
        let l = list_with(f.to_str().unwrap(), Duration::from_secs(5), read_dirs(f.clone()));
        assert_eq!(l.error.as_deref(), Some("notdir"));
    }

    #[cfg(unix)]
    #[test]
    fn a_link_to_a_folder_is_a_folder() {
        let t = tempfile::tempdir().unwrap();
        std::fs::create_dir(t.path().join("real")).unwrap();
        std::os::unix::fs::symlink(t.path().join("real"), t.path().join("link")).unwrap();
        std::os::unix::fs::symlink(t.path().join("nowhere"), t.path().join("dangling")).unwrap();
        let l = list_with(t.path().to_str().unwrap(), Duration::from_secs(5), read_dirs(t.path().to_path_buf()));
        let names: Vec<&str> = l.dirs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["link", "real"]);
    }

    #[test]
    fn an_unresponsive_reader_times_out_without_blocking() {
        let started = std::time::Instant::now();
        let l = list_with("/fake/dead", Duration::from_millis(100), Box::new(|| {
            std::thread::sleep(Duration::from_secs(3));
            Ok(Vec::new())
        }));
        assert_eq!(l.error.as_deref(), Some("timeout"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn expand_wants_an_absolute_path() {
        assert_eq!(expand("relative/dir"), None);
        assert_eq!(expand("  /tmp/x  ").as_deref(), Some("/tmp/x"));
        if let Some(home) = home_dir() {
            assert_eq!(expand("~/a").unwrap(), format!("{}/a", home.to_string_lossy()));
        }
    }
}
