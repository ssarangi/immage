// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The closed-eye verdict of each photo the page is showing, for the glyph on its thumbnail and scene card. A cell watches its
// photo while it is mounted (the grids mount only what is on screen), the queue (eyesQueue.ts) asks the backend for the
// watched photos a few at a time after scrolling pauses, and the answers are kept per photo. Nothing is asked while Faces is
// off or there is no eye model; the verdict is the "preview" of notes/faces-p4-backend-notes.md until the model is validated.
import { useEffect, useSyncExternalStore } from "react";
import { EyeQueue, scanPaths, type EyeLabel } from "./eyesQueue";
import { eyeVerdict, setEyeVerdicts } from "./eyesKnown";
import { eyesVersion, subscribeEyes } from "./eyesKnown";
import { peopleApi, peopleState, subscribePeople } from "./peopleStore";

export type { EyeLabel };

const watchers = new Map<string, number>();
const listeners = new Map<string, Set<() => void>>();
let lastTick = -1;

const queue = new EyeQueue(
  (paths) => peopleApi().eyesFor(paths, null).then((r) => r.map((x) => ({ path: x.path, state: x.state, primaryFace: x.primaryFace }))),
  (results) => {
    for (const path of setEyeVerdicts(results)) listeners.get(path)?.forEach((l) => l());
  },
);

/** whether eye verdicts can exist: Faces is on and the optional eye model file is there */
export const eyesActive = () => { const s = peopleState().status; return !!s && s.enabled && s.installed && s.eyes_available; };

const want = () => {
  const s = peopleState();
  // the faces changed (naming, a scan): the verdicts are asked for again
  // Faces changed (a scan, naming, or this very store's batches, which make the backend say so): the verdicts stay, only the photos still
  // unknown are asked for again. Forgetting them all here once emptied Find's Closed eyes result the moment its own scan finished.
  if (s.tick !== lastTick) { lastTick = s.tick; queue.reask((p) => (eyeVerdict(p) ?? null) === null); }
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
  return useSyncExternalStore((l) => subscribe(path, l), () => (active ? eyeVerdict(path) ?? null : null));
}

/** the cached verdict, for code that is not a component */
export const eyeOf = (path: string): EyeLabel | null => (eyesActive() ? eyeVerdict(path) ?? null : null);

// ---- Find's Closed eyes: the open folder's photos, in chunks, with a progress line ----
// `total` is the photos still to check when the run began (those with no verdict yet), `known` the folder's photos with a verdict now
export type EyesScan = { running: boolean; done: number; total: number; known: number };
let scan: EyesScan = { running: false, done: 0, total: 0, known: 0 };
let scanToken = 0;
const scanListeners = new Set<() => void>();
const setScan = (s: EyesScan) => { scan = s; scanListeners.forEach((l) => l()); };
export const useEyesScan = () => useSyncExternalStore((l) => { scanListeners.add(l); return () => { scanListeners.delete(l); }; }, () => scan);
/** the eye verdicts changed: a number for the memo that filters the photos */
export const useEyesVersion = () => useSyncExternalStore(subscribeEyes, eyesVersion);
/** whether the Closed eyes row can exist */
export const useEyesOn = () => useSyncExternalStore(subscribePeople, eyesActive);

/** asks for every photo's verdict, chunk by chunk; a newer call (another folder) or `stopEyesScan` ends this one */
export function startEyesScan(paths: readonly string[]) {
  const token = ++scanToken;
  if (!eyesActive()) { setScan({ running: false, done: 0, total: 0, known: 0 }); return; }
  const known = () => paths.reduce((n, p) => n + (eyeVerdict(p) !== undefined ? 1 : 0), 0);
  void scanPaths(paths, (p) => peopleApi().eyesFor(p, null).then((r) => r.map((x) => ({ path: x.path, state: x.state, primaryFace: x.primaryFace }))), {
    have: (p) => eyeVerdict(p) !== undefined, onChunk: (r) => { setEyeVerdicts(r); },
    progress: (done, total) => { if (token === scanToken) setScan({ running: done < total, done, total, known: known() }); }, stopped: () => token !== scanToken,
  }).catch(() => {}).then(() => { if (token === scanToken) setScan({ ...scan, running: false, known: known() }); });
}
export const stopEyesScan = () => { scanToken++; setScan({ running: false, done: 0, total: 0, known: 0 }); };
