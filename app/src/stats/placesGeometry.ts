// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Places map's arithmetic, with no imports besides types so verify/stats-places.test.mjs runs it as it is:
// quadkeys to tiles, bounds and centroids, Web Mercator pixels, bubble sizes, the merge of overlapping bubbles
// at a zoom, the graticule of the offline fallback, which viewport changes need a new request, and a debounce.
import type { PlaceCell } from "../statsTypes";

/** west, south, east, north in degrees */
export type Bbox = [west: number, south: number, east: number, north: number];

export const MIN_ZOOM = 2;
export const MAX_ZOOM = 12;
/** Web Mercator stops here */
export const MAX_LAT = 85.0511287798;
/** the backend keeps a 10 % margin around the box it was asked for (stats.rs `places`) */
export const BACKEND_MARGIN = 0.1;

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

// ---------------------------------------------------------------- quadkeys

export interface TileRef { level: number; x: number; y: number }

/** Level, x and y of a quadkey (digits 0..3, x bit 1, y bit 2: Bing's scheme); null for anything else. */
export function parseKey(key: string): TileRef | null {
  if (key.length > 22) return null;
  let x = 0, y = 0;
  for (let i = 0; i < key.length; i++) {
    const d = key.charCodeAt(i) - 48;
    if (!(d >= 0 && d < 4)) return null;
    x = x * 2 + (d & 1);
    y = y * 2 + (d >> 1);
  }
  return { level: key.length, x, y };
}

export const keyOf = (level: number, x: number, y: number): string => {
  let out = "";
  for (let i = level; i >= 1; i--) {
    const bit = Math.pow(2, i - 1);
    out += ((Math.floor(x / bit) & 1) | ((Math.floor(y / bit) & 1) << 1)).toString();
  }
  return out;
};

const lngOfX = (x: number, n: number) => (x / n) * 360 - 180;
const latOfY = (y: number, n: number) => (Math.atan(Math.sinh(Math.PI * (1 - (2 * y) / n))) * 180) / Math.PI;

/** The tile's box; the level-0 key "" is the whole map. */
export function tileBounds(key: string): Bbox | null {
  const t = parseKey(key);
  if (!t) return null;
  const n = Math.pow(2, t.level);
  return [lngOfX(t.x, n), latOfY(t.y + 1, n), lngOfX(t.x + 1, n), latOfY(t.y, n)];
}

/** The middle of the tile, as [lat, lng] (the cells carry their own weighted centroid; this is for a key alone). */
export function tileCentroid(key: string): [number, number] | null {
  const b = tileBounds(key);
  return b ? [(b[1] + b[3]) / 2, (b[0] + b[2]) / 2] : null;
}

/** Whether a cell lies inside a selected area: the same tile, or below it (a place filter is a prefix match). */
export const inside = (cellKey: string, selected: string) => cellKey.startsWith(selected);

/** Width of a tile in km at its latitude. */
export const tileWidthKm = (level: number, lat: number) => (40075.017 * Math.cos((lat * Math.PI) / 180)) / Math.pow(2, level);

// ---------------------------------------------------------------- projection

/** World pixels at a zoom (256 px tiles), origin at the top left of the world. */
export function project(lat: number, lng: number, zoom: number): { x: number; y: number } {
  const size = 256 * Math.pow(2, zoom);
  const s = Math.sin((clamp(lat, -MAX_LAT, MAX_LAT) * Math.PI) / 180);
  return { x: ((lng + 180) / 360) * size, y: (0.5 - Math.log((1 + s) / (1 - s)) / (4 * Math.PI)) * size };
}

// ---------------------------------------------------------------- bubbles

export type Mode = "photos" | "top" | "heat";
export const RADIUS_CAP = 58;
export const RADIUS_MIN = 11;
/**
 * A bubble's radius: the area follows the count, relative to the biggest cell of the answer (`max`), from
 * RADIUS_MIN to RADIUS_CAP. A merged bubble can go past the biggest cell and stops at the cap.
 */
export const bubbleRadius = (value: number, max = 1000) => RADIUS_MIN + (RADIUS_CAP - RADIUS_MIN) * Math.sqrt(Math.min(1, Math.max(0, value) / Math.max(1, max)));
export const SOLO_RADIUS = 6;
/** a label fits inside from here on */
export const LABEL_MIN = 100;

/** 1,204 -> "1.2k", 12,340 -> "12k", 1,230,000 -> "1.2M"; below 1,000 the number as it is. */
export function compact(n: number): string {
  if (n < 1000) return String(Math.round(n));
  const one = (v: number, unit: string) => `${v >= 10 ? Math.round(v) : Math.round(v * 10) / 10}${unit}`;
  return n < 1e6 ? one(n / 1e3, "k") : one(n / 1e6, "M");
}

