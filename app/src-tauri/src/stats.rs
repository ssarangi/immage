// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

//! Statistics: pure aggregation over the photos of the search cache.
//!
//! `parse_row` turns one cache row (its `metadata_json` plus the capture
//! columns) into a `Parsed` photo, `Index::build` interns those into compact
//! `Row`s, and `snapshot` / `top_page` / `place_detail` / `matching` answer a
//! request from an index. Nothing here touches SQLite or Tauri, so every
//! bucket, filter and cell is unit-tested below. stats_cmds.rs reads the
//! cache, keeps the index, and attaches session handles to photos.
//!
//! Cross-filtering: one pass computes for every photo the set of filter
//! dimensions it fails (a bit mask). An aggregate that owns dimensions D counts
//! a photo when it fails nothing outside D, so a chart still shows the
//! alternatives to its own selection.

use crate::typed_stats::*;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

pub const TOP_PAGE: usize = 24;
pub const PLACE_CAP: usize = 1500;
pub const HEAT_CAP: usize = 2000;
pub const LIST_CAP: usize = 40;
// photos a lens or body needs before an insight calls its keeper rate better
pub const INSIGHT_MIN_N: u32 = 30;
const QZ: u32 = 22; // the finest quadkey level kept per photo
const FINE: u32 = 18; // "distinct places" are counted at this level
const NO_TIME: i64 = i64::MIN;

// ---------------------------------------------------------------- bucket definitions

pub fn focal_defs() -> Vec<FocalDef> {
    let d = |id, label: &str, from_mm, to_mm| FocalDef { id, label: label.into(), from_mm, to_mm };
    vec![
        d(FocalBucketId::Uw, "Ultra-wide", 0.0, Some(20.0)),
        d(FocalBucketId::Wide, "Wide", 21.0, Some(34.0)),
        d(FocalBucketId::Normal, "Normal", 35.0, Some(69.0)),
        d(FocalBucketId::Portrait, "Portrait", 70.0, Some(135.0)),
        d(FocalBucketId::Tele, "Telephoto", 136.0, Some(300.0)),
        d(FocalBucketId::Super, "Super-tele", 301.0, None),
    ]
}
const FOCAL_IDS: [FocalBucketId; 6] = [
    FocalBucketId::Uw,
    FocalBucketId::Wide,
    FocalBucketId::Normal,
    FocalBucketId::Portrait,
    FocalBucketId::Tele,
    FocalBucketId::Super,
];

pub fn aperture_defs() -> Vec<ApertureDef> {
    let d = |id, label: &str, from_f, to_f, name: &str| ApertureDef {
        id,
        label: label.into(),
        from_f,
        to_f,
        name: name.into(),
    };
    vec![
        d(ApertureGroupId::A1, "f/1.2-1.8", 0.0, Some(1.8), "Wide open"),
        d(ApertureGroupId::A2, "f/2-2.8", 1.81, Some(2.8), "Fast"),
        d(ApertureGroupId::A3, "f/4-5.6", 2.81, Some(5.6), "Mid"),
        d(ApertureGroupId::A4, "f/8-11", 5.61, Some(11.0), "Landscape"),
        d(ApertureGroupId::A5, "f/16+", 11.01, None, "Stopped down"),
    ]
}
const APERTURE_IDS: [ApertureGroupId; 5] = [
    ApertureGroupId::A1,
    ApertureGroupId::A2,
    ApertureGroupId::A3,
    ApertureGroupId::A4,
    ApertureGroupId::A5,
];

pub fn shutter_defs() -> Vec<ShutterDef> {
    let d = |id, label: &str, from, to, caption: &str| ShutterDef {
        id,
        label: label.into(),
        from,
        to,
        caption: caption.into(),
    };
    vec![
        d(ShutterRegimeId::Freeze, "Freeze", 0.0, Some(1.0 / 750.0), ">= 1/1000"),
        d(ShutterRegimeId::Fast, "Fast", 1.0 / 750.0, Some(1.0 / 180.0), "1/250-1/500"),
        d(ShutterRegimeId::Handheld, "Handheld", 1.0 / 180.0, Some(1.0 / 45.0), "1/60-1/200"),
        d(ShutterRegimeId::Slow, "Slow", 1.0 / 45.0, Some(1.0 / 6.0), "1/15-1/50"),
        d(ShutterRegimeId::Motion, "Motion", 1.0 / 6.0, Some(1.0001), "1/8-1 s"),
        d(ShutterRegimeId::Long, "Long", 1.0001, None, "> 1 s"),
    ]
}
const SHUTTER_IDS: [ShutterRegimeId; 6] = [
    ShutterRegimeId::Freeze,
    ShutterRegimeId::Fast,
    ShutterRegimeId::Handheld,
    ShutterRegimeId::Slow,
    ShutterRegimeId::Motion,
    ShutterRegimeId::Long,
];
const ISO_IDS: [IsoBinId; 7] = [
    IsoBinId::I100,
    IsoBinId::I200,
    IsoBinId::I400,
    IsoBinId::I800,
    IsoBinId::I1600,
    IsoBinId::I3200,
    IsoBinId::I6400Plus,
];
const ASPECT_IDS: [AspectClassId; 8] = [
    AspectClassId::R16x9,
    AspectClassId::R3x2,
    AspectClassId::R4x3,
    AspectClassId::R1x1,
    AspectClassId::R5x4,
    AspectClassId::Portrait,
    AspectClassId::Pano,
    AspectClassId::Other,
];
const LABEL_IDS: [ColorLabel; 5] = [
    ColorLabel::Red,
    ColorLabel::Yellow,
    ColorLabel::Green,
    ColorLabel::Blue,
    ColorLabel::Purple,
];

pub fn defs() -> Defs {
    Defs { focal: focal_defs(), aperture: aperture_defs(), shutter: shutter_defs() }
}

const UNKNOWN: u8 = u8::MAX;

/// The focal bucket of a rounded 35 mm equivalent focal length (0 = unknown).
pub fn focal_bucket(mm: u32) -> u8 {
    match mm {
        0 => UNKNOWN,
        1..=20 => 0,
        21..=34 => 1,
        35..=69 => 2,
        70..=135 => 3,
        136..=300 => 4,
        _ => 5,
    }
}
/// Aperture group of an f-number: each group includes its upper edge.
pub fn aperture_group(f: f32) -> u8 {
    if f <= 0.0 {
        UNKNOWN
    } else if f <= 1.8 {
        0
    } else if f <= 2.8 {
        1
    } else if f <= 5.6 {
        2
    } else if f <= 11.0 {
        3
    } else {
        4
    }
}
/// Shutter regime of an exposure time in seconds: half-open [from, to).
pub fn shutter_regime(t: f32) -> u8 {
    let t = f64::from(t);
    if t <= 0.0 {
        UNKNOWN
    } else if t < 1.0 / 750.0 {
        0
    } else if t < 1.0 / 180.0 {
        1
    } else if t < 1.0 / 45.0 {
        2
    } else if t < 1.0 / 6.0 {
        3
    } else if t < 1.0001 {
        4
    } else {
        5
    }
}
/// ISO bin: the nearest stop at or below, 6400 and up pooled.
pub fn iso_bin(iso: f32) -> u8 {
    if iso <= 0.0 {
        UNKNOWN
    } else if iso < 150.0 {
        0
    } else if iso < 300.0 {
        1
    } else if iso < 600.0 {
        2
    } else if iso < 1200.0 {
        3
    } else if iso < 2400.0 {
        4
    } else if iso < 4800.0 {
        5
    } else {
        6
    }
}
/// Aspect class from the orientation-corrected long/short ratio, within 2 %.
pub fn aspect_class(w: u32, h: u32) -> u8 {
    if w == 0 || h == 0 {
        return UNKNOWN;
    }
    let r = f64::from(w.max(h)) / f64::from(w.min(h));
    let near = |t: f64| (r - t).abs() / t <= 0.02 + 1e-9;
    if near(1.0) {
        3
    } else if near(5.0 / 4.0) {
        4
    } else if near(4.0 / 3.0) {
        2
    } else if near(1.5) {
        1
    } else if near(16.0 / 9.0) {
        0
    } else if r > 2.0 {
        6
    } else {
        7
    }
}

// ---------------------------------------------------------------- names

