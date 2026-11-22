// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! A fault-injecting file system for the catalog scanner's tests: an in-memory
//! tree (or a seeded synthetic one too big to keep) with latency, stalled and
//! denied directories, directories that vanish, and a mount that flips offline
//! after a number of listings.

use crate::catalog_scan::{EntryKind, FsEntry, FsError, ScanFs};
use std::collections::{BTreeMap, HashSet};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::Duration;

#[derive(Clone, Copy)]
pub struct Synth {
    pub seed: u64,
    pub fanout: usize,
    pub depth: usize,
    pub files: usize,
}

type Trigger = Box<dyn Fn(&FakeFs) + Send + Sync>;

pub struct FakeFs {
    pub root: PathBuf,
    tree: Mutex<BTreeMap<String, Vec<FsEntry>>>,
    synth: Option<Synth>,
    pub latency: Mutex<Duration>,
    stall: Mutex<HashSet<String>>,
    deny: Mutex<HashSet<String>>,
    vanished: Mutex<HashSet<String>>,
    pub online: AtomicBool,
    ident: Mutex<String>,
    pub lists: AtomicU64,
    pub listed_dirs: Mutex<Vec<String>>,
    triggers: Mutex<Vec<(u64, Trigger)>>,
    running: AtomicUsize,
    pub max_running: AtomicUsize,
}

fn mix(mut x: u64) -> u64 {
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51afd7ed558ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ceb9fe1a85ec53);
    x ^ (x >> 33)
}

fn hash_str(seed: u64, s: &str) -> u64 {
    s.bytes().fold(seed, |h, b| mix(h ^ b as u64))
}

pub const BASE_MTIME: i64 = 1_700_000_000_000_000_000;

impl FakeFs {
    pub fn new() -> Self {
        Self {
            root: PathBuf::from("/fake"),
            tree: Mutex::new(BTreeMap::from([(String::new(), Vec::new())])),
            synth: None,
            latency: Mutex::new(Duration::ZERO),
            stall: Default::default(),
            deny: Default::default(),
            vanished: Default::default(),
            online: AtomicBool::new(true),
            ident: Mutex::new("mount-1".into()),
            lists: AtomicU64::new(0),
            listed_dirs: Default::default(),
            triggers: Default::default(),
            running: AtomicUsize::new(0),
            max_running: AtomicUsize::new(0),
        }
    }

    pub fn synthetic(s: Synth) -> Self {
        let mut f = Self::new();
        f.synth = Some(s);
        f
    }

    /// Number of directories (and files) a synthetic tree has.
    pub fn synth_counts(s: Synth) -> (u64, u64) {
        let dirs: u64 = (0..=s.depth as u32).map(|d| (s.fanout as u64).pow(d)).sum();
        (dirs, dirs * s.files as u64)
    }

    fn entry(name: &str, kind: EntryKind, size: u64, mtime: i64) -> FsEntry {
        FsEntry { name: OsString::from(name), kind, size, mtime_ns: mtime, dev: 1 }
    }

    pub fn add_dir(&self, rel: &str) {
        let mut t = self.tree.lock().unwrap();
        let mut cur = String::new();
        for c in rel.split('/').filter(|c| !c.is_empty()) {
            let next = if cur.is_empty() { c.to_string() } else { format!("{cur}/{c}") };
            t.entry(cur.clone()).or_default();
            if !t[&cur].iter().any(|e| e.name == c) {
                t.get_mut(&cur).unwrap().push(Self::entry(c, EntryKind::Dir, 0, BASE_MTIME));
            }
            t.entry(next.clone()).or_default();
            cur = next;
        }
    }

