// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

use super::*;
use crate::catalog_meta_fixtures::*;

fn done(e: Extract) -> Meta {
    match e {
        Extract::Done(m) => *m,
        other => panic!("expected Done, got {other:?}"),
    }
}

fn spec() -> Spec {
    Spec::default()
}

/// 2025-10-05 14:08:39 as the epoch seconds of that wall time read as UTC.
const TAKEN: i64 = 1_759_673_319;

fn check_common(m: &Meta) {
    assert_eq!(m.make.as_deref(), Some("SONY"));
    assert_eq!(m.model.as_deref(), Some("ILCE-7RM3"));
    assert_eq!(m.body_serial.as_deref(), Some("1234567"));
    assert_eq!(m.lens.as_deref(), Some("FE 85mm F1.8"));
    assert_eq!(m.focal, Some(85.0));
    assert_eq!(m.focal35, Some(85.0));
    assert_eq!(m.aperture, Some(1.8));
    assert_eq!(m.iso, Some(800));
    assert_eq!(m.exposure, Some(0.004));
    assert_eq!(m.bias, Some(-0.7));
    assert_eq!(m.taken_at, Some(TAKEN));
    assert_eq!(m.subsec_ms, Some(120));
    assert_eq!(m.tz_offset, Some(-8 * 3600));
    assert_eq!(m.orientation, Some(6));
    let g = m.gps.as_ref().expect("gps");
    assert!((g.lat - (37.0 + 46.0 / 60.0 + 30.0 / 3600.0)).abs() < 1e-9);
    assert!((g.lon + (122.0 + 25.0 / 60.0 + 9.0 / 3600.0)).abs() < 1e-9);
    assert_eq!(g.alt, Some(123.4));
    assert_eq!(g.lat_ref.as_deref(), Some("N"));
    assert_eq!(g.lon_ref.as_deref(), Some("W"));
}

#[test]
fn dates_offsets_and_subseconds() {
    use crate::catalog_meta_tiff::{days_from_civil, parse_datetime, parse_offset, parse_subsec};
    assert_eq!(days_from_civil(1970, 1, 1), 0);
    assert_eq!(days_from_civil(2000, 3, 1), 11_017);
    assert_eq!(parse_datetime("2025:10:05 14:08:39"), Some(TAKEN));
    assert_eq!(parse_datetime("2025-10-05T14:08:39"), Some(TAKEN));
    assert_eq!(parse_datetime("0000:00:00 00:00:00"), None);
    assert_eq!(parse_datetime("2025:13:05 14:08:39"), None);
    assert_eq!(parse_datetime("    :  :     :  :  "), None);
    assert_eq!(parse_datetime("2025:10:05"), None);
    assert_eq!(parse_offset("-08:00"), Some(-28_800));
    assert_eq!(parse_offset("+05:30"), Some(19_800));
    assert_eq!(parse_offset("Z"), Some(0));
    assert_eq!(parse_offset("+0530"), Some(19_800));
    assert_eq!(parse_offset("+99:00"), None);
    assert_eq!(parse_offset("garbage"), None);
    assert_eq!(parse_subsec("12"), Some(120));
    assert_eq!(parse_subsec("123456"), Some(123));
    assert_eq!(parse_subsec("5"), Some(500));
    assert_eq!(parse_subsec("x"), None);
}

#[test]
fn arw_shaped_file_little_endian() {
    let bytes = arw_like(true, &spec(), 135_330, 396_841, 600_000);
    let m = done(extract(&bytes[..65_536], bytes.len() as u64, "ARW"));
    check_common(&m);
    assert_eq!(m.container, "raw-tiff");
    assert!(m.is_raw);
    assert_eq!((m.width, m.height), (Some(7952), Some(5304)));
    assert_eq!((m.raw_width, m.raw_height), (Some(8000), Some(5320)));
    let p = m.preview.expect("preview");
    assert_eq!((p.offset, p.len), (135_330, 396_841));
    assert_eq!(m.software.as_deref(), Some("ILCE-7RM3 v3.10"));
}

#[test]
fn big_endian_raw_with_the_same_content() {
    let bytes = arw_like(false, &spec(), 70_000, 100_000, 300_000);
    let m = done(extract(&bytes[..65_536], bytes.len() as u64, "nef"));
    check_common(&m);
    assert_eq!(m.preview.map(|p| (p.offset, p.len)), Some((70_000, 100_000)));
}

