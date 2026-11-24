// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! One small decoded picture of a photo for the background analyses, made without darktable's
//! pipeline: the JPEG itself, or the preview a RAW file carries inside it. A photo is decoded
//! once here and the result is shared (a few recent pictures are kept), so Auto-keywords, the
//! near-duplicate hash and any later analysis (faces, eyes) read the same pixels instead of
//! each asking the engine for a render.
//!
//! Smart search (CLIP) still uses the engine's render of the edited photo: its stored
//! embeddings were made from that and an unedited preview is not the same picture.
use image::{DynamicImage, RgbImage};
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::{Arc, Mutex};

/// The long edge of the picture handed to analyses.
pub const EDGE: u32 = 1024;
/// A preview shorter than this on its long edge is too small to tag from; the engine renders instead.
pub const MIN_EDGE: u32 = 640;
/// How much of a RAW file is searched for its preview (every preview sits in the first part).
const HEAD: u64 = 24 << 20;
/// A plain JPEG larger than this is left to the engine.
const MAX_JPEG: u64 = 96 << 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// The photo is a JPEG and was decoded itself.
    Jpeg,
    /// A RAW file's embedded preview JPEG.
    Preview,
}

pub struct Picture {
    pub rgb: RgbImage,
    pub source: Source,
}

fn rd16(b: &[u8], at: usize) -> Option<usize> {
    Some(((*b.get(at)? as usize) << 8) | *b.get(at + 1)? as usize)
}

/// Where the JPEG starting at `start` ends and its pixel size, by walking its marker segments
/// and then its entropy-coded data; None when it is cut short or not a JPEG.
fn jpeg_span(b: &[u8], start: usize) -> Option<(usize, u32, u32)> {
    if b.get(start..start + 3)? != [0xFF, 0xD8, 0xFF] {
        return None;
    }
    let mut at = start + 2;
    let mut size: Option<(u32, u32)> = None;
    loop {
        if *b.get(at)? != 0xFF {
            return None;
        }
        let m = *b.get(at + 1)?;
        match m {
            0xFF => {
                at += 1;
                continue;
            }
            0x01 | 0xD0..=0xD8 => {
                at += 2;
                continue;
            }
            0xD9 => return Some((at + 2, size?.0, size?.1)),
            _ => {}
        }
        let len = rd16(b, at + 2)?;
        if len < 2 {
            return None;
        }
        if matches!(m, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
            size = Some((rd16(b, at + 7)? as u32, rd16(b, at + 5)? as u32));
        }
        if m == 0xDA {
            // entropy-coded data: runs to the next marker that is not a stuffed zero or a restart
            let mut i = at + 2 + len;
            loop {
                let p = b.get(i..)?.iter().position(|&x| x == 0xFF)? + i;
                match *b.get(p + 1)? {
                    0x00 | 0xD0..=0xD7 => i = p + 2,
                    0xD9 => return Some((p + 2, size?.0, size?.1)),
                    _ => {
                        at = p;
                        break;
                    }
                }
            }
            continue;
        }
        at += 2 + len;
    }
}

/// The biggest complete JPEG inside `b` (a RAW file's head), as a byte range. Thumbnails and the
/// full-size preview both qualify; the one with the most pixels wins.
pub fn largest_embedded_jpeg(b: &[u8]) -> Option<(usize, usize)> {
    let mut best: Option<(u64, usize, usize)> = None;
    let mut i = 0;
    while i + 3 <= b.len() {
        match b[i..].windows(3).position(|w| w == [0xFF, 0xD8, 0xFF]) {
            None => break,
            Some(p) => i += p,
        }
        if let Some((end, w, h)) = jpeg_span(b, i) {
            let px = u64::from(w) * u64::from(h);
            if best.is_none_or(|(n, _, _)| px > n) {
                best = Some((px, i, end));
            }
            // a nested thumbnail lies inside this one's bytes only for malformed files: step past it
            i = end;
        } else {
            i += 3;
        }
    }
    best.map(|(_, s, e)| (s, e))
}

