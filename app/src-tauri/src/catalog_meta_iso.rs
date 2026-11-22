// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! ISO base media files, for the catalog: HEIC, HEIF and AVIF (the Exif and XMP
//! items found through `meta`/`iinf`/`iloc`, the image size from `ispe`), Canon
//! CR3 (the CMT1-CMT4 TIFF boxes in the `moov` uuid box, the PRVW preview, the
//! CRAW sample size) and the creation time and picture size of MP4 and MOV.
//!
//! Boxes are walked over the head window only. A box header past the window
//! (inside the file) is a request for more bytes (`need`), never an error;
//! sizes are checked before use and nesting is shallow.

use crate::catalog_meta::{Meta, Span};
use crate::catalog_meta_tiff::Tiff;
use crate::catalog_meta_xmp;

const MAX_DEPTH: u32 = 8;
/// Exif and XMP items are never legitimately larger than this.
const MAX_ITEM: u64 = 4 * 1024 * 1024;
const CANON_MOOV: [u8; 16] = [0x85, 0xc0, 0xb6, 0x87, 0x82, 0x0f, 0x11, 0xe0, 0x81, 0x11, 0xf4, 0xce, 0x46, 0x2b, 0x6a, 0x48];
const CANON_PRVW: [u8; 16] = [0xea, 0xf4, 0x2b, 0x5e, 0x1c, 0x98, 0x4b, 0x88, 0xb9, 0xfb, 0xb7, 0xdc, 0x40, 0x6e, 0x4d, 0x16];
/// Seconds between 1904-01-01 (QuickTime) and 1970-01-01.
const MAC_EPOCH_TO_UNIX: u64 = 2_082_844_800;

#[derive(Default)]
struct Item {
    id: u32,
    typ: [u8; 4],
    content_type: String,
}

struct Loc {
    id: u32,
    offset: u64,
    len: u64,
}

struct Iso<'a, 'm> {
    buf: &'a [u8],
    file_size: u64,
    meta: &'m mut Meta,
    need: u64,
    brand: [u8; 4],
    items: Vec<Item>,
    locs: Vec<Loc>,
    ispe: (u32, u32),
    sample: (u32, u32),
    track: (u32, u32),
    cmt1: bool,
    prvw: bool,
    mdat: bool,
}

fn be16(b: &[u8], o: u64) -> Option<u16> {
    let o = usize::try_from(o).ok()?;
    Some(u16::from_be_bytes(b.get(o..o.checked_add(2)?)?.try_into().ok()?))
}

fn be32(b: &[u8], o: u64) -> Option<u32> {
    let o = usize::try_from(o).ok()?;
    Some(u32::from_be_bytes(b.get(o..o.checked_add(4)?)?.try_into().ok()?))
}

fn be64(b: &[u8], o: u64) -> Option<u64> {
    let o = usize::try_from(o).ok()?;
    Some(u64::from_be_bytes(b.get(o..o.checked_add(8)?)?.try_into().ok()?))
}

/// True for the brands of files this module reads.
pub fn is_iso(head: &[u8]) -> bool {
    head.len() >= 12 && matches!(&head[4..8], b"ftyp" | b"moov" | b"mdat" | b"wide" | b"free" | b"skip" | b"pnot")
}

/// Fills `meta` from an ISO file's head. Returns the absolute end offset more
/// bytes were wanted at (0 when the window sufficed) and whether anything was read.
pub fn read(buf: &[u8], file_size: u64, meta: &mut Meta) -> (u64, bool) {
    let mut iso = Iso {
        buf,
        file_size,
        meta,
        need: 0,
        brand: [0; 4],
        items: Vec::new(),
        locs: Vec::new(),
        ispe: (0, 0),
        sample: (0, 0),
        track: (0, 0),
        cmt1: false,
        prvw: false,
        mdat: false,
    };
    iso.children(0, file_size, 0, true);
    iso.finish();
    let found = iso.meta.make.is_some() || iso.meta.taken_at.is_some() || iso.meta.width.is_some() || iso.meta.model.is_some();
    (iso.need, found)
}

impl Iso<'_, '_> {
    fn want(&mut self, end: u64) {
        if end <= self.file_size {
            self.need = self.need.max(end);
        }
    }

