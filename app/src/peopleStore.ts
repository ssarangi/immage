// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The People view's state, as a small external store (as scenesStore.ts is): the backend's status and
// lists, polled and refreshed by the `faces-changed` and `faces-progress` events at most every 2 s (and
// held while the pointer is on a tile, so nothing moves under it), the one Undo line, and every change the
// user makes: optimistic, undone on failure, each reversible through `faces_undo`. The page reaches the
// backend only through `PeopleApi`, so the harness runs the real view on fixtures.
import { useSyncExternalStore } from "react";
import { commands } from "./commands";
import type { ClusterRow, FaceFilter, FaceRow, FacesStats, FacesStatus, MergeSuggestion, PersonRow, Side } from "./bindings";
import { settings } from "./settings";
import { fromBackend } from "./faceOverlayModel";
import { dropFaceSets, setFaceSource, type FaceFilter as PeopleQuery } from "./facesFilter";
import { setScanJob } from "./background";
import {
  mergeUndo, faceKey, groupKey, keyId, maybeKey, needsReview, scanJob, scanView, type PeopleData, type ScanCtx, type ScanView, type UndoEntry, type UndoKind,
} from "./peopleModel";

// one face of one photo, in the displayed orientation (boxes are fractions of the picture), for the overlay and the Library card
export type PhotoFace = {
  face: number; x: number; y: number; w: number; h: number;
  person: number | null; name: string | null; state: "confirmed" | "suggested" | "unassigned" | "rejected"; confidence: number;
};
// the person page, the Find filter and the overlay read these from the backend (bindings.ts has the commands)
export type PersonPhotos = { paths: string[]; total: number };
export type FilterPaths = { allow: string[] | null; deny: string[] };

export type PeopleApi = {
  personPhotos(person: number, offset: number, limit: number): Promise<PersonPhotos>;
  photoFaces(path: string): Promise<PhotoFace[]>;
  filterPaths(f: PeopleQuery): Promise<FilterPaths>;
  status(): Promise<FacesStatus>;
  paused(): Promise<boolean>;
  people(): Promise<PersonRow[]>;
  groups(): Promise<ClusterRow[]>;
  list(filter: FaceFilter, offset: number, limit: number): Promise<FaceRow[]>;
  createPerson(name: string, faces: number[]): Promise<number>;
  rename(id: number, name: string): Promise<void>;
  merge(from: number, into: number): Promise<void>;
  remove(id: number, removed: boolean): Promise<void>;
  cover(id: number, face: number): Promise<void>;
  assign(person: number, faces: number[]): Promise<void>;
  notPerson(person: number, faces: number[]): Promise<void>;
  notFace(faces: number[]): Promise<void>;
  unignore(faces: number[]): Promise<void>;
  failedPhotos(limit: number): Promise<string[]>;
  mergeSuggestions(): Promise<MergeSuggestion[]>;
  stats(): Promise<FacesStats>;
  mergeGroups(from: number, into: number): Promise<void>;
  notSame(a: Side, b: Side): Promise<void>;
  undo(): Promise<string | null>;
  deleteAll(): Promise<void>;
  retry(): Promise<void>;
  scanFolder(path: string): Promise<void>;
  setEnabled(on: boolean): Promise<void>;
  pause(paused: boolean): Promise<void>;
  /** a face's picture as JPEG bytes; null when the backend has no way to hand them to the page */
  crop(face: number, size: number): Promise<Blob | null>;
  /** the backend's events; returns the way to stop listening */
  subscribe(onChanged: () => void, onProgress: (p: { scanned: number; pending: number | null; faces_found: number }) => void): () => void;
};