/// A camera maker as one spelling: "SONY" and "Sony" are one maker.
pub fn norm_maker(raw: &str) -> String {
    let s = raw.trim();
    let upper = s.to_ascii_uppercase();
    let known = [
        ("SONY", "Sony"),
        ("NIKON", "Nikon"),
        ("CANON", "Canon"),
        ("FUJIFILM", "Fujifilm"),
        ("OLYMPUS", "Olympus"),
        ("OM DIGITAL", "OM System"),
        ("PANASONIC", "Panasonic"),
        ("LEICA", "Leica"),
        ("PENTAX", "Pentax"),
        ("RICOH", "Ricoh"),
        ("HASSELBLAD", "Hasselblad"),
        ("SIGMA", "Sigma"),
        ("SAMSUNG", "Samsung"),
        ("GOPRO", "GoPro"),
        ("DJI", "DJI"),
        ("APPLE", "Apple"),
        ("GOOGLE", "Google"),
        ("XIAOMI", "Xiaomi"),
        ("HUAWEI", "Huawei"),
    ];
    for (prefix, name) in known {
        if upper == prefix || upper.starts_with(&format!("{prefix} ")) {
            return name.to_string();
        }
    }
    if s.len() > 3 && s == upper {
        // an unknown maker in capitals: "ACME CORP" -> "Acme Corp"
        return s
            .split_whitespace()
            .map(|w| {
                let mut c = w.chars();
                c.next().map_or(String::new(), |f| f.to_uppercase().collect::<String>() + &c.as_str().to_lowercase())
            })
            .collect::<Vec<_>>()
            .join(" ");
    }
    s.to_string()
}
/// A lens name with its whitespace tidied.
pub fn norm_lens(raw: &str) -> String {
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}
fn body_label(maker: &str, model: &str) -> String {
    let m = maker.to_ascii_lowercase();
    if model.to_ascii_lowercase().starts_with(&m) {
        model.to_string()
    } else {
        format!("{maker} {model}")
    }
}
pub fn file_type(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.rsplit_once('.').map(|(_, e)| e.to_ascii_uppercase()).unwrap_or_default()
}
fn parent_dir(path: &str) -> &str {
    path.rsplit_once(['/', '\\']).map_or("", |(d, _)| if d.is_empty() { "/" } else { d })
}
fn under(dir: &str, root: &str) -> bool {
    let root = root.trim_end_matches(['/', '\\']);
    root.is_empty() || dir == root || (dir.starts_with(root) && matches!(dir.as_bytes().get(root.len()), Some(b'/' | b'\\')))
}

// ---------------------------------------------------------------- dates

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}
/// "YYYY:MM:DD HH:MM:SS[.f]" (or with dashes, or a T) as local seconds since 1970.
pub fn parse_local(s: &str) -> Option<i64> {
    let b = s.trim().as_bytes();
    if b.len() < 19 {
        return None;
    }
    let num = |a: usize, z: usize| -> Option<i64> {
        let t = std::str::from_utf8(b.get(a..z)?).ok()?;
        if t.bytes().all(|c| c.is_ascii_digit()) { t.parse().ok() } else { None }
    };
    let (y, mo, d, h, mi, sec) = (num(0, 4)?, num(5, 7)?, num(8, 10)?, num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1900..=2200).contains(&y) || !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || sec > 60 {
        return None;
    }
    // a day that does not exist (February 30) is not a capture time
    let days = days_from_civil(y, mo, d);
    if civil_from_days(days) != (y, mo, d) {
        return None;
    }
    Some(days * 86_400 + h * 3600 + mi * 60 + sec.min(59))
}
struct Civil {
    year: i32,
    month: u8,
    day: u8,
    hour: u8,
    weekday: u8,
}
fn civil(t: i64) -> Civil {
    let days = t.div_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    Civil {
        year: y as i32,
        month: m as u8,
        day: d as u8,
        hour: (t.rem_euclid(86_400) / 3600) as u8,
        // 1970-01-01 was a Thursday; Monday = 0
        weekday: ((days + 3).rem_euclid(7)) as u8,
    }
}
fn fmt_date(t: i64) -> String {
    let c = civil(t);
    format!("{:04}-{:02}-{:02}", c.year, c.month, c.day)
}
fn fmt_iso(t: i64) -> String {
    let c = civil(t);
    let s = t.rem_euclid(86_400);
    format!("{:04}-{:02}-{:02}T{:02}:{:02}:{:02}", c.year, c.month, c.day, s / 3600, s % 3600 / 60, s % 60)
}
fn date_key(s: &str) -> Option<i64> {
    parse_local(&format!("{s}T00:00:00")).or_else(|| parse_local(&format!("{} 00:00:00", s.replace('-', ":"))))
}

// ---------------------------------------------------------------- one photo

