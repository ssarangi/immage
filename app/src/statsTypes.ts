// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// A local copy of the Statistics data contract (notes/statistics-data-contract.ts) used until the
// generated types in bindings.ts exist. When bindings.ts has them, re-export from there and delete this copy.
/**
 * Statistics view: data contract between the Rust backend (search-cache aggregates) and the
 * front end. Everything is PRECOMPUTED: the UI only draws. Types are TypeScript-style; the Rust
 * side derives them with specta (typed_stats.rs) so bindings.ts stays generated.
 *
 * Conventions
 *  - Counts are integers. Shares are computed by the UI from counts (never sent as floats),
 *    except `keeperRate`, which is sent as top/total counts so the UI can divide.
 *  - "top" = rating >= Filter.minRating (default 4). Every aggregate carries `n` (all frames
 *    matching the filter, minus the aggregate's own dimension, see "Cross-filtering") and `top`.
 *  - Times are the camera's local capture time (EXIF DateTimeOriginal, no timezone).
 *  - Missing data is never zero: each aggregate block reports `known` (photos that had the field)
 *    so charts can say "44,602 / 46,880 with camera data".
 *  - Ids: `PhotoRef.id` is the opaque handle of library_ids.rs (same one Find uses).
 *  - Payload budget: whole response <= ~120 KB for 300k photos (histograms, top-N lists, capped
 *    cell lists). Photo lists are never sent beyond the caps noted below.
 */

// ---------------------------------------------------------------- request

export type StatsScope =
  | { kind: "library" }
  | { kind: "folder"; folder: string; includeSubfolders: boolean }
  | { kind: "dateRange"; from: string /* YYYY-MM-DD */; to: string };

/** The one filter object. Every chart element click sets/clears exactly one field. */
export interface StatsFilter {
  minRating: 0 | 1 | 2 | 3 | 4 | 5;   // threshold for "top images"; default 4. NOT a row filter by itself
  onlyTop: boolean;                   // true: restrict every aggregate to top images
  labels?: ColorLabel[];              // any of
  makers?: string[];                  // normalised maker ("Sony", "Canon", "Apple")
  bodies?: BodyKey[];                 // maker + model (+ nothing about serial)
  lenses?: string[];                  // normalised lens name
  years?: number[];                   // capture year
  months?: number[];                  // 1..12
  hours?: number[];                   // 0..23
  weekdays?: number[];                // 0..6, Monday = 0 (optional chart, later)
  focalBuckets?: FocalBucketId[];
  apertureGroups?: ApertureGroupId[];
  shutterRegimes?: ShutterRegimeId[];
  isoBins?: IsoBinId[];
  aspect?: AspectClassId[];
  fileTypes?: string[];               // "ARW", "CR3", "JPG", ...
  place?: PlaceKey[];                 // cell keys from the map at ANY zoom level (prefix match)
  edited?: boolean;
  /** free: keep the user's Find filter out of here; Statistics has its own filter state. */
}

export interface StatsRequest {
  scope: StatsScope;
  filter: StatsFilter;
  /** which blocks the UI needs now; lets the backend skip work (S1 ships overview+cameras+lenses). */
  sections: StatsSection[];
  /** gear-by-year chart toggle: "year" = photos in that year, "cumulative" = up to that year */
  yearMode?: "that-year" | "cumulative";
  /** cache validation: the response echoes it; unchanged => backend may answer {notModified:true} */
  etag?: string;
  /** the map's viewport (not in the first contract draft): cells are tiles at zoom + 3; `heat` asks for the heat points too */
  places?: { zoom: number; bbox?: [west: number, south: number, east: number, north: number]; heat?: boolean };
}

export type StatsSection =
  | "overview" | "cameras" | "lenses" | "exposure" | "time" | "places" | "top";

export type ColorLabel = "red" | "yellow" | "green" | "blue" | "purple";
export type BodyKey = string; // `${maker}|${model}`, e.g. "Sony|ILCE-7M4"
export type PlaceKey = string; // quadkey-like, see PlaceCell

// ---------------------------------------------------------------- buckets (definitions)

/** The backend owns these definitions and sends them once as `defs` so the UI can label bars. */
export type FocalBucketId = "uw" | "wide" | "normal" | "portrait" | "tele" | "super";
export const FOCAL_BUCKETS: readonly FocalDef[] = [
  // 35 mm equivalent when the EXIF has it, else the real focal length (crop-factor lookup is optional)
  { id: "uw",       label: "Ultra-wide", fromMm: 0,   toMm: 20  },  // [0, 20]
  { id: "wide",     label: "Wide",       fromMm: 21,  toMm: 34  },
  { id: "normal",   label: "Normal",     fromMm: 35,  toMm: 69  },
  { id: "portrait", label: "Portrait",   fromMm: 70,  toMm: 135 },
  { id: "tele",     label: "Telephoto",  fromMm: 136, toMm: 300 },
  { id: "super",    label: "Super-tele", fromMm: 301, toMm: null },
];