export interface Bubble {
  /** the biggest member's key: what a click selects */
  key: string;
  keys: string[];
  lat: number;
  lng: number;
  /** photos, and top photos, of every member */
  n: number;
  top: number;
  /** more than one cell merged here: a click zooms to it */
  merged: boolean;
  /** a lone photo */
  solo?: PlaceCell["solo"];
  /** the pixel position and radius at the zoom they were made for */
  x: number;
  y: number;
  r: number;
  /** the bounding box of the members' centroids, to zoom to */
  box: Bbox;
}

const valueOf = (c: { n: number; top: number }, mode: Mode) => (mode === "top" ? c.top : c.n);

/**
 * Bubbles for the cells at a zoom: biggest first, and a cell that lands under an already placed bubble
 * (their circles overlap by more than `overlap` of the smaller radius) is merged into it, the centre moving
 * to the weighted centroid. The sum of the counts is kept. `px` maps a position to pixels at the map's zoom.
 * In Top mode a cell with no top photo is left out.
 */
export function bubbles(cells: readonly PlaceCell[], px: (lat: number, lng: number) => { x: number; y: number }, mode: Mode = "photos", overlap = 0.45): Bubble[] {
  const sorted = cells.filter((c) => valueOf(c, mode) > 0).sort((a, b) => valueOf(b, mode) - valueOf(a, mode) || (a.key < b.key ? -1 : 1));
  const out: Bubble[] = [];
  const weight = new Map<Bubble, number>();
  const max = sorted.length ? valueOf(sorted[0], mode) : 1;
  const radius = (b: Bubble) => (b.n === 1 && !b.merged ? SOLO_RADIUS : bubbleRadius(valueOf(b, mode), max));
  for (const c of sorted) {
    const p = px(c.lat, c.lng);
    const solo = c.n === 1;
    const r = solo ? SOLO_RADIUS : bubbleRadius(valueOf(c, mode), max);
    let host: Bubble | undefined;
    for (const b of out) {
      const d = Math.hypot(b.x - p.x, b.y - p.y);
      if (d < b.r + r - overlap * 2 * Math.min(b.r, r)) { host = b; break; }
    }
    if (!host) {
      const b: Bubble = { key: c.key, keys: [c.key], lat: c.lat, lng: c.lng, n: c.n, top: c.top, merged: false, solo: c.solo, x: p.x, y: p.y, r,
        box: [c.lng, c.lat, c.lng, c.lat] };
      out.push(b);
      weight.set(b, valueOf(c, mode));
      continue;
    }
    const w0 = weight.get(host)!, w1 = valueOf(c, mode);
    host.lat = (host.lat * w0 + c.lat * w1) / (w0 + w1);
    host.lng = (host.lng * w0 + c.lng * w1) / (w0 + w1);
    weight.set(host, w0 + w1);
    host.n += c.n;
    host.top += c.top;
    host.keys.push(c.key);
    host.merged = true;
    host.solo = undefined;
    host.box = [Math.min(host.box[0], c.lng), Math.min(host.box[1], c.lat), Math.max(host.box[2], c.lng), Math.max(host.box[3], c.lat)];
    const q = px(host.lat, host.lng);
    host.x = q.x;
    host.y = q.y;
    host.r = radius(host);
  }
  return out;
}

export type Dir = "left" | "right" | "up" | "down";

/**
 * The bubble an arrow key moves the focus to: among those in that direction (within 60 degrees of it), the one
 * nearest with a penalty for straying sideways; null at the edge.
 */
export function neighbour<T extends { x: number; y: number }>(list: readonly T[], from: T, dir: Dir): T | null {
  const ux = dir === "left" ? -1 : dir === "right" ? 1 : 0, uy = dir === "up" ? -1 : dir === "down" ? 1 : 0;
  let best: T | null = null, score = Infinity;
  for (const b of list) {
    if (b === from) continue;
    const dx = b.x - from.x, dy = b.y - from.y;
    const along = dx * ux + dy * uy, across = Math.abs(dx * uy - dy * ux);
    if (along <= 0 || across > along * 1.73) continue;
    const v = along + across * 2;
    if (v < score) { score = v; best = b; }
  }
  return best;
}

// ---------------------------------------------------------------- the offline graticule

/** A degree step giving four to eight lines across a span. */
export function graticuleStep(span: number): number {
  const steps = [90, 45, 30, 20, 10, 5, 2, 1, 0.5, 0.2, 0.1, 0.05, 0.02, 0.01];
  for (const s of steps) if (span / s >= 4) return s;
  return steps[steps.length - 1];
}