/// One search-cache row as read from SQLite.
pub struct RawRow {
    pub variant_id: i64,
    pub path: String,
    pub capture_time_ns: Option<i64>,
    pub capture_time_raw: Option<String>,
    pub timezone_offset_seconds: Option<i32>,
    pub metadata: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct MetaImage {
    rating: u8,
    rejected: bool,
    labels: u32,
    edited: bool,
    width: u32,
    height: u32,
    location: Option<MetaLoc>,
    exif: MetaExif,
}
#[derive(Deserialize)]
struct MetaLoc {
    latitude: f64,
    longitude: f64,
}
#[derive(Deserialize, Default)]
#[serde(default)]
struct MetaExif {
    maker: String,
    model: String,
    lens: String,
    body_serial: String,
    exposure: f64,
    aperture: f64,
    iso: f64,
    focal_length: f64,
    crop_factor: f64,
    datetime: String,
    capture_time_raw: String,
}

/// A photo with its fields cleaned, before interning.
#[derive(Clone, Debug, Default)]
pub struct Parsed {
    pub path: String,
    pub maker: String,
    pub model: String,
    pub lens: String,
    pub serial: String,
    pub rating: u8,
    pub rejected: bool,
    pub labels: u8,
    pub edited: bool,
    pub width: u32,
    pub height: u32,
    // 35 mm equivalent when the crop factor is known, else the real length
    pub focal: f32,
    pub aperture: f32,
    pub shutter: f32,
    pub iso: f32,
    // local capture time, seconds since 1970 as if the camera clock were UTC
    pub taken: Option<i64>,
    pub gps: Option<(f64, f64)>,
}

fn pos(v: f64) -> f32 {
    if v.is_finite() && v > 0.0 { v as f32 } else { 0.0 }
}

/// Parse a cache row. None when the row has no metadata yet (not analysed).
///
/// The capture time is the camera's local clock: EXIF `datetime`, else the raw
/// column. Only when both are missing is the stored instant used, shifted by
/// its timezone offset, so a photo is filed under the hour it was taken at its
/// location, not at the viewer's.
pub fn parse_row(raw: &RawRow) -> Option<Parsed> {
    let image: MetaImage = serde_json::from_str(raw.metadata.as_deref()?).ok()?;
    let e = &image.exif;
    let taken = parse_local(&e.datetime)
        .or_else(|| raw.capture_time_raw.as_deref().and_then(parse_local))
        .or_else(|| parse_local(&e.capture_time_raw))
        .or_else(|| {
            let (ns, off) = (raw.capture_time_ns?, raw.timezone_offset_seconds?);
            Some(ns.div_euclid(1_000_000_000) + i64::from(off))
        });
    let focal = pos(e.focal_length);
    let crop = e.crop_factor;
    let focal = if focal > 0.0 && crop.is_finite() && (0.5..=12.0).contains(&crop) { focal * crop as f32 } else { focal };
    let gps = image
        .location
        .filter(|l| l.latitude.is_finite() && l.longitude.is_finite() && l.latitude.abs() <= 90.0 && l.longitude.abs() <= 180.0)
        .filter(|l| l.latitude != 0.0 || l.longitude != 0.0)
        .map(|l| (l.latitude, l.longitude));
    let maker = norm_maker(&e.maker);
    let model = e.model.trim().to_string();
    Some(Parsed {
        path: raw.path.clone(),
        maker,
        model,
        lens: norm_lens(&e.lens),
        serial: e.body_serial.trim().to_string(),
        rating: if image.rejected { 0 } else { image.rating.min(5) },
        rejected: image.rejected,
        labels: (image.labels & 31) as u8,
        edited: image.edited,
        width: image.width,
        height: image.height,
        focal,
        aperture: pos(e.aperture),
        shutter: pos(e.exposure),
        iso: pos(e.iso),
        taken,
        gps,
    })
}

// ---------------------------------------------------------------- the index

#[derive(Clone, Debug)]
pub struct Row {
    pub path: Box<str>,
    pub folder: u32,
    pub ftype: u16,
    pub maker: u16,
    pub body: u16,
    pub lens: u16,
    pub serial: u32,
    pub rating: u8,
    pub rejected: bool,
    pub labels: u8,
    pub edited: bool,
    pub width: u32,
    pub height: u32,
    pub focal: f32,
    pub mm: u32,
    pub aperture: f32,
    pub shutter: f32,
    pub iso: f32,
    pub fb: u8,
    pub ap: u8,
    pub sh: u8,
    pub isb: u8,
    pub asp: u8,
    pub taken: i64,
    pub year: i16,
    pub month: u8,
    #[allow(dead_code)]
    pub day: u8,
    pub hour: u8,
    pub weekday: u8,
    pub lat: f64,
    pub lng: f64,
    pub qx: u32,
    pub qy: u32,
}
const NONE16: u16 = u16::MAX;
const NONE32: u32 = u32::MAX;

impl Row {
    fn has_gps(&self) -> bool {
        self.qx != NONE32
    }
    fn is_top(&self, min: u8) -> bool {
        !self.rejected && self.rating >= min
    }
    fn dated(&self) -> bool {
        self.taken != NO_TIME
    }
}

#[derive(Clone, Debug)]
pub struct Body {
    pub key: String,
    pub maker: u16,
    pub model: String,
    pub label: String,
}

#[derive(Default)]
struct Interner {
    map: HashMap<String, u32>,
    list: Vec<String>,
}
impl Interner {
    fn id(&mut self, s: &str) -> u32 {
        if let Some(&i) = self.map.get(s) {
            return i;
        }
        let i = self.list.len() as u32;
        self.list.push(s.to_string());
        self.map.insert(s.to_string(), i);
        i
    }
}

pub struct Index {
    pub rows: Vec<Row>,
    pub makers: Vec<String>,
    pub bodies: Vec<Body>,
    pub lenses: Vec<String>,
    pub folders: Vec<String>,
    pub ftypes: Vec<String>,
    // photos the cache knows without metadata yet, by folder id
    pub unindexed: Vec<(u32, u32)>,
}

/// Web Mercator tile of a position at `level`.
pub fn tile_xy(lat: f64, lng: f64, level: u32) -> (u32, u32) {
    let n = f64::from(1u32 << level);
    let lat = lat.clamp(-85.051_128_78, 85.051_128_78).to_radians();
    let x = ((lng + 180.0) / 360.0 * n).floor().clamp(0.0, n - 1.0);
    let y = ((1.0 - (lat.tan() + 1.0 / lat.cos()).ln() / std::f64::consts::PI) / 2.0 * n).floor().clamp(0.0, n - 1.0);
    (x as u32, y as u32)
}
/// The quadkey of tile (x, y) at `level`: one digit (0-3) per level.
pub fn quadkey(level: u32, x: u32, y: u32) -> String {
    (1..=level)
        .map(|i| {
            let bit = level - i;
            let d = ((x >> bit) & 1) | (((y >> bit) & 1) << 1);
            char::from(b'0' + d as u8)
        })
        .collect()
}
/// Level, x and y of a quadkey; None for a key with other characters or too deep.
pub fn parse_quadkey(key: &str) -> Option<(u32, u32, u32)> {
    if key.len() > QZ as usize {
        return None;
    }
    let (mut x, mut y) = (0u32, 0u32);
    for c in key.bytes() {
        let d = c.checked_sub(b'0').filter(|d| *d < 4)?;
        x = (x << 1) | u32::from(d & 1);
        y = (y << 1) | u32::from(d >> 1);
    }
    Some((key.len() as u32, x, y))
}
fn tile_center(level: u32, x: u32, y: u32) -> (f64, f64) {
    let n = f64::from(1u32 << level);
    let lng = (f64::from(x) + 0.5) / n * 360.0 - 180.0;
    let t = std::f64::consts::PI * (1.0 - 2.0 * (f64::from(y) + 0.5) / n);
    (t.sinh().atan().to_degrees(), lng)
}

impl Index {
    /// Intern photos into rows. `unindexed` are the paths the cache knows but has no metadata for.
    pub fn build(parsed: Vec<Parsed>, unindexed: &[String]) -> Index {
        let mut makers = Interner::default();
        let mut bodies: Vec<Body> = Vec::new();
        let mut body_ids: HashMap<String, u16> = HashMap::new();
        let mut lenses = Interner::default();
        let mut folders = Interner::default();
        let mut ftypes = Interner::default();
        let mut serials = Interner::default();
        let mut rows = Vec::with_capacity(parsed.len());
        for p in parsed {
            let maker = if p.maker.is_empty() { NONE16 } else { makers.id(&p.maker) as u16 };
            let body = if p.maker.is_empty() || p.model.is_empty() {
                NONE16
            } else {
                let key = format!("{}|{}", p.maker, p.model);
                *body_ids.entry(key.clone()).or_insert_with(|| {
                    bodies.push(Body { key, maker, model: p.model.clone(), label: body_label(&p.maker, &p.model) });
                    (bodies.len() - 1) as u16
                })
            };
            let lens = if p.lens.is_empty() { NONE16 } else { lenses.id(&p.lens) as u16 };
            let serial = if p.serial.is_empty() || body == NONE16 { NONE32 } else { serials.id(&p.serial) };
            let folder = folders.id(parent_dir(&p.path));
            let ftype = ftypes.id(&file_type(&p.path)) as u16;
            let (year, month, day, hour, weekday, taken) = match p.taken {
                Some(t) => {
                    let c = civil(t);
                    (c.year as i16, c.month, c.day, c.hour, c.weekday, t)
                }
                None => (0, 0, 0, UNKNOWN, UNKNOWN, NO_TIME),
            };
            let (lat, lng, qx, qy) = match p.gps {
                Some((la, lo)) => {
                    let (x, y) = tile_xy(la, lo, QZ);
                    (la, lo, x, y)
                }
                None => (f64::NAN, f64::NAN, NONE32, NONE32),
            };
            let mm = if p.focal > 0.0 { (p.focal.round() as u32).max(1) } else { 0 };
            rows.push(Row {
                path: p.path.into_boxed_str(),
                folder,
                ftype,
                maker,
                body,
                lens,
                serial,
                rating: p.rating,
                rejected: p.rejected,
                labels: p.labels,
                edited: p.edited,
                width: p.width,
                height: p.height,
                focal: p.focal,
                mm,
                aperture: p.aperture,
                shutter: p.shutter,
                iso: p.iso,
                fb: focal_bucket(mm),
                ap: aperture_group(p.aperture),
                sh: shutter_regime(p.shutter),
                isb: iso_bin(p.iso),
                asp: aspect_class(p.width, p.height),
                taken,
                year,
                month,
                day,
                hour,
                weekday,
                lat,
                lng,
                qx,
                qy,
            });
        }
        let mut pending: HashMap<u32, u32> = HashMap::new();
        for path in unindexed {
            *pending.entry(folders.id(parent_dir(path))).or_default() += 1;
        }
        let mut unindexed: Vec<(u32, u32)> = pending.into_iter().collect();
        unindexed.sort_unstable();
        Index {
            rows,
            makers: makers.list,
            bodies,
            lenses: lenses.list,
            folders: folders.list,
            ftypes: ftypes.list,
            unindexed,
        }
    }
}

// ---------------------------------------------------------------- scope and filter

/// The folders the library is made of: photos outside the roots or under a hidden folder are not in it.
#[derive(Clone, Debug, Default)]
pub struct Library {
    pub roots: Vec<String>,
    pub hidden: Vec<String>,
}

/// A scope resolved against an index: which folders count, and the date window.
pub struct Scoped {
    pub folder_ok: Vec<bool>,
    pub dates: Option<(i64, i64)>,
}

pub fn resolve_scope(index: &Index, scope: &StatsScope, library: &Library) -> Result<Scoped, String> {
    let in_library = |d: &str| library.roots.iter().any(|r| under(d, r)) && !library.hidden.iter().any(|h| under(d, h));
    let mut dates = None;
    let ok: Vec<bool> = match scope {
        StatsScope::Library => index.folders.iter().map(|d| in_library(d)).collect(),
        StatsScope::Folder { folder, include_subfolders } => index
            .folders
            .iter()
            .map(|d| {
                in_library(d)
                    && if *include_subfolders {
                        under(d, folder)
                    } else {
                        d.trim_end_matches(['/', '\\']) == folder.trim_end_matches(['/', '\\'])
                    }
            })
            .collect(),
        StatsScope::DateRange { from, to } => {
            let f = date_key(from).ok_or("Invalid statistics start date")?;
            let t = date_key(to).ok_or("Invalid statistics end date")?;
            dates = Some((f.min(t), f.max(t) + 86_399));
            index.folders.iter().map(|d| in_library(d)).collect()
        }
    };
    Ok(Scoped { folder_ok: ok, dates })
}

const D_TOP: u32 = 1;
const D_LABEL: u32 = 1 << 1;
const D_MAKER: u32 = 1 << 2;
const D_BODY: u32 = 1 << 3;
const D_LENS: u32 = 1 << 4;
const D_YEAR: u32 = 1 << 5;
const D_MONTH: u32 = 1 << 6;
const D_HOUR: u32 = 1 << 7;
const D_WEEKDAY: u32 = 1 << 8;
const D_FOCAL: u32 = 1 << 9;
const D_APERTURE: u32 = 1 << 10;
const D_SHUTTER: u32 = 1 << 11;
const D_ISO: u32 = 1 << 12;
const D_ASPECT: u32 = 1 << 13;
const D_FTYPE: u32 = 1 << 14;
const D_PLACE: u32 = 1 << 15;
const D_EDITED: u32 = 1 << 16;
const OUT: u32 = 1 << 31; // outside the scope: no aggregate counts it

fn bits<T: Copy>(v: &Option<Vec<T>>, f: impl Fn(T) -> Option<u32>) -> Option<u32> {
    v.as_ref().filter(|v| !v.is_empty()).map(|v| v.iter().filter_map(|x| f(*x)).fold(0, |a, b| a | (1 << b)))
}

struct Compiled {
    min: u8,
    only_top: bool,
    labels: Option<u32>,
    makers: Option<Vec<bool>>,
    bodies: Option<Vec<bool>>,
    lenses: Option<Vec<bool>>,
    ftypes: Option<Vec<bool>>,
    years: Option<Vec<i32>>,
    months: Option<u32>,
    hours: Option<u32>,
    weekdays: Option<u32>,
    focal: Option<u32>,
    aperture: Option<u32>,
    shutter: Option<u32>,
    iso: Option<u32>,
    aspect: Option<u32>,
    place: Option<Vec<(u32, u32, u32)>>,
    edited: Option<bool>,
}

fn name_set(names: &Option<Vec<String>>, list: &[String], norm: impl Fn(&str) -> String) -> Option<Vec<bool>> {
    names.as_ref().filter(|v| !v.is_empty()).map(|v| {
        let wanted: HashSet<String> = v.iter().map(|s| norm(s)).collect();
        list.iter().map(|n| wanted.contains(&norm(n))).collect()
    })
}

fn compile(index: &Index, f: &StatsFilter) -> Compiled {
    let body_keys: Vec<String> = index.bodies.iter().map(|b| b.key.clone()).collect();
    Compiled {
        min: f.min_rating.max(1).min(5),
        only_top: f.only_top,
        labels: bits(&f.labels, |l| LABEL_IDS.iter().position(|x| *x == l).map(|p| p as u32)),
        makers: name_set(&f.makers, &index.makers, |s| s.to_string()),
        bodies: name_set(&f.bodies, &body_keys, |s| s.to_string()),
        lenses: name_set(&f.lenses, &index.lenses, |s| s.to_string()),
        ftypes: name_set(&f.file_types, &index.ftypes, |s| s.to_ascii_uppercase()),
        years: f.years.clone().filter(|v| !v.is_empty()),
        months: bits(&f.months, |m| (1..=12).contains(&m).then_some(u32::from(m))),
        hours: bits(&f.hours, |h| (h < 24).then_some(u32::from(h))),
        weekdays: bits(&f.weekdays, |d| (d < 7).then_some(u32::from(d))),
        focal: bits(&f.focal_buckets, |b| FOCAL_IDS.iter().position(|x| *x == b).map(|p| p as u32)),
        aperture: bits(&f.aperture_groups, |b| APERTURE_IDS.iter().position(|x| *x == b).map(|p| p as u32)),
        shutter: bits(&f.shutter_regimes, |b| SHUTTER_IDS.iter().position(|x| *x == b).map(|p| p as u32)),
        iso: bits(&f.iso_bins, |b| ISO_IDS.iter().position(|x| *x == b).map(|p| p as u32)),
        aspect: bits(&f.aspect, |b| ASPECT_IDS.iter().position(|x| *x == b).map(|p| p as u32)),
        place: f
            .place
            .as_ref()
            .filter(|v| !v.is_empty())
            .map(|v| v.iter().filter_map(|k| parse_quadkey(k)).collect()),
        edited: f.edited,
    }
}

fn set_has(set: &Option<Vec<bool>>, id: u16) -> bool {
    set.as_ref().is_none_or(|s| id != NONE16 && s.get(usize::from(id)).copied().unwrap_or(false))
}
fn mask_has(mask: Option<u32>, bit: u8) -> bool {
    mask.is_none_or(|m| bit < 32 && m >> bit & 1 == 1)
}

impl Compiled {
    // the dimensions this photo fails
    fn fails(&self, r: &Row, idx: &Index) -> u32 {
        let mut m = 0;
        if self.only_top && !r.is_top(self.min) {
            m |= D_TOP;
        }
        if let Some(l) = self.labels {
            if r.labels as u32 & l == 0 {
                m |= D_LABEL;
            }
        }
        if !set_has(&self.makers, r.maker) {
            m |= D_MAKER;
        }
        if !set_has(&self.bodies, r.body) {
            m |= D_BODY;
        }
        if !set_has(&self.lenses, r.lens) {
            m |= D_LENS;
        }
        if let Some(y) = &self.years {
            if r.year == 0 || !y.contains(&i32::from(r.year)) {
                m |= D_YEAR;
            }
        }
        if !mask_has(self.months, r.month) || (self.months.is_some() && r.month == 0) {
            m |= D_MONTH;
        }
        if !mask_has(self.hours, r.hour) {
            m |= D_HOUR;
        }
        if !mask_has(self.weekdays, r.weekday) {
            m |= D_WEEKDAY;
        }
        if !mask_has(self.focal, r.fb) {
            m |= D_FOCAL;
        }
        if !mask_has(self.aperture, r.ap) {
            m |= D_APERTURE;
        }
        if !mask_has(self.shutter, r.sh) {
            m |= D_SHUTTER;
        }
        if !mask_has(self.iso, r.isb) {
            m |= D_ISO;
        }
        if let Some(a) = self.aspect {
            let portrait = r.width > 0 && r.height > r.width;
            let hit = (r.asp != UNKNOWN && a >> r.asp & 1 == 1) || (portrait && a >> 5 & 1 == 1);
            if !hit {
                m |= D_ASPECT;
            }
        }
        if let Some(t) = &self.ftypes {
            if !t.get(usize::from(r.ftype)).copied().unwrap_or(false) {
                m |= D_FTYPE;
            }
        }
        if let Some(cells) = &self.place {
            let hit = r.has_gps()
                && cells.iter().any(|&(l, x, y)| r.qx >> (QZ - l) == x && r.qy >> (QZ - l) == y);
            if !hit {
                m |= D_PLACE;
            }
        }
        if let Some(e) = self.edited {
            if r.edited != e {
                m |= D_EDITED;
            }
        }
        let _ = idx;
        m
    }
}

/// An index with a request applied: for each row, the dimensions it fails (`OUT` if outside the scope).
pub struct Query<'a> {
    pub index: &'a Index,
    fails: Vec<u32>,
    pub min: u8,
}

