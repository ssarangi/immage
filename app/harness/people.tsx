// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Dev-only harness for People: the real PeopleView over an in-memory fake of the faces commands (PeopleApi)
// and synthetic faces, in a plain browser, no Tauri. Nothing here is a photo of anyone: a face is a number and
// its picture a generated gradient.
//
//   /harness/people.html?state=review&theme=dark&dir=ltr&w=1440
//
//   state   off (consent, model missing) | download | installed (consent, model ready) | empty | scanning | partial
//           (faces, scan running) | review (default) | naming (review, the name box open) | error
//   offline, net, failed  numbers for the strip notes (folders offline, network sources left out, photos not read)
//   offline, net, failed  counts for the strip notes (folders offline, network sources left out, photos not read)
//   folderscope=1  the folder scan (36 of 333 photos stored, one folder waiting); nofolder=1  no folder is open
//   grow=1  the scan goes on and ends by itself, with no event (the poll alone); stale=0  the backend drops a named group at once
//   many    the number of faces (default 1400; 5000 for the scale run)
//   theme   a built-in theme id        dir  ltr | rtl        w  width of the view in px (default 1440)
//   break=1 adds styles that move things (the shift test must catch it)
//   shift=1 runs the no-layout-shift, keyboard and scale test and writes JSON into #shift-report
import "./tauriShim";
import { createRoot } from "react-dom/client";
import { useEffect, useState } from "react";
import "../src/styles.css";
import { BUILTIN_THEMES, THEME_TOKENS } from "../src/themes";
import type { ClusterRow, FaceFilter, FaceRow, FacesStatus, MergeSuggestion, PackInfo, PersonRow, Side } from "../src/bindings";
import { PeopleView } from "../src/PeopleView";
import { FaceOverlay } from "../src/FaceOverlay";
import { PeopleCard } from "../src/PeopleCard";
import { runOverlayTest } from "./facesOverlayTest";
import { settings } from "../src/settings";
import { peopleState, refreshAll, resetPeople, setPeopleApi, setPeopleOn, setScanFolderTotal, startPeople, type PeopleApi } from "../src/peopleStore";
import { runPeopleShift } from "./peopleShift";

const q = new URLSearchParams(location.search);
const theme = BUILTIN_THEMES.find((t) => t.id === q.get("theme")) ?? BUILTIN_THEMES[0];
for (const token of THEME_TOKENS) document.documentElement.style.setProperty(`--${token}`, theme.colors[token]);
document.documentElement.style.colorScheme = theme.colorScheme;
document.documentElement.dataset.theme = theme.id;
document.documentElement.dataset.scheme = theme.colorScheme;
document.documentElement.dir = q.get("dir") === "rtl" ? "rtl" : "ltr";
document.body.style.cssText = "margin:0;background:var(--window)";
if (q.get("break") === "1") {   // a deliberately bad style: the shift test must catch it (negative control)
  const st = document.createElement("style");
  st.textContent = `.people-undo:not(:has(*)) { height: 0; padding: 0; border: 0; overflow: hidden; }
    .people-strip:not(:has(button)) { height: 0; padding: 0; border: 0; overflow: hidden; }
    .ptile[aria-selected="true"] { margin: 5px; } .ptile:hover .pcap { height: 50px; } .people-selbar { min-height: 0; }
    .people-rail-item[aria-selected="true"] { height: 44px; }`;
  document.head.append(st);
}

// ---- the fake backend ----
type F = { id: number; path: string; person: number | null; state: "unreviewed" | "confirmed" | "rejected"; cluster: number | null; maybe: number | null };
type P = { id: number; name: string; state: "active" | "ignored"; cover: number | null; manual: boolean };
type Db = { faces: F[]; people: P[]; nextPerson: number };
const NAMES = ["Dad", "Maya", "Sam", "Nana", "Alex", "Marcus", "Zoe", "Old statue"];
const total = Number(q.get("many") ?? 1400);
const mode = q.get("state") ?? "review";
const folderScope = q.get("folderscope") === "1";   // the folder scan: the backend counts folders, not photos

