// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Closed-eye verdicts for the photos on screen (notes/faces-p4-backend-notes.md): `faces_eyes_for` computes what is missing
// and is bounded (200 paths a call, about 20 s), so the page asks for the photos it is showing, a few at a time. The wanted
// set is what is on screen right now: asking again replaces it, so photos scrolled past are not asked for (one already
// on its way finishes and is kept). The call waits a moment after the last change (scrolling), runs a few batches at a
// time, remembers every answer, and never asks twice for a photo until `forget()` (the faces changed). A failed batch does
// not wedge the queue.
export type EyeLabel = "open" | "semi" | "closed";
export type EyeResult = { path: string; state: EyeLabel | null; primaryFace?: number | null };
export type Timers = { set: (fn: () => void, ms: number) => unknown; clear: (h: unknown) => void };
const real: Timers = { set: (fn, ms) => setTimeout(fn, ms), clear: (h) => clearTimeout(h as ReturnType<typeof setTimeout>) };

export class EyeQueue {
  private wanted: string[] = [];
  private asked = new Set<string>();      // answered, failed or on its way: never asked again until forget()
  private running = 0;
  private timer: unknown = null;
  private generation = 0;
  readonly batch: number; readonly concurrency: number; readonly debounceMs: number;
  private fetch: (paths: string[]) => Promise<EyeResult[]>;
  private onResult: (r: EyeResult[]) => void;
  private timers: Timers;
  constructor(fetch: (paths: string[]) => Promise<EyeResult[]>, onResult: (r: EyeResult[]) => void,
              o: { batch?: number; concurrency?: number; debounceMs?: number; timers?: Timers } = {}) {
    this.fetch = fetch; this.onResult = onResult;
    this.batch = Math.min(200, o.batch ?? 24); this.concurrency = o.concurrency ?? 2; this.debounceMs = o.debounceMs ?? 250;
    this.timers = o.timers ?? real;
  }

  /** the photos on screen now, nearest first: replaces what was wanted */
  want(paths: readonly string[]) {
    this.wanted = [...new Set(paths)].filter((p) => !this.asked.has(p));
    if (this.timer !== null) this.timers.clear(this.timer);
    this.timer = this.wanted.length ? this.timers.set(() => { this.timer = null; this.pump(); }, this.debounceMs) : null;
  }

  /** the faces changed: every verdict may be different */
  forget() { this.asked.clear(); this.generation++; }

  get pending() { return this.wanted.length; }

  private pump() {
    while (this.running < this.concurrency) {
      const take = this.wanted.filter((p) => !this.asked.has(p)).slice(0, this.batch);
      if (!take.length) return;
      take.forEach((p) => this.asked.add(p));
      this.wanted = this.wanted.filter((p) => !take.includes(p));
      this.running++;
      const gen = this.generation;
      this.fetch(take).then((r) => { if (gen === this.generation) this.onResult(r); }, () => { /* asked again after forget() */ })
        .finally(() => { this.running--; this.pump(); });
    }
  }
}

/**
 * Asks for the verdicts of a whole list in chunks, one call at a time (Find's Closed eyes over the open folder):
 * `onChunk` hears each answer, `progress` how far it is, and `stopped()` ends it between chunks. Photos in `have` are skipped.
 */
export async function scanPaths(paths: readonly string[], fetch: (paths: string[]) => Promise<EyeResult[]>, o: {
  size?: number; have?: (path: string) => boolean; onChunk: (r: EyeResult[]) => void; progress?: (done: number, total: number) => void; stopped?: () => boolean;
}): Promise<{ done: number; total: number }> {
  const size = Math.min(200, o.size ?? 24);
  const todo = paths.filter((p) => !o.have?.(p));
  let done = 0;
  o.progress?.(0, todo.length);
  for (let i = 0; i < todo.length && !o.stopped?.(); i += size) {
    const chunk = todo.slice(i, i + size);
    try { o.onChunk(await fetch(chunk)); } catch { /* the chunk is skipped: its photos stay unknown */ }
    done += chunk.length;
    o.progress?.(done, todo.length);
  }
  return { done, total: todo.length };
}