export type ApertureGroupId = "a1" | "a2" | "a3" | "a4" | "a5";
export const APERTURE_GROUPS: readonly ApertureDef[] = [
  { id: "a1", label: "f/1.2-1.8", fromF: 0,    toF: 1.8,  name: "Wide open" },     // <= 1.8
  { id: "a2", label: "f/2-2.8",   fromF: 1.81, toF: 2.8,  name: "Fast" },
  { id: "a3", label: "f/4-5.6",   fromF: 2.81, toF: 5.6,  name: "Mid" },
  { id: "a4", label: "f/8-11",    fromF: 5.61, toF: 11,   name: "Landscape" },
  { id: "a5", label: "f/16+",     fromF: 11.01, toF: null, name: "Stopped down" },
];

export type ShutterRegimeId = "freeze" | "fast" | "handheld" | "slow" | "motion" | "long";
export const SHUTTER_REGIMES: readonly ShutterDef[] = [
  // exposure time in SECONDS, half-open [from, to). Edges sit between nominal stops so 1/1000 and
  // 1/60 (which cameras record as 0.001 and 0.0166) land in the bucket the label names.
  { id: "freeze",   label: "Freeze",   from: 0,       to: 1 / 750, caption: ">= 1/1000" },
  { id: "fast",     label: "Fast",     from: 1 / 750, to: 1 / 180, caption: "1/250-1/500" },
  { id: "handheld", label: "Handheld", from: 1 / 180, to: 1 / 45,  caption: "1/60-1/200" },
  { id: "slow",     label: "Slow",     from: 1 / 45,  to: 1 / 6,   caption: "1/15-1/50" },
  { id: "motion",   label: "Motion",   from: 1 / 6,   to: 1.0001,  caption: "1/8-1 s" },
  { id: "long",     label: "Long",     from: 1.0001,  to: null,    caption: "> 1 s" },
];
// The captions are display copy (i18n keys); `from`/`to` are authoritative.

export type IsoBinId = "100" | "200" | "400" | "800" | "1600" | "3200" | "6400+";
// Bin = nearest stop at or below: <150 -> 100, <300 -> 200, <600 -> 400, <1200 -> 800, <2400 -> 1600, <4800 -> 3200, else 6400+

export type AspectClassId = "16:9" | "3:2" | "4:3" | "1:1" | "5:4" | "portrait" | "pano" | "other";
// Classified on the orientation-corrected long/short side ratio within +-2%: 1.78 -> 16:9, 1.5 -> 3:2, 1.33 -> 4:3,
// 1.25 -> 5:4, 1.0 -> 1:1, > 2.0 -> pano; portrait is a flag, not a class: see Aspect.portraitN

export type Daypart = "night" | "early" | "day" | "late"; // by hour only (no sun maths): 21-4 night, 5-8 early, 9-15 day, 16-20 late
// Hour windows are display hints; they are NOT solar-accurate and the UI says "early and late light".

// ---------------------------------------------------------------- response

export interface FocalDef { id: FocalBucketId; label: string; fromMm: number; toMm: number | null }
export interface ApertureDef { id: ApertureGroupId; label: string; fromF: number; toF: number | null; name: string }
export interface ShutterDef { id: ShutterRegimeId; label: string; from: number; to: number | null; caption: string }

export interface StatsResponse {
  etag: string;
  generatedAt: string;          // ISO
  coverage: Coverage;
  defs: { focal: FocalDef[]; aperture: ApertureDef[]; shutter: ShutterDef[] };
  overview?: Overview;
  cameras?: Cameras;
  lenses?: Lenses;
  exposure?: Exposure;
  time?: TimeStats;
  places?: Places;
  top?: TopImages;
}

export interface Coverage {
  /** photos in the scope that are in the search cache and analysed (what every number is based on) */
  indexed: number;
  /** photos on disk in the indexed roots (from folder listings); null while unknown */
  total: number | null;
  /** photos queued for analysis right now */
  pending: number;
  roots: number;                 // search roots (folders added)
  pendingRoots: { folder: string; pending: number }[]; // capped 5, for "2 folders"
  paused: boolean;               // search_pause
  /** staleness hint: a rating edit may lag behind the cache */
  maxAgeSec: number;
  /** why total is not complete: */
  state: "ready" | "indexing" | "paused" | "no-roots" | "empty";
}

/** Matches count under the current filter, shown in the sticky filter bar + "Open N in grid". */
export interface Matches { n: number; top: number }