#[test]
fn preview_that_does_not_fit_the_file_is_dropped() {
    let bytes = arw_like(true, &spec(), 290_000, 100_000, 300_000);
    let m = done(extract(&bytes, bytes.len() as u64, "arw"));
    assert!(m.preview.is_none() || m.preview.unwrap().offset != 290_000);
}

#[test]
fn dng_with_subifd_preview_and_raw() {
    let s = spec();
    let ifds = vec![
        Ifd {
            entries: vec![
                (0x00FE, V::Long(vec![1])),
                (0x0100, V::Long(vec![256])),
                (0x0101, V::Long(vec![171])),
                (0x0103, V::Short(vec![7])),
                (0x0111, V::Long(vec![5000])),
                (0x0117, V::Long(vec![3000])),
                (0x010F, ascii("Leica Camera AG")),
                (0x0110, ascii("LEICA M10")),
                (0x0112, V::Short(vec![1])),
                (0xC612, V::Byte(vec![1, 4, 0, 0])),
                (0xC62F, ascii("4711")),
                (0x014A, V::Ptrs(vec![1, 2])),
                (0x8769, V::Ptr(3)),
            ],
            next: None,
        },
        Ifd {
            entries: vec![(0x00FE, V::Long(vec![1])), (0x0100, V::Long(vec![1600])), (0x0101, V::Long(vec![1067])), (0x0103, V::Short(vec![7])), (0x0111, V::Long(vec![9000])), (0x0117, V::Long(vec![20000]))],
            next: None,
        },
        Ifd { entries: vec![(0x00FE, V::Long(vec![0])), (0x0100, V::Long(vec![5976])), (0x0101, V::Long(vec![3992])), (0x0103, V::Short(vec![7])), (0x0106, V::Short(vec![32803]))], next: None },
        Ifd { entries: exif_entries(&Spec { serial: None, w: 5976, h: 3992, ..s }), next: None },
    ];
    let mut bytes = tiff(true, 42, &ifds);
    bytes.resize(40_000, 0);
    let m = done(extract(&bytes, bytes.len() as u64, "dng"));
    assert_eq!(m.make.as_deref(), Some("Leica Camera AG"));
    assert_eq!(m.body_serial.as_deref(), Some("4711"));
    assert_eq!((m.raw_width, m.raw_height), (Some(5976), Some(3992)));
    // the larger of the two JPEG previews wins
    let p = m.preview.unwrap();
    assert_eq!((p.offset, p.len, p.width), (9000, 20000, Some(1600)));
}

#[test]
fn orf_and_rw2_headers_are_accepted() {
    for magic in [0x4F52u16, 0x5352, 0x55] {
        let mut b = tiff(true, magic, &[Ifd { entries: vec![(0x010F, ascii("OLYMPUS")), (0x0110, ascii("E-M1"))], next: None }]);
        b.resize(2000, 0);
        let m = done(extract(&b, 2000, "orf"));
        assert_eq!(m.model.as_deref(), Some("E-M1"), "{magic:#x}");
    }
}

#[test]
fn jpeg_exif_xmp_and_frame_size() {
    let b = make_jpeg(Some(&spec()), Some(DARKTABLE_XMP), 4000, 3000, 0);
    let m = done(extract(&b, b.len() as u64, "jpg"));
    check_common(&m);
    assert_eq!(m.container, "jpeg");
    assert!(!m.is_raw);
    assert_eq!((m.width, m.height), (Some(4000), Some(3000)));
    assert!(m.preview.is_none());
    let x = m.xmp.expect("xmp");
    assert_eq!(x.rating, Some(4));
}

#[test]
fn jpeg_without_exif_still_gives_a_size() {
    let b = make_jpeg(None, None, 640, 480, 0);
    let m = done(extract(&b, b.len() as u64, "jpeg"));
    assert_eq!((m.width, m.height), (Some(640), Some(480)));
    assert_eq!(m.make, None);
}