impl<'a> Query<'a> {
    pub fn new(index: &'a Index, scoped: &Scoped, filter: &StatsFilter) -> Query<'a> {
        let cf = compile(index, filter);
        let min = cf.min;
        let fails = index
            .rows
            .iter()
            .map(|r| {
                if !scoped.folder_ok[r.folder as usize] {
                    return OUT;
                }
                if let Some((a, b)) = scoped.dates {
                    if !r.dated() || r.taken < a || r.taken > b {
                        return OUT;
                    }
                }
                cf.fails(r, index)
            })
            .collect();
        Query { index, fails, min }
    }
    fn counts(&self, own: u32, i: usize) -> bool {
        let f = self.fails[i];
        f & OUT == 0 && f & !own == 0
    }
    fn top(&self, r: &Row) -> bool {
        r.is_top(self.min)
    }
    /// Indices of the rows that pass the whole filter.
    pub fn matching(&self) -> Vec<u32> {
        (0..self.fails.len()).filter(|&i| self.fails[i] == 0).map(|i| i as u32).collect()
    }
}

// ---------------------------------------------------------------- accumulators

#[derive(Clone, Copy)]
struct Acc {
    n: u32,
    top: u32,
    best: u32,
}
impl Acc {
    const NEW: Acc = Acc { n: 0, top: 0, best: NONE32 };
}
fn cover_key(r: &Row) -> (i8, i64) {
    (if r.rejected { -1 } else { r.rating as i8 }, r.taken)
}
fn add(acc: &mut Acc, rows: &[Row], i: usize, top: bool, cover: bool) {
    acc.n += 1;
    acc.top += u32::from(top);
    if cover && (acc.best == NONE32 || cover_key(&rows[i]) > cover_key(&rows[acc.best as usize])) {
        acc.best = i as u32;
    }
}
fn newer(a: i64, b: i64) -> bool {
    a != NO_TIME && (b == NO_TIME || a > b)
}

/// Turns row numbers into photos for the response; the caller attaches handles.
pub struct Photos<'a> {
    pub index: &'a Index,
    pub id: &'a dyn Fn(&str) -> String,
}
impl Photos<'_> {
    pub fn get(&self, i: u32) -> PhotoRef {
        let r = &self.index.rows[i as usize];
        let id = (self.id)(&r.path);
        let label = LABEL_IDS.iter().enumerate().find(|(b, _)| r.labels >> b & 1 == 1).map(|(_, l)| *l);
        PhotoRef {
            thumb: ThumbRef::Cache { id: id.clone() },
            id,
            path: r.path.to_string(),
            rating: r.rating,
            rejected: r.rejected,
            label,
            body: (r.body != NONE16).then(|| self.index.bodies[usize::from(r.body)].label.clone()),
            lens: (r.lens != NONE16).then(|| self.index.lenses[usize::from(r.lens)].clone()),
            focal_mm: (r.focal > 0.0).then(|| f64::from(r.focal)),
            f_number: (r.aperture > 0.0).then(|| f64::from(r.aperture)),
            shutter_sec: (r.shutter > 0.0).then(|| f64::from(r.shutter)),
            iso: (r.iso > 0.0).then(|| f64::from(r.iso)),
            taken: r.dated().then(|| fmt_iso(r.taken)),
            place: r.has_gps().then(|| format!("{:.2}, {:.2}", r.lat, r.lng)),
            width: (r.width > 0).then_some(r.width),
            height: (r.height > 0).then_some(r.height),
        }
    }
    fn opt(&self, i: u32) -> Option<PhotoRef> {
        (i != NONE32).then(|| self.get(i))
    }
}

fn median(v: &mut [f32]) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    let k = (v.len() - 1) / 2;
    let (_, m, _) = v.select_nth_unstable_by(k, |a, b| a.total_cmp(b));
    Some(f64::from(*m))
}
fn pct(n: u32, d: u32) -> f64 {
    if d == 0 { 0.0 } else { (f64::from(n) / f64::from(d) * 100.0).round() }
}
fn num(n: f64) -> ParamValue {
    ParamValue::Number(n)
}
fn argmax(v: &[u32]) -> Option<usize> {
    let (i, m) = v.iter().enumerate().fold((0, 0), |(bi, bm), (i, &x)| if x > bm { (i, x) } else { (bi, bm) });
    (m > 0).then_some(i)
}

// ---------------------------------------------------------------- sections

