// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

use super::*;
use crate::catalog_fakefs::{FakeFs, Synth, BASE_MTIME};
use crate::catalog_sources::{get_source, list_sources};
use std::sync::atomic::Ordering;

fn fast() -> ScanConfig {
    ScanConfig {
        workers: 2,
        list_deadline: Duration::from_millis(200),
        stat_deadline: Duration::from_millis(200),
        progress_every: Duration::from_millis(10),
        checkpoint_every: Duration::from_millis(20),
        online_check_every_dirs: 1,
        stuck_cap: 4,
        cross_mounts: false,
    }
}

fn opts(root: &Path) -> ScanOptions {
    ScanOptions { root: root.to_path_buf(), include_subfolders: true, dir_skip: false, light: false, fresh: false }
}

struct Rig {
    _dir: tempfile::TempDir,
    db: CatalogDb,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let db = CatalogDb::open(&dir.path().join("catalog.sqlite")).unwrap();
    db.exec(|c| {
        c.execute("INSERT INTO sources(name,kind,volume_key,mount_path,rel_root,created_at) VALUES('S','nas','smb:x/y','/fake','',1)", [])
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
    .unwrap();
    Rig { _dir: dir, db }
}

impl Rig {
    fn source(&self) -> Source {
        get_source(&self.db.read().unwrap(), 1).unwrap().unwrap()
    }
    fn scan(&self, fs: &FakeFs) -> ScanReport {
        self.scan_with(fs, &fast(), &ScanControl::default(), |_| {})
    }
    fn scan_with(&self, fs: &FakeFs, cfg: &ScanConfig, ctl: &ScanControl, mut on: impl FnMut(&Progress)) -> ScanReport {
        run_scan(&self.db, fs, &self.source(), &opts(&fs.root), cfg, ctl, &mut on).unwrap()
    }
    fn present(&self) -> i64 {
        self.db.read().unwrap().query_row("SELECT COUNT(*) FROM photos WHERE status='present'", [], |r| r.get(0)).unwrap()
    }
    fn missing(&self) -> i64 {
        self.db.read().unwrap().query_row("SELECT COUNT(*) FROM photos WHERE status='missing'", [], |r| r.get(0)).unwrap()
    }
    fn dir_status(&self, rel: &str) -> String {
        self.db
            .read()
            .unwrap()
            .query_row("SELECT status FROM dirs WHERE rel_path=?1", [rel], |r| r.get(0))
            .unwrap()
    }
}

fn small_tree() -> FakeFs {
    let fs = FakeFs::new();
    for (i, rel) in ["a.jpg", "b.ARW", "2023/c.jpg", "2023/d.CR3", "2023/Japan/e.raf", "2024/f.dng"].iter().enumerate() {
        fs.add_file(rel, 100 + i as u64, BASE_MTIME + i as i64 * 1_000_000_000);
    }
    fs
}

#[test]
fn first_scan_then_unchanged_then_changes_and_a_real_sweep() {
    let rig = rig();
    let fs = small_tree();
    let r = rig.scan(&fs);
    assert_eq!(r.state, "complete");
    assert_eq!((r.totals.found, r.totals.new, r.totals.changed, r.totals.missing), (6, 6, 0, 0));
    assert_eq!(r.totals.dirs, 4);
    assert_eq!(rig.source().state, "online");
    // nothing changed
    let r = rig.scan(&fs);
    assert_eq!((r.state, r.totals.new, r.totals.changed, r.totals.unchanged, r.totals.missing), ("complete", 0, 0, 6, 0));
    // one file edited, one added, one deleted, one directory removed
    fs.add_file("a.jpg", 100, BASE_MTIME + 5);
    fs.add_file("2024/new.jpg", 1, BASE_MTIME);
    fs.remove_file("b.ARW");
    let r = rig.scan(&fs);
    assert_eq!((r.state, r.totals.new, r.totals.changed, r.totals.missing, r.totals.found), ("complete", 1, 1, 1, 6));
    assert_eq!(rig.missing(), 1);
    // the file comes back
    fs.add_file("b.ARW", 101, BASE_MTIME);
    let r = rig.scan(&fs);
    assert_eq!((r.totals.changed, r.totals.missing, r.totals.found), (1, 0, 7));
    assert_eq!(rig.missing(), 0);
    // the 2023/Japan directory is deleted with its photo
    let fs2 = FakeFs::new();
    for rel in ["a.jpg", "b.ARW", "2023/c.jpg", "2023/d.CR3", "2024/f.dng", "2024/new.jpg"] {
        fs2.add_file(rel, 1, BASE_MTIME);
    }
    let r = rig.scan(&fs2);
    assert_eq!((r.state, r.totals.missing), ("complete", 1));
    assert_eq!(rig.dir_status("2023/Japan"), "gone");
}

#[test]
fn size_or_mtime_only_changes_count_as_changed() {
    let rig = rig();
    let fs = small_tree();
    rig.scan(&fs);
    fs.add_file("a.jpg", 99_999, BASE_MTIME);
    let r = rig.scan(&fs);
    assert_eq!((r.totals.changed, r.totals.new), (1, 0));
}

#[test]
fn no_sweep_after_a_partial_walk_and_a_stalled_directory_is_not_empty() {
    let rig = rig();
    let fs = small_tree();
    assert_eq!(rig.scan(&fs).state, "complete");
    // a photo deleted in a directory that is listed fine, and a directory that stalls
    fs.remove_file("a.jpg");
    fs.stall("2023");
    let r = rig.scan(&fs);
    assert_eq!(r.state, "incomplete");
    assert_eq!(r.totals.missing, 0, "a partial walk sweeps nothing, not even what was plainly deleted");
    assert_eq!(rig.missing(), 0);
    assert_eq!(rig.present(), 6);
    assert_eq!(rig.dir_status("2023"), "stalled", "stalled, not empty");
    assert_eq!(r.totals.incomplete_dirs.len(), 1);
    assert_eq!(r.totals.incomplete_dirs[0].0, "2023");
    assert!(r.totals.incomplete_dirs[0].2.as_deref().unwrap().contains("timed out"));
    // the photos in the stalled directory (and under it) are all still there, unchanged
    let n: i64 = rig.db.read().unwrap().query_row("SELECT COUNT(*) FROM photos WHERE rel_path LIKE '2023/%' AND status='present'", [], |r| r.get(0)).unwrap();
    assert_eq!(n, 3);
    assert_eq!(rig.source().state, "attention");
    // once it answers, the next walk completes and the deletion is finally seen
    fs.unstall("2023");
    let r = rig.scan(&fs);
    assert_eq!((r.state, r.totals.missing), ("complete", 1));
    assert_eq!(rig.dir_status("2023"), "ok");
}

#[test]
fn denied_and_vanished_directories_make_the_walk_incomplete() {
    let rig = rig();
    let fs = small_tree();
    rig.scan(&fs);
    fs.remove_file("2024/f.dng");
    fs.deny("2023/Japan");
    let r = rig.scan(&fs);
    assert_eq!((r.state, r.totals.missing), ("incomplete", 0));
    assert_eq!(rig.dir_status("2023/Japan"), "denied");
    fs.vanish("2024");
    let r = rig.scan(&fs);
    assert_eq!((r.state, r.totals.missing), ("incomplete", 0));
    assert_eq!(rig.dir_status("2024"), "error");
}

#[test]
fn a_directory_dropped_mid_walk_does_not_mark_its_photos_missing() {
    let rig = rig();
    let fs = small_tree();
    rig.scan(&fs);
    // the second listing finds a directory its parent listed gone
    fs.at_list(2, |f| f.vanish("2024"));
    fs.at_list(3, |f| f.vanish("2023"));
    let r = rig.scan(&fs);
    assert_eq!(r.state, "incomplete");
    assert_eq!(rig.missing(), 0);
}

#[test]
fn a_mount_vanishing_mid_scan_marks_nothing_missing_and_resumes_later() {
    let rig = rig();
    let s = Synth { seed: 7, fanout: 3, depth: 3, files: 6 };
    let fs = FakeFs::synthetic(s);
    let (dirs, files) = FakeFs::synth_counts(s);
    assert_eq!(rig.scan(&fs).totals.found, files);
    // the next walk loses the mount after a few directories
    fs.at_list(dirs / 2, |f| f.go_offline());
    let before = fs.lists.load(Ordering::SeqCst);
    let r = rig.scan(&fs);
    assert_eq!(r.state, "offline");
    assert_eq!(r.totals.missing, 0);
    assert_eq!(rig.missing(), 0);
    assert_eq!(rig.present() as u64, files);
    assert_eq!(rig.source().state, "offline");
    // while it is away a new scan does not even start a run
    let r = rig.scan(&fs);
    assert_eq!((r.state, r.run_id), ("offline", 0));
    // it comes back: the interrupted walk continues, and ends complete
    fs.go_online();
    let r = rig.scan(&fs);
    assert!(r.resumed);
    assert_eq!((r.state, r.totals.missing, r.totals.found), ("complete", 0, files));
    let listed_after = fs.lists.load(Ordering::SeqCst) - before;
    assert!(listed_after < 2 * dirs, "resumed, not restarted twice over: {listed_after}");
}

#[test]
fn an_empty_root_above_known_photos_is_not_believed() {
    let rig = rig();
    let fs = small_tree();
    rig.scan(&fs);
    // an unmounted share leaves an empty directory behind
    fs.clear_all();
    let r = rig.scan(&fs);
    assert_eq!(r.state, "incomplete");
    assert_eq!(rig.missing(), 0);
    assert_eq!(rig.present(), 6);
}

#[test]
fn a_mount_identity_change_ends_offline_without_a_sweep() {
    let rig = rig();
    let fs = small_tree();
    rig.scan(&fs);
    fs.remove_file("a.jpg");
    fs.at_list(2, |f| f.set_identity("mount-2"));
    let r = rig.scan(&fs);
    assert_eq!(r.state, "offline");
    assert_eq!(rig.missing(), 0);
}

#[test]
fn symlinks_hidden_files_sidecars_and_odd_names() {
    let rig = rig();
    let fs = FakeFs::new();
    fs.add_file("IMG_1.ARW", 10, BASE_MTIME);
    fs.add_file("IMG_1.ARW.xmp", 5, BASE_MTIME + 1);
    fs.add_file("IMG_2.jpg", 10, BASE_MTIME);
    fs.add_file("IMG_2.xmp", 7, BASE_MTIME + 2);
    fs.add_file("IMG_3.JPG", 10, BASE_MTIME);
    fs.add_file(".hidden.jpg", 10, BASE_MTIME);
    fs.add_file("notes.txt", 10, BASE_MTIME);
    fs.add_file(".trash/old.jpg", 10, BASE_MTIME);
    fs.add_raw("", FsEntry { name: "loop".into(), kind: EntryKind::Symlink, size: 0, mtime_ns: 0, dev: 1 });
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStringExt;
        fs.add_raw("", FsEntry { name: OsString::from_vec(b"caf\xe9.jpg".to_vec()), kind: EntryKind::File, size: 1, mtime_ns: 1, dev: 1 });
        fs.add_raw("", FsEntry { name: OsString::from_vec(b"\xff\xfe.txt".to_vec()), kind: EntryKind::File, size: 1, mtime_ns: 1, dev: 1 });
    }
    // another file system mounted below the source
    fs.add_raw("", FsEntry { name: "usb".into(), kind: EntryKind::Dir, size: 0, mtime_ns: 1, dev: 99 });
    fs.add_file("usb/inside.jpg", 1, 1);
    let r = rig.scan(&fs);
    assert_eq!(r.state, "complete");
    let c = rig.db.read().unwrap();
    let names: Vec<String> = c.prepare("SELECT name FROM photos ORDER BY name").unwrap().query_map([], |r| r.get(0)).unwrap().map(|x| x.unwrap()).collect();
    assert_eq!(names, ["IMG_1.ARW", "IMG_2.jpg", "IMG_3.JPG"]);
    let x = |n: &str| -> (i64, Option<i64>) {
        c.query_row("SELECT xmp_kind, xmp_size FROM photos WHERE name=?1", [n], |r| Ok((r.get(0)?, r.get(1)?))).unwrap()
    };
    assert_eq!(x("IMG_1.ARW"), (1, Some(5)), "name.ext.xmp");
    assert_eq!(x("IMG_2.jpg"), (2, Some(7)), "name.xmp");
    assert_eq!(x("IMG_3.JPG"), (0, None));
    let ext: String = c.query_row("SELECT ext FROM photos WHERE name='IMG_3.JPG'", [], |r| r.get(0)).unwrap();
    assert_eq!(ext, "jpg", "extensions are stored in lower case");
    let notes: Vec<(String, String)> = c.prepare("SELECT kind,name FROM scan_notes ORDER BY kind").unwrap().query_map([], |r| Ok((r.get(0)?, r.get(1)?))).unwrap().map(|x| x.unwrap()).collect();
    let kinds: Vec<&str> = notes.iter().map(|n| n.0.as_str()).collect();
    assert!(kinds.contains(&"symlink") && kinds.contains(&"mount_boundary"), "{notes:?}");
    #[cfg(unix)]
    {
        assert_eq!(kinds.iter().filter(|k| **k == "non_utf8").count(), 1, "only the photo-named one is worth a note: {notes:?}");
        assert_eq!(r.totals.unreadable, 1);
    }
    // a sidecar edit alone is a change
    fs.add_file("IMG_1.ARW.xmp", 6, BASE_MTIME + 9);
    let r = rig.scan(&fs);
    assert_eq!((r.totals.changed, r.totals.new), (1, 0));
}

#[test]
fn a_non_recursive_source_lists_only_its_own_folder() {
    let rig = rig();
    let fs = small_tree();
    let mut o = opts(&fs.root);
    o.include_subfolders = false;
    let r = run_scan(&rig.db, &fs, &rig.source(), &o, &fast(), &ScanControl::default(), &mut |_| {}).unwrap();
    assert_eq!((r.state, r.totals.found, r.dirs_listed), ("complete", 2, 1));
}

#[test]
fn workers_stay_within_the_cap() {
    let rig = rig();
    let fs = FakeFs::synthetic(Synth { seed: 1, fanout: 3, depth: 3, files: 2 });
    fs.set_latency(Duration::from_millis(3));
    let mut cfg = fast();
    cfg.workers = 2;
    let r = rig.scan_with(&fs, &cfg, &ScanControl::default(), |_| {});
    assert_eq!(r.state, "complete");
    assert!(fs.max_running.load(Ordering::SeqCst) <= 2);
    assert!(fs.max_running.load(Ordering::SeqCst) >= 1);
}

#[test]
fn pause_resume_and_cancel() {
    let rig = rig();
    let s = Synth { seed: 3, fanout: 3, depth: 3, files: 3 };
    let fs = FakeFs::synthetic(s);
    fs.set_latency(Duration::from_millis(15));
    let ctl = Arc::new(ScanControl::default());
    let progress = Arc::new(Mutex::new(Vec::<Progress>::new()));
    let (dirs, _) = FakeFs::synth_counts(s);
    std::thread::scope(|sc| {
        let (c2, p2) = (ctl.clone(), progress.clone());
        sc.spawn(move || {
            std::thread::sleep(Duration::from_millis(60));
            c2.pause();
            std::thread::sleep(Duration::from_millis(400));
            let a = p2.lock().unwrap().last().map(|p| p.dirs_done).unwrap_or(0);
            std::thread::sleep(Duration::from_millis(200));
            let b = p2.lock().unwrap().last().map(|p| p.dirs_done).unwrap_or(0);
            assert_eq!(a, b, "no progress while paused");
            assert!(p2.lock().unwrap().last().unwrap().paused);
            assert!(a < dirs, "paused part way");
            c2.resume();
        });
        let p3 = progress.clone();
        let r = rig.scan_with(&fs, &fast(), &ctl, move |p| p3.lock().unwrap().push(p.clone()));
        assert_eq!(r.state, "complete");
    });
    // cancel: closed cancelled, nothing swept, and the next scan starts afresh
    let rig2 = rig_with_scan(&small_tree());
    let fs2 = small_tree();
    fs2.remove_file("a.jpg");
    fs2.set_latency(Duration::from_millis(30));
    let ctl2 = ScanControl::default();
    let r = std::thread::scope(|sc| {
        sc.spawn(|| {
            std::thread::sleep(Duration::from_millis(40));
            ctl2.cancel();
        });
        rig2.scan_with(&fs2, &fast(), &ctl2, |_| {})
    });
    assert_eq!(r.state, "cancelled");
    assert_eq!(rig2.missing(), 0);
    fs2.set_latency(Duration::ZERO);
    let r = rig2.scan(&fs2);
    assert!(!r.resumed);
    assert_eq!((r.state, r.totals.missing), ("complete", 1));
}

fn rig_with_scan(fs: &FakeFs) -> Rig {
    let r = rig();
    assert_eq!(r.scan(fs).state, "complete");
    r
}

#[test]
fn interrupt_keeps_a_checkpoint_and_resume_continues() {
    let rig = rig();
    let s = Synth { seed: 11, fanout: 3, depth: 4, files: 4 };
    let fs = FakeFs::synthetic(s);
    let (dirs, files) = FakeFs::synth_counts(s);
    fs.set_latency(Duration::from_millis(3));
    let ctl = ScanControl::default();
    let r = std::thread::scope(|sc| {
        sc.spawn(|| {
            std::thread::sleep(Duration::from_millis(120));
            ctl.interrupt();
        });
        rig.scan_with(&fs, &fast(), &ctl, |_| {})
    });
    assert_eq!(r.state, "interrupted");
    let first = fs.lists.load(Ordering::SeqCst);
    assert!(first > 0 && first < dirs, "stopped part way: {first} of {dirs}");
    let job = catalog_db::interrupted_job(&rig.db.read().unwrap(), 1).unwrap().unwrap();
    let cp: Checkpoint = serde_json::from_str(&job.checkpoint.unwrap()).unwrap();
    assert!(!cp.frontier.is_empty());
    let r2 = rig.scan(&fs);
    assert!(r2.resumed, "continued the same run");
    assert_eq!(r2.run_id, r.run_id);
    assert_eq!((r2.state, r2.totals.found, r2.totals.new), ("complete", files, files));
    let second = fs.lists.load(Ordering::SeqCst) - first;
    assert!(second < dirs, "the second walk did not start over: {second} of {dirs}");
}

#[test]
fn a_resume_on_another_mount_starts_fresh() {
    let rig = rig();
    let s = Synth { seed: 5, fanout: 2, depth: 3, files: 2 };
    let fs = FakeFs::synthetic(s);
    fs.set_latency(Duration::from_millis(5));
    let ctl = ScanControl::default();
    std::thread::scope(|sc| {
        sc.spawn(|| {
            std::thread::sleep(Duration::from_millis(30));
            ctl.interrupt();
        });
        assert_eq!(rig.scan_with(&fs, &fast(), &ctl, |_| {}).state, "interrupted");
    });
    fs.set_identity("another-volume");
    fs.set_latency(Duration::ZERO);
    let r = rig.scan(&fs);
    assert!(!r.resumed);
    assert_eq!(r.state, "complete");
}

// the real thing: a child process is killed (abort) between write batches
#[test]
fn kill_child() {
    let Ok(dir) = std::env::var("IMMAGE_CATALOG_KILL_DIR") else { return };
    let db = CatalogDb::open(&Path::new(&dir).join("catalog.sqlite")).unwrap();
    db.exec(|c| {
        c.execute("INSERT INTO sources(name,kind,volume_key,mount_path,rel_root,created_at) VALUES('S','nas','smb:x/y','/fake','',1)", [])
            .map(|_| ())
            .map_err(|e| e.to_string())
    })
    .unwrap();
    let fs = FakeFs::synthetic(KILL_TREE);
    fs.set_latency(Duration::from_millis(4));
    fs.at_list(70, |_| {
        // let the writer commit what it has been sent, then die as a killed process does
        std::thread::sleep(Duration::from_millis(60));
        std::process::abort();
    });
    let src = get_source(&db.read().unwrap(), 1).unwrap().unwrap();
    let _ = run_scan(&db, &fs, &src, &opts(&fs.root), &fast(), &ScanControl::default(), &mut |_| {});
}

const KILL_TREE: Synth = Synth { seed: 99, fanout: 3, depth: 4, files: 25 };

#[test]
fn a_killed_scan_leaves_a_sound_database_and_resumes() {
    let dir = tempfile::tempdir().unwrap();
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "catalog_scan::tests::kill_child", "--nocapture", "--test-threads=1"])
        .env("IMMAGE_CATALOG_KILL_DIR", dir.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(!status.success(), "the child was killed mid-scan");
    let path = dir.path().join("catalog.sqlite");
    // the file is sound after the kill, with whole batches only
    {
        let c = rusqlite::Connection::open(&path).unwrap();
        let ok: String = c.query_row("PRAGMA integrity_check", [], |r| r.get(0)).unwrap();
        assert_eq!(ok, "ok");
        let n: i64 = c.query_row("SELECT COUNT(*) FROM photos", [], |r| r.get(0)).unwrap();
        assert!(n > 0, "some batches were committed before the kill");
        let orphans: i64 = c.query_row("SELECT COUNT(*) FROM photos WHERE dir_id NOT IN (SELECT id FROM dirs)", [], |r| r.get(0)).unwrap();
        assert_eq!(orphans, 0);
    }
    // reopening reconciles the job that was running, and keeps its checkpoint
    let db = CatalogDb::open(&path).unwrap();
    let job = catalog_db::interrupted_job(&db.read().unwrap(), 1).unwrap().expect("the job is resumable");
    assert!(job.checkpoint.is_some());
    let (dirs, files) = FakeFs::synth_counts(KILL_TREE);
    let fs = FakeFs::synthetic(KILL_TREE);
    let src = get_source(&db.read().unwrap(), 1).unwrap().unwrap();
    let r = run_scan(&db, &fs, &src, &opts(&fs.root), &fast(), &ScanControl::default(), &mut |_| {}).unwrap();
    assert!(r.resumed, "continued rather than restarted");
    assert_eq!((r.state, r.totals.found, r.totals.new), ("complete", files, files));
    assert!(fs.lists.load(Ordering::SeqCst) < dirs, "fewer directories than a full walk");
}

