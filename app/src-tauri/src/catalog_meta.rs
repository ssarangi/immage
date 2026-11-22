// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! The catalog's metadata extractor: a read-only, bounded, format-sniffing
//! parser over the first bytes of a photo. It never touches the file system;
//! the caller reads a window (64 KB first) and this module either returns the
//! metadata or says how many bytes it would need ([`Extract::NeedMore`]), so the
//! caller can read 256 KB, then 1 MB, and no more than a file needs.
//!
//! Formats: JPEG (APP1 Exif and XMP, SOF size), TIFF and every TIFF-based RAW
//! (ARW, NEF, NRW, DNG, CR2, ORF, RW2, PEF, SRW, ERF, KDC, 3FR, IIQ, ...) through
//! [`crate::catalog_meta_tiff`]; ISO base media (HEIC, HEIF, AVIF, Canon CR3, MP4,
//! MOV) through [`crate::catalog_meta_iso`]; Fujifilm RAF (the Exif of its
//! embedded JPEG). Anything else is [`Extract::Unsupported`] and is left for the
//! engine's exiv2 fallback.
//!
//! No external library: kamadak-exif (BSD-2) would parse Exif but gives no "need
//! more bytes" signal on a truncated window, does not follow SubIFDs (where a
//! RAW's true size and preview live) and reads the whole container; nom-exif
//! needs the whole stream. Writing the few parsers kept the dependency tree
//! unchanged and the limits explicit. See notes/catalog-meta-notes.md.

use crate::catalog_meta_tiff::Tiff;
use crate::catalog_meta_xmp::{self, Xmp};

/// The ladder of read sizes: the baseline measured a 64 KB read at about 22 ms
/// over SMB and 256 KB at about 43 ms.
pub const HEAD_LADDER: [usize; 3] = [64 * 1024, 256 * 1024, 1024 * 1024];

/// Extensions of raw formats (the same set the library calls raw).
pub const RAW_EXTS: &[&str] = &[
    "3fr", "ari", "arw", "bay", "cr2", "cr3", "crw", "dcr", "dcs", "dng", "erf", "fff", "iiq", "k25", "kdc", "mdc", "mef", "mos",
    "mrw", "nef", "nrw", "orf", "pef", "raf", "raw", "rw2", "rwl", "sr2", "srf", "srw", "x3f",
];

pub fn is_raw_ext(ext: &str) -> bool {
    RAW_EXTS.iter().any(|e| e.eq_ignore_ascii_case(ext))
}