function seed(): Db {
  const faces: F[] = [];
  const people: P[] = NAMES.map((name, i) => ({ id: i + 1, name, state: i === NAMES.length - 1 ? "ignored" : "active", cover: null, manual: false }));
  let id = 1;
  const add = (o: Partial<F>) => { faces.push({ id, path: `/synthetic/IMG_${String(id).padStart(5, "0")}.jpg`, person: null, state: "unreviewed", cluster: null, maybe: null, ...o }); id++; };
  const confirmed = Math.floor(total * 0.25);
  for (let i = 0; i < confirmed; i++) add({ person: (i % 7) + 1, state: "confirmed" });
  let cluster = 1, left = total - confirmed;
  while (left > 0 && cluster < 4000) {
    const size = Math.min(left, 2 + ((cluster * 7) % 7));
    for (let k = 0; k < size; k++) add({ cluster, maybe: cluster % 5 === 0 ? 2 : null });
    left -= size; cluster++;
    if (cluster % 3 === 0) { for (let k = 0; k < 2 && left > 0; k++) { add({}); left--; } }   // singles with no group
  }
  for (const f of faces) if (f.person === null && f.maybe === null && f.id % 53 === 0) f.state = "rejected";   // some "not a face"
  if (q.get("view") === "overlay") {   // one photo with four faces, for the overlay and the card
    const g = "/synthetic/Oct 12 - Nani Portraits/_DSC1096.ARW";
    add({ path: g, person: 1, state: "confirmed" }); add({ path: g, person: 2, state: "confirmed" }); add({ path: g, maybe: 2 }); add({ path: g });
  }
  for (const p of people) p.cover = faces.find((f) => f.person === p.id)?.id ?? null;
  return { faces, people, nextPerson: people.length + 1 };
}

let db: Db = seed();
const undoStack: Db[] = [];
const calls: string[] = [];
const snap = (): Db => ({ faces: db.faces.map((f) => ({ ...f })), people: db.people.map((p) => ({ ...p })), nextPerson: db.nextPerson });
const mutate = (label: string, fn: () => void) => { calls.push(label); undoStack.push(snap()); fn(); sizes = null; };

const scan = { photos: null as number | null, fw: 0, offline: Number(q.get("offline") ?? 0), net: Number(q.get("net") ?? 0), failed: Number(q.get("failed") ?? 0), enabled: mode !== "off" && mode !== "download" && mode !== "installed", installed: mode !== "off" && mode !== "download", scanned: 0, pending: null as number | null, health: "ok", failure: null as string | null, paused: false, fail: mode === "error" };
if (mode === "scanning" || mode === "partial") { scan.scanned = 340; scan.pending = 1240; }
if (mode === "empty") { db = { faces: [], people: [], nextPerson: 1 }; scan.scanned = 0; scan.pending = 0; }
if (folderScope) { scan.enabled = true; scan.installed = true; scan.pending = 1; scan.fw = 1; scan.scanned = 0; scan.photos = 36; }
if (mode === "scanning") db = { faces: [], people: [], nextPerson: 1 };
else if (mode === "review" || mode === "naming" || mode === "off") scan.scanned = total, scan.pending = 0;

