// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// What the Catalog's progress events do to the page's picture of the jobs, with no React and no
// Tauri (catalogStore.ts wires it to the events). One index of a source is three jobs one after
// another (find, read, thumbnails); the page chains them, so `next` says which job to start when a
// finish event ends the one before it.
import type { MetaProgress, Progress, ThumbProgress } from "./bindings";
import type { Ended, Jobs, MetaDone, ScanDone, ThumbDone } from "./catalogModel";

export interface JobsState {
  jobs: Record<number, Jobs>;
  ended: Record<number, Ended>;
}

export const EMPTY: JobsState = { jobs: {}, ended: {} };

export type CatalogEvent =
  | { kind: "scan-progress"; p: Progress }
  | { kind: "scan-finished"; s: ScanDone }
  | { kind: "metadata-progress"; p: MetaProgress }
  | { kind: "metadata-finished"; s: MetaDone }
  | { kind: "thumbnails-progress"; p: ThumbProgress }
  | { kind: "thumbnails-finished"; s: ThumbDone }
  /** the page asked for an index of a source: the three jobs follow one another */
  | { kind: "begin"; id: number }
  /** a running job seen when the page (re)opened */
  | { kind: "adopt"; id: number };

export type Next = null | { start: "metadata" | "thumbnails"; id: number } | { end: number };

const patch = (st: JobsState, id: number, f: (j: Jobs) => Jobs, e?: (x: Ended) => Ended): JobsState => ({
  jobs: { ...st.jobs, [id]: f(st.jobs[id] ?? {}) },
  ended: e ? { ...st.ended, [id]: e(st.ended[id] ?? {}) } : st.ended,
});

const goodEnough = (s: string) => s === "complete" || s === "incomplete";

export function apply(st: JobsState, ev: CatalogEvent): { state: JobsState; next: Next } {
  switch (ev.kind) {
    case "begin": {
      const { [ev.id]: _gone, ...ended } = st.ended;
      return { state: { jobs: { ...st.jobs, [ev.id]: { pipeline: true } }, ended }, next: null };
    }
    case "adopt":
      return { state: patch(st, ev.id, (j) => ({ ...j, pipeline: true })), next: null };
    case "scan-progress":
      return { state: patch(st, ev.p.source_id, (j) => ({ ...j, scan: ev.p, pipeline: j.pipeline ?? true })), next: null };
    case "metadata-progress":
      return { state: patch(st, ev.p.source_id, (j) => ({ ...j, meta: ev.p, pipeline: j.pipeline ?? true })), next: null };
    case "thumbnails-progress":
      return { state: patch(st, ev.p.source_id, (j) => ({ ...j, thumbs: ev.p, pipeline: j.pipeline ?? true })), next: null };
    case "scan-finished": {
      const id = ev.s.source_id, go = goodEnough(ev.s.state) && !!st.jobs[id]?.pipeline;
      const state = patch(st, id, ({ scan: _s, ...j }) => ({ ...j, pipeline: go }), (x) => ({ ...x, scan: ev.s, at: go ? undefined : Date.now() }));
      return { state, next: go ? { start: "metadata", id } : { end: id } };
    }
    case "metadata-finished": {
      const id = ev.s.source_id, go = ev.s.state === "complete" && !!st.jobs[id]?.pipeline;
      const state = patch(st, id, ({ meta: _m, ...j }) => ({ ...j, pipeline: go }), (x) => ({ ...x, meta: ev.s, at: go ? undefined : Date.now() }));
      return { state, next: go ? { start: "thumbnails", id } : { end: id } };
    }
    case "thumbnails-finished": {
      const id = ev.s.source_id;
      const state = patch(st, id, ({ thumbs: _t, ...j }) => ({ ...j, pipeline: false }), (x) => ({ ...x, thumbs: ev.s, at: Date.now() }));
      return { state, next: { end: id } };
    }
  }
}

/** A job that a finish event left no `pipeline` for is over; drop the empty entry. */
export function tidy(st: JobsState): JobsState {
  const jobs: Record<number, Jobs> = {};
  for (const [k, j] of Object.entries(st.jobs)) if (j.scan || j.meta || j.thumbs || j.pipeline) jobs[Number(k)] = j;
  return { jobs, ended: st.ended };
}
