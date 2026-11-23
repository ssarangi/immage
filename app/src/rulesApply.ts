// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Applying Scenes rules is a plan then a cull_apply that can take a while and, if the engine stalls, never
// answer. `watched` runs such a step and fails it when nothing has been heard for `stallMs`, so the panel
// can always leave its busy state and say what happened. `ping` is called on every sign of life.
export type Watch = { ping: () => void };

export function watched<T>(stallMs: number, run: (watch: Watch) => Promise<T>, message: string): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    let over = false;
    const arm = () => {
      if (over) return;
      if (timer !== undefined) clearTimeout(timer);
      timer = setTimeout(() => { over = true; reject(new Error(message)); }, stallMs);
    };
    const end = () => { over = true; if (timer !== undefined) clearTimeout(timer); };
    arm();
    run({ ping: arm }).then((v) => { if (!over) { end(); resolve(v); } else end(); }, (e) => { if (!over) { end(); reject(e); } else end(); });
  });
}

export type ApplyProgress = { done: number; total: number };

/// the cull-progress event's payload as the apply's progress, for this session's apply stage only
export function applyProgress(e: { job?: number; stage?: string; done?: number; total?: number }, session: number): ApplyProgress | null {
  if (e.stage !== "apply" || e.job !== session || typeof e.done !== "number" || typeof e.total !== "number") return null;
  return { done: e.done, total: e.total };
}