/// The EXIF orientation (1 to 8) of a JPEG, 1 when it has none.
pub fn orientation(jpeg: &[u8]) -> u8 {
    let find = || -> Option<u8> {
        let mut at = 2;
        while at + 4 <= jpeg.len() && jpeg[at] == 0xFF {
            let m = jpeg[at + 1];
            if m == 0xDA || m == 0xD9 {
                return None;
            }
            let len = rd16(jpeg, at + 2)?;
            if m == 0xE1 && jpeg.get(at + 4..at + 10) == Some(b"Exif\0\0") {
                let t = at + 10;
                let le = match jpeg.get(t..t + 2)? {
                    b"II" => true,
                    b"MM" => false,
                    _ => return None,
                };
                let r16 = |o: usize| -> Option<u16> {
                    let s = jpeg.get(t + o..t + o + 2)?;
                    Some(if le { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) })
                };
                let r32 = |o: usize| -> Option<usize> {
                    let s = jpeg.get(t + o..t + o + 4)?;
                    Some(if le { u32::from_le_bytes([s[0], s[1], s[2], s[3]]) } else { u32::from_be_bytes([s[0], s[1], s[2], s[3]]) } as usize)
                };
                let ifd = r32(4)?;
                for n in 0..usize::from(r16(ifd)?).min(64) {
                    let e = ifd + 2 + n * 12;
                    if r16(e)? == 0x0112 {
                        return u8::try_from(r16(e + 8)?).ok().filter(|o| (1..=8).contains(o));
                    }
                }
                return None;
            }
            at += 2 + len;
        }
        None
    };
    find().unwrap_or(1)
}

fn decode(jpeg: &[u8], edge: u32) -> Result<RgbImage, String> {
    let mut reader = image::ImageReader::with_format(std::io::Cursor::new(jpeg), image::ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(1 << 30);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| e.to_string())?;
    let img = if img.width().max(img.height()) > edge { img.thumbnail(edge, edge) } else { img };
    Ok(crate::catalog_thumb_src::orient(DynamicImage::from(img), orientation(jpeg)).to_rgb8())
}

fn is_jpeg_name(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "jpg" | "jpeg"))
}
fn is_raw_name(path: &Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        matches!(e.to_ascii_lowercase().as_str(),
            "arw" | "sr2" | "srf" | "nef" | "nrw" | "cr2" | "cr3" | "crw" | "raf" | "orf" | "rw2" | "pef" | "dng" | "srw" | "3fr" | "iiq" | "erf" | "mef" | "mrw" | "kdc" | "dcr" | "x3f" | "rwl")
    })
}

/// The photo's own small picture, or None when the engine should render it: a file type this
/// does not read, no usable preview, a preview shorter than `MIN_EDGE`, or a failed decode.
pub fn picture(path: &Path) -> Option<Picture> {
    picture_at(path, EDGE)
}

/// `picture` with another long edge (a face crop wants more pixels than an analysis).
pub fn picture_at(path: &Path, edge: u32) -> Option<Picture> {
    let size = std::fs::metadata(path).ok()?.len();
    let mut file = std::fs::File::open(path).ok()?;
    if is_jpeg_name(path) {
        if size > MAX_JPEG {
            return None;
        }
        let mut bytes = Vec::with_capacity(size as usize);
        file.read_to_end(&mut bytes).ok()?;
        let rgb = decode(&bytes, edge).ok()?;
        return Some(Picture { rgb, source: Source::Jpeg });
    }
    if !is_raw_name(path) {
        return None;
    }
    let mut head = Vec::new();
    file.seek(SeekFrom::Start(0)).ok()?;
    file.take(HEAD).read_to_end(&mut head).ok()?;
    let (s, e) = largest_embedded_jpeg(&head)?;
    let rgb = decode(&head[s..e], edge).ok()?;
    // the preview's own size, before the shrink to EDGE: a shrunk one is at least MIN_EDGE
    if rgb.width().max(rgb.height()) < MIN_EDGE {
        return None;
    }
    Some(Picture { rgb, source: Source::Preview })
}

