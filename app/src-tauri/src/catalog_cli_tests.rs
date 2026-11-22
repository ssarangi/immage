// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

use super::*;

struct Cli {
    dir: tempfile::TempDir,
    db: PathBuf,
}

fn cli() -> Cli {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("catalog.sqlite");
    Cli { dir, db }
}

impl Cli {
    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let mut a: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        a.push("--db".into());
        a.push(self.db.to_string_lossy().into_owned());
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let code = run(a, &mut out, &mut err);
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    }
    fn tree(&self) -> PathBuf {
        let root = self.dir.path().join("photos");
        std::fs::create_dir_all(root.join("2023")).unwrap();
        for f in ["a.jpg", "2023/b.ARW", "2023/c.CR3", "2023/notes.txt"] {
            std::fs::write(root.join(f), b"x").unwrap();
        }
        root
    }
}

#[test]
fn add_list_scan_rescan_status_verify_remove() {
    let c = cli();
    let root = c.tree();
    let p = root.to_string_lossy().into_owned();
    // add
    let (code, out, err) = c.run(&["add-source", &p, "--name", "Holiday", "--skip-test"]);
    assert_eq!(code, EXIT_OK, "{out}{err}");
    assert!(out.contains("Folder   "), "{out}");
    assert!(out.contains("Storage  "), "{out}");
    assert!(out.contains("Added source 1 \"Holiday\". Policy: on launch."), "{out}");
    assert!(out.contains("Run: immage catalog scan 1"), "{out}");
    // list (human and json)
    let (code, out, _) = c.run(&["list-sources"]);
    assert_eq!(code, EXIT_OK);
    assert!(out.starts_with("ID  NAME"), "{out}");
    assert!(out.contains("Holiday") && out.contains("never-indexed"), "{out}");
    let (_, out, _) = c.run(&["list-sources", "--json"]);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(v[0]["name"], "Holiday");
    assert_eq!(v[0]["photos"], 0);
    assert_eq!(v[0]["availability"], "online");
    // first scan
    let (code, out, err) = c.run(&["scan", "1"]);
    assert_eq!(code, EXIT_OK, "{out}{err}");
    assert!(out.contains("Holiday: checked 3 photos in "), "{out}");
    assert!(out.contains("3 new, 0 changed, 0 missing."), "{out}");
    // json scan after a new file and a deleted one
    std::fs::write(root.join("2023/d.RAF"), b"yy").unwrap();
    std::fs::remove_file(root.join("a.jpg")).unwrap();
    let (code, out, _) = c.run(&["rescan", "Holiday", "--json"]);
    assert_eq!(code, EXIT_OK);
    let v: Value = serde_json::from_str(out.lines().next().unwrap()).unwrap();
    assert_eq!((v["state"].as_str(), v["found"].as_u64(), v["new"].as_u64(), v["missing"].as_u64()), (Some("complete"), Some(3), Some(1), Some(1)));
    assert!(v["files_per_s"]["list"].is_number());
    assert_eq!(v["incomplete_dirs"], json!([]));
    // status
    let (code, out, _) = c.run(&["status"]);
    assert_eq!(code, EXIT_OK, "{out}");
    assert!(out.starts_with("Catalog  "), "{out}");
    assert!(out.contains("schema 1") && out.contains("3 photos"), "{out}");
    let (_, out, _) = c.run(&["status", "--json"]);
    let v: Value = serde_json::from_str(&out).unwrap();
    assert_eq!((v["photos"].as_u64(), v["missing"].as_u64(), v["sources"]["online"].as_u64()), (Some(3), Some(1), Some(1)));
    // verify
    let (code, out, _) = c.run(&["verify"]);
    assert_eq!(code, EXIT_OK, "{out}");
    assert!(out.contains("ok       database integrity") && out.contains("Found no problems."), "{out}");
    let (code, out, _) = c.run(&["verify", "--deep"]);
    assert_eq!(code, EXIT_OK, "{out}");
    assert!(out.contains("files match the catalog (3 checked)"), "{out}");
    // remove asks, and refuses without a terminal
    let (code, _, err) = c.run(&["remove-source", "1"]);
    assert_eq!(code, EXIT_USAGE);
    assert!(err.contains("--yes"), "{err}");
    let (code, out, _) = c.run(&["remove-source", "Holiday", "--yes"]);
    assert_eq!(code, EXIT_OK, "{out}");
    assert!(out.contains("Removed source 1"), "{out}");
    let (_, out, _) = c.run(&["list-sources"]);
    assert!(out.contains("No sources yet"), "{out}");
}

