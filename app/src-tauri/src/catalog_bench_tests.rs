// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

use super::*;
use crate::catalog_fakefs::FakeFs;
use crate::catalog_meta_fixtures::*;

fn cfg(dir: &Path) -> BenchConfig {
    let mut c = BenchConfig::new(true, dir.to_path_buf());
    c.window = Duration::from_millis(20);
    c.sample_every = Duration::from_millis(20);
    c.bucket = Duration::from_millis(100);
    c.duration = Duration::from_secs(30);
    c.backoff_pause = Duration::from_millis(10);
    c
}

fn fake() -> FakeFs {
    let fs = FakeFs::new();
    let spec = Spec::default();
    fs.add_blob("Trip/SECRETNAME_1.ARW", arw_like(true, &spec, 5000, 3000, 20_000), 1);
    fs.add_blob("Trip/SECRETNAME_2.jpg", make_jpeg(Some(&spec), None, 100, 80, 0), 1);
    fs.add_blob("Trip/SECRETNAME_3.jpg", make_jpeg(Some(&spec), None, 100, 80, 3), 1);
    fs.add_blob("Raw/SECRETNAME_4.cr3", make_cr3(&spec, 400).0, 1);
    fs.add_blob("Raw/SECRETNAME_5.heic", make_heic(&spec, None, 100_000), 1);
    fs.add_blob("Raw/SECRETNAME_6.NEF", vec![0x55; 4000], 1);
    fs.add_file("Raw/notes.txt", 10, 1);
    fs.add_file("Raw/SECRETNAME_1.xmp", 10, 1);
    fs.add_file("Raw/.dotname.jpg", 10, 1);
    fs
}

fn run(fs: &FakeFs, c: &BenchConfig) -> Value {
    run_bench(fs, &fs.root, json!({ "found": false }), c, Box::new(|| false)).unwrap()
}

#[test]
fn reports_formats_ladder_and_aggregates_without_names() {
    let tmp = tempfile::tempdir().unwrap();
    let fs = fake();
    let v = run(&fs, &cfg(tmp.path()));
    assert_eq!(v["discovery"]["photos"], 6);
    assert_eq!(v["discovery"]["xmp_sidecars"], 1);
    assert_eq!(v["discovery"]["walk_complete"], true);
    assert_eq!(v["discovery"]["dirs"], 3);
    assert_eq!(v["metadata"]["files"], 6);
    let by = &v["metadata"]["by_format"];
    for k in ["arw", "jpg", "cr3", "heic", "nef"] {
        assert!(by[k]["files"].as_u64().unwrap() >= 1, "{k}");
    }
    assert_eq!(by["jpg"]["files"], 2);
    // the heic keeps its metadata past 64 KB: one escalation
    assert!(by["heic"]["retried"].as_u64().unwrap() >= 1, "{by}");
    assert!(v["metadata"]["ladder_finished_at"]["256k"].as_u64().unwrap() >= 1);
    // garbage is a parse failure of its format
    let f = &by["nef"]["parse_failures"];
    assert_eq!(f["unsupported"].as_u64().unwrap() + f["corrupt"].as_u64().unwrap(), 1);
    assert_eq!(v["metadata"]["parse_failures_total"], 1);
    assert!(v["thumbnails"]["by_format"]["arw"]["reads"].as_u64().unwrap() >= 1);
    assert!(v["sqlite"]["batches"]["count"].as_u64().unwrap() >= 2);
    assert!(v["sqlite"]["rows"].as_u64().unwrap() >= 6);
    assert_eq!(v["errors"]["total"], 0);
    assert!(v["network"]["payload_bytes_read"].as_u64().unwrap() > 0);
    assert!(v["stat"]["file"]["count"].as_u64().unwrap() >= 5);
    assert!(!v["timeline"].as_array().unwrap().is_empty());
    assert!(v["process"]["peak_rss_mb"].as_f64().unwrap() > 0.0);
    // aggregated: no file or folder name anywhere, in either output
    let all = format!("{v}{}", render_report(&v));
    for name in ["SECRETNAME", "Trip", "Raw/", "notes", "dotname"] {
        assert!(!all.contains(name), "{name} leaked");
    }
    assert!(render_report(&v).contains("DISCOVERY"));
    // the scratch database is gone
    assert_eq!(std::fs::read_dir(tmp.path()).unwrap().count(), 0);
}

#[test]
fn max_files_caps_the_work() {
    let tmp = tempfile::tempdir().unwrap();
    let fs = fake();
    let mut c = cfg(tmp.path());
    c.max_files = 3;
    let v = run(&fs, &c);
    assert_eq!(v["metadata"]["files"], 3);
    assert_eq!(v["discovery"]["walk_complete"], false);
}

