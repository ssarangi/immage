// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The arithmetic behind the Exposure and Time charts (aperture rings, shutter bars, ISO ladder, the 24-hour
// clock, seasonality): geometry, bar heights, dayparts and the insights, all from the payload's counts. No
// imports, so verify/stats-model.test.mjs loads it as it is.

export type ExpAccent = "cyan" | "amber" | "green" | "violet" | "rose";

const MIN_SAMPLE = 100;

// ---- aperture

/** The f-number each group stands for on its ring (the geometric middle of its range); the hole shrinks with it. */
export const IRIS_F: Record<string, number> = { a1: 1.6, a2: 2.5, a3: 4.8, a4: 9, a5: 18 };
export const APERTURE_ACCENT: Record<string, ExpAccent> = { a1: "violet", a2: "cyan", a3: "green", a4: "amber", a5: "rose" };
export const SHUTTER_ACCENT: Record<string, ExpAccent> = { freeze: "cyan", fast: "cyan", handheld: "green", slow: "amber", motion: "rose", long: "violet" };

/** Radius of the aperture hole: wide open fills the blade ring, f/16 and beyond leaves a pinhole. */
export const irisRadius = (f: number) => 34 * Math.sqrt(1.6 / Math.max(f, 1.6));

/** A regular polygon's points as an SVG attribute (seven blades for a lens iris). */
export function polygonPoints(sides: number, r: number, cx: number, cy: number, rotation = -Math.PI / 2): string {
  return Array.from({ length: sides }, (_, i) => {
    const a = rotation + (i * 2 * Math.PI) / sides;
    return `${(cx + r * Math.cos(a)).toFixed(2)},${(cy + r * Math.sin(a)).toFixed(2)}`;
  }).join(" ");
}

/** "f/1.2-1.8" as "f/1.2–1.8": the contract's labels use a hyphen, the display an en dash. */
export const dashed = (label: string) => label.replace(/(\d)-(\d)/, "$1–$2");

// ---- bars

/** Pixel heights for bars scaled to the largest; a non-empty bar is never thinner than `min`. */
export function barHeights(values: readonly number[], maxPx: number, min = 3): number[] {
  const max = Math.max(0, ...values);
  return values.map((v) => (v > 0 && max > 0 ? Math.max(min, Math.round((v / max) * maxPx * 10) / 10) : 0));
}

// ---- clock

export type Daypart = "night" | "early" | "day" | "late";
/** By hour only, no sun maths: 21-04 night, 05-08 early, 09-15 day, 16-20 late. */
export function daypart(hour: number): Daypart {
  if (hour >= 5 && hour <= 8) return "early";
  if (hour >= 16 && hour <= 20) return "late";
  if (hour >= 21 || hour <= 4) return "night";
  return "day";
}
export const DAYPART_ACCENT: Record<Daypart, ExpAccent> = { night: "violet", early: "amber", day: "cyan", late: "amber" };
/** The legend's hour ranges, end exclusive: "05-09" is 05:00 to 08:59. */
export const EARLY_HOURS: [number, number] = [5, 9];
export const LATE_HOURS: [number, number] = [16, 21];

const rad = (deg: number) => (deg * Math.PI) / 180;
const pt = (cx: number, cy: number, r: number, a: number) => `${(cx + r * Math.cos(a)).toFixed(1)} ${(cy + r * Math.sin(a)).toFixed(1)}`;

/** The wedge's outer radius: inner radius plus the count's share of the busiest hour (an empty hour is a 2 px stub). */
export const wedgeRadius = (n: number, max: number, inner: number, outer: number) => inner + (max > 0 && n > 0 ? Math.max(2 / (outer - inner), n / max) : 0) * (outer - inner);

/** One hour's wedge, 13 degrees wide (2 degree gaps), hour 0 at 12 o'clock, clockwise. */
export function wedgePath(hour: number, inner: number, outer: number, cx: number, cy: number, half = 6.5): string {
  const a0 = rad(hour * 15 - 90 - half), a1 = rad(hour * 15 - 90 + half);
  return `M${pt(cx, cy, inner, a0)} L${pt(cx, cy, outer, a0)} A${outer.toFixed(1)} ${outer.toFixed(1)} 0 0 1 ${pt(cx, cy, outer, a1)} L${pt(cx, cy, inner, a1)} A${inner} ${inner} 0 0 0 ${pt(cx, cy, inner, a0)}Z`;
}
/** Where an hour's label sits at this radius. */
export function hourPoint(hour: number, r: number, cx: number, cy: number): { x: number; y: number } {
  const a = rad(hour * 15 - 90);
  return { x: Math.round((cx + r * Math.cos(a)) * 10) / 10, y: Math.round((cy + r * Math.sin(a)) * 10) / 10 };
}

/** The busiest entry's index (the first on a tie), or -1 when everything is zero. */
export function peakIndex(values: readonly number[]): number {
  let best = -1, max = 0;
  values.forEach((v, i) => { if (v > max) { max = v; best = i; } });
  return best;
}

/** Share of frames in the early and late light hours. */
export function lightShare(hours: readonly { n: number }[]): number {
  const total = hours.reduce((a, h) => a + h.n, 0);
  const light = hours.reduce((a, h, i) => a + (daypart(i) === "early" || daypart(i) === "late" ? h.n : 0), 0);
  return total > 0 ? light / total : 0;
}

// ---- insights

export interface ApertureInsight { most: string; mostShare: number; best: string | null; bestRate: number }
/** The most-used group, and the group with the best keeper rate among those with enough frames, when it is another. */
export function apertureInsight(groups: readonly { id: string; n: number; top: number }[], min = MIN_SAMPLE): ApertureInsight | null {
  const total = groups.reduce((a, g) => a + g.n, 0);
  const mi = peakIndex(groups.map((g) => g.n));
  if (mi < 0 || !total) return null;
  const eligible = groups.filter((g) => g.n >= min);
  const best = [...eligible].sort((a, b) => b.top / b.n - a.top / a.n || b.n - a.n)[0];
  const bestRate = best ? best.top / best.n : 0;
  return { most: groups[mi].id, mostShare: groups[mi].n / total, best: best && bestRate > 0 ? best.id : null, bestRate };
}

export interface ShutterInsight { id: string; share: number; longShare: number }
/** The biggest regime's share, and the share of exposures over a second. */
export function shutterInsight(regimes: readonly { id: string; n: number }[]): ShutterInsight | null {
  const total = regimes.reduce((a, r) => a + r.n, 0);
  const i = peakIndex(regimes.map((r) => r.n));
  if (i < 0 || !total) return null;
  const long = regimes.find((r) => r.id === "long");
  return { id: regimes[i].id, share: regimes[i].n / total, longShare: long ? long.n / total : 0 };
}

/** Share of frames at ISO 3200 and up. */
export function highIsoShare(bins: readonly { id: string; n: number }[]): number {
  const total = bins.reduce((a, b) => a + b.n, 0);
  const high = bins.filter((b) => b.id === "3200" || b.id === "6400+").reduce((a, b) => a + b.n, 0);
  return total > 0 ? high / total : 0;
}

export interface MonthInsight { month: number; n: number; ratio: number; topRate: number }
/** The busiest month (1 to 12), against the average month, and how many of its frames are top images. */
export function monthInsight(months: readonly { n: number; top: number }[]): MonthInsight | null {
  const total = months.reduce((a, m) => a + m.n, 0);
  const i = peakIndex(months.map((m) => m.n));
  if (i < 0 || !total) return null;
  const avg = total / months.length;
  return { month: i + 1, n: months[i].n, ratio: months[i].n / avg, topRate: months[i].top / months[i].n };
}