#[test]
fn offline_overlap_and_usage_exit_codes() {
    let c = cli();
    let root = c.tree();
    let p = root.to_string_lossy().into_owned();
    // not reachable: 4, and nothing added
    let ghost = c.dir.path().join("not-there");
    let (code, _, err) = c.run(&["add-source", &ghost.to_string_lossy(), "--skip-test"]);
    assert_eq!(code, EXIT_OFFLINE, "{err}");
    assert!(err.contains("--allow-offline"), "{err}");
    let (_, out, _) = c.run(&["list-sources"]);
    assert!(out.contains("No sources yet"));
    // allowed: exit 0, and scanning it is exit 4 (every requested source unreachable)
    let (code, out, _) = c.run(&["add-source", &ghost.to_string_lossy(), "--allow-offline", "--name", "Ghost"]);
    assert_eq!(code, EXIT_OK);
    assert!(out.contains("Will index when it appears."), "{out}");
    let (_, out, _) = c.run(&["list-sources"]);
    assert!(out.lines().any(|l| l.contains("Ghost") && l.contains("offline")), "{out}");
    let (code, out, _) = c.run(&["scan", "Ghost"]);
    assert_eq!(code, EXIT_OFFLINE, "{out}");
    assert!(out.contains("not reachable") && out.contains("Nothing was changed"), "{out}");
    // a real one beside the ghost: partial, not 4
    assert_eq!(c.run(&["add-source", &p, "--skip-test"]).0, EXIT_OK);
    let (code, _, _) = c.run(&["scan", "--all"]);
    assert_eq!(code, EXIT_PARTIAL);
    // overlap
    let (code, _, err) = c.run(&["add-source", &root.join("2023").to_string_lossy(), "--skip-test"]);
    assert_eq!(code, EXIT_USAGE);
    assert!(err.contains("overlaps source"), "{err}");
    // usage errors
    assert_eq!(c.run(&[]).0, EXIT_USAGE);
    assert_eq!(c.run(&["frobnicate"]).0, EXIT_USAGE);
    assert_eq!(c.run(&["scan"]).0, EXIT_USAGE);
    assert_eq!(c.run(&["scan", "999"]).0, EXIT_USAGE);
    assert_eq!(c.run(&["add-source"]).0, EXIT_USAGE);
    assert_eq!(c.run(&["add-source", &p, "--policy", "sometimes"]).0, EXIT_USAGE);
    assert_eq!(c.run(&["scan", "1", "--bogus"]).0, EXIT_USAGE);
    assert_eq!(c.run(&["scan", "1", "--concurrency", "0"]).0, EXIT_USAGE);
    assert_eq!(c.run(&["add-source", "relative/path"]).0, EXIT_USAGE);
    // help
    let (code, out, _) = c.run(&["--help"]);
    assert_eq!(code, EXIT_OK);
    assert!(out.contains("Usage: immage catalog"), "{out}");
    let (code, out, _) = c.run(&["add-source", "--help"]);
    assert_eq!(code, EXIT_OK);
    assert!(out.contains("Example:"), "{out}");
    // commands that come later say so
    assert_eq!(c.run(&["thumbnails", "--all"]).0, EXIT_FAIL);
}

#[test]
fn a_catalog_that_cannot_be_used_is_exit_3() {
    let c = cli();
    // locked: a running instance holds it for writing
    let held = CatalogDb::open(&c.db).unwrap();
    let (code, _, err) = c.run(&["scan", "--all"]);
    assert_eq!(code, EXIT_CATALOG);
    assert!(err.contains("open in Immage"), "{err}");
    // reading still works beside it
    assert_eq!(c.run(&["list-sources"]).0, EXIT_OK);
    assert_eq!(c.run(&["status"]).0, EXIT_OK);
    drop(held);
    // a newer schema
    let newer = rusqlite::Connection::open(&c.db).unwrap();
    newer.pragma_update(None, "user_version", 99).unwrap();
    drop(newer);
    assert_eq!(c.run(&["status"]).0, EXIT_CATALOG);
    assert_eq!(c.run(&["scan", "--all"]).0, EXIT_CATALOG);
    // garbage
    std::fs::write(&c.db, vec![7u8; 4096]).unwrap();
    let (code, _, err) = c.run(&["verify"]);
    assert_eq!(code, EXIT_CATALOG, "{err}");
    // no catalog at all
    let none = Cli { dir: tempfile::tempdir().unwrap(), db: c.dir.path().join("missing.sqlite") };
    assert_eq!(none.run(&["status"]).0, EXIT_FAIL);
}