export interface Overview {
  matches: Matches;
  photos: number;               // = matches.n
  folders: number;
  firstYear: number | null; lastYear: number | null;
  perYear: { year: number; n: number; top: number }[];          // sparkline, <= 30
  ratingDist: [number, number, number, number, number, number]; // 0..5 stars; rejected excluded
  rejected: number;
  labels: Record<ColorLabel, number>;
  gear: { bodies: number; lenses: number; makers: number };
  favouriteFocal: { mm: number; share: number; topShare: number } | null; // modal rounded focal (35 mm eq)
  medianSetup: { focalMm: number | null; fNumber: number | null; shutterSec: number | null; iso: number | null };
  peak: { hour: number | null; month: number | null; monthN: number };
  fileTypes: { type: string; n: number }[];  // <= 8
  edited: number;
  insights: Insight[];          // <= 6, server-ranked (see Insight)
}

/** Pre-written sentence parts, not prose: the UI formats with i18n (keys + params). */
export interface Insight {
  id: "keeper-body" | "keeper-lens" | "favourite-focal" | "best-aperture" | "peak-month" | "handheld-share" | "golden-hour";
  params: Record<string, string | number>;
  /** a filter that makes the sentence clickable (e.g. keeper-lens -> {lenses:[...]}) */
  apply?: Partial<StatsFilter>;
}

// ---- cameras

export interface Cameras {
  known: number;                              // photos with maker+model
  makers: MakerRow[];                         // sorted by n, all (<= 40); UI shows top 5 + "Other"
  bodies: BodyRow[];                          // sorted by n, <= 40
  years: { year: number; n: number; cumulative: number }[];  // era chips counts: "all", "<=min", then years
  /** body introduced/last-seen per year for the optional gear timeline (S5) */
  timeline?: { body: BodyKey; first: string; last: string; perYear: number[] /* aligned to years */ }[];
}
export interface MakerRow { maker: string; n: number; top: number; bodies: number }
export interface BodyRow {
  key: BodyKey; maker: string; model: string;
  n: number; top: number;
  keeperRate: { top: number; n: number };     // UI divides; lets it grey out n < 100
  cover: PhotoRef | null;                     // best rated, newest, in the filtered set
  firstShot: string | null; lastShot: string | null; // YYYY-MM-DD
  serials: number;                            // distinct body serials (shutter-count-free "copies owned")
  topLens: string | null;
  topPlace: string | null;
}

// ---- lenses

export interface Lenses {
  known: number;
  lenses: LensRow[];            // sorted by n, <= 40, min 20 photos each (min is a parameter, default 100 in UI copy)
  focal: FocalHist;             // whole-library focal length distribution
  focalByBucket: { id: FocalBucketId; n: number; top: number; cover: PhotoRef | null; modalMm: number }[];
  /** zoom lenses only: how often each focal end is used (heat strip along the range) */
  zoomUsage?: { lens: string; minMm: number; maxMm: number; bins: number[] /* 16 log-spaced bins */ }[];
}
export interface LensRow {
  name: string;                 // normalised
  maker: string | null;
  n: number; top: number;
  keeperRate: { top: number; n: number };
  focalRange: { minMm: number; maxMm: number; modalMm: number; modalShare: number /* count/n of modal rounded mm */ } | null;
  isPrime: boolean;
  cover: PhotoRef | null;
}
export interface FocalHist {
  /** log-ish bins: 1 mm resolution to 100 mm, then 5 mm to 300, then 25 mm; sparse array of [mm, n, top] */
  bins: [mm: number, n: number, top: number][];
  modalMm: number | null;
}

// ---- exposure

export interface Exposure {
  known: { aperture: number; shutter: number; iso: number };
  aperture: { id: ApertureGroupId; n: number; top: number; cover: PhotoRef | null }[];   // always 5, may be 0
  shutter: { id: ShutterRegimeId; n: number; top: number; cover: PhotoRef | null }[];    // always 6
  iso: { id: IsoBinId; n: number; top: number }[];                                       // always 7
  median: { fNumber: number | null; shutterSec: number | null; iso: number | null };
  /** the 'light story': aperture x shutter heat grid (5 x 6), counts; for the optional later view */
  apertureByShutter?: number[][];
}

// ---- time

export interface TimeStats {
  known: number;                                  // photos with a capture time
  hours: { n: number; top: number }[];            // length 24
  months: { n: number; top: number }[];           // length 12 (all years pooled)
  years: { year: number; n: number; top: number }[];
  weekdays?: { n: number; top: number }[];        // length 7, Monday first
  /** 12 x 24 month by hour matrix for the optional heat view */
  monthByHour?: number[][];
  peak: { hour: number; month: number };
}