    fn children(&mut self, start: u64, end: u64, depth: u32, top: bool) {
        if depth > MAX_DEPTH {
            return;
        }
        let blen = self.buf.len() as u64;
        let mut p = start;
        while p.checked_add(8).is_some_and(|e| e <= end) {
            if p + 8 > blen {
                if top && !self.mdat {
                    self.want(p + 32);
                }
                return;
            }
            let Some(size32) = be32(self.buf, p) else { return };
            let typ: [u8; 4] = self.buf[p as usize + 4..p as usize + 8].try_into().unwrap();
            let (hdr, size) = match size32 {
                1 => {
                    let Some(s) = be64(self.buf, p + 8) else {
                        if top {
                            self.want(p + 16);
                        }
                        return;
                    };
                    (16, s)
                }
                0 => (8, end - p),
                n => (8, n as u64),
            };
            if size < hdr || p.saturating_add(size) > end {
                return;
            }
            self.visit(typ, p + hdr, p + size, depth, top);
            if top && self.mdat {
                return;
            }
            p += size;
        }
    }

    fn visit(&mut self, typ: [u8; 4], cs: u64, ce: u64, depth: u32, top: bool) {
        let blen = self.buf.len() as u64;
        let end = ce.min(blen);
        match &typ {
            b"ftyp" => {
                if let Some(b) = self.buf.get(cs as usize..cs as usize + 4) {
                    self.brand = b.try_into().unwrap();
                }
            }
            b"mdat" => self.mdat = true,
            b"meta" => {
                self.children(cs + 4, end, depth + 1, false);
                if top && (self.items.is_empty() || self.locs.is_empty()) {
                    self.want(ce);
                }
            }
            b"iprp" | b"ipco" | b"trak" | b"mdia" | b"minf" | b"stbl" => self.children(cs, end, depth + 1, false),
            b"moov" => {
                self.children(cs, end, depth + 1, false);
                if &self.brand == b"crx " && !self.cmt1 && ce > blen {
                    self.want(ce);
                }
            }
            b"iinf" => {
                let ver = self.buf.get(cs as usize).copied().unwrap_or(0);
                let skip = if ver == 0 { 6 } else { 8 };
                self.children(cs + skip, end, depth + 1, false);
            }
            b"infe" => self.infe(cs, end),
            b"iloc" => self.iloc(cs, end),
            b"ispe" => {
                if let (Some(w), Some(h)) = (be32(self.buf, cs + 4), be32(self.buf, cs + 8)) {
                    if (w as u64) * (h as u64) > (self.ispe.0 as u64) * (self.ispe.1 as u64) {
                        self.ispe = (w, h);
                    }
                }
            }
            b"stsd" => self.children(cs + 8, end, depth + 1, false),
            b"CRAW" | b"avc1" | b"hvc1" | b"hev1" | b"mp4v" | b"av01" | b"jpeg" | b"apcn" | b"apch" | b"ap4h" => {
                if let (Some(w), Some(h)) = (be16(self.buf, cs + 24), be16(self.buf, cs + 26)) {
                    if (w as u64) * (h as u64) > (self.sample.0 as u64) * (self.sample.1 as u64) {
                        self.sample = (w as u32, h as u32);
                    }
                }
            }
            b"mvhd" => {
                let ver = self.buf.get(cs as usize).copied().unwrap_or(0);
                let created = if ver == 1 { be64(self.buf, cs + 4) } else { be32(self.buf, cs + 4).map(u64::from) };
                if let Some(c) = created.filter(|c| *c > MAC_EPOCH_TO_UNIX && self.meta.taken_at.is_none()) {
                    self.meta.taken_at = Some((c - MAC_EPOCH_TO_UNIX) as i64);
                    self.meta.tz_offset = Some(0);
                }
            }
            b"tkhd" => {
                let ver = self.buf.get(cs as usize).copied().unwrap_or(0);
                let o = if ver == 1 { 88 } else { 76 };
                if let (Some(w), Some(h)) = (be32(self.buf, cs + o), be32(self.buf, cs + o + 4)) {
                    let (w, h) = (w >> 16, h >> 16);
                    if (w as u64) * (h as u64) > (self.track.0 as u64) * (self.track.1 as u64) {
                        self.track = (w, h);
                    }
                }
            }
            b"uuid" => {
                let Some(id) = self.buf.get(cs as usize..cs as usize + 16) else { return };
                if id == CANON_MOOV {
                    self.children(cs + 16, end, depth + 1, false);
                } else if id == CANON_PRVW {
                    // an 8 byte header, then the PRVW box; some files omit it
                    for skip in [24u64, 16] {
                        if self.buf.get((cs + skip + 4) as usize..(cs + skip + 8) as usize) == Some(b"PRVW") {
                            self.children(cs + skip, end, depth + 1, false);
                            break;
                        }
                    }
                }
            }
            b"PRVW" => {
                // version/flags, 1, width, height, 1, JPEG size, then the JPEG
                if let (Some(w), Some(h), Some(len)) = (be16(self.buf, cs + 6), be16(self.buf, cs + 8), be32(self.buf, cs + 12)) {
                    let (off, len) = (cs + 16, len as u64);
                    if len > 0 && off + len <= ce.min(self.file_size) && !self.prvw {
                        self.prvw = true;
                        self.meta.preview = Some(Span { offset: off, len, width: Some(w as u32), height: Some(h as u32) });
                    }
                }
            }
            b"CMT1" | b"CMT2" | b"CMT3" | b"CMT4" => {
                let avail = self.buf.get(cs as usize..end as usize).unwrap_or(&[]);
                if let Some(t) = Tiff::new(avail, ce - cs, cs) {
                    match &typ {
                        b"CMT1" => {
                            self.cmt1 = t.read(self.meta);
                        }
                        b"CMT2" => t.read_exif_root(self.meta),
                        b"CMT4" => t.read_gps_root(self.meta),
                        _ => {}
                    }
                    self.need = self.need.max(t.need().min(self.file_size));
                }
            }
            _ => {}
        }
    }

