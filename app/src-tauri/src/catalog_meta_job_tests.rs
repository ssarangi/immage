// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

use super::*;
use crate::catalog_db::CatalogDb;
use crate::catalog_fakefs::{FakeFs, BASE_MTIME};
use crate::catalog_meta_fixtures::*;
use crate::catalog_scan::{run_scan, ScanConfig, ScanOptions};
use crate::catalog_sources::get_source;
use std::path::Path;

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

fn spec_n(n: u32) -> Spec {
    Spec { datetime: leak(format!("2025:10:05 14:{:02}:{:02}", (n / 60) % 60, n % 60)), serial: Some(leak(format!("SN{n}"))), ..Spec::default() }
}

fn jpeg_n(n: u32) -> Vec<u8> {
    make_jpeg(Some(&spec_n(n)), None, 600, 400, n as usize % 50)
}

fn fast() -> MetaConfig {
    let mut c = MetaConfig::for_network(false);
    c.workers = 2;
    c.read_deadline = Duration::from_millis(500);
    c.flush_every = Duration::from_millis(20);
    c.progress_every = Duration::from_millis(10);
    c.checkpoint_every = Duration::from_millis(20);
    c
}

fn scan_cfg() -> ScanConfig {
    let mut c = ScanConfig::for_network(false);
    c.workers = 2;
    c.progress_every = Duration::from_millis(10);
    c.checkpoint_every = Duration::from_millis(20);
    c
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
    fn scan(&self, fs: &FakeFs) {
        let o = ScanOptions { root: fs.root.clone(), include_subfolders: true, dir_skip: false, light: false, fresh: false };
        let r = run_scan(&self.db, fs, &self.source(), &o, &scan_cfg(), &ScanControl::default(), &mut |_| {}).unwrap();
        assert_eq!(r.state, "complete");
    }
    fn meta(&self, fs: &FakeFs) -> MetaReport {
        self.meta_with(fs, &fast(), &ScanControl::default(), |_| {})
    }
    fn meta_with(&self, fs: &FakeFs, cfg: &MetaConfig, ctl: &ScanControl, mut on: impl FnMut(&MetaProgress)) -> MetaReport {
        run_metadata(&self.db, fs, &self.source(), cfg, ctl, &mut on).unwrap()
    }
    fn q<T: rusqlite::types::FromSql>(&self, sql: &str) -> T {
        self.db.read().unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
    }
    fn state_of(&self, rel: &str) -> i64 {
        self.db.read().unwrap().query_row("SELECT meta_state FROM photos WHERE rel_path=?1", [rel], |r| r.get(0)).unwrap()
    }
}

fn fs_with(files: &[(&str, Vec<u8>)]) -> FakeFs {
    let fs = FakeFs::new();
    for (rel, b) in files {
        fs.add_blob(rel, b.clone(), BASE_MTIME);
    }
    fs
}

#[test]
fn reads_the_head_and_fills_the_columns() {
    let r = rig();
    let fs = fs_with(&[
        ("a.ARW", arw_like(true, &Spec::default(), 135_330, 396_841, 600_000)),
        ("b.jpg", make_jpeg(Some(&Spec::default()), None, 4000, 3000, 0)),
        ("c.heic", make_heic(&Spec::default(), None, 0)),
        ("d.cr3", make_cr3(&Spec::default(), 400).0),
    ]);
    r.scan(&fs);
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE meta_state=0"), 4);
    let rep = r.meta(&fs);
    assert_eq!((rep.state, rep.total, rep.done, rep.failed, rep.unparsed), ("complete", 4, 4, 0, 0));
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE meta_state=1"), 4);
    let c = r.db.read().unwrap();
    let row = |rel: &str| {
        c.query_row(
            "SELECT make, model, lens, iso, focal, focal35, aperture, exposure, exposure_bias, taken_at, subsec_ms, tz_offset, orientation,
                    width, height, raw_width, preview_offset, preview_len, gps_lat, gps_lon, gps_alt, body_serial, container, is_raw, software
             FROM photos WHERE rel_path=?1",
            [rel],
            |r| {
                Ok((
                    r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, i64>(3)?, r.get::<_, f64>(4)?, r.get::<_, f64>(5)?,
                    r.get::<_, f64>(6)?, r.get::<_, f64>(7)?, r.get::<_, f64>(8)?, r.get::<_, i64>(9)?, r.get::<_, i64>(10)?, r.get::<_, i64>(11)?,
                    r.get::<_, i64>(12)?, r.get::<_, i64>(13)?, r.get::<_, i64>(14)?, r.get::<_, Option<i64>>(15)?, r.get::<_, Option<i64>>(16)?,
                    r.get::<_, Option<i64>>(17)?, r.get::<_, f64>(18)?, r.get::<_, f64>(19)?, r.get::<_, f64>(20)?, r.get::<_, String>(21)?,
                    r.get::<_, String>(22)?, r.get::<_, i64>(23)?, r.get::<_, Option<String>>(24)?,
                ))
            },
        )
        .unwrap()
    };
    for rel in ["a.ARW", "b.jpg", "c.heic", "d.cr3"] {
        let t = row(rel);
        assert_eq!((t.0.as_str(), t.1.as_str(), t.2.as_str(), t.3), ("SONY", "ILCE-7RM3", "FE 85mm F1.8", 800), "{rel}");
        assert_eq!((t.4, t.5, t.6, t.7), (85.0, 85.0, 1.8, 0.004), "{rel}");
        assert_eq!((t.9, t.10, t.11, t.12), (1_759_673_319, 120, -28_800, 6), "{rel}");
        assert!((t.18 - 37.775).abs() < 1e-3 && t.20 == 123.4, "{rel}");
        assert_eq!(t.21, "1234567");
    }
    let a = row("a.ARW");
    assert_eq!((a.13, a.14, a.15, a.16, a.17), (7952, 5304, Some(8000), Some(135_330), Some(396_841)));
    assert_eq!((a.22.as_str(), a.23), ("raw-tiff", 1));
    assert_eq!(row("b.jpg").13, 4000);
    assert_eq!(row("d.cr3").22, "cr3");
    // every row has a content key
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE content_key IS NULL"), 0);
}

