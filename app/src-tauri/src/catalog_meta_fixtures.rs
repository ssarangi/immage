// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! Byte-exact synthetic photos for the metadata tests: TIFF/EXIF blocks,
//! JPEGs, ARW- and DNG-shaped RAWs, HEIC, CR3, RAF and MP4 built from the
//! published layouts. Nothing here is taken from a real photo.

#[derive(Clone)]
pub enum V {
    Ascii(String),
    Short(Vec<u16>),
    Long(Vec<u32>),
    Rat(Vec<(u32, u32)>),
    SRat(Vec<(i32, i32)>),
    Byte(Vec<u8>),
    Undef(Vec<u8>),
    /// A LONG holding the offset of IFD number n.
    Ptr(usize),
    /// LONGs holding the offsets of several IFDs.
    Ptrs(Vec<usize>),
}

pub struct Ifd {
    pub entries: Vec<(u16, V)>,
    pub next: Option<usize>,
}

fn put16(o: &mut Vec<u8>, le: bool, v: u16) {
    o.extend_from_slice(&if le { v.to_le_bytes() } else { v.to_be_bytes() });
}
fn put32(o: &mut Vec<u8>, le: bool, v: u32) {
    o.extend_from_slice(&if le { v.to_le_bytes() } else { v.to_be_bytes() });
}

fn encode(v: &V, le: bool, offsets: &[u32]) -> (u16, u32, Vec<u8>) {
    let mut b = Vec::new();
    match v {
        V::Ascii(s) => {
            b.extend_from_slice(s.as_bytes());
            b.push(0);
            (2, b.len() as u32, b)
        }
        V::Short(x) => {
            x.iter().for_each(|v| put16(&mut b, le, *v));
            (3, x.len() as u32, b)
        }
        V::Long(x) => {
            x.iter().for_each(|v| put32(&mut b, le, *v));
            (4, x.len() as u32, b)
        }
        V::Rat(x) => {
            x.iter().for_each(|(n, d)| {
                put32(&mut b, le, *n);
                put32(&mut b, le, *d);
            });
            (5, x.len() as u32, b)
        }
        V::SRat(x) => {
            x.iter().for_each(|(n, d)| {
                put32(&mut b, le, *n as u32);
                put32(&mut b, le, *d as u32);
            });
            (10, x.len() as u32, b)
        }
        V::Byte(x) => (1, x.len() as u32, x.clone()),
        V::Undef(x) => (7, x.len() as u32, x.clone()),
        V::Ptr(i) => {
            put32(&mut b, le, offsets[*i]);
            (4, 1, b)
        }
        V::Ptrs(x) => {
            x.iter().for_each(|i| put32(&mut b, le, offsets[*i]));
            (4, x.len() as u32, b)
        }
    }
}

/// A TIFF block (header at 0): IFDs laid out one after another, each followed
/// by its out-of-line values. `magic` is 42 for TIFF, 0x4F52 for ORF, 0x55 for RW2.
pub fn tiff(le: bool, magic: u16, ifds: &[Ifd]) -> Vec<u8> {
    let sizes: Vec<usize> = ifds
        .iter()
        .map(|i| {
            let data: usize = i
                .entries
                .iter()
                .map(|(_, v)| {
                    let (_, _, b) = encode(v, le, &vec![0; ifds.len()]);
                    if b.len() > 4 { b.len() + b.len() % 2 } else { 0 }
                })
                .sum();
            2 + i.entries.len() * 12 + 4 + data
        })
        .collect();
    let mut offsets = Vec::new();
    let mut at = 8u32;
    for s in &sizes {
        offsets.push(at);
        at += *s as u32;
    }
    let mut out = Vec::new();
    out.extend_from_slice(if le { b"II" } else { b"MM" });
    put16(&mut out, le, magic);
    put32(&mut out, le, offsets.first().copied().unwrap_or(8));
    for (n, ifd) in ifds.iter().enumerate() {
        let mut entries = ifd.entries.clone();
        entries.sort_by_key(|(t, _)| *t);
        put16(&mut out, le, entries.len() as u16);
        let mut data_at = offsets[n] as usize + 2 + entries.len() * 12 + 4;
        let mut data = Vec::new();
        for (tag, v) in &entries {
            let (typ, count, mut b) = encode(v, le, &offsets);
            put16(&mut out, le, *tag);
            put16(&mut out, le, typ);
            put32(&mut out, le, count);
            if b.len() <= 4 {
                b.resize(4, 0);
                out.extend_from_slice(&b);
            } else {
                put32(&mut out, le, data_at as u32);
                if b.len() % 2 == 1 {
                    b.push(0);
                }
                data_at += b.len();
                data.extend_from_slice(&b);
            }
        }
        put32(&mut out, le, ifd.next.map_or(0, |i| offsets[i]));
        out.extend_from_slice(&data);
    }
    out
}