fn overview(q: &Query, p: &Photos) -> Overview {
    let idx = q.index;
    let rows = &idx.rows;
    let mut m = Matches::default();
    let mut folders = vec![false; idx.folders.len()];
    let mut per_year: std::collections::BTreeMap<i32, (u32, u32)> = Default::default();
    let mut rating = [0u32; 6];
    let mut rejected = 0;
    let mut labels = [0u32; 5];
    let (mut bs, mut ls, mut ms) = (vec![false; idx.bodies.len()], vec![false; idx.lenses.len()], vec![false; idx.makers.len()]);
    let mut mm = vec![(0u32, 0u32); 2001];
    let (mut focal_n, mut focal_top) = (0u32, 0u32);
    let (mut fv, mut av, mut sv, mut iv) = (vec![], vec![], vec![], vec![]);
    let (mut hours, mut months) = ([0u32; 24], [0u32; 12]);
    let mut ftypes = vec![0u32; idx.ftypes.len()];
    let mut edited = 0;
    let mut asp = [(0u32, 0u32); 8];
    let mut body_acc = vec![(0u32, 0u32); idx.bodies.len()];
    let mut lens_acc = vec![(0u32, 0u32); idx.lenses.len()];
    let mut ap_acc = [(0u32, 0u32); 5];
    let mut sh_n = [0u32; 6];
    for (i, r) in rows.iter().enumerate() {
        if !q.counts(0, i) {
            continue;
        }
        let top = q.top(r);
        m.n += 1;
        m.top += u32::from(top);
        folders[r.folder as usize] = true;
        if r.rejected {
            rejected += 1;
        } else {
            rating[usize::from(r.rating.min(5))] += 1;
        }
        for (b, c) in labels.iter_mut().enumerate() {
            *c += u32::from(r.labels >> b & 1);
        }
        if r.year != 0 {
            let e = per_year.entry(i32::from(r.year)).or_default();
            e.0 += 1;
            e.1 += u32::from(top);
            hours[usize::from(r.hour)] += 1;
            months[usize::from(r.month) - 1] += 1;
        }
        if r.maker != NONE16 {
            ms[usize::from(r.maker)] = true;
        }
        if r.body != NONE16 {
            bs[usize::from(r.body)] = true;
            let e = &mut body_acc[usize::from(r.body)];
            e.0 += 1;
            e.1 += u32::from(top);
        }
        if r.lens != NONE16 {
            ls[usize::from(r.lens)] = true;
            let e = &mut lens_acc[usize::from(r.lens)];
            e.0 += 1;
            e.1 += u32::from(top);
        }
        if r.mm > 0 {
            let k = (r.mm as usize).min(2000);
            mm[k].0 += 1;
            mm[k].1 += u32::from(top);
            focal_n += 1;
            focal_top += u32::from(top);
            fv.push(r.focal);
        }
        if r.ap != UNKNOWN {
            av.push(r.aperture);
            let e = &mut ap_acc[usize::from(r.ap)];
            e.0 += 1;
            e.1 += u32::from(top);
        }
        if r.sh != UNKNOWN {
            sv.push(r.shutter);
            sh_n[usize::from(r.sh)] += 1;
        }
        if r.isb != UNKNOWN {
            iv.push(r.iso);
        }
        ftypes[usize::from(r.ftype)] += 1;
        edited += u32::from(r.edited);
        if r.asp != UNKNOWN {
            asp[usize::from(r.asp)].0 += 1;
            asp[usize::from(r.asp)].1 += u32::from(top);
        }
        if r.width > 0 && r.height > r.width {
            asp[5].0 += 1;
            asp[5].1 += u32::from(top);
        }
    }
    let known_time: u32 = hours.iter().sum();
    let peak_hour = argmax(&hours);
    let peak_month = argmax(&months);
    let fav = argmax_pair(&mm);
    let favourite_focal = fav.map(|k| FavouriteFocal {
        mm: k as u32,
        share: f64::from(mm[k].0) / f64::from(focal_n.max(1)),
        top_share: if focal_top == 0 { 0.0 } else { f64::from(mm[k].1) / f64::from(focal_top) },
    });
    let mut types: Vec<FileTypeCount> = ftypes
        .iter()
        .enumerate()
        .filter(|(_, n)| **n > 0)
        .map(|(i, n)| FileTypeCount { kind: idx.ftypes[i].clone(), n: *n })
        .collect();
    types.sort_by(|a, b| b.n.cmp(&a.n).then(a.kind.cmp(&b.kind)));
    types.truncate(8);
    let mut years: Vec<YearCount> = per_year.iter().map(|(y, (n, t))| YearCount { year: *y, n: *n, top: *t }).collect();
    if years.len() > 30 {
        years.drain(..years.len() - 30);
    }
    // insights, in priority order
    let mut insights = Vec::new();
    let best_rate = |acc: &[(u32, u32)]| -> Option<usize> {
        let ok: Vec<usize> = (0..acc.len()).filter(|&i| acc[i].0 >= INSIGHT_MIN_N).collect();
        if ok.len() < 2 {
            return None;
        }
        let rate = |i: usize| f64::from(acc[i].1) / f64::from(acc[i].0);
        let b = *ok.iter().max_by(|&&a, &&b| rate(a).total_cmp(&rate(b)).then(acc[a].0.cmp(&acc[b].0)))?;
        (acc[b].1 > 0).then_some(b)
    };
    let ins = |id, params: &[(&str, ParamValue)], apply: Option<StatsFilter>| Insight {
        id,
        params: params.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(),
        apply,
    };
    if let Some(b) = best_rate(&body_acc) {
        insights.push(ins(
            InsightId::KeeperBody,
            &[
                ("body", ParamValue::Text(idx.bodies[b].label.clone())),
                ("rate", num(pct(body_acc[b].1, body_acc[b].0))),
                ("n", num(f64::from(body_acc[b].0))),
            ],
            Some(StatsFilter { bodies: Some(vec![idx.bodies[b].key.clone()]), ..Default::default() }),
        ));
    }
    if let Some(l) = best_rate(&lens_acc) {
        insights.push(ins(
            InsightId::KeeperLens,
            &[
                ("lens", ParamValue::Text(idx.lenses[l].clone())),
                ("rate", num(pct(lens_acc[l].1, lens_acc[l].0))),
                ("n", num(f64::from(lens_acc[l].0))),
            ],
            Some(StatsFilter { lenses: Some(vec![idx.lenses[l].clone()]), ..Default::default() }),
        ));
    }
    if let Some(f) = &favourite_focal {
        let b = FOCAL_IDS[usize::from(focal_bucket(f.mm))];
        insights.push(ins(
            InsightId::FavouriteFocal,
            &[("mm", num(f64::from(f.mm))), ("share", num((f.share * 100.0).round()))],
            Some(StatsFilter { focal_buckets: Some(vec![b]), ..Default::default() }),
        ));
    }
    let golden: u32 = [5, 6, 7, 8, 16, 17, 18, 19].iter().map(|h| hours[*h]).sum();
    if known_time > 0 && golden > 0 {
        insights.push(ins(
            InsightId::GoldenHour,
            &[("share", num(pct(golden, known_time))), ("n", num(f64::from(golden)))],
            Some(StatsFilter { hours: Some(vec![5, 6, 7, 8, 16, 17, 18, 19]), ..Default::default() }),
        ));
    }
    if let Some(mo) = peak_month {
        insights.push(ins(
            InsightId::PeakMonth,
            &[("month", num((mo + 1) as f64)), ("n", num(f64::from(months[mo]))), ("share", num(pct(months[mo], known_time)))],
            Some(StatsFilter { months: Some(vec![(mo + 1) as u8]), ..Default::default() }),
        ));
    }
    if let Some(a) = best_rate(&ap_acc) {
        insights.push(ins(
            InsightId::BestAperture,
            &[("group", ParamValue::Text(format!("{:?}", APERTURE_IDS[a]).to_lowercase())), ("rate", num(pct(ap_acc[a].1, ap_acc[a].0)))],
            Some(StatsFilter { aperture_groups: Some(vec![APERTURE_IDS[a]]), ..Default::default() }),
        ));
    }
    let sh_known: u32 = sh_n.iter().sum();
    if sh_known > 0 && sh_n[2] > 0 {
        insights.push(ins(
            InsightId::HandheldShare,
            &[("share", num(pct(sh_n[2], sh_known))), ("n", num(f64::from(sh_n[2])))],
            Some(StatsFilter { shutter_regimes: Some(vec![ShutterRegimeId::Handheld]), ..Default::default() }),
        ));
    }
    insights.truncate(6);
    let _ = p;
    Overview {
        matches: m,
        photos: m.n,
        folders: folders.iter().filter(|f| **f).count() as u32,
        first_year: per_year.keys().next().copied(),
        last_year: per_year.keys().next_back().copied(),
        per_year: years,
        rating_dist: rating,
        rejected,
        labels: label_counts(labels),
        gear: Gear {
            bodies: bs.iter().filter(|b| **b).count() as u32,
            lenses: ls.iter().filter(|b| **b).count() as u32,
            makers: ms.iter().filter(|b| **b).count() as u32,
        },
        favourite_focal,
        median_setup: MedianSetup {
            focal_mm: median(&mut fv),
            f_number: median(&mut av),
            shutter_sec: median(&mut sv),
            iso: median(&mut iv),
        },
        peak: Peak {
            hour: peak_hour.map(|h| h as u8),
            month: peak_month.map(|m| (m + 1) as u8),
            month_n: peak_month.map_or(0, |m| months[m]),
        },
        file_types: types,
        edited,
        insights,
        aspect: ASPECT_IDS
            .iter()
            .enumerate()
            .map(|(i, id)| AspectCount { id: *id, n: asp[i].0, top: asp[i].1 })
            .collect(),
    }
}
fn argmax_pair(v: &[(u32, u32)]) -> Option<usize> {
    let (i, m) = v.iter().enumerate().fold((0, 0), |(bi, bm), (i, x)| if x.0 > bm { (i, x.0) } else { (bi, bm) });
    (m > 0).then_some(i)
}
fn label_counts(l: [u32; 5]) -> LabelCounts {
    LabelCounts { red: l[0], yellow: l[1], green: l[2], blue: l[3], purple: l[4] }
}

