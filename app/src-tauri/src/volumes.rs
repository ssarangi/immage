// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// the drive each of the library's folders is on, for the folder tree's
// drive headings: folders of the same name on the Mac and on a NAS are
// told apart by the drive they sit under, as in Lightroom

use serde::Serialize;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{mpsc, Arc, Condvar, LazyLock, Mutex};
use std::time::{Duration, Instant};

#[derive(Serialize, Clone, Debug, PartialEq, specta::Type)]
pub struct Volume {
    pub path: String,
    // where the drive is mounted: the folders of one drive share it
    pub mount: String,
    pub name: String,
    // the server a network share comes from
    pub host: Option<String>,
    pub network: bool,
    // false when the folder is not there, as when its share is not mounted,
    // or its drive is not answering (answering()); the rest is then only a
    // guess, and the client keeps what it last saw
    pub online: bool,
}

pub fn volume(path: &str) -> Volume {
    // a drive that is not answering is offline, and only its mount is known
    describe(path, answering(Path::new(path)) && Path::new(path).is_dir())
}

// the drive of a folder the caller has just found answering and there, so
// it is not asked again (volume() would stat it twice more)
pub fn volume_online(path: &str) -> Volume { describe(path, true) }

fn describe(path: &str, online: bool) -> Volume {
    let found = if online { probe(Path::new(path)) } else { drive_of(Path::new(path)) };
    let mut v = found.unwrap_or_else(|| Volume {
        path: String::new(),
        mount: String::new(),
        name: String::new(),
        host: None,
        network: false,
        online,
    });
    v.path = path.to_string();
    v.online = online;
    v
}

// Mount-table-only identity for background scans: never asks a server.
pub fn mount_identity(path: &Path) -> Option<String> { drive_of(path).map(|v|v.mount) }

// Snapshot mount points once for a recursive walk. Individual entries may
// themselves be file mounts, which cannot be detected from the parent's device.
pub(crate) fn mount_points() -> std::collections::HashSet<std::path::PathBuf> {
    #[cfg(target_os = "linux")]
    {
        std::fs::read_to_string("/proc/self/mounts").unwrap_or_default().lines()
            .filter_map(|line| line.split_whitespace().nth(1))
            .map(|path| std::path::PathBuf::from(unescape(path))).collect()
    }
    #[cfg(target_os = "macos")]
    {
        mounts().into_iter().map(|(_, path, _, _)| std::path::PathBuf::from(path)).collect()
    }
    #[cfg(windows)]
    {
        // Junctions and other reparse points are found by symlink_metadata.
        std::collections::HashSet::new()
    }
}

// whether the path is on a network share, from the mount table alone
pub fn on_network(path: &Path) -> bool { drive_of(path).is_some_and(|v| v.network) }

// why a database file may not live at this path, or None when it may: its
// mount is a network file system (or an automount, which is one until
// something is mounted), or the mount cannot be told and the path is under
// /mnt or /media. SQLite's WAL needs shared memory and working locks, which
// network file systems do not give. A path that does not exist yet is judged
// by its nearest existing parent, symlinks resolved.
pub fn network_refusal(path: &Path) -> Option<String> {
    let path = resolve_for_mount(path);
    #[cfg(target_os = "linux")]
    {
        let table = std::fs::read_to_string("/proc/self/mounts").ok();
        refusal_from(table.as_deref(), &path)
    }
    #[cfg(not(target_os = "linux"))]
    {
        on_network(&path).then(|| format!("{} is on a network share", path.display()))
    }
}

fn resolve_for_mount(path: &Path) -> std::path::PathBuf {
    let abs = if path.is_absolute() { path.to_path_buf() } else { std::env::current_dir().unwrap_or_default().join(path) };
    let mut tail = Vec::new();
    let mut at = abs.as_path();
    loop {
        if let Ok(real) = std::fs::canonicalize(at) {
            let mut out = real;
            for part in tail.iter().rev() { out.push(part); }
            return out;
        }
        match (at.parent(), at.file_name()) {
            (Some(parent), Some(name)) => { tail.push(name.to_os_string()); at = parent; }
            _ => return abs,
        }
    }
}

