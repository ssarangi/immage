// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Top images section's logic, without React or any import but types (so a node test loads it as it is):
// the paged feed over `stats_top_page` cursors, the strips of best frames per group, the empty state's way out,
// the keyboard map of the grid and the window of cards that are on screen.
import type { ColorLabel, PhotoRef, StatsFilter, StatsRequest, StatsResponse, TopImages } from "./statsTypes";

export const TOP_PAGE = 24;
/** the labels, in the order the app shows them */
export const LABELS: readonly ColorLabel[] = ["red", "yellow", "green", "blue", "purple"];

// ---------------------------------------------------------------- the feed

export type FeedPhase = "idle" | "loading" | "ready" | "error";
export type PagePhase = "idle" | "loading" | "error";

export interface FeedSnapshot {
  /** the request the items answer (or the one on its way when `stale`), null before the first */
  key: string | null;
  phase: FeedPhase;
  /** a new request is on its way; `items` are the last answer's */
  stale: boolean;
  items: PhotoRef[];
  total: number;
  next: string | null;
  paging: PagePhase;
  error: string | null;
  pageError: string | null;
  byLabel: Record<ColorLabel, number> | null;
  byRating: number[] | null;
}

export interface TopSource {
  first(req: StatsRequest): Promise<TopImages>;
  page(cursor: string): Promise<{ items: PhotoRef[]; next: string | null }>;
}

const EMPTY: FeedSnapshot = { key: null, phase: "idle", stale: false, items: [], total: 0, next: null, paging: "idle", error: null, pageError: null, byLabel: null, byRating: null };
const message = (e: unknown) => String(e instanceof Error ? e.message : e);
/** the backend refuses a cursor from before the index was rebuilt: the first page is asked for again */
export const isChanged = (e: unknown) => message(e).includes("stats-changed");

/** The same photos in the same order (a refresh that found nothing new keeps the grid, and what was paged in, as it is). */
export function samePage(a: readonly PhotoRef[], b: readonly PhotoRef[]): boolean {
  return a.length >= b.length && b.every((p, i) => a[i].id === p.id && a[i].rating === p.rating && a[i].label === p.label);
}

/** `page` appended to `items`, without a photo that is there already (the index moved between two pages). */
export function appendPage(items: readonly PhotoRef[], page: readonly PhotoRef[]): PhotoRef[] {
  const seen = new Set(items.map((p) => p.id));
  const out = [...items];
  for (const p of page) if (!seen.has(p.id)) { seen.add(p.id); out.push(p); }
  return out;
}

/**
 * The top images for one request, a page at a time. A new request (a filter, the scope or the threshold changed)
 * starts a new generation: its answer replaces the grid, and any page still on its way from an older one is
 * dropped when it lands. The old photos stay (`stale`) until the new first page arrives, so nothing blinks.
 */
export class TopFeed {
  private snap: FeedSnapshot = EMPTY;
  private gen = 0;
  private req: StatsRequest | null = null;
  private listeners = new Set<() => void>();
  constructor(private source: TopSource) {}

  get = (): FeedSnapshot => this.snap;
  subscribe = (cb: () => void) => { this.listeners.add(cb); return () => { this.listeners.delete(cb); }; };
  private set(next: Partial<FeedSnapshot>) { this.snap = { ...this.snap, ...next }; this.listeners.forEach((l) => l()); }

  /** the request to answer; the same one again changes nothing */
  request(req: StatsRequest | null) {
    const key = req ? JSON.stringify(req) : null;
    if (key === this.snap.key) return;
    this.req = req;
    if (!req) { this.gen++; this.set({ ...EMPTY }); return; }
    this.set({ key, stale: this.snap.items.length > 0 || this.snap.phase === "ready", phase: this.snap.items.length ? this.snap.phase : "loading", paging: "idle", pageError: null, error: null });
    void this.load(false);
  }

  /** the search cache changed: ask the first page again, and keep the grid if nothing in it moved */
  refresh() {
    if (!this.req || this.snap.phase === "loading" || this.snap.stale) return;
    void this.load(true);
  }

  /** Try again after an error: the first page, or the page that failed */
  retry() {
    if (!this.req) return;
    if (this.snap.phase === "error") { this.set({ phase: "loading", error: null }); void this.load(false); }
    else if (this.snap.pageError) this.more();
  }

  more() {
    const { next, paging, phase, stale } = this.snap;
    if (!next || paging === "loading" || phase !== "ready" || stale) return;
    const gen = this.gen;
    this.set({ paging: "loading", pageError: null });
    this.source.page(next).then((page) => {
      if (gen !== this.gen) return; // a newer request has taken over: this page belongs to the old one
      this.set({ items: appendPage(this.snap.items, page.items), next: page.next, paging: "idle" });
    }).catch((e) => {
      if (gen !== this.gen) return;
      if (isChanged(e)) { this.set({ paging: "idle" }); void this.load(false); return; }
      this.set({ paging: "error", pageError: message(e) });
    });
  }