// one pass per call site: the size of every group, so "loose" is a lookup (the 5,000-face run asks thousands of times)
let sizes: Map<number, number> | null = null, sizesFor: F[] | null = null, sizesLen = -1;
const groupSize = (c: number) => {
  if (!sizes || sizesFor !== db.faces || sizesLen !== db.faces.length) { sizes = new Map(); for (const g of db.faces) if (g.cluster !== null && g.state === "unreviewed" && g.person === null) sizes.set(g.cluster, (sizes.get(g.cluster) ?? 0) + 1); sizesFor = db.faces; sizesLen = db.faces.length; }
  return sizes.get(c) ?? 0;
};
const isUnknown = (f: F) => f.state === "unreviewed" && f.person === null && f.maybe === null && (f.cluster === null || groupSize(f.cluster) < 2);
const rows = (): PersonRow[] => db.people.map((p) => {
  const mine = db.faces.filter((f) => f.person === p.id && f.state !== "rejected");
  return { id: p.id, name: p.name, state: p.state, faces: mine.length, photos: mine.length, cover_face_id: p.cover, cover_manual: p.manual,
           suggestions: db.faces.filter((f) => f.maybe === p.id && f.person === null && f.state === "unreviewed").length };
});
const faceRow = (f: F): FaceRow => ({ id: f.id, path: f.path, x: 0.2, y: 0.2, w: 0.3, h: 0.3, score: 0.9, frontal: 0.8, person_id: f.person, state: f.state, suggested_person: f.maybe, suggested_sim: f.maybe ? 0.5 : null, cluster_id: f.cluster });
const status = (): FacesStatus => {
  const unrev = db.faces.filter((f) => f.state === "unreviewed" && f.person === null);
  const groups = new Set(unrev.filter((f) => f.cluster !== null).map((f) => f.cluster)).size;
  return { enabled: scan.enabled, installed: scan.installed, eyes_available: false, scanned: scan.scanned, faces_found: db.faces.length, pending: scan.pending, folders_waiting: scan.fw,
    infer_ms: scan.pending && !scan.fw ? 300 : null, health: scan.health, failure: scan.failure, undo: undoStack.length ? "change" : null, offline_folders: scan.offline, recognizer: "sface",
    skipped_network_sources: scan.net,
    totals: { photos: scan.photos ?? db.faces.length, failed_photos: scan.failed, ignored_faces: db.faces.filter((f) => f.state === "rejected").length, ignored_people: db.people.filter((p) => p.state === "ignored").length,
              unknown: db.faces.filter((f) => isUnknown(f)).length, faces: db.faces.filter((f) => f.state !== "rejected").length, people: db.people.filter((p) => p.state === "active").length,
              unreviewed: unrev.length, groups, suggestions: unrev.filter((f) => f.maybe !== null).length } };
};
const svg = (face: number) => {
  const h = (face * 47) % 360;
  return `<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128"><defs><linearGradient id="g" x1="0" y1="0" x2="1" y2="1"><stop offset="0" stop-color="hsl(${h} 40% 52%)"/><stop offset="1" stop-color="hsl(${(h + 50) % 360} 36% 30%)"/></linearGradient></defs><rect width="128" height="128" fill="url(#g)"/><circle cx="64" cy="54" r="24" fill="hsl(${h} 30% 78%)"/><path d="M20 128c4-30 22-42 44-42s40 12 44 42z" fill="hsl(${h} 30% 78%)"/></svg>`;
};
const listeners: { changed: () => void; progress: (p: { scanned: number; pending: number | null; faces_found: number }) => void }[] = [];
const wait = <T,>(v: T) => Promise.resolve(v);