fn cameras(q: &Query, p: &Photos, with_timeline: bool) -> Cameras {
    let idx = q.index;
    let rows = &idx.rows;
    let own = D_MAKER | D_BODY;
    let nb = idx.bodies.len();
    let mut body = vec![Acc::NEW; nb];
    let mut maker = vec![Acc::NEW; idx.makers.len()];
    let mut first = vec![NO_TIME; nb];
    let mut last = vec![NO_TIME; nb];
    let mut serials: HashSet<(u16, u32)> = HashSet::new();
    let mut pair: HashMap<(u16, u16), u32> = HashMap::new();
    let mut known = 0;
    for (i, r) in rows.iter().enumerate() {
        if !q.counts(own, i) || r.body == NONE16 {
            continue;
        }
        let top = q.top(r);
        known += 1;
        let b = usize::from(r.body);
        add(&mut body[b], rows, i, top, true);
        add(&mut maker[usize::from(r.maker)], rows, i, top, false);
        if r.dated() {
            if first[b] == NO_TIME || r.taken < first[b] {
                first[b] = r.taken;
            }
            if newer(r.taken, last[b]) {
                last[b] = r.taken;
            }
        }
        if r.serial != NONE32 {
            serials.insert((r.body, r.serial));
        }
        if r.lens != NONE16 {
            *pair.entry((r.body, r.lens)).or_default() += 1;
        }
    }
    let mut serial_n = vec![0u32; nb];
    for (b, _) in &serials {
        serial_n[usize::from(*b)] += 1;
    }
    let mut top_lens: Vec<(u32, u16)> = vec![(0, NONE16); nb];
    for ((b, l), n) in &pair {
        let e = &mut top_lens[usize::from(*b)];
        // the most used lens; on a tie the lower id keeps it stable
        if *n > e.0 || (*n == e.0 && *l < e.1) {
            *e = (*n, *l);
        }
    }
    let mut order: Vec<usize> = (0..nb).filter(|&b| body[b].n > 0).collect();
    order.sort_by(|&a, &b| body[b].n.cmp(&body[a].n).then(idx.bodies[a].key.cmp(&idx.bodies[b].key)));
    let mut body_count = vec![0u32; idx.makers.len()];
    for &b in &order {
        body_count[usize::from(idx.bodies[b].maker)] += 1;
    }
    let mut makers: Vec<MakerRow> = (0..idx.makers.len())
        .filter(|&m| maker[m].n > 0)
        .map(|m| MakerRow { maker: idx.makers[m].clone(), n: maker[m].n, top: maker[m].top, bodies: body_count[m] })
        .collect();
    makers.sort_by(|a, b| b.n.cmp(&a.n).then(a.maker.cmp(&b.maker)));
    makers.truncate(LIST_CAP);
    order.truncate(LIST_CAP);
    let bodies: Vec<BodyRow> = order
        .iter()
        .map(|&b| BodyRow {
            key: idx.bodies[b].key.clone(),
            maker: idx.makers[usize::from(idx.bodies[b].maker)].clone(),
            model: idx.bodies[b].model.clone(),
            n: body[b].n,
            top: body[b].top,
            keeper_rate: KeeperRate { top: body[b].top, n: body[b].n },
            cover: p.opt(body[b].best),
            first_shot: (first[b] != NO_TIME).then(|| fmt_date(first[b])),
            last_shot: (last[b] != NO_TIME).then(|| fmt_date(last[b])),
            serials: serial_n[b],
            top_lens: (top_lens[b].1 != NONE16).then(|| idx.lenses[usize::from(top_lens[b].1)].clone()),
            top_place: None,
        })
        .collect();
    // the era chips: photos per year with the cameras filter, ignoring the year selection too
    let own_y = own | D_YEAR;
    let mut per_year: std::collections::BTreeMap<i32, u32> = Default::default();
    let mut by_body_year: HashMap<(u16, i32), u32> = HashMap::new();
    for (i, r) in rows.iter().enumerate() {
        if r.year == 0 || !q.counts(own_y, i) || r.body == NONE16 {
            continue;
        }
        *per_year.entry(i32::from(r.year)).or_default() += 1;
        if with_timeline {
            *by_body_year.entry((r.body, i32::from(r.year))).or_default() += 1;
        }
    }
    let mut cum = 0;
    let years: Vec<CameraYear> = per_year
        .iter()
        .map(|(y, n)| {
            cum += n;
            CameraYear { year: *y, n: *n, cumulative: cum }
        })
        .collect();
    let timeline = with_timeline.then(|| {
        order
            .iter()
            .filter(|&&b| first[b] != NO_TIME)
            .map(|&b| BodyTimeline {
                body: idx.bodies[b].key.clone(),
                first: fmt_date(first[b]),
                last: fmt_date(last[b]),
                per_year: years.iter().map(|y| by_body_year.get(&(b as u16, y.year)).copied().unwrap_or(0)).collect(),
            })
            .collect()
    });
    Cameras { known, makers, bodies, years, timeline }
}

fn focal_bin(mm: u32) -> u32 {
    if mm < 100 {
        mm
    } else if mm < 300 {
        100 + (mm - 100) / 5 * 5
    } else {
        300 + (mm - 300) / 25 * 25
    }
}

fn lenses(q: &Query, p: &Photos) -> Lenses {
    let idx = q.index;
    let rows = &idx.rows;
    let nl = idx.lenses.len();
    let mut lens = vec![Acc::NEW; nl];
    let mut mm_by_lens: HashMap<(u16, u32), u32> = HashMap::new();
    let mut maker_of: HashMap<(u16, u16), u32> = HashMap::new();
    let mut known = 0;
    for (i, r) in rows.iter().enumerate() {
        if !q.counts(D_LENS, i) || r.lens == NONE16 {
            continue;
        }
        known += 1;
        add(&mut lens[usize::from(r.lens)], rows, i, q.top(r), true);
        if r.mm > 0 {
            *mm_by_lens.entry((r.lens, r.mm)).or_default() += 1;
        }
        if r.maker != NONE16 {
            *maker_of.entry((r.lens, r.maker)).or_default() += 1;
        }
    }
    // the focal length picture ignores the focal selection, not the lens one
    let mut bins: std::collections::BTreeMap<u32, (u32, u32)> = Default::default();
    let mut bucket = vec![Acc::NEW; 6];
    let mut bucket_mm: Vec<HashMap<u32, u32>> = vec![HashMap::new(); 6];
    let mut mm_total: HashMap<u32, u32> = HashMap::new();
    for (i, r) in rows.iter().enumerate() {
        if r.mm == 0 || !q.counts(D_FOCAL, i) {
            continue;
        }
        let top = q.top(r);
        let e = bins.entry(focal_bin(r.mm)).or_default();
        e.0 += 1;
        e.1 += u32::from(top);
        add(&mut bucket[usize::from(r.fb)], rows, i, top, true);
        *bucket_mm[usize::from(r.fb)].entry(r.mm).or_default() += 1;
        *mm_total.entry(r.mm).or_default() += 1;
    }
    let modal = |m: &HashMap<u32, u32>| m.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))).map(|(k, v)| (*k, *v));
    // per lens: focal range and modal length
    let mut ranges: HashMap<u16, (u32, u32, u32, u32, u32)> = HashMap::new(); // min, max, modal, modal_n, total
    for ((l, mm), n) in &mm_by_lens {
        let e = ranges.entry(*l).or_insert((u32::MAX, 0, 0, 0, 0));
        e.0 = e.0.min(*mm);
        e.1 = e.1.max(*mm);
        e.4 += n;
        if *n > e.3 || (*n == e.3 && *mm < e.2) {
            e.2 = *mm;
            e.3 = *n;
        }
    }
    let mut order: Vec<usize> = (0..nl).filter(|&l| lens[l].n > 0).collect();
    order.sort_by(|&a, &b| lens[b].n.cmp(&lens[a].n).then(idx.lenses[a].cmp(&idx.lenses[b])));
    order.truncate(LIST_CAP);
    let rows_out: Vec<LensRow> = order
        .iter()
        .map(|&l| {
            let range = ranges.get(&(l as u16));
            let maker = maker_of
                .iter()
                .filter(|((li, _), _)| usize::from(*li) == l)
                .max_by(|a, b| a.1.cmp(b.1).then(b.0 .1.cmp(&a.0 .1)))
                .map(|((_, m), _)| idx.makers[usize::from(*m)].clone());
            LensRow {
                name: idx.lenses[l].clone(),
                maker,
                n: lens[l].n,
                top: lens[l].top,
                keeper_rate: KeeperRate { top: lens[l].top, n: lens[l].n },
                focal_range: range.map(|r| FocalRange {
                    min_mm: f64::from(r.0),
                    max_mm: f64::from(r.1),
                    modal_mm: f64::from(r.2),
                    modal_share: f64::from(r.3) / f64::from(r.4.max(1)),
                }),
                is_prime: range.is_some_and(|r| r.0 == r.1),
                cover: p.opt(lens[l].best),
            }
        })
        .collect();
    // heat strips for zoom lenses: 16 log-spaced bins between the shortest and longest focal lengths
    let mut zoom = Vec::new();
    for &l in &order {
        let Some(r) = ranges.get(&(l as u16)).filter(|r| r.1 > r.0 && r.0 > 0) else { continue };
        let (lo, hi) = (f64::from(r.0).ln(), f64::from(r.1).ln());
        let mut b = vec![0u32; 16];
        for ((li, mm), n) in &mm_by_lens {
            if usize::from(*li) == l {
                let t = ((f64::from(*mm).ln() - lo) / (hi - lo) * 16.0).floor().clamp(0.0, 15.0);
                b[t as usize] += n;
            }
        }
        zoom.push(ZoomUsage { lens: idx.lenses[l].clone(), min_mm: f64::from(r.0), max_mm: f64::from(r.1), bins: b });
    }
    Lenses {
        known,
        lenses: rows_out,
        focal: FocalHist {
            bins: bins.iter().map(|(mm, (n, t))| (*mm, *n, *t)).collect(),
            modal_mm: modal(&mm_total).map(|(k, _)| k),
        },
        focal_by_bucket: (0..6)
            .map(|b| FocalBucketRow {
                id: FOCAL_IDS[b],
                n: bucket[b].n,
                top: bucket[b].top,
                cover: p.opt(bucket[b].best),
                modal_mm: modal(&bucket_mm[b]).map_or(0, |(k, _)| k),
            })
            .collect(),
        zoom_usage: Some(zoom),
    }
}