const real: PeopleApi = {
  personPhotos: (person, offset, limit) => commands.facesPersonPhotos(person, offset, limit),
  photoFaces: async (path) => fromBackend(await commands.facesPhotoFaces(path)),
  filterPaths: (f) => commands.facesFilterPaths(f),
  status: () => commands.facesStatus(),
  paused: async () => (await commands.analysisStatus()).paused,
  people: () => commands.facesPeople(true),
  groups: () => commands.facesGroups(),
  list: (f, o, n) => commands.facesList(f, o, n),
  createPerson: (name, faces) => commands.facesPersonCreate(name, faces),
  rename: async (id, name) => { await commands.facesPersonRename(id, name); },
  merge: async (from, into) => { await commands.facesPersonMerge(from, into); },
  remove: async (id, removed) => { await commands.facesPersonRemove(id, removed); },
  cover: async (id, face) => { await commands.facesPersonCover(id, face); },
  assign: async (person, faces) => { await commands.facesAssign(person, faces); },
  notPerson: async (person, faces) => { await commands.facesNotPerson(person, faces); },
  notFace: async (faces) => { await commands.facesNotFace(faces); },
  unignore: async (faces) => { await commands.facesUnignore(faces); },
  failedPhotos: (limit) => commands.facesFailedPhotos(limit),
  mergeSuggestions: () => commands.facesMergeSuggestions(),
  stats: () => commands.facesStats(),
  mergeGroups: async (from, into) => { await commands.facesMergeGroups(from, into); },
  notSame: async (a, b) => { await commands.facesNotSame(a, b); },
  undo: () => commands.facesUndo(),
  deleteAll: async () => { await commands.facesDeleteAll(); },
  retry: async () => { await commands.facesRetry(); },
  scanFolder: async (path) => { await commands.facesScanFolderWith(path, true, false); },   // network shares stay off: no control turns them on
  setEnabled: async (on) => { await commands.analysisSet({ id: "faces", enabled: on }); },
  pause: async (paused) => { await commands.analysisPause(paused); },
  crop: async (face, size) => new Blob([await commands.facesCropBytes(face, size)], { type: "image/jpeg" }),
  subscribe: (onChanged, onProgress) => {
    const stops: (() => void)[] = [];
    let dead = false;
    void (async () => {
      try {
        const { listen } = await import("@tauri-apps/api/event");
        const a = await listen("faces-changed", () => onChanged());
        const b = await listen<{ scanned: number; pending: number | null; faces_found: number }>("faces-progress", (e) => onProgress(e.payload));
        if (dead) { a(); b(); } else stops.push(a, b);
      } catch { /* no event bridge (a plain browser): the poll keeps it current */ }
    })();
    return () => { dead = true; stops.forEach((s) => s()); };
  },
};
let api: PeopleApi = real;
setFaceSource(() => api);
export const setPeopleApi = (a: PeopleApi) => { api = a; };
export const peopleApi = () => api;

export type PeopleState = {
  on: boolean;                       // the view shows
  status: FacesStatus | null;
  failed: boolean;                   // the status could not be read
  paused: boolean;
  scan: ScanView;
  loaded: boolean;                   // the lists have been read once
  tick: number;                      // bumped whenever the faces changed (naming, a scan), for what shows one photo's faces
  suggestions: MergeSuggestion[];    // pairs the backend thinks are one person (never acted on by itself)
  data: PeopleData;
  undo: (UndoEntry & { id: number }) | null;
  error: string;                     // the last change that failed, said plainly
  busy: boolean;
};

const EMPTY_DATA: PeopleData = { people: [], groups: [], unknown: [], unknownMore: false, ignoredFaces: [], ignoredMore: false };
const NO_STATUS_SCAN: ScanView = { kind: "off", done: 0, total: null, left: null, reason: null };
const EMPTY: PeopleState = { on: false, status: null, failed: false, paused: false, scan: NO_STATUS_SCAN, loaded: false, tick: 0, suggestions: [], data: EMPTY_DATA, undo: null, error: "", busy: false };