    pub fn add_file(&self, rel: &str, size: u64, mtime: i64) {
        let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel));
        self.add_dir(dir);
        let mut t = self.tree.lock().unwrap();
        let list = t.get_mut(dir).unwrap();
        list.retain(|e| e.name != name);
        list.push(Self::entry(name, EntryKind::File, size, mtime));
    }

    pub fn add_raw(&self, dir: &str, e: FsEntry) {
        self.add_dir(dir);
        self.tree.lock().unwrap().get_mut(dir).unwrap().push(e);
    }

    pub fn remove_file(&self, rel: &str) {
        let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel));
        if let Some(l) = self.tree.lock().unwrap().get_mut(dir) {
            l.retain(|e| e.name != name);
        }
    }

    pub fn clear_all(&self) {
        let mut t = self.tree.lock().unwrap();
        t.clear();
        t.insert(String::new(), Vec::new());
    }

    pub fn set_dir_mtime(&self, rel: &str, mtime: i64) {
        let (parent, name) = rel.rsplit_once('/').unwrap_or(("", rel));
        if let Some(l) = self.tree.lock().unwrap().get_mut(parent) {
            for e in l.iter_mut().filter(|e| e.name == name) {
                e.mtime_ns = mtime;
            }
        }
    }

    pub fn stall(&self, rel: &str) {
        self.stall.lock().unwrap().insert(rel.into());
    }
    pub fn unstall(&self, rel: &str) {
        self.stall.lock().unwrap().remove(rel);
    }
    pub fn deny(&self, rel: &str) {
        self.deny.lock().unwrap().insert(rel.into());
    }
    pub fn vanish(&self, rel: &str) {
        self.vanished.lock().unwrap().insert(rel.into());
    }
    pub fn set_latency(&self, d: Duration) {
        *self.latency.lock().unwrap() = d;
    }
    pub fn set_identity(&self, s: &str) {
        *self.ident.lock().unwrap() = s.into();
    }
    /// Runs `f` when the n-th listing from now is asked for.
    pub fn at_list(&self, n: u64, f: impl Fn(&FakeFs) + Send + Sync + 'static) {
        let at = self.lists.load(Ordering::SeqCst) + n;
        self.triggers.lock().unwrap().push((at, Box::new(f)));
    }
    pub fn go_offline(&self) {
        self.online.store(false, Ordering::SeqCst);
    }
    pub fn go_online(&self) {
        self.online.store(true, Ordering::SeqCst);
    }

    fn rel_of(&self, dir: &Path) -> String {
        dir.strip_prefix(&self.root).unwrap_or(Path::new("")).to_string_lossy().into_owned()
    }

    fn synth_entries(&self, s: Synth, rel: &str) -> Vec<FsEntry> {
        let depth = rel.split('/').filter(|c| !c.is_empty()).count();
        let mut out = Vec::new();
        if depth < s.depth {
            for i in 0..s.fanout {
                out.push(Self::entry(&format!("d{i}"), EntryKind::Dir, 0, BASE_MTIME));
            }
        }
        for i in 0..s.files {
            let h = hash_str(s.seed, &format!("{rel}/{i}"));
            let ext = ["ARW", "jpg", "RAF", "dng", "CR3"][(h % 5) as usize];
            out.push(Self::entry(&format!("IMG_{i:05}.{ext}"), EntryKind::File, 1_000 + h % 50_000_000, BASE_MTIME + (h % 1_000_000) as i64 * 1_000));
        }
        out
    }
}

impl ScanFs for FakeFs {
    fn list(&self, dir: &Path, deadline: Duration) -> Result<Vec<FsEntry>, FsError> {
        let n = self.lists.fetch_add(1, Ordering::SeqCst) + 1;
        let fire: Vec<usize> = {
            let t = self.triggers.lock().unwrap();
            t.iter().enumerate().filter(|(_, (at, _))| *at == n).map(|(i, _)| i).collect()
        };
        for i in fire {
            let triggers = self.triggers.lock().unwrap();
            (triggers[i].1)(self);
        }
        let now = self.running.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_running.fetch_max(now, Ordering::SeqCst);
        let lat = *self.latency.lock().unwrap();
        if !lat.is_zero() {
            std::thread::sleep(lat);
        }
        let rel = self.rel_of(dir);
        self.listed_dirs.lock().unwrap().push(rel.clone());
        let r = (|| {
            if !self.online.load(Ordering::SeqCst) {
                return Err(FsError::Io("Transport endpoint is not connected".into()));
            }
            if self.stall.lock().unwrap().contains(&rel) {
                return Err(FsError::Timeout);
            }
            let _ = deadline;
            if self.deny.lock().unwrap().contains(&rel) {
                return Err(FsError::Denied);
            }
            if self.vanished.lock().unwrap().contains(&rel) {
                return Err(FsError::NotFound);
            }
            if let Some(s) = self.synth {
                return Ok(self.synth_entries(s, &rel));
            }
            self.tree.lock().unwrap().get(&rel).cloned().ok_or(FsError::NotFound)
        })();
        self.running.fetch_sub(1, Ordering::SeqCst);
        r
    }

    fn stat_dir(&self, dir: &Path, _deadline: Duration) -> Result<(i64, u64), FsError> {
        let rel = self.rel_of(dir);
        if !self.online.load(Ordering::SeqCst) {
            return Err(FsError::Io("not connected".into()));
        }
        let (parent, name) = rel.rsplit_once('/').unwrap_or(("", rel.as_str()));
        if rel.is_empty() {
            return Ok((BASE_MTIME, 1));
        }
        if self.synth.is_some() {
            return Ok((BASE_MTIME, 1));
        }
        self.tree
            .lock()
            .unwrap()
            .get(parent)
            .and_then(|l| l.iter().find(|e| e.name == name).map(|e| (e.mtime_ns, e.dev)))
            .ok_or(FsError::NotFound)
    }

    fn online(&self) -> bool {
        self.online.load(Ordering::SeqCst)
    }

    fn identity(&self) -> Option<String> {
        Some(self.ident.lock().unwrap().clone())
    }
}