fn exposure(q: &Query, p: &Photos) -> Exposure {
    let rows = &q.index.rows;
    let (mut ap, mut sh) = ([Acc::NEW; 5], [Acc::NEW; 6]);
    let mut iso = [(0u32, 0u32); 7];
    let mut known = (0, 0, 0);
    for (i, r) in rows.iter().enumerate() {
        let top = q.top(r);
        if r.ap != UNKNOWN && q.counts(D_APERTURE, i) {
            known.0 += 1;
            add(&mut ap[usize::from(r.ap)], rows, i, top, true);
        }
        if r.sh != UNKNOWN && q.counts(D_SHUTTER, i) {
            known.1 += 1;
            add(&mut sh[usize::from(r.sh)], rows, i, top, true);
        }
        if r.isb != UNKNOWN && q.counts(D_ISO, i) {
            known.2 += 1;
            iso[usize::from(r.isb)].0 += 1;
            iso[usize::from(r.isb)].1 += u32::from(top);
        }
    }
    let (mut av, mut sv, mut iv) = (vec![], vec![], vec![]);
    let mut grid = vec![vec![0u32; 6]; 5];
    for (i, r) in rows.iter().enumerate() {
        if !q.counts(0, i) {
            continue;
        }
        if r.ap != UNKNOWN {
            av.push(r.aperture);
        }
        if r.sh != UNKNOWN {
            sv.push(r.shutter);
        }
        if r.isb != UNKNOWN {
            iv.push(r.iso);
        }
        if r.ap != UNKNOWN && r.sh != UNKNOWN {
            grid[usize::from(r.ap)][usize::from(r.sh)] += 1;
        }
    }
    Exposure {
        known: ExposureKnown { aperture: known.0, shutter: known.1, iso: known.2 },
        aperture: (0..5).map(|g| ApertureRow { id: APERTURE_IDS[g], n: ap[g].n, top: ap[g].top, cover: p.opt(ap[g].best) }).collect(),
        shutter: (0..6).map(|g| ShutterRow { id: SHUTTER_IDS[g], n: sh[g].n, top: sh[g].top, cover: p.opt(sh[g].best) }).collect(),
        iso: (0..7).map(|g| IsoRow { id: ISO_IDS[g], n: iso[g].0, top: iso[g].1 }).collect(),
        median: ExposureMedian { f_number: median(&mut av), shutter_sec: median(&mut sv), iso: median(&mut iv) },
        aperture_by_shutter: Some(grid),
    }
}

fn time(q: &Query) -> TimeStats {
    let rows = &q.index.rows;
    let mut hours = vec![Count { n: 0, top: 0 }; 24];
    let mut months = vec![Count { n: 0, top: 0 }; 12];
    let mut weekdays = vec![Count { n: 0, top: 0 }; 7];
    let mut years: std::collections::BTreeMap<i32, (u32, u32)> = Default::default();
    let mut grid = vec![vec![0u32; 24]; 12];
    let mut known = 0;
    let all_time = D_HOUR | D_MONTH | D_YEAR | D_WEEKDAY;
    for (i, r) in rows.iter().enumerate() {
        if !r.dated() {
            continue;
        }
        let top = u32::from(q.top(r));
        if q.counts(all_time, i) {
            known += 1;
        }
        if q.counts(D_HOUR, i) {
            hours[usize::from(r.hour)].n += 1;
            hours[usize::from(r.hour)].top += top;
        }
        if q.counts(D_MONTH, i) {
            months[usize::from(r.month) - 1].n += 1;
            months[usize::from(r.month) - 1].top += top;
        }
        if q.counts(D_WEEKDAY, i) {
            weekdays[usize::from(r.weekday)].n += 1;
            weekdays[usize::from(r.weekday)].top += top;
        }
        if q.counts(D_YEAR, i) {
            let e = years.entry(i32::from(r.year)).or_default();
            e.0 += 1;
            e.1 += top;
        }
        if q.counts(0, i) {
            grid[usize::from(r.month) - 1][usize::from(r.hour)] += 1;
        }
    }
    let hn: Vec<u32> = hours.iter().map(|c| c.n).collect();
    let mn: Vec<u32> = months.iter().map(|c| c.n).collect();
    TimeStats {
        known,
        peak: HourMonth { hour: argmax(&hn).unwrap_or(0) as u8, month: argmax(&mn).map_or(0, |m| m + 1) as u8 },
        hours,
        months,
        years: years.iter().map(|(y, (n, t))| YearCount { year: *y, n: *n, top: *t }).collect(),
        weekdays: Some(weekdays),
        month_by_hour: Some(grid),
    }
}

#[derive(Clone, Copy, Default)]
struct Cell {
    n: u32,
    top: u32,
    lat: f64,
    lng: f64,
    first: u32,
}

fn cell_out(level: u32, x: u32, y: u32, c: &Cell, p: &Photos) -> PlaceCell {
    let n = f64::from(c.n);
    let r5 = |v: f64| (v * 1e5).round() / 1e5;
    PlaceCell {
        key: quadkey(level, x, y),
        lat: r5(c.lat / n),
        lng: r5(c.lng / n),
        n: c.n,
        top: c.top,
        solo: (c.n == 1).then(|| p.get(c.first)),
        name: None,
    }
}

/// The cells of the map: tiles at `zoom + 3`, over `bbox` (plus a 10 % margin), at most `PLACE_CAP`.
/// When more would be sent, every cell merges into its parent tile until they fit.
fn places(q: &Query, p: &Photos, view: &PlacesView) -> Places {
    let rows = &q.index.rows;
    let zoom = u32::from(view.zoom.min(19));
    let mut level = (zoom + 3).min(QZ);
    let mut known = 0;
    let mut fine: HashSet<u64> = HashSet::new();
    let (mut w, mut s, mut e, mut n) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    let frame = view.bbox.filter(|b| b.0 <= b.2 && b.1 <= b.3).map(|(w, s, e, n)| {
        let (dx, dy) = ((e - w) * 0.1, (n - s) * 0.1);
        (w - dx, s - dy, e + dx, n + dy)
    });
    let mut cells: HashMap<(u32, u32), Cell> = HashMap::new();
    for (i, r) in rows.iter().enumerate() {
        if !r.has_gps() || !q.counts(D_PLACE, i) {
            continue;
        }
        known += 1;
        fine.insert(u64::from(r.qx >> (QZ - FINE)) << 32 | u64::from(r.qy >> (QZ - FINE)));
        w = w.min(r.lng);
        e = e.max(r.lng);
        s = s.min(r.lat);
        n = n.max(r.lat);
        if frame.is_some_and(|f| r.lng < f.0 || r.lng > f.2 || r.lat < f.1 || r.lat > f.3) {
            continue;
        }
        let c = cells.entry((r.qx >> (QZ - level), r.qy >> (QZ - level))).or_insert(Cell { first: i as u32, ..Default::default() });
        c.n += 1;
        c.top += u32::from(q.top(r));
        c.lat += r.lat;
        c.lng += r.lng;
    }
    // the quick-jump list: the biggest cells at the requested level, before any merging
    let mut biggest: Vec<(&(u32, u32), &Cell)> = cells.iter().collect();
    biggest.sort_by(|a, b| b.1.n.cmp(&a.1.n).then(a.0.cmp(b.0)));
    let top: Vec<PlaceCell> = biggest.iter().take(12).map(|(k, c)| cell_out(level, k.0, k.1, c, p)).collect();
    while cells.len() > PLACE_CAP && level > 0 {
        level -= 1;
        let mut merged: HashMap<(u32, u32), Cell> = HashMap::new();
        for ((x, y), c) in &cells {
            let m = merged.entry((x >> 1, y >> 1)).or_insert(Cell { first: c.first, ..Default::default() });
            m.n += c.n;
            m.top += c.top;
            m.lat += c.lat;
            m.lng += c.lng;
        }
        cells = merged;
    }
    let mut out: Vec<PlaceCell> = cells.iter().map(|(k, c)| cell_out(level, k.0, k.1, c, p)).collect();
    out.sort_by(|a, b| b.n.cmp(&a.n).then(a.key.cmp(&b.key)));
    let heat = view.heat.unwrap_or(false).then(|| {
        out.iter()
            .take(HEAT_CAP)
            .map(|c| (c.lat, c.lng, f64::from(c.n)))
            .collect()
    });
    Places {
        known,
        total: fine.len() as u32,
        zoom: view.zoom.min(19),
        bbox: (known > 0).then_some((w, s, e, n)),
        cells: out,
        heat,
        top,
    }
}