    fn cstr(&self, from: u64, end: u64) -> (String, u64) {
        let s = from as usize;
        let e = (end as usize).min(self.buf.len());
        let b = self.buf.get(s..e).unwrap_or(&[]);
        let n = b.iter().position(|c| *c == 0).unwrap_or(b.len()).min(256);
        (String::from_utf8_lossy(&b[..n]).into_owned(), from + n as u64 + 1)
    }

    fn infe(&mut self, cs: u64, end: u64) {
        let ver = self.buf.get(cs as usize).copied().unwrap_or(0);
        if ver < 2 {
            return;
        }
        let (id, mut p) = if ver == 2 { (be16(self.buf, cs + 4).map(u32::from), cs + 6) } else { (be32(self.buf, cs + 4), cs + 8) };
        let Some(id) = id else { return };
        p += 2; // protection index
        let Some(t) = self.buf.get(p as usize..p as usize + 4) else { return };
        let typ: [u8; 4] = t.try_into().unwrap();
        let mut item = Item { id, typ, content_type: String::new() };
        if &typ == b"mime" {
            let (_name, next) = self.cstr(p + 4, end);
            item.content_type = self.cstr(next, end).0;
        }
        if self.items.len() < 4096 {
            self.items.push(item);
        }
    }

    fn iloc(&mut self, cs: u64, end: u64) {
        let Some(&ver) = self.buf.get(cs as usize) else { return };
        let (Some(a), Some(b)) = (self.buf.get(cs as usize + 4), self.buf.get(cs as usize + 5)) else { return };
        let (off_size, len_size, base_size, idx_size) = ((a >> 4) as u64, (a & 15) as u64, (b >> 4) as u64, if ver >= 1 { (b & 15) as u64 } else { 0 });
        let mut p = cs + 6;
        let count = if ver < 2 {
            let c = be16(self.buf, p).map(u32::from);
            p += 2;
            c
        } else {
            let c = be32(self.buf, p);
            p += 4;
            c
        };
        let Some(count) = count else { return };
        let uint = |p: u64, n: u64| -> Option<u64> {
            match n {
                0 => Some(0),
                4 => be32(self.buf, p).map(u64::from),
                8 => be64(self.buf, p),
                _ => None,
            }
        };
        for _ in 0..count.min(4096) {
            if p >= end {
                break;
            }
            let id = if ver < 2 { be16(self.buf, p).map(u32::from) } else { be32(self.buf, p) };
            p += if ver < 2 { 2 } else { 4 };
            let mut method = 0;
            if ver >= 1 {
                method = be16(self.buf, p).unwrap_or(0) & 15;
                p += 2;
            }
            p += 2; // data reference index
            let Some(base) = uint(p, base_size) else { return };
            p += base_size;
            let Some(ext) = be16(self.buf, p) else { return };
            p += 2;
            let mut first: Option<(u64, u64)> = None;
            for i in 0..ext.min(64) {
                p += if ver >= 1 { idx_size } else { 0 };
                let (Some(o), Some(l)) = (uint(p, off_size), uint(p + off_size, len_size)) else { return };
                p += off_size + len_size;
                if i == 0 {
                    first = Some((o, l));
                }
            }
            if let (Some(id), Some((o, l)), 0, 1) = (id, first, method, ext) {
                if let Some(off) = base.checked_add(o) {
                    if self.locs.len() < 4096 {
                        self.locs.push(Loc { id, offset: off, len: l });
                    }
                }
            }
        }
    }