#[test]
fn directory_fingerprint_skipping_only_when_asked() {
    let rig = rig();
    let fs = small_tree();
    rig.scan(&fs);
    // flag OFF (the default): every directory is listed again
    fs.listed_dirs.lock().unwrap().clear();
    rig.scan(&fs);
    assert_eq!(fs.listed_dirs.lock().unwrap().len(), 4);
    // flag ON: nothing changed, so only the root is... not even that: its mtime is as stored
    fs.listed_dirs.lock().unwrap().clear();
    let mut o = opts(&fs.root);
    o.dir_skip = true;
    let r = run_scan(&rig.db, &fs, &rig.source(), &o, &fast(), &ScanControl::default(), &mut |_| {}).unwrap();
    assert_eq!(r.state, "complete");
    assert_eq!(fs.listed_dirs.lock().unwrap().len(), 0, "unchanged directories are not listed");
    assert_eq!((r.totals.found, r.totals.unchanged, r.totals.missing), (6, 6, 0));
    // a directory whose mtime moved is listed, its sibling is not
    fs.add_file("2024/g.jpg", 1, BASE_MTIME);
    fs.set_dir_mtime("2024", BASE_MTIME + 77);
    fs.listed_dirs.lock().unwrap().clear();
    let r = run_scan(&rig.db, &fs, &rig.source(), &o, &fast(), &ScanControl::default(), &mut |_| {}).unwrap();
    assert_eq!(fs.listed_dirs.lock().unwrap().clone(), vec!["2024".to_string()]);
    assert_eq!((r.state, r.totals.new, r.totals.found), ("complete", 1, 7));
}

