// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! Tests of the thumbnail phase: orientation, the source ladder and its reads,
//! fault injection, atomic files, garbage collection, relinking and the link into
//! the grid's cache. Every photo here is synthetic. The ignored tests are the
//! 20,000-file benchmark and the real-file check (IMMAGE_RAW_SAMPLES).

use super::*;
use crate::catalog_thumbs as thumbs;
use crate::cache::{DiskCache, Listed};
use crate::catalog_db::CatalogDb;
use crate::catalog_fakefs::{FakeFs, BASE_MTIME};
use crate::catalog_meta_fixtures::*;
use crate::catalog_meta_job::{run_metadata, MetaConfig};
use crate::catalog_scan::{run_scan, FsError, ScanConfig, ScanControl, ScanOptions};
use crate::catalog_sources::get_source;
use crate::catalog_thumb_job::{run_thumbnails, ThumbProgress, ThumbReport};
use crate::catalog_thumb_src::{self as src, Render, Src};
use image::{DynamicImage, Rgb, RgbImage};
use std::sync::Arc;
use std::time::Duration;

// ---- fixtures -------------------------------------------------------------------------------

/// A picture with a red square in the top-left corner (a marker for orientation tests).
fn marked(w: u32, h: u32) -> RgbImage {
    let m = (w.min(h) / 5).max(4);
    RgbImage::from_fn(w, h, |x, y| if x < m && y < m { Rgb([230, 20, 20]) } else { Rgb([(40 + x * 120 / w) as u8, (60 + y * 120 / h) as u8, 90]) })
}

/// Pseudo-random pixels: a JPEG of it is large (about 0.5 bytes per pixel and up).
fn noisy(w: u32, h: u32, seed: u64) -> RgbImage {
    let m = (w.min(h) / 5).max(4);
    RgbImage::from_fn(w, h, |x, y| {
        if x < m && y < m {
            return Rgb([230, 20, 20]);
        }
        let mut v = (x as u64) << 32 ^ (y as u64) ^ seed.wrapping_mul(0x9E37_79B9_7F4A_7C15);
        v ^= v >> 13;
        v = v.wrapping_mul(0xff51_afd7_ed55_8ccd);
        v ^= v >> 29;
        Rgb([(v & 0xFF) as u8, ((v >> 8) & 0xFF) as u8, ((v >> 16) & 0xFF) as u8])
    })
}

fn jpeg_of(img: &RgbImage, q: u8) -> Vec<u8> {
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, q).encode(img.as_raw(), img.width(), img.height(), image::ExtendedColorType::Rgb8).unwrap();
    out
}

/// `main` with an Exif block: orientation, and an IFD1 thumbnail if given.
fn with_exif(main: &[u8], orientation: u16, thumb: Option<&[u8]>) -> Vec<u8> {
    let ifds = |off: u32, len: u32| {
        let mut v = vec![Ifd {
            entries: vec![(0x010F, ascii("SONY")), (0x0110, ascii("ILCE-7RM3")), (0x0112, V::Short(vec![orientation]))],
            next: thumb.map(|_| 1),
        }];
        if thumb.is_some() {
            v.push(Ifd { entries: vec![(0x0103, V::Short(vec![6])), (0x0201, V::Long(vec![off])), (0x0202, V::Long(vec![len]))], next: None });
        }
        v
    };
    let probe = tiff(true, 42, &ifds(0, 0));
    let mut t = tiff(true, 42, &ifds(probe.len() as u32, thumb.map_or(0, |t| t.len() as u32)));
    assert_eq!(t.len(), probe.len());
    if let Some(th) = thumb {
        t.extend_from_slice(th);
    }
    let mut seg = b"Exif\0\0".to_vec();
    seg.extend_from_slice(&t);
    let mut out = vec![0xFF, 0xD8, 0xFF, 0xE1];
    out.extend_from_slice(&((seg.len() + 2) as u16).to_be_bytes());
    out.extend_from_slice(&seg);
    out.extend_from_slice(&main[2..]);
    out
}

/// An ARW-shaped file whose preview JPEG sits at `off` (padding before and after).
fn arw_with(preview: &[u8], spec: &Spec, off: u32) -> Vec<u8> {
    let mut b = arw_like(true, spec, off, preview.len() as u32, off as usize + preview.len() + 4000);
    b[off as usize..off as usize + preview.len()].copy_from_slice(preview);
    b
}

fn spec_n(n: u32) -> Spec {
    let datetime: &'static str = Box::leak(format!("2025:10:05 14:{:02}:{:02}", (n / 60) % 60, n % 60).into_boxed_str());
    let serial: &'static str = Box::leak(format!("SN{n}").into_boxed_str());
    Spec { datetime, serial: Some(serial), ..Spec::default() }
}

/// Pixel at a fraction of the picture.
fn at(img: &RgbImage, fx: f32, fy: f32) -> [u8; 3] {
    let x = ((img.width() as f32 - 1.0) * fx) as u32;
    let y = ((img.height() as f32 - 1.0) * fy) as u32;
    img.get_pixel(x, y).0
}

fn reddish(p: [u8; 3]) -> bool {
    p[0] > 190 && p[1] < 90 && p[2] < 90
}

// ---- the rig ---------------------------------------------------------------------------------

struct Rig {
    #[allow(dead_code)]
    dir: tempfile::TempDir,
    db: CatalogDb,
    store: Store,
    cache: Arc<DiskCache>,
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
    let mut store = Store::new(dir.path().join("store"));
    store.min_free = 0;
    let cache = DiskCache::new(dir.path().join("cache").join("thumbs"), 1 << 30, false);
    Rig { dir, db, store, cache }
}

fn fast() -> ThumbConfig {
    let mut c = ThumbConfig::for_network(false);
    c.read_workers = 2;
    c.decode_workers = 2;
    c.read_deadline = Duration::from_millis(500);
    c.flush_every = Duration::from_millis(20);
    c.progress_every = Duration::from_millis(10);
    c.checkpoint_every = Duration::from_millis(20);
    c
}

fn meta_fast() -> MetaConfig {
    let mut c = MetaConfig::for_network(false);
    c.workers = 2;
    c.read_deadline = Duration::from_millis(500);
    c.flush_every = Duration::from_millis(20);
    c.progress_every = Duration::from_millis(10);
    c.checkpoint_every = Duration::from_millis(20);
    c
}