#[test]
fn jpeg_with_a_long_gap_before_the_frame_header_asks_for_more() {
    let b = make_jpeg(None, None, 640, 480, 100_000);
    match extract(&b[..65_536], b.len() as u64, "jpg") {
        Extract::NeedMore { bytes, .. } => assert!(bytes > 65_536 && bytes <= 262_144, "{bytes}"),
        other => panic!("{other:?}"),
    }
    let m = done(extract(&b[..262_144.min(b.len())], b.len() as u64, "jpg"));
    assert_eq!(m.width, Some(640));
}

#[test]
fn tiff_that_points_past_the_window_asks_for_exactly_enough() {
    // the Exif IFD sits far into a large file
    // a filler value in IFD0 pushes the Exif IFD (laid out after it) to about 100 KB
    let ifds = vec![
        Ifd { entries: vec![(0x010F, ascii("NIKON")), (0x0110, ascii("Z 9")), (0x8769, V::Ptr(2)), (0xC4A5, V::Undef(vec![7; 100_000]))], next: None },
        Ifd { entries: vec![], next: None },
        Ifd { entries: exif_entries(&spec()), next: None },
    ];
    let mut far = tiff(true, 42, &ifds);
    far.resize(far.len() + 1000, 0);
    let size = far.len() as u64;
    match extract(&far[..65_536], size, "nef") {
        Extract::NeedMore { bytes, partial } => {
            assert!(bytes > 100_000 && bytes <= size, "{bytes}");
            assert_eq!(partial.make.as_deref(), Some("NIKON"));
            // the next step of the ladder covers it
            let next = HEAD_LADDER.iter().copied().find(|n| *n as u64 >= bytes).unwrap();
            let m = done(extract(&far[..next.min(far.len())], size, "nef"));
            assert_eq!(m.iso, Some(800));
        }
        other => panic!("{other:?}"),
    }
    // the whole file in the window never asks again
    assert_eq!(done(extract(&far, size, "nef")).iso, Some(800));
}

#[test]
fn heic_exif_xmp_and_size() {
    let b = make_heic(&spec(), Some(DARKTABLE_XMP), 0);
    let m = done(extract(&b, b.len() as u64, "heic"));
    check_common(&m);
    assert_eq!(m.container, "heif");
    assert!(!m.is_raw);
    assert_eq!((m.width, m.height), (Some(7952), Some(5304)));
    assert_eq!(m.xmp.unwrap().rating, Some(4));
}

