// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The pure model of the People view (notes/faces-ux-spec.md): the sections and their tiles, the stable
// order that never moves a tile under the pointer, the selection reducer, the grid geometry and the
// marquee, the keyboard map, the name combobox's ranking, the Undo line's merging and the plain-language
// state of the scan. No React and no commands: a Node test and the harness check all of it.
import type { ClusterRow, FaceRow, FaceTotals, FacesStatus, MergeSuggestion, PersonRow } from "./bindings";

export type Section = "people" | "review" | "unassigned" | "ignored";
export const SECTIONS: Section[] = ["people", "review", "unassigned", "ignored"];

// ---- tiles ----

export type Tile =
  | { key: string; kind: "person"; id: number; name: string; faces: number; photos: number; cover: number | null; review: number; ignored: boolean }
  | { key: string; kind: "group"; id: number; size: number; rep: number; members: number[] }
  | { key: string; kind: "maybe"; person: number; name: string; size: number; rep: number | null }
  | { key: string; kind: "face"; id: number; path: string; maybe: number | null; ignored?: boolean };

export type PeopleData = {
  people: PersonRow[];            // active and ignored
  groups: ClusterRow[];
  unknown: FaceRow[];             // faces with no person and no group (loaded so far)
  unknownMore: boolean;           // more of them than are loaded
  ignoredFaces: FaceRow[];        // faces marked "not a face" (loaded so far)
  ignoredMore: boolean;
  covers?: ReadonlyMap<number, number>;
};

export const personKey = (id: number) => `p:${id}`;
export const groupKey = (id: number) => `g:${id}`;
export const maybeKey = (id: number) => `s:${id}`;
export const faceKey = (id: number) => `f:${id}`;
export const keyId = (key: string) => Number(key.slice(2));

export function personTile(p: PersonRow): Tile {
  return { key: personKey(p.id), kind: "person", id: p.id, name: p.name, faces: p.faces, photos: p.photos, cover: p.cover_face_id,
           review: p.suggestions, ignored: p.state !== "active" };
}

// the tiles of a section in their natural order (most photos first, biggest groups first)
export function sectionTiles(section: Section, d: PeopleData): Tile[] {
  switch (section) {
    case "people":
      return d.people.filter((p) => p.state === "active").sort((a, b) => b.photos - a.photos || a.name.localeCompare(b.name)).map(personTile);
    case "ignored":
      return [...d.people.filter((p) => p.state !== "active").sort((a, b) => a.name.localeCompare(b.name)).map(personTile),
              ...d.ignoredFaces.map((f): Tile => ({ key: faceKey(f.id), kind: "face", id: f.id, path: f.path, maybe: null, ignored: true }))];
    case "review": {
      const maybes: Tile[] = d.people.filter((p) => p.state === "active" && p.suggestions > 0).sort((a, b) => b.suggestions - a.suggestions)
        .map((p) => ({ key: maybeKey(p.id), kind: "maybe" as const, person: p.id, name: p.name, size: p.suggestions, rep: p.cover_face_id }));
      const groups: Tile[] = [...d.groups].sort((a, b) => b.size - a.size || a.id - b.id)
        .map((g) => ({ key: groupKey(g.id), kind: "group" as const, id: g.id, size: g.size, rep: g.rep_face_id, members: g.members?.length ? g.members : [g.rep_face_id] }));
      return [...maybes, ...groups];
    }
    case "unassigned":
      return d.unknown.map((f) => ({ key: faceKey(f.id), kind: "face" as const, id: f.id, path: f.path, maybe: f.suggested_person }));
  }
}

// What the rail counts: the backend's totals when it has them (the grid may hold only the pages loaded so far)
export function sectionCounts(d: PeopleData, totals: FaceTotals | null = null): Record<Section, number> {
  return {
    people: d.people.filter((p) => p.state === "active").length,
    review: sectionTiles("review", d).length,
    unassigned: totals ? totals.unknown : d.unknown.length,
    ignored: totals ? totals.ignored_people + totals.ignored_faces : d.people.filter((p) => p.state !== "active").length + d.ignoredFaces.length,
  };
}

// New data keeps the order the user is looking at: tiles that stay keep their place, those that went are
// dropped, new ones append. The order changes only when the user sorts or leaves the view (`prev` is null).
export function stableOrder(prev: readonly string[] | null, next: readonly Tile[]): Tile[] {
  if (!prev || !prev.length) return [...next];
  const by = new Map(next.map((t) => [t.key, t]));
  const out: Tile[] = [];
  const seen = new Set<string>();
  for (const k of prev) { const t = by.get(k); if (t) { out.push(t); seen.add(k); } }
  for (const t of next) if (!seen.has(t.key)) out.push(t);
  return out;
}