pub fn ascii(s: &str) -> V {
    V::Ascii(s.into())
}

/// What a fixture says about the photo.
#[derive(Clone)]
pub struct Spec {
    pub make: &'static str,
    pub model: &'static str,
    pub serial: Option<&'static str>,
    pub lens: Option<&'static str>,
    pub datetime: &'static str,
    pub offset: Option<&'static str>,
    pub subsec: Option<&'static str>,
    pub orientation: u16,
    pub gps: bool,
    pub w: u32,
    pub h: u32,
}

impl Default for Spec {
    fn default() -> Self {
        Self {
            make: "SONY",
            model: "ILCE-7RM3",
            serial: Some("1234567"),
            lens: Some("FE 85mm F1.8"),
            datetime: "2025:10:05 14:08:39",
            offset: Some("-08:00"),
            subsec: Some("12"),
            orientation: 6,
            gps: true,
            w: 7952,
            h: 5304,
        }
    }
}

/// The Exif IFD entries and the GPS IFD entries of a spec.
pub fn exif_entries(s: &Spec) -> Vec<(u16, V)> {
    let mut e = vec![
        (0x829A, V::Rat(vec![(1, 250)])),
        (0x829D, V::Rat(vec![(18, 10)])),
        (0x8827, V::Short(vec![800])),
        (0x9003, ascii(s.datetime)),
        (0x9004, ascii(s.datetime)),
        (0x9204, V::SRat(vec![(-7, 10)])),
        (0x920A, V::Rat(vec![(850, 10)])),
        (0xA405, V::Short(vec![85])),
        (0xA002, V::Long(vec![s.w])),
        (0xA003, V::Long(vec![s.h])),
    ];
    if let Some(o) = s.offset {
        e.push((0x9011, ascii(o)));
    }
    if let Some(o) = s.subsec {
        e.push((0x9291, ascii(o)));
    }
    if let Some(l) = s.lens {
        e.push((0xA434, ascii(l)));
    }
    if let Some(n) = s.serial {
        e.push((0xA431, ascii(n)));
    }
    e
}

pub fn gps_entries() -> Vec<(u16, V)> {
    vec![
        (1, ascii("N")),
        (2, V::Rat(vec![(37, 1), (46, 1), (3000, 100)])),
        (3, ascii("W")),
        (4, V::Rat(vec![(122, 1), (25, 1), (900, 100)])),
        (5, V::Byte(vec![0])),
        (6, V::Rat(vec![(1234, 10)])),
    ]
}

/// IFD0 + Exif + GPS (+ an IFD1 thumbnail of `thumb_len` bytes at `thumb_off`), the layout of a plain TIFF-Exif block.
pub fn exif_block(le: bool, s: &Spec) -> Vec<u8> {
    let mut ifd0 = vec![
        (0x010F, ascii(s.make)),
        (0x0110, ascii(s.model)),
        (0x0112, V::Short(vec![s.orientation])),
        (0x0131, ascii("TestCam v1.0")),
        (0x0132, ascii(s.datetime)),
        (0x8769, V::Ptr(1)),
    ];
    let mut ifds = vec![Ifd { entries: vec![], next: None }, Ifd { entries: exif_entries(s), next: None }];
    if s.gps {
        ifd0.push((0x8825, V::Ptr(2)));
        ifds.push(Ifd { entries: gps_entries(), next: None });
    }
    ifds[0].entries = ifd0;
    tiff(le, 42, &ifds)
}

