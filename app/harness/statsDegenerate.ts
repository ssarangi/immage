// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Degenerate libraries for the Statistics harness (?deg=). Each one rewrites a recorded reply the way the
// backend would answer for a library of that shape, so every section can be checked for NaN, division by zero,
// empty labelled bars and crashes. Pure and import-free (types excepted), so verify/stats-degenerate.test.mjs
// loads it as it is.
//
//   one        a library of one photo (rated 5)
//   onecam     every photo from one body, one lens
//   nolens     no photo has lens data
//   nogps      no photo has a position (also ?gps=none)
//   unrated    no photo is rated: nothing is a top image
//   all5       every photo is rated 5
//   oneyear    every photo is from a single year
import type { StatsResponse } from "../src/statsTypes";

export const DEGENERATE = ["one", "onecam", "nolens", "nogps", "unrated", "all5", "oneyear"] as const;
export type Degenerate = (typeof DEGENERATE)[number];

type Row = { n: number; top: number; [k: string]: unknown };
type Obj = Record<string, unknown>;
/** lists with one slot per bucket (the chart always draws every slot); the others are lists of what exists */
const FIXED = new Set(["hours", "months", "weekdays", "aperture", "shutter", "iso", "focalByBucket"]);

const zero = (r: Row) => { r.n = 0; r.top = 0; };
function only(list: unknown, keep: number): void {
  // one photo in the slot `keep`, none in the others
  const rows = list as Row[];
  rows.forEach((r, i) => { if (i === keep) { r.n = 1; r.top = Math.min(r.top, 1) || 0; } else zero(r); });
}
const biggest = (rows: Row[]) => rows.reduce((b, r, i) => (r.n > rows[b].n ? i : b), 0);

/** the number of photos the shape leaves */
function shrink(o: Obj, photos: number): void {
  // one photo: keep the biggest entry of every list (a fixed list keeps its slots), everything else is 0 or 1
  for (const k of Object.keys(o)) {
    const v = o[k];
    if (Array.isArray(v) && v.length && v.every((x) => x && typeof x === "object" && !Array.isArray(x) && typeof (x as Row).n === "number")) {
      const rows = v as Row[];
      if (FIXED.has(k)) only(rows, biggest(rows));
      else { const at = biggest(rows); o[k] = [rows[at]]; only(o[k], 0); if (photos === 1) (o[k] as Row[])[0].top = 1; }
    } else if (v && typeof v === "object" && !Array.isArray(v)) shrink(v as Obj, photos);
  }
}

/** `top` follows the rule for the shape: none (unrated), all (all5), or as recorded */
function tops(o: unknown, mode: "none" | "all"): void {
  if (Array.isArray(o)) { o.forEach((x) => tops(x, mode)); return; }
  if (!o || typeof o !== "object") return;
  const r = o as Obj;
  if (typeof r.n === "number" && typeof r.top === "number") r.top = mode === "none" ? 0 : r.n;
  for (const k of Object.keys(r)) tops(r[k], mode);
}