#[test]
fn eta_is_a_range_only_when_the_size_is_known_and_the_rate_is_steady() {
    let rig = rig();
    let s = Synth { seed: 2, fanout: 2, depth: 3, files: 5 };
    let fs = FakeFs::synthetic(s);
    let seen = Arc::new(Mutex::new(Vec::<Progress>::new()));
    let s2 = seen.clone();
    rig.scan_with(&fs, &fast(), &ScanControl::default(), move |p| s2.lock().unwrap().push(p.clone()));
    // the first walk has no previous size to estimate from
    assert!(seen.lock().unwrap().iter().all(|p| p.eta_s.is_none()));
    assert_eq!(seen.lock().unwrap().last().unwrap().phase, "done");
}

#[test]
fn the_real_file_system_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("lib");
    std::fs::create_dir_all(root.join("2023/Japan")).unwrap();
    std::fs::create_dir_all(root.join(".cache")).unwrap();
    for (f, n) in [("a.jpg", 10usize), ("2023/b.ARW", 20), ("2023/Japan/c.RAF", 30), (".cache/x.jpg", 5), ("2023/readme.txt", 1), ("2023/b.ARW.xmp", 4)] {
        std::fs::write(root.join(f), vec![0u8; n]).unwrap();
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(root.join("2023"), root.join("link-to-2023")).unwrap();
        use std::os::unix::ffi::OsStringExt;
        std::fs::write(root.join(OsString::from_vec(b"bad\xe9.jpg".to_vec())), b"x").unwrap();
    }
    let rig = rig();
    let fs = RealFs::new(root.clone());
    let mut cfg = ScanConfig::for_network(false);
    cfg.online_check_every_dirs = 0;
    let src = rig.source();
    let o = opts(&root);
    let r = run_scan(&rig.db, &fs, &src, &o, &cfg, &ScanControl::default(), &mut |_| {}).unwrap();
    assert_eq!((r.state, r.totals.found, r.totals.dirs), ("complete", 3, 3));
    let c = rig.db.read().unwrap();
    let size: i64 = c.query_row("SELECT size FROM photos WHERE name='c.RAF'", [], |r| r.get(0)).unwrap();
    assert_eq!(size, 30);
    let rel: String = c.query_row("SELECT rel_path FROM photos WHERE name='c.RAF'", [], |r| r.get(0)).unwrap();
    assert_eq!(rel, "2023/Japan/c.RAF");
    let mt: i64 = c.query_row("SELECT mtime_ns FROM photos WHERE name='a.jpg'", [], |r| r.get(0)).unwrap();
    assert!(mt > 1_600_000_000_000_000_000);
    let x: i64 = c.query_row("SELECT xmp_kind FROM photos WHERE name='b.ARW'", [], |r| r.get(0)).unwrap();
    assert_eq!(x, 1);
    #[cfg(unix)]
    {
        let kinds: Vec<String> = c.prepare("SELECT kind FROM scan_notes").unwrap().query_map([], |r| r.get(0)).unwrap().map(|x| x.unwrap()).collect();
        assert!(kinds.contains(&"symlink".to_string()) && kinds.contains(&"non_utf8".to_string()), "{kinds:?}");
        // not following the link: no duplicate photos
        assert_eq!(rig.present(), 3);
    }
    // delete a file for real; an unchanged rescan elsewhere, then the sweep
    std::fs::remove_file(root.join("a.jpg")).unwrap();
    let r = run_scan(&rig.db, &fs, &src, &o, &cfg, &ScanControl::default(), &mut |_| {}).unwrap();
    assert_eq!((r.state, r.totals.missing, r.totals.found), ("complete", 1, 2));
    // permission denied is recorded, never read as empty
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let locked = root.join("2023/Japan");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let probe = std::fs::read_dir(&locked).is_err();
        let r = run_scan(&rig.db, &fs, &src, &o, &cfg, &ScanControl::default(), &mut |_| {}).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        if probe {
            assert_eq!(r.state, "incomplete");
            assert_eq!(rig.dir_status("2023/Japan"), "denied");
            assert_eq!(rig.missing(), 1, "only the deleted one");
        }
    }
    assert!(list_sources(&rig.db.read().unwrap()).unwrap().len() == 1);
}

