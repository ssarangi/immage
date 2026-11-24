// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Find's People row (notes/faces-ux-spec.md 4.1) as data and as a test on a photo's path: who must be in the photo,
// who must not, whether the included people must all be there ("together"), photos with no people, photos with
// faces still to name. The backend turns a filter into paths (`faces_filter_paths`: the photos that may pass, and
// those that never do); the page keeps them per filter, so the test stays synchronous like the rest of the folder
// filter. Until the paths of a filter have arrived its photos are not shown.

export type FaceFilter = { include: number[]; exclude: number[]; together: boolean; none: boolean; unnamed: boolean };
export const NO_FACES: FaceFilter = { include: [], exclude: [], together: false, none: false, unnamed: false };

export const facesOnFilter = (f: FaceFilter | undefined): boolean =>
  !!f && (f.include.length > 0 || f.exclude.length > 0 || f.none || f.unnamed);

// a stored filter, filled out (an older saved filter has none)
export const normalizeFaces = (f: Partial<FaceFilter> | undefined): FaceFilter => ({ ...NO_FACES, ...(f ?? {}) });

// where the paths come from; peopleStore.ts registers its PeopleApi (a harness registers its fake)
export type FaceSource = { filterPaths(f: FaceFilter): Promise<{ allow: string[] | null; deny: string[] }> };
let source: (() => FaceSource) | null = null;
export const setFaceSource = (f: () => FaceSource) => { source = f; };

export type FacePaths = { allow: ReadonlySet<string> | null; deny: ReadonlySet<string> };
export type FaceSets = ReadonlyMap<string, FacePaths>;

// a filter as the key its paths are kept under (the order of the people does not matter)
export const filterKey = (f: FaceFilter) => JSON.stringify([[...f.include].sort(), [...f.exclude].sort(), f.together, f.none, f.unnamed]);

// whether a photo passes: in the allowed set (when the filter has positive conditions) and not in the denied one.
// A filter whose paths are not here yet admits nothing, so the grid never flashes the wrong photos.
export function faceAllows(path: string, f: FaceFilter, sets: FaceSets): boolean {
  const p = sets.get(filterKey(f));
  if (!p) return false;
  return (p.allow === null || p.allow.has(path)) && !p.deny.has(path);
}

// ---- the paths, loaded when a filter needs them ----

let cache = new Map<string, FacePaths>();
let version = 0, unavailable = false;
const inflight = new Set<string>();
const listeners = new Set<() => void>();
const bump = () => { version++; listeners.forEach((l) => l()); };
export const subscribeFaceSets = (l: () => void) => { listeners.add(l); return () => { listeners.delete(l); }; };
export const faceSetsVersion = () => version;
export const faceSets = (): FaceSets => cache;
export const setsUnavailable = () => unavailable;

// asks for what a filter needs; the filmstrip shows the photos once the paths are in
export function wantFaceSets(f: FaceFilter | undefined) {
  if (!f || !facesOnFilter(f) || !source) return;
  const key = filterKey(f);
  if (cache.has(key) || inflight.has(key)) return;
  inflight.add(key);
  source().filterPaths(f).then((r) => {
    cache = new Map(cache).set(key, { allow: r.allow ? new Set(r.allow) : null, deny: new Set(r.deny) });
  }, () => {
    unavailable = true;
    cache = new Map(cache).set(key, { allow: new Set(), deny: new Set() });
  }).finally(() => { inflight.delete(key); bump(); });
}

// a change to the faces (naming, merging) makes every set stale
export function dropFaceSets() {
  cache = new Map(); unavailable = false;
  bump();
}
