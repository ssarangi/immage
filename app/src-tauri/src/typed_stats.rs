// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// request and reply types of the Statistics commands (stats_cmds.rs), typed in
// bindings.rs. they follow notes/statistics-data-contract.ts field for field;
// stats.rs computes them from the search cache's metadata
use serde::{Deserialize, Serialize};
use specta::Type;
use std::collections::BTreeMap;

// ---------------------------------------------------------------- request

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum StatsScope {
    Library,
    Folder { folder: String, include_subfolders: bool },
    // YYYY-MM-DD, inclusive; photos without a capture time are left out
    DateRange { from: String, to: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ColorLabel {
    Red,
    Yellow,
    Green,
    Blue,
    Purple,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum FocalBucketId {
    Uw,
    Wide,
    Normal,
    Portrait,
    Tele,
    Super,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
pub enum ApertureGroupId {
    #[serde(rename = "a1")]
    A1,
    #[serde(rename = "a2")]
    A2,
    #[serde(rename = "a3")]
    A3,
    #[serde(rename = "a4")]
    A4,
    #[serde(rename = "a5")]
    A5,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum ShutterRegimeId {
    Freeze,
    Fast,
    Handheld,
    Slow,
    Motion,
    Long,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
pub enum IsoBinId {
    #[serde(rename = "100")]
    I100,
    #[serde(rename = "200")]
    I200,
    #[serde(rename = "400")]
    I400,
    #[serde(rename = "800")]
    I800,
    #[serde(rename = "1600")]
    I1600,
    #[serde(rename = "3200")]
    I3200,
    #[serde(rename = "6400+")]
    I6400Plus,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
pub enum AspectClassId {
    #[serde(rename = "16:9")]
    R16x9,
    #[serde(rename = "3:2")]
    R3x2,
    #[serde(rename = "4:3")]
    R4x3,
    #[serde(rename = "1:1")]
    R1x1,
    #[serde(rename = "5:4")]
    R5x4,
    #[serde(rename = "portrait")]
    Portrait,
    #[serde(rename = "pano")]
    Pano,
    #[serde(rename = "other")]
    Other,
}

// every chart element click sets or clears exactly one field. an absent field
// selects nothing; a photo missing the field's data never matches a set one
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase", default)]
pub struct StatsFilter {
    // the "top images" threshold, 0..5 (a request below 1 still needs one star)
    pub min_rating: u8,
    pub only_top: bool,
    pub labels: Option<Vec<ColorLabel>>,
    pub makers: Option<Vec<String>>,
    // "Maker|Model"
    pub bodies: Option<Vec<String>>,
    pub lenses: Option<Vec<String>>,
    pub years: Option<Vec<i32>>,
    // 1..12
    pub months: Option<Vec<u8>>,
    // 0..23
    pub hours: Option<Vec<u8>>,
    // 0..6, Monday = 0
    pub weekdays: Option<Vec<u8>>,
    pub focal_buckets: Option<Vec<FocalBucketId>>,
    pub aperture_groups: Option<Vec<ApertureGroupId>>,
    pub shutter_regimes: Option<Vec<ShutterRegimeId>>,
    pub iso_bins: Option<Vec<IsoBinId>>,
    pub aspect: Option<Vec<AspectClassId>>,
    pub file_types: Option<Vec<String>>,
    // quadkeys of any length
    pub place: Option<Vec<String>>,
    pub edited: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Type)]
#[serde(rename_all = "lowercase")]
pub enum StatsSection {
    Overview,
    Cameras,
    Lenses,
    Exposure,
    Time,
    Places,
    Top,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, Type)]
pub enum YearMode {
    #[serde(rename = "that-year")]
    ThatYear,
    #[serde(rename = "cumulative")]
    Cumulative,
}

// the map viewport of the `places` section (not in the contract file: its
// StatsRequest had nowhere to say which part of the map is shown)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
pub struct PlacesView {
    // 0..=19; the cells are tiles at zoom + 3
    pub zoom: u8,
    // west, south, east, north in degrees; absent: the whole world
    pub bbox: Option<(f64, f64, f64, f64)>,
    // also send the `heat` points (not in the contract file)
    pub heat: Option<bool>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StatsRequest {
    pub scope: StatsScope,
    pub filter: StatsFilter,
    pub sections: Vec<StatsSection>,
    // the response carries both the per-year and cumulative counts, so the
    // toggle needs no round trip; accepted for the contract's sake
    pub year_mode: Option<YearMode>,
    pub etag: Option<String>,
    pub places: Option<PlacesView>,
}

// ---------------------------------------------------------------- definitions

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FocalDef {
    pub id: FocalBucketId,
    pub label: String,
    pub from_mm: f64,
    pub to_mm: Option<f64>,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ApertureDef {
    pub id: ApertureGroupId,
    pub label: String,
    pub from_f: f64,
    pub to_f: Option<f64>,
    pub name: String,
}

#[derive(Clone, Serialize, Deserialize, Type)]
pub struct ShutterDef {
    pub id: ShutterRegimeId,
    pub label: String,
    pub from: f64,
    pub to: Option<f64>,
    pub caption: String,
}

#[derive(Clone, Serialize, Deserialize, Type)]
pub struct Defs {
    pub focal: Vec<FocalDef>,
    pub aperture: Vec<ApertureDef>,
    pub shutter: Vec<ShutterDef>,
}

// ---------------------------------------------------------------- response

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct StatsResponse {
    pub etag: String,
    pub generated_at: String,
    pub coverage: Coverage,
    pub defs: Defs,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overview: Option<Overview>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cameras: Option<Cameras>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lenses: Option<Lenses>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exposure: Option<Exposure>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<TimeStats>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub places: Option<Places>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top: Option<TopImages>,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NotModified {
    pub not_modified: bool,
}

// `stats_query`'s reply: the statistics, or "your etag is still current"
#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(untagged)]
pub enum StatsReply {
    Full(Box<StatsResponse>),
    NotModified(NotModified),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "kebab-case")]
pub enum CoverageState {
    Ready,
    Indexing,
    Paused,
    NoRoots,
    Empty,
}

#[derive(Clone, Serialize, Deserialize, Type)]
pub struct PendingRoot {
    pub folder: String,
    pub pending: u32,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Coverage {
    // photos in the scope whose metadata is in the cache: what every number is based on
    pub indexed: u32,
    // photos the cache knows in the scope, indexed or not; null while no folder is added
    pub total: Option<u32>,
    // photos queued for analysis right now (library wide, as Find reports it)
    pub pending: u32,
    pub roots: u32,
    pub pending_roots: Vec<PendingRoot>,
    pub paused: bool,
    // how old the numbers can be, seconds
    pub max_age_sec: u32,
    pub state: CoverageState,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize, Type)]
pub struct Matches {
    pub n: u32,
    pub top: u32,
}

#[derive(Clone, Serialize, Deserialize, Type)]
pub struct YearCount {
    pub year: i32,
    pub n: u32,
    pub top: u32,
}

#[derive(Clone, Copy, Default, Serialize, Deserialize, Type)]
pub struct LabelCounts {
    pub red: u32,
    pub yellow: u32,
    pub green: u32,
    pub blue: u32,
    pub purple: u32,
}

#[derive(Clone, Serialize, Deserialize, Type)]
pub struct Count {
    pub n: u32,
    pub top: u32,
}

#[derive(Clone, Serialize, Deserialize, Type)]
pub struct FileTypeCount {
    #[serde(rename = "type")]
    pub kind: String,
    pub n: u32,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Gear {
    pub bodies: u32,
    pub lenses: u32,
    pub makers: u32,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FavouriteFocal {
    pub mm: u32,
    // photos at this focal length / photos with a focal length
    pub share: f64,
    // the same among top photos only
    pub top_share: f64,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct MedianSetup {
    pub focal_mm: Option<f64>,
    pub f_number: Option<f64>,
    pub shutter_sec: Option<f64>,
    pub iso: Option<f64>,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Peak {
    pub hour: Option<u8>,
    pub month: Option<u8>,
    pub month_n: u32,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(untagged)]
pub enum ParamValue {
    Text(String),
    Number(f64),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Type)]
pub enum InsightId {
    #[serde(rename = "keeper-body")]
    KeeperBody,
    #[serde(rename = "keeper-lens")]
    KeeperLens,
    #[serde(rename = "favourite-focal")]
    FavouriteFocal,
    #[serde(rename = "best-aperture")]
    BestAperture,
    #[serde(rename = "peak-month")]
    PeakMonth,
    #[serde(rename = "handheld-share")]
    HandheldShare,
    #[serde(rename = "golden-hour")]
    GoldenHour,
}

// the sentence's parts, not prose: the page formats them
#[derive(Clone, Serialize, Deserialize, Type)]
pub struct Insight {
    pub id: InsightId,
    pub params: BTreeMap<String, ParamValue>,
    pub apply: Option<StatsFilter>,
}

#[derive(Clone, Serialize, Deserialize, Type)]
pub struct AspectCount {
    pub id: AspectClassId,
    pub n: u32,
    pub top: u32,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Overview {
    pub matches: Matches,
    pub photos: u32,
    pub folders: u32,
    pub first_year: Option<i32>,
    pub last_year: Option<i32>,
    pub per_year: Vec<YearCount>,
    // 0..5 stars, rejected photos excluded
    pub rating_dist: [u32; 6],
    pub rejected: u32,
    pub labels: LabelCounts,
    pub gear: Gear,
    pub favourite_focal: Option<FavouriteFocal>,
    pub median_setup: MedianSetup,
    pub peak: Peak,
    pub file_types: Vec<FileTypeCount>,
    pub edited: u32,
    pub insights: Vec<Insight>,
    // orientation classes of the matches (not in the contract file; the filter
    // has an aspect field and this is its chart). "portrait" counts upright
    // frames of any ratio, so the other ids' counts overlap with it
    pub aspect: Vec<AspectCount>,
}

// ---- photos

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum ThumbRef {
    Cache { id: String },
    None,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PhotoRef {
    // the session handle of library_ids.rs, the one Find hands out
    pub id: String,
    pub path: String,
    pub thumb: ThumbRef,
    pub rating: u8,
    pub rejected: bool,
    pub label: Option<ColorLabel>,
    pub body: Option<String>,
    pub lens: Option<String>,
    pub focal_mm: Option<f64>,
    pub f_number: Option<f64>,
    pub shutter_sec: Option<f64>,
    pub iso: Option<f64>,
    // local capture time, ISO 8601 without a zone
    pub taken: Option<String>,
    pub place: Option<String>,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

// ---- cameras

#[derive(Clone, Serialize, Deserialize, Type)]
pub struct MakerRow {
    pub maker: String,
    pub n: u32,
    pub top: u32,
    pub bodies: u32,
}

#[derive(Clone, Copy, Serialize, Deserialize, Type)]
pub struct KeeperRate {
    pub top: u32,
    pub n: u32,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BodyRow {
    pub key: String,
    pub maker: String,
    pub model: String,
    pub n: u32,
    pub top: u32,
    pub keeper_rate: KeeperRate,
    pub cover: Option<PhotoRef>,
    pub first_shot: Option<String>,
    pub last_shot: Option<String>,
    pub serials: u32,
    pub top_lens: Option<String>,
    // always null: place names need geocoding the cache does not hold
    pub top_place: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Type)]
pub struct CameraYear {
    pub year: i32,
    pub n: u32,
    pub cumulative: u32,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct BodyTimeline {
    pub body: String,
    pub first: String,
    pub last: String,
    // aligned to Cameras.years
    pub per_year: Vec<u32>,
}

#[derive(Clone, Serialize, Deserialize, Type)]
pub struct Cameras {
    pub known: u32,
    pub makers: Vec<MakerRow>,
    pub bodies: Vec<BodyRow>,
    pub years: Vec<CameraYear>,
    pub timeline: Option<Vec<BodyTimeline>>,
}

// ---- lenses

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FocalRange {
    pub min_mm: f64,
    pub max_mm: f64,
    pub modal_mm: f64,
    // photos at the modal rounded focal length / the lens's photos with one
    pub modal_share: f64,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct LensRow {
    pub name: String,
    pub maker: Option<String>,
    pub n: u32,
    pub top: u32,
    pub keeper_rate: KeeperRate,
    pub focal_range: Option<FocalRange>,
    pub is_prime: bool,
    pub cover: Option<PhotoRef>,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FocalHist {
    // sparse [bin start mm, n, top]: 1 mm to 100, 5 mm to 300, then 25 mm
    pub bins: Vec<(u32, u32, u32)>,
    pub modal_mm: Option<u32>,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct FocalBucketRow {
    pub id: FocalBucketId,
    pub n: u32,
    pub top: u32,
    pub cover: Option<PhotoRef>,
    pub modal_mm: u32,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ZoomUsage {
    pub lens: String,
    pub min_mm: f64,
    pub max_mm: f64,
    // 16 log-spaced bins from min to max
    pub bins: Vec<u32>,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Lenses {
    pub known: u32,
    pub lenses: Vec<LensRow>,
    pub focal: FocalHist,
    pub focal_by_bucket: Vec<FocalBucketRow>,
    pub zoom_usage: Option<Vec<ZoomUsage>>,
}

// ---- exposure

#[derive(Clone, Serialize, Deserialize, Type)]
pub struct ExposureKnown {
    pub aperture: u32,
    pub shutter: u32,
    pub iso: u32,
}

#[derive(Clone, Serialize, Deserialize, Type)]
pub struct ApertureRow {
    pub id: ApertureGroupId,
    pub n: u32,
    pub top: u32,
    pub cover: Option<PhotoRef>,
}

#[derive(Clone, Serialize, Deserialize, Type)]
pub struct ShutterRow {
    pub id: ShutterRegimeId,
    pub n: u32,
    pub top: u32,
    pub cover: Option<PhotoRef>,
}

#[derive(Clone, Serialize, Deserialize, Type)]
pub struct IsoRow {
    pub id: IsoBinId,
    pub n: u32,
    pub top: u32,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct ExposureMedian {
    pub f_number: Option<f64>,
    pub shutter_sec: Option<f64>,
    pub iso: Option<f64>,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct Exposure {
    pub known: ExposureKnown,
    pub aperture: Vec<ApertureRow>,
    pub shutter: Vec<ShutterRow>,
    pub iso: Vec<IsoRow>,
    pub median: ExposureMedian,
    // 5 aperture groups x 6 shutter regimes
    pub aperture_by_shutter: Option<Vec<Vec<u32>>>,
}

// ---- time

#[derive(Clone, Serialize, Deserialize, Type)]
pub struct HourMonth {
    pub hour: u8,
    pub month: u8,
}

#[derive(Clone, Serialize, Deserialize, Type)]
pub struct TimeStats {
    pub known: u32,
    // 24 entries
    pub hours: Vec<Count>,
    // 12 entries, every year pooled
    pub months: Vec<Count>,
    pub years: Vec<YearCount>,
    // 7 entries, Monday first
    pub weekdays: Option<Vec<Count>>,
    // 12 months x 24 hours
    pub month_by_hour: Option<Vec<Vec<u32>>>,
    pub peak: HourMonth,
}

// ---- places

#[derive(Clone, Serialize, Deserialize, Type)]
pub struct PlaceCell {
    pub key: String,
    pub lat: f64,
    pub lng: f64,
    pub n: u32,
    pub top: u32,
    // a lone photo: the page draws a pin and opens it directly
    #[serde(skip_serializing_if = "Option::is_none")]
    pub solo: Option<PhotoRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Type)]
pub struct Places {
    pub known: u32,
    // distinct cells at the finest level (18)
    pub total: u32,
    pub zoom: u8,
    // west, south, east, north of every photo with a position under the filter
    pub bbox: Option<(f64, f64, f64, f64)>,
    pub cells: Vec<PlaceCell>,
    // lat, lng, weight; at most 2,000
    pub heat: Option<Vec<(f64, f64, f64)>>,
    pub top: Vec<PlaceCell>,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct KeyCount {
    pub key: String,
    pub n: u32,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct NameCount {
    pub name: String,
    pub n: u32,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct PlaceDetail {
    pub key: String,
    pub name: Option<String>,
    pub lat: f64,
    pub lng: f64,
    pub radius_km: f64,
    pub n: u32,
    pub top: u32,
    pub first_shot: Option<String>,
    pub last_shot: Option<String>,
    pub top_body: Option<KeyCount>,
    pub top_lens: Option<NameCount>,
    pub modal_focal_mm: Option<u32>,
    pub months: Vec<u32>,
    pub photos: Vec<PhotoRef>,
}

// ---- top images

#[derive(Clone, Serialize, Deserialize, Type)]
pub struct TopPage {
    pub next: Option<String>,
}

#[derive(Clone, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct TopImages {
    pub total: u32,
    pub items: Vec<PhotoRef>,
    pub page: TopPage,
    pub by_rating: [u32; 6],
    pub by_label: LabelCounts,
}

// `stats_top_page`'s reply
#[derive(Clone, Serialize, Deserialize, Type)]
pub struct TopPageReply {
    pub items: Vec<PhotoRef>,
    pub next: Option<String>,
}

// `stats_open_in_grid`'s reply: handles in the grid's order (rating, then recency)
#[derive(Clone, Serialize, Deserialize, Type)]
pub struct GridIds {
    pub ids: Vec<String>,
    // false when the list was cut at the cap
    pub complete: bool,
}