const liveGroups = (): ClusterRow[] => {
  const by = new Map<number, number[]>();
  for (const f of db.faces) if (f.state === "unreviewed" && f.person === null && f.cluster !== null) by.set(f.cluster, [...(by.get(f.cluster) ?? []), f.id]);
  return [...by].filter(([, ids]) => ids.length >= 2).map(([id, ids]): ClusterRow => ({ id, rep_face_id: ids[0], size: ids.length, photos: ids.length, members: ids.slice(0, 4) }));
};
let ghosts: ClusterRow[] = [];
let suggestGroup = -1;   // the group the test wants to look like someone: one that is on screen
const notSamePairs = new Set<string>();   // pairs the user said are not the same
const pairKey = (kind: string, a: number, b: number) => `${kind}:${Math.min(a, b)}:${Math.max(a, b)}`;
const notPairs = new Set<string>();   // group:person pairs the user said are not them
const ghost = (faces: number[]) => { if (q.get("stale") !== "0") ghosts.push(...liveGroups().filter((g) => g.members.some((m) => faces.includes(m)))); };
const fake: PeopleApi = {
  personPhotos: (person, offset, limit) => {
    const all = db.faces.filter((f) => f.person === person && f.state !== "rejected").map((f) => f.path);
    return wait({ paths: all.slice(offset, offset + limit), total: all.length });
  },
  photoFaces: (path) => wait(db.faces.filter((f) => f.path === path && f.state !== "rejected").map((f, i) => ({
    face: f.id, x: 0.1 + 0.22 * i, y: 0.2 + (f.id % 5) * 0.05, w: 0.16, h: 0.28, person: f.person ?? f.maybe, name: db.people.find((p) => p.id === (f.person ?? f.maybe))?.name ?? null,
    state: f.person !== null ? "confirmed" as const : f.maybe !== null ? "suggested" as const : "unassigned" as const, confidence: 0.9,
  }))),
  filterPaths: (f) => {
    const alive = db.faces.filter((x) => x.state !== "rejected");
    const of = (id: number) => new Set(alive.filter((x) => x.person === id).map((x) => x.path));
    const deny = new Set<string>();
    for (const id of f.exclude) of(id).forEach((p) => deny.add(p));
    if (f.none) alive.forEach((x) => deny.add(x.path));
    let allow: Set<string> | null = null;
    const narrow = (s: Set<string>) => { allow = allow === null ? s : new Set([...allow].filter((p) => s.has(p))); };
    if (f.include.length) {
      const sets = f.include.map(of);
      narrow(f.together ? new Set([...sets[0]].filter((p) => sets.every((s) => s.has(p)))) : new Set(sets.flatMap((s) => [...s])));
    }
    if (f.unnamed) narrow(new Set(alive.filter((x) => x.person === null).map((x) => x.path)));
    return wait({ allow: allow ? [...(allow as Set<string>)] : f.none ? null : null, deny: [...deny] });
  },
  status: () => scan.fail ? Promise.reject(new Error("harness: the engine did not answer")) : wait(status()),
  paused: () => wait(scan.paused),
  people: () => wait(rows()),
  groups: () => {
    const live = liveGroups();
    // the real backend regroups when it is idle: until then a group whose faces were all named is still listed (stale=1, the default)
    return wait([...live, ...ghosts.filter((g) => !live.some((l) => l.id === g.id))]);
  },
  list: (filter: FaceFilter, offset, limit) => {
    const pick = (f: F) => filter.kind === "person" ? f.person === filter.id && f.state !== "rejected"
      : filter.kind === "cluster" ? f.cluster === filter.id && f.state === "unreviewed" && f.person === null
      : filter.kind === "suggested" ? f.maybe === filter.id && f.person === null && f.state === "unreviewed"
      : filter.kind === "unknown" ? isUnknown(f)
      : filter.kind === "ignored" ? f.state === "rejected"
      : f.path === filter.path;
    return wait(db.faces.filter(pick).slice(offset, offset + limit).map(faceRow));
  },
  createPerson: (name, faces) => {
    if (db.people.some((p) => p.state === "active" && p.name.toLowerCase() === name.toLowerCase())) return Promise.reject(new Error(`${name} already exists`));
    const id = db.nextPerson;
    ghost(faces);
    mutate(`create:${name}:${faces.length}`, () => { db.nextPerson++; db.people.push({ id, name, state: "active", cover: faces[0] ?? null, manual: false }); for (const f of db.faces) if (faces.includes(f.id)) Object.assign(f, { person: id, state: "confirmed", cluster: null, maybe: null }); });
    return wait(id);
  },
  rename: (id, name) => {
    if (db.people.some((p) => p.id !== id && p.state === "active" && p.name.toLowerCase() === name.toLowerCase())) return Promise.reject(new Error(`${name} already exists`));
    mutate(`rename:${id}:${name}`, () => { db.people.find((p) => p.id === id)!.name = name; });
    return wait(undefined);
  },
  merge: (from, into) => { mutate(`merge:${from}:${into}`, () => { for (const f of db.faces) if (f.person === from) f.person = into; db.people = db.people.filter((p) => p.id !== from); }); return wait(undefined); },
  remove: (id, removed) => { mutate(`remove:${id}:${removed}`, () => { db.people.find((p) => p.id === id)!.state = removed ? "ignored" : "active"; }); return wait(undefined); },
  cover: (id, face) => { mutate(`cover:${id}:${face}`, () => { const p = db.people.find((x) => x.id === id)!; p.cover = face; p.manual = true; }); return wait(undefined); },
  assign: (person, faces) => { ghost(faces); mutate(`assign:${person}:${faces.length}`, () => { for (const f of db.faces) if (faces.includes(f.id)) Object.assign(f, { person, state: "confirmed", cluster: null, maybe: null }); }); return wait(undefined); },
  notPerson: (person, faces) => { for (const f of db.faces) if (faces.includes(f.id) && f.cluster !== null) notPairs.add(`${f.cluster}:${person}`); mutate(`notPerson:${person}:${faces.length}`, () => { for (const f of db.faces) if (faces.includes(f.id) && f.maybe === person) f.maybe = null; }); return wait(undefined); },
  notFace: (faces) => { mutate(`notFace:${faces.length}`, () => { for (const f of db.faces) if (faces.includes(f.id)) f.state = "rejected"; }); return wait(undefined); },
  unignore: (faces) => { mutate(`unignore:${faces.length}`, () => { for (const f of db.faces) if (faces.includes(f.id) && f.state === "rejected") f.state = "unreviewed"; }); return wait(undefined); },
  failedPhotos: (limit) => wait(Array.from({ length: Math.min(limit, scan.failed) }, (_, i) => `/synthetic/broken_${i}.jpg`)),
  mergeSuggestions: () => {
    const g = liveGroups().sort((x, y) => y.size - x.size), ps = db.people.filter((x) => x.state === "active");
    const out: MergeSuggestion[] = [];
    const pick = g.find((x) => x.id === suggestGroup) ?? g[0];
    if (pick && ps[0] && !notPairs.has(`${pick.id}:${ps[0].id}`)) out.push({ kind: "group_person", a: pick.id, b: ps[0].id, similarity: 0.9, confidence: "high" });
    const others = g.filter((x) => x !== pick);
    // the first two groups that were not said to be different people
    let pair: [ClusterRow, ClusterRow] | null = null;
    for (let i = 0; i < Math.min(others.length, 6) && !pair; i++) for (let j = i + 1; j < Math.min(others.length, 6) && !pair; j++) if (!notSamePairs.has(pairKey("group", others[i].id, others[j].id))) pair = [others[i], others[j]];
    if (pair) out.push({ kind: "groups", a: pair[0].id, b: pair[1].id, similarity: 0.8, confidence: "high" });
    if (ps[1] && ps[2] && !notSamePairs.has(pairKey("person", ps[1].id, ps[2].id))) out.push({ kind: "people", a: ps[1].id, b: ps[2].id, similarity: 0.5, confidence: "medium" });
    if (others[2] && others[3]) out.push({ kind: "groups", a: others[2].id, b: others[3].id, similarity: 0.46, confidence: "low" });
    return wait(out);
  },
  stats: () => {
    const t = status().totals;
    return wait({ store: { files: db.faces.length, photos: t.photos, faces: t.faces, people: t.people, ignored_people: t.ignored_people, groups: t.groups, unassigned: t.unknown,
                           rejected: t.ignored_faces, failed: t.failed_photos, db_bytes: 4_200_000 + db.faces.length * 700, models: [] },
                  crops_bytes: db.faces.length * 3000, crops_files: db.faces.length, recognizer: "sface", model_version: "sface-2021dec", pack_installed: scan.installed,
                  eyes_available: false, arcface_available: false });
  },
  settings: () => wait({ enabled: scan.enabled, scope: folderScope ? "folder" : "catalog", strictness: "balanced", recognizer: "sface", detect_size: 1024, arcface_available: false }),
  setRecognizer: () => fake.settings(), setStrictness: () => fake.settings(), setDetectSize: () => fake.settings(), reprocess: () => { calls.push("reprocess"); return wait(db.faces.length); },
  mergeGroups: (from, into) => { mutate(`mergeGroups:${from}:${into}`, () => { for (const f of db.faces) if (f.cluster === from) f.cluster = into; }); return wait(undefined); },
  notSame: (a: Side, b: Side) => { mutate(`notSame:${a.kind}:${a.id}:${b.id}`, () => { notSamePairs.add(pairKey(a.kind, a.id, b.id)); }); return wait(undefined); },
  undo: () => { const prev = undoStack.pop(); calls.push("undo"); if (!prev) return wait(null); db = prev; sizes = null; ghosts = []; return wait("undone"); },
  deleteAll: () => { db = { faces: [], people: [], nextPerson: 1 }; scan.enabled = false; undoStack.length = 0; calls.push("deleteAll"); return wait(undefined); },
  retry: () => { scan.health = "ok"; scan.failure = null; return wait(undefined); },
  scanFolder: (path) => { calls.push(`scanFolder:${path}`); return wait(undefined); },
  setEnabled: (on) => { calls.push(`enabled:${on}`); scan.enabled = on; return wait(undefined); },
  pause: (p) => { scan.paused = p; return wait(undefined); },
  crop: (face) => wait(new Blob([svg(face)], { type: "image/svg+xml" })),
  subscribe: (changed, progress) => { const l = { changed, progress }; listeners.push(l); return () => { listeners.splice(listeners.indexOf(l), 1); }; },
};
setPeopleApi(fake);

