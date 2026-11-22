// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! A small, bounded TIFF/EXIF reader over a byte window: IFD0, the Exif and GPS
//! IFDs, the SubIFDs and the IFD chain. It serves every TIFF-based RAW (ARW,
//! NEF, DNG, CR2, ORF, RW2, PEF, SRW, ...), TIFF itself, the EXIF block of a
//! JPEG, the Exif item of a HEIF and Canon's CMT boxes.
//!
//! Rules that keep it safe on hostile or truncated input:
//!
//! * every offset and size is checked in `u64` before it is used, and a read
//!   that is inside the file but past the window reports where the window would
//!   have to end (`Miss::Beyond`) instead of failing, so the caller can read
//!   more; a read past the end of the file is `Miss::Bad`;
//! * IFDs are visited at most once (a loop is cut), chains and SubIFD lists are
//!   short, entry counts and value sizes are capped, nothing is allocated from a
//!   count the file claims;
//! * no value is trusted: dates, GPS and rationals are range-checked.

use crate::catalog_meta::{Gps, Meta, Span};
use std::cell::Cell;
use std::collections::HashSet;

/// IFD entries read from one directory, at most.
const MAX_ENTRIES: u64 = 2048;
/// A directory chain (IFD0 -> IFD1 -> ...) is this long at most.
const MAX_CHAIN: usize = 6;
const MAX_SUBIFDS: usize = 24;
/// Directory nesting (IFD0 -> SubIFD -> SubIFD) is this deep at most.
const MAX_DEPTH: u32 = 4;
/// Longest string value kept.
const MAX_STR: u64 = 512;
/// A JPEG of at least this many bytes is a preview worth locating; smaller ones are thumbnails.
const GOOD_PREVIEW: u64 = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Miss {
    /// Inside the file, but the window ends before this absolute end offset.
    Beyond(u64),
    /// Outside the file or malformed.
    Bad,
}

type R<T> = Result<T, Miss>;

#[derive(Clone, Copy)]
struct Entry {
    tag: u16,
    typ: u16,
    count: u64,
    /// Where the 4 value bytes of the entry are, in the TIFF block.
    at: u64,
}

fn type_size(t: u16) -> u64 {
    match t {
        1 | 2 | 6 | 7 => 1,
        3 | 8 => 2,
        4 | 9 | 11 | 13 => 4,
        5 | 10 | 12 => 8,
        _ => 0,
    }
}

/// A TIFF block: `buf` is what is available of it, `len` how long it is in
/// the file, `base` where it starts in the file (for reporting misses).
pub struct Tiff<'a> {
    buf: &'a [u8],
    len: u64,
    base: u64,
    le: bool,
    /// The highest absolute end offset a required read needed beyond the window.
    need: Cell<u64>,
}

/// What one image directory says about its image.
#[derive(Default, Clone, Debug)]
struct Image {
    width: u32,
    height: u32,
    compression: u32,
    photometric: u32,
    subfile: u32,
    jpeg: Option<(u64, u64)>,
    strips: Option<(u64, u64)>,
}

impl Image {
    fn area(&self) -> u64 {
        self.width as u64 * self.height as u64
    }
    fn is_jpeg_coded(&self) -> bool {
        matches!(self.compression, 6 | 7) && !matches!(self.photometric, 32803 | 34892)
    }
    /// A raw sensor image: CFA or linear raw, or a compression no viewer reads.
    fn is_raw(&self) -> bool {
        matches!(self.photometric, 32803 | 34892)
            || matches!(self.compression, 32767 | 32769 | 32770 | 32773 | 34713 | 34892 | 65535 | 34316 | 34925)
            || (self.compression == 1 && self.subfile == 0 && self.photometric == 32803)
    }
    fn preview(&self) -> Option<(u64, u64)> {
        if let Some(j) = self.jpeg {
            return Some(j);
        }
        if self.is_jpeg_coded() {
            return self.strips;
        }
        None
    }
}

impl<'a> Tiff<'a> {
    /// `buf` starts with the TIFF header. `len` is the whole block's length in
    /// the file when known (the file size, or a segment's length).
    pub fn new(buf: &'a [u8], len: u64, base: u64) -> Option<Self> {
        if buf.len() < 8 {
            return None;
        }
        let le = match &buf[..2] {
            b"II" => true,
            b"MM" => false,
            _ => return None,
        };
        let t = Tiff { buf, len: len.max(buf.len() as u64), base, le, need: Cell::new(0) };
        // 42 is TIFF; ORF uses 'OR'/'RS', Panasonic RW2 uses 0x55
        let magic = t.u16(2).ok()?;
        if !matches!(magic, 42 | 0x4F52 | 0x5352 | 0x55) {
            return None;
        }
        Some(t)
    }