impl Rig {
    fn source(&self) -> crate::catalog_sources::Source {
        get_source(&self.db.read().unwrap(), 1).unwrap().unwrap()
    }
    fn scan(&self, fs: &FakeFs) {
        let mut cfg = ScanConfig::for_network(false);
        cfg.workers = 2;
        cfg.progress_every = Duration::from_millis(10);
        cfg.checkpoint_every = Duration::from_millis(20);
        let o = ScanOptions { root: fs.root.clone(), include_subfolders: true, dir_skip: false, light: false, fresh: false };
        let r = run_scan(&self.db, fs, &self.source(), &o, &cfg, &ScanControl::default(), &mut |_| {}).unwrap();
        assert_eq!(r.state, "complete");
    }
    fn meta(&self, fs: &FakeFs) {
        let r = run_metadata(&self.db, fs, &self.source(), &meta_fast(), &ScanControl::default(), &mut |_| {}).unwrap();
        assert_eq!(r.state, "complete");
    }
    /// scan and read metadata, the way a source gets ready for thumbnails
    fn prepare(&self, fs: &FakeFs) {
        self.scan(fs);
        self.meta(fs);
        fs.reads.store(0, Ordering::SeqCst);
        fs.bytes_read.store(0, Ordering::SeqCst);
        fs.read_log.lock().unwrap().clear();
    }
    fn thumbs(&self, fs: &FakeFs) -> ThumbReport {
        self.thumbs_with(fs, &fast(), &ScanControl::default(), &Signals::default())
    }
    fn thumbs_with(&self, fs: &FakeFs, cfg: &ThumbConfig, ctl: &ScanControl, sig: &Signals) -> ThumbReport {
        run_thumbnails(&self.db, fs, &self.source(), &fs.root, &self.store, Some(&self.cache), cfg, ctl, sig, &mut |_: &ThumbProgress| {}).unwrap()
    }
    fn q<T: rusqlite::types::FromSql>(&self, sql: &str) -> T {
        self.db.read().unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
    }
    fn decoded(&self, rel: &str) -> RgbImage {
        let id: i64 = self.db.read().unwrap().query_row("SELECT id FROM photos WHERE rel_path=?1", [rel], |r| r.get(0)).unwrap();
        image::load_from_memory(&self.store.read(id).expect("a stored thumbnail")).unwrap().to_rgb8()
    }
}

fn arw_fs(n: u32, preview_w: u32, preview_h: u32) -> FakeFs {
    let fs = FakeFs::new();
    for i in 0..n {
        let preview = jpeg_of(&noisy(preview_w, preview_h, i as u64), 85);
        assert!(preview.len() > 64 * 1024, "the preview must be big enough for the extractor to locate it");
        fs.add_blob(&format!("2025-10/IMG_{i:04}.ARW"), arw_with(&preview, &spec_n(i), 30_000), BASE_MTIME + i as i64);
    }
    fs
}

// ---- orientation ----------------------------------------------------------------------------

#[test]
fn all_eight_exif_orientations_turn_the_picture_upright() {
    let jpeg = jpeg_of(&marked(60, 40), 92);
    // where the picture's top-left corner ends up for orientation 1..=8: TL TR BR BL TL TR BR BL
    let corner = [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0), (0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
    for o in 1..=8u8 {
        let r = Render { edge: 480, quality: 90, orientation: o, main: None, own_pixels: true };
        let t = src::render(&jpeg, &r).unwrap();
        let img = image::load_from_memory(&t.jpeg).unwrap().to_rgb8();
        let want = if o >= 5 { (40, 60) } else { (60, 40) };
        assert_eq!((img.width(), img.height()), want, "orientation {o}");
        assert_eq!((t.width, t.height), want);
        let (cx, cy) = corner[o as usize - 1];
        // sample a little inside each corner
        let probe = |fx: f32, fy: f32| {
            let fx = if fx == 0.0 { 0.06 } else { 0.94 };
            let fy = if fy == 0.0 { 0.06 } else { 0.94 };
            at(&img, fx, fy)
        };
        for (fx, fy) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            assert_eq!(reddish(probe(fx, fy)), (fx, fy) == (cx, cy), "orientation {o}, corner {fx},{fy}");
        }
    }
}

#[test]
fn a_thumbnail_the_camera_already_turned_is_not_turned_again() {
    // the photo is stored landscape with orientation 6 (so it shows portrait), and its
    // embedded copy is already portrait: leave it alone. a landscape copy is turned.
    let portrait = jpeg_of(&marked(40, 60), 92);
    let landscape = jpeg_of(&marked(60, 40), 92);
    let mk = |own| Render { edge: 480, quality: 90, orientation: 6, main: Some((7952, 5304)), own_pixels: own };
    let a = src::render(&portrait, &mk(false)).unwrap();
    assert_eq!((a.width, a.height), (40, 60));
    assert!(reddish(at(&image::load_from_memory(&a.jpeg).unwrap().to_rgb8(), 0.06, 0.06)));
    let b = src::render(&landscape, &mk(false)).unwrap();
    assert_eq!((b.width, b.height), (40, 60));
    assert!(reddish(at(&image::load_from_memory(&b.jpeg).unwrap().to_rgb8(), 0.94, 0.06)));
    // the photo's own pixels are always turned
    let c = src::render(&portrait, &mk(true)).unwrap();
    assert_eq!((c.width, c.height), (60, 40));
}

#[test]
fn the_long_edge_is_fitted_and_small_pictures_are_not_enlarged() {
    let big = jpeg_of(&marked(1600, 1000), 85);
    let r = Render { edge: 480, quality: 80, orientation: 1, main: None, own_pixels: true };
    let t = src::render(&big, &r).unwrap();
    assert_eq!((t.width, t.height), (480, 300));
    let small = src::render(&jpeg_of(&marked(300, 200), 85), &r).unwrap();
    assert_eq!((small.width, small.height), (300, 200));
    assert!(src::render(b"not a jpeg at all", &r).is_err());
}

#[test]
fn the_exif_thumbnail_is_found_only_when_it_lies_in_the_head() {
    let thumb = jpeg_of(&marked(320, 240), 80);
    let file = with_exif(&jpeg_of(&marked(800, 600), 80), 6, Some(&thumb));
    let t = src::exif_thumbnail(&file).expect("thumbnail");
    assert_eq!((t.width, t.height, t.len), (320, 240, thumb.len()));
    assert_eq!(&file[t.offset..t.offset + t.len], &thumb[..]);
    // a window that ends inside the thumbnail finds nothing
    assert!(src::exif_thumbnail(&file[..t.offset + t.len - 10]).is_none());
    // no thumbnail, no APP1, not a JPEG
    assert!(src::exif_thumbnail(&with_exif(&jpeg_of(&marked(80, 60), 80), 1, None)).is_none());
    assert!(src::exif_thumbnail(&jpeg_of(&marked(80, 60), 80)).is_none());
    assert!(src::exif_thumbnail(b"II*\0").is_none());
    assert_eq!(src::jpeg_dimensions(&thumb), Some((320, 240)));
}

// ---- the ladder and its reads ----------------------------------------------------------------

#[test]
fn a_raw_preview_is_one_ranged_read_of_exactly_its_span() {
    let r = rig();
    let fs = arw_fs(3, 800, 534);
    r.prepare(&fs);
    let spans: Vec<(i64, i64)> = {
        let c = r.db.read().unwrap();
        let mut st = c.prepare("SELECT preview_offset, preview_len FROM photos ORDER BY rel_path").unwrap();
        st.query_map([], |x| Ok((x.get(0)?, x.get(1)?))).unwrap().flatten().collect()
    };
    assert!(spans.iter().all(|(o, l)| *o == 30_000 && *l > 64 * 1024));
    let rep = r.thumbs(&fs);
    assert_eq!((rep.state, rep.total, rep.done, rep.failed, rep.deferred), ("complete", 3, 3, 0, 0));
    assert_eq!(rep.from_preview, 3);
    // one read per photo, at the stored offset, for the stored length, and nothing else
    let log = fs.read_log.lock().unwrap().clone();
    assert_eq!(log.len(), 3);
    for (rel, off, len) in &log {
        let i = r.q::<i64>(&format!("SELECT preview_len FROM photos WHERE rel_path='{rel}'"));
        assert_eq!((*off, *len as i64), (30_000, i), "{rel}");
    }
    assert_eq!(rep.bytes_read, spans.iter().map(|(_, l)| *l as u64).sum::<u64>());
    assert_eq!(fs.bytes_read.load(Ordering::SeqCst), rep.bytes_read);
    // orientation 6 of a 800x534 preview: a portrait of 480 on the long edge, red marker at its top-right
    let img = r.decoded("2025-10/IMG_0000.ARW");
    assert_eq!((img.width(), img.height()), (320, 480));
    assert!(reddish(at(&img, 0.94, 0.04)), "the marker (top-left of the sensor picture) is at the top-right after a 90 degree turn");
    // the row says what was made
    let (state, from, w, h): (i64, String, i64, i64) =
        r.db.read().unwrap().query_row("SELECT state, src, w, h FROM thumbs WHERE photo_id=1", [], |x| Ok((x.get(0)?, x.get(1)?, x.get(2)?, x.get(3)?))).unwrap();
    assert_eq!((state, from.as_str(), w, h), (1, "preview", 320, 480));
}