#[test]
fn heic_with_the_exif_item_far_into_the_file_asks_for_more() {
    let b = make_heic(&spec(), None, 100_000);
    match extract(&b[..65_536], b.len() as u64, "heic") {
        Extract::NeedMore { bytes, .. } => {
            assert!(bytes > 100_000);
            let m = done(extract(&b[..bytes as usize], b.len() as u64, "heic"));
            check_common(&m);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn cr3_cmt_boxes_preview_and_raw_size() {
    let (b, jpeg_at) = make_cr3(&spec(), 5000);
    let m = done(extract(&b, b.len() as u64, "cr3"));
    check_common(&m);
    assert_eq!(m.container, "cr3");
    assert!(m.is_raw);
    assert_eq!((m.raw_width, m.raw_height), (Some(7952), Some(5304)));
    let p = m.preview.expect("PRVW");
    assert_eq!((p.offset, p.len), (jpeg_at, 5000));
    assert_eq!((p.width, p.height), (Some(1620), Some(1080)));
}

#[test]
fn raf_reads_the_embedded_jpeg_and_locates_it() {
    let b = make_raf(&spec(), 0, 50_000);
    let m = done(extract(&b, b.len() as u64, "raf"));
    assert_eq!(m.container, "raf");
    assert!(m.is_raw);
    assert_eq!(m.iso, Some(800));
    assert_eq!(m.make.as_deref(), Some("SONY"), "the embedded Exif wins over the header's name");
    let p = m.preview.unwrap();
    assert_eq!(p.offset, 148);
    assert_eq!((p.width, p.height), (Some(6240), Some(4160)));
    assert!(p.len > 100 && p.offset + p.len <= b.len() as u64);
}

#[test]
fn raf_with_a_damaged_exif_keeps_the_header_name() {
    let mut b = make_raf(&spec(), 0, 100);
    for x in b[160..260].iter_mut() {
        *x = 0xAA;
    }
    let m = done(extract(&b, b.len() as u64, "raf"));
    assert_eq!(m.model.as_deref(), Some("X-T4"));
}

#[test]
fn mp4_creation_time_and_size() {
    // 2025-10-05 14:08:39 UTC in seconds since 1904
    let created = (TAKEN + 2_082_844_800) as u32;
    let b = make_mp4(created, 3840, 2160);
    let m = done(extract(&b, b.len() as u64, "mp4"));
    assert_eq!(m.container, "video");
    assert_eq!(m.taken_at, Some(TAKEN));
    assert_eq!(m.tz_offset, Some(0));
    assert_eq!((m.width, m.height), (Some(3840), Some(2160)));
}

#[test]
fn unknown_formats_are_unsupported_and_empty_ones_corrupt() {
    assert_eq!(extract(b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR....", 5000, "png"), Extract::Unsupported);
    assert_eq!(extract(b"RIFF....WEBPVP8 ", 5000, "webp"), Extract::Unsupported);
    assert!(matches!(extract(b"tiny", 4, "jpg"), Extract::Corrupt(_)));
    // a TIFF whose IFD0 is out of the file
    let mut b = vec![0u8; 100];
    b[..8].copy_from_slice(&[b'I', b'I', 42, 0, 0xFF, 0xFF, 0xFF, 0x7F]);
    assert!(matches!(extract(&b, 100, "tif"), Extract::Corrupt(_)));
}

// ---- robustness ----------------------------------------------------------------------

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

fn corpus() -> Vec<(Vec<u8>, &'static str)> {
    let s = spec();
    vec![
        (arw_like(true, &s, 5000, 3000, 20_000), "arw"),
        (arw_like(false, &s, 5000, 3000, 20_000), "nef"),
        (make_jpeg(Some(&s), Some(DARKTABLE_XMP), 800, 600, 0), "jpg"),
        (make_heic(&s, Some(DARKTABLE_XMP), 0), "heic"),
        (make_cr3(&s, 300).0, "cr3"),
        (make_raf(&s, 0, 100), "raf"),
        (make_mp4(3_800_000_000, 1920, 1080), "mp4"),
    ]
}

#[test]
fn every_truncation_of_every_format_is_safe() {
    for (bytes, ext) in corpus() {
        for n in 0..bytes.len().min(4000) {
            // a window shorter than the file may ask for more but never panics
            let _ = extract(&bytes[..n], bytes.len() as u64, ext);
            let _ = extract(&bytes[..n], n as u64, ext);
        }
        for n in (4000..bytes.len()).step_by(97) {
            let _ = extract(&bytes[..n], bytes.len() as u64, ext);
        }
    }
}

#[test]
fn random_corruption_never_panics_or_asks_for_absurd_bytes() {
    let mut rng = Rng(0x9E3779B97F4A7C15);
    for (bytes, ext) in corpus() {
        for _ in 0..3000 {
            let mut b = bytes.clone();
            for _ in 0..1 + rng.next() % 8 {
                let at = (rng.next() % b.len() as u64) as usize;
                match rng.next() % 4 {
                    0 => b[at] = rng.next() as u8,
                    1 => b[at] = 0xFF,
                    2 => b[at] = 0,
                    _ => {
                        let end = (at + 4).min(b.len());
                        b[at..end].fill(0xFF);
                    }
                }
            }
            if let Extract::NeedMore { bytes: n, .. } = extract(&b, b.len() as u64, ext) {
                assert!(n <= b.len() as u64, "asked past the end of the file: {n}");
            }
        }
    }
}

#[test]
fn ifd_loops_absurd_counts_and_wild_offsets_are_cut() {
    // IFD0 whose next pointer is itself, and a SubIFD list pointing back at IFD0
    let mut ifds = vec![
        Ifd { entries: vec![(0x010F, ascii("LOOP")), (0x014A, V::Ptrs(vec![0, 0, 0])), (0x8769, V::Ptr(0)), (0x8825, V::Ptr(0))], next: Some(0) },
    ];
    let b = tiff(true, 42, &ifds);
    let m = done(extract(&b, b.len() as u64, "dng"));
    assert_eq!(m.make.as_deref(), Some("LOOP"));
    // an entry count that fills the whole file
    let mut c = tiff(true, 42, &[Ifd { entries: vec![(0x010F, ascii("X"))], next: None }]);
    c[8..10].copy_from_slice(&0xFFFFu16.to_le_bytes());
    assert!(matches!(extract(&c, c.len() as u64, "arw"), Extract::Corrupt(_)));
    // a string of absurd count, and a rational of absurd count
    ifds[0].entries = vec![(0x010F, V::Ascii("A".into()))];
    let mut d = tiff(true, 42, &ifds);
    // entry 0: tag 0x010F, type 2, count at +4
    let count_at = 8 + 2 + 4;
    d[count_at..count_at + 4].copy_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    let r = extract(&d, d.len() as u64, "arw");
    assert!(!matches!(r, Extract::NeedMore { .. }));
    // an Exif pointer to the last byte
    let mut e = tiff(true, 42, &[Ifd { entries: vec![(0x8769, V::Long(vec![0xFFFF_FFF0]))], next: None }]);
    e.resize(64, 0);
    assert!(matches!(extract(&e, 64, "arw"), Extract::Corrupt(_)));
}

#[test]
fn deep_box_nesting_is_cut() {
    let mut inner = vec![0u8; 8];
    inner[3] = 8;
    inner[4..8].copy_from_slice(b"free");
    for _ in 0..200 {
        let mut b = ((inner.len() + 8) as u32).to_be_bytes().to_vec();
        b.extend_from_slice(b"trak");
        b.extend_from_slice(&inner);
        inner = b;
    }
    let mut f = vec![0, 0, 0, 12];
    f.extend_from_slice(b"ftypisom");
    let mut moov = ((inner.len() + 8) as u32).to_be_bytes().to_vec();
    moov.extend_from_slice(b"moov");
    f.extend_from_slice(&moov);
    f.extend_from_slice(&inner);
    let _ = extract(&f, f.len() as u64, "mp4");
}

#[test]
fn a_window_that_covers_the_file_never_asks_for_more() {
    for (bytes, ext) in corpus() {
        assert!(!matches!(extract(&bytes, bytes.len() as u64, ext), Extract::NeedMore { .. }), "{ext}");
    }
}

// ---- XMP --------------------------------------------------------------------------------

#[test]
fn darktable_sidecar() {
    let x = crate::catalog_meta_xmp::parse(DARKTABLE_XMP.as_bytes()).unwrap();
    assert_eq!(x.rating, Some(4));
    assert_eq!(x.labels, Some(1 | 4));
    assert_eq!(x.keywords, vec!["portrait".to_string(), "Alice & Bob".to_string()]);
    assert_eq!(x.history_end, Some(3));
    assert_eq!(x.edited, Some(true));
}

#[test]
fn darktable_default_history_is_not_an_edit() {
    let xml = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="r"><rdf:Description xmlns:darktable="d" darktable:history_end="2">
        <darktable:history><rdf:Seq>
          <rdf:li darktable:num="0" darktable:operation="rawprepare" darktable:enabled="1"/>
          <rdf:li darktable:num="1" darktable:operation="demosaic" darktable:enabled="1"/>
          <rdf:li darktable:num="2" darktable:operation="vignette" darktable:enabled="1"/>
        </rdf:Seq></darktable:history></rdf:Description></rdf:RDF></x:xmpmeta>"#;
    let x = crate::catalog_meta_xmp::parse(xml.as_bytes()).unwrap();
    // vignette is past history_end (not applied)
    assert_eq!(x.edited, Some(false));
    let none = xml.replace("history_end=\"2\"", "history_end=\"0\"");
    assert_eq!(crate::catalog_meta_xmp::parse(none.as_bytes()).unwrap().edited, Some(false));
}

#[test]
fn adobe_sidecar_with_element_syntax_and_reject() {
    let xml = r#"<?xml version="1.0"?><x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="r">
      <rdf:Description xmlns:xmp="a" xmlns:dc="b" xmlns:lr="c">
        <xmp:Rating>-1</xmp:Rating><xmp:Label>Blue</xmp:Label>
        <dc:subject><rdf:Bag><rdf:li>one</rdf:li><rdf:li>two</rdf:li><rdf:li>one</rdf:li></rdf:Bag></dc:subject>
        <lr:hierarchicalSubject><rdf:Bag><rdf:li>a|b</rdf:li></rdf:Bag></lr:hierarchicalSubject>
      </rdf:Description></rdf:RDF></x:xmpmeta>"#;
    let x = crate::catalog_meta_xmp::parse(xml.as_bytes()).unwrap();
    assert_eq!(x.rating, Some(-1));
    assert_eq!(x.labels, Some(8));
    assert_eq!(x.keywords, vec!["one".to_string(), "two".to_string()]);
    assert_eq!(x.history_end, None);
    assert_eq!(x.edited, None);
}

#[test]
fn xmp_is_tolerant_and_bounded() {
    assert!(crate::catalog_meta_xmp::parse(b"not xml at all").is_none());
    assert!(crate::catalog_meta_xmp::parse(b"<a><b></a>").is_none());
    assert!(crate::catalog_meta_xmp::parse(b"<rdf:Description xmp:Rating=\"3").is_none());
    let x = crate::catalog_meta_xmp::parse(b"<rdf:Description xmp:Rating='5' xmp:Label='Red'/>").unwrap();
    assert_eq!((x.rating, x.labels), (Some(5), Some(1)));
    // deep nesting, many keywords, and a size over the cap
    let deep = "<a>".repeat(10_000);
    let _ = crate::catalog_meta_xmp::parse(deep.as_bytes());
    let many: String = (0..5000).map(|i| format!("<dc:subject><rdf:li>k{i}</rdf:li></dc:subject>")).collect();
    let x = crate::catalog_meta_xmp::parse(many.as_bytes()).unwrap();
    assert_eq!(x.keywords.len(), 1000);
    let big = vec![b'<'; crate::catalog_meta_xmp::MAX_SIDECAR as usize + 1];
    assert!(crate::catalog_meta_xmp::parse(&big).is_none());
    let mut rng = Rng(7);
    for _ in 0..2000 {
        let mut b = DARKTABLE_XMP.as_bytes().to_vec();
        for _ in 0..6 {
            let at = (rng.next() % b.len() as u64) as usize;
            b[at] = rng.next() as u8;
        }
        let _ = crate::catalog_meta_xmp::parse(&b);
    }
}

#[test]
fn an_xmp_packet_inside_a_tiff_raw_is_read() {
    let mut ifd = vec![(0x010F, ascii("PENTAX")), (0x02BC, V::Byte(DARKTABLE_XMP.as_bytes().to_vec()))];
    ifd.push((0x0110, ascii("K-3")));
    let mut b = tiff(true, 42, &[Ifd { entries: ifd, next: None }]);
    b.resize(20_000, 0);
    let m = done(extract(&b, b.len() as u64, "pef"));
    assert_eq!(m.xmp.unwrap().rating, Some(4));
}

// ---- real files, local only --------------------------------------------------------------

fn hashed(label: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(label.as_bytes()).iter().take(3).map(|b| format!("{b:02x}")).collect()
}

fn pct(v: &[u64], p: f64) -> u64 {
    v[((v.len() - 1) as f64 * p).round() as usize]
}

/// `IMMAGE_RAW_SAMPLES=<dir with a few raw files>`: reads each file's first MB
/// into memory only (read only, in place), and asserts structural facts. Prints
/// the smallest prefix that parses to the same result (aggregates, hashed labels).
#[test]
#[ignore = "needs IMMAGE_RAW_SAMPLES"]
fn raw_samples_header_sizes() {
    let Ok(dir) = std::env::var("IMMAGE_RAW_SAMPLES") else { return };
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| is_raw_ext(&e.to_string_lossy())))
        .collect();
    files.sort();
    files.truncate(40);
    assert!(!files.is_empty(), "no raw files in the sample directory");
    let mut needed = Vec::new();
    let mut needed_raw = Vec::new();
    let mut tail = Vec::new();
    let mut parse_us = Vec::new();
    let mut ok64 = 0;
    let mut outliers = Vec::new();
    for p in &files {
        use std::io::Read;
        let size = std::fs::metadata(p).unwrap().len();
        let mut buf = Vec::new();
        std::fs::File::open(p).unwrap().take(1024 * 1024).read_to_end(&mut buf).unwrap();
        let t = std::time::Instant::now();
        let at64 = extract(&buf[..buf.len().min(65_536)], size, "arw");
        parse_us.push(t.elapsed().as_micros() as u64);
        if matches!(at64, Extract::Done(_)) {
            ok64 += 1;
        }
        let m = done(extract(&buf, size, "arw"));
        // structural facts only
        assert_eq!(m.container, "raw-tiff");
        assert!(m.make.is_some() && m.model.is_some() && m.taken_at.is_some() && m.iso.is_some());
        let pv = m.preview.expect("preview located");
        assert!(pv.len > 10_000 && pv.offset + pv.len <= size, "preview inside the file");
        // the true raw size sits in a SubIFD about 133 KB in: present only from a bigger window
        let mut core = m.clone();
        core.raw_width = None;
        core.raw_height = None;
        let same = |n: usize, with_raw: bool| {
            matches!(extract(&buf[..n.min(buf.len())], size, "arw"), Extract::Done(x) if {
                let mut x = x.clone();
                if !with_raw {
                    x.raw_width = None;
                    x.raw_height = None;
                }
                *x == if with_raw { m.clone() } else { core.clone() }
            })
        };
        let smallest = |with_raw: bool| {
            let (mut lo, mut hi) = (0usize, buf.len() / 256);
            assert!(same(hi * 256, with_raw));
            while lo < hi {
                let mid = (lo + hi) / 2;
                if same(mid * 256, with_raw) { hi = mid } else { lo = mid + 1 }
            }
            (hi * 256) as u64
        };
        let n_core = smallest(false);
        needed_raw.push(smallest(true));
        needed.push(n_core);
        tail.push(pv.offset);
        if n_core > 65_536 {
            outliers.push(hashed(&p.to_string_lossy()));
        }
    }
    needed.sort_unstable();
    needed_raw.sort_unstable();
    tail.sort_unstable();
    parse_us.sort_unstable();
    eprintln!("raw samples: {} files, parsed from a 64 KB window: {ok64}", files.len());
    eprintln!(
        "smallest prefix that parses (bytes): min {} median {} p95 {} max {}",
        needed[0], pct(&needed, 0.5), pct(&needed, 0.95), needed[needed.len() - 1]
    );
    eprintln!(
        "smallest prefix incl. the raw size (bytes): min {} median {} p95 {} max {}",
        needed_raw[0], pct(&needed_raw, 0.5), pct(&needed_raw, 0.95), needed_raw[needed_raw.len() - 1]
    );
    eprintln!("preview offset (bytes): min {} median {} max {}", tail[0], pct(&tail, 0.5), tail[tail.len() - 1]);
    eprintln!("parse cost from 64 KB (us): min {} median {} p95 {} max {}", parse_us[0], pct(&parse_us, 0.5), pct(&parse_us, 0.95), parse_us[parse_us.len() - 1]);
    eprintln!("files needing more than 64 KB: {:?}", outliers);
}

/// `IMMAGE_META_FIXTURES=<dir>`: parses files made by exiftool/Pillow and prints
/// one JSON line each for comparison with `exiftool -j`.
#[test]
#[ignore = "needs IMMAGE_META_FIXTURES"]
fn exiftool_made_files_dump() {
    let Ok(dir) = std::env::var("IMMAGE_META_FIXTURES") else { return };
    let mut names: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.path()).collect();
    names.sort();
    for p in names {
        let bytes = std::fs::read(&p).unwrap();
        let ext = p.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
        let r = extract(&bytes[..bytes.len().min(65_536)], bytes.len() as u64, &ext);
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        match r {
            Extract::Done(m) => {
                let g = m.gps.as_ref().map(|g| (g.lat, g.lon, g.alt));
                eprintln!(
                    "{}",
                    serde_json::json!({"file": name, "make": m.make, "model": m.model, "serial": m.body_serial, "lens": m.lens, "focal": m.focal, "focal35": m.focal35,
                        "f": m.aperture, "iso": m.iso, "exp": m.exposure, "bias": m.bias, "taken": m.taken_at, "sub": m.subsec_ms, "tz": m.tz_offset,
                        "orient": m.orientation, "w": m.width, "h": m.height, "rw": m.raw_width, "rh": m.raw_height, "gps": g,
                        "preview": m.preview.map(|p| (p.offset, p.len)), "software": m.software, "rating": m.xmp.as_ref().and_then(|x| x.rating)})
                );
            }
            other => eprintln!("{}", serde_json::json!({"file": name, "result": format!("{other:?}").chars().take(60).collect::<String>()})),
        }
    }
}