export function degenerate<T extends Partial<StatsResponse>>(reply: T, kind: Degenerate): T {
  const r = reply as unknown as Obj;
  const ov = r.overview as Obj | undefined;
  const cam = r.cameras as Obj | undefined, len = r.lenses as Obj | undefined, tm = r.time as Obj | undefined;
  const topBlock = r.top as Obj | undefined, places = r.places as Obj | undefined, expo = r.exposure as Obj | undefined;
  if (kind === "one") {
    for (const k of ["overview", "cameras", "lenses", "exposure", "time", "places"]) if (r[k]) shrink(r[k] as Obj, 1);
    if (ov) {
      Object.assign(ov, { photos: 1, folders: 1, firstYear: 2024, lastYear: 2024, rejected: 0, edited: 0, ratingDist: [0, 0, 0, 0, 0, 1],
        gear: { bodies: 1, lenses: 1, makers: 1 }, perYear: [{ year: 2024, n: 1, top: 1 }], fileTypes: [{ type: "JPG", n: 1 }], insights: [] });
      ov.matches = { n: 1, top: 1 };
      ov.labels = { red: 0, yellow: 0, green: 0, blue: 0, purple: 0 };
      if (ov.favouriteFocal) ov.favouriteFocal = { ...(ov.favouriteFocal as Obj), share: 1, topShare: 1 };
    }
    if (cam) Object.assign(cam, { known: 1, years: [{ year: 2024, n: 1, cumulative: 1 }] });
    if (len) Object.assign(len, { known: 1 });
    if (expo) Object.assign(expo, { known: { aperture: 1, shutter: 1, iso: 1 }, apertureByShutter: undefined });
    if (tm) Object.assign(tm, { known: 1, years: [{ year: 2024, n: 1, top: 1 }], monthByHour: undefined });
    if (places) { Object.assign(places, { known: 1, total: 1 }); }
    if (topBlock) { const items = (topBlock.items as unknown[]).slice(0, 1); Object.assign(topBlock, { total: 1, items, page: { next: null }, byRating: [0, 0, 0, 0, 0, 1], byLabel: { red: 0, yellow: 0, green: 0, blue: 0, purple: 0 } }); }
    tops(r.cameras, "all"); tops(r.lenses, "all"); tops(r.exposure, "all"); tops(r.time, "all");
  } else if (kind === "onecam") {
    if (cam) {
      const makers = cam.makers as Row[], bodies = cam.bodies as Row[];
      const n = makers.reduce((a, m) => a + m.n, 0), top = makers.reduce((a, m) => a + m.top, 0);
      cam.makers = [{ ...makers[0], n, top, bodies: 1 }];
      cam.bodies = [{ ...bodies[0], n, top, keeperRate: { top, n }, maker: makers[0].maker }];
      cam.known = n;
    }
    if (ov) ov.gear = { bodies: 1, lenses: 1, makers: 1 };
    if (len) {
      const lenses = len.lenses as Row[];
      const n = lenses.reduce((a, l) => a + l.n, 0), top = lenses.reduce((a, l) => a + l.top, 0);
      len.lenses = [{ ...lenses[0], n, top, keeperRate: { top, n } }];
    }
  } else if (kind === "nolens") {
    if (len) {
      Object.assign(len, { known: 0, lenses: [], focal: { bins: [], modalMm: null }, zoomUsage: undefined });
      (len.focalByBucket as Row[] | undefined)?.forEach((b) => { zero(b); b.cover = null; });
    }
    if (ov) { (ov.gear as Obj).lenses = 0; ov.favouriteFocal = null; (ov.medianSetup as Obj).focalMm = null; }
  } else if (kind === "nogps") {
    if (places) Object.assign(places, { known: 0, total: 0, bbox: null, cells: [], top: [], heat: undefined });
  } else if (kind === "unrated") {
    tops(r, "none");
    if (ov) { ov.ratingDist = [Math.max(...[(ov.photos as number) ?? 0]), 0, 0, 0, 0, 0]; (ov.matches as Obj).top = 0; ov.insights = []; }
    if (topBlock) Object.assign(topBlock, { total: 0, items: [], page: { next: null }, byRating: [0, 0, 0, 0, 0, 0], byLabel: { red: 0, yellow: 0, green: 0, blue: 0, purple: 0 } });
    for (const k of ["bodies", "lenses"]) for (const row of ((r.cameras as Obj | undefined)?.[k] ?? (r.lenses as Obj | undefined)?.[k] ?? []) as Row[]) {
      if (row.keeperRate) row.keeperRate = { top: 0, n: row.n };
    }
  } else if (kind === "all5") {
    tops(r, "all");
    if (ov) { ov.ratingDist = [0, 0, 0, 0, 0, (ov.photos as number) ?? 0]; (ov.matches as Obj).top = ov.photos; ov.insights = []; }
    if (topBlock) Object.assign(topBlock, { total: (ov?.photos as number) ?? topBlock.total, byRating: [0, 0, 0, 0, 0, topBlock.total] });
    for (const row of [...((cam?.bodies ?? []) as Row[]), ...((len?.lenses ?? []) as Row[])]) if (row.keeperRate) row.keeperRate = { top: row.n, n: row.n };
  } else if (kind === "oneyear") {
    const year = 2024;
    if (ov) {
      Object.assign(ov, { firstYear: year, lastYear: year });
      const py = ov.perYear as Row[]; const n = py.reduce((a, y) => a + y.n, 0), top = py.reduce((a, y) => a + y.top, 0);
      ov.perYear = [{ year, n, top }];
    }
    if (cam) { const ys = cam.years as { n: number }[]; const n = ys.reduce((a, y) => a + y.n, 0); cam.years = [{ year, n, cumulative: n }]; }
    if (tm) { const ys = tm.years as Row[]; const n = ys.reduce((a, y) => a + y.n, 0), top = ys.reduce((a, y) => a + y.top, 0); tm.years = [{ year, n, top }]; }
  }
  return reply;
}