#[test]
fn big_files_cost_one_64k_read_and_small_ones_less() {
    let r = rig();
    let fs = fs_with(&[("big.ARW", arw_like(true, &Spec::default(), 135_330, 396_841, 600_000)), ("small.jpg", jpeg_n(1))]);
    r.scan(&fs);
    let rep = r.meta(&fs);
    assert_eq!(rep.reads, 2);
    assert_eq!(rep.escalations, 0);
    let log = fs.read_log.lock().unwrap().clone();
    assert!(log.iter().all(|(_, off, len)| *off == 0 && *len == 65_536), "{log:?}");
    assert!(fs.bytes_read.load(Ordering::SeqCst) < 65_536 + 3000);
}

#[test]
fn escalates_in_steps_when_the_parser_asks() {
    let r = rig();
    let heic = make_heic(&Spec::default(), None, 100_000);
    let fs = fs_with(&[("far.heic", heic.clone())]);
    r.scan(&fs);
    let rep = r.meta(&fs);
    assert_eq!((rep.done, rep.escalations), (1, 1));
    let log = fs.read_log.lock().unwrap().clone();
    // the first window, then only the part from 64 KB to 256 KB
    assert_eq!(log[0], ("far.heic".to_string(), 0, 65_536));
    assert_eq!(log[1].1, 65_536);
    assert_eq!(log[1].2, 262_144 - 65_536);
    assert_eq!(r.q::<i64>("SELECT iso FROM photos"), 800);
    assert_eq!(r.q::<i64>("SELECT head_bytes FROM photos") as usize, heic.len().min(262_144));
}