/** The meridians and parallels to draw over a box, on multiples of the step. */
export function graticule(box: Bbox, step = graticuleStep(Math.max(box[2] - box[0], (box[3] - box[1]) * 1.4))): { lngs: number[]; lats: number[]; step: number } {
  const run = (a: number, b: number, lo: number, hi: number) => {
    const out: number[] = [];
    for (let v = Math.ceil(Math.max(a, lo) / step) * step; v <= Math.min(b, hi) + 1e-9; v += step) out.push(Math.round(v * 1e6) / 1e6);
    return out.slice(0, 80);
  };
  return { lngs: run(box[0], box[2], -180, 180), lats: run(box[1], box[3], -MAX_LAT, MAX_LAT), step };
}

/** "38°N", "9.5°W": Latin units and compass letters, as "mm" and "f/" are. */
export function degrees(v: number, axis: "lat" | "lng", step = 1): string {
  const digits = step >= 1 ? 0 : step >= 0.1 ? 1 : 2;
  const a = String(Number(Math.abs(v).toFixed(digits)));
  const letter = axis === "lat" ? (v < 0 ? "S" : "N") : v < 0 ? "W" : "E";
  return `${a}°${v === 0 ? "" : letter}`;
}

/** "38.72° N, 9.14° W" */
export function coords(lat: number, lng: number, digits = 2): string {
  const f = (v: number, p: string, n: string) => `${Math.abs(v).toFixed(digits)}° ${v < 0 ? n : p}`;
  return `${f(lat, "N", "S")}, ${f(lng, "E", "W")}`;
}

/** A place filter's chip text: where its tile is centred ("47.5° N, 122.3° W"); the key itself if it is not one. */
export function placeText(key: string): string {
  const c = tileCentroid(key);
  return c ? coords(c[0], c[1], 1) : key;
}

// ---------------------------------------------------------------- viewport requests

export interface View { zoom: number; bbox: Bbox | null }

export const clampBbox = (b: Bbox): Bbox => [clamp(b[0], -180, 180), clamp(b[1], -MAX_LAT, MAX_LAT), clamp(b[2], -180, 180), clamp(b[3], -MAX_LAT, MAX_LAT)];
const round = (v: number) => Math.round(v * 1e4) / 1e4;
export const roundBbox = (b: Bbox): Bbox => [round(b[0]), round(b[1]), round(b[2]), round(b[3])];

function grown(b: Bbox, by: number): Bbox {
  const dx = (b[2] - b[0]) * by, dy = (b[3] - b[1]) * by;
  return [b[0] - dx, b[1] - dy, b[2] + dx, b[3] + dy];
}
const contains = (outer: Bbox, inner: Bbox) => inner[0] >= outer[0] && inner[1] >= outer[1] && inner[2] <= outer[2] && inner[3] <= outer[3];

/**
 * Whether a viewport needs a new answer given the one the cells on screen were asked with: another zoom does
 * (the cells are other tiles); the same zoom does only once the view leaves the box asked for plus the margin
 * the backend adds. An answer for the whole world (no box) covers every view at its zoom.
 */
export function needsRequest(asked: View | null, now: View): boolean {
  if (!asked) return true;
  if (asked.zoom !== now.zoom) return true;
  if (!asked.bbox) return false;
  if (!now.bbox) return true;
  return !contains(grown(asked.bbox, BACKEND_MARGIN), now.bbox);
}

/** A cell's tile fits the view's bounds, for a "fit to" request that wants the zoom whose cells are about this size. */
export function zoomFor(box: Bbox, widthPx: number, heightPx: number, min = MIN_ZOOM, max = MAX_ZOOM): number {
  for (let z = max; z >= min; z--) {
    const a = project(box[3], box[0], z), b = project(box[1], box[2], z);
    if (b.x - a.x <= widthPx && b.y - a.y <= heightPx) return z;
  }
  return min;
}

// ---------------------------------------------------------------- debounce

export interface Timers { set(fn: () => void, ms: number): unknown; clear(handle: unknown): void }
const realTimers: Timers = { set: (fn, ms) => setTimeout(fn, ms), clear: (h) => clearTimeout(h as ReturnType<typeof setTimeout>) };

/** The latest value after `ms` of quiet; `flush` runs it now, `cancel` drops it. Timers can be given for a test. */
export function debounce<T>(fn: (v: T) => void, ms: number, timers: Timers = realTimers) {
  let handle: unknown = null;
  let pending: { v: T } | null = null;
  const run = () => { handle = null; const p = pending; pending = null; if (p) fn(p.v); };
  return {
    call(v: T) { pending = { v }; if (handle !== null) timers.clear(handle); handle = timers.set(run, ms); },
    flush() { if (handle !== null) timers.clear(handle); run(); },
    cancel() { if (handle !== null) timers.clear(handle); handle = null; pending = null; },
    get waiting() { return pending !== null; },
  };
}