// ---- selection ----

export type Sel = { keys: ReadonlySet<string>; anchor: number | null; active: number };
export const NO_SEL: Sel = { keys: new Set(), anchor: null, active: 0 };

export type SelAction =
  | { type: "click"; index: number }                         // plain click
  | { type: "toggle"; index: number }                        // cmd or ctrl click
  | { type: "range"; index: number }                         // shift click, from the anchor
  | { type: "move"; index: number; extend: boolean }         // the keyboard's active tile (shift extends)
  | { type: "all" }
  | { type: "clear" }
  | { type: "marquee"; indexes: number[]; additive: boolean; base: ReadonlySet<string> };

export function select(s: Sel, a: SelAction, order: readonly string[]): Sel {
  const last = Math.max(0, order.length - 1);
  const clamp = (i: number) => Math.min(Math.max(0, i), last);
  const runKeys = (from: number, to: number) => {
    const [lo, hi] = from <= to ? [from, to] : [to, from];
    return order.slice(lo, hi + 1);
  };
  switch (a.type) {
    case "click": { const i = clamp(a.index); return { keys: new Set(order[i] ? [order[i]] : []), anchor: i, active: i }; }
    case "toggle": {
      const i = clamp(a.index), k = order[i];
      if (!k) return s;
      const keys = new Set(s.keys);
      if (!keys.delete(k)) keys.add(k);
      return { keys, anchor: i, active: i };
    }
    case "range": {
      const i = clamp(a.index), from = s.anchor ?? i;
      return { keys: new Set(runKeys(from, i)), anchor: from, active: i };
    }
    case "move": {
      const i = clamp(a.index);
      if (!a.extend) return { keys: new Set(order[i] ? [order[i]] : []), anchor: i, active: i };
      const from = s.anchor ?? s.active;
      return { keys: new Set(runKeys(from, i)), anchor: from, active: i };
    }
    case "all": return { keys: new Set(order), anchor: s.anchor, active: s.active };
    case "clear": return { keys: new Set(), anchor: null, active: s.active };
    case "marquee": {
      const keys = new Set(a.additive ? a.base : []);
      for (const i of a.indexes) if (order[i]) keys.add(order[i]);
      return { keys, anchor: a.indexes.length ? a.indexes[0] : s.anchor, active: s.active };
    }
  }
}

// selection survives a data change by key; keys that are gone fall out
export function keepSelected(s: Sel, order: readonly string[]): Sel {
  const have = new Set(order);
  const keys = new Set([...s.keys].filter((k) => have.has(k)));
  if (keys.size === s.keys.size) return { ...s, active: Math.min(s.active, Math.max(0, order.length - 1)) };
  return { keys, anchor: null, active: Math.min(s.active, Math.max(0, order.length - 1)) };
}

// ---- grid geometry ----

export const TILE_MIN = 72, TILE_MAX = 160, TILE_DEFAULT = 112;
export const GAP = 12, PAD = 16, CAPTION = 36;   // two caption lines, fixed

export type Geometry = { cols: number; tile: number; rowH: number; pad: number; gap: number };
export function geometry(width: number, tile: number): Geometry {
  const t = Math.min(TILE_MAX, Math.max(TILE_MIN, Math.round(tile)));
  const cols = Math.max(1, Math.floor((width - 2 * PAD + GAP) / (t + GAP)));
  return { cols, tile: t, rowH: t + CAPTION + GAP, pad: PAD, gap: GAP };
}
export const rowsOf = (count: number, g: Geometry) => Math.ceil(count / g.cols);
export const totalHeight = (count: number, g: Geometry) => rowsOf(count, g) * g.rowH + 2 * g.pad;
export const tilePos = (index: number, g: Geometry) => ({
  x: g.pad + (index % g.cols) * (g.tile + g.gap), y: g.pad + Math.floor(index / g.cols) * g.rowH,
});

// the tiles to mount: the rows in view and two each side
export function visibleRange(scrollTop: number, viewH: number, count: number, g: Geometry, over = 2): { from: number; to: number } {
  const rows = rowsOf(count, g);
  const first = Math.max(0, Math.floor((scrollTop - g.pad) / g.rowH) - over);
  const last = Math.min(rows - 1, Math.floor((scrollTop + viewH - g.pad) / g.rowH) + over);
  if (last < first) return { from: 0, to: 0 };
  return { from: first * g.cols, to: Math.min(count, (last + 1) * g.cols) };
}