#[cfg(any(target_os = "linux", test))]
fn refusal_from(table: Option<&str>, path: &Path) -> Option<String> {
    match table.and_then(|t| mount_of(t, path)) {
        Some((source, mount, fs)) if is_network_fs(&fs) || fs == "autofs" => Some(format!(
            "{} is on a network share ({source} at {mount}, type {fs})", path.display())),
        Some(_) => None,
        None if path.starts_with("/mnt") || path.starts_with("/media") => Some(format!(
            "{} is under /mnt or /media and its file system could not be determined; it may be a network share",
            path.display())),
        None => None,
    }
}

// how long a drive has to answer, and how long an answer is trusted for
const DRIVE_WAIT: Duration = Duration::from_secs(3);
const DRIVE_FRESH: Duration = Duration::from_secs(2);
// threads that ask drives; a stat that never returns holds one for good, so
// the count (not the number of callers) bounds the leak
const PROBERS: usize = 4;

#[derive(Default)]
struct Drive {
    stuck: bool,
    answered: Option<Instant>,
    // a stat is in flight for this drive: later callers wait for it
    // instead of starting another
    probing: bool,
}

// drives by mount point. a share whose server has gone away but is still
// mounted holds stat() and readdir() in the kernel for good, as can a
// failing disk, so a stat is tried on a worker thread first. while it
// waits, the drive is stuck and is not tried again; it answers again when
// that stat returns
static DRIVES: LazyLock<Mutex<HashMap<String, Drive>>> = LazyLock::new(Default::default);
static DRIVE_CHANGED: Condvar = Condvar::new();

type ProbeJob = Box<dyn FnOnce() + Send>;

// PROBERS threads, started on first use and reused for every stat
static PROBE_POOL: LazyLock<Mutex<mpsc::Sender<ProbeJob>>> = LazyLock::new(|| {
    let (tx, rx) = mpsc::channel::<ProbeJob>();
    let rx = Arc::new(Mutex::new(rx));
    for i in 0..PROBERS {
        let rx = rx.clone();
        let _ = std::thread::Builder::new().name(format!("drive-probe-{i}")).spawn(move || loop {
            let job = rx.lock().unwrap().recv();
            match job {
                Ok(job) => job(),
                Err(_) => break,
            }
        });
    }
    Mutex::new(tx)
});

// whether the path can be touched without waiting for ever: its drive has
// answered a stat within DRIVE_WAIT
pub fn answering(path: &Path) -> bool {
    let drive = drive_of(path).map(|v| v.mount).unwrap_or_default();
    let at = path.to_path_buf();
    answering_with(&drive, DRIVE_WAIT, move || {
        // not found is an answer too
        let _ = std::fs::metadata(&at);
    })
}

// answering() with the stat and the wait given. one stat per drive is ever
// in flight, however many callers ask meanwhile, and it runs on the shared
// worker threads, so a drive that never answers costs one worker, not a
// thread per caller
pub(crate) fn answering_with(drive: &str, wait: Duration, stat: impl FnOnce() + Send + 'static) -> bool {
    let mut drives = DRIVES.lock().unwrap();
    {
        let s = drives.entry(drive.to_string()).or_default();
        if s.stuck {
            return false;
        }
        if s.answered.is_some_and(|t| t.elapsed() < DRIVE_FRESH) {
            return true;
        }
        if !s.probing {
            s.probing = true;
            let key = drive.to_string();
            let job: ProbeJob = Box::new(move || {
                stat();
                let mut drives = DRIVES.lock().unwrap();
                let s = drives.entry(key).or_default();
                s.probing = false;
                s.stuck = false;
                s.answered = Some(Instant::now());
                DRIVE_CHANGED.notify_all();
            });
            if PROBE_POOL.lock().unwrap().send(job).is_err() {
                s.probing = false;
                return false;
            }
        }
    }
    let until = Instant::now() + wait;
    loop {
        if !drives.get(drive).is_some_and(|s| s.probing) {
            return true;
        }
        let left = until.saturating_duration_since(Instant::now());
        if left.is_zero() {
            // under the lock the stat's worker answers with, so a late answer is never overwritten
            drives.entry(drive.to_string()).or_default().stuck = true;
            return false;
        }
        drives = DRIVE_CHANGED.wait_timeout(drives, left).unwrap().0;
    }
}

