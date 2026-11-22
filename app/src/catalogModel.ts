// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Catalog's page-side model, with no React and no Tauri: what a source is
// to the user (one state, one word), how far its first index has got, how long
// that may take, and which source a folder belongs to. catalogStore.ts feeds it
// the backend's replies and events; verify/catalog-ui.test.mjs tests it.
import type { MetaProgress, Progress, SourceStatus, ThumbProgress } from "./bindings";

// What the backend sends when a run ends (typed in catalog_cmds / typed_catalog.rs; events
// carry no generated type, so these mirror ScanSummary, MetaSummary and ThumbSummary).
export type RunState = "complete" | "incomplete" | "offline" | "cancelled" | "interrupted" | "failed" | "disk_full";
export interface ScanDone {
  source_id: number; state: RunState; elapsed_s: number; found: number; folders: number; new: number; changed: number;
  unchanged: number; missing: number; unreadable: number; incomplete_dirs: string[]; error: string | null;
}
export interface MetaDone { source_id: number; state: RunState; elapsed_s: number; total: number; done: number; failed: number; unparsed: number; error: string | null }
export interface ThumbDone { source_id: number; state: RunState; elapsed_s: number; total: number; done: number; failed: number; deferred: number; thumbnail_bytes: number; error: string | null }

// one glyph per state, in this order of importance when several apply
export type SourceState = "indexing" | "paused" | "attention" | "offline" | "asleep" | "never" | "online";

export interface Jobs {
  scan?: Progress;
  meta?: MetaProgress;
  thumbs?: ThumbProgress;
  /** this page started the first index and it has not ended yet (between two steps no job is running) */
  pipeline?: boolean;
}

export interface Ended {
  scan?: ScanDone;
  meta?: MetaDone;
  thumbs?: ThumbDone;
  /** epoch ms the pipeline ended */
  at?: number;
}

export function isOnline(s: SourceStatus): boolean {
  return s.availability.state === "Online";
}

/** The folder a source covers, as the user knows it: where it is mounted now, or where it was. */
export function sourceRoot(s: SourceStatus): string {
  const a = s.availability;
  if (a.state === "Online") return a.detail.abs_root;
  const mount = a.state === "Asleep" ? a.detail.mount_point : s.source.mount_path;
  return joinPath(mount ?? "", s.source.rel_root);
}

function joinPath(base: string, rel: string): string {
  const r = rel.replace(/^[\\/]+/, "");
  if (!r) return base;
  return base.endsWith("/") || base.endsWith("\\") ? base + r : `${base}/${r}`;
}

const trimEnd = (p: string) => (p.length > 1 ? p.replace(/[\\/]+$/, "") : p);
/** `path` is `root` or inside it */
export function pathWithin(path: string, root: string): boolean {
  const p = trimEnd(path), r = trimEnd(root);
  return p === r || p.startsWith(r + "/") || p.startsWith(r + "\\");
}

export type FolderCatalog =
  | { kind: "none" }
  /** the folder is a source's root */
  | { kind: "root"; source: SourceStatus }
  /** the folder is inside a source (so adding it is refused) */
  | { kind: "inside"; source: SourceStatus };

/** Whether a folder of the Folders list is in the Catalog. */
export function folderCatalog(path: string, sources: readonly SourceStatus[]): FolderCatalog {
  let inside: SourceStatus | null = null;
  for (const s of sources) {
    const root = sourceRoot(s);
    if (!root) continue;
    if (trimEnd(root) === trimEnd(path)) return { kind: "root", source: s };
    if (pathWithin(path, root)) inside = s;
  }
  return inside ? { kind: "inside", source: inside } : { kind: "none" };
}

/** The one state a source shows. */
export function sourceState(s: SourceStatus, jobs: Jobs | undefined): SourceState {
  const running = jobs?.scan ?? jobs?.meta ?? jobs?.thumbs;
  if (running || jobs?.pipeline) {
    const paused = (jobs?.scan?.paused ?? false) || (jobs?.meta?.paused ?? false) || (jobs?.thumbs?.paused ?? false);
    return paused ? "paused" : "indexing";
  }
  const a = s.availability.state;
  if (a === "Offline") return "offline";
  if (a === "Asleep") return "asleep";
  const src = s.source;
  if (src.error || src.state === "failed" || src.state === "incomplete") return "attention";
  if (src.state === "interrupted" || src.state === "cancelled") return "paused";
  if (src.last_scan_end === null && src.photos === 0) return "never";
  return "online";
}

export type Phase = "finding" | "reading" | "thumbnails";
export interface PhaseView {
  id: Phase;
  status: "waiting" | "active" | "done";
  /** photos done so far, and in all when known (null while it is still being found) */
  done: number;
  total: number | null;
  rate: number | null;
}

/** The three rows of the first-index card. */
export function phases(jobs: Jobs | undefined, ended: Ended | undefined, photos: number): PhaseView[] {
  const scan = jobs?.scan, meta = jobs?.meta, thumbs = jobs?.thumbs;
  const found = scan ? scan.found : ended?.scan?.found ?? photos;
  const scanDone = !scan && (ended?.scan !== undefined || !!meta || !!thumbs || photos > 0);
  const finding: PhaseView = {
    id: "finding",
    status: scan ? "active" : scanDone ? "done" : "waiting",
    done: found, total: scan ? null : found, rate: scan?.rate_per_s ?? null,
  };
  const metaDone = ended?.meta?.state === "complete" && !meta;
  const reading: PhaseView = {
    id: "reading",
    status: meta ? "active" : metaDone ? "done" : "waiting",
    done: meta ? meta.done : metaDone ? ended!.meta!.done : 0,
    total: meta ? meta.total : metaDone ? ended!.meta!.total : found || null,
    rate: meta?.rate_per_s ?? null,
  };
  const thumbDone = ended?.thumbs?.state === "complete" && !thumbs;
  const making: PhaseView = {
    id: "thumbnails",
    status: thumbs ? "active" : thumbDone ? "done" : "waiting",
    done: thumbs ? thumbs.done + thumbs.skipped : thumbDone ? ended!.thumbs!.done : 0,
    total: thumbs ? thumbs.total : thumbDone ? ended!.thumbs!.total : null,
    rate: thumbs?.rate_per_s ?? null,
  };
  return [finding, reading, making];
}