let state: PeopleState = EMPTY;
const listeners = new Set<() => void>();
export const peopleState = () => state;
export function setPeople(patch: Partial<PeopleState>) {
  state = { ...state, ...patch };
  listeners.forEach((l) => l());
}
const subscribe = (l: () => void) => { listeners.add(l); return () => { listeners.delete(l); }; };
export const usePeople = () => useSyncExternalStore(subscribe, peopleState);
export const usePeopleOn = () => useSyncExternalStore(subscribe, () => state.on);
export const usePeopleBadge = () => useSyncExternalStore(subscribe, () => needsReview(state.status));

export const resetPeople = (patch: Partial<PeopleState> = {}) => { state = { ...EMPTY, ...patch }; listeners.forEach((l) => l()); };
export const setPeopleOn = (on: boolean) => {
  if (state.on === on) return;
  setPeople({ on });
  if (on) void refreshAll();
};

// ---- reading ----

const UNKNOWN_PAGE = 500;
let holds = 0, dirty = false, lastRead = 0, timer: ReturnType<typeof setTimeout> | undefined, reading = false;
const READ_GAP_MS = 2000;

// the folder scope has no photo total in the backend: the page knows the scanned folder's photo count
let folderTotal: number | null = null;
const scanCtx = (): ScanCtx => ({ folderScope: settings.get(SCOPE_KEY) === "folder", folderTotal });
export function setScanFolderTotal(open: string | null, n: number) {
  const next = open !== null && n > 0 && open === scopeFolderPath(open) ? n : null;
  if (next === folderTotal) return;
  folderTotal = next;
  if (state.status) applyStatus(state.status, state.paused);
}

function applyStatus(s: FacesStatus, paused: boolean) {
  const before = state.scan.kind;
  const scan = scanView(s, paused, scanCtx());
  setPeople({ status: s, failed: false, paused, scan });
  setScanJob(scanJob(scan));
  // the scan ended: the lists are read once more, whatever the pointer is over
  if ((before === "scanning" || before === "paused") && scan.kind === "idle" && state.on) void refreshAll(true);
}

// no call may wedge the view: a command that never answers (a lock held by the scan, a dead bridge) is given up on
// after CALL_MS, so the guards below always reset and the next poll tries again
const CALL_MS = 10000;
const bounded = <T,>(p: Promise<T>, ms = CALL_MS): Promise<T> => new Promise<T>((res, rej) => {
  const t = setTimeout(() => rej(new Error("no answer")), ms);
  p.then((v) => { clearTimeout(t); res(v); }, (e) => { clearTimeout(t); rej(e); });
});
// what the lists depend on: when it changes the lists are read again, when it does not they are left alone
const sigOf = (s: FacesStatus | null) => (s ? JSON.stringify([s.totals, s.enabled, s.installed]) : "");
let listSig = "", statusBusy = false;

export async function refreshStatus() {
  if (statusBusy) return;
  statusBusy = true;
  try {
    const [s, paused] = await Promise.all([bounded(api.status()), bounded(api.paused()).catch(() => state.paused)]);
    applyStatus(s, paused);
  } catch { if (!state.status) setPeople({ failed: true }); }
  finally { statusBusy = false; }
}