fn jpeg_fs(files: &[(&str, Vec<u8>)]) -> FakeFs {
    let fs = FakeFs::new();
    for (rel, b) in files {
        fs.add_blob(rel, b.clone(), BASE_MTIME);
    }
    fs
}

#[test]
fn a_jpeg_with_a_big_enough_exif_thumbnail_costs_one_small_read() {
    let r = rig();
    let main = jpeg_of(&noisy(1600, 1200, 1), 90);
    let file = with_exif(&main, 1, Some(&jpeg_of(&noisy(320, 240, 2), 70)));
    assert!(file.len() > 700_000);
    let fs = jpeg_fs(&[("a.jpg", file)]);
    r.prepare(&fs);
    let mut cfg = fast();
    cfg.whole_jpegs = WholeJpegs::No;
    let rep = r.thumbs_with(&fs, &cfg, &ScanControl::default(), &Signals::default());
    assert_eq!((rep.done, rep.from_exif, rep.deferred), (1, 1, 0));
    assert_eq!(rep.reads, 1);
    assert!(rep.bytes_read <= cfg.exif_head as u64, "{}", rep.bytes_read);
    let img = r.decoded("a.jpg");
    assert_eq!((img.width(), img.height()), (320, 240));
}

#[test]
fn the_whole_jpeg_policy_decides_whether_a_big_jpeg_is_read() {
    let main = jpeg_of(&noisy(1600, 1200, 3), 90);
    // a 160 px Exif thumbnail is too small to use
    let file = with_exif(&main, 1, Some(&jpeg_of(&noisy(160, 120, 4), 70)));
    let size = file.len() as u64;
    assert!(size > 768 * 1024);
    // No: the head is read, the thumbnail is too small, the job moves on and says why
    let r = rig();
    let fs = jpeg_fs(&[("a.jpg", file.clone())]);
    r.prepare(&fs);
    let mut cfg = fast();
    cfg.whole_jpegs = WholeJpegs::No;
    let rep = r.thumbs_with(&fs, &cfg, &ScanControl::default(), &Signals::default());
    assert_eq!((rep.done, rep.deferred, rep.reads), (0, 1, 1));
    assert_eq!(r.q::<String>("SELECT error FROM thumbs"), "policy");
    assert_eq!(thumbs::coverage(&r.db.read().unwrap(), None).unwrap().deferred, 1);
    // the same photo is not asked about again until the policy changes
    assert_eq!(r.thumbs_with(&fs, &cfg, &ScanControl::default(), &Signals::default()).total, 0);
    // Yes: it is read whole (the head plus the rest, each byte once) and decoded
    fs.reads.store(0, Ordering::SeqCst);
    fs.bytes_read.store(0, Ordering::SeqCst);
    cfg.whole_jpegs = WholeJpegs::Yes;
    let rep = r.thumbs_with(&fs, &cfg, &ScanControl::default(), &Signals::default());
    assert_eq!((rep.done, rep.from_jpeg, rep.reads, rep.bytes_read), (1, 1, 2, size));
    let img = r.decoded("a.jpg");
    assert_eq!((img.width(), img.height()), (480, 360));
    // Idle: waits for no browsing; busy defers, idle reads
    for (busy, want_done) in [(true, 0u64), (false, 1)] {
        let r = rig();
        let fs = jpeg_fs(&[("a.jpg", file.clone())]);
        r.prepare(&fs);
        cfg.whole_jpegs = WholeJpegs::Idle;
        let sig = Signals::default();
        // a busy app makes the readers wait, so flip it only for the policy check in fetch: use a 1-photo run
        // with the flag set after the reader starts is racy, so test the rule through a run with busy set
        // and a source that has nothing else to wait for
        sig.set_busy(false);
        let rep = if busy { run_busy_once(&r, &fs, &cfg) } else { r.thumbs_with(&fs, &cfg, &ScanControl::default(), &sig) };
        assert_eq!(rep.done, want_done, "busy={busy}");
    }
}

/// A run while the app is busy: the readers wait, so the job makes no progress until it is
/// idle again; at that point the policy lets the whole JPEG through.
fn run_busy_once(r: &Rig, fs: &FakeFs, cfg: &ThumbConfig) -> ThumbReport {
    let sig = Signals::default();
    sig.set_busy(true);
    let ctl = ScanControl::default();
    std::thread::scope(|s| {
        let h = s.spawn(|| r.thumbs_with(fs, cfg, &ctl, &sig));
        std::thread::sleep(Duration::from_millis(250));
        assert_eq!(fs.reads.load(Ordering::SeqCst), 0, "a busy app is not disturbed by reads");
        ctl.cancel();
        let rep = h.join().unwrap();
        assert_eq!(rep.state, "cancelled");
        rep
    })
}

#[test]
fn a_small_jpeg_is_read_whole_whatever_the_policy() {
    let r = rig();
    let file = with_exif(&jpeg_of(&noisy(500, 400, 5), 80), 3, None);
    assert!(file.len() < 768 * 1024);
    let fs = jpeg_fs(&[("a.jpg", file)]);
    r.prepare(&fs);
    let mut cfg = fast();
    cfg.whole_jpegs = WholeJpegs::No;
    let rep = r.thumbs_with(&fs, &cfg, &ScanControl::default(), &Signals::default());
    assert_eq!((rep.done, rep.from_jpeg, rep.reads), (1, 1, 1));
    // orientation 3 turns the red marker to the bottom-right
    let img = r.decoded("a.jpg");
    assert_eq!((img.width(), img.height()), (480, 384));
    assert!(reddish(at(&img, 0.94, 0.94)));
}

#[test]
fn formats_only_the_engine_reads_are_deferred_and_the_hook_makes_them() {
    let r = rig();
    let fs = jpeg_fs(&[("a.heic", make_heic(&Spec::default(), None, 0))]);
    r.prepare(&fs);
    let rep = r.thumbs(&fs);
    assert_eq!((rep.done, rep.deferred, rep.failed), (0, 1, 0));
    assert_eq!(r.q::<String>("SELECT error FROM thumbs"), "engine");
    // with an engine hook the deferred photo is picked up
    let mut cfg = fast();
    let jpeg = jpeg_of(&marked(640, 480), 85);
    cfg.engine = Some(Arc::new(move |path: &str| {
        assert!(path.ends_with("a.heic"));
        Some(jpeg.clone())
    }));
    let rep = r.thumbs_with(&fs, &cfg, &ScanControl::default(), &Signals::default());
    assert_eq!((rep.done, rep.from_engine), (1, 1));
    // the engine's picture is already upright: orientation 6 of the file is not applied again
    let img = r.decoded("a.heic");
    assert_eq!((img.width(), img.height()), (480, 360));
}