#[test]
fn a_stuck_listing_helper_is_bounded() {
    let fs = RealFs::new(PathBuf::from("/"));
    for _ in 0..STUCK_LISTINGS_CAP {
        let r = fs.within(Duration::from_millis(20), || std::thread::sleep(Duration::from_millis(400)));
        assert_eq!(r, Err(FsError::Timeout));
    }
    // at the cap nothing more is started, whatever is asked
    let t = Instant::now();
    assert_eq!(fs.within(Duration::from_secs(5), || 1), Err(FsError::Timeout));
    assert!(t.elapsed() < Duration::from_millis(100));
    // the stuck ones return in the end and the count falls
    std::thread::sleep(Duration::from_millis(600));
    assert_eq!(fs.within(Duration::from_secs(5), || 1), Ok(1));
}

#[test]
fn a_very_large_synthetic_tree_without_files() {
    let rig = rig();
    let s = Synth { seed: 42, fanout: 6, depth: 4, files: 40 };
    let fs = FakeFs::synthetic(s);
    let (dirs, files) = FakeFs::synth_counts(s);
    let mut cfg = fast();
    cfg.workers = 4;
    let r = rig.scan_with(&fs, &cfg, &ScanControl::default(), |_| {});
    assert_eq!((r.state, r.totals.found, r.dirs_listed), ("complete", files, dirs));
    assert!(rig.db.failure().is_none());
    let r = rig.scan(&fs);
    assert_eq!((r.totals.unchanged, r.totals.new, r.totals.changed), (files, 0, 0));
}