// the lists. While a pointer is on a tile the answer waits (`holdRefresh(false)` brings it), and at most one
// read runs per READ_GAP_MS, so a long scan does not repaint the grid on every photo.
export async function refreshAll(force = false) {
  if (!state.on) { await refreshStatus(); return; }
  if (reading) { dirty = true; return; }
  const wait = lastRead + READ_GAP_MS - Date.now();
  if (wait > 0 && !force) { schedule(wait); return; }
  reading = true; dirty = false; lastRead = Date.now();
  try {
    const [s, paused, people, groups, unknown, ignored] = await Promise.all([
      bounded(api.status()), bounded(api.paused()).catch(() => state.paused), bounded(api.people()), bounded(api.groups()),
      bounded(api.list({ kind: "unknown" }, 0, UNKNOWN_PAGE)), bounded(api.list({ kind: "ignored" }, 0, UNKNOWN_PAGE)),
    ]);
    listSig = sigOf(s);
    // quiet: no pairs is a fine answer, and the lists never wait for it
    const pairs = await bounded(api.mergeSuggestions()).catch(() => state.suggestions);
    const prev = state.data.unknown;
    // more than the first page was loaded before: keep that much, so the grid does not shrink under the user
    const keep = prev.length > unknown.length && unknown.length === UNKNOWN_PAGE ? await api.list({ kind: "unknown" }, 0, prev.length) : unknown;
    applyStatus(s, paused);
    setPeople({ suggestions: pairs });
    // a pointer on the grid keeps the tiles still, but only when there are tiles to keep: the counts and the
    // strip always move, and a grid that is empty fills at once
    const held = holds > 0 && !force && (state.data.people.length + state.data.groups.length + state.data.unknown.length + state.data.ignoredFaces.length) > 0;
    if (held) { dirty = true; return; }
    setPeople({ data: unsettled({ people, groups, unknown: keep, unknownMore: keep.length >= UNKNOWN_PAGE && keep.length % UNKNOWN_PAGE === 0, ignoredFaces: ignored, ignoredMore: ignored.length >= UNKNOWN_PAGE }), loaded: true });
  } catch (e) {
    if (!state.status) setPeople({ failed: true }); else setPeople({ error: String(e) });
  } finally { reading = false; }
  if (dirty && holds === 0) schedule(READ_GAP_MS);
}

function schedule(ms: number) {
  clearTimeout(timer);
  timer = setTimeout(() => { void refreshAll(); }, Math.max(50, ms));
}

export async function loadMoreUnknown() {
  const d = state.data;
  if (!d.unknownMore) return;
  try {
    const more = await api.list({ kind: "unknown" }, d.unknown.length, UNKNOWN_PAGE);
    const seen = new Set(state.data.unknown.map((f) => f.id));
    const add = more.filter((f) => !seen.has(f.id));
    setPeople({ data: { ...state.data, unknown: [...state.data.unknown, ...add], unknownMore: more.length >= UNKNOWN_PAGE } });
  } catch (e) { setPeople({ error: String(e) }); }
}

export async function loadMoreIgnored() {
  const d = state.data;
  if (!d.ignoredMore) return;
  try {
    const more = await api.list({ kind: "ignored" }, d.ignoredFaces.length, UNKNOWN_PAGE);
    const seen = new Set(state.data.ignoredFaces.map((f) => f.id));
    setPeople({ data: { ...state.data, ignoredFaces: [...state.data.ignoredFaces, ...more.filter((f) => !seen.has(f.id))], ignoredMore: more.length >= UNKNOWN_PAGE } });
  } catch (e) { setPeople({ error: String(e) }); }
}

export function holdRefresh(on: boolean) {
  holds = Math.max(0, holds + (on ? 1 : -1));
  if (holds === 0 && dirty) schedule(300);
}

// the app calls this once: events and a slow poll (memory-only commands) keep the tab's dot and the activity
// line current; the lists are read only while the view is open
export function startPeople(): () => void {
  void refreshStatus();
  const stop = api.subscribe(
    () => { dropFaceSets(); setPeople({ tick: state.tick + 1 }); if (state.on) { dirty = true; schedule(READ_GAP_MS); } else void refreshStatus(); },
    (p) => {
      if (!state.status) return;
      applyStatus({ ...state.status, scanned: p.scanned, pending: p.pending, faces_found: p.faces_found }, state.paused);
    },
  );
  // the status is memory-only and always read; the lists only when what they depend on changed (or an event said so)
  const poll = setInterval(() => {
    void refreshStatus().then(() => {
      if (state.on && (sigOf(state.status) !== listSig || dirty)) void refreshAll();
    });
  }, 2000);
  return () => { stop(); clearInterval(poll); clearTimeout(timer); };
}

// ---- changes ----