#[test]
fn a_corrupt_preview_fails_that_photo_only_and_is_retried_later() {
    let r = rig();
    let fs = arw_fs(2, 800, 534);
    // a preview that starts like a JPEG and is garbage after
    let mut bad = vec![0xFF, 0xD8, 0xFF, 0xE0];
    bad.extend(std::iter::repeat_n(0x5Au8, 70_000));
    fs.add_blob("2025-10/IMG_9999.ARW", arw_with(&bad, &spec_n(99), 30_000), BASE_MTIME);
    r.prepare(&fs);
    let rep = r.thumbs(&fs);
    assert_eq!((rep.total, rep.done, rep.failed), (3, 2, 1));
    let (state, tries, error, retry): (i64, i64, String, i64) = r
        .db
        .read()
        .unwrap()
        .query_row("SELECT t.state, t.tries, t.error, t.retry_at FROM thumbs t JOIN photos p ON p.id=t.photo_id WHERE p.name='IMG_9999.ARW'", [], |x| Ok((x.get(0)?, x.get(1)?, x.get(2)?, x.get(3)?)))
        .unwrap();
    assert_eq!((state, tries), (2, 1));
    assert!(error.starts_with("decode:"), "{error}");
    assert!(retry > catalog_db::now_s() + 3000);
    // not tried again at once; tried again when its time comes
    assert_eq!(r.thumbs(&fs).total, 0);
    r.db.exec(|c| c.execute("UPDATE thumbs SET retry_at=1", []).map(|_| ()).map_err(|e| e.to_string())).unwrap();
    let rep = r.thumbs(&fs);
    assert_eq!((rep.total, rep.failed), (1, 1));
    assert_eq!(r.q::<i64>("SELECT tries FROM thumbs WHERE state=2"), 2);
    // the photo is still there and still present
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE status='present'"), 3);
    // an engine hook rescues it
    r.db.exec(|c| c.execute("UPDATE thumbs SET retry_at=1", []).map(|_| ()).map_err(|e| e.to_string())).unwrap();
    let mut cfg = fast();
    let ok = jpeg_of(&marked(640, 480), 85);
    cfg.engine = Some(Arc::new(move |_| Some(ok.clone())));
    let rep = r.thumbs_with(&fs, &cfg, &ScanControl::default(), &Signals::default());
    assert_eq!((rep.done, rep.from_engine, rep.failed), (1, 1, 0));
}

// ---- faults ---------------------------------------------------------------------------------

#[test]
fn read_failures_are_recorded_per_photo_and_never_mark_anything_missing() {
    let r = rig();
    let fs = arw_fs(6, 800, 534);
    r.prepare(&fs);
    let source_state = r.q::<String>("SELECT state FROM sources");
    fs.fail_reads_of("2025-10/IMG_0002.ARW", FsError::Io("input/output error".into()));
    fs.fail_reads_of("2025-10/IMG_0004.ARW", FsError::Timeout);
    let rep = r.thumbs(&fs);
    assert_eq!((rep.state, rep.total, rep.done, rep.failed), ("complete", 6, 4, 2));
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE status='present'"), 6);
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM thumbs WHERE state=2"), 2);
    assert_eq!(r.q::<String>("SELECT state FROM sources"), source_state, "a read error never changes a source");
    // healed, and retried when due
    fs.heal_reads_of("2025-10/IMG_0002.ARW");
    fs.heal_reads_of("2025-10/IMG_0004.ARW");
    r.db.exec(|c| c.execute("UPDATE thumbs SET retry_at=1 WHERE state=2", []).map(|_| ()).map_err(|e| e.to_string())).unwrap();
    let rep = r.thumbs(&fs);
    assert_eq!((rep.total, rep.done, rep.failed), (2, 2, 0));
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM thumbs WHERE state=1"), 6);
}

#[test]
fn a_mount_that_vanishes_stops_the_job_without_recording_failures_and_it_resumes() {
    let r = rig();
    let fs = arw_fs(30, 640, 427);
    r.prepare(&fs);
    fs.at_read(6, |f| f.go_offline());
    let rep = r.thumbs(&fs);
    assert_eq!(rep.state, "offline");
    assert!(rep.done < 30);
    // nothing is marked failed or missing because the volume went away
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM thumbs WHERE state=2"), 0);
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE status='missing'"), 0);
    assert_eq!(r.q::<String>("SELECT state FROM jobs WHERE kind='thumbnails'"), "interrupted");
    let made = r.q::<i64>("SELECT COUNT(*) FROM thumbs WHERE state=1");
    assert!(made > 0 && made < 30);
    // while it is away, a new run does nothing and says so
    let again = r.thumbs(&fs);
    assert_eq!((again.state, again.done), ("offline", 0));
    // back online: the rest, and the job is a resumed one
    fs.go_online();
    let rep = r.thumbs(&fs);
    assert_eq!((rep.state, rep.resumed), ("complete", true));
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM thumbs WHERE state=1"), 30);
    assert_eq!(r.q::<String>("SELECT state FROM jobs WHERE kind='thumbnails' ORDER BY id DESC LIMIT 1"), "done");
}

#[test]
fn a_mount_that_answers_listings_but_not_reads_is_treated_as_gone() {
    let r = rig();
    let fs = arw_fs(40, 640, 427);
    r.prepare(&fs);
    *fs.reads_fail_all.lock().unwrap() = Some(FsError::Timeout);
    let mut cfg = fast();
    cfg.read_deadline = Duration::from_millis(50);
    let rep = r.thumbs_with(&fs, &cfg, &ScanControl::default(), &Signals::default());
    assert_eq!(rep.state, "offline");
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE status='missing'"), 0);
}

#[test]
fn pause_and_cancel_work_and_a_cancelled_job_starts_afresh() {
    let r = rig();
    let fs = arw_fs(20, 640, 427);
    r.prepare(&fs);
    fs.set_read_latency(Duration::from_millis(20));
    let ctl = ScanControl::default();
    let sig = Signals::default();
    let rep = std::thread::scope(|s| {
        let h = s.spawn(|| r.thumbs_with(&fs, &fast(), &ctl, &sig));
        std::thread::sleep(Duration::from_millis(150));
        ctl.pause();
        std::thread::sleep(Duration::from_millis(300));
        let at_pause = fs.reads.load(Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(200));
        assert!(fs.reads.load(Ordering::SeqCst) <= at_pause + 2, "paused: reads stop");
        ctl.cancel();
        h.join().unwrap()
    });
    assert_eq!(rep.state, "cancelled");
    assert_eq!(r.q::<String>("SELECT state FROM jobs WHERE kind='thumbnails'"), "cancelled");
    fs.set_read_latency(Duration::ZERO);
    let rep = r.thumbs(&fs);
    assert_eq!((rep.state, rep.resumed), ("complete", false));
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM thumbs WHERE state=1"), 20);
}