fn top_order(rows: &[Row], a: u32, b: u32) -> std::cmp::Ordering {
    let (x, y) = (&rows[a as usize], &rows[b as usize]);
    y.rating.cmp(&x.rating).then(y.taken.cmp(&x.taken)).then(x.path.cmp(&y.path))
}

/// The rows of the top images under the full filter, best first, up to `limit` after `offset`.
pub fn top_rows(q: &Query, offset: usize, limit: usize) -> (u32, Vec<u32>) {
    let rows = &q.index.rows;
    let mut v: Vec<u32> = (0..rows.len())
        .filter(|&i| q.fails[i] == 0 && q.top(&rows[i]))
        .map(|i| i as u32)
        .collect();
    let total = v.len() as u32;
    let want = (offset + limit).min(v.len());
    if want < v.len() {
        v.select_nth_unstable_by(want, |a, b| top_order(rows, *a, *b));
        v.truncate(want);
    }
    v.sort_by(|a, b| top_order(rows, *a, *b));
    (total, v.into_iter().skip(offset).collect())
}

fn top_images(q: &Query, p: &Photos) -> TopImages {
    let rows = &q.index.rows;
    let mut by_rating = [0u32; 6];
    let mut labels = [0u32; 5];
    for (i, r) in rows.iter().enumerate() {
        if q.fails[i] == 0 && q.top(r) {
            by_rating[usize::from(r.rating.min(5))] += 1;
            for (b, c) in labels.iter_mut().enumerate() {
                *c += u32::from(r.labels >> b & 1);
            }
        }
    }
    let (total, items) = top_rows(q, 0, TOP_PAGE);
    TopImages {
        total,
        page: TopPage { next: None },
        items: items.iter().map(|i| p.get(*i)).collect(),
        by_rating,
        by_label: label_counts(labels),
    }
}

/// Rows that pass the whole filter, best first (rating, then recency), at most `cap`; false when cut.
pub fn grid_rows(q: &Query, cap: usize) -> (Vec<u32>, bool) {
    let rows = &q.index.rows;
    let mut v = q.matching();
    v.sort_by(|a, b| {
        let (x, y) = (&rows[*a as usize], &rows[*b as usize]);
        let key = |r: &Row| if r.rejected { -1 } else { i16::from(r.rating) };
        key(y).cmp(&key(x)).then(y.taken.cmp(&x.taken)).then(x.path.cmp(&y.path))
    });
    let complete = v.len() <= cap;
    v.truncate(cap);
    (v, complete)
}

/// What a snapshot needs besides the index.
pub struct Wanted<'a> {
    pub sections: &'a [StatsSection],
    pub places: Option<PlacesView>,
}

pub struct Sections {
    pub overview: Option<Overview>,
    pub cameras: Option<Cameras>,
    pub lenses: Option<Lenses>,
    pub exposure: Option<Exposure>,
    pub time: Option<TimeStats>,
    pub places: Option<Places>,
    pub top: Option<TopImages>,
}

/// Every requested section for one query.
pub fn snapshot(q: &Query, p: &Photos, w: &Wanted) -> Sections {
    let has = |s| w.sections.contains(&s);
    Sections {
        overview: has(StatsSection::Overview).then(|| overview(q, p)),
        cameras: has(StatsSection::Cameras).then(|| cameras(q, p, true)),
        lenses: has(StatsSection::Lenses).then(|| lenses(q, p)),
        exposure: has(StatsSection::Exposure).then(|| exposure(q, p)),
        time: has(StatsSection::Time).then(|| time(q)),
        places: has(StatsSection::Places)
            .then(|| places(q, p, &w.places.clone().unwrap_or(PlacesView { zoom: 2, bbox: None, heat: None }))),
        top: has(StatsSection::Top).then(|| top_images(q, p)),
    }
}

/// Counts for the coverage block under a scope: (indexed, known = indexed + unindexed, unindexed per folder).
pub fn scope_counts(index: &Index, scoped: &Scoped) -> (u32, u32, Vec<(u32, u32)>) {
    let indexed = index
        .rows
        .iter()
        .filter(|r| scoped.folder_ok[r.folder as usize] && scoped.dates.is_none_or(|(a, b)| r.dated() && r.taken >= a && r.taken <= b))
        .count() as u32;
    // photos without metadata have no date: a date range cannot count them
    let pending: Vec<(u32, u32)> = if scoped.dates.is_some() {
        vec![]
    } else {
        index.unindexed.iter().filter(|(f, _)| scoped.folder_ok[*f as usize]).copied().collect()
    };
    let unindexed: u32 = pending.iter().map(|(_, n)| n).sum();
    (indexed, indexed + unindexed, pending)
}

/// Whether an indexed folder is the root or below it.
pub fn folder_under(index: &Index, folder: u32, root: &str) -> bool {
    under(&index.folders[folder as usize], root)
}

/// One map cell's side panel, under the filter minus its place selection.
pub fn place_detail(q: &Query, p: &Photos, key: &str) -> Result<PlaceDetail, String> {
    let (level, x, y) = parse_quadkey(key).ok_or("Invalid place key")?;
    let rows = &q.index.rows;
    let mut picked: Vec<u32> = Vec::new();
    for (i, r) in rows.iter().enumerate() {
        if r.has_gps() && q.counts(D_PLACE, i) && r.qx >> (QZ - level) == x && r.qy >> (QZ - level) == y {
            picked.push(i as u32);
        }
    }
    let n = picked.len() as u32;
    let (mut la, mut lo) = (0.0, 0.0);
    let (mut top, mut months) = (0, vec![0u32; 12]);
    let (mut first, mut last) = (NO_TIME, NO_TIME);
    let mut bodies: HashMap<u16, u32> = HashMap::new();
    let mut lenses: HashMap<u16, u32> = HashMap::new();
    let mut mm: HashMap<u32, u32> = HashMap::new();
    for &i in &picked {
        let r = &rows[i as usize];
        la += r.lat;
        lo += r.lng;
        top += u32::from(q.top(r));
        if r.dated() {
            months[usize::from(r.month) - 1] += 1;
            if first == NO_TIME || r.taken < first {
                first = r.taken;
            }
            if newer(r.taken, last) {
                last = r.taken;
            }
        }
        if r.body != NONE16 {
            *bodies.entry(r.body).or_default() += 1;
        }
        if r.lens != NONE16 {
            *lenses.entry(r.lens).or_default() += 1;
        }
        if r.mm > 0 {
            *mm.entry(r.mm).or_default() += 1;
        }
    }
    let (lat, lng) = if n > 0 { (la / f64::from(n), lo / f64::from(n)) } else { tile_center(level, x, y) };
    let radius = picked
        .iter()
        .map(|&i| haversine_km(lat, lng, rows[i as usize].lat, rows[i as usize].lng))
        .fold(0.0, f64::max);
    picked.sort_by(|a, b| top_order(rows, *a, *b));
    let best = |m: &HashMap<u16, u32>| m.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))).map(|(k, v)| (*k, *v));
    Ok(PlaceDetail {
        key: key.to_string(),
        name: None,
        lat: (lat * 1e5).round() / 1e5,
        lng: (lng * 1e5).round() / 1e5,
        radius_km: (radius * 100.0).round() / 100.0,
        n,
        top,
        first_shot: (first != NO_TIME).then(|| fmt_date(first)),
        last_shot: (last != NO_TIME).then(|| fmt_date(last)),
        top_body: best(&bodies).map(|(b, n)| KeyCount { key: q.index.bodies[usize::from(b)].key.clone(), n }),
        top_lens: best(&lenses).map(|(l, n)| NameCount { name: q.index.lenses[usize::from(l)].clone(), n }),
        modal_focal_mm: mm.iter().max_by(|a, b| a.1.cmp(b.1).then(b.0.cmp(a.0))).map(|(k, _)| *k),
        months,
        photos: picked.iter().take(6).map(|i| p.get(*i)).collect(),
    })
}
fn haversine_km(a: f64, b: f64, c: f64, d: f64) -> f64 {
    let (p1, p2) = (a.to_radians(), c.to_radians());
    let h = ((c - a).to_radians() / 2.0).sin().powi(2) + p1.cos() * p2.cos() * ((d - b).to_radians() / 2.0).sin().powi(2);
    6371.0 * 2.0 * h.sqrt().asin()
}

#[cfg(test)]
mod tests;
