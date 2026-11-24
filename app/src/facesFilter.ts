// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Find's People row (notes/faces-ux-spec.md 4.1) as data and as a test on a photo's path: who must be in the photo,
// who must not, whether the included people must all be there ("together"), photos with no people, photos with
// faces still to name. The test reads path sets the page has asked the backend for (`faceSets`), so it is
// synchronous like the rest of the folder filter; until a set has arrived the photo is not shown.

// where the sets come from; peopleStore.ts registers its PeopleApi (a harness registers its fake)
export type FaceSource = {
  personPhotos(person: number, offset: number, limit: number): Promise<{ paths: string[]; total: number }>;
  photoSets(): Promise<{ withFaces: string[]; unnamed: string[] }>;
};
let source: (() => FaceSource) | null = null;
export const setFaceSource = (f: () => FaceSource) => { source = f; };

export type FaceFilter = { include: number[]; exclude: number[]; together: boolean; none: boolean; unnamed: boolean };
export const NO_FACES: FaceFilter = { include: [], exclude: [], together: false, none: false, unnamed: false };

export const facesOnFilter = (f: FaceFilter | undefined): boolean =>
  !!f && (f.include.length > 0 || f.exclude.length > 0 || f.none || f.unnamed);

// a stored filter, filled out (an older saved filter has none)
export const normalizeFaces = (f: Partial<FaceFilter> | undefined): FaceFilter => ({ ...NO_FACES, ...(f ?? {}) });

export type FaceSets = { person: ReadonlyMap<number, ReadonlySet<string>>; withFaces: ReadonlySet<string> | null; unnamed: ReadonlySet<string> | null };

// whether a photo passes. include: any of them (or all, together); exclude: none of them; none: no faces at all;
// unnamed: a face still to name. Rows combine as "and". A set that is not here yet says no.
export function faceAllows(path: string, f: FaceFilter, sets: FaceSets): boolean {
  if (f.include.length) {
    const has = f.include.map((id) => sets.person.get(id));
    if (has.some((s) => !s)) return false;
    const ok = f.together ? has.every((s) => s!.has(path)) : has.some((s) => s!.has(path));
    if (!ok) return false;
  }
  if (f.exclude.length) {
    const has = f.exclude.map((id) => sets.person.get(id));
    if (has.some((s) => !s)) return false;
    if (has.some((s) => s!.has(path))) return false;
  }
  if (f.none) { if (!sets.withFaces || sets.withFaces.has(path)) return false; }
  if (f.unnamed) { if (!sets.unnamed || !sets.unnamed.has(path)) return false; }
  return true;
}

// ---- the sets, loaded when a filter needs them ----

const PAGE = 2000;
let person = new Map<number, ReadonlySet<string>>();
let withFaces: ReadonlySet<string> | null = null, unnamed: ReadonlySet<string> | null = null;
let version = 0, unavailable = false;
export const setsUnavailable = () => unavailable;
const inflight = new Set<string>();
const listeners = new Set<() => void>();
const bump = () => { version++; listeners.forEach((l) => l()); };
const subscribe = (l: () => void) => { listeners.add(l); return () => { listeners.delete(l); }; };
export const faceSets = (): FaceSets => ({ person, withFaces, unnamed });
// a number that changes when a set arrives or is dropped, for the memo that filters the photos
export const subscribeFaceSets = subscribe;
export const faceSetsVersion = () => version;

async function loadPerson(id: number) {
  const k = `p${id}`;
  if (!source || person.has(id) || inflight.has(k)) return;
  inflight.add(k);
  try {
    const all = new Set<string>();
    for (let offset = 0; ; offset += PAGE) {
      const r = await source!().personPhotos(id, offset, PAGE);
      r.paths.forEach((p) => all.add(p));
      if (offset + PAGE >= r.total || !r.paths.length) break;
    }
    person = new Map(person).set(id, all);
  } catch { person = new Map(person).set(id, new Set()); }
  finally { inflight.delete(k); bump(); }
}

async function loadSets() {
  if (!source || unavailable || (withFaces && unnamed) || inflight.has("sets")) return;
  inflight.add("sets");
  try {
    const s = await source!().photoSets();
    withFaces = new Set(s.withFaces); unnamed = new Set(s.unnamed);
  } catch { unavailable = true; }
  finally { inflight.delete("sets"); bump(); }
}

// asks for what a filter needs; the filmstrip shows the photos once the sets are in
export function wantFaceSets(f: FaceFilter | undefined) {
  if (!facesOnFilter(f)) return;
  [...f!.include, ...f!.exclude].forEach((id) => void loadPerson(id));
  if (f!.none || f!.unnamed) void loadSets();
}

// a change to the faces (naming, merging) makes every set stale
export function dropFaceSets() {
  person = new Map(); withFaces = null; unnamed = null; unavailable = false;
  bump();
}