/// A byte range of the file (an embedded preview).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub offset: u64,
    pub len: u64,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Gps {
    pub lat: f64,
    pub lon: f64,
    pub alt: Option<f64>,
    pub lat_ref: Option<String>,
    pub lon_ref: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Meta {
    /// jpeg | tiff | raw-tiff | heif | cr3 | raf | video
    pub container: &'static str,
    pub is_raw: bool,
    pub make: Option<String>,
    pub model: Option<String>,
    pub body_serial: Option<String>,
    pub lens: Option<String>,
    pub software: Option<String>,
    pub focal: Option<f64>,
    pub focal35: Option<f64>,
    pub aperture: Option<f64>,
    pub iso: Option<u32>,
    pub exposure: Option<f64>,
    pub bias: Option<f64>,
    /// The written wall-clock time read as UTC, in seconds since the epoch.
    /// The instant is `taken_at - tz_offset` when the offset is known.
    pub taken_at: Option<i64>,
    pub subsec_ms: Option<u32>,
    /// Seconds east of UTC.
    pub tz_offset: Option<i32>,
    pub orientation: Option<u8>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// The sensor image of a RAW.
    pub raw_width: Option<u32>,
    pub raw_height: Option<u32>,
    pub preview: Option<Span>,
    pub gps: Option<Gps>,
    pub xmp: Option<Xmp>,
    /// Set by the caller when the window could not cover what the parser wanted.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Extract {
    Done(Box<Meta>),
    /// The window ended before something the parser needs: read at least this
    /// many bytes from the start of the file and try again.
    NeedMore { bytes: u64, partial: Box<Meta> },
    /// Not a format this extractor reads.
    Unsupported,
    /// Recognised, but damaged or without anything to read.
    Corrupt(String),
}

fn have_core(m: &Meta) -> bool {
    m.make.is_some() || m.model.is_some() || m.taken_at.is_some() || m.width.is_some() || m.iso.is_some()
}

/// Reads the metadata of a file from `head`, its first bytes. `file_size` is
/// the whole file's size, so a window that covers it never asks for more.
/// `ext` (without the dot) only decides what counts as a raw file.
pub fn extract(head: &[u8], file_size: u64, ext: &str) -> Extract {
    if head.len() < 12 {
        return if file_size < 12 { Extract::Corrupt("file too short".into()) } else { Extract::NeedMore { bytes: 12, partial: Box::default() } };
    }
    let mut meta = Meta::default();
    let mut need = 0u64;
    let found;
    if head.starts_with(&[0xFF, 0xD8]) {
        meta.container = "jpeg";
        found = jpeg(head, file_size, 0, &mut meta, &mut need);
        // the file is its own preview
        meta.preview = None;
    } else if head.starts_with(b"FUJIFILMCCD-RAW") {
        meta.container = "raf";
        found = raf(head, file_size, &mut meta, &mut need);
    } else if let Some(t) = Tiff::new(head, file_size, 0) {
        meta.container = if is_raw_ext(ext) { "raw-tiff" } else { "tiff" };
        found = t.read(&mut meta);
        need = need.max(t.need());
    } else if crate::catalog_meta_iso::is_iso(head) {
        let (n, f) = crate::catalog_meta_iso::read(head, file_size, &mut meta);
        meta.container = if head[8..12] == *b"crx " { "cr3" } else if meta.make.is_none() && meta.preview.is_none() && meta.xmp.is_none() && meta.model.is_none() && !is_heif_brand(&head[8..12]) { "video" } else { "heif" };
        need = need.max(n);
        found = f;
    } else {
        return Extract::Unsupported;
    }
    meta.is_raw = is_raw_ext(ext) || matches!(meta.container, "raf" | "cr3" | "raw-tiff");
    if need > head.len() as u64 && (head.len() as u64) < file_size {
        return Extract::NeedMore { bytes: need, partial: Box::new(meta) };
    }
    let _ = found;
    if !have_core(&meta) {
        return Extract::Corrupt("no readable metadata".into());
    }
    Extract::Done(Box::new(meta))
}

fn is_heif_brand(b: &[u8]) -> bool {
    matches!(b, b"heic" | b"heix" | b"hevc" | b"hevx" | b"mif1" | b"msf1" | b"avif" | b"avis" | b"heim" | b"heis")
}

/// A JPEG stream at `buf[0]`: `total` is its length in the file and `base`
/// where it starts (so misses are reported in file offsets). The frame header's
/// size is the JPEG's own (for a RAF, the embedded preview's, which is its size).
fn jpeg(buf: &[u8], total: u64, base: u64, meta: &mut Meta, need: &mut u64) -> bool {
    let mut found = false;
    let mut have_sof = false;
    let blen = buf.len() as u64;
    let mut p = 2u64;
    // a JPEG's headers are at its start: stop after a bounded number of segments
    for _ in 0..256 {
        // find the next marker
        while p < blen && buf[p as usize] != 0xFF {
            p += 1;
        }
        while p < blen && buf[p as usize] == 0xFF {
            p += 1;
        }
        if p >= blen {
            if !have_sof && p < total {
                *need = (*need).max(base + p + 16);
            }
            break;
        }
        let marker = buf[p as usize];
        p += 1;
        if marker == 0xD9 || marker == 0xDA {
            break;
        }
        if marker == 0x00 || marker == 0x01 || (0xD0..=0xD8).contains(&marker) {
            continue;
        }
        if p + 2 > blen {
            if p + 2 <= total {
                *need = (*need).max(base + p + 2 + 16);
            }
            break;
        }
        let len = u16::from_be_bytes([buf[p as usize], buf[p as usize + 1]]) as u64;
        if len < 2 {
            break;
        }
        let seg_end = p + len;
        let payload = &buf[(p + 2) as usize..seg_end.min(blen) as usize];
        match marker {
            0xE1 if payload.starts_with(b"Exif\0\0") => {
                if let Some(t) = Tiff::new(&payload[6..], len - 2 - 6, base + p + 2 + 6) {
                    found |= t.read(meta);
                    *need = (*need).max(t.need());
                }
            }
            0xE1 if payload.starts_with(b"http://ns.adobe.com/xap/1.0/\0") && seg_end <= blen && meta.xmp.is_none() => {
                if let Some(x) = catalog_meta_xmp::parse(&payload[29..]) {
                    meta.xmp = Some(x);
                    found = true;
                }
            }
            0xC0..=0xCF if !matches!(marker, 0xC4 | 0xC8 | 0xCC) && payload.len() >= 5 => {
                let h = u16::from_be_bytes([payload[1], payload[2]]) as u32;
                let w = u16::from_be_bytes([payload[3], payload[4]]) as u32;
                if w > 0 && h > 0 {
                    have_sof = true;
                    found = true;
                    meta.width = Some(w);
                    meta.height = Some(h);
                }
            }
            _ => {}
        }
        if seg_end > blen {
            // the segment runs past the window; it only matters if the size is still unknown
            if !have_sof && meta.width.is_none() && seg_end + 16 <= total {
                *need = (*need).max(base + seg_end + 16);
            }
            break;
        }
        p = seg_end;
    }
    found
}

/// Fujifilm RAF: a big-endian header with the embedded JPEG's offset and length
/// at 84 and 88; that JPEG carries the Exif.
fn raf(buf: &[u8], file_size: u64, meta: &mut Meta, need: &mut u64) -> bool {
    let be = |o: usize| -> Option<u64> { buf.get(o..o + 4).map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as u64) };
    let (Some(off), Some(len)) = (be(84), be(88)) else {
        *need = (*need).max(96);
        return false;
    };
    // the camera name, from the header, for files whose Exif cannot be read
    if let Some(name) = buf.get(28..60).and_then(crate::catalog_meta_tiff::clean) {
        meta.model = Some(name);
        meta.make = Some("FUJIFILM".into());
    }
    if len < 4 || off.checked_add(len).is_none_or(|e| e > file_size) {
        return false;
    }
    meta.preview = Some(Span { offset: off, len, width: None, height: None });
    if off >= buf.len() as u64 {
        *need = (*need).max(off + 16 * 1024);
        return true;
    }
    let inner = &buf[off as usize..(off + len).min(buf.len() as u64) as usize];
    if inner.starts_with(&[0xFF, 0xD8]) {
        let mut m = Meta::default();
        jpeg(inner, len, off, &mut m, need);
        // the size of a RAF is its embedded JPEG's; the JPEG is also the preview
        m.preview = Some(Span { offset: off, len, width: m.width, height: m.height });
        m.container = meta.container;
        m.make = m.make.or_else(|| meta.make.take());
        m.model = m.model.or_else(|| meta.model.take());
        *meta = m;
    }
    true
}

#[cfg(test)]
#[path = "catalog_meta_tests.rs"]
mod tests;