  private async load(quiet: boolean) {
    // a quiet refresh keeps the generation unless it finds something new, so a page on its way is not dropped for nothing
    const gen = quiet ? this.gen : ++this.gen;
    const req = this.req!;
    try {
      const top = await this.source.first(req);
      if (gen !== this.gen) return;
      const facets = { byLabel: top.byLabel, byRating: [...top.byRating] };
      if (quiet && this.snap.phase === "ready" && top.total === this.snap.total && samePage(this.snap.items, top.items)) {
        this.set({ ...facets, total: top.total });
        return;
      }
      this.gen++;
      this.set({ phase: "ready", stale: false, items: top.items, total: top.total, next: top.page.next, paging: "idle", error: null, pageError: null, ...facets });
    } catch (e) {
      if (gen !== this.gen) return;
      if (quiet) return;
      this.set({ phase: "error", stale: false, error: message(e), items: [], total: 0, next: null });
    }
  }
}

// ---------------------------------------------------------------- requests

/** the filter without its colour labels: the facet strip counts every label, so another can be picked */
export function withoutLabels(f: StatsFilter): StatsFilter {
  const { labels: _l, ...rest } = f;
  void _l;
  return rest as StatsFilter;
}

// ---------------------------------------------------------------- the empty state

export interface EmptyAdvice {
  /** the threshold that would show photos, when lowering it is the way out; null at one star */
  lowerTo: number | null;
  /** filters are on: clearing them is a way out too */
  canClear: boolean;
}
/** What the empty state offers: "lower the threshold" sets threshold - 1 (down to one star), and "clear filters" when some are on. */
export function emptyAdvice(minRating: number, filterCount: number): EmptyAdvice {
  return { lowerTo: minRating > 1 ? minRating - 1 : null, canClear: filterCount > 0 };
}

// ---------------------------------------------------------------- strips: the best frame of each lens, camera, place

export type StripKind = "lenses" | "bodies" | "places";
export interface StripGroup { key: string; name: string; n: number; top: number; photo: PhotoRef }

/** Groups with a best frame, the most top images first (then the most photos, then the name), at most `limit`. */
export function stripGroups(kind: "lenses" | "bodies", r: Pick<StatsResponse, "lenses" | "cameras"> | null, name: (key: string) => string, limit = 12): StripGroup[] {
  const rows = kind === "lenses"
    ? (r?.lenses?.lenses ?? []).map((l) => ({ key: l.name, n: l.n, top: l.top, photo: l.cover }))
    : (r?.cameras?.bodies ?? []).map((b) => ({ key: b.key, n: b.n, top: b.top, photo: b.cover }));
  return rows.filter((x): x is typeof x & { photo: PhotoRef } => !!x.photo && x.top > 0)
    .sort((a, b) => b.top - a.top || b.n - a.n || a.key.localeCompare(b.key))
    .slice(0, limit).map((x) => ({ key: x.key, name: name(x.key), n: x.n, top: x.top, photo: x.photo }));
}

// ---------------------------------------------------------------- keyboard

/** how many cards sit in the first row: the grid's column count, read from the cards' tops */
export function columnsOf(tops: readonly number[]): number {
  if (!tops.length) return 1;
  let n = 1;
  while (n < tops.length && Math.abs(tops[n] - tops[0]) < 2) n++;
  return n;
}

export type Move = { index: number; more: boolean };
/**
 * Where an arrow key goes from card `i` of `count` in a grid `cols` wide. Down from the last row, and End, stay on
 * the last card and ask for the next page (`more`) when the grid has one.
 */
export function moveFocus(key: string, i: number, cols: number, count: number, hasMore: boolean): Move | null {
  if (!count) return null;
  const last = count - 1;
  let to: number;
  switch (key) {
    case "ArrowRight": to = i + 1; break;
    case "ArrowLeft": to = i - 1; break;
    case "ArrowDown": to = i + cols; break;
    case "ArrowUp": to = i - cols; break;
    case "Home": to = 0; break;
    case "End": to = last; break;
    case "PageDown": to = i + cols * 3; break;
    case "PageUp": to = i - cols * 3; break;
    default: return null;
  }
  const beyond = to > last;
  return { index: Math.max(0, Math.min(last, to)), more: hasMore && (beyond || to >= last - cols) };
}

// ---------------------------------------------------------------- the cards on screen

/**
 * The first and last card whose rows are within `margin` px of the view, by binary search over the cards' tops
 * (they only grow down the grid). Cards outside it keep their box but not their image.
 */
export function visibleRange(tops: (i: number) => number, bottoms: (i: number) => number, count: number, viewTop: number, viewBottom: number, margin: number): [number, number] {
  if (!count) return [0, -1];
  const lo = viewTop - margin, hi = viewBottom + margin;
  let a = 0, b = count; // first card whose bottom is below lo
  while (a < b) { const m = (a + b) >> 1; if (bottoms(m) < lo) a = m + 1; else b = m; }
  const from = a;
  a = from; b = count; // first card whose top is below hi
  while (a < b) { const m = (a + b) >> 1; if (tops(m) <= hi) a = m + 1; else b = m; }
  return [from, a - 1];
}