// the tiles a rectangle (in the scroll content's coordinates) touches; computed, not hit-tested, so
// tiles that are not mounted count
export function tilesInRect(r: { x: number; y: number; w: number; h: number }, count: number, g: Geometry): number[] {
  const x0 = r.x, x1 = r.x + r.w, y0 = r.y, y1 = r.y + r.h;
  const colA = Math.max(0, Math.floor((x0 - g.pad) / (g.tile + g.gap))), colB = Math.min(g.cols - 1, Math.floor((x1 - g.pad) / (g.tile + g.gap)));
  const rowA = Math.max(0, Math.floor((y0 - g.pad) / g.rowH)), rowB = Math.floor((y1 - g.pad) / g.rowH);
  const out: number[] = [];
  for (let row = rowA; row <= rowB; row++) {
    for (let col = colA; col <= colB; col++) {
      const i = row * g.cols + col;
      if (i >= count) break;
      const p = tilePos(i, g);
      if (p.x < x1 && p.x + g.tile > x0 && p.y < y1 && p.y + g.tile + CAPTION > y0) out.push(i);
    }
  }
  return out;
}

// ---- keyboard ----

export type KeyIn = { key: string; shift: boolean; meta: boolean; alt: boolean };
export type Intent =
  | { kind: "move"; index: number; extend: boolean }
  | { kind: "name"; seed: string }
  | { kind: "open" } | { kind: "all" } | { kind: "clear" } | { kind: "rename" }
  | { kind: "merge" } | { kind: "ignore" } | { kind: "reject" } | { kind: "confirm" } | { kind: "cover" } | { kind: "undo" };
export type KeyCtx = { section: Section; open: boolean; count: number; cols: number; active: number; dir: "ltr" | "rtl" };

// What a key does in the grid. In the sections that hold unnamed faces any letter opens the name box
// with that letter in it (the quickest path: arrows, type "Dad", Enter); in People the letters are the
// shortcuts M (merge), I (ignore) and C (cover), as the spec lists them.
export function gridKey(e: KeyIn, c: KeyCtx): Intent | null {
  if (c.count === 0 && e.key !== "Escape") return null;
  const step = (d: number) => ({ kind: "move" as const, index: Math.min(Math.max(0, c.active + d), c.count - 1), extend: e.shift });
  const fwd = c.dir === "rtl" ? -1 : 1;
  const cmd = e.meta;
  if (cmd) {
    if (e.key === "a" || e.key === "A") return { kind: "all" };
    if (e.key === "z" || e.key === "Z") return { kind: "undo" };
    return null;
  }
  if (e.alt) return null;
  switch (e.key) {
    case "ArrowRight": return step(fwd);
    case "ArrowLeft": return step(-fwd);
    case "ArrowDown": return step(c.cols);
    case "ArrowUp": return step(-c.cols);
    case "Home": return { kind: "move", index: 0, extend: e.shift };
    case "End": return { kind: "move", index: c.count - 1, extend: e.shift };
    case "PageDown": return step(c.cols * 4);
    case "PageUp": return step(-c.cols * 4);
    case "Escape": return { kind: "clear" };
    case "Enter": return { kind: "open" };
    case "F2": return c.section === "people" ? { kind: "rename" } : null;
    case "Backspace": case "Delete": return c.section === "people" ? null : { kind: "reject" };
    case " ": return null;
  }
  const naming = c.section === "review" || c.section === "unassigned" || c.open;
  if (naming && (e.key === "n" || e.key === "N")) return { kind: "name", seed: "" };
  if (naming && e.key.length === 1 && /\p{L}/u.test(e.key)) return { kind: "name", seed: e.key };
  if (c.section === "people") {
    if (e.key === "m" || e.key === "M") return { kind: "merge" };
    if (e.key === "i" || e.key === "I") return { kind: "ignore" };
    if (e.key === "c" || e.key === "C") return { kind: "cover" };
  }
  if (c.section === "ignored" && (e.key === "r" || e.key === "R")) return { kind: "ignore" };
  return null;
}

// ---- the name combobox ----

export type NameOption = { kind: "person"; id: number; name: string; photos: number } | { kind: "create"; name: string };

const norm = (s: string) => s.normalize("NFKD").replace(/\p{M}/gu, "").trim().toLowerCase();

