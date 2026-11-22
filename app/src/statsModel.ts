// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Statistics view's state, as pure functions: scope, the one filter object, the
// top-images threshold, the chips the filter bar shows, and the request the backend
// is sent. No imports besides types, so verify/stats-model.test.mjs runs it as it is.
// The module-level store that keeps it while the app is open is statsStore.ts.
import type { StatsFilter, StatsRequest, StatsScope, StatsSection } from "./statsTypes";

export type Dim = "makers" | "bodies" | "lenses" | "years" | "months" | "hours" | "weekdays" | "focalBuckets"
  | "apertureGroups" | "shutterRegimes" | "isoBins" | "aspect" | "fileTypes" | "place" | "labels";
export const DIMS: readonly Dim[] = ["makers", "bodies", "lenses", "years", "months", "hours", "weekdays", "focalBuckets",
  "apertureGroups", "shutterRegimes", "isoBins", "aspect", "fileTypes", "place", "labels"];
const NUMERIC: ReadonlySet<Dim> = new Set<Dim>(["years", "months", "hours", "weekdays"]);
// a time or focal drill-down gets the amber chip, as it does in the charts
const AMBER: ReadonlySet<Dim> = new Set<Dim>(["months", "hours", "weekdays", "focalBuckets", "place"]);
// the chip a whole filter dimension shows as one chip (a run of years reads "2014 to 2021")
const GROUPED: ReadonlySet<Dim> = new Set<Dim>(["years"]);

export const DEFAULT_MIN_RATING = 4;
export type MinRating = StatsFilter["minRating"];
export type YearMode = NonNullable<StatsRequest["yearMode"]>;

export interface StatsState {
  scope: StatsScope;
  filter: StatsFilter;
  yearMode: YearMode;
  /** the chip keys in the order they were added, so Esc can remove the last one */
  order: string[];
}

export const LIBRARY_SCOPE: StatsScope = { kind: "library" };

export function clampRating(n: number): MinRating {
  const v = Math.round(Number.isFinite(n) ? n : DEFAULT_MIN_RATING);
  return Math.min(5, Math.max(1, v)) as MinRating;
}

export function initialState(minRating: number = DEFAULT_MIN_RATING): StatsState {
  return { scope: LIBRARY_SCOPE, filter: { minRating: clampRating(minRating), onlyTop: false }, yearMode: "that-year", order: [] };
}

export const valuesOf = (f: StatsFilter, dim: Dim): readonly (string | number)[] =>
  ((f as unknown as Record<string, readonly (string | number)[] | undefined>)[dim]) ?? [];

function withValues(s: StatsState, dim: Dim, values: readonly (string | number)[]): StatsState {
  const filter = { ...s.filter } as Record<string, unknown>;
  if (values.length) filter[dim] = [...values]; else delete filter[dim];
  const keys = values.length ? chipKeys(dim, values) : [];
  const order = s.order.filter((k) => !keyIsDim(k, dim) || keys.includes(k));
  for (const k of keys) if (!order.includes(k)) order.push(k);
  return { ...s, filter: filter as unknown as StatsFilter, order };
}
const keyIsDim = (key: string, dim: Dim) => key === dim || key.startsWith(`${dim}:`);
const chipKeys = (dim: Dim, values: readonly (string | number)[]) => GROUPED.has(dim) ? [dim] : values.map((v) => `${dim}:${v}`);

/** Replaces a dimension's selection. */
export function setValues(s: StatsState, dim: Dim, values: readonly (string | number)[]): StatsState {
  const seen = new Set<string>();
  const list = values.filter((v) => !seen.has(String(v)) && seen.add(String(v)));
  const sorted = NUMERIC.has(dim) ? [...list].sort((a, b) => Number(a) - Number(b)) : list;
  return withValues(s, dim, sorted);
}

/**
 * The click on a mark: it sets its value, clicking it again clears it, clicking another mark of
 * the same chart replaces it, and Shift or Cmd click adds to it (or removes from it).
 */
export function toggleValue(s: StatsState, dim: Dim, value: string | number, additive = false): StatsState {
  const v = NUMERIC.has(dim) ? Number(value) : String(value);
  const have = valuesOf(s.filter, dim);
  const on = have.some((x) => String(x) === String(v));
  if (additive) return setValues(s, dim, on ? have.filter((x) => String(x) !== String(v)) : [...have, v]);
  if (on && have.length === 1) return setValues(s, dim, []);
  return setValues(s, dim, [v]);
}

export const clearDim = (s: StatsState, dim: Dim): StatsState => withValues(s, dim, []);