/// An ARW-shaped file: IFD0 with a JPEG preview (0x201/0x202), the Exif IFD,
/// SubIFD with the raw, IFD1 thumbnail; the preview and raw data are padding
/// out to `file_len`. Returns the bytes and the preview's offset and length.
pub fn arw_like(le: bool, s: &Spec, preview_off: u32, preview_len: u32, file_len: usize) -> Vec<u8> {
    let mut e0 = vec![
        (0x010F, ascii(s.make)),
        (0x0110, ascii(s.model)),
        (0x0112, V::Short(vec![s.orientation])),
        (0x0131, ascii("ILCE-7RM3 v3.10")),
        (0x0103, V::Short(vec![6])),
        (0x0201, V::Long(vec![preview_off])),
        (0x0202, V::Long(vec![preview_len])),
        (0x014A, V::Ptr(3)),
        (0x8769, V::Ptr(1)),
    ];
    let mut ifds = vec![
        Ifd { entries: vec![], next: Some(2) },
        Ifd { entries: exif_entries(s), next: None },
        Ifd { entries: vec![(0x0103, V::Short(vec![6])), (0x0201, V::Long(vec![43388])), (0x0202, V::Long(vec![8718])), (0x010F, ascii(s.make))], next: None },
        Ifd {
            entries: vec![
                (0x00FE, V::Long(vec![0])),
                (0x0100, V::Long(vec![s.w + 48])),
                (0x0101, V::Long(vec![s.h + 16])),
                (0x0103, V::Short(vec![32767])),
                (0x0106, V::Short(vec![32803])),
                (0x0111, V::Long(vec![532_480])),
                (0x0117, V::Long(vec![42_560_000])),
            ],
            next: None,
        },
    ];
    if s.gps {
        e0.push((0x8825, V::Ptr(4)));
        ifds.push(Ifd { entries: gps_entries(), next: None });
    }
    ifds[0].entries = e0;
    let mut out = tiff(le, 42, &ifds);
    if out.len() < file_len {
        out.resize(file_len, 0xAB);
    }
    out
}