#[test]
fn asks_past_the_ladder_keep_what_was_found_and_say_so() {
    let r = rig();
    let fs = fs_with(&[("huge.heic", make_heic(&Spec::default(), None, 2_000_000))]);
    r.scan(&fs);
    let mut cfg = fast();
    cfg.max_read = 262_144;
    let rep = r.meta_with(&fs, &cfg, &ScanControl::default(), |_| {});
    assert_eq!(rep.done, 1);
    let extra: Option<String> = r.q("SELECT extra_json FROM photos");
    assert_eq!(extra.as_deref(), Some(r#"{"truncated":true}"#));
    // the ask (past 2 MB) is beyond the ladder: it is not chased with reads that cannot help
    assert_eq!(rep.reads, 1);
}

#[test]
fn unchanged_photos_are_never_opened_again() {
    let r = rig();
    let files: Vec<(String, Vec<u8>)> = (0..12).map(|i| (format!("d{}/p{i}.jpg", i % 3), jpeg_n(i))).collect();
    let fs = FakeFs::new();
    for (rel, b) in &files {
        fs.add_blob(rel, b.clone(), BASE_MTIME);
    }
    r.scan(&fs);
    let first = r.meta(&fs);
    assert_eq!((first.total, first.done), (12, 12));
    let reads = fs.reads.load(Ordering::SeqCst);
    // a rescan changes nothing, and neither does the metadata run
    r.scan(&fs);
    let again = r.meta(&fs);
    assert_eq!((again.total, again.skipped, again.reads), (0, 12, 0));
    assert_eq!(fs.reads.load(Ordering::SeqCst), reads);
    // one file changes (new bytes, new mtime): only it is read
    fs.add_blob("d1/p1.jpg", jpeg_n(99), BASE_MTIME + 5_000_000_000);
    r.scan(&fs);
    let third = r.meta(&fs);
    assert_eq!((third.total, third.done, third.skipped), (1, 1, 11));
    assert_eq!(fs.reads.load(Ordering::SeqCst), reads + 1);
    assert_eq!(r.q::<String>("SELECT body_serial FROM photos WHERE rel_path='d1/p1.jpg'"), "SN99");
}

#[test]
fn read_errors_are_recorded_per_photo_and_retried_by_the_next_run() {
    let r = rig();
    let fs = fs_with(&[("ok.jpg", jpeg_n(1)), ("denied.jpg", jpeg_n(2)), ("slow.jpg", jpeg_n(3)), ("gone.jpg", jpeg_n(4))]);
    r.scan(&fs);
    fs.fail_reads_of("denied.jpg", FsError::Denied);
    fs.fail_reads_of("slow.jpg", FsError::Timeout);
    fs.fail_reads_of("gone.jpg", FsError::NotFound);
    let rep = r.meta(&fs);
    assert_eq!((rep.state, rep.done, rep.failed), ("complete", 1, 3));
    assert_eq!(r.state_of("ok.jpg"), 1);
    for rel in ["denied.jpg", "slow.jpg", "gone.jpg"] {
        assert_eq!(r.state_of(rel), 2, "{rel}");
    }
    let why: String = r.q("SELECT meta_error FROM photos WHERE rel_path='denied.jpg'");
    assert_eq!(why, "permission denied");
    let why: String = r.q("SELECT meta_error FROM photos WHERE rel_path='slow.jpg'");
    assert!(why.contains("timed out"), "{why}");
    // a timeout is tried twice, a denial once
    let reads_of = |rel: &str| fs.read_log.lock().unwrap().iter().filter(|(p, _, _)| p == rel).count();
    assert_eq!((reads_of("denied.jpg"), reads_of("slow.jpg")), (1, 2));
    // nothing went missing, no source went offline
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE status='present'"), 4);
    assert_eq!(r.q::<String>("SELECT state FROM sources"), "online");
    // healed: the next run reads only the failed ones
    fs.heal_reads_of("denied.jpg");
    fs.heal_reads_of("slow.jpg");
    let before = fs.reads.load(Ordering::SeqCst);
    let again = r.meta(&fs);
    assert_eq!((again.total, again.done, again.failed), (3, 2, 1));
    assert_eq!(fs.reads.load(Ordering::SeqCst), before + 3);
    assert_eq!(r.state_of("denied.jpg"), 1);
    assert_eq!(r.q::<Option<String>>("SELECT meta_error FROM photos WHERE rel_path='denied.jpg'"), None);
    // without --retry-failed the failed one is left alone
    let mut cfg = fast();
    cfg.retry_failed = false;
    let none = r.meta_with(&fs, &cfg, &ScanControl::default(), |_| {});
    assert_eq!(none.total, 0);
}

#[test]
fn files_the_extractor_cannot_read_are_unparsed_not_failed() {
    let r = rig();
    let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    png.resize(3000, 7);
    let fs = fs_with(&[("a.png", png), ("b.jpg", jpeg_n(1)), ("empty.jpg", vec![]), ("junk.jpg", vec![0xFF, 0xD8, 0xFF, 0xD9, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0])]);
    r.scan(&fs);
    let rep = r.meta(&fs);
    assert_eq!((rep.done, rep.failed, rep.unparsed), (1, 0, 3));
    assert_eq!(r.state_of("a.png"), 3);
    assert_eq!(r.q::<String>("SELECT meta_error FROM photos WHERE rel_path='a.png'"), "unsupported format");
    // a content key is kept, so even these relink
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE content_key IS NULL AND rel_path<>'empty.jpg'"), 0);
    let reads = fs.reads.load(Ordering::SeqCst);
    let again = r.meta(&fs);
    assert_eq!(again.total, 0);
    assert_eq!(fs.reads.load(Ordering::SeqCst), reads, "unparsed files are not retried until they change");
    let cov = coverage(&r.db.read().unwrap(), None).unwrap();
    assert_eq!((cov.done, cov.pending, cov.failed, cov.unparsed), (1, 0, 0, 3));
}

#[test]
fn a_mount_that_stops_answering_ends_the_job_offline_without_writing_failures() {
    let r = rig();
    let files: Vec<(String, Vec<u8>)> = (0..60).map(|i| (format!("p{i:02}.jpg"), jpeg_n(i))).collect();
    let fs = FakeFs::new();
    for (rel, b) in &files {
        fs.add_blob(rel, b.clone(), BASE_MTIME);
    }
    r.scan(&fs);
    // after 20 reads the mount goes away
    fs.at_read(20, |f| f.go_offline());
    let rep = r.meta(&fs);
    assert_eq!(rep.state, "offline");
    let done = r.q::<i64>("SELECT COUNT(*) FROM photos WHERE meta_state=1");
    assert!((10..60).contains(&done), "{done}");
    // no read error from the vanishing mount was written, nothing is missing, the source is not marked offline
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE meta_state=2"), 0);
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE status='present'"), 60);
    assert_eq!(r.q::<String>("SELECT state FROM sources"), "online");
    assert_eq!(r.q::<String>("SELECT state FROM jobs WHERE kind='metadata'"), "interrupted");
    // a run against the dead mount does nothing at all
    let dead = r.meta(&fs);
    assert_eq!((dead.state, dead.reads), ("offline", 0));
    // back online: the interrupted job continues and finishes
    fs.go_online();
    let rep = r.meta(&fs);
    assert!(rep.resumed);
    assert_eq!(rep.state, "complete");
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE meta_state=1"), 60);
    assert_eq!(rep.done, 60);
}

#[test]
fn every_read_failing_while_the_mount_answers_is_a_per_photo_failure_not_an_outage() {
    let r = rig();
    let fs = FakeFs::new();
    for i in 0..40 {
        fs.add_blob(&format!("p{i}.jpg"), jpeg_n(i), BASE_MTIME);
    }
    r.scan(&fs);
    *fs.reads_fail_all.lock().unwrap() = Some(FsError::Denied);
    let rep = r.meta(&fs);
    assert_eq!((rep.state, rep.failed), ("complete", 40));
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE status='present'"), 40);
}

#[test]
fn sidecars_set_rating_labels_keywords_and_edited() {
    let r = rig();
    let plain = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="r"><rdf:Description xmlns:xmp="a" xmlns:dc="b" xmp:Rating="-1"><dc:subject><rdf:Bag><rdf:li>reject me</rdf:li></rdf:Bag></dc:subject></rdf:Description></rdf:RDF></x:xmpmeta>"#;
    let fs = fs_with(&[
        ("a.ARW", arw_like(true, &Spec::default(), 5000, 3000, 20_000)),
        ("a.ARW.xmp", DARKTABLE_XMP.as_bytes().to_vec()),
        ("b.jpg", jpeg_n(5)),
        ("b.xmp", plain.as_bytes().to_vec()),
        ("c.jpg", jpeg_n(6)),
    ]);
    r.scan(&fs);
    assert_eq!(r.q::<i64>("SELECT xmp_kind FROM photos WHERE rel_path='a.ARW'"), 1);
    assert_eq!(r.q::<i64>("SELECT xmp_kind FROM photos WHERE rel_path='b.jpg'"), 2);
    let rep = r.meta(&fs);
    assert_eq!(rep.done, 3);
    let c = r.db.read().unwrap();
    let get = |rel: &str| {
        c.query_row("SELECT rating, label, rejected, edited, xmp_seen_size FROM photos WHERE rel_path=?1", [rel], |r| {
            Ok((r.get::<_, Option<i64>>(0)?, r.get::<_, Option<i64>>(1)?, r.get::<_, Option<i64>>(2)?, r.get::<_, Option<i64>>(3)?, r.get::<_, Option<i64>>(4)?))
        })
        .unwrap()
    };
    let a = get("a.ARW");
    assert_eq!((a.0, a.1, a.2, a.3), (Some(4), Some(5), Some(0), Some(1)));
    assert_eq!(a.4, Some(DARKTABLE_XMP.len() as i64));
    let b = get("b.jpg");
    assert_eq!((b.0, b.2), (Some(0), Some(1)));
    assert_eq!(get("c.jpg"), (None, None, None, None, None));
    let kws: Vec<String> = {
        let mut st = c.prepare("SELECT keyword FROM keywords WHERE src='sidecar' ORDER BY keyword").unwrap();
        st.query_map([], |r| r.get(0)).unwrap().flatten().collect()
    };
    assert_eq!(kws, vec!["Alice & Bob", "portrait", "reject me"]);
}

#[test]
fn a_changed_sidecar_is_re_read_alone() {
    let r = rig();
    let fs = fs_with(&[("a.jpg", jpeg_n(1)), ("a.jpg.xmp", DARKTABLE_XMP.as_bytes().to_vec()), ("b.jpg", jpeg_n(2))]);
    r.scan(&fs);
    r.meta(&fs);
    let rating = |rel: &str| r.q::<Option<i64>>(&format!("SELECT rating FROM photos WHERE rel_path='{rel}'"));
    assert_eq!(rating("a.jpg"), Some(4));
    fs.read_log.lock().unwrap().clear();
    // the user rates it 2 in darktable: only the sidecar changes
    let edited = DARKTABLE_XMP.replace("xmp:Rating=\"4\"", "xmp:Rating=\"2\"");
    fs.add_blob("a.jpg.xmp", edited.into_bytes(), BASE_MTIME + 9_000_000_000);
    r.scan(&fs);
    let rep = r.meta(&fs);
    assert_eq!((rep.total, rep.done, rep.sidecar_only), (1, 0, 1));
    assert_eq!(rating("a.jpg"), Some(2));
    let log = fs.read_log.lock().unwrap().clone();
    assert!(log.iter().all(|(p, _, _)| p == "a.jpg.xmp"), "the photo itself was not opened: {log:?}");
    // nothing left to do
    assert_eq!(r.meta(&fs).total, 0);
    // the sidecar is deleted: the photo is read again, the sidecar's data no longer applies
    fs.remove_file("a.jpg.xmp");
    r.scan(&fs);
    assert_eq!(r.meta(&fs).total, 1);
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE xmp_seen_size IS NOT NULL"), 0);
}

#[test]
fn oversized_and_unreadable_sidecars_never_fail_the_photo() {
    let r = rig();
    let big = vec![b'x'; crate::catalog_meta_xmp::MAX_SIDECAR as usize + 10];
    let fs = fs_with(&[("a.jpg", jpeg_n(1)), ("a.jpg.xmp", big), ("b.jpg", jpeg_n(2)), ("b.jpg.xmp", b"<x/>".to_vec())]);
    r.scan(&fs);
    fs.fail_reads_of("b.jpg.xmp", FsError::Timeout);
    fs.fail_reads_of("b.jpg.XMP", FsError::Timeout);
    let rep = r.meta(&fs);
    assert_eq!((rep.done, rep.failed), (2, 0));
    assert_eq!(r.q::<Option<String>>("SELECT extra_json FROM photos WHERE rel_path='a.jpg'").as_deref(), Some(r#"{"xmp_too_big":true}"#));
    // the unreadable sidecar is retried next run, on its own
    fs.heal_reads_of("b.jpg.xmp");
    fs.heal_reads_of("b.jpg.XMP");
    let again = r.meta(&fs);
    assert_eq!((again.total, again.sidecar_only), (1, 1));
}

#[test]
fn concurrency_is_capped_per_mount() {
    let r = rig();
    let fs = FakeFs::new();
    for i in 0..40 {
        fs.add_blob(&format!("p{i}.jpg"), jpeg_n(i), BASE_MTIME);
    }
    r.scan(&fs);
    fs.set_read_latency(Duration::from_millis(5));
    let mut cfg = fast();
    cfg.workers = 2;
    let rep = r.meta_with(&fs, &cfg, &ScanControl::default(), |_| {});
    assert_eq!(rep.done, 40);
    assert_eq!(fs.max_reads_running.load(Ordering::SeqCst), 2);
    assert_eq!(MetaConfig::for_network(true).workers, 2);
    assert_eq!(MetaConfig::for_network(true).read_deadline, Duration::from_secs(5));
    assert_eq!(MetaConfig::for_network(true).first_read, 65_536);
}

#[test]
fn adaptive_backs_off_when_latency_rises_and_recovers_when_it_calms() {
    let t0 = Instant::now();
    let mut a = Adaptive::new(4);
    // a calm volume: 20 ms reads
    for _ in 0..40 {
        a.record(20.0);
    }
    assert_eq!(a.tick(t0), 4);
    // latency climbs past the floor and 6x the baseline
    for _ in 0..40 {
        a.record(400.0);
    }
    assert_eq!(a.tick(t0 + Duration::from_secs(1)), 3);
    // a hold-down: no second step within 5 s
    for _ in 0..40 {
        a.record(400.0);
    }
    assert_eq!(a.tick(t0 + Duration::from_secs(3)), 3);
    for _ in 0..40 {
        a.record(400.0);
    }
    assert_eq!(a.tick(t0 + Duration::from_secs(7)), 2);
    // never below one reader
    for step in 0..10u64 {
        for _ in 0..40 {
            a.record(900.0);
        }
        a.tick(t0 + Duration::from_secs(20 + step * 6));
    }
    assert_eq!(a.allowed(), 1);
    // calm again: one more reader per 10 s of calm, up to the limit
    let mut now = t0 + Duration::from_secs(100);
    for _ in 0..80 {
        for _ in 0..40 {
            a.record(20.0);
        }
        a.tick(now);
        now += Duration::from_secs(6);
    }
    assert_eq!(a.allowed(), 4);
    // a fast local disk never trips it: sub-floor jitter is ignored
    let mut b = Adaptive::new(4);
    for i in 0..200 {
        b.record(if i % 10 == 0 { 30.0 } else { 0.05 });
    }
    assert_eq!(b.tick(Instant::now()), 4);
}

#[test]
fn adaptive_limit_is_applied_to_the_readers() {
    // a volume that is slow from the start: the job still finishes, with the limit it settled on
    let r = rig();
    let fs = FakeFs::new();
    for i in 0..30 {
        fs.add_blob(&format!("p{i}.jpg"), jpeg_n(i), BASE_MTIME);
    }
    r.scan(&fs);
    fs.set_read_latency(Duration::from_millis(2));
    let mut cfg = fast();
    cfg.workers = 3;
    cfg.adaptive = true;
    let rep = r.meta_with(&fs, &cfg, &ScanControl::default(), |_| {});
    assert_eq!(rep.done, 30);
    assert!(fs.max_reads_running.load(Ordering::SeqCst) <= 3);
}

#[test]
fn pause_resume_cancel_and_interrupt() {
    let r = rig();
    let fs = FakeFs::new();
    for i in 0..60 {
        fs.add_blob(&format!("p{i:02}.jpg"), jpeg_n(i), BASE_MTIME);
    }
    r.scan(&fs);
    fs.set_read_latency(Duration::from_millis(3));
    // pause stops the reading, resume continues it
    let ctl = ScanControl::default();
    let seen_paused = std::sync::atomic::AtomicBool::new(false);
    let mut paused_at = None;
    let rep = r.meta_with(&fs, &fast(), &ctl, |p| {
        if p.done >= 10 && paused_at.is_none() {
            ctl.pause();
            paused_at = Some(p.done);
        }
        if p.paused {
            seen_paused.store(true, Ordering::SeqCst);
            // resume from the progress callback after a moment
            std::thread::sleep(Duration::from_millis(40));
            ctl.resume();
        }
    });
    assert!(seen_paused.load(Ordering::SeqCst));
    assert_eq!((rep.state, rep.done), ("complete", 60));

    // cancel: final, the rest stays pending
    let r = rig();
    let fs = FakeFs::new();
    for i in 0..60 {
        fs.add_blob(&format!("p{i:02}.jpg"), jpeg_n(i), BASE_MTIME);
    }
    r.scan(&fs);
    fs.set_read_latency(Duration::from_millis(3));
    let ctl = ScanControl::default();
    let rep = r.meta_with(&fs, &fast(), &ctl, |p| {
        if p.done >= 10 {
            ctl.cancel();
        }
    });
    assert_eq!(rep.state, "cancelled");
    assert!(rep.done >= 10 && rep.done < 60);
    assert_eq!(r.q::<String>("SELECT state FROM jobs WHERE kind='metadata'"), "cancelled");
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE meta_state=0"), 60 - rep.done as i64);
    // a later run starts a new job (the cancelled one does not resume) and finishes the rest
    let again = r.meta(&fs);
    assert!(!again.resumed);
    assert_eq!(again.state, "complete");
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE meta_state=1"), 60);

    // interrupt (Ctrl-C): checkpoint kept, the next run resumes the same job
    let r = rig();
    let fs = FakeFs::new();
    for i in 0..60 {
        fs.add_blob(&format!("p{i:02}.jpg"), jpeg_n(i), BASE_MTIME);
    }
    r.scan(&fs);
    fs.set_read_latency(Duration::from_millis(3));
    let ctl = ScanControl::default();
    let first = r.meta_with(&fs, &fast(), &ctl, |p| {
        if p.done >= 15 {
            ctl.interrupt();
        }
    });
    assert_eq!(first.state, "interrupted");
    let job = first.job_id;
    let cp: String = r.q("SELECT checkpoint FROM jobs WHERE kind='metadata'");
    assert!(cp.contains("\"done\""));
    let second = r.meta(&fs);
    assert!(second.resumed);
    assert_eq!(second.job_id, job);
    assert_eq!(second.state, "complete");
    assert_eq!(second.done, 60, "the resumed job counts what the first one did");
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE meta_state=1"), 60);
    assert_eq!(r.q::<String>("SELECT state FROM jobs WHERE kind='metadata'"), "done");
}

#[test]
fn a_photo_rescanned_while_being_read_is_not_marked_done() {
    let r = rig();
    let fs = fs_with(&[("a.jpg", jpeg_n(1))]);
    r.scan(&fs);
    let w = |size: u64, mtime: i64| Work { id: 1, rel: "a.jpg".into(), ext: "jpg".into(), size, mtime_ns: mtime, xmp_kind: 0, xmp_size: None, xmp_mtime_ns: None, full: true };
    let (size, mtime): (i64, i64) = (r.q("SELECT size FROM photos"), r.q("SELECT mtime_ns FROM photos"));
    // the read was of an older version of the file (the scan has since changed the row): nothing is written
    let stale = Outcome::Full {
        w: w(size as u64 + 1, mtime),
        row: Box::new(Row { make: Some("OLD".into()), ..Default::default() }),
        keywords: vec!["x".into()],
        sidecar: SidecarWrite::default(),
        unparsed: None,
    };
    let fail = Outcome::Failed { w: w(size as u64, mtime + 1), reason: "x".into(), read_error: false };
    let written = r.db.exec(move |c| write_batch(c, vec![stale, fail])).unwrap();
    assert_eq!((written.done, written.failed, written.raced), (0, 0, 2));
    assert_eq!(r.state_of("a.jpg"), 0);
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM keywords"), 0);
}

#[test]
fn relink_follows_a_rename_and_keeps_the_photo_id_and_its_data() {
    let r = rig();
    let b1 = jpeg_n(1);
    let fs = fs_with(&[("d/a.jpg", b1.clone()), ("d/other.jpg", jpeg_n(2))]);
    r.scan(&fs);
    assert_eq!(r.meta(&fs).done, 2);
    let id: i64 = r.q("SELECT id FROM photos WHERE rel_path='d/a.jpg'");
    // user data keyed by the photo id, from other features
    r.db.exec(move |c| c.execute("INSERT INTO keywords(photo_id,keyword,src) VALUES(?1,'tag','user')", [id]).map(|_| ()).map_err(|e| e.to_string())).unwrap();
    fs.rename_file("d/a.jpg", "d/renamed.jpg");
    r.scan(&fs);
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE status='missing'"), 1);
    let rep = r.meta(&fs);
    assert_eq!(rep.relink.relinked, 1);
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos"), 2, "no duplicate row");
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE status='missing'"), 0);
    let (nid, path, name, state): (i64, String, String, i64) = r
        .db
        .read()
        .unwrap()
        .query_row("SELECT id, rel_path, name, meta_state FROM photos WHERE rel_path='d/renamed.jpg'", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))
        .unwrap();
    assert_eq!((nid, path.as_str(), name.as_str(), state), (id, "d/renamed.jpg", "renamed.jpg", 1));
    assert_eq!(r.q::<i64>(&format!("SELECT COUNT(*) FROM keywords WHERE photo_id={id} AND keyword='tag'")), 1, "the tag survived");
    assert_eq!(r.q::<String>("SELECT body_serial FROM photos WHERE id=1"), "SN1");
    // stable: another scan and run change nothing
    r.scan(&fs);
    assert_eq!(r.meta(&fs).relink.relinked, 0);
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos"), 2);
}

#[test]
fn relink_follows_a_move_to_another_folder() {
    let r = rig();
    let fs = fs_with(&[("2023/a.jpg", jpeg_n(1)), ("2023/b.jpg", jpeg_n(2)), ("2024/c.jpg", jpeg_n(3))]);
    r.scan(&fs);
    r.meta(&fs);
    let id: i64 = r.q("SELECT id FROM photos WHERE rel_path='2023/a.jpg'");
    fs.rename_file("2023/a.jpg", "2024/sorted/a.jpg");
    r.scan(&fs);
    let rep = r.meta(&fs);
    assert_eq!(rep.relink.relinked, 1);
    assert_eq!(r.q::<i64>("SELECT id FROM photos WHERE rel_path='2024/sorted/a.jpg'"), id);
    let dir: String = r.q(&format!("SELECT d.rel_path FROM photos p JOIN dirs d ON d.id=p.dir_id WHERE p.id={id}"));
    assert_eq!(dir, "2024/sorted");
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE status='present'"), 3);
}

#[test]
fn a_copy_is_not_a_move() {
    let r = rig();
    let b = jpeg_n(1);
    let fs = fs_with(&[("a.jpg", b.clone()), ("keep.jpg", jpeg_n(2))]);
    r.scan(&fs);
    r.meta(&fs);
    // a copy: same bytes and mtime, the original stays
    fs.add_blob("copy.jpg", b, BASE_MTIME);
    r.scan(&fs);
    let rep = r.meta(&fs);
    assert_eq!(rep.relink.relinked, 0);
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE status='present'"), 3);
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE status='missing'"), 0);
    // the original is then deleted: now the copy is the survivor and the original's row is missing; the
    // pair is relinkable (one missing, one present), which is what "move by copy and delete" is
    fs.remove_file("a.jpg");
    r.scan(&fs);
    let rep = r.meta(&fs);
    assert_eq!(rep.relink.relinked, 1);
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos"), 2);
}

#[test]
fn ambiguous_keys_do_nothing() {
    let r = rig();
    // two identical files (same bytes and mtime) in two places
    let b = jpeg_n(1);
    let fs = fs_with(&[("x/a.jpg", b.clone()), ("y/a.jpg", b.clone()), ("z/keep.jpg", jpeg_n(9))]);
    r.scan(&fs);
    r.meta(&fs);
    assert_eq!(r.q::<i64>("SELECT COUNT(DISTINCT content_key) FROM photos"), 2);
    // both vanish and one new identical file appears: two candidates for one key
    fs.remove_file("x/a.jpg");
    fs.remove_file("y/a.jpg");
    fs.add_blob("w/new.jpg", b.clone(), BASE_MTIME);
    r.scan(&fs);
    let rep = r.meta(&fs);
    assert_eq!(rep.relink.relinked, 0);
    assert_eq!(rep.relink.ambiguous, 1);
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE status='missing'"), 2);
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE status='present'"), 2);
    // and the other way: one missing, two new identical files
    let r = rig();
    let fs = fs_with(&[("a.jpg", b.clone()), ("keep.jpg", jpeg_n(9))]);
    r.scan(&fs);
    r.meta(&fs);
    fs.remove_file("a.jpg");
    fs.add_blob("n1.jpg", b.clone(), BASE_MTIME);
    fs.add_blob("n2.jpg", b, BASE_MTIME);
    r.scan(&fs);
    let rep = r.meta(&fs);
    assert_eq!((rep.relink.relinked, rep.relink.ambiguous), (0, 1));
}

#[test]
fn different_mtime_or_content_is_not_the_same_photo() {
    let r = rig();
    let fs = fs_with(&[("a.jpg", jpeg_n(1)), ("b.jpg", jpeg_n(2))]);
    r.scan(&fs);
    r.meta(&fs);
    fs.remove_file("a.jpg");
    // same bytes, different mtime: a different key, so no relink
    fs.add_blob("a2.jpg", jpeg_n(1), BASE_MTIME + 1_000_000_000);
    fs.remove_file("b.jpg");
    // different bytes, same mtime
    fs.add_blob("b2.jpg", jpeg_n(3), BASE_MTIME);
    r.scan(&fs);
    assert_eq!(r.meta(&fs).relink.relinked, 0);
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE status='missing'"), 2);
}

#[test]
fn relink_does_nothing_without_a_complete_scan_that_marked_the_old_row_missing() {
    let r = rig();
    let fs = fs_with(&[("d/a.jpg", jpeg_n(1)), ("d/b.jpg", jpeg_n(2))]);
    r.scan(&fs);
    r.meta(&fs);
    fs.rename_file("d/a.jpg", "e/a.jpg");
    // the old directory did not answer: the old row is not marked missing, so the new one stays new
    fs.stall("d");
    let o = ScanOptions { root: fs.root.clone(), include_subfolders: true, dir_skip: false, light: false, fresh: false };
    let rep = run_scan(&r.db, &fs, &r.source(), &o, &scan_cfg(), &ScanControl::default(), &mut |_| {}).unwrap();
    assert_eq!(rep.state, "incomplete");
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE status='missing'"), 0);
    let m = r.meta(&fs);
    assert_eq!(m.relink.relinked, 0);
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE status='present'"), 3);
    // the directory answers again and a complete scan marks the old row missing; the next run relinks
    fs.unstall("d");
    r.scan(&fs);
    assert_eq!(r.meta(&fs).relink.relinked, 1);
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos"), 2);
}

#[test]
fn schema_v2_migrates_a_v1_catalog_without_losing_anything() {
    use rusqlite::Connection;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalog.sqlite");
    {
        let mut c = Connection::open(&path).unwrap();
        catalog_db::migrate(&mut c, &catalog_db::LADDER[..1], None).unwrap();
        c.execute("INSERT INTO sources(name,volume_key,created_at) VALUES('S','v',1)", []).unwrap();
        c.execute("INSERT INTO dirs(source_id,rel_path) VALUES(1,'')", []).unwrap();
        for (i, state) in [0, 1, 2].iter().enumerate() {
            c.execute(
                "INSERT INTO photos(source_id,dir_id,name,rel_path,ext,size,mtime_ns,discovered_at,seen_at,meta_state,make,iso)
                 VALUES(1,1,?1,?1,'jpg',10,20,1,1,?2,'CANON',?3)",
                rusqlite::params![format!("p{i}.jpg"), state, 100 * (i as i64 + 1)],
            )
            .unwrap();
        }
        assert_eq!(c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
    }
    let db = CatalogDb::open(&path).unwrap();
    let c = db.read().unwrap();
    assert_eq!(c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0)).unwrap(), catalog_db::SCHEMA_VERSION);
    assert_eq!(catalog_db::SCHEMA_VERSION, 3);
    // the data and states survive; the new columns are NULL
    let rows: Vec<(String, i64, String, i64, Option<i64>)> = c
        .prepare("SELECT name, meta_state, make, iso, raw_width FROM photos ORDER BY id")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(rows, vec![("p0.jpg".into(), 0, "CANON".into(), 100, None), ("p1.jpg".into(), 1, "CANON".into(), 200, None), ("p2.jpg".into(), 2, "CANON".into(), 300, None)]);
    // the backup of the v1 file was taken, and the new table and indexes work
    assert!(dir.path().join("catalog.sqlite.pre-v1.bak").exists());
    db.exec(|c| c.execute("INSERT INTO keywords(photo_id,keyword) VALUES(1,'k')", []).map(|_| ()).map_err(|e| e.to_string())).unwrap();
    let idx: i64 = c.query_row("SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name IN ('photos_content_key','photos_meta_state','keywords_word')", [], |r| r.get(0)).unwrap();
    assert_eq!(idx, 3);
    // deleting a photo deletes its keywords
    db.exec(|c| c.execute("DELETE FROM photos WHERE id=1", []).map(|_| ()).map_err(|e| e.to_string())).unwrap();
    assert_eq!(c.query_row("SELECT COUNT(*) FROM keywords", [], |r| r.get::<_, i64>(0)).unwrap(), 0);
}

#[test]
fn the_content_key_depends_on_what_it_should() {
    let w = |size: u64, mtime: i64| Work { id: 1, rel: "a".into(), ext: "jpg".into(), size, mtime_ns: mtime, xmp_kind: 0, xmp_size: None, xmp_mtime_ns: None, full: true };
    let head = vec![5u8; 100_000];
    let base = content_key(&head, &w(10, 1), Some(5), Some("S"));
    assert_eq!(base.len(), 32);
    assert_eq!(base, content_key(&head, &w(10, 1), Some(5), Some("S")));
    // bytes past the first 64 KB do not matter (so the window size does not)
    let mut tail = head.clone();
    tail[80_000] = 9;
    assert_eq!(base, content_key(&tail, &w(10, 1), Some(5), Some("S")));
    assert_eq!(base, content_key(&head[..70_000], &w(10, 1), Some(5), Some("S")));
    let mut early = head.clone();
    early[10] = 9;
    for other in [
        content_key(&early, &w(10, 1), Some(5), Some("S")),
        content_key(&head, &w(11, 1), Some(5), Some("S")),
        content_key(&head, &w(10, 2), Some(5), Some("S")),
        content_key(&head, &w(10, 1), Some(6), Some("S")),
        content_key(&head, &w(10, 1), Some(5), Some("T")),
        content_key(&head, &w(10, 1), None, Some("S")),
    ] {
        assert_ne!(base, other);
    }
}

#[test]
fn sidecar_names_follow_the_scan_rules() {
    assert_eq!(sidecar_names("d/IMG.ARW", 1), vec!["d/IMG.ARW.xmp", "d/IMG.ARW.XMP"]);
    assert_eq!(sidecar_names("d/IMG.ARW", 2), vec!["d/IMG.xmp", "d/IMG.XMP"]);
    assert_eq!(sidecar_names("a.b/IMG", 2), vec!["a.b/IMG.xmp", "a.b/IMG.XMP"]);
}

#[test]
fn real_files_end_to_end_with_pread() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("a.jpg"), jpeg_n(1)).unwrap();
    std::fs::write(dir.path().join("sub/b.ARW"), arw_like(true, &Spec::default(), 5000, 3000, 80_000)).unwrap();
    std::fs::write(dir.path().join("sub/b.ARW.xmp"), DARKTABLE_XMP).unwrap();
    std::fs::write(dir.path().join("c.png"), vec![0x89, b'P', b'N', b'G', 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]).unwrap();
    let cat = tempfile::tempdir().unwrap();
    let db = CatalogDb::open(&cat.path().join("catalog.sqlite")).unwrap();
    db.exec(|c| c.execute("INSERT INTO sources(name,kind,volume_key,mount_path,rel_root,created_at) VALUES('S','local','dev:x','/x','',1)", []).map(|_| ()).map_err(|e| e.to_string())).unwrap();
    let src = get_source(&db.read().unwrap(), 1).unwrap().unwrap();
    let fs = RealFs::new(dir.path().to_path_buf());
    let o = ScanOptions { root: dir.path().to_path_buf(), include_subfolders: true, dir_skip: false, light: false, fresh: false };
    let s = run_scan(&db, &fs, &src, &o, &scan_cfg(), &ScanControl::default(), &mut |_| {}).unwrap();
    assert_eq!(s.state, "complete");
    let rep = run_metadata(&db, &fs, &src, &fast(), &ScanControl::default(), &mut |_| {}).unwrap();
    assert_eq!((rep.state, rep.done, rep.unparsed, rep.failed), ("complete", 2, 1, 0));
    let c = db.read().unwrap();
    let (make, rating): (String, i64) = c.query_row("SELECT make, rating FROM photos WHERE rel_path='sub/b.ARW'", [], |r| Ok((r.get(0)?, r.get(1)?))).unwrap();
    assert_eq!((make.as_str(), rating), ("SONY", 4));
    assert!(Path::new(&dir.path().join("a.jpg")).exists());
}

