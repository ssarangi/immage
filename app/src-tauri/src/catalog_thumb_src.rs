// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! What the thumbnail phase does with bytes it has already read: find the
//! Exif thumbnail of a JPEG head, decode a JPEG, resize it, turn it upright by
//! the photo's EXIF orientation and encode the small JPEG that is stored.
//! Nothing here touches a file system; the job (`catalog_thumb_job`) decides
//! which bytes to read.
//!
//! Scaled DCT decoding (1/8, 1/4) would let a large JPEG be reduced without
//! materialising it, but no crate in the dependency tree offers it (`image`
//! uses zune-jpeg, which always decodes full size), so a JPEG without a usable
//! Exif thumbnail is decoded whole and then box-filtered. That is why the job
//! has a policy for reading whole JPEGs.

use image::{DynamicImage, ImageFormat};
use std::io::Cursor;

/// Where a thumbnail's pixels came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Src {
    /// A RAW's (or RAF's, CR3's) embedded preview JPEG, one ranged read.
    Preview,
    /// The Exif thumbnail of a JPEG, read from the head.
    Exif,
    /// The JPEG itself, read whole.
    Jpeg,
    /// The engine's own thumbnail (a hook the app provides).
    Engine,
}

impl Src {
    pub fn word(self) -> &'static str {
        match self {
            Src::Preview => "preview",
            Src::Exif => "exif",
            Src::Jpeg => "jpeg",
            Src::Engine => "engine",
        }
    }
}

/// A thumbnail inside a JPEG head: where it is (in the file) and how big.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExifThumb {
    pub offset: usize,
    pub len: usize,
    pub width: u32,
    pub height: u32,
}

fn rd16(b: &[u8], at: usize, le: bool) -> Option<u16> {
    let s = b.get(at..at + 2)?;
    Some(if le { u16::from_le_bytes([s[0], s[1]]) } else { u16::from_be_bytes([s[0], s[1]]) })
}

fn rd32(b: &[u8], at: usize, le: bool) -> Option<u32> {
    let s = b.get(at..at + 4)?;
    Some(if le { u32::from_le_bytes([s[0], s[1], s[2], s[3]]) } else { u32::from_be_bytes([s[0], s[1], s[2], s[3]]) })
}

/// The size of a JPEG from its first start-of-frame marker.
pub fn jpeg_dimensions(b: &[u8]) -> Option<(u32, u32)> {
    if !b.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut at = 2;
    while at + 4 <= b.len() {
        if b[at] != 0xFF {
            return None;
        }
        let m = b[at + 1];
        if m == 0xFF {
            at += 1;
            continue;
        }
        if m == 0xD8 || (0xD0..=0xD7).contains(&m) || m == 0x01 {
            at += 2;
            continue;
        }
        if m == 0xD9 || m == 0xDA {
            return None;
        }
        let len = rd16(b, at + 2, false)? as usize;
        if matches!(m, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) {
            let h = rd16(b, at + 5, false)? as u32;
            let w = rd16(b, at + 7, false)? as u32;
            return (w > 0 && h > 0).then_some((w, h));
        }
        at += 2 + len;
    }
    None
}

/// The Exif thumbnail (IFD1's JPEGInterchangeFormat) of a JPEG whose first bytes
/// are `head`, only when all of it lies inside `head`.
pub fn exif_thumbnail(head: &[u8]) -> Option<ExifThumb> {
    if !head.starts_with(&[0xFF, 0xD8]) {
        return None;
    }
    let mut at = 2;
    while at + 4 <= head.len() {
        if head[at] != 0xFF {
            return None;
        }
        let m = head[at + 1];
        if m == 0xFF {
            at += 1;
            continue;
        }
        if m == 0xD8 || (0xD0..=0xD7).contains(&m) || m == 0x01 {
            at += 2;
            continue;
        }
        if m == 0xD9 || m == 0xDA {
            return None;
        }
        let len = rd16(head, at + 2, false)? as usize;
        if m == 0xE1 && head.get(at + 4..at + 10) == Some(b"Exif\0\0") {
            return thumb_in_tiff(head, at + 10);
        }
        at += 2 + len;
    }
    None
}