// Faces the user has just named or ignored. The backend regroups when it is idle, so for a while it can still list a
// group whose faces are all named; those groups are kept out of the lists here (by their faces), so the grid and the
// counts drop them at once and for good. Undo and Delete clear it, and the next read is then the backend's own.
const settled = new Set<number>();
const joined = new Set<number>();   // groups that were joined into another
const unsettled = (d: PeopleData): PeopleData => !settled.size && !joined.size ? d : {
  ...d, groups: d.groups.filter((g) => !joined.has(g.id) && !(g.members?.length ? g.members : [g.rep_face_id]).some((m) => settled.has(m))),
  unknown: d.unknown.filter((f) => !settled.has(f.id)),
};
const settle = (faces: readonly number[]) => { faces.forEach((f) => settled.add(f)); if (settled.size > 20000) settled.clear(); };

let undoId = 1;
const entry = (kind: UndoKind, n: number, name = "", other = ""): UndoEntry => ({ kind, n, name, other, steps: 1 });

function postUndo(e: UndoEntry) {
  const prev = state.undo;
  const merged = mergeUndo(prev && { kind: prev.kind, n: prev.n, name: prev.name, other: prev.other, steps: prev.steps }, e);
  setPeople({ undo: { ...merged, id: undoId++ }, error: "" });
}
export const clearUndo = () => { if (state.undo) setPeople({ undo: null }); };

// `local` shows the change at once; a refusal puts the lists back and says why
async function change(local: (d: PeopleData) => PeopleData, call: () => Promise<void>, done: UndoEntry | null) {
  const before = state.data;
  setPeople({ data: local(before), busy: true, error: "" });
  try {
    await call();
    if (done) postUndo(done);
  } catch (e) {
    settled.clear(); joined.clear();
    setPeople({ data: before, error: String(e) });
  } finally {
    dropFaceSets();
    setPeople({ busy: false, tick: state.tick + 1 });
    void refreshAll(true);
  }
}

export async function undoLast(steps: number) {
  settled.clear(); joined.clear();
  setPeople({ undo: null, busy: true });
  try { for (let i = 0; i < steps; i++) if (!(await api.undo())) break; }
  catch (e) { setPeople({ error: String(e) }); }
  finally { setPeople({ busy: false }); void refreshAll(true); }
}

// the faces behind tiles: a group's and a suggestion's are asked for, a face is itself
export async function facesOf(keys: readonly string[]): Promise<number[]> {
  const out: number[] = [];
  for (const k of keys) {
    const id = keyId(k);
    if (k.startsWith("f:")) out.push(id);
    else if (k.startsWith("g:")) out.push(...(await api.list({ kind: "cluster", id }, 0, 2000)).map((f) => f.id));
    else if (k.startsWith("s:")) out.push(...(await api.list({ kind: "suggested", id }, 0, 2000)).map((f) => f.id));
  }
  return [...new Set(out)];
}

const without = (d: PeopleData, keys: readonly string[]): PeopleData => {
  const k = new Set(keys);
  return { ...d, ignoredFaces: d.ignoredFaces.filter((f) => !k.has(faceKey(f.id))), groups: d.groups.filter((g) => !k.has(groupKey(g.id))), unknown: d.unknown.filter((f) => !k.has(faceKey(f.id))),
           people: d.people.map((p) => (k.has(maybeKey(p.id)) ? { ...p, suggestions: 0 } : p)) };
};

export type NameTarget = { kind: "person"; id: number; name: string } | { kind: "create"; name: string };

// names the faces behind these tiles: all of them confirmed
export async function nameTiles(keys: readonly string[], to: NameTarget) {
  const faces = await facesOf(keys);
  if (!faces.length) return;
  settle(faces);
  await change((d) => without(d, keys),
    async () => { if (to.kind === "create") await api.createPerson(to.name, faces); else await api.assign(to.id, faces); },
    entry("assigned", faces.length, to.name));
}