/// A few recent pictures, shared between analyses: the second analysis of a photo reads this
/// instead of decoding it again. Keyed by path and file fingerprint, so a changed file misses.
pub struct Shared {
    items: Mutex<Vec<(String, String, Arc<RgbImage>, Source)>>,
}
const KEEP: usize = 4;
impl Shared {
    pub fn new() -> Self {
        Self { items: Mutex::new(Vec::new()) }
    }
    /// The picture of `path` for this file fingerprint, decoding it only the first time.
    pub fn get(&self, path: &str, fingerprint: &str) -> Option<(Arc<RgbImage>, Source)> {
        {
            let items = self.items.lock().ok()?;
            if let Some((_, _, img, src)) = items.iter().find(|(p, f, _, _)| p == path && f == fingerprint) {
                return Some((img.clone(), *src));
            }
        }
        let pic = picture(Path::new(path))?;
        let img = Arc::new(pic.rgb);
        let mut items = self.items.lock().ok()?;
        items.retain(|(p, _, _, _)| p != path);
        items.insert(0, (path.to_string(), fingerprint.to_string(), img.clone(), pic.source));
        items.truncate(KEEP);
        Some((img, pic.source))
    }
    /// Puts a picture the engine rendered in the shared set, so the next analysis of this photo reuses it.
    pub fn put(&self, path: &str, fingerprint: &str, img: Arc<RgbImage>) {
        if let Ok(mut items) = self.items.lock() {
            items.retain(|(p, _, _, _)| p != path);
            items.insert(0, (path.to_string(), fingerprint.to_string(), img, Source::Jpeg));
            items.truncate(KEEP);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn jpeg(w: u32, h: u32, shade: u8) -> Vec<u8> {
        let img = RgbImage::from_fn(w, h, |x, y| image::Rgb([(x * 255 / w) as u8, (y * 255 / h) as u8, shade]));
        let mut out = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85).encode(img.as_raw(), w, h, image::ExtendedColorType::Rgb8).unwrap();
        out
    }
    /// A RAW-shaped file: a TIFF header, junk without JPEG markers, a small thumbnail, more junk,
    /// the full-size preview, then sensor data.
    pub(crate) fn raw_like(preview: &[u8], thumb: &[u8]) -> Vec<u8> {
        let mut f = b"II*\0\x08\0\0\0".to_vec();
        f.extend(std::iter::repeat_n(0x11u8, 5000));
        f.extend_from_slice(thumb);
        f.extend(std::iter::repeat_n(0x22u8, 3000));
        f.extend_from_slice(preview);
        f.extend(std::iter::repeat_n(0x33u8, 20000));
        f
    }

    #[test]
    fn the_biggest_complete_jpeg_in_a_raw_like_file_is_found_exactly() {
        let thumb = jpeg(160, 120, 10);
        let preview = jpeg(1600, 1067, 200);
        let file = raw_like(&preview, &thumb);
        let (s, e) = largest_embedded_jpeg(&file).unwrap();
        assert_eq!(&file[s..e], &preview[..], "the full preview, byte for byte");
        let only_thumb = raw_like(&[], &thumb);
        let (s, e) = largest_embedded_jpeg(&only_thumb).unwrap();
        assert_eq!(&only_thumb[s..e], &thumb[..]);
        assert!(largest_embedded_jpeg(&[0x11; 4000]).is_none());
        // cut short in the middle of its data: not a complete jpeg
        assert!(largest_embedded_jpeg(&preview[..preview.len() / 2]).is_none());
    }

    #[test]
    fn a_raw_like_file_gives_its_preview_and_a_small_one_is_left_to_the_engine() {
        let dir = tempfile::tempdir().unwrap();
        let big = dir.path().join("a.arw");
        std::fs::write(&big, raw_like(&jpeg(1600, 1067, 120), &jpeg(160, 120, 10))).unwrap();
        let p = picture(&big).unwrap();
        assert_eq!(p.source, Source::Preview);
        assert_eq!(p.rgb.width().max(p.rgb.height()), EDGE);
        let small = dir.path().join("b.nef");
        std::fs::write(&small, raw_like(&jpeg(400, 300, 120), &jpeg(160, 120, 10))).unwrap();
        assert!(picture(&small).is_none(), "a 400 px preview is too small to tag from");
        let none = dir.path().join("c.cr2");
        std::fs::write(&none, vec![0x44u8; 9000]).unwrap();
        assert!(picture(&none).is_none());
        let other = dir.path().join("d.png");
        std::fs::write(&other, b"x").unwrap();
        assert!(picture(&other).is_none());
    }

    #[test]
    fn a_jpeg_is_decoded_to_the_analysis_size() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.JPG");
        std::fs::write(&p, jpeg(3000, 2000, 90)).unwrap();
        let pic = picture(&p).unwrap();
        assert_eq!(pic.source, Source::Jpeg);
        assert_eq!((pic.rgb.width(), pic.rgb.height()), (1024, 683));
    }