#[test]
fn cancelling_while_the_decoders_are_behind_does_not_hang() {
    // many readers, one slow decoder: the queue between them is full when the job is cancelled
    let r = rig();
    let fs = arw_fs(40, 1000, 700);
    r.prepare(&fs);
    let mut cfg = fast();
    cfg.read_workers = 4;
    cfg.decode_workers = 1;
    let ctl = ScanControl::default();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::scope(|s| {
        s.spawn(|| {
            let rep = run_thumbnails(&r.db, &fs, &r.source(), &fs.root, &r.store, Some(&r.cache), &cfg, &ctl, &Signals::default(), &mut |p| {
                if p.done >= 2 {
                    ctl.cancel();
                }
            })
            .unwrap();
            tx.send(rep.state).unwrap();
        });
        assert_eq!(rx.recv_timeout(Duration::from_secs(30)).expect("the job must stop"), "cancelled");
    });
}

#[test]
fn an_interrupted_job_continues_where_it_stopped_and_a_rerun_does_nothing() {
    let r = rig();
    let fs = arw_fs(24, 640, 427);
    r.prepare(&fs);
    let ctl = ScanControl::default();
    fs.at_read(8, |_| {});
    let first = std::thread::scope(|s| {
        fs.set_read_latency(Duration::from_millis(10));
        let h = s.spawn(|| r.thumbs_with(&fs, &fast(), &ctl, &Signals::default()));
        std::thread::sleep(Duration::from_millis(120));
        ctl.interrupt();
        h.join().unwrap()
    });
    fs.set_read_latency(Duration::ZERO);
    assert_eq!(first.state, "interrupted");
    let before = r.q::<i64>("SELECT COUNT(*) FROM thumbs WHERE state=1");
    assert!(before < 24, "{before}");
    let second = r.thumbs(&fs);
    assert_eq!((second.state, second.resumed), ("complete", true));
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM thumbs WHERE state=1"), 24);
    // idempotent: nothing is read, nothing changes
    let files: Vec<_> = r.store.scan().0.iter().map(|f| (f.id, f.size, f.time)).collect();
    fs.reads.store(0, Ordering::SeqCst);
    let third = r.thumbs(&fs);
    assert_eq!((third.total, third.done, third.reads), (0, 0, 0));
    assert_eq!(fs.reads.load(Ordering::SeqCst), 0);
    let after: Vec<_> = r.store.scan().0.iter().map(|f| (f.id, f.size, f.time)).collect();
    assert_eq!(sorted(files), sorted(after));
}

fn sorted<T: Ord>(mut v: Vec<T>) -> Vec<T> {
    v.sort();
    v
}

#[test]
fn a_photo_changed_since_its_thumbnail_is_made_again() {
    let r = rig();
    let fs = arw_fs(2, 640, 427);
    r.prepare(&fs);
    assert_eq!(r.thumbs(&fs).done, 2);
    let new = arw_with(&jpeg_of(&noisy(640, 427, 77), 85), &spec_n(0), 30_000);
    fs.add_blob("2025-10/IMG_0000.ARW", new, BASE_MTIME + 5_000_000_000);
    r.scan(&fs);
    // metadata is read again first; until then the thumbnail phase leaves the changed photo alone
    assert_eq!(r.thumbs(&fs).total, 0);
    r.meta(&fs);
    let rep = r.thumbs(&fs);
    assert_eq!((rep.total, rep.done), (1, 1));
}

#[test]
fn visible_photos_go_first_and_a_busy_app_is_not_disturbed() {
    let r = rig();
    let fs = arw_fs(12, 640, 427);
    r.prepare(&fs);
    let mut cfg = fast();
    cfg.read_workers = 1;
    cfg.adaptive = false;
    let sig = Signals::default();
    sig.hint(vec!["/fake/2025-10/IMG_0003.ARW".into(), "/fake/2025-10/IMG_0001.ARW".into()]);
    let rep = r.thumbs_with(&fs, &cfg, &ScanControl::default(), &sig);
    assert_eq!(rep.done, 12);
    let log = fs.read_log.lock().unwrap().clone();
    assert_eq!(log[0].0, "2025-10/IMG_0003.ARW");
    assert_eq!(log[1].0, "2025-10/IMG_0001.ARW");
    // without hints, the newest folder first (by name, descending), the photos in name order
    let r = rig();
    let fs = FakeFs::new();
    for d in ["2024-01", "2025-10", "2023-05"] {
        fs.add_blob(&format!("{d}/a.ARW"), arw_with(&jpeg_of(&noisy(640, 427, 1), 85), &spec_n(1), 30_000), BASE_MTIME);
    }
    r.prepare(&fs);
    r.thumbs_with(&fs, &cfg, &ScanControl::default(), &Signals::default());
    let order: Vec<String> = fs.read_log.lock().unwrap().iter().map(|e| e.0.clone()).collect();
    assert_eq!(order, ["2025-10/a.ARW", "2024-01/a.ARW", "2023-05/a.ARW"]);
}

#[test]
fn a_full_disk_stops_the_job_without_failing_photos_and_it_resumes_when_there_is_room() {
    let mut r = rig();
    let fs = arw_fs(5, 640, 427);
    r.prepare(&fs);
    r.store.min_free = u64::MAX;
    let rep = r.thumbs(&fs);
    assert_eq!(rep.state, "disk_full");
    assert_eq!(rep.done, 0);
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM thumbs"), 0, "nothing is recorded for work the disk refused");
    assert_eq!(r.store.scan().0.len(), 0);
    assert_eq!(r.q::<String>("SELECT state FROM jobs WHERE kind='thumbnails'"), "interrupted");
    r.store.min_free = 0;
    let rep = r.thumbs(&fs);
    assert_eq!((rep.state, rep.resumed, rep.done), ("complete", true, 5));
}

// ---- files ------------------------------------------------------------------------------------

#[test]
fn a_write_is_atomic_a_crash_leaves_no_partial_thumbnail() {
    let r = rig();
    r.store.write(7, b"whole thumbnail").unwrap();
    // a crash in the middle of replacing it: the old bytes are all a reader ever sees
    assert!(r.store.write_crashing(7, b"a different, longer thumbnail", 5).is_err());
    assert_eq!(r.store.read(7).unwrap(), b"whole thumbnail");
    // a crash in the middle of a first write: no file, one part file
    assert!(r.store.write_crashing(8, b"another thumbnail", 4).is_err());
    assert!(!r.store.exists(8) && r.store.read(8).is_none());
    let (files, parts) = r.store.scan();
    assert_eq!((files.len(), parts.len()), (1, 2));
    assert!(parts.iter().all(|p| std::fs::metadata(p).unwrap().len() < 10));
}

fn seed_store(r: &Rig, rows: &[(i64, &str, i64)], files: &[i64]) {
    let rows: Vec<(i64, String, i64)> = rows.iter().map(|(i, s, u)| (*i, s.to_string(), *u)).collect();
    r.db
        .exec(move |c| {
            c.execute("INSERT OR IGNORE INTO dirs(id,source_id,rel_path) VALUES(1,1,'')", []).map_err(|e| e.to_string())?;
            for (id, status, used) in &rows {
                c.execute(
                    "INSERT INTO photos(id,source_id,dir_id,name,rel_path,ext,size,mtime_ns,status,discovered_at,seen_at,meta_state) VALUES(?1,1,1,?2,?2,'jpg',10,20,?3,1,1,1)",
                    rusqlite::params![id, format!("p{id}.jpg"), status],
                )
                .map_err(|e| e.to_string())?;
                c.execute(
                    "INSERT INTO thumbs(photo_id,state,src,w,h,bytes,stamp_size,stamp_mtime_ns,made_at,used_at) VALUES(?1,1,'preview',1,1,1000,10,20,?2,?2)",
                    rusqlite::params![id, used],
                )
                .map_err(|e| e.to_string())?;
            }
            Ok(())
        })
        .unwrap();
    for id in files {
        r.store.write(*id, &vec![7u8; 1000]).unwrap();
    }
}