/// A JPEG: SOI, APP0, an optional APP1 Exif, an optional APP1 XMP, a padding COM, SOF0, SOS.
pub fn make_jpeg(s: Option<&Spec>, xmp: Option<&str>, w: u16, h: u16, pad_before_sof: usize) -> Vec<u8> {
    let mut o = vec![0xFF, 0xD8];
    let seg = |o: &mut Vec<u8>, m: u8, body: &[u8]| {
        o.extend_from_slice(&[0xFF, m]);
        o.extend_from_slice(&((body.len() + 2) as u16).to_be_bytes());
        o.extend_from_slice(body);
    };
    seg(&mut o, 0xE0, b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
    if let Some(s) = s {
        let mut b = b"Exif\0\0".to_vec();
        b.extend_from_slice(&exif_block(true, s));
        seg(&mut o, 0xE1, &b);
    }
    if let Some(x) = xmp {
        let mut b = b"http://ns.adobe.com/xap/1.0/\0".to_vec();
        b.extend_from_slice(x.as_bytes());
        seg(&mut o, 0xE1, &b);
    }
    let mut left = pad_before_sof;
    while left > 0 {
        let n = left.min(60_000);
        seg(&mut o, 0xFE, &vec![b'x'; n]);
        left -= n;
    }
    let mut sof = vec![8];
    sof.extend_from_slice(&h.to_be_bytes());
    sof.extend_from_slice(&w.to_be_bytes());
    sof.extend_from_slice(&[3, 1, 0x22, 0, 2, 0x11, 1, 3, 0x11, 1]);
    seg(&mut o, 0xC0, &sof);
    seg(&mut o, 0xDA, &[3, 1, 0, 2, 0x11, 3, 0x11, 0, 63, 0]);
    o.extend_from_slice(&[0x12, 0x34, 0x56, 0xFF, 0xD9]);
    o
}

fn bx(typ: &[u8; 4], body: &[u8]) -> Vec<u8> {
    let mut o = ((body.len() + 8) as u32).to_be_bytes().to_vec();
    o.extend_from_slice(typ);
    o.extend_from_slice(body);
    o
}

fn full(typ: &[u8; 4], ver: u8, body: &[u8]) -> Vec<u8> {
    let mut b = vec![ver, 0, 0, 0];
    b.extend_from_slice(body);
    bx(typ, &b)
}

/// A HEIC: ftyp, meta (hdlr, pitm, iinf with an Exif and an XMP item, iloc, iprp/ipco/ispe), mdat with the items.
/// `exif_at_end` puts the Exif item `gap` bytes into the mdat, to put it past a small window.
pub fn make_heic(s: &Spec, xmp: Option<&str>, gap: usize) -> Vec<u8> {
    let exif = {
        let mut e = 6u32.to_be_bytes().to_vec();
        e.extend_from_slice(b"Exif\0\0");
        e.extend_from_slice(&exif_block(false, s));
        e
    };
    let xmp_b = xmp.map(|x| x.as_bytes().to_vec());
    let infe = |id: u16, ty: &[u8; 4], ct: Option<&str>| {
        let mut b = id.to_be_bytes().to_vec();
        b.extend_from_slice(&[0, 0]);
        b.extend_from_slice(ty);
        b.push(0);
        if let Some(c) = ct {
            b.extend_from_slice(c.as_bytes());
            b.push(0);
        }
        full(b"infe", 2, &b)
    };
    let mut iinf = (if xmp_b.is_some() { 3u16 } else { 2u16 }).to_be_bytes().to_vec();
    iinf.extend(infe(1, b"hvc1", None));
    iinf.extend(infe(2, b"Exif", None));
    if xmp_b.is_some() {
        iinf.extend(infe(3, b"mime", Some("application/rdf+xml")));
    }
    let ispe = full(b"ispe", 0, &[s.w.to_be_bytes(), s.h.to_be_bytes()].concat());
    let ispe_small = full(b"ispe", 0, &[160u32.to_be_bytes(), 120u32.to_be_bytes()].concat());
    let ipco = bx(b"ipco", &[ispe_small, ispe].concat());
    let iprp = bx(b"iprp", &ipco);
    let hdlr = full(b"hdlr", 0, &[0, 0, 0, 0, b'p', b'i', b'c', b't', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    let pitm = full(b"pitm", 0, &1u16.to_be_bytes());
    // iloc is version 1: offset size 4, length size 4, base offset size 0, index size 0
    let n_items = if xmp_b.is_some() { 3 } else { 2 };
    let iinf_box = full(b"iinf", 0, &iinf);
    let build = |img_off: usize, exif_off: usize, xmp_off: usize| {
        let item = |id: u16, off: usize, len: usize| {
            let mut b = id.to_be_bytes().to_vec();
            b.extend_from_slice(&[0, 0, 0, 0, 0, 1]);
            b.extend_from_slice(&(off as u32).to_be_bytes());
            b.extend_from_slice(&(len as u32).to_be_bytes());
            b
        };
        let mut iloc = vec![0x44, 0x00];
        iloc.extend_from_slice(&(n_items as u16).to_be_bytes());
        iloc.extend(item(1, img_off, 16));
        iloc.extend(item(2, exif_off, exif.len()));
        if let Some(x) = &xmp_b {
            iloc.extend(item(3, xmp_off, x.len()));
        }
        let iloc_box = full(b"iloc", 1, &iloc);
        full(b"meta", 0, &[hdlr.clone(), pitm.clone(), iinf_box.clone(), iloc_box, iprp.clone()].concat())
    };
    let meta_len = build(0, 0, 0).len();
    let img_off = 20 + meta_len + 8;
    let exif_off = img_off + 16 + gap;
    let meta = build(img_off, exif_off, exif_off + exif.len());
    let mut ftyp = b"heic".to_vec();
    ftyp.extend_from_slice(&[0, 0, 0, 0]);
    ftyp.extend_from_slice(b"mif1");
    let ftyp = bx(b"ftyp", &ftyp);
    let mut mdat = vec![0u8; 16 + gap];
    mdat.extend_from_slice(&exif);
    if let Some(x) = &xmp_b {
        mdat.extend_from_slice(x);
    }
    let mut out = ftyp;
    out.extend(meta);
    out.extend(bx(b"mdat", &mdat));
    out
}

/// A CR3: ftyp crx, moov with Canon's uuid (CMT1, CMT2, CMT4) and a CRAW track, a PRVW uuid, mdat.
pub fn make_cr3(s: &Spec, prvw_len: u32) -> (Vec<u8>, u64) {
    let cmt1 = tiff(true, 42, &[Ifd { entries: vec![(0x010F, ascii(s.make)), (0x0110, ascii(s.model)), (0x0112, V::Short(vec![s.orientation])), (0x0132, ascii(s.datetime))], next: None }]);
    let cmt2 = tiff(true, 42, &[Ifd { entries: exif_entries(s), next: None }]);
    let cmt4 = tiff(true, 42, &[Ifd { entries: gps_entries(), next: None }]);
    let canon_id = [0x85, 0xc0, 0xb6, 0x87, 0x82, 0x0f, 0x11, 0xe0, 0x81, 0x11, 0xf4, 0xce, 0x46, 0x2b, 0x6a, 0x48];
    let mut cu = canon_id.to_vec();
    cu.extend(bx(b"CNCV", b"Canon CR3 test"));
    cu.extend(bx(b"CMT1", &cmt1));
    cu.extend(bx(b"CMT2", &cmt2));
    cu.extend(bx(b"CMT4", &cmt4));
    let mut craw = vec![0u8; 24];
    craw[6..8].copy_from_slice(&1u16.to_be_bytes());
    craw.extend_from_slice(&(s.w as u16).to_be_bytes());
    craw.extend_from_slice(&(s.h as u16).to_be_bytes());
    craw.extend_from_slice(&[0u8; 20]);
    let stsd = full(b"stsd", 0, &[1u32.to_be_bytes().to_vec(), bx(b"CRAW", &craw)].concat());
    let trak = bx(b"trak", &bx(b"mdia", &bx(b"minf", &bx(b"stbl", &stsd))));
    let moov = bx(b"moov", &[bx(b"uuid", &cu), trak].concat());
    let mut ftyp = b"crx ".to_vec();
    ftyp.extend_from_slice(&[0, 0, 0, 1]);
    ftyp.extend_from_slice(b"crx isom");
    let ftyp = bx(b"ftyp", &ftyp);
    let prvw_id = [0xea, 0xf4, 0x2b, 0x5e, 0x1c, 0x98, 0x4b, 0x88, 0xb9, 0xfb, 0xb7, 0xdc, 0x40, 0x6e, 0x4d, 0x16];
    let mut prvw = vec![0, 0, 0, 0, 0, 1, 0x06, 0x54, 0x04, 0x38, 0, 1];
    prvw.extend_from_slice(&prvw_len.to_be_bytes());
    prvw.extend(vec![0xEE; prvw_len as usize]);
    let mut pu = prvw_id.to_vec();
    pu.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 1]);
    pu.extend(bx(b"PRVW", &prvw));
    let mut out = ftyp;
    out.extend(moov);
    let prvw_box_at = out.len();
    out.extend(bx(b"uuid", &pu));
    out.extend(bx(b"mdat", &[0u8; 64]));
    // the JPEG starts 16 bytes into PRVW's payload: uuid header 8 + 16 + 8 pre + PRVW header 8
    (out, (prvw_box_at + 8 + 16 + 8 + 8 + 16) as u64)
}

/// A Fujifilm RAF: the 16 byte magic, a camera name, the JPEG offset and length at 84/88, then the JPEG.
pub fn make_raf(s: &Spec, jpeg_pad: usize, trailer: usize) -> Vec<u8> {
    let jpeg = make_jpeg(Some(s), None, 6240, 4160, jpeg_pad);
    let mut h = vec![0u8; 148];
    h[..16].copy_from_slice(b"FUJIFILMCCD-RAW ");
    h[16..20].copy_from_slice(b"0201");
    h[28..28 + 6].copy_from_slice(b"X-T4\0\0");
    h[84..88].copy_from_slice(&148u32.to_be_bytes());
    h[88..92].copy_from_slice(&(jpeg.len() as u32).to_be_bytes());
    h.extend(jpeg);
    h.resize(h.len() + trailer, 0);
    h
}

/// An MP4: ftyp isom, moov with mvhd (creation time, seconds since 1904) and a track.
pub fn make_mp4(creation_1904: u32, w: u16, h: u16) -> Vec<u8> {
    let mut mvhd = Vec::new();
    mvhd.extend_from_slice(&creation_1904.to_be_bytes());
    mvhd.extend_from_slice(&creation_1904.to_be_bytes());
    mvhd.extend_from_slice(&1000u32.to_be_bytes());
    mvhd.extend_from_slice(&5000u32.to_be_bytes());
    mvhd.extend(vec![0u8; 80]);
    let mut tk = vec![0u8; 76 - 4];
    tk.extend_from_slice(&((w as u32) << 16).to_be_bytes());
    tk.extend_from_slice(&((h as u32) << 16).to_be_bytes());
    let tkhd = full(b"tkhd", 0, &tk);
    let moov = bx(b"moov", &[full(b"mvhd", 0, &mvhd), bx(b"trak", &tkhd)].concat());
    let mut ftyp = b"isom".to_vec();
    ftyp.extend_from_slice(&[0, 0, 2, 0]);
    ftyp.extend_from_slice(b"isomiso2mp41");
    let mut out = bx(b"ftyp", &ftyp);
    out.extend(moov);
    out.extend(bx(b"mdat", &[0u8; 32]));
    out
}

pub const DARKTABLE_XMP: &str = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/" x:xmptk="XMP Core 4.4.0-Exiv2">
 <rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
  <rdf:Description rdf:about=""
    xmlns:xmp="http://ns.adobe.com/xap/1.0/"
    xmlns:dc="http://purl.org/dc/elements/1.1/"
    xmlns:darktable="http://darktable.sf.net/"
   xmp:Rating="4"
   darktable:history_end="3"
   darktable:xmp_version="5">
   <dc:subject>
    <rdf:Bag>
     <rdf:li>portrait</rdf:li>
     <rdf:li>Alice &amp; Bob</rdf:li>
    </rdf:Bag>
   </dc:subject>
   <darktable:colorlabels>
    <rdf:Seq>
     <rdf:li>0</rdf:li>
     <rdf:li>2</rdf:li>
    </rdf:Seq>
   </darktable:colorlabels>
   <darktable:history>
    <rdf:Seq>
     <rdf:li darktable:num="0" darktable:operation="rawprepare" darktable:enabled="1"/>
     <rdf:li darktable:num="1" darktable:operation="temperature" darktable:enabled="1"/>
     <rdf:li darktable:num="2" darktable:operation="colorbalancergb" darktable:enabled="1"/>
     <rdf:li darktable:num="3" darktable:operation="vignette" darktable:enabled="1"/>
    </rdf:Seq>
   </darktable:history>
  </rdf:Description>
 </rdf:RDF>
</x:xmpmeta>
<?xpacket end="w"?>"#;