// existing people that match what is typed, best first (prefix, then contains, then the most photos),
// and a Create row last when the name is not already taken (names are unique, case-insensitively)
export function nameOptions(query: string, people: readonly PersonRow[], limit = 8): NameOption[] {
  const q = norm(query);
  const active = people.filter((p) => p.state === "active");
  const scored = active.map((p) => {
    const n = norm(p.name);
    const rank = !q ? 2 : n === q ? 0 : n.startsWith(q) ? 1 : n.split(/\s+/).some((w) => w.startsWith(q)) ? 2 : n.includes(q) ? 3 : 9;
    return { p, rank };
  }).filter((x) => x.rank < 9).sort((a, b) => a.rank - b.rank || b.p.photos - a.p.photos || a.p.name.localeCompare(b.p.name));
  const out: NameOption[] = scored.slice(0, limit).map(({ p }) => ({ kind: "person", id: p.id, name: p.name, photos: p.photos }));
  const trimmed = query.trim();
  if (trimmed && !active.some((p) => norm(p.name) === q)) out.push({ kind: "create", name: trimmed });
  return out;
}

// ---- the Undo line ----

export type UndoKind = "joined" | "notSame" | "unignored" | "assigned" | "confirmed" | "ignored" | "removed" | "merged" | "renamed" | "cover" | "restored" | "notPerson" | "notFace";
export type UndoEntry = { kind: UndoKind; n: number; name: string; other: string; steps: number };

// a second action of the same kind and name while the line is up joins it ("Ignored 3 faces"); each
// action is one step of faces_undo, and Undo takes them all back
export function mergeUndo(prev: UndoEntry | null, next: UndoEntry): UndoEntry {
  if (!prev || prev.kind !== next.kind || prev.name !== next.name || prev.other !== next.other
      || next.kind === "merged" || next.kind === "renamed" || next.kind === "cover") return next;
  return { ...next, n: prev.n + next.n, steps: prev.steps + next.steps };
}

// how long the Undo line stays: a change to many faces (a whole group) gets longer
export const undoMs = (n: number) => (n > 20 ? 8000 : 5000);

// ---- the scan, in plain words ----

export type ScanKind = "off" | "idle" | "scanning" | "paused" | "waiting" | "blocked" | "retrying";
export type ScanView = { kind: ScanKind; done: number; total: number | null; left: number | null; reason: string | null };

// Where the scan's numbers come from. In the Catalog scope the backend counts the photos still to scan (`pending`).
// In the folder scope it counts only the folders waiting, never photos, so the total is the open folder's own photo
// count (`folderTotal`, null when that folder is not the one scanned) and "done" is the photos the store holds.
export type ScanCtx = { folderScope: boolean; folderTotal: number | null };
export const NO_CTX: ScanCtx = { folderScope: false, folderTotal: null };

// from faces_status and the shared pause. `left` is the estimate in seconds, only once a photo has been timed
export function scanView(s: Pick<FacesStatus, "enabled" | "installed" | "scanned" | "pending" | "infer_ms" | "health" | "failure" | "folders_waiting"> & { totals?: Pick<FaceTotals, "photos"> },
                         paused: boolean, ctx: ScanCtx = NO_CTX): ScanView {
  const folder = ctx.folderScope;
  const stored = s.totals?.photos ?? 0;
  const done = folder ? Math.max(s.scanned, ctx.folderTotal === null ? stored : Math.min(stored, ctx.folderTotal)) : s.scanned;
  const pend = folder ? (ctx.folderTotal === null ? null : Math.max(0, ctx.folderTotal - done)) : s.pending;
  const total = folder ? ctx.folderTotal : pend === null ? null : done + pend;
  const left = pend !== null && pend > 0 && s.infer_ms ? Math.max(1, Math.round((pend * s.infer_ms) / 1000)) : null;
  const base = { done, total, left, reason: s.failure };
  if (!s.enabled || !s.installed) return { ...base, kind: "off" };
  if (s.health === "blocked") return { ...base, kind: "blocked" };
  if (s.health === "retrying") return { ...base, kind: "retrying" };
  const busy = folder ? s.folders_waiting > 0 && (total === null || done < total) : (pend ?? 0) > 0 || s.folders_waiting > 0;
  if (busy && paused) return { ...base, kind: "paused" };
  if (busy) return { ...base, kind: "scanning" };
  if (pend === null && done === 0) return { ...base, kind: "waiting" };
  return { ...base, kind: "idle" };
}

