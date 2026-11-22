// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The arithmetic behind the Statistics charts, kept out of the components so it can be
// tested (verify/stats-model.test.mjs) and so the charts only draw. Charts never
// invent numbers: everything here derives from the payload's counts. No imports.

export type Accent = "cyan" | "amber" | "green" | "violet" | "rose" | "neutral";
export type ShapeKind = "circle" | "square" | "diamond" | "triangle" | "hexagon" | "dash";

export const share = (n: number, total: number) => (total > 0 ? n / total : 0);

/** Keeper rate from the payload's top and total counts; null when there is nothing to divide. */
export const keeperRate = (r: { top: number; n: number }): number | null => (r.n > 0 ? r.top / r.n : null);

/** Fewer than this many frames: the rate is a hint, not a measure (hatched, "small sample"). */
export const MIN_SAMPLE = 100;
export const SMALL_AGGREGATE = 50;
export const isSmallSample = (n: number, min = MIN_SAMPLE) => n < min;

/** 12400 -> "12.4k", 1200000 -> "1.2M"; counts under 1000 stay whole. */
export function compact(n: number): string {
  const a = Math.abs(n);
  if (a < 1000) return String(Math.round(n));
  if (a < 10000) return `${trim(n / 1000)}k`;
  if (a < 1_000_000) return `${Math.round(n / 1000)}k`;
  return `${trim(n / 1_000_000)}M`;
}
const trim = (v: number) => (Math.round(v * 10) / 10).toString();

// ---- donut

const MAKER_STYLE: { accent: Accent; shape: ShapeKind }[] = [
  { accent: "cyan", shape: "circle" }, { accent: "amber", shape: "square" }, { accent: "green", shape: "diamond" },
  { accent: "violet", shape: "triangle" }, { accent: "rose", shape: "hexagon" },
];
export const OTHER_STYLE: { accent: Accent; shape: ShapeKind } = { accent: "neutral", shape: "dash" };
export const OTHER_ID = "\u0000other";

export interface Mark { id: string; label: string; n: number; top: number; accent: Accent; shape: ShapeKind; members?: string[] }

/** The five biggest makers, the rest as one "Other" slice (a series beyond six never needs its own colour). */
export function groupMakers(rows: readonly { maker: string; n: number; top: number }[], otherLabel: string, keep = 5): Mark[] {
  const sorted = [...rows].sort((a, b) => b.n - a.n || a.maker.localeCompare(b.maker));
  const head: Mark[] = sorted.slice(0, keep).map((r, i) => ({ id: r.maker, label: r.maker, n: r.n, top: r.top, ...MAKER_STYLE[i] }));
  const rest = sorted.slice(keep);
  if (rest.length) {
    head.push({ id: OTHER_ID, label: otherLabel, n: rest.reduce((a, r) => a + r.n, 0), top: rest.reduce((a, r) => a + r.top, 0),
      ...OTHER_STYLE, members: rest.map((r) => r.maker) });
  }
  return head;
}

export interface Arc { id: string; len: number; offset: number; fraction: number }
/** One circle's dash arrays: segments with `gap` between them, in order, from 12 o'clock. */
export function donutArcs(marks: readonly { id: string; n: number }[], radius: number, gap = 3): Arc[] {
  const circ = 2 * Math.PI * radius;
  const total = marks.reduce((a, m) => a + m.n, 0);
  let off = 0;
  return marks.map((m) => {
    const fraction = share(m.n, total);
    const len = fraction * circ;
    const arc = { id: m.id, len: Math.max(len - gap, marks.length > 1 ? 0.5 : len), offset: off, fraction };
    off += len;
    return arc;
  });
}

// ---- ranked rows

export type LensMode = "most-used" | "keeper" | "top";
export interface RankSource { name: string; n: number; top: number }

export function rankRows<T extends { n: number; top: number }>(rows: readonly T[], mode: LensMode, minSample = 1): T[] {
  const eligible = rows.filter((r) => r.n >= minSample);
  const by = (a: T, b: T) => {
    if (mode === "keeper") return (keeperRate(b) ?? 0) - (keeperRate(a) ?? 0) || b.n - a.n;
    if (mode === "top") return b.top - a.top || b.n - a.n;
    return b.n - a.n || b.top - a.top;
  };
  return [...eligible].sort(by);
}

/** Two-layer bar widths (0..1): `all` is the faint layer, `top` the bright one. */
export function barWidths(rows: readonly { n: number; top: number }[], mode: LensMode): { all: number; top: number }[] {
  const maxN = Math.max(1, ...rows.map((r) => r.n));
  const maxRate = Math.max(1e-9, ...rows.map((r) => keeperRate(r) ?? 0));
  return rows.map((r) => {
    const all = r.n / maxN;
    if (mode === "keeper") return { all: 1, top: Math.min(1, (keeperRate(r) ?? 0) / maxRate) };
    return { all, top: r.top / maxN };
  });
}

// ---- field of view

/** The horizontal angle of view on a 35 mm frame (36 mm wide), in radians. */
export const fovHalfAngle = (mm: number) => Math.atan(18 / Math.max(mm, 1));
export const fovDegrees = (mm: number) => (2 * fovHalfAngle(mm) * 180) / Math.PI;