const PACK_BASE: PackInfo = {
  id: "faces", name: "Faces", license: "MIT, Apache-2.0", license_url: "", source_host: "example.invalid", bytes: 8_600_000, bytes_on_disk: 0, bytes_left: 8_600_000,
  files: 2, files_ok: 0, state: "notInstalled", checking: false, error: null, version: "1", update_version: null, update_bytes: null, verified_at: null, free_bytes: 5e10,
  unsupported_reason: null, analyses: ["faces"], progress: null, download_consent: false, analysis_consent: false, folder: "/synthetic",
};
const packFor = (m: string): PackInfo => m === "download"
  ? { ...PACK_BASE, state: "downloading", progress: { id: "faces", done: 3_100_000, total: 8_600_000, bytes_per_sec: 2_000_000, eta_s: 3, phase: "download" } }
  : m === "off" ? PACK_BASE : { ...PACK_BASE, state: "installed", bytes_on_disk: 8_600_000, files_ok: 2, verified_at: 1 };

type H = { suggestFor: (g: number) => void; calls: string[]; scan: typeof scan; folderScope: boolean; grow: boolean; fake: PeopleApi; db: () => Db; setPack: (m: string) => void; tick: () => void; changed: () => void; peopleState: typeof peopleState };

function Harness() {
  const [pack, setPack] = useState<PackInfo>(packFor(mode));
  const [width] = useState(Number(q.get("w") ?? 1440));
  useEffect(() => {
    const h: H = {
      suggestFor: (g) => { suggestGroup = g; }, calls, scan, folderScope, grow: q.get("grow") === "1", fake, db: () => db, setPack: (m) => { scan.installed = m === "installed"; setPack(packFor(m)); }, peopleState,
      tick: () => { if (scan.pending) { scan.scanned += 10; scan.pending = Math.max(0, scan.pending - 10); } listeners.forEach((l) => l.progress({ scanned: scan.scanned, pending: scan.pending, faces_found: db.faces.length })); },
      changed: () => listeners.forEach((l) => l.changed()),
    };
    (window as unknown as { __h: H }).__h = h;
    if (folderScope) { settings.set("faces.scope", "folder"); if (q.get("noscopefolder") !== "1") settings.set("faces.scope_folder", "/synthetic/Shoot"); setScanFolderTotal("/synthetic/Shoot", 333); }
    // grow=1: the scan goes on by itself (no event is ever sent: the poll alone must keep the view current), then ends
    let grown: ReturnType<typeof setInterval> | undefined;
    if (q.get("grow") === "1") {
      let n = 0;
      grown = setInterval(() => {
        n++;
        scan.photos = (scan.photos ?? 0) + 20;
        const top = Math.max(0, ...db.faces.map((f) => f.id));
        for (let k = 0; k < 4; k++) db.faces.push({ id: top + 1 + k, path: `/synthetic/g${n}_${k}.jpg`, person: null, state: "unreviewed", cluster: 500 + n, maybe: null });
        if (n >= 6) { scan.fw = 0; scan.pending = 0; scan.photos = 333; clearInterval(grown); }
      }, 1000);
    }
    const stop = startPeople();
    resetPeople({ on: false });
    setPeopleOn(true);
    void refreshAll(true).then(() => {
      document.body.dataset.ready = "1";
      if (mode === "naming") setTimeout(() => {   // the name box open on the first group, as a user's N would
        const grid = document.querySelector<HTMLElement>(".people-scroll");
        grid?.focus();
        grid?.dispatchEvent(new KeyboardEvent("keydown", { key: "n", bubbles: true }));
      }, 200);
      if (q.get("shift") === "1") void runPeopleShift().then((r) => r, (e) => ({ ok: false, error: String(e?.stack ?? e) }))
        .then((r) => { const el = document.getElementById("shift-report")!; el.textContent = JSON.stringify(r); el.dataset.done = "1"; });
    });
    return () => { stop(); clearInterval(grown); };
  }, []);
  return (
    <div style={{ position: "fixed", inset: 0, width, display: "flex" }}>
      <div style={{ position: "relative", flex: 1, display: "flex" }}>
        <div className="people-host">
          <PeopleView onOpenPhoto={(path) => calls.push(`openPhoto:${path}`)} onOpenGrid={(pp) => calls.push(`openGrid:${pp.id}`)} folder={q.get("nofolder") === "1" ? null : "/synthetic/Shoot"} hasCatalog={true} pack={pack} onPack={(a) => { if (a === "download") { calls.push("pack:download"); setPack(packFor("installed")); scan.installed = true; } }} />
        </div>
      </div>
    </div>
  );
}
// ?view=overlay: a photo at 600 x 400 (times the zoom) under the real overlay, beside the Library column's People card
function OverlayStage() {
  const [zoom, setZoom] = useState(1);
  const [show, setShow] = useState(true);
  const [sel, setSel] = useState<number | null>(null);
  const [open, setOpen] = useState(true);
  const path = "/synthetic/Oct 12 - Nani Portraits/_DSC1096.ARW";
  useEffect(() => {
    const h = { suggestFor: () => {}, calls, scan, folderScope, grow: false, fake, db: () => db, setPack: () => {}, tick: () => {}, changed: () => {}, peopleState, setZoom, setShow, photo: path };
    (window as unknown as { __h: unknown }).__h = h;
    scan.enabled = true; scan.installed = true;
    const stop = startPeople();
    resetPeople({ on: false });
    void refreshAll(true).then(() => {
      document.body.dataset.ready = "1";
      if (q.get("shift") === "1") void runOverlayTest().then((r) => r, (e) => ({ ok: false, error: String(e?.stack ?? e) }))
        .then((r) => { const el = document.getElementById("shift-report")!; el.textContent = JSON.stringify(r); el.dataset.done = "1"; });
    });
    return stop;
  }, []);
  return (
    <div className="ov-stage" style={{ display: "flex", width: 1200, height: 760 }}>
      <aside className="left" style={{ width: 300, height: 760 }}>
        <PeopleCard path={path} open={open} onToggle={() => setOpen(!open)} overlay={show} onOverlay={setShow} selected={sel} onSelect={setSel} />
      </aside>
      <div style={{ position: "relative", flex: 1, overflow: "hidden", background: "#222" }}>
        <div style={{ position: "absolute", left: 20, top: 20, width: 560 * zoom, height: 380 * zoom, background: "linear-gradient(135deg,#456,#223)" }}>
          {show && <div className="mask-box" style={{ position: "absolute", inset: 0 }}><FaceOverlay path={path} selected={sel} onSelect={setSel} /></div>}
        </div>
      </div>
    </div>
  );
}
createRoot(document.getElementById("root")!).render(q.get("view") === "overlay" ? <OverlayStage /> : <Harness />);