#[cfg(unix)]
#[test]
fn an_unreadable_folder_makes_the_scan_partial_and_names_it() {
    use std::os::unix::fs::PermissionsExt;
    let c = cli();
    let root = c.tree();
    std::fs::create_dir_all(root.join("locked")).unwrap();
    std::fs::write(root.join("locked/z.jpg"), b"x").unwrap();
    let locked = root.join("locked");
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let can_read = std::fs::read_dir(&locked).is_ok(); // root ignores permissions
    assert_eq!(c.run(&["add-source", &root.to_string_lossy(), "--skip-test"]).0, EXIT_OK);
    let (code, out, _) = c.run(&["scan", "1"]);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    if !can_read {
        assert_eq!(code, EXIT_PARTIAL, "{out}");
        assert!(out.contains("incomplete: locked (permission denied); nothing marked missing"), "{out}");
        let (code, out, _) = c.run(&["status"]);
        assert_eq!(code, EXIT_PARTIAL);
        assert!(out.contains("Attention"), "{out}");
    }
}

#[test]
fn formats_and_progress_lines() {
    assert_eq!(fmt_n(0), "0");
    assert_eq!(fmt_n(999), "999");
    assert_eq!(fmt_n(98_200), "98,200");
    assert_eq!(fmt_n(1_234_567), "1,234,567");
    assert_eq!(fmt_dur(38.0), "38 s");
    assert_eq!(fmt_dur(860.0), "14 min 20 s");
    assert_eq!(fmt_dur(7500.0), "2 h 5 min");
    assert_eq!(fmt_ago(None), "never");
    assert_eq!(parse_policy("every=6h").as_deref(), Some("every=6h"));
    assert_eq!(parse_policy("manual").as_deref(), Some("manual"));
    assert_eq!(parse_policy("launch").as_deref(), Some("launch"));
    assert_eq!(parse_policy("every=0h"), None);
    assert_eq!(parse_policy("every=xh"), None);
    let mut p = Progress {
        source_id: 1, job_id: 1, run_id: 1, phase: "finding".into(), dirs_done: 1240, dirs_queued: 3, found: 12_400, new: 0,
        changed: 0, unchanged: 0, errors: 0, elapsed_s: 10.0, rate_per_s: 1240.0, eta_s: None, current_dir: None, paused: false,
    };
    let line = progress_line("Archive 2012-2019", &p);
    assert!(line.contains("finding photos") && line.contains("12,400 so far") && line.contains("1,240 folders") && line.contains("1,240/s"), "{line}");
    assert!(!line.contains("left"));
    p.eta_s = Some((360, 720));
    let line = progress_line("A", &p);
    assert!(line.contains("about 9 min left (6 min-12 min)"), "{line}");
    p.paused = true;
    assert!(progress_line("A", &p).contains("(paused)") && !progress_line("A", &p).contains("left"));
    p.paused = false;
    p.errors = 2;
    assert!(progress_line("A", &p).contains("2 folders did not answer"));
}

#[test]
fn the_default_catalog_is_in_the_runtime_folder() {
    let p = default_db();
    assert!(p.ends_with("catalog.sqlite"));
    assert!(runtime_dir().to_string_lossy().contains("app-runtime") || runtime_dir().to_string_lossy().contains("app.immage"));
}

#[test]
fn argument_parsing() {
    let a = parse_args(&["scan".into(), "--db=/x/c.sqlite".into(), "--json".into(), "3".into(), "--concurrency".into(), "4".into(), "--light".into()]).unwrap();
    assert_eq!(a.db, Some(PathBuf::from("/x/c.sqlite")));
    assert!(a.json && a.flag("--light"));
    assert_eq!(a.concurrency, Some(4));
    assert_eq!(a.positional, ["scan", "3"]);
    let a = parse_args(&["--catalog".into(), "/d".into(), "status".into()]).unwrap();
    assert_eq!(a.db_path(), PathBuf::from("/d/catalog.sqlite"));
    assert!(parse_args(&["--db".into()]).is_err());
    assert!(parse_args(&["--nonsense".into()]).is_err());
}
