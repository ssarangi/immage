// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The page's one picture of the Catalog: the sources and where they are, the jobs running on them
// and how the last ones ended. It listens to the backend's events (catalog_cmds.rs), asks for the
// sources again every few seconds while something shows them, and chains the three jobs of an
// index. catalogEvents.ts and catalogModel.ts are the logic; this file is the wiring.
import { useSyncExternalStore } from "react";
import { commands, type CatalogStatus, type MetaProgress, type Progress, type SourceStatus, type ThumbProgress } from "./commands";
import { apply, EMPTY, tidy, type CatalogEvent, type JobsState, type Next } from "./catalogEvents";
import type { MetaDone, ScanDone, ThumbDone } from "./catalogModel";

export interface CatalogState extends JobsState {
  loaded: boolean;
  sources: SourceStatus[];
  status: CatalogStatus | null;
  error: string | null;
}

let state: CatalogState = { ...EMPTY, loaded: false, sources: [], status: null, error: null };
const listeners = new Set<() => void>();
const set = (next: Partial<CatalogState>) => { state = { ...state, ...next }; listeners.forEach((l) => l()); };
export const getCatalog = () => state;

const message = (e: unknown) => String(e instanceof Error ? e.message : e);

export async function refresh(): Promise<void> {
  try {
    const [sources, status] = await Promise.all([commands.sourcesList(), commands.catalogStatus()]);
    set({ loaded: true, sources, status, error: null });
  } catch (e) {
    set({ loaded: true, error: message(e) });
  }
}

function feed(ev: CatalogEvent) {
  const { state: s, next } = apply(state, ev);
  set(tidy(s));
  step(next);
}

function step(next: Next) {
  if (!next) return;
  if ("end" in next) { void refresh(); return; }
  const call = next.start === "metadata" ? commands.metadataStart([next.id]) : commands.thumbnailsStart([next.id]);
  call.catch((e) => { set({ error: message(e) }); feed({ kind: next.start === "metadata" ? "metadata-finished" : "thumbnails-finished", s: failed(next.id, message(e)) }); });
}

const failed = (id: number, error: string) => ({
  source_id: id, state: "failed" as const, elapsed_s: 0, total: 0, done: 0, failed: 0, unparsed: 0, deferred: 0, thumbnail_bytes: 0, error,
});

let started = false;
function start() {
  if (started) return;
  started = true;
  void import("@tauri-apps/api/event").then(({ listen }) => {
    const on = <T,>(name: string, make: (p: T) => CatalogEvent) => listen<T>(name, (e) => feed(make(e.payload)));
    void on<Progress>("catalog://scan-progress", (p) => ({ kind: "scan-progress", p }));
    void on<ScanDone>("catalog://scan-finished", (s) => ({ kind: "scan-finished", s }));
    void on<MetaProgress>("catalog://metadata-progress", (p) => ({ kind: "metadata-progress", p }));
    void on<MetaDone>("catalog://metadata-finished", (s) => ({ kind: "metadata-finished", s }));
    void on<ThumbProgress>("catalog://thumbnails-progress", (p) => ({ kind: "thumbnails-progress", p }));
    void on<ThumbDone>("catalog://thumbnails-finished", (s) => ({ kind: "thumbnails-finished", s }));
  }).catch(() => { /* no Tauri (a harness): the page is fed by hand */ });
  // jobs that were running before this page opened (a reload)
  void Promise.all([commands.jobsActive(), commands.metadataJobsActive(), commands.thumbnailsJobsActive()]).then(([a, b, c]) => {
    for (const p of a) feed({ kind: "scan-progress", p });
    for (const p of b) feed({ kind: "metadata-progress", p });
    for (const p of c) feed({ kind: "thumbnails-progress", p });
  }).catch(() => {});
  void refresh();
}

let timer: ReturnType<typeof setInterval> | undefined;
function subscribe(l: () => void) {
  listeners.add(l);
  start();
  timer ??= setInterval(() => { if (document.visibilityState !== "hidden") void refresh(); }, 8000);
  return () => {
    listeners.delete(l);
    if (!listeners.size) { clearInterval(timer); timer = undefined; }
  };
}

export const useCatalog = (): CatalogState => useSyncExternalStore(subscribe, getCatalog, getCatalog);

/** Starts the three jobs of an index of one source (find, read, thumbnails), one after another. */
export async function indexSource(id: number, light = false): Promise<void> {
  feed({ kind: "begin", id });
  try { await commands.scanStart([id], light); }
  catch (e) { set({ error: message(e) }); feed({ kind: "scan-finished", s: { source_id: id, state: "failed", elapsed_s: 0, found: 0, folders: 0, new: 0, changed: 0, unchanged: 0, missing: 0, unreadable: 0, incomplete_dirs: [], error: message(e) } }); }
}

/** A folder becomes a source and its first index starts. The reply is the source's id. */
export async function addToCatalog(path: string, name: string | null = null): Promise<number> {
  const src = await commands.sourceAdd({ path, name, policy: "manual", include_subfolders: true, index_now: false, allow_offline: false });
  await refresh();
  void indexSource(src.id);
  return src.id;
}

export async function removeFromCatalog(id: number): Promise<void> {
  await commands.sourceRemove(id, false);
  const { [id]: _j, ...jobs } = state.jobs;
  const { [id]: _e, ...ended } = state.ended;
  set({ jobs, ended });
  await refresh();
}

/** Pause, resume and stop act on the job that is running now (the backend names jobs, not sources). */
const jobOf = (id: number) => { const j = state.jobs[id]; return j?.scan?.job_id ?? j?.meta?.job_id ?? j?.thumbs?.job_id ?? null; };
export async function pauseSource(id: number) { const j = jobOf(id); if (j !== null) await commands.jobPause(j); }
export async function resumeSource(id: number) { const j = jobOf(id); if (j !== null) await commands.jobResume(j); else await indexSource(id); }
export async function stopSource(id: number) { const j = jobOf(id); if (j !== null) await commands.jobCancel(j); }

/** Rebuild: forget that any photo was read, then index every source that answers again. */
export async function rebuildCatalog(): Promise<number> {
  const n = await commands.catalogRebuild();
  for (const s of state.sources) if (s.availability.state === "Online") void indexSource(s.source.id);
  return n;
}

export function dismissEnded(id: number) {
  const { [id]: _e, ...ended } = state.ended;
  set({ ended });
}