// ---- places

/**
 * Photos with GPS are pre-bucketed into grid cells at fixed zoom levels. A cell key is a
 * quadkey (Web Mercator tiles), so a child's key starts with its parent's key and the UI can zoom
 * without another round trip. Levels sent: the requested `zoom` plus nothing else; the UI asks again
 * (debounced) on zoom/pan. The backend caps cells at 1,500 per response, smallest cells dropped
 * into their parent first, so payload stays < 60 KB.
 */
export interface Places {
  known: number;                // photos with GPS
  total: number;                // distinct cells at maxZoom (the "1,918 places")
  zoom: number;                 // the zoom the cells are for (cells = tiles at zoom + 3, i.e. 8x8 sub-grid)
  bbox: [west: number, south: number, east: number, north: number] | null; // null: no positioned photos
  cells: PlaceCell[];           // inside bbox (+10% margin) only
  heat?: [lat: number, lng: number, w: number][]; // alternative to cells for the Heat mode, <= 2,000 points
  top: PlaceCell[];             // the 12 biggest cells at the zoom, for the quick-jump chips
}
export interface PlaceCell {
  key: PlaceKey;                // quadkey
  lat: number; lng: number;     // weighted centroid (6 decimals max 5 is enough; 5 sent)
  n: number; top: number;
  /** a lone photo: UI draws a pin and opens it directly */
  solo?: PhotoRef;
  name?: string;                // reverse-geocoded, only when Geoapify is configured and cached
}
/** Sent when a cell is selected (separate command stats_place(key, filter)): the side panel. */
export interface PlaceDetail {
  key: PlaceKey;
  name: string | null;          // null => the panel shows coordinates and an "Add place names" hint
  lat: number; lng: number; radiusKm: number;
  n: number; top: number;
  firstShot: string | null; lastShot: string | null;
  topBody: { key: BodyKey; n: number } | null;
  topLens: { name: string; n: number } | null;
  modalFocalMm: number | null;
  months: number[];             // length 12
  photos: PhotoRef[];           // 6 best (rating desc, newest)
}

// ---- top images

export interface TopImages {
  total: number;                // matches with rating >= minRating under the filter
  items: PhotoRef[];            // first page: 24
  page: { next: string | null }; // opaque cursor; command stats_top_page(cursor) returns {items, next}
  /** facet strip above the grid */
  byRating: [number, number, number, number, number, number];
  byLabel: Record<ColorLabel, number>;
}

export interface PhotoRef {
  id: string;                   // library handle
  path: string;                 // for open-in-lightbox when the id is stale
  thumb: ThumbRef;
  rating: number;               // 0..5
  rejected: boolean;
  label: ColorLabel | null;     // first set label
  body: string | null;          // "Sony ILCE-7M4"
  lens: string | null;
  focalMm: number | null; fNumber: number | null; shutterSec: number | null; iso: number | null;
  taken: string | null;         // ISO local
  place: string | null;         // name or "38.72, -9.14"
  width: number | null; height: number | null;
}
/** The thumbnails come from the existing thumbnail cache, not from this payload. */
export type ThumbRef = { kind: "cache"; id: string } | { kind: "none" };

// ---------------------------------------------------------------- cross-filtering semantics
/**
 * Every aggregate X is computed with the filter MINUS the filter fields X itself edits, so the
 * chart still shows the alternatives you can switch to (a faceted-search model):
 *   - cameras.makers/bodies ignore filter.makers/bodies
 *   - lenses.lenses ignores filter.lenses; lenses.focal ignores focalBuckets
 *   - exposure.aperture ignores apertureGroups; shutter ignores shutterRegimes; iso ignores isoBins
 *   - time.hours ignores hours; time.months ignores months; time.years ignores years
 *   - places.cells ignores place
 *   - overview + matches use the full filter
 * The UI marks the selected elements (see spec) and shows the removable chip bar.
 */

// ---------------------------------------------------------------- commands

/**
 * Rust commands (stats_cmds.rs, typed in typed_stats.rs, listed in collect_commands!):
 *   stats_query(req: StatsRequest): StatsResponse | { notModified: true }   // async, off the UI thread
 *   stats_place(key: PlaceKey, filter: StatsFilter, scope: StatsScope): PlaceDetail
 *   stats_top_page(cursor: string): { items: PhotoRef[]; next: string | null }
 *   stats_open_in_grid(scope, filter): { ids: string[] }  // or: hands the filter to Find (see spec)
 *   stats_coverage(): Coverage                              // cheap, polled every 2 s while indexing
 * Event: "stats_dirty" (search cache changed: rating edit, indexing finished) => UI re-queries, debounced 1 s.
 */