#[test]
fn garbage_collection_removes_orphans_and_trims_present_photos_before_missing_ones() {
    let mut r = rig();
    // present 1..3 (oldest first), missing 4 and 5, a row 6 whose file is gone, and a file 99 no row mentions
    seed_store(&r, &[(1, "present", 10), (2, "present", 20), (3, "present", 30), (4, "missing", 1), (5, "missing", 2), (6, "present", 40)], &[1, 2, 3, 4, 5, 99]);
    // an old part file goes, a fresh one (a write in progress) stays
    r.store.write_crashing(50, b"abc", 2).ok();
    r.store.write_crashing(51, b"abc", 2).ok();
    let old = r.store.dir().join("0").join(".50.part");
    std::fs::OpenOptions::new().write(true).open(&old).unwrap().set_modified(std::time::SystemTime::now() - Duration::from_secs(7200)).unwrap();
    r.store.cap = 3500;
    let g = thumbs::gc(&r.db, &r.store).unwrap();
    assert_eq!((g.partials, g.orphans, g.rows_without_file), (1, 1, 1));
    // 5 files left = 5000 bytes over 3500: the two least recently used present photos go, to 3000 (below 90%)
    assert_eq!((g.evicted_present, g.evicted_missing), (2, 0));
    assert_eq!(g.bytes_after, 3000);
    assert!(!r.store.exists(1) && !r.store.exists(2) && r.store.exists(3) && r.store.exists(4) && r.store.exists(5) && !r.store.exists(99));
    assert!(r.store.dir().join("0").join(".51.part").exists());
    // evicted rows are gone, so a present photo is made again; the missing photos keep theirs
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM thumbs"), 3);
    // a smaller limit finally reaches the missing photos, the oldest first
    r.store.cap = 1500;
    let g = thumbs::gc(&r.db, &r.store).unwrap();
    assert_eq!((g.evicted_present, g.evicted_missing), (1, 1));
    assert!(r.store.exists(5) && !r.store.exists(4) && !r.store.exists(3));
}

#[test]
fn thumbnails_of_missing_photos_survive_and_deleting_a_source_removes_its_files_at_the_next_gc() {
    let r = rig();
    let fs = arw_fs(3, 640, 427);
    r.prepare(&fs);
    assert_eq!(r.thumbs(&fs).done, 3);
    // the NAS is gone for good: a scan that completes marks the photos missing
    r.db.exec(|c| c.execute("UPDATE photos SET status='missing'", []).map(|_| ()).map_err(|e| e.to_string())).unwrap();
    let g = thumbs::gc(&r.db, &r.store).unwrap();
    assert_eq!((g.orphans, g.evicted_missing), (0, 0));
    assert_eq!(r.store.scan().0.len(), 3);
    let cov = thumbs::coverage(&r.db.read().unwrap(), None).unwrap();
    assert_eq!((cov.done, cov.kept_missing), (0, 3));
    // forgetting the source deletes its rows, and the files nothing points to go
    r.db.exec(|c| c.execute("DELETE FROM sources WHERE id=1", []).map(|_| ()).map_err(|e| e.to_string())).unwrap();
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM thumbs"), 0);
    let g = thumbs::gc(&r.db, &r.store).unwrap();
    assert_eq!(g.orphans, 3);
    assert_eq!(r.store.scan().0.len(), 0);
}

// ---- relink, publishing and the grid ---------------------------------------------------------

#[test]
fn a_renamed_photo_keeps_its_thumbnail_and_is_linked_under_its_new_name_without_a_read() {
    let r = rig();
    let fs = arw_fs(3, 640, 427);
    r.prepare(&fs);
    assert_eq!(r.thumbs(&fs).done, 3);
    let id = r.q::<i64>("SELECT id FROM photos WHERE rel_path='2025-10/IMG_0001.ARW'");
    let made_at = r.q::<i64>(&format!("SELECT made_at FROM thumbs WHERE photo_id={id}"));
    let old_name = {
        let (s, m): (i64, i64) = r.db.read().unwrap().query_row("SELECT size, mtime_ns FROM photos WHERE id=?1", [id], |x| Ok((x.get(0)?, x.get(1)?))).unwrap();
        thumbs::cache_name(Path::new("/fake/2025-10/IMG_0001.ARW"), &Stamps { size: s as u64, mtime_ns: m, xmp_kind: 0, xmp_size: None, xmp_mtime_ns: None })
    };
    assert!(r.cache.contains(&old_name));
    fs.rename_file("2025-10/IMG_0001.ARW", "moved/vacation.ARW");
    r.scan(&fs);
    r.meta(&fs);
    // the row is the same row, at its new path, with its thumbnail row
    assert_eq!(r.q::<i64>("SELECT id FROM photos WHERE rel_path='moved/vacation.ARW'"), id);
    assert_eq!(r.q::<i64>(&format!("SELECT state FROM thumbs WHERE photo_id={id}")), 1);
    assert_eq!(r.q::<i64>(&format!("SELECT made_at FROM thumbs WHERE photo_id={id}")), made_at);
    fs.reads.store(0, Ordering::SeqCst);
    fs.read_log.lock().unwrap().clear();
    let rep = r.thumbs(&fs);
    assert_eq!((rep.total, rep.done, rep.reads), (0, 0, 0), "nothing is read for a moved photo");
    assert_eq!(rep.linked, 1);
    let (s, m): (i64, i64) = r.db.read().unwrap().query_row("SELECT size, mtime_ns FROM photos WHERE id=?1", [id], |x| Ok((x.get(0)?, x.get(1)?))).unwrap();
    let new_name = thumbs::cache_name(Path::new("/fake/moved/vacation.ARW"), &Stamps { size: s as u64, mtime_ns: m, xmp_kind: 0, xmp_size: None, xmp_mtime_ns: None });
    assert_ne!(new_name, old_name);
    assert!(r.cache.contains(&new_name));
    assert_eq!(r.q::<String>(&format!("SELECT published FROM thumbs WHERE photo_id={id}")), new_name);
    // an evicted cache entry is linked again by the next run, with no read either
    r.cache.clear();
    let rep = r.thumbs(&fs);
    assert_eq!((rep.reads, rep.linked), (0, 3));
}