// whether the path is under an automount that has nothing mounted yet (or
// again): touching it would mount the share, so a poll should not. Linux only
#[allow(dead_code)] // for the catalog's light mount poll
pub fn autofs_idle(path: &Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        let Ok(table) = std::fs::read_to_string("/proc/self/mounts") else { return false };
        mount_of(&table, path).is_some_and(|(_, _, fs)| fs == "autofs")
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = path;
        false
    }
}

// the server in a mount's source: //user@nas.local/Pictures, nas:/export,
// \\nas\share
fn host_of(source: &str) -> Option<String> {
    let s = source.replace('\\', "/");
    let host = if let Some(rest) = s.strip_prefix("//") {
        let h = rest.split('/').next()?;
        h.rsplit('@').next()?.to_string()
    } else if let Some((h, _)) = s.split_once(':') {
        if h.contains('/') { return None; }
        h.to_string()
    } else {
        return None;
    };
    (!host.is_empty()).then_some(host)
}

#[cfg(not(windows))]
fn last_component(mount: &str) -> String {
    Path::new(mount).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| mount.to_string())
}

// a local disk's own name ("Macintosh HD"), asked of that disk alone:
// sysinfo's list of disks asks every share too
#[cfg(target_os = "macos")]
fn disk_name(mount: &str) -> Option<String> {
    use std::ffi::CString;
    let c = CString::new(mount).ok()?;
    let mut list: libc::attrlist = unsafe { std::mem::zeroed() };
    list.bitmapcount = libc::ATTR_BIT_MAP_COUNT;
    list.volattr = libc::ATTR_VOL_INFO | libc::ATTR_VOL_NAME;
    let mut buf = [0u8; 1024];
    let got = unsafe {
        libc::getattrlist(c.as_ptr(), &mut list as *mut _ as *mut libc::c_void, buf.as_mut_ptr() as *mut libc::c_void,
                          buf.len(), 0)
    };
    if got != 0 { return None; }
    // the buffer's length, then the name's attrreference_t: its offset from
    // the reference itself, and its length with the terminating NUL
    let offset = i32::from_ne_bytes(buf[4..8].try_into().ok()?);
    let len = u32::from_ne_bytes(buf[8..12].try_into().ok()?) as usize;
    let start = 4usize.checked_add_signed(offset as isize)?;
    let name = buf.get(start..start + len)?;
    let name = String::from_utf8_lossy(name).trim_end_matches('\0').to_string();
    (!name.is_empty()).then_some(name)
}

// the mounts as the kernel last saw them: MNT_NOWAIT asks no server, so a
// share that has gone away is listed without waiting on it. returns
// (source, mount point, file system type, local)
#[cfg(target_os = "macos")]
fn mounts() -> Vec<(String, String, String, bool)> {
    use std::ffi::CStr;
    let n = unsafe { libc::getfsstat(std::ptr::null_mut(), 0, libc::MNT_NOWAIT) };
    if n <= 0 { return Vec::new(); }
    // room for a mount made in between
    let mut list: Vec<libc::statfs> = vec![unsafe { std::mem::zeroed() }; n as usize + 4];
    let size = (list.len() * std::mem::size_of::<libc::statfs>()) as libc::c_int;
    let n = unsafe { libc::getfsstat(list.as_mut_ptr(), size, libc::MNT_NOWAIT) };
    let text = |a: &[libc::c_char]| unsafe { CStr::from_ptr(a.as_ptr()) }.to_string_lossy().into_owned();
    list.iter().take(n.max(0) as usize)
        .map(|st| (text(&st.f_mntfromname), text(&st.f_mntonname), text(&st.f_fstypename),
                   st.f_flags & libc::MNT_LOCAL as u32 != 0))
        .collect()
}

