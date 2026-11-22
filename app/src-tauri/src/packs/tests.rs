// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The pack pipeline against a mock HTTP server (mockito): nothing here reaches
//! the network, and no real model weights are read or downloaded.
use super::*;
use mockito::Server;

fn sha(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn body(seed: u8, n: usize) -> Vec<u8> {
    (0..n).map(|i| (i as u8).wrapping_mul(31).wrapping_add(seed)).collect()
}
fn noop(_: &Path) -> Result<(), String> {
    Ok(())
}
fn spec_named(revision: &'static str, files: &[(&'static str, &[u8])]) -> &'static PackSpec {
    let files: Vec<PackFile> = files
        .iter()
        .map(|(path, b)| PackFile {
            path,
            size: b.len() as u64,
            digest: Box::leak(sha(b).into_boxed_str()),
            git_blob: false,
        })
        .collect();
    Box::leak(Box::new(PackSpec {
        id: PackId::Clip,
        name: "Test pack",
        license: "MIT",
        license_url: "https://example.invalid/license",
        license_text: "MIT",
        source_host: "mock.invalid",
        base_url: "https://mock.invalid/rev",
        revision,
        files: Box::leak(files.into_boxed_slice()),
        analyses: &[],
        hooks: PackHooks { prepare: noop, validate: noop },
    }))
}
struct Rig {
    _dir: tempfile::TempDir,
    store: Arc<PackStore>,
    a: Vec<u8>,
    b: Vec<u8>,
}
impl Rig {
    fn new(server: &Server) -> Rig {
        let a = body(1, 300_000);
        let b = body(7, 70_000);
        let spec = spec_named("rev", &[("onnx/a.bin", &a), ("b.json", &b)]);
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(PackStore::new(spec, dir.path().join("packs")));
        store.set_test_seams(Some(format!("{}/rev", server.url())), None);
        Rig { _dir: dir, store, a, b }
    }
    fn state(&self) -> PackState {
        self.store.info().state
    }
    fn wait_idle(&self) {
        for _ in 0..200 {
            if !self.store.is_downloading() {
                return;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("download did not stop");
    }
}
fn serve_all(server: &mut Server, rig: &Rig) -> (mockito::Mock, mockito::Mock) {
    let a = server.mock("GET", "/rev/onnx/a.bin").with_body(&rig.a).create();
    let b = server.mock("GET", "/rev/b.json").with_body(&rig.b).create();
    (a, b)
}

#[test]
fn install_from_a_mock_server_publishes_a_verified_pack() {
    let mut server = Server::new();
    let rig = Rig::new(&server);
    let (a, b) = serve_all(&mut server, &rig);
    assert_eq!(rig.state(), PackState::NotInstalled);
    rig.store.install_blocking().unwrap();
    a.assert();
    b.assert();
    let info = rig.store.info();
    assert_eq!(info.state, PackState::Installed);
    assert!(info.verified_at.is_some(), "verified right after the install");
    assert_eq!((info.files, info.files_ok), (2, 2));
    assert_eq!(info.bytes, 370_000);
    assert!(info.bytes_on_disk >= 370_000 && info.bytes_on_disk < 372_000);
    // the layout CLIP has always had: <root>/<revision>/<file>, and nothing left over
    let root = rig.store.final_dir();
    assert_eq!(std::fs::read(root.join("onnx/a.bin")).unwrap(), rig.a);
    assert!(root.join("b.json").is_file());
    let names: Vec<_> = std::fs::read_dir(root.parent().unwrap()).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(names, vec![std::ffi::OsString::from("rev")]);
    // a second install of a valid pack fetches nothing
    rig.store.install_blocking().unwrap();
    a.assert();
}

#[test]
fn an_interrupted_download_resumes_with_a_range_request() {
    let mut server = Server::new();
    let rig = Rig::new(&server);
    let head = rig.a[..100_000].to_vec();
    let cut = server
        .mock("GET", "/rev/onnx/a.bin")
        .with_chunked_body(move |w| {
            w.write_all(&head)?;
            w.flush()?;
            Err(std::io::Error::new(std::io::ErrorKind::ConnectionReset, "dropped"))
        })
        .expect(1)
        .create();
    let b = server.mock("GET", "/rev/b.json").with_body(&rig.b).expect(1).create();
    let err = rig.store.install_blocking().unwrap_err();
    cut.assert();
    assert_eq!(err.kind, PackErrorKind::Offline, "{err}");
    // stopped, not failed: the partial file is kept, and the card says what is left
    let info = rig.store.info();
    assert_eq!(info.state, PackState::Stopped);
    assert_eq!(info.error.as_ref().unwrap().kind, PackErrorKind::Offline);
    let kept = std::fs::metadata(rig.store.partial_dir().join("onnx/a.bin.part")).unwrap().len();
    assert!(kept > 0 && kept <= 100_000);
    assert_eq!(info.bytes_left, 370_000 - kept);
    assert!(!rig.store.final_dir().exists());
    cut.remove();
    // the resume asks for the rest only
    let tail = server
        .mock("GET", "/rev/onnx/a.bin")
        .match_header("range", format!("bytes={kept}-").as_str())
        .with_status(206)
        .with_header("content-range", &format!("bytes {kept}-{}/{}", rig.a.len() - 1, rig.a.len()))
        .with_body(&rig.a[kept as usize..])
        .expect(1)
        .create();
    rig.store.install_blocking().unwrap();
    tail.assert();
    b.assert();
    assert_eq!(rig.state(), PackState::Installed);
    assert_eq!(std::fs::read(rig.store.final_dir().join("onnx/a.bin")).unwrap(), rig.a);
    assert!(!rig.store.partial_dir().exists());
}

#[test]
fn a_server_that_ignores_the_range_restarts_the_file() {
    let mut server = Server::new();
    let rig = Rig::new(&server);
    let dest = rig.store.partial_dir().join("onnx");
    std::fs::create_dir_all(&dest).unwrap();
    std::fs::write(dest.join("a.bin.part"), &rig.a[..5_000]).unwrap();
    let (a, b) = serve_all(&mut server, &rig); // always a full 200 body
    rig.store.install_blocking().unwrap();
    a.assert();
    b.assert();
    assert_eq!(rig.state(), PackState::Installed);
}

#[test]
fn a_hash_mismatch_is_never_kept_or_used() {
    let mut server = Server::new();
    let rig = Rig::new(&server);
    let mut wrong = rig.a.clone();
    wrong[1234] ^= 0xff;
    server.mock("GET", "/rev/onnx/a.bin").with_body(&wrong).create();
    let _b = server.mock("GET", "/rev/b.json").with_body(&rig.b).create();
    let err = rig.store.install_blocking().unwrap_err();
    assert_eq!(err.kind, PackErrorKind::HashMismatch, "{err}");
    assert!(err.detail.contains("onnx/a.bin"));
    assert!(!rig.store.final_dir().exists());
    assert!(!rig.store.partial_dir().join("onnx/a.bin.part").exists());
    assert!(!rig.store.partial_dir().join("onnx/a.bin").exists());
    let info = rig.store.info();
    assert_eq!(info.state, PackState::NeedsRepair);
    assert_eq!(info.error.unwrap().kind, PackErrorKind::HashMismatch);
}

#[test]
fn a_file_longer_than_its_pin_is_refused() {
    let mut server = Server::new();
    let rig = Rig::new(&server);
    let mut long = rig.a.clone();
    long.push(0);
    server.mock("GET", "/rev/onnx/a.bin").with_body(&long).create();
    let err = rig.store.install_blocking().unwrap_err();
    assert_eq!(err.kind, PackErrorKind::Other, "{err}"); // the length header disagrees with the pin
    assert!(!rig.store.final_dir().exists());
}

#[test]
fn too_little_free_space_is_refused_before_any_request() {
    let mut server = Server::new();
    let rig = Rig::new(&server);
    let (a, b) = {
        let a = server.mock("GET", "/rev/onnx/a.bin").with_body(&rig.a).expect(0).create();
        let b = server.mock("GET", "/rev/b.json").with_body(&rig.b).expect(0).create();
        (a, b)
    };
    rig.store.set_test_seams(Some(format!("{}/rev", server.url())), Some(1_000));
    let err = rig.store.install_blocking().unwrap_err();
    assert_eq!(err.kind, PackErrorKind::DiskFull);
    a.assert();
    b.assert();
    let info = rig.store.info();
    assert_eq!(info.state, PackState::NotInstalled);
    assert_eq!(info.error.unwrap().kind, PackErrorKind::DiskFull);
    assert_eq!(info.free_bytes, 1_000);
}

struct FullDisk(usize);
impl Write for FullDisk {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if self.0 == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::StorageFull));
        }
        let n = buf.len().min(self.0);
        self.0 -= n;
        Ok(n)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