#[test]
fn slow_reads_back_off_and_the_run_still_finishes() {
    let tmp = tempfile::tempdir().unwrap();
    let fs = FakeFs::new();
    for i in 0..60 {
        fs.add_blob(&format!("a/{i}.jpg"), make_jpeg(Some(&Spec::default()), None, 100, 80, 0), 1);
    }
    fs.set_read_latency(Duration::from_millis(15));
    let mut c = cfg(tmp.path());
    c.concurrency = 4;
    c.latency_limit_ms = 5.0;
    let v = run(&fs, &c);
    assert_eq!(v["metadata"]["files"], 60);
    assert!(v["concurrency"]["backoffs"].as_u64().unwrap() >= 1, "{}", v["concurrency"]);
    assert!(v["concurrency"]["lowest"].as_u64().unwrap() < 4);
    assert_eq!(v["safety"]["ended"], "finished");
    assert!(fs.max_reads_running.load(Ordering::SeqCst) <= 4);
}

#[test]
fn errors_are_counted_by_kind_and_an_error_storm_stops_reading() {
    let tmp = tempfile::tempdir().unwrap();
    let fs = FakeFs::new();
    for i in 0..200 {
        fs.add_blob(&format!("a/{i}.jpg"), make_jpeg(Some(&Spec::default()), None, 100, 80, 0), 1);
    }
    *fs.reads_fail_all.lock().unwrap() = Some(FsError::Io("Input/output error (os error 5)".into()));
    let mut c = cfg(tmp.path());
    c.concurrency = 1;
    let v = run(&fs, &c);
    assert_eq!(v["safety"]["ended"], "error-storm");
    assert!(v["errors"]["by_kind"]["io (os error 5)"].as_u64().unwrap() >= 64);
    assert!(v["metadata"]["files"].as_u64().unwrap() < 200);
    assert_eq!(v["errors"]["timeouts"], 0);
}

#[test]
fn timeouts_and_duration_are_reported() {
    let tmp = tempfile::tempdir().unwrap();
    let fs = fake();
    fs.fail_reads_of("Trip/SECRETNAME_2.jpg", FsError::Timeout);
    let v = run(&fs, &cfg(tmp.path()));
    assert_eq!(v["errors"]["timeouts"], 1);
    assert_eq!(v["metadata"]["by_format"]["jpg"]["parse_failures"]["read_errors"], 1);
    let mut c = cfg(tmp.path());
    c.duration = Duration::from_millis(1);
    let v = run(&fs, &c);
    assert_eq!(v["discovery"]["walk_complete"], false);
}

#[test]
fn mount_facts_keep_only_safe_options() {
    let t = MountTable::parse(
        "36 35 0:30 / /mnt/nas rw,relatime shared:1 - cifs //nas.local/photos rw,vers=3.1.1,cache=strict,username=bob,addr=10.0.0.2,domain=H,rsize=4194304,password=x\n",
    );
    let m = mount_facts(&t, Path::new("/mnt/nas/2023"));
    assert_eq!(m["fs_type"], "cifs");
    assert_eq!(m["host"], "nas.local");
    assert_eq!(m["share"], "photos");
    assert_eq!(m["options"]["rsize"], "4194304");
    assert_eq!(m["options"]["cache"], "strict");
    let s = m.to_string();
    for bad in ["bob", "10.0.0.2", "password", "domain"] {
        assert!(!s.contains(bad), "{bad}");
    }
}

#[test]
fn cli_writes_json_and_text_and_refuses_a_report_inside_the_folder() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("photos");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join("a.jpg"), make_jpeg(Some(&Spec::default()), None, 100, 80, 0)).unwrap();
    let before: Vec<_> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().file_name()).collect();
    let out = dir.path().join("report");
    let go = |extra: &[&str]| {
        let mut a: Vec<String> = ["bench", root.to_str().unwrap(), "--duration", "20", "--scratch", dir.path().to_str().unwrap()].iter().map(|s| s.to_string()).collect();
        a.extend(extra.iter().map(|s| s.to_string()));
        let (mut o, mut e) = (Vec::new(), Vec::new());
        let code = crate::catalog_cli::run(a, &mut o, &mut e);
        (code, String::from_utf8(o).unwrap(), String::from_utf8(e).unwrap())
    };
    let (code, text, _) = go(&["--out", out.to_str().unwrap(), "--max-files", "10"]);
    assert_eq!(code, 0, "{text}");
    let j: Value = serde_json::from_str(&std::fs::read_to_string(out.with_extension("json")).unwrap()).unwrap();
    assert_eq!(j["metadata"]["files"], 1);
    assert!(std::fs::read_to_string(out.with_extension("txt")).unwrap().contains("DISCOVERY"));
    let inside = root.join("r");
    let (code, _, err) = go(&["--out", inside.to_str().unwrap()]);
    assert_eq!(code, 2, "{err}");
    let (code, _, _) = go(&["--max-files", "0"]);
    assert_eq!(code, 2);
    // the measured folder is untouched
    let after: Vec<_> = std::fs::read_dir(&root).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(before, after);
}