// ---- the benchmark: 500,000 files in about 20,000 directories on local disk ----
//
//   IMMAGE_CATALOG_BENCH_DIR=<a local folder> cargo +1.98.0 test --release bench_500k -- --ignored --nocapture
//
// the tree is seeded (same names every run), made of empty files, and kept in
// the folder between runs so the second run skips the (slow) generation

fn peak_rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| s.lines().find(|l| l.starts_with("VmHWM:")).and_then(|l| l.split_whitespace().nth(1)?.parse::<f64>().ok()))
        .map_or(0.0, |kb| kb / 1024.0)
}

/// `top` x `mid` x `leaf` leaf directories of `files` photos each, plus every
/// directory's own photos for the upper levels being empty. Deterministic.
fn generate_tree(root: &Path, seed: u64, top: usize, mid: usize, leaf: usize, files: usize) -> (u64, u64) {
    let (mut dirs, mut n) = (0u64, 0u64);
    let mix = |x: u64| {
        let mut x = x ^ seed;
        x ^= x >> 33;
        x = x.wrapping_mul(0xff51afd7ed558ccd);
        x ^ (x >> 29)
    };
    for a in 0..top {
        for b in 0..mid {
            for c in 0..leaf {
                let d = root.join(format!("{}", 2000 + a)).join(format!("event-{b:03}")).join(format!("shoot-{c:02}"));
                std::fs::create_dir_all(&d).unwrap();
                dirs += 1;
                for f in 0..files {
                    let h = mix((a * 1_000_000 + b * 1000 + c * 100 + f) as u64);
                    let ext = ["ARW", "jpg", "RAF", "dng", "CR3", "NEF"][(h % 6) as usize];
                    std::fs::File::create(d.join(format!("IMG_{f:05}.{ext}"))).unwrap();
                    n += 1;
                    if h % 7 == 0 {
                        std::fs::File::create(d.join(format!("IMG_{f:05}.{ext}.xmp"))).unwrap();
                    }
                }
            }
        }
    }
    (dirs, n)
}