// the drive a path is on, from the mount table alone: the longest mount
// point it is under, the home folder's being the startup disk
#[cfg(target_os = "macos")]
fn drive_of(path: &Path) -> Option<Volume> {
    let (source, mount, fstype, local) = mounts().into_iter()
        .filter(|(_, at, _, _)| path.starts_with(at))
        .max_by_key(|(_, at, _, _)| at.len())?;
    let network = !local || matches!(fstype.as_str(), "smbfs" | "afpfs" | "nfs" | "webdav");
    // the home folder is on the system's data volume, which Finder shows as
    // the startup disk itself
    let mount = if mount == "/System/Volumes/Data" { "/".to_string() } else { mount };
    let name = last_component(&mount);
    Some(Volume { path: String::new(), mount, name, host: if network { host_of(&source) } else { None }, network,
                  online: true })
}

#[cfg(target_os = "macos")]
fn probe(path: &Path) -> Option<Volume> {
    let mut v = drive_of(path)?;
    if !v.network {
        v.name = disk_name(&v.mount).unwrap_or_else(|| if v.mount == "/" { "Macintosh HD".into() } else { v.name });
    }
    Some(v)
}

// /proc/self/mounts' fields escape spaces and the like as octal: \040
#[cfg(any(target_os = "linux", test))]
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

// the mount holding a path, from a mounts table: the longest mount point
// it is under. returns (source, mount point, file system type)
#[cfg(any(target_os = "linux", test))]
fn mount_of(table: &str, path: &Path) -> Option<(String, String, String)> {
    table.lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let (src, at, fs) = (f.next()?, f.next()?, f.next()?);
            Some((unescape(src), unescape(at), fs.to_string()))
        })
        .filter(|(_, at, _)| path.starts_with(at))
        .max_by_key(|(_, at, _)| at.len())
}

#[cfg(any(target_os = "linux", test))]
pub(crate) fn is_network_fs(fs: &str) -> bool {
    // pooling and cluster file systems count too: any of them may be slow or
    // away. an autofs entry says nothing of what is behind it, so it is
    // treated as a network too (from_table) until something is mounted there
    matches!(
        fs,
        "nfs" | "nfs4" | "cifs" | "smb3" | "smbfs" | "fuse.sshfs" | "sshfs" | "davfs" | "fuse.rclone" | "9p"
            | "fuse.gvfsd-fuse" | "fuse.mergerfs" | "ceph" | "fuse.ceph" | "glusterfs" | "fuse.glusterfs"
            | "lustre" | "afs" | "fuse.davfs" | "fuse.davfs2" | "fuse.s3fs" | "fuse.gcsfuse" | "fuse.smbnetfs"
            | "fuse.cryfs" | "fuse.encfs"
    ) || fs.starts_with("fuse.rclone") || fs.starts_with("nfs") || fs.starts_with("ceph")
}

#[cfg(any(target_os = "linux", test))]
fn from_table(table: &str, path: &Path) -> Option<Volume> {
    let (source, mount, fs) = mount_of(table, path)?;
    // autofs: the share is only mounted while it is in use; until then ask
    // nothing of it, and treat it as the network it most likely is
    let network = is_network_fs(&fs) || fs == "autofs";
    let name = if mount == "/" { "Computer".into() } else { last_component(&mount) };
    Some(Volume { path: String::new(), mount, name, host: if network { host_of(&source) } else { None }, network, online: true })
}

