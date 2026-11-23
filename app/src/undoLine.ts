// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The pure parts of the Keywording card's inline Undo line ("Removed sea - Undo", shown for 5 s in the
// Auto-keywords group's state row after taking Auto-keywords off a photo): what a removal took off, how
// removals merge into one line, and the timer that dismisses it and waits while the pointer or the
// keyboard focus is on the line.
import type { RestoreItem } from "./bindings";

export const UNDO_MS = 5000;
// after a hold ends, at least this much of the line is left, so moving the pointer away is not the
// same as dismissing it
export const UNDO_FLOOR_MS = 2000;

export type UndoLine = { key: string; items: RestoreItem[] };

// What taking `names` (all of each photo's when empty) off these photos removes: per photo, the
// keywords on it that Auto-keywords added. `ai` is each photo's Auto-keywords' names.
export function removalItems(rows: readonly { path: string; keywords: readonly string[] }[],
                             ai: ReadonlyMap<string, ReadonlySet<string>>, names: readonly string[]): RestoreItem[] {
  const out: RestoreItem[] = [];
  for (const r of rows) {
    const mine = ai.get(r.path);
    if (!mine) continue;
    const gone = [...(names.length ? names : mine)].filter((k) => mine.has(k) && r.keywords.includes(k));
    if (gone.length) out.push({ path: r.path, names: [...new Set(gone)] });
  }
  return out;
}

// a removal while the line is still up joins it (for the same photos); another photo starts a new one
export function mergeUndo(prev: UndoLine | null, key: string, items: RestoreItem[]): UndoLine | null {
  if (!items.length) return prev && prev.key === key ? prev : null;
  if (!prev || prev.key !== key) return { key, items };
  const by = new Map(prev.items.map((i) => [i.path, new Set(i.names)]));
  for (const i of items) { const s = by.get(i.path) ?? new Set<string>(); i.names.forEach((n) => s.add(n)); by.set(i.path, s); }
  return { key, items: [...by].map(([path, s]) => ({ path, names: [...s] })) };
}

// the keywords the line is about: its one name, or how many
export function undoSummary(u: UndoLine): { n: number; first: string } {
  const all = [...new Set(u.items.flatMap((i) => i.names))];
  return { n: all.length, first: all[0] ?? "" };
}

export type Clock = { set: (fn: () => void, ms: number) => unknown; clear: (h: unknown) => void; now: () => number };
const real: Clock = { set: (fn, ms) => setTimeout(fn, ms), clear: (h) => clearTimeout(h as ReturnType<typeof setTimeout>), now: () => Date.now() };

// Counts down `ms` and calls `expire` once. Hover and focus each hold it (`hold(reason)`); it runs again
// when the last hold is released. `start` begins a new countdown (also while held: it then waits).
export function createUndoTimer(expire: () => void, ms = UNDO_MS, clock: Clock = real) {
  const holds = new Set<string>();
  let left = ms, handle: unknown = null, armedAt = 0, live = false;
  const stop = () => { if (handle !== null) { clock.clear(handle); handle = null; left = Math.max(0, left - (clock.now() - armedAt)); } };
  const arm = () => {
    if (!live || handle !== null || holds.size) return;
    armedAt = clock.now();
    handle = clock.set(() => { handle = null; live = false; holds.clear(); expire(); }, left);
  };
  return {
    start() { stop(); live = true; left = ms; arm(); },
    hold(reason: string) { if (!live) return; stop(); holds.add(reason); },
    release(reason: string) { if (!holds.delete(reason) || holds.size) return; left = Math.max(left, Math.min(ms, UNDO_FLOOR_MS)); arm(); },
    cancel() { stop(); live = false; holds.clear(); },
    get running() { return handle !== null; },
    get held() { return holds.size > 0; },
  };
}