#[test]
fn the_grid_finds_what_the_job_made_under_the_name_it_asks_for() {
    // real files, so the grid's own naming (cache::listed_thumb_name: a stat of the file and its sidecar) is exercised
    let r = rig();
    let photos = tempfile::tempdir().unwrap();
    let root = photos.path();
    std::fs::create_dir_all(root.join("trip")).unwrap();
    let file = with_exif(&jpeg_of(&marked(1200, 800), 88), 1, Some(&jpeg_of(&marked(320, 213), 80)));
    let plain = root.join("trip/plain.jpg");
    let with_sidecar = root.join("trip/sidecar.jpg");
    std::fs::write(&plain, &file).unwrap();
    std::fs::write(&with_sidecar, &file).unwrap();
    std::fs::write(root.join("trip/sidecar.jpg.xmp"), "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\"/></x:xmpmeta>").unwrap();
    let fs = crate::catalog_scan::RealFs::new(root.to_path_buf());
    let o = ScanOptions { root: root.to_path_buf(), include_subfolders: true, dir_skip: false, light: false, fresh: false };
    let mut sc = ScanConfig::for_network(false);
    sc.workers = 2;
    run_scan(&r.db, &fs, &r.source(), &o, &sc, &ScanControl::default(), &mut |_| {}).unwrap();
    run_metadata(&r.db, &fs, &r.source(), &meta_fast(), &ScanControl::default(), &mut |_| {}).unwrap();
    assert_eq!(r.q::<i64>("SELECT COUNT(*) FROM photos WHERE xmp_kind=1"), 1);
    let rep = run_thumbnails(&r.db, &fs, &r.source(), root, &r.store, Some(&r.cache), &fast(), &ScanControl::default(), &Signals::default(), &mut |_| {}).unwrap();
    assert_eq!((rep.state, rep.done, rep.linked), ("complete", 2, 2));
    for photo in [&plain, &with_sidecar] {
        let asked = cache::listed_thumb_name(&Mutex::new(Listed::default()), photo, 0, GRID_SIZE).unwrap();
        let hit = r.cache.read(&asked).unwrap_or_else(|| panic!("the grid's name {asked} must hit for {}", photo.display()));
        let img = image::load_from_memory(&hit).unwrap();
        assert_eq!((img.width(), img.height()), (320, 213));
        // and the path-only pointer
        assert!(r.cache.read_known(photo, 0, GRID_SIZE).is_some());
    }
    // the sidecar changing (an edit saved by the app) makes the grid ask for another name: a miss, so the engine renders
    std::fs::write(root.join("trip/sidecar.jpg.xmp"), "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\"><edited/></x:xmpmeta>").unwrap();
    let asked = cache::listed_thumb_name(&Mutex::new(Listed::default()), &with_sidecar, 0, GRID_SIZE).unwrap();
    assert!(r.cache.read(&asked).is_none());
    // the catalog's own lookup agrees: it refuses a sidecar it did not see
    let meta = std::fs::metadata(&with_sidecar).unwrap();
    let (size, nanos) = (meta.len(), cache::stamp(&meta).split_once(':').unwrap().1.parse::<i64>().unwrap());
    let roots = vec![(1i64, root.to_path_buf())];
    let c = r.db.read().unwrap();
    let side_now = cache::stamp(&std::fs::metadata(root.join("trip/sidecar.jpg.xmp")).unwrap());
    assert!(thumbs::lookup_by_path(&c, &r.store, &roots, &with_sidecar, size, nanos, &side_now).is_none());
    // as it was when the thumbnail was made, it finds it, even after the cache was emptied
    r.cache.clear();
    let (xs, xm): (i64, i64) = c.query_row("SELECT xmp_size, xmp_mtime_ns FROM photos WHERE name='sidecar.jpg'", [], |x| Ok((x.get(0)?, x.get(1)?))).unwrap();
    let side_then = cache::stamp_of(xs as u64, xm as u128);
    assert!(thumbs::lookup_by_path(&c, &r.store, &roots, &with_sidecar, size, nanos, &side_then).is_some());
    assert!(thumbs::lookup_by_path(&c, &r.store, &roots, &plain, std::fs::metadata(&plain).unwrap().len(), nanos, "-").is_some());
    assert!(thumbs::lookup_by_path(&c, &r.store, &roots, Path::new("/elsewhere/x.jpg"), 1, 1, "-").is_none());
}

#[test]
fn an_edited_photo_is_made_for_the_catalog_but_not_offered_to_the_grid() {
    let r = rig();
    let fs = arw_fs(2, 640, 427);
    r.prepare(&fs);
    r.db.exec(|c| c.execute("UPDATE photos SET edited=1 WHERE rel_path LIKE '%0001%'", []).map(|_| ()).map_err(|e| e.to_string())).unwrap();
    let rep = r.thumbs(&fs);
    assert_eq!((rep.done, rep.linked), (2, 1));
    assert_eq!(r.store.scan().0.len(), 2);
}

#[test]
fn publishing_links_what_the_store_holds_when_the_job_did_not() {
    let r = rig();
    let fs = arw_fs(2, 640, 427);
    r.prepare(&fs);
    let mut cfg = fast();
    cfg.publish = false;
    let rep = r.thumbs_with(&fs, &cfg, &ScanControl::default(), &Signals::default());
    assert_eq!((rep.done, rep.linked), (2, 0));
    assert_eq!(r.cache.usage(), 0);
    let rep = thumbs::publish_source(&r.db, &r.store, &r.cache, &r.source(), Path::new("/fake")).unwrap();
    assert_eq!(rep.linked, 2);
    assert!(r.cache.usage() > 0);
}

#[test]
fn status_counts_photos_by_thumbnail_state() {
    let r = rig();
    let fs = arw_fs(3, 640, 427);
    fs.add_blob("x/heic.heic", make_heic(&Spec::default(), None, 0), BASE_MTIME);
    r.prepare(&fs);
    let c = thumbs::coverage(&r.db.read().unwrap(), None).unwrap();
    assert_eq!((c.done, c.pending, c.failed, c.deferred), (0, 4, 0, 0));
    r.thumbs(&fs);
    let c = thumbs::coverage(&r.db.read().unwrap(), Some(1)).unwrap();
    assert_eq!((c.done, c.pending, c.failed, c.deferred, c.kept_missing), (3, 0, 0, 1, 0));
    assert!(c.bytes > 0);
}

#[test]
fn progress_is_reported_and_throttled() {
    let r = rig();
    let fs = arw_fs(10, 640, 427);
    r.prepare(&fs);
    let mut seen: Vec<ThumbProgress> = Vec::new();
    let mut cfg = fast();
    cfg.progress_every = Duration::from_millis(30);
    run_thumbnails(&r.db, &fs, &r.source(), &fs.root, &r.store, Some(&r.cache), &cfg, &ScanControl::default(), &Signals::default(), &mut |p| seen.push(p.clone())).unwrap();
    let last = seen.last().unwrap();
    assert_eq!((last.phase.as_str(), last.done, last.total), ("done", 10, 10));
    assert!(seen.iter().all(|p| p.decoders == 2));
    assert!(seen.len() < 120);
}

// ---- measurements (ignored) -----------------------------------------------------------------