fn thumb_in_tiff(head: &[u8], t: usize) -> Option<ExifThumb> {
    let le = match head.get(t..t + 2)? {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    if rd16(head, t + 2, le)? != 42 {
        return None;
    }
    let ifd0 = t + rd32(head, t + 4, le)? as usize;
    let n0 = rd16(head, ifd0, le)? as usize;
    let next = rd32(head, ifd0 + 2 + 12 * n0, le)? as usize;
    if next == 0 {
        return None;
    }
    let ifd1 = t + next;
    let n1 = (rd16(head, ifd1, le)? as usize).min(64);
    let (mut off, mut len) = (None, None);
    for i in 0..n1 {
        let e = ifd1 + 2 + 12 * i;
        let tag = rd16(head, e, le)?;
        let typ = rd16(head, e + 2, le)?;
        let val = if typ == 3 { rd16(head, e + 8, le)? as u32 } else { rd32(head, e + 8, le)? };
        match tag {
            0x0201 => off = Some(val as usize),
            0x0202 => len = Some(val as usize),
            _ => {}
        }
    }
    let (off, len) = (t + off?, len?);
    if len < 128 || off.checked_add(len)? > head.len() {
        return None;
    }
    let bytes = &head[off..off + len];
    let (width, height) = jpeg_dimensions(bytes)?;
    Some(ExifThumb { offset: off, len, width, height })
}

/// Applies an EXIF orientation (1 to 8) to an image that is stored as the
/// sensor wrote it.
pub fn orient(img: DynamicImage, o: u8) -> DynamicImage {
    match o {
        2 => img.fliph(),
        3 => img.rotate180(),
        4 => img.flipv(),
        5 => img.rotate90().fliph(),
        6 => img.rotate90(),
        7 => img.rotate270().fliph(),
        8 => img.rotate270(),
        _ => img,
    }
}

/// How a thumbnail is made.
#[derive(Debug, Clone, Copy)]
pub struct Render {
    /// The long edge of the stored thumbnail.
    pub edge: u32,
    pub quality: u8,
    /// The photo's EXIF orientation (1 when unknown).
    pub orientation: u8,
    /// The size of the full photo as the catalog knows it (Exif size), to notice
    /// a preview or thumbnail a camera already turned upright.
    pub main: Option<(u32, u32)>,
    /// Whether the pixels are the photo's own (a whole JPEG) rather than a small
    /// embedded copy that may be stored upright already.
    pub own_pixels: bool,
}

#[derive(Debug, Clone)]
pub struct Thumb {
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// An embedded copy is already upright when the photo says it is rotated a
/// quarter turn but the copy has the shape of the rotated photo. Some cameras and
/// phones store their thumbnails that way.
fn effective_orientation(r: &Render, w: u32, h: u32) -> u8 {
    let o = r.orientation.clamp(1, 8);
    if r.own_pixels || o < 5 || w == h {
        return o;
    }
    let Some((mw, mh)) = r.main else { return o };
    if mw == mh {
        return o;
    }
    // the shape the copy should have after the orientation: the photo's, turned
    let upright_wide = mh > mw;
    if (w > h) == upright_wide { 1 } else { o }
}

/// Decodes a JPEG, fits it in `edge` on the long side, turns it upright and
/// encodes the result.
pub fn render(bytes: &[u8], r: &Render) -> Result<Thumb, String> {
    let mut reader = image::ImageReader::with_format(Cursor::new(bytes), ImageFormat::Jpeg);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(1 << 30);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| format!("decode: {e}"))?;
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return Err("decode: empty image".into());
    }
    let img = if w.max(h) > r.edge { img.thumbnail(r.edge, r.edge) } else { img };
    let img = orient(img, effective_orientation(r, w, h));
    let rgb = img.to_rgb8();
    let (width, height) = (rgb.width(), rgb.height());
    let mut jpeg = Vec::with_capacity(32 * 1024);
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, r.quality)
        .encode(rgb.as_raw(), width, height, image::ExtendedColorType::Rgb8)
        .map_err(|e| format!("encode: {e}"))?;
    Ok(Thumb { jpeg, width, height })
}