export function setOnlyTop(s: StatsState, on: boolean): StatsState {
  if (s.filter.onlyTop === on) return s;
  const order = on ? [...s.order, "onlyTop"] : s.order.filter((k) => k !== "onlyTop");
  return { ...s, filter: { ...s.filter, onlyTop: on }, order };
}
export const toggleOnlyTop = (s: StatsState) => setOnlyTop(s, !s.filter.onlyTop);

/** The top-images threshold: it defines "top" everywhere and is not a filter by itself. */
export function setMinRating(s: StatsState, n: number): StatsState {
  const minRating = clampRating(n);
  return minRating === s.filter.minRating ? s : { ...s, filter: { ...s.filter, minRating } };
}

export const setYearMode = (s: StatsState, yearMode: YearMode): StatsState => s.yearMode === yearMode ? s : { ...s, yearMode };

export function setScope(s: StatsState, scope: StatsScope): StatsState {
  return JSON.stringify(s.scope) === JSON.stringify(scope) ? s : { ...s, scope };
}

/** Everything the user selected; the scope, the threshold and the year mode stay. */
export function clearAll(s: StatsState): StatsState {
  return { ...s, filter: { minRating: s.filter.minRating, onlyTop: false }, order: [] };
}

export type Tone = "cyan" | "amber";
export interface Chip {
  key: string;
  dim: Dim | "onlyTop";
  /** one value, or the values of a grouped chip (years); for onlyTop, empty */
  values: (string | number)[];
  tone: Tone;
}

export function chips(s: StatsState): Chip[] {
  const out: Chip[] = [];
  if (s.filter.onlyTop) out.push({ key: "onlyTop", dim: "onlyTop", values: [], tone: "cyan" });
  for (const dim of DIMS) {
    const vals = valuesOf(s.filter, dim);
    if (!vals.length) continue;
    const tone: Tone = AMBER.has(dim) ? "amber" : "cyan";
    if (GROUPED.has(dim)) out.push({ key: dim, dim, values: [...vals], tone });
    else for (const v of vals) out.push({ key: `${dim}:${v}`, dim, values: [v], tone });
  }
  const rank = (k: string) => { const i = s.order.indexOf(k); return i < 0 ? Infinity : i; };
  return out.sort((a, b) => rank(a.key) - rank(b.key));
}

export const activeCount = (s: StatsState) => chips(s).length;

export function removeChip(s: StatsState, key: string): StatsState {
  if (key === "onlyTop") return setOnlyTop(s, false);
  const i = key.indexOf(":");
  if (i < 0) return clearDim(s, key as Dim);
  const dim = key.slice(0, i) as Dim;
  const value = key.slice(i + 1);
  return setValues(s, dim, valuesOf(s.filter, dim).filter((x) => String(x) !== value));
}

/** Esc: the last added chip goes; null when there is none. */
export function clearLast(s: StatsState): StatsState | null {
  const list = chips(s);
  return list.length ? removeChip(s, list[list.length - 1].key) : null;
}

/** The selected values of a dimension as strings, for a chart to mark its selected elements. */
export const selectedSet = (s: StatsState, dim: Dim): ReadonlySet<string> => new Set(valuesOf(s.filter, dim).map(String));

/** Drops the filters that cannot exist in a new scope (`exists` says); reports how many went. */
export function prune(s: StatsState, exists: (dim: Dim, value: string | number) => boolean): { state: StatsState; removed: number } {
  let state = s;
  let removed = 0;
  for (const dim of DIMS) {
    const have = valuesOf(state.filter, dim);
    const keep = have.filter((v) => exists(dim, v));
    if (keep.length !== have.length) { removed += have.length - keep.length; state = setValues(state, dim, keep); }
  }
  return { state, removed };
}

const SECTION_ORDER: StatsSection[] = ["overview", "cameras", "lenses", "exposure", "time", "places", "top"];

/** The backend request: empty filter fields are left out so equal states give equal requests. */
export function buildRequest(s: StatsState, sections: readonly StatsSection[], etag?: string): StatsRequest {
  const f: Record<string, unknown> = { minRating: s.filter.minRating, onlyTop: s.filter.onlyTop };
  for (const dim of DIMS) { const v = valuesOf(s.filter, dim); if (v.length) f[dim] = [...v]; }
  if (s.filter.edited !== undefined) f.edited = s.filter.edited;
  const req: StatsRequest = {
    scope: s.scope,
    filter: f as unknown as StatsFilter,
    sections: SECTION_ORDER.filter((x) => sections.includes(x)),
    yearMode: s.yearMode,
  };
  if (etag) req.etag = etag;
  return req;
}

/** Two states ask the backend the same thing. */
export const sameRequest = (a: StatsRequest, b: StatsRequest) => JSON.stringify({ ...a, etag: 0 }) === JSON.stringify({ ...b, etag: 0 });