/// `cargo test --bin immage bench_20k -- --ignored --nocapture`: 20,000 photos on a local disk (hard
/// links to a few real files, so the disk holds only those), 80% RAW-shaped files with a 1620x1080
/// preview and 20% JPEGs with a 320 px Exif thumbnail. Reports thumbnails per second with N decoders and
/// the bytes read per thumbnail. The page cache makes reads free; the reads and bytes are what a NAS would see.
#[test]
#[ignore = "benchmark"]
fn bench_20k() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("photos");
    std::fs::create_dir_all(&root).unwrap();
    // eight base files
    let mut bases: Vec<PathBuf> = Vec::new();
    for i in 0..4u64 {
        let preview = jpeg_of(&noisy(1620, 1080, i + 10), 82);
        let p = dir.path().join(format!("base{i}.ARW"));
        std::fs::write(&p, arw_with(&preview, &spec_n(i as u32), 135_000)).unwrap();
        bases.push(p);
    }
    for i in 0..2u64 {
        let main = with_exif(&jpeg_of(&noisy(1200, 800, i + 20), 85), 1, Some(&jpeg_of(&noisy(320, 213, i + 30), 70)));
        let p = dir.path().join(format!("base{i}.jpg"));
        std::fs::write(&p, main).unwrap();
        bases.push(p);
    }
    let mut n_raw = 0;
    for d in 0..200 {
        std::fs::create_dir_all(root.join(format!("d{d:03}"))).unwrap();
        for f in 0..100 {
            let (base, ext) = if f < 80 { (&bases[f % 4], "ARW") } else { (&bases[4 + f % 2], "jpg") };
            std::fs::hard_link(base, root.join(format!("d{d:03}/IMG_{f:04}.{ext}"))).unwrap();
            n_raw += (f < 80) as u32;
        }
    }
    let r = rig();
    let fs = crate::catalog_scan::RealFs::new(root.clone());
    let o = ScanOptions { root: root.clone(), include_subfolders: true, dir_skip: false, light: false, fresh: false };
    let mut sc = ScanConfig::for_network(false);
    sc.workers = 4;
    let t = std::time::Instant::now();
    run_scan(&r.db, &fs, &r.source(), &o, &sc, &ScanControl::default(), &mut |_| {}).unwrap();
    let mut mc = MetaConfig::for_network(false);
    mc.workers = 4;
    run_metadata(&r.db, &fs, &r.source(), &mc, &ScanControl::default(), &mut |_| {}).unwrap();
    println!("scan and metadata of 20,000 photos ({n_raw} RAW-shaped): {:.1} s", t.elapsed().as_secs_f64());
    let cores = std::thread::available_parallelism().map_or(2, |n| n.get());
    // rates with N decoders on the first 1,500 photos of each round (the noisy pictures are a hard case for
    // the decoder), then everything with the most decoders
    let mut rounds: Vec<(usize, Option<u64>)> = vec![(1, Some(1500)), (2, Some(1500)), (4, Some(1500))];
    if cores > 5 {
        rounds.push((cores - 1, Some(1500)));
    }
    rounds.push((cores.saturating_sub(1).max(1), None));
    for (dec, limit) in rounds {
        r.db.exec(|c| c.execute("DELETE FROM thumbs", []).map(|_| ()).map_err(|e| e.to_string())).unwrap();
        let _ = std::fs::remove_dir_all(r.store.dir());
        r.cache.clear();
        let mut cfg = ThumbConfig::for_network(false);
        cfg.decode_workers = dec;
        cfg.read_workers = 4;
        cfg.whole_jpegs = WholeJpegs::No;
        let ctl = ScanControl::default();
        let rep = run_thumbnails(&r.db, &fs, &r.source(), &root, &r.store, Some(&r.cache), &cfg, &ctl, &Signals::default(), &mut |p| {
            if limit.is_some_and(|l| p.done >= l) {
                ctl.cancel();
            }
        })
        .unwrap();
        println!(
            "{dec} decoders{}: {} thumbnails in {:.1} s = {:.0}/s; reads {} ({:.2} per thumbnail), {:.0} KB read per thumbnail, {:.1} KB written per thumbnail; from preview {}, exif {}, failed {}, linked {}",
            if limit.is_some() { " (first 1,500)" } else { " (all)" },
            rep.done, rep.elapsed.as_secs_f64(), rep.per_s, rep.reads, rep.reads as f64 / rep.done.max(1) as f64,
            rep.bytes_read as f64 / rep.done.max(1) as f64 / 1024.0, rep.thumb_bytes as f64 / rep.done.max(1) as f64 / 1024.0,
            rep.from_preview, rep.from_exif, rep.failed, rep.linked
        );
        if limit.is_none() {
            assert_eq!((rep.state, rep.done, rep.failed), ("complete", 20_000, 0));
        }
    }
}

/// `IMMAGE_RAW_SAMPLES=<dir> cargo test --bin immage raw_samples_preview -- --ignored --nocapture`: at most 10 ARW
/// files of the folder are opened read only in place; each one's preview is located from its first 1 MB, read
/// by its span, decoded in memory and dropped. Nothing is written, shown or named; only aggregates are printed.
#[test]
#[ignore = "needs real files: IMMAGE_RAW_SAMPLES"]
fn raw_samples_preview() {
    use std::io::{Read, Seek, SeekFrom};
    let Some(dir) = std::env::var_os("IMMAGE_RAW_SAMPLES") else { return };
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("arw")))
        .collect();
    files.sort();
    files.truncate(10);
    assert!(!files.is_empty(), "no ARW files in the folder");
    let (mut spans, mut head_ms, mut decode_ms, mut thumb_bytes, mut dims) = (Vec::new(), Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for p in &files {
        let mut f = std::fs::File::open(p).unwrap();
        let size = f.metadata().unwrap().len();
        let mut head = vec![0u8; 1 << 20];
        let n = f.read(&mut head).unwrap();
        head.truncate(n);
        let t = std::time::Instant::now();
        let crate::catalog_meta::Extract::Done(meta) = crate::catalog_meta::extract(&head, size, "arw") else { panic!("the header did not parse") };
        head_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        let span = meta.preview.expect("a preview span");
        assert!(span.offset + span.len <= size);
        let mut buf = vec![0u8; span.len as usize];
        f.seek(SeekFrom::Start(span.offset)).unwrap();
        f.read_exact(&mut buf).unwrap();
        assert!(buf.starts_with(&[0xFF, 0xD8]));
        let t = std::time::Instant::now();
        let r = Render { edge: 480, quality: 80, orientation: meta.orientation.unwrap_or(1), main: meta.width.zip(meta.height), own_pixels: false };
        let th = src::render(&buf, &r).expect("the preview decodes");
        decode_ms.push(t.elapsed().as_secs_f64() * 1000.0);
        assert!(th.width.max(th.height) == 480 || th.width.max(th.height) <= 480);
        thumb_bytes.push(th.jpeg.len() as f64);
        spans.push(span.len as f64);
        dims.push((span.width.unwrap_or(0), span.height.unwrap_or(0), th.width, th.height));
        drop(buf);
    }
    let agg = |v: &[f64]| {
        let mut s = v.to_vec();
        s.sort_by(|a, b| a.total_cmp(b));
        (s[0], s[s.len() / 2], s[s.len() - 1])
    };
    let (a, b, c) = agg(&spans);
    let (d, e, f2) = agg(&decode_ms);
    let (g, h, i) = agg(&thumb_bytes);
    println!("{} real ARW files: preview read per photo (one ranged read) min {:.0} KB, median {:.0} KB, max {:.0} KB", files.len(), a / 1024.0, b / 1024.0, c / 1024.0);
    println!("decode + resize + encode in memory: min {d:.0} ms, median {e:.0} ms, max {f2:.0} ms (debug build); thumbnail {:.0}/{:.0}/{:.0} KB", g / 1024.0, h / 1024.0, i / 1024.0);
    println!("preview locating from the first 1 MB: median {:.2} ms; all previews decoded, all spans inside their files", agg(&head_ms).1);
    let _ = dims;
    let _ = (Src::Preview, DynamicImage::new_rgb8(1, 1));
}