/** 0..1 across the whole run (finding counts for nothing until its total is known), or null while it is still finding. */
export function overallFraction(p: readonly PhaseView[]): number | null {
  const reading = p[1];
  if (p[0].status === "active") return null;
  if (reading.total === null || reading.total === 0) return reading.status === "done" ? 1 : null;
  return Math.max(0, Math.min(1, reading.done / reading.total));
}

/** The time left as a range (low, high) in seconds from the backend, the rate unstable (null), or under a minute. */
export type Eta = { kind: "none" } | { kind: "soon" } | { kind: "range"; lo: number; hi: number };
export function etaOf(range: readonly [number, number] | null | undefined): Eta {
  if (!range) return { kind: "none" };
  const [lo, hi] = range;
  if (!(hi > 0) || hi < 60) return { kind: "soon" };
  return { kind: "range", lo, hi };
}

/** "8 min", "1 h 5 min", "45 s": a duration the way the card says it. */
export function durationText(seconds: number): { unit: "s" | "min" | "h"; a: number; b?: number } {
  const s = Math.max(0, Math.round(seconds));
  if (s < 60) return { unit: "s", a: s };
  const m = Math.round(s / 60);
  if (m < 60) return { unit: "min", a: m };
  const h = Math.floor(m / 60);
  return { unit: "h", a: h, b: m % 60 };
}

/** Plain words for why a run ended badly; never a code. */
export type Problem = "offline" | "failed" | "partial" | "disk_full" | null;
export function problemOf(ended: Ended | undefined): Problem {
  if (!ended) return null;
  const states = [ended.scan?.state, ended.meta?.state, ended.thumbs?.state];
  if (states.includes("offline")) return "offline";
  if (states.includes("disk_full")) return "disk_full";
  if (states.includes("failed")) return "failed";
  if (ended.scan?.state === "incomplete") return "partial";
  return null;
}

/** Whether the first index of this source has ended well, with everything made. */
export function isReady(ended: Ended | undefined): boolean {
  return !!ended?.scan && ended.scan.state === "complete" && ended.meta?.state === "complete" && ended.thumbs?.state === "complete";
}

/** Group the sources by where they live, as the Folders list groups by drive. */
export function groupByVolume(list: readonly SourceStatus[]): { key: string; name: string; host: string | null; online: boolean; items: SourceStatus[] }[] {
  const out = new Map<string, { key: string; name: string; host: string | null; online: boolean; items: SourceStatus[] }>();
  for (const s of list) {
    const key = s.source.volume_key;
    const g = out.get(key) ?? {
      key, name: s.source.share || s.source.host || baseOf(s.source.mount_path ?? "") || s.source.name, host: s.source.host, online: isOnline(s), items: [],
    };
    g.items.push(s);
    out.set(key, g);
  }
  return [...out.values()];
}

function baseOf(p: string): string {
  return trimEnd(p).split(/[\\/]/).pop() ?? "";
}

/** A size for the disk-used line: one decimal above a megabyte, none below. */
export function sizeOf(bytes: number): { n: number; unit: "B" | "KB" | "MB" | "GB" } {
  if (bytes < 1024) return { n: bytes, unit: "B" };
  if (bytes < 1024 ** 2) return { n: Math.round(bytes / 1024), unit: "KB" };
  if (bytes < 1024 ** 3) return { n: Math.round(bytes / 1024 ** 2), unit: "MB" };
  return { n: Math.round(bytes / 1024 ** 3 * 10) / 10, unit: "GB" };
}

/** What the Catalog card's header says while the card is folded: the work across all sources in one line. */
export interface Summary {
  /** the busiest state of any source: running beats paused; "idle" has neither */
  state: "indexing" | "paused" | "idle";
  /** 0..1 over the sources that are reading or making thumbnails; null while only finding, or when idle */
  fraction: number | null;
  done: number;
  total: number | null;
  /** the work being counted is thumbnails (and no source is reading photos) */
  thumbs: boolean;
  /** sources that need attention, shown beside the work, not instead of it */
  attention: number;
}

export function catalogSummary(sources: readonly SourceStatus[], jobs: Record<number, Jobs>): Summary {
  let running = 0, paused = 0, attention = 0, done = 0, total = 0, counted = 0, thumbSources = 0;
  for (const s of sources) {
    const j = jobs[s.source.id];
    const state = sourceState(s, j);
    if (state === "attention") attention++;
    if (state !== "indexing" && state !== "paused") continue;
    if (state === "paused") paused++; else running++;
    const p = phases(j, undefined, s.source.photos);
    if (p[0].status === "active") continue;
    const use = p[2].status === "active" && p[1].status !== "active" ? p[2] : p[1];
    if (use.total === null || use.total === 0) continue;
    counted++;
    if (use === p[2]) thumbSources++;
    done += use.done; total += use.total;
  }
  const busy = running + paused;
  if (busy === 0) return { state: "idle", fraction: null, done: 0, total: null, thumbs: false, attention };
  return {
    state: running > 0 ? "indexing" : "paused",
    fraction: counted > 0 ? Math.max(0, Math.min(1, done / total)) : null,
    done, total: counted > 0 ? total : null,
    thumbs: counted > 0 && thumbSources === counted,
    attention,
  };
}