#[test]
fn a_write_that_runs_out_of_space_is_a_disk_full_error_and_clears_the_partial_files() {
    let data = body(3, 200_000);
    let asset = PackFile { path: "x", size: 200_000, digest: Box::leak(sha(&data).into_boxed_str()), git_blob: false };
    let err = copy_verified(&mut &data[..], &mut FullDisk(50_000), &asset).unwrap_err();
    assert_eq!(err.kind, PackErrorKind::DiskFull);
    // the pipeline removes what it kept, so no hidden gigabyte is left behind
    let spec = spec_named("rev", &[("x", &data)]);
    let dir = tempfile::tempdir().unwrap();
    let store = PackStore::new(spec, dir.path().into());
    let result = store.install_with(spec.files, |_, dest| {
        std::fs::write(dest, b"partly written").unwrap();
        Err(PackError::new(PackErrorKind::DiskFull, "disk full"))
    });
    assert_eq!(result.unwrap_err().kind, PackErrorKind::DiskFull);
    assert!(!store.partial_dir().exists());
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
}

fn slow_a(server: &mut Server, a: &[u8], pause_ms: u64) -> mockito::Mock {
    let (head, tail) = (a[..100_000].to_vec(), a[100_000..].to_vec());
    server
        .mock("GET", "/rev/onnx/a.bin")
        .with_chunked_body(move |w| {
            w.write_all(&head)?;
            w.flush()?;
            std::thread::sleep(Duration::from_millis(pause_ms));
            let _ = w.write_all(&tail);
            Ok(())
        })
        .create()
}
fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
    for _ in 0..400 {
        if f() {
            return;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    panic!("timed out waiting for {what}");
}

#[test]
fn cancel_keeps_the_partial_files_and_resume_finishes_the_pack() {
    let mut server = Server::new();
    let rig = Rig::new(&server);
    let slow = slow_a(&mut server, &rig.a, 800);
    let _b = server.mock("GET", "/rev/b.json").with_body(&rig.b).create();
    rig.store.start().unwrap();
    wait_for("some bytes", || rig.store.info().progress.is_some_and(|p| p.done > 0));
    assert_eq!(rig.state(), PackState::Downloading);
    rig.store.cancel();
    rig.wait_idle();
    let info = rig.store.info();
    assert_eq!(info.state, PackState::Stopped);
    assert!(info.error.is_none(), "a cancel is not an error");
    assert!(info.bytes_left < info.bytes && info.bytes_left > 0);
    assert!(!rig.store.final_dir().exists());
    slow.remove();
    let kept = std::fs::metadata(rig.store.partial_dir().join("onnx/a.bin.part")).unwrap().len();
    let tail = server
        .mock("GET", "/rev/onnx/a.bin")
        .match_header("range", format!("bytes={kept}-").as_str())
        .with_status(206)
        .with_header("content-range", &format!("bytes {kept}-{}/{}", rig.a.len() - 1, rig.a.len()))
        .with_body(&rig.a[kept as usize..])
        .create();
    rig.store.start().unwrap();
    rig.wait_idle();
    tail.assert();
    assert_eq!(rig.state(), PackState::Installed);
}

#[test]
fn a_second_install_or_a_check_while_one_runs_is_refused() {
    let mut server = Server::new();
    let rig = Rig::new(&server);
    let _slow = slow_a(&mut server, &rig.a, 600);
    let _b = server.mock("GET", "/rev/b.json").with_body(&rig.b).create();
    rig.store.start().unwrap();
    assert!(rig.store.start().unwrap_err().contains("already running"));
    assert!(rig.store.install_blocking().is_err());
    assert!(rig.store.check_files().is_err());
    assert!(rig.store.remove(false).unwrap_err().contains("Cancel"));
    rig.store.cancel();
    rig.wait_idle();
}

#[test]
fn remove_deletes_the_pack_or_only_the_partial_files() {
    let mut server = Server::new();
    let rig = Rig::new(&server);
    serve_all(&mut server, &rig);
    rig.store.install_blocking().unwrap();
    rig.store.remove(true).unwrap();
    assert_eq!(rig.state(), PackState::Installed, "removing partial files leaves the install");
    rig.store.remove(false).unwrap();
    let info = rig.store.info();
    assert_eq!(info.state, PackState::NotInstalled);
    assert_eq!(info.bytes_on_disk, 0);
    assert!(!rig.store.final_dir().exists());
    // partial files of a stopped download go too, and the card returns to Not installed
    let dir = rig.store.partial_dir().join("onnx");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("a.bin.part"), b"abc").unwrap();
    assert_eq!(rig.state(), PackState::Stopped);
    rig.store.remove(true).unwrap();
    assert_eq!(rig.state(), PackState::NotInstalled);
}