// the activity line and the strip say the same thing: both read this (total 0 means no count yet)
export function scanJob(v: ScanView): { done: number; total: number; paused: boolean } | null {
  return v.kind === "scanning" || v.kind === "paused" ? { done: v.done, total: v.total ?? 0, paused: v.kind === "paused" } : null;
}

export type ViewKind = "consent" | "loading" | "error" | "scanning" | "none" | "ready";

// what the centre shows: the consent card until People is on, a skeleton while the first faces are looked for
export function viewKind(s: FacesStatus | null, failed: boolean, scan: ScanView, loaded: boolean): ViewKind {
  if (!s) return failed ? "error" : "loading";
  if (!s.enabled || !s.installed) return "consent";
  if (s.totals.faces === 0) return scan.kind === "scanning" || scan.kind === "waiting" || scan.kind === "paused" ? "scanning" : loaded ? "none" : "loading";
  return "ready";
}

// the dot on the People tab: faces that need a look
export function needsReview(s: FacesStatus | null): boolean {
  return !!s && s.enabled && s.installed && (s.totals.groups > 0 || s.totals.suggestions > 0);
}

// the quiet lines beside the scan: photos that could not be read, folders that do not answer, network sources left out
export type ScanNotes = { failed: number; offline: number; network: number };
export function scanNotes(s: FacesStatus | null): ScanNotes {
  if (!s) return { failed: 0, offline: 0, network: 0 };
  return { failed: s.totals.failed_photos ?? 0, offline: s.offline_folders ?? 0, network: s.skipped_network_sources ?? 0 };
}

// ---- suggestions: groups that may be one person, a group that looks like someone named, two people who may be one ----

export type SuggestKind = "groups" | "group_person" | "people";
export type SuggestRow = { key: string; kind: SuggestKind; a: number; b: number; confidence: "high" | "medium"; similarity: number | null };

// The backend's pairs, kept to what the page still shows: both sides must still be there (a group that was just
// named is gone), "low" confidence is left out as noise, a pair the user dismissed stays hidden, and the strongest
// come first (high before medium, then by similarity). Only ever a suggestion: nothing here changes data.
export function suggestRows(list: readonly MergeSuggestion[], d: PeopleData, dismissed: ReadonlySet<string> = new Set()): SuggestRow[] {
  const groups = new Set(d.groups.map((g) => g.id));
  const active = new Set(d.people.filter((p) => p.state === "active").map((p) => p.id));
  const seen = new Set<string>();
  const out: SuggestRow[] = [];
  for (const s of list) {
    const kind = s.kind as SuggestKind;
    if (kind !== "groups" && kind !== "group_person" && kind !== "people") continue;
    if (s.confidence !== "high" && s.confidence !== "medium") continue;
    const ok = kind === "groups" ? groups.has(s.a) && groups.has(s.b) && s.a !== s.b
      : kind === "group_person" ? groups.has(s.a) && active.has(s.b) : active.has(s.a) && active.has(s.b) && s.a !== s.b;
    const key = `${kind}:${s.a}:${s.b}`;
    if (!ok || dismissed.has(key) || seen.has(key)) continue;
    seen.add(key);
    out.push({ key, kind, a: s.a, b: s.b, confidence: s.confidence, similarity: s.similarity });
  }
  return out.map((r, i) => ({ r, i })).sort((x, y) => (x.r.confidence === y.r.confidence ? 0 : x.r.confidence === "high" ? -1 : 1)
    || (y.r.similarity ?? 0) - (x.r.similarity ?? 0) || x.i - y.i).map((x) => x.r);
}

// the first `n` and how many more there are ("Show N more")
export function capRows<T>(rows: readonly T[], n = 3): { shown: T[]; more: number } {
  return { shown: rows.slice(0, n), more: Math.max(0, rows.length - n) };
}

// the person each group looks like (its best "group_person" pair), for the chip on the group's tile
export function lookAlikes(rows: readonly SuggestRow[]): Map<string, number> {
  const out = new Map<string, number>();
  for (const r of rows) if (r.kind === "group_person" && !out.has(groupKey(r.a))) out.set(groupKey(r.a), r.b);
  return out;
}

// two or more groups made one: the biggest takes the others in, so the group the user knows stays where it is
export function joinPlan(ids: readonly number[], sizes: ReadonlyMap<number, number>): { into: number; from: number[] } | null {
  const sorted = [...new Set(ids)].sort((a, b) => (sizes.get(b) ?? 0) - (sizes.get(a) ?? 0) || a - b);
  return sorted.length < 2 ? null : { into: sorted[0], from: sorted.slice(1) };
}