export interface FanWedge { id: string; mm: number; len: number; x: number; y1: number; y2: number; labelX: number; labelY: number }
/** Wedges from the camera glyph at (ox, oy): wider lenses are the wider angles, busier buckets reach further. */
export function fanWedges(buckets: readonly { id: string; modalMm: number; n: number }[], ox: number, oy: number,
  inner = 90, reach = 210): FanWedge[] {
  const max = Math.max(1, ...buckets.map((b) => b.n));
  return [...buckets].sort((a, b) => a.modalMm - b.modalMm).map((b) => {
    const h = fovHalfAngle(b.modalMm);
    const len = inner + Math.sqrt(b.n / max) * reach;
    return { id: b.id, mm: b.modalMm, len, x: ox + len * Math.cos(h), y1: oy - len * Math.sin(h), y2: oy + len * Math.sin(h),
      labelX: ox + len * Math.cos(h), labelY: oy - len * Math.sin(h) };
  });
}

// ---- insights derived on the client, from the same counts the charts show

export interface BodyInsight { key: string; model: string; rate: number; avg: number; ratio: number }
/** The body whose keeper rate beats the library's by most, among bodies with enough frames. */
export function keeperBodyInsight(bodies: readonly { key: string; maker: string; model: string; n: number; top: number }[],
  min = MIN_SAMPLE): BodyInsight | null {
  const n = bodies.reduce((a, b) => a + b.n, 0);
  const avg = n ? bodies.reduce((a, b) => a + b.top, 0) / n : 0;
  const best = bodies.filter((b) => b.n >= min).sort((a, b) => (keeperRate(b) ?? 0) - (keeperRate(a) ?? 0))[0];
  if (!best || !avg) return null;
  const rate = keeperRate(best)!;
  const name = best.model.toLowerCase().startsWith(best.maker.toLowerCase()) ? best.model : `${best.maker} ${best.model}`.trim();
  return rate > avg ? { key: best.key, model: name, rate, avg, ratio: rate / avg } : null;
}

/** A zoom lens whose frames sit at one focal length (modalShare), when that differs from the favourite. */
export function lensFocalInsight(lenses: readonly { name: string; n: number; isPrime: boolean;
  focalRange: { modalMm: number; modalShare: number } | null }[], favouriteMm: number | null, min = MIN_SAMPLE) {
  return lenses.filter((l) => !l.isPrime && l.n >= min && l.focalRange && l.focalRange.modalShare >= 0.5
    && (favouriteMm === null || Math.abs(l.focalRange.modalMm - favouriteMm) > 2))
    .sort((a, b) => b.focalRange!.modalShare - a.focalRange!.modalShare)[0] ?? null;
}

// ---- tiny charts

/** Bars for a sparkline: values scaled to 0..1 against the largest. */
export const scaleBars = (values: readonly number[]) => {
  const max = Math.max(0, ...values);
  return values.map((v) => (max > 0 ? v / max : 0));
};

/** Folds a histogram of [mm, n, top] bins into at most `count` bars, in order. */
export function foldBins(bins: readonly (readonly [number, number, number])[], count: number): number[] {
  if (!bins.length) return [];
  const sorted = [...bins].sort((a, b) => a[0] - b[0]);
  const per = Math.ceil(sorted.length / count);
  const out: number[] = [];
  for (let i = 0; i < sorted.length; i += per) out.push(sorted.slice(i, i + per).reduce((a, b) => a + b[1], 0));
  return out;
}

/** A run of years as "2022", or "2014 to 2021" (the separator is the caller's). */
export function yearRuns(years: readonly number[]): [number, number][] {
  const sorted = [...years].sort((a, b) => a - b);
  const out: [number, number][] = [];
  for (const y of sorted) {
    const last = out[out.length - 1];
    if (last && y === last[1] + 1) last[1] = y; else out.push([y, y]);
  }
  return out;
}

/** The era chips: all, "up to" the earliest years folded together, then one chip per recent year. */
export function eraChips(years: readonly { year: number; n: number; cumulative: number }[], mode: "that-year" | "cumulative",
  maxChips = 5): { id: string; years: number[]; n: number; upTo?: number }[] {
  const sorted = [...years].sort((a, b) => a.year - b.year);
  if (!sorted.length) return [];
  const count = (y: { n: number; cumulative: number }) => (mode === "cumulative" ? y.cumulative : y.n);
  const out: { id: string; years: number[]; n: number; upTo?: number }[] = [];
  const recent = sorted.slice(-maxChips);
  const older = sorted.slice(0, sorted.length - recent.length);
  if (older.length) {
    const last = older[older.length - 1];
    out.push({ id: `<=${last.year}`, years: older.map((y) => y.year), n: mode === "cumulative" ? last.cumulative : older.reduce((a, y) => a + y.n, 0), upTo: last.year });
  }
  for (const y of recent) {
    out.push({ id: String(y.year), years: mode === "cumulative" ? sorted.filter((x) => x.year <= y.year).map((x) => x.year) : [y.year], n: count(y) });
  }
  return out;
}

/** Whether the selected years are exactly this chip. */
export const sameYears = (a: readonly number[], b: readonly number[]) => a.length === b.length && [...a].sort().join() === [...b].sort().join();