#[test]
fn repair_downloads_only_the_files_that_fail_their_check() {
    let mut server = Server::new();
    let rig = Rig::new(&server);
    let (a, b) = serve_all(&mut server, &rig);
    rig.store.install_blocking().unwrap();
    a.remove();
    b.remove();
    // damage one file, keeping its size (a bit flip), as a bad disk would
    let path = rig.store.final_dir().join("onnx/a.bin");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes[10] ^= 1;
    std::fs::write(&path, bytes).unwrap();
    let check = rig.store.check_files().unwrap();
    assert_eq!((check.files, check.files_ok), (2, 1));
    assert_eq!(check.failed, vec!["onnx/a.bin".to_string()]);
    let info = rig.store.info();
    assert_eq!(info.state, PackState::NeedsRepair);
    assert_eq!(info.error.unwrap().kind, PackErrorKind::HashMismatch);
    assert!(info.verified_at.is_none());
    // the file that is fine is not fetched again
    let b = server.mock("GET", "/rev/b.json").with_body(&rig.b).expect(0).create();
    let a = server.mock("GET", "/rev/onnx/a.bin").with_body(&rig.a).expect(1).create();
    rig.store.install_blocking().unwrap();
    a.assert();
    b.assert();
    assert_eq!(rig.state(), PackState::Installed);
    assert!(rig.store.info().verified_at.is_some());
    assert_eq!(std::fs::read(&path).unwrap(), rig.a);
    // the damaged folder is not kept
    assert_eq!(std::fs::read_dir(rig.store.final_dir().parent().unwrap()).unwrap().count(), 1);
}