// a suggestion confirmed, or one more face confirmed for the person it was suggested for
export async function confirmTiles(keys: readonly string[], names: ReadonlyMap<number, string>) {
  for (const k of keys) {
    if (!k.startsWith("s:")) continue;
    const person = keyId(k), faces = await facesOf([k]);
    if (faces.length) settle(faces);
    if (faces.length) await change((d) => without(d, [k]), () => api.assign(person, faces), entry("confirmed", faces.length, names.get(person) ?? ""));
  }
}

// "Not Maya": the suggestion or the face is rejected for that person for good
export async function notPerson(keys: readonly string[], faceMaybe: ReadonlyMap<string, number | null>, names: ReadonlyMap<number, string>) {
  for (const k of keys) {
    const person = k.startsWith("s:") ? keyId(k) : faceMaybe.get(k);
    if (person == null) continue;
    const faces = await facesOf([k]);
    if (faces.length) await change((d) => without(d, [k]), () => api.notPerson(person, faces), entry("notPerson", faces.length, names.get(person) ?? ""));
  }
}

// ignore faces ("not a face") or, in People, ignore people; restore in Ignored
export async function ignoreTiles(keys: readonly string[]) {
  const people = keys.filter((k) => k.startsWith("p:")).map(keyId);
  const faces = await facesOf(keys.filter((k) => !k.startsWith("p:") && !k.startsWith("s:")));
  if (faces.length) settle(faces);
  if (faces.length) await change((d) => without(d, keys), () => api.notFace(faces), entry("ignored", faces.length));
  for (const id of people) {
    const p = state.data.people.find((x) => x.id === id);
    await change((d) => ({ ...d, people: d.people.map((x) => (x.id === id ? { ...x, state: "ignored" } : x)) }), () => api.remove(id, true), entry("ignored", p?.faces ?? 1));
  }
}
// faces taken back from "not a face"
export async function unignoreFaces(keys: readonly string[]) {
  const faces = keys.filter((k) => k.startsWith("f:")).map(keyId);
  if (faces.length) await change((d) => without(d, keys), () => api.unignore(faces), entry("unignored", faces.length));
}
export async function restorePeople(ids: readonly number[]) {
  for (const id of ids) {
    const p = state.data.people.find((x) => x.id === id);
    await change((d) => ({ ...d, people: d.people.map((x) => (x.id === id ? { ...x, state: "active" } : x)) }), () => api.remove(id, false), entry("restored", 1, p?.name ?? ""));
  }
}

export async function renamePerson(id: number, name: string) {
  const old = state.data.people.find((p) => p.id === id)?.name ?? "";
  await change((d) => ({ ...d, people: d.people.map((p) => (p.id === id ? { ...p, name } : p)) }), () => api.rename(id, name), entry("renamed", 1, name, old));
}

// `from` people into `into`: their faces and photos become `into`'s
export async function mergePeople(from: readonly number[], into: number) {
  const target = state.data.people.find((p) => p.id === into);
  for (const id of from) {
    const p = state.data.people.find((x) => x.id === id);
    await change((d) => ({ ...d, people: d.people.filter((x) => x.id !== id).map((x) => (x.id === into && p ? { ...x, faces: x.faces + p.faces, photos: x.photos + p.photos } : x)) }),
      () => api.merge(id, into), entry("merged", 1, target?.name ?? "", p?.name ?? ""));
  }
}

// unnamed groups made one (faces_merge_groups): `from` joins `into`, the grid shows it at once
export async function mergeGroups(from: readonly number[], into: number) {
  for (const id of from) {
    const g = state.data.groups.find((x) => x.id === id);
    joined.add(id);
    await change((d) => ({ ...d, groups: d.groups.filter((x) => x.id !== id).map((x) => (x.id === into ? { ...x, size: x.size + (g?.size ?? 0) } : x)) }),
      () => api.mergeGroups(id, into), entry("joined", g?.size ?? 1));
  }
}