    #[test]
    fn orientation_is_read_from_exif_and_applied() {
        // an APP1 Exif segment with one IFD0 entry: orientation 6
        let mut exif = b"Exif\0\0II*\0\x08\0\0\0\x01\0".to_vec();
        exif.extend_from_slice(&[0x12, 0x01, 0x03, 0x00, 0x01, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0]);
        let base = jpeg(200, 100, 50);
        let mut with = vec![0xFF, 0xD8, 0xFF, 0xE1];
        with.extend_from_slice(&((exif.len() + 2) as u16).to_be_bytes());
        with.extend_from_slice(&exif);
        with.extend_from_slice(&base[2..]);
        assert_eq!(orientation(&with), 6);
        assert_eq!(orientation(&base), 1);
        let img = decode(&with, EDGE).unwrap();
        assert_eq!((img.width(), img.height()), (100, 200), "turned upright");
    }

    #[test]
    fn the_shared_set_decodes_a_photo_once_and_a_changed_file_misses() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.jpg");
        std::fs::write(&p, jpeg(800, 600, 90)).unwrap();
        let shared = Shared::new();
        let path = p.to_str().unwrap();
        let (a, _) = shared.get(path, "f1").unwrap();
        std::fs::remove_file(&p).unwrap();
        let (b, _) = shared.get(path, "f1").unwrap();
        assert!(Arc::ptr_eq(&a, &b), "the second reader gets the same pixels, with no file to read");
        assert!(shared.get(path, "f2").is_none(), "another fingerprint is another file");
    }

    /// Timing of the file path on RAW-like files with a small and a full-size embedded preview
    /// (cargo test preview_timing -- --ignored --nocapture); the engine's render of a real RAW is
    /// not measurable without a real RAW.
    #[test]
    #[ignore = "timing only"]
    fn preview_timing() {
        let dir = tempfile::tempdir().unwrap();
        for (name, w, h) in [("small.arw", 1616u32, 1080u32), ("full.arw", 6000, 4000)] {
            let p = dir.path().join(name);
            std::fs::write(&p, raw_like(&jpeg(w, h, 120), &jpeg(160, 120, 10))).unwrap();
            let t = std::time::Instant::now();
            let pic = picture(&p).unwrap();
            println!("{name}: {} KB file, preview {w}x{h} -> {}x{} in {:.0} ms", std::fs::metadata(&p).unwrap().len() / 1024, pic.rgb.width(), pic.rgb.height(), t.elapsed().as_secs_f64() * 1000.0);
        }
    }
}