#[test]
fn a_missing_file_makes_the_install_need_repair_without_hashing() {
    let mut server = Server::new();
    let rig = Rig::new(&server);
    serve_all(&mut server, &rig);
    rig.store.install_blocking().unwrap();
    std::fs::remove_file(rig.store.final_dir().join("b.json")).unwrap();
    let info = rig.store.info();
    assert_eq!(info.state, PackState::NeedsRepair);
    assert_eq!(info.files_ok, 1);
}

#[test]
fn an_install_made_before_checks_were_recorded_is_recognised_then_checked_once() {
    let rig = Rig::new(&Server::new());
    // what the old CLIP installer left: the revision folder with the files, no record of a check
    let root = rig.store.final_dir();
    std::fs::create_dir_all(root.join("onnx")).unwrap();
    std::fs::write(root.join("onnx/a.bin"), &rig.a).unwrap();
    std::fs::write(root.join("b.json"), &rig.b).unwrap();
    let info = rig.store.info();
    assert_eq!(info.state, PackState::Installed);
    assert!(info.verified_at.is_none());
    rig.store.ensure_checked();
    wait_for("the background check", || rig.store.info().verified_at.is_some());
    assert_eq!(rig.state(), PackState::Installed);
    // a later change to a file drops the record until it is checked again
    std::fs::write(root.join("b.json"), body(9, 70_000)).unwrap();
    assert!(rig.store.info().verified_at.is_none());
}

#[test]
fn the_manager_lists_every_pack_and_keeps_them_apart() {
    let mut server = Server::new();
    let one = Rig::new(&server);
    let two = {
        let spec = spec_named("rev2", &[("only.bin", b"hello model")]);
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(PackStore::new(spec, dir.path().join("dummy")));
        (dir, store)
    };
    let manager = PackManager::new(vec![one.store.clone(), two.1.clone()]);
    serve_all(&mut server, &one);
    one.store.install_blocking().unwrap();
    let list = manager.list();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].state, PackState::Installed);
    assert_eq!(list[1].state, PackState::NotInstalled, "a second pack is untouched by the first");
    assert_eq!(list[1].bytes, 11);
    assert!(two.1.info().folder.ends_with("dummy"));
    two.1.remove(false).unwrap();
    manager.cancel_all();
}

#[test]
fn progress_and_state_events_are_emitted() {
    let mut server = Server::new();
    let rig = Rig::new(&server);
    serve_all(&mut server, &rig);
    let seen = Arc::new(Mutex::new((0u32, 0u32)));
    let s = seen.clone();
    rig.store.set_events(Arc::new(move |e| match e {
        PackEvent::State(_) => s.lock().unwrap().0 += 1,
        PackEvent::Progress(_) => s.lock().unwrap().1 += 1,
    }));
    rig.store.start().unwrap();
    rig.wait_idle();
    let (states, progress) = *seen.lock().unwrap();
    assert!(states >= 2, "state events at the start and the end");
    assert!(progress >= 1, "at least one throttled progress event");
}