// "not the same", remembered by the backend, so the pair is not offered again
export async function notSame(a: Side, b: Side) {
  await change((d) => d, () => api.notSame(a, b), entry("notSame", 1));
}

export async function setCover(id: number, face: number) {
  const name = state.data.people.find((p) => p.id === id)?.name ?? "";
  await change((d) => ({ ...d, people: d.people.map((p) => (p.id === id ? { ...p, cover_face_id: face, cover_manual: true } : p)) }), () => api.cover(id, face), entry("cover", 1, name));
}

export const failedPhotos = (limit = 200) => api.failedPhotos(limit);
export const personFaces = (id: number, limit = 24) => api.list({ kind: "person", id }, 0, limit);
export const groupFaces = (id: number, limit = 200) => api.list({ kind: "cluster", id }, 0, limit);
export const maybeFaces = (id: number, limit = 200) => api.list({ kind: "suggested", id }, 0, limit);

// ---- turning it on and off ----

export type Scope = "catalog" | "folder";
export const SCOPE_KEY = "faces.scope";
// the folder a "folder" scope covers: the one stored when People was turned on, else the open one (the scope was set
// without the page, as the settings file or an older build would), else none
export function scopeFolderPath(open: string | null): string | null {
  if (settings.get(SCOPE_KEY) !== "folder") return null;
  return settings.get("faces.scope_folder") ?? open;
}
export const SCOPE_FOLDER_KEY = "faces.scope_folder";
export const CONSENT_KEY = "faces.consent_at";

// the consent line was ticked: it is recorded locally, the scope is kept for the scan, People turns on
export async function turnOn(scope: Scope, folder: string | null) {
  setPeople({ busy: true, error: "" });
  try {
    settings.set(SCOPE_KEY, scope);
    if (scope === "folder" && folder) settings.set(SCOPE_FOLDER_KEY, folder); else settings.remove(SCOPE_FOLDER_KEY);
    settings.set(CONSENT_KEY, new Date().toISOString());
    if (scope === "folder" && folder) await api.scanFolder(folder);
    await api.setEnabled(true);
    await refreshAll(true);
  } catch (e) { setPeople({ error: String(e) }); }
  finally { setPeople({ busy: false }); }
}
// People turned on again after having been off (its consent is already recorded; the first time is the People view's card)
export async function resumePeople() {
  try { await api.setEnabled(true); await refreshAll(true); } catch (e) { setPeople({ error: String(e) }); }
}
export async function turnOff() {
  try { await api.setEnabled(false); await refreshAll(true); } catch (e) { setPeople({ error: String(e) }); }
}
export async function deleteAllFaceData() {
  settled.clear(); joined.clear();
  try { await api.deleteAll(); resetPeople({ on: state.on }); await refreshAll(true); } catch (e) { setPeople({ error: String(e) }); }
}
export async function togglePause() {
  try { await api.pause(!state.paused); await refreshStatus(); } catch (e) { setPeople({ error: String(e) }); }
}
export const retryFailed = () => api.retry().then(() => refreshStatus(), (e) => setPeople({ error: String(e) }));

// the people, read once for a picker that is not the People view (Find's row, the overlay's name box)
let ensured: Promise<void> | null = null;
export function ensurePeople(): Promise<void> {
  ensured ??= (async () => {
    try {
      const [s, people] = await Promise.all([bounded(api.status()), bounded(api.people())]);
      setPeople({ status: s, data: { ...state.data, people } });
    } catch { /* the pickers stay empty; the next ask tries again */ }
    finally { setTimeout(() => { ensured = null; }, 10000); }
  })();
  return ensured;
}
// whether faces are on, for the places that show nothing when they are off (Find's row, the overlay, the card)
export const useFacesOn = () => useSyncExternalStore(subscribe, () => !!state.status?.enabled && !!state.status?.installed);
export const facesOn = () => !!state.status?.enabled && !!state.status?.installed;
