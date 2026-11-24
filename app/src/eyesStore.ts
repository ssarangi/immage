// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The closed-eye verdict of each photo the page is showing, for the glyph on its thumbnail and scene card. A cell watches its
// photo while it is mounted (the grids mount only what is on screen), the queue (eyesQueue.ts) asks the backend for the
// watched photos a few at a time after scrolling pauses, and the answers are kept per photo. Nothing is asked while Faces is
// off or there is no eye model; the verdict is the "preview" of notes/faces-p4-backend-notes.md until the model is validated.
import { useEffect, useSyncExternalStore } from "react";
import { EyeQueue, type EyeLabel } from "./eyesQueue";
import { peopleApi, peopleState, subscribePeople } from "./peopleStore";

export type { EyeLabel };

const known = new Map<string, EyeLabel | null>();
const watchers = new Map<string, number>();
const listeners = new Map<string, Set<() => void>>();
let lastTick = -1;

const queue = new EyeQueue(
  (paths) => peopleApi().eyesFor(paths, null).then((r) => r.map((x) => ({ path: x.path, state: x.state }))),
  (results) => {
    for (const r of results) {
      if (known.get(r.path) === r.state) continue;
      known.set(r.path, r.state);
      listeners.get(r.path)?.forEach((l) => l());
    }
  },
);

/** whether eye verdicts can exist: Faces is on and the optional eye model file is there */
export const eyesActive = () => { const s = peopleState().status; return !!s && s.enabled && s.installed && s.eyes_available; };

const want = () => {
  const s = peopleState();
  // the faces changed (naming, a scan): the verdicts are asked for again
  if (s.tick !== lastTick) { lastTick = s.tick; queue.forget(); }
  queue.want(eyesActive() ? [...watchers.keys()] : []);
};

function subscribe(path: string, l: () => void) {
  if (!listeners.has(path)) listeners.set(path, new Set());
  listeners.get(path)!.add(l);
  watchers.set(path, (watchers.get(path) ?? 0) + 1);
  want();
  return () => {
    listeners.get(path)?.delete(l);
    const n = (watchers.get(path) ?? 1) - 1;
    if (n <= 0) watchers.delete(path); else watchers.set(path, n);
    want();
  };
}

/** the primary face's verdict for a photo, or null while it is unknown (nothing shown) */
export function useEye(path: string): EyeLabel | null {
  // the status arrives (or the eye model appears) after the first cells are mounted: ask again then
  const active = useSyncExternalStore(subscribePeople, eyesActive);
  useEffect(() => { want(); }, [active]);
  return useSyncExternalStore((l) => subscribe(path, l), () => (active ? known.get(path) ?? null : null));
}

/** the cached verdict, for code that is not a component */
export const eyeOf = (path: string): EyeLabel | null => (eyesActive() ? known.get(path) ?? null : null);