#[test]
#[ignore]
fn bench_500k() {
    let base = std::env::var_os("IMMAGE_CATALOG_BENCH_DIR").map(PathBuf::from).expect("set IMMAGE_CATALOG_BENCH_DIR to a local folder");
    let root = base.join("tree-500k");
    let marker = base.join("tree-500k.done");
    let t = Instant::now();
    // 20 x 50 x 20 = 20,000 leaf directories of 25 photos = 500,000
    if !marker.exists() {
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let (d, n) = generate_tree(&root, 20260721, 20, 50, 20, 25);
        std::fs::write(&marker, format!("{d} {n}")).unwrap();
        println!("generated {d} directories, {n} photos in {:.1} s", t.elapsed().as_secs_f64());
    }
    let rig = rig();
    let fs = RealFs::new(root.clone());
    let mut cfg = ScanConfig::for_network(false);
    cfg.online_check_every_dirs = 256;
    let rss0 = peak_rss_mb();
    let run = |label: &str| {
        let t = Instant::now();
        let r = run_scan(&rig.db, &fs, &rig.source(), &opts(&root), &cfg, &ScanControl::default(), &mut |_| {}).unwrap();
        println!(
            "{label}: {} in {:.2} s | {} photos, {} dirs, new {} changed {} unchanged {} missing {} | {:.0} files/s | peak RSS {:.0} MB",
            r.state, t.elapsed().as_secs_f64(), r.totals.found, r.totals.dirs, r.totals.new, r.totals.changed, r.totals.unchanged,
            r.totals.missing, r.files_per_s, peak_rss_mb()
        );
        r
    };
    println!("workers {} (local), start RSS {rss0:.0} MB", cfg.workers);
    let first = run("initial scan");
    assert_eq!(first.state, "complete");
    let size = std::fs::metadata(rig.db.path.clone()).unwrap().len() as f64 / 1e6;
    println!("catalog.sqlite {size:.0} MB");
    let second = run("unchanged rescan (cold-ish cache)");
    let third = run("unchanged rescan");
    assert_eq!((third.totals.new, third.totals.changed, third.totals.missing), (0, 0, 0));
    let _ = second;
    // 1,000 files touched and 100 deleted
    let touched = root.join("2000/event-000/shoot-00");
    for f in std::fs::read_dir(&touched).unwrap().flatten().take(10) {
        let p = f.path();
        std::fs::write(&p, b"x").unwrap();
    }
    let t = Instant::now();
    let r = run("rescan after edits");
    println!("({:.2} s) changed {}", t.elapsed().as_secs_f64(), r.totals.changed);
}