    /// The highest end offset (absolute) that a required read wanted and the
    /// window did not have; 0 when the window was enough.
    pub fn need(&self) -> u64 {
        self.need.get()
    }

    fn slice(&self, off: u64, n: u64) -> R<&'a [u8]> {
        let end = off.checked_add(n).ok_or(Miss::Bad)?;
        if end > self.len {
            return Err(Miss::Bad);
        }
        if end > self.buf.len() as u64 {
            return Err(Miss::Beyond(self.base + end));
        }
        Ok(&self.buf[off as usize..end as usize])
    }

    fn u16(&self, off: u64) -> R<u16> {
        let b = self.slice(off, 2)?;
        Ok(if self.le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) })
    }

    fn u32(&self, off: u64) -> R<u32> {
        let b = self.slice(off, 4)?;
        let a = [b[0], b[1], b[2], b[3]];
        Ok(if self.le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) })
    }

    /// Records a miss of a required read; other failures are only `None`.
    fn must<T>(&self, r: R<T>) -> Option<T> {
        match r {
            Ok(v) => Some(v),
            Err(Miss::Beyond(end)) => {
                self.need.set(self.need.get().max(end));
                None
            }
            Err(Miss::Bad) => None,
        }
    }

    fn entries(&self, off: u64) -> R<(Vec<Entry>, Option<u64>)> {
        let n = self.u16(off)? as u64;
        if n > MAX_ENTRIES {
            return Err(Miss::Bad);
        }
        let table = self.slice(off + 2, n * 12)?;
        let mut v = Vec::with_capacity(n as usize);
        for i in 0..n {
            let e = &table[(i * 12) as usize..(i * 12 + 12) as usize];
            let (tag, typ, count) = if self.le {
                (u16::from_le_bytes([e[0], e[1]]), u16::from_le_bytes([e[2], e[3]]), u32::from_le_bytes([e[4], e[5], e[6], e[7]]))
            } else {
                (u16::from_be_bytes([e[0], e[1]]), u16::from_be_bytes([e[2], e[3]]), u32::from_be_bytes([e[4], e[5], e[6], e[7]]))
            };
            v.push(Entry { tag, typ, count: count as u64, at: off + 2 + i * 12 + 8 });
        }
        // the next-IFD pointer is optional: a window that ends before it is no loss
        let next = self.u32(off + 2 + n * 12).ok().filter(|o| *o != 0).map(|o| o as u64);
        Ok((v, next))
    }

    /// The bytes of an entry's value, at most `cap` of them.
    fn data(&self, e: &Entry, cap: u64) -> R<&'a [u8]> {
        let size = type_size(e.typ).checked_mul(e.count).ok_or(Miss::Bad)?;
        if type_size(e.typ) == 0 {
            return Err(Miss::Bad);
        }
        let want = size.min(cap);
        if size <= 4 {
            return self.slice(e.at, want);
        }
        let off = self.u32(e.at)? as u64;
        self.slice(off, want)
    }

    fn uint(&self, e: &Entry, idx: u64) -> R<u64> {
        let sz = type_size(e.typ);
        if idx >= e.count {
            return Err(Miss::Bad);
        }
        let d = self.data(e, (idx + 1) * sz)?;
        let o = (idx * sz) as usize;
        match e.typ {
            1 | 7 | 6 => Ok(d[o] as u64),
            3 | 8 => Ok(self.rd16(&d[o..]) as u64),
            4 | 9 | 13 => Ok(self.rd32(&d[o..]) as u64),
            _ => Err(Miss::Bad),
        }
    }

    fn rd16(&self, b: &[u8]) -> u16 {
        if self.le { u16::from_le_bytes([b[0], b[1]]) } else { u16::from_be_bytes([b[0], b[1]]) }
    }
    fn rd32(&self, b: &[u8]) -> u32 {
        let a = [b[0], b[1], b[2], b[3]];
        if self.le { u32::from_le_bytes(a) } else { u32::from_be_bytes(a) }
    }

    fn rational(&self, e: &Entry, idx: u64) -> R<f64> {
        if idx >= e.count || !matches!(e.typ, 5 | 10) {
            return Err(Miss::Bad);
        }
        let d = self.data(e, (idx + 1) * 8)?;
        let o = (idx * 8) as usize;
        let (n, dn) = (self.rd32(&d[o..]), self.rd32(&d[o + 4..]));
        if dn == 0 {
            return Err(Miss::Bad);
        }
        let v = if e.typ == 10 { (n as i32) as f64 / (dn as i32) as f64 } else { n as f64 / dn as f64 };
        if v.is_finite() { Ok(v) } else { Err(Miss::Bad) }
    }

    /// Any numeric value as a float: rationals and integers alike.
    fn number(&self, e: &Entry) -> R<f64> {
        match e.typ {
            5 | 10 => self.rational(e, 0),
            3 | 4 | 1 | 8 | 9 => self.uint(e, 0).map(|v| if e.typ == 8 { (v as u16 as i16) as f64 } else if e.typ == 9 { (v as u32 as i32) as f64 } else { v as f64 }),
            _ => Err(Miss::Bad),
        }
    }

    fn string(&self, e: &Entry) -> R<Option<String>> {
        if !matches!(e.typ, 2 | 1 | 7) {
            return Err(Miss::Bad);
        }
        let d = self.data(e, MAX_STR)?;
        Ok(clean(d))
    }

    /// Reads everything the catalog wants, starting at the first IFD.
    pub fn read(&self, meta: &mut Meta) -> bool {
        let Some(first) = self.must(self.u32(4)) else { return false };
        let mut st = State::default();
        let mut images: Vec<Image> = Vec::new();
        let mut off = Some(first as u64);
        let mut n = 0;
        let mut ok = false;
        while let Some(o) = off {
            if n >= MAX_CHAIN || !st.seen.insert(o) {
                break;
            }
            match self.ifd(o, 0, n == 0, meta, &mut st, &mut images, n == 0) {
                Ok(next) => {
                    ok = true;
                    off = next;
                }
                Err(()) => break,
            }
            n += 1;
        }
        // pick the preview and the raw size from the images found
        let best_preview = images
            .iter()
            .filter_map(|i| i.preview().map(|(o, l)| (i.area(), l, o)))
            .filter(|(_, l, o)| *l > 0 && o.checked_add(*l).is_some_and(|e| e <= self.len))
            .max_by_key(|(a, l, _)| (*a, *l));
        if let Some((_, len, off)) = best_preview {
            let (w, h) = images
                .iter()
                .find(|i| i.preview() == Some((off, len)))
                .map_or((0, 0), |i| (i.width, i.height));
            meta.preview = Some(Span { offset: self.base + off, len, width: (w > 0).then_some(w), height: (h > 0).then_some(h) });
        }
        if let Some(raw) = images.iter().filter(|i| i.is_raw() && i.width > 0 && i.height > 0).max_by_key(|i| i.area()) {
            meta.raw_width = Some(raw.width);
            meta.raw_height = Some(raw.height);
        }
        // the sensor-sized image of a plain TIFF, when no Exif size was given
        if meta.width.is_none() {
            let main = images
                .iter()
                .filter(|i| i.width > 0 && i.height > 0 && i.subfile & 1 == 0 && !i.is_jpeg_coded())
                .max_by_key(|i| i.area())
                .or_else(|| images.first().filter(|i| i.width > 0 && i.height > 0));
            if let Some(m) = main {
                meta.width = Some(m.width);
                meta.height = Some(m.height);
            }
        }
        ok
    }

    /// For Canon's CMT2: the block's first directory is the Exif IFD itself.
    pub fn read_exif_root(&self, meta: &mut Meta) {
        if let Some(o) = self.must(self.u32(4)) {
            self.exif_ifd(o as u64, meta, &mut State::default());
        }
    }

    /// For Canon's CMT4: the block's first directory is the GPS IFD.
    pub fn read_gps_root(&self, meta: &mut Meta) {
        if let Some(o) = self.must(self.u32(4)) {
            self.gps_ifd(o as u64, meta);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn ifd(&self, off: u64, depth: u32, primary: bool, meta: &mut Meta, st: &mut State, images: &mut Vec<Image>, required: bool) -> Result<Option<u64>, ()> {
        let (entries, next) = match self.entries(off) {
            Ok(x) => x,
            Err(Miss::Beyond(end)) => {
                if required {
                    self.need.set(self.need.get().max(end));
                }
                return Err(());
            }
            Err(Miss::Bad) => return Err(()),
        };
        let mut img = Image::default();
        let mut exif: Option<u64> = None;
        let mut gps: Option<u64> = None;
        let mut subs: Vec<u64> = Vec::new();
        let (mut strip_off, mut strip_len) = (None, None);
        for e in &entries {
            match e.tag {
                0x00FE => img.subfile = self.uint(e, 0).unwrap_or(0) as u32,
                0x0100 => img.width = self.uint(e, 0).unwrap_or(0).min(u32::MAX as u64) as u32,
                0x0101 => img.height = self.uint(e, 0).unwrap_or(0).min(u32::MAX as u64) as u32,
                0x0103 => img.compression = self.uint(e, 0).unwrap_or(0) as u32,
                0x0106 => img.photometric = self.uint(e, 0).unwrap_or(0) as u32,
                0x010F if primary || meta.make.is_none() => {
                    if let Some(Some(s)) = self.must(self.string(e)) {
                        meta.make.get_or_insert(s);
                    }
                }
                0x0110 if primary || meta.model.is_none() => {
                    if let Some(Some(s)) = self.must(self.string(e)) {
                        meta.model.get_or_insert(s);
                    }
                }
                0x0112 if depth == 0 && meta.orientation.is_none() => {
                    if let Ok(v) = self.uint(e, 0) {
                        if (1..=8).contains(&v) {
                            meta.orientation = Some(v as u8);
                        }
                    }
                }
                // an embedded XMP packet (DNG and some other RAWs): only when it is in the window
                0x02BC if depth == 0 && meta.xmp.is_none() && e.count <= 1 << 20 => {
                    if let Ok(d) = self.data(e, e.count) {
                        meta.xmp = crate::catalog_meta_xmp::parse(d);
                    }
                }
                0x0131 if meta.software.is_none() => {
                    if let Some(Some(s)) = self.must(self.string(e)) {
                        meta.software = Some(s);
                    }
                }
                0x0132 if st.datetime.is_none() => {
                    if let Some(Some(s)) = self.must(self.string(e)) {
                        st.datetime = Some(s);
                    }
                }
                0x0111 => strip_off = if e.count == 1 { self.uint(e, 0).ok() } else { None },
                0x0117 => strip_len = if e.count == 1 { self.uint(e, 0).ok() } else { None },
                0x0201 => {
                    if let Ok(o) = self.uint(e, 0) {
                        img.jpeg = Some((o, img.jpeg.map_or(0, |j| j.1)));
                    }
                }
                0x0202 => {
                    if let Ok(l) = self.uint(e, 0) {
                        img.jpeg = Some((img.jpeg.map_or(0, |j| j.0), l));
                    }
                }
                0x014A if depth < MAX_DEPTH => {
                    let n = e.count.min(MAX_SUBIFDS as u64);
                    for i in 0..n {
                        if let Some(o) = self.must(self.uint(e, i)) {
                            subs.push(o);
                        }
                    }
                }
                0x8769 if depth < MAX_DEPTH => exif = self.must(self.uint(e, 0)),
                0x8825 if depth < MAX_DEPTH => gps = self.must(self.uint(e, 0)),
                // DNG: the camera's serial
                0xC62F if meta.body_serial.is_none() => {
                    if let Some(Some(s)) = self.must(self.string(e)) {
                        meta.body_serial = Some(s);
                    }
                }
                0xC614 if meta.model.is_none() => {
                    if let Some(Some(s)) = self.must(self.string(e)) {
                        meta.model = Some(s);
                    }
                }
                _ => {}
            }
        }
        if let (Some(o), Some(l)) = (strip_off, strip_len) {
            img.strips = Some((o, l));
        }
        if img.width > 0 || img.jpeg.is_some() || img.strips.is_some() {
            images.push(img);
        }
        if let Some(o) = exif {
            if st.seen.insert(o) {
                self.exif_ifd(o, meta, st);
            }
        }
        if let Some(o) = gps {
            if st.seen.insert(o) {
                self.gps_ifd(o, meta);
            }
        }
        // SubIFDs hold the sensor image (its true size) and sometimes the only big
        // preview. A RAW such as ARW keeps them far from the head (about 133 KB in
        // a Sony ARW) while IFD0 already locates a full preview: then they are read
        // only when the window happens to hold them, never worth another read.
        let have_preview = images.iter().any(|i| i.preview().is_some_and(|(_, l)| l >= GOOD_PREVIEW));
        for o in subs {
            if st.seen.insert(o) {
                let _ = self.ifd(o, depth + 1, false, meta, st, images, !have_preview);
            }
        }
        if primary {
            if let (None, Some(dt)) = (meta.taken_at, st.datetime.as_deref()) {
                if let Some(t) = parse_datetime(dt) {
                    meta.taken_at = Some(t);
                }
            }
        }
        Ok(next)
    }

    fn exif_ifd(&self, off: u64, meta: &mut Meta, st: &mut State) {
        let (entries, _) = match self.entries(off) {
            Ok(x) => x,
            Err(e) => {
                self.must::<()>(Err(e));
                return;
            }
        };
        let (mut original, mut digitized, mut off_orig, mut off_any, mut sub_orig) = (None, None, None, None, None);
        let (mut iso, mut iso_speed) = (None, None);
        for e in &entries {
            match e.tag {
                0x829A => meta.exposure = self.must(self.number(e)).filter(|v| *v > 0.0 && *v < 1e5),
                0x829D => meta.aperture = self.must(self.number(e)).filter(|v| *v > 0.0 && *v < 1000.0),
                0x8827 => iso = self.must(self.uint(e, 0)),
                0x8833 => iso_speed = self.must(self.uint(e, 0)),
                0x9003 => original = self.must(self.string(e)).flatten(),
                0x9004 => digitized = self.must(self.string(e)).flatten(),
                0x9010 => off_any = self.must(self.string(e)).flatten(),
                0x9011 => off_orig = self.must(self.string(e)).flatten(),
                0x9291 => sub_orig = self.must(self.string(e)).flatten(),
                0x9204 => meta.bias = self.must(self.number(e)).filter(|v| v.abs() < 100.0),
                0x920A => meta.focal = self.must(self.number(e)).filter(|v| *v > 0.0 && *v < 10_000.0),
                0xA405 => meta.focal35 = self.must(self.uint(e, 0)).filter(|v| *v > 0 && *v < 10_000).map(|v| v as f64),
                0xA002 => st.pixel_x = self.must(self.uint(e, 0)).map(|v| v.min(u32::MAX as u64) as u32),
                0xA003 => st.pixel_y = self.must(self.uint(e, 0)).map(|v| v.min(u32::MAX as u64) as u32),
                0xA431 if meta.body_serial.is_none() => meta.body_serial = self.must(self.string(e)).flatten(),
                0xA434 => meta.lens = self.must(self.string(e)).flatten(),
                0xA433 => st.lens_make = self.must(self.string(e)).flatten(),
                _ => {}
            }
        }
        if let Some(v) = iso.filter(|v| *v > 0 && *v < 65535).or(iso_speed.filter(|v| *v > 0 && *v < 100_000_000)) {
            meta.iso = Some(v as u32);
        }
        if let Some(t) = original.as_deref().and_then(parse_datetime).or_else(|| digitized.as_deref().and_then(parse_datetime)) {
            meta.taken_at = Some(t);
        }
        meta.subsec_ms = sub_orig.as_deref().and_then(parse_subsec);
        meta.tz_offset = off_orig.as_deref().or(off_any.as_deref()).and_then(parse_offset);
        if let (Some(w), Some(h)) = (st.pixel_x, st.pixel_y) {
            if w > 0 && h > 0 {
                meta.width = Some(w);
                meta.height = Some(h);
            }
        }
        // a lens model that is only the maker's name carries no information
        if let (Some(l), Some(m)) = (meta.lens.as_deref(), st.lens_make.as_deref()) {
            if l.eq_ignore_ascii_case(m) {
                meta.lens = None;
            }
        }
    }

    fn gps_ifd(&self, off: u64, meta: &mut Meta) {
        let (entries, _) = match self.entries(off) {
            Ok(x) => x,
            Err(e) => {
                self.must::<()>(Err(e));
                return;
            }
        };
        let (mut lat_ref, mut lon_ref, mut lat, mut lon, mut alt_ref, mut alt) = (None, None, None, None, 0u64, None);
        let dms = |e: &Entry| -> Option<f64> {
            if e.count < 1 || e.count > 3 {
                return None;
            }
            let d = self.must(self.rational(e, 0))?;
            let m = if e.count > 1 { self.must(self.rational(e, 1))? } else { 0.0 };
            let s = if e.count > 2 { self.must(self.rational(e, 2))? } else { 0.0 };
            Some(d + m / 60.0 + s / 3600.0)
        };
        for e in &entries {
            match e.tag {
                1 => lat_ref = self.must(self.string(e)).flatten(),
                2 => lat = dms(e),
                3 => lon_ref = self.must(self.string(e)).flatten(),
                4 => lon = dms(e),
                5 => alt_ref = self.must(self.uint(e, 0)).unwrap_or(0),
                6 => alt = self.must(self.rational(e, 0)),
                _ => {}
            }
        }
        let sign = |r: &Option<String>, neg: char| -> Option<f64> {
            match r.as_deref()?.trim().chars().next()?.to_ascii_uppercase() {
                c if c == neg => Some(-1.0),
                'N' | 'E' => Some(1.0),
                _ => None,
            }
        };
        if let (Some(la), Some(lo), Some(sa), Some(so)) = (lat, lon, sign(&lat_ref, 'S'), sign(&lon_ref, 'W')) {
            let (la, lo) = (la * sa, lo * so);
            if la.abs() <= 90.0 && lo.abs() <= 180.0 {
                meta.gps = Some(Gps {
                    lat: la,
                    lon: lo,
                    alt: alt.filter(|a| a.abs() < 100_000.0).map(|a| if alt_ref == 1 { -a } else { a }),
                    lat_ref: lat_ref.map(|s| s.trim().chars().take(1).collect()),
                    lon_ref: lon_ref.map(|s| s.trim().chars().take(1).collect()),
                });
            }
        }
    }
}

#[derive(Default)]
struct State {
    seen: HashSet<u64>,
    datetime: Option<String>,
    pixel_x: Option<u32>,
    pixel_y: Option<u32>,
    lens_make: Option<String>,
}

/// An ASCII value without its terminator and padding; None when empty or not text.
pub fn clean(d: &[u8]) -> Option<String> {
    let end = d.iter().position(|b| *b == 0).unwrap_or(d.len());
    let s = String::from_utf8_lossy(&d[..end]);
    let s = s.trim_matches(|c: char| c.is_whitespace() || c.is_control());
    if s.is_empty() || s.chars().all(|c| c == '\u{FFFD}') {
        None
    } else {
        Some(s.chars().take(MAX_STR as usize).collect())
    }
}

/// Days since 1970-01-01 of a civil date (proleptic Gregorian).
pub fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// "2025:10:05 14:08:39" (also `-` separators or a `T`) as seconds since the
/// epoch of the written wall-clock time read as UTC. None for blank or zero dates.
pub fn parse_datetime(s: &str) -> Option<i64> {
    let b = s.trim().as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |a: usize, n: usize| -> Option<i64> {
        let part = b.get(a..a + n)?;
        if !part.iter().all(u8::is_ascii_digit) {
            return None;
        }
        Some(part.iter().fold(0i64, |x, d| x * 10 + (d - b'0') as i64))
    };
    let (y, mo, d, h, mi, se) = (num(0, 4)?, num(5, 2)?, num(8, 2)?, num(11, 2)?, num(14, 2)?, num(17, 2)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || se > 60 || y < 1 {
        return None;
    }
    Some(days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + se)
}

/// SubSecTime "12" is 0.12 s, so 120 ms.
pub fn parse_subsec(s: &str) -> Option<u32> {
    let digits: Vec<u32> = s.trim().chars().take_while(char::is_ascii_digit).map(|c| c as u32 - '0' as u32).collect();
    if digits.is_empty() {
        return None;
    }
    let mut ms = 0;
    for i in 0..3 {
        ms = ms * 10 + digits.get(i).copied().unwrap_or(0);
    }
    Some(ms)
}

/// "+05:30", "-08:00" or "Z" in seconds east of UTC.
pub fn parse_offset(s: &str) -> Option<i32> {
    let s = s.trim();
    if s.eq_ignore_ascii_case("z") {
        return Some(0);
    }
    let b = s.as_bytes();
    let sign = match b.first()? {
        b'+' => 1,
        b'-' => -1,
        _ => return None,
    };
    let rest = &s[1..];
    let (h, m) = match rest.split_once(':') {
        Some((h, m)) => (h, m),
        None if rest.len() == 4 => rest.split_at(2),
        None => (rest, "0"),
    };
    let (h, m): (i32, i32) = (h.parse().ok()?, m.parse().ok()?);
    if h > 14 || m > 59 {
        return None;
    }
    Some(sign * (h * 3600 + m * 60))
}