#[cfg(target_os = "linux")]
fn probe(path: &Path) -> Option<Volume> {
    let there = path.ancestors().find(|p| p.exists())?;
    let there = std::fs::canonicalize(there).ok()?;
    from_table(&std::fs::read_to_string("/proc/self/mounts").ok()?, &there)
}

// the mount table, which asks no server; the path as given, as resolving
// its links would
#[cfg(target_os = "linux")]
fn drive_of(path: &Path) -> Option<Volume> {
    from_table(&std::fs::read_to_string("/proc/self/mounts").ok()?, path)
}

// a UNC path, or a drive letter mapped to a share: the drive's type asks
// no server
#[cfg(windows)]
fn drive_of(path: &Path) -> Option<Volume> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDriveTypeW;
    const DRIVE_REMOTE: u32 = 4;
    let p = path.to_string_lossy();
    let unc = p.strip_prefix(r"\\?\UNC\").or_else(|| {
        if p.starts_with(r"\\?\") { None } else { p.strip_prefix(r"\\") }
    });
    let (mount, network) = if let Some(rest) = unc {
        let mut parts = rest.splitn(3, '\\');
        (format!(r"\\{}\{}\", parts.next()?, parts.next().unwrap_or_default()), true)
    } else {
        let drive = p.strip_prefix(r"\\?\").unwrap_or(&p);
        let root = format!("{}\\", drive.get(..2).filter(|d| d.ends_with(':'))?);
        let remote = unsafe { GetDriveTypeW(&HSTRING::from(root.as_str())) } == DRIVE_REMOTE;
        (root, remote)
    };
    let drive = mount.trim_end_matches('\\').to_string();
    let name = drive.rsplit('\\').next().unwrap_or(&drive).to_string();
    Some(Volume { path: String::new(), host: if network && mount.starts_with(r"\\") { host_of(&mount) } else { None },
                  mount, name, network, online: true })
}

#[cfg(windows)]
fn probe(path: &Path) -> Option<Volume> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::{GetDriveTypeW, GetVolumeInformationW, GetVolumePathNameW};
    const DRIVE_REMOTE: u32 = 4;
    let there = path.ancestors().find(|p| p.exists()).unwrap_or(path);
    let mut root = [0u16; 261];
    unsafe { GetVolumePathNameW(&HSTRING::from(there.as_os_str()), &mut root) }.ok()?;
    let len = root.iter().position(|&c| c == 0).unwrap_or(root.len());
    let mount = String::from_utf16_lossy(&root[..len]);
    let at = HSTRING::from(mount.as_str());
    let unc = mount.starts_with(r"\\");
    let network = unc || unsafe { GetDriveTypeW(&at) } == DRIVE_REMOTE;
    let mut label = [0u16; 261];
    let _ = unsafe { GetVolumeInformationW(&at, Some(&mut label), None, None, None, None) };
    let label = String::from_utf16_lossy(&label[..label.iter().position(|&c| c == 0).unwrap_or(0)]);
    let drive = mount.trim_end_matches('\\').to_string();
    let name = if unc { drive.rsplit('\\').next().unwrap_or(&drive).to_string() }
               else if label.is_empty() { drive.clone() } else { format!("{label} ({drive})") };
    Some(Volume { path: String::new(), mount: mount.clone(), name, host: if unc { host_of(&mount) } else { None }, network, online: true })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const TABLE: &str = "\
/dev/nvme0n1p2 / ext4 rw,relatime 0 0
/dev/nvme0n1p1 /boot/efi vfat rw 0 0
//nas.local/Pictures /mnt/Pictures cifs rw,vers=3.0 0 0
nas:/export/photos /mnt/My\\040Photos nfs4 rw 0 0
";

    const OWNER: &str = "\
/dev/nvme0n1p2 / ext4 rw,relatime 0 0
tmpfs /tmp tmpfs rw 0 0
systemd-1 /mnt/synology/Photos_Backup autofs rw,fd=44,pgrp=1,timeout=0,direct 0 0
//SynologyDS220.local/Photos/Photos_Backup /mnt/synology/Photos_Backup cifs rw,vers=3.0 0 0
nas:/export/db /mnt/nfs nfs4 rw 0 0
/dev/sdb1 /mnt/local ext4 rw 0 0
";

    #[test]
    fn a_database_is_refused_on_a_network_share() {
        let r = |p: &str| refusal_from(Some(OWNER), Path::new(p));
        assert!(r("/mnt/synology/Photos_Backup/catalog.sqlite").unwrap().contains("cifs"));
        assert!(r("/mnt/synology/Photos_Backup/new/dir/catalog.sqlite").is_some());
        assert!(r("/mnt/nfs/catalog.sqlite").unwrap().contains("nfs4"));
        assert!(r("/home/me/.local/share/app.immage/catalog.sqlite").is_none());
        assert!(r("/tmp/x/catalog.sqlite").is_none());
        assert!(r("/mnt/local/catalog.sqlite").is_none());
        // idle automount: refused too
        let idle = "/dev/nvme0n1p2 / ext4 rw 0 0\nsystemd-1 /mnt/synology/P autofs rw 0 0\n";
        assert!(refusal_from(Some(idle), Path::new("/mnt/synology/P/c.sqlite")).is_some());
        // mount undetermined: refused under /mnt and /media only
        assert!(refusal_from(None, Path::new("/mnt/x/c.sqlite")).is_some());
        assert!(refusal_from(None, Path::new("/media/me/x/c.sqlite")).is_some());
        assert!(refusal_from(None, Path::new("/home/me/c.sqlite")).is_none());
    }

    #[test]
    fn network_refusal_resolves_missing_paths() {
        let dir = tempfile::tempdir().unwrap();
        assert!(network_refusal(&dir.path().join("not/yet/catalog.sqlite")).is_none());
    }

    #[test]
    fn longest_mount_wins() {
        let v = from_table(TABLE, Path::new("/mnt/Pictures/2026/05")).unwrap();
        assert_eq!(v.mount, "/mnt/Pictures");
        assert_eq!(v.name, "Pictures");
        assert!(v.network);
        assert_eq!(v.host.as_deref(), Some("nas.local"));
        let home = from_table(TABLE, Path::new("/home/me/Pictures/2026")).unwrap();
        assert_eq!(home.mount, "/");
        assert_eq!(home.name, "Computer");
        assert!(!home.network);
        // a mount point that is only a prefix of the name is not its mount
        assert_eq!(from_table(TABLE, Path::new("/mnt/Pictures2")).unwrap().mount, "/");
    }

    #[test]
    fn escaped_mount_and_nfs_host() {
        let v = from_table(TABLE, Path::new("/mnt/My Photos/2026")).unwrap();
        assert_eq!(v.mount, "/mnt/My Photos");
        assert_eq!(v.host.as_deref(), Some("nas"));
    }

    // the startup disk from the mount table, named as Finder names it,
    // without listing every disk
    #[cfg(target_os = "macos")]
    #[test]
    fn startup_disk() {
        let home = std::env::var("HOME").unwrap();
        let v = drive_of(Path::new(&home)).unwrap();
        assert_eq!(v.mount, "/");
        assert!(!v.network);
        assert!(!probe(Path::new(&home)).unwrap().name.is_empty());
        assert!(disk_name("/").is_some());
    }

    // a stuck drive is not tried again, and answers once its stat returns
    #[test]
    fn stuck_drive() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        assert!(answering(&dir));
        let drive = drive_of(&dir).map(|v| v.mount).unwrap_or_default();
        DRIVES.lock().unwrap().entry(drive.clone()).or_default().stuck = true;
        let start = Instant::now();
        assert!(!answering(&dir));
        assert!(start.elapsed() < Duration::from_millis(100));
        DRIVES.lock().unwrap().remove(&drive);
        assert!(answering(&dir));
    }

    const AUTOFS_IDLE: &str = "\
/dev/nvme0n1p2 / ext4 rw,relatime 0 0
systemd-1 /mnt/synology/Photos_Backup autofs rw,relatime,fd=44,pgrp=1,timeout=0,minproto=5,maxproto=5,direct,pipe_ino=19873 0 0
";
    const AUTOFS_MOUNTED: &str = "\
/dev/nvme0n1p2 / ext4 rw,relatime 0 0
systemd-1 /mnt/synology/Photos_Backup autofs rw,relatime,fd=44,pgrp=1,timeout=0,minproto=5,maxproto=5,direct,pipe_ino=19873 0 0
//SynologyDS220.local/Photos/Photos_Backup /mnt/synology/Photos_Backup cifs rw,relatime,vers=3.0,cache=strict,soft,actimeo=1 0 0
";

    // the mounted share is what the path is on; an idle automount is a network too
    #[test]
    fn autofs_is_asked_what_is_behind_it() {
        let p = Path::new("/mnt/synology/Photos_Backup/2023/Japan");
        let v = from_table(AUTOFS_MOUNTED, p).unwrap();
        assert!(v.network);
        assert_eq!(v.host.as_deref(), Some("SynologyDS220.local"));
        let idle = from_table(AUTOFS_IDLE, p).unwrap();
        assert!(idle.network, "nothing mounted yet: assume the network it most likely is");
        assert_eq!(idle.mount, "/mnt/synology/Photos_Backup");
        assert_eq!(mount_of(AUTOFS_IDLE, p).unwrap().2, "autofs");
        assert_eq!(mount_of(AUTOFS_MOUNTED, p).unwrap().2, "cifs");
    }

    #[test]
    fn more_network_file_systems() {
        for fs in ["fuse.gvfsd-fuse", "fuse.mergerfs", "ceph", "glusterfs", "lustre", "fuse.rclone", "fuse.rclone-mount", "nfs4"] {
            assert!(is_network_fs(fs), "{fs}");
        }
        for fs in ["ext4", "btrfs", "xfs", "vfat", "exfat", "tmpfs", "fuse.portal"] {
            assert!(!is_network_fs(fs), "{fs}");
        }
    }

    // many callers on one stuck drive start one stat, on a pooled worker
    #[test]
    fn a_stuck_drive_costs_one_stat_not_a_thread_per_caller() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let started = Arc::new(AtomicUsize::new(0));
        let key = "test-drive-pool";
        let handles: Vec<_> = (0..16).map(|_| {
            let started = started.clone();
            std::thread::spawn(move || {
                answering_with(key, Duration::from_millis(100), move || {
                    started.fetch_add(1, Ordering::SeqCst);
                    std::thread::sleep(Duration::from_millis(600));
                })
            })
        }).collect();
        let answers: Vec<bool> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(answers.iter().all(|a| !a), "a slow stat is not an answer");
        assert_eq!(started.load(Ordering::SeqCst), 1);
        // while stuck nobody waits
        let t = Instant::now();
        assert!(!answering_with(key, Duration::from_secs(5), || panic!("must not stat a stuck drive")));
        assert!(t.elapsed() < Duration::from_millis(100));
        std::thread::sleep(Duration::from_millis(700));
        // it answered in the end, and that answer is fresh
        assert!(answering_with(key, Duration::from_secs(5), || panic!("fresh answer, no new stat")));
        DRIVES.lock().unwrap().remove(key);
    }

    #[test]
    fn hosts() {
        assert_eq!(host_of("//siftu@192.168.0.249/Pictures").as_deref(), Some("192.168.0.249"));
        assert_eq!(host_of(r"\\nas\share\").as_deref(), Some("nas"));
        assert_eq!(host_of("/dev/disk3s5"), None);
    }
}