    /// After the walk: the Exif and XMP items, the sizes, the video's picture.
    fn finish(&mut self) {
        let is_video = matches!(&self.brand, b"qt  " | b"mp41" | b"mp42" | b"isom" | b"M4V " | b"M4A " | b"3gp4" | b"3gp5" | b"avc1" | b"iso2" | b"iso4" | b"iso5" | b"iso6");
        let find = |ty: &[u8; 4], ct: Option<&str>, items: &[Item], locs: &[Loc]| -> Option<(u64, u64)> {
            let it = items.iter().find(|i| &i.typ == ty && ct.is_none_or(|c| i.content_type == c))?;
            locs.iter().find(|l| l.id == it.id).map(|l| (l.offset, l.len))
        };
        if let Some((off, len)) = find(b"Exif", None, &self.items, &self.locs) {
            if len > 8 && len <= MAX_ITEM {
                self.exif_item(off, len);
            }
        }
        if let Some((off, len)) = find(b"mime", Some("application/rdf+xml"), &self.items, &self.locs) {
            let end = off.saturating_add(len);
            if len <= MAX_ITEM && end <= self.buf.len() as u64 {
                if let Some(x) = catalog_meta_xmp::parse(&self.buf[off as usize..end as usize]) {
                    self.meta.xmp = Some(x);
                }
            }
        }
        if self.ispe.0 > 0 && self.meta.width.is_none() {
            self.meta.width = Some(self.ispe.0);
            self.meta.height = Some(self.ispe.1);
        }
        if &self.brand == b"crx " && self.sample.0 > 0 {
            self.meta.raw_width = Some(self.sample.0);
            self.meta.raw_height = Some(self.sample.1);
            if self.meta.width.is_none() {
                self.meta.width = Some(self.sample.0);
                self.meta.height = Some(self.sample.1);
            }
        }
        if is_video || self.brand == [0; 4] {
            let (w, h) = if self.track.0 > 0 { self.track } else { self.sample };
            if w > 0 && self.meta.width.is_none() {
                self.meta.width = Some(w);
                self.meta.height = Some(h);
            }
        }
    }

    fn exif_item(&mut self, off: u64, len: u64) {
        let blen = self.buf.len() as u64;
        let Some(skip) = be32(self.buf, off) else {
            // the item starts past the window: ask for its start
            self.want(off.saturating_add(len.min(16 * 1024)));
            return;
        };
        let start = off + 4 + skip as u64;
        if skip as u64 + 4 >= len || start >= self.file_size {
            return;
        }
        let avail = self.buf.get(start as usize..(off + len).min(blen) as usize).unwrap_or(&[]);
        match Tiff::new(avail, len - 4 - skip as u64, start) {
            Some(t) => {
                t.read(self.meta);
                self.need = self.need.max(t.need());
            }
            None if start >= blen => self.want(start.saturating_add(16 * 1024).min(off + len)),
            None => {}
        }
        // the thumbnail inside an Exif item is not the file's preview
        self.meta.preview = None;
    }
}