// ---- the benchmark -------------------------------------------------------------------------

/// `cargo test --bin immage bench_50k -- --ignored --nocapture`: a seeded 50,000-photo tree of small
/// JPEG and TIFF files (EXIF, GPS on a third of them, sidecars on a tenth) on the local disk, then
/// a scan, a metadata run, a no-op rerun, and a one-percent change. `IMMAGE_CATALOG_BENCH_DIR` picks the disk.
#[test]
#[ignore = "benchmark"]
fn bench_50k() {
    let base = std::env::var("IMMAGE_CATALOG_BENCH_DIR").ok().map(PathBuf::from);
    let tmp;
    let root = match &base {
        Some(b) => {
            let r = b.join(format!("meta-bench-{}", std::process::id()));
            std::fs::create_dir_all(&r).unwrap();
            r
        }
        None => {
            tmp = tempfile::tempdir().unwrap();
            tmp.path().to_path_buf()
        }
    };
    let t = Instant::now();
    let mut n = 0u32;
    let mut rng = 0x2545F4914F6CDD1Du64;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    for d in 0..250 {
        let dir = root.join(format!("shoot{d:03}"));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..200 {
            let r = next();
            let spec = Spec {
                datetime: leak(format!("20{:02}:{:02}:{:02} {:02}:{:02}:{:02}", 15 + r % 10, 1 + (r >> 8) % 12, 1 + (r >> 16) % 28, (r >> 24) % 24, (r >> 32) % 60, (r >> 40) % 60)),
                serial: Some(leak(format!("SN{}", r % 5000))),
                gps: r % 3 == 0,
                ..Spec::default()
            };
            n += 1;
            let (name, bytes) = if r % 4 == 0 {
                (format!("IMG_{i:04}.tif"), {
                    let mut b = exif_block(true, &spec);
                    b.resize(4096 + (r % 2000) as usize, 0);
                    b
                })
            } else {
                (format!("IMG_{i:04}.jpg"), make_jpeg(Some(&spec), None, 600, 400, (r % 3000) as usize))
            };
            std::fs::write(dir.join(&name), bytes).unwrap();
            if r % 10 == 0 {
                std::fs::write(dir.join(format!("{name}.xmp")), DARKTABLE_XMP).unwrap();
            }
        }
    }
    eprintln!("bench: generated {n} files in {:.1}s", t.elapsed().as_secs_f64());
    let cat = tempfile::tempdir().unwrap();
    let db = CatalogDb::open(&cat.path().join("catalog.sqlite")).unwrap();
    db.exec(|c| c.execute("INSERT INTO sources(name,kind,volume_key,mount_path,rel_root,created_at) VALUES('S','local','dev:x','/x','',1)", []).map(|_| ()).map_err(|e| e.to_string())).unwrap();
    let src = get_source(&db.read().unwrap(), 1).unwrap().unwrap();
    let fs = RealFs::new(root.clone());
    let o = ScanOptions { root: root.clone(), include_subfolders: true, dir_skip: false, light: false, fresh: false };
    let mut scfg = ScanConfig::for_network(false);
    scfg.progress_every = Duration::from_secs(3600);
    let t = Instant::now();
    let s = run_scan(&db, &fs, &src, &o, &scfg, &ScanControl::default(), &mut |_| {}).unwrap();
    eprintln!("bench: scan {} photos in {:.2}s ({:.0}/s)", s.totals.found, t.elapsed().as_secs_f64(), s.files_per_s);
    for workers in [1usize, 2, 4, 8] {
        db.exec(|c| c.execute("UPDATE photos SET meta_state=0", []).map(|_| ()).map_err(|e| e.to_string())).unwrap();
        let mut cfg = MetaConfig::for_network(false);
        cfg.workers = workers;
        cfg.progress_every = Duration::from_secs(3600);
        let t = Instant::now();
        let rep = run_metadata(&db, &fs, &src, &cfg, &ScanControl::default(), &mut |_| {}).unwrap();
        eprintln!(
            "bench: metadata workers={workers}: {} photos in {:.2}s = {:.0}/s, {:.1} MB read, p50 {:.2} ms p95 {:.2} ms, state {}",
            rep.done, t.elapsed().as_secs_f64(), rep.per_s, rep.bytes_read as f64 / 1e6, rep.p50_ms, rep.p95_ms, rep.state
        );
        assert_eq!(rep.done, n as u64);
    }
    // nothing changed: nothing is read
    let t = Instant::now();
    let rep = run_metadata(&db, &fs, &src, &MetaConfig::for_network(false), &ScanControl::default(), &mut |_| {}).unwrap();
    eprintln!("bench: unchanged rerun: total {} skipped {} in {:.3}s", rep.total, rep.skipped, t.elapsed().as_secs_f64());
    assert_eq!((rep.total, rep.reads), (0, 0));
    let c = db.read().unwrap();
    let with_gps: i64 = c.query_row("SELECT COUNT(*) FROM photos WHERE gps_lat IS NOT NULL", [], |r| r.get(0)).unwrap();
    let with_rating: i64 = c.query_row("SELECT COUNT(*) FROM photos WHERE rating IS NOT NULL", [], |r| r.get(0)).unwrap();
    eprintln!("bench: with GPS {with_gps}, with a sidecar rating {with_rating}, db {:.1} MB", std::fs::metadata(cat.path().join("catalog.sqlite")).unwrap().len() as f64 / 1e6);
    if let Some(b) = base {
        let _ = std::fs::remove_dir_all(b.join(format!("meta-bench-{}", std::process::id())));
    }
}
