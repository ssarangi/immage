// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Scenes mode, the pure part: the timeline model built from Rust's grouping, what a
// frame, scene and block are (state, best frame, counts), the virtual list's row
// heights, the keyboard map and the camera line. No React, no i18n and no commands,
// so verify/scenes.test.mjs runs it under Node. Rust (scenes.rs) cuts scenes and
// blocks; everything here is derived from those cuts, the library's flags and the
// scores, so a rating or a score arriving never regroups anything.
import type { CullBlurRegion, Force, RuleScene, Rules, ScenesBuild, Verdict } from "./bindings";

// ---- layout constants: every height is known before a thumbnail loads ----
export const CARD_W = 220;
export const CARD_THUMB_H = 165;
export const CARD_H = 229;          // the thumbnail box and the two text rows under it
export const GAP = 12;
export const ROW_H = CARD_H + GAP;  // one row of cards
export const DAY_H = 40;            // a day header
export const BLOCK_H = 36;          // a time block's header
export const RAIL_W = 40;           // the rail's gutter at the inline start
export const PAD = 16;              // the list's padding at both sides

export type SceneInfo = { n: number; block: number; frames: number[]; unhashed: number };
export type BlockInfo = {
  index: number; startMs: number | null; endMs: number | null; noTime: boolean; approx: boolean;
  first: number;    // index of its first scene in SceneModel.scenes
  count: number;    // scenes
  frames: number;
  day: string;      // "YYYY-MM-DD" of its start, "" with no time
};
export type SceneModel = { blocks: BlockInfo[]; scenes: SceneInfo[]; frameCount: number };

const pad = (n: number) => String(n).padStart(2, "0");
// the camera's clock as Rust sends it is "as if UTC", so the UTC fields are the wall-clock fields
export function dayOf(ms: number): string {
  const d = new Date(ms);
  return `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}-${pad(d.getUTCDate())}`;
}

export function buildModel(build: Pick<ScenesBuild, "blocks">): SceneModel {
  const scenes: SceneInfo[] = [];
  const blocks: BlockInfo[] = [];
  let frameCount = 0;
  build.blocks.forEach((b, index) => {
    const first = scenes.length;
    let frames = 0;
    for (const s of b.scenes) {
      scenes.push({ n: scenes.length + 1, block: index, frames: s.frames, unhashed: s.unhashed });
      frames += s.frames.length;
    }
    frameCount += frames;
    blocks.push({
      index, startMs: b.startMs, endMs: b.endMs, noTime: b.noTime, approx: b.approx, first, count: b.scenes.length, frames,
      day: b.startMs === null ? "" : dayOf(b.startMs),
    });
  });
  return { blocks, scenes, frameCount };
}

// ---- the page's list of photos, for the request ----
type ExifLike = { datetime?: string; file_datetime?: string; capture_time_raw?: string };
export type SceneSource = { path: string; filename: string; version?: number; exif: ExifLike; remote?: unknown };

// the capture time Rust should read: the raw text keeps the camera's fraction of a
// second, but only when it is of the same second as `datetime` (a sidecar may have
// changed that); else the file's modified time, marked so the card can say so
export function captureText(exif: ExifLike): { takenAt: string; fromFile: boolean } {
  const dt = exif.datetime ?? "";
  const raw = exif.capture_time_raw ?? "";
  if (dt) return { takenAt: raw.length >= 19 && raw.slice(0, 19) === dt.slice(0, 19) ? raw : dt, fromFile: false };
  if (exif.file_datetime) return { takenAt: exif.file_datetime, fromFile: true };
  return { takenAt: "", fromFile: false };
}

export function requestItems(entries: readonly SceneSource[]) {
  return entries.map((e) => ({ path: e.path, version: e.version ?? 0, name: e.filename, ...captureText(e.exif) }));
}

export const frameKey = (path: string, version?: number) => `${version ?? 0}:${path}`;

// the list's identity: when it changes the timeline is built again. a rating or a
// label changes entries but not this
export function listSignature(entries: readonly SceneSource[]): string {
  let h = 0x811c9dc5;
  for (const e of entries) {
    const s = `${e.path}\u0000${e.version ?? 0}\u0000${e.exif.datetime ?? ""}\u0000${e.exif.file_datetime ?? ""}`;
    for (let i = 0; i < s.length; i++) h = Math.imul(h ^ s.charCodeAt(i), 16777619);
  }
  return `${entries.length}:${(h >>> 0).toString(16)}`;
}

// ---- frame, scene and block state ----
export const GREEN = 4;   // the green colour label's bit: what Accept writes
export type FrameState = "accepted" | "rejected" | "undecided";
export type SceneState = FrameState | "mixed";
export type Flags = { path: string; version?: number; rejected: boolean; labels: number; rating: number };

// Rejected is the library's flag. Accepted needs both the green label and Scenes'
// own memory of writing it, so a label the user chose by hand is not an accept
export function frameState(e: Flags, accepted: ReadonlySet<string>): FrameState {
  if (e.rejected) return "rejected";
  return accepted.has(frameKey(e.path, e.version)) && (e.labels & GREEN) !== 0 ? "accepted" : "undecided";
}

export type Ctx<E extends Flags = Flags> = {
  entries: ReadonlyArray<E | undefined>;      // by request index
  quality: ReadonlyArray<number | null>;          // 0-1, by request index
  accepted: ReadonlySet<string>;
};

export type SceneStats = {
  accepted: number; rejected: number; undecided: number; scored: number; frames: number;
  /** the thumbnail frame: the best one once every frame is scored, else the first in time */
  shown: number;
  /** the best frame, only once every frame is scored (no star until then) */
  best: number | null;
  bestQuality: number | null;
};

export function sceneStats<E extends Flags>(scene: Pick<SceneInfo, "frames">, ctx: Ctx<E>): SceneStats {
  let accepted = 0, rejected = 0, undecided = 0, scored = 0, best: number | null = null, bestQ = -1;
  for (const f of scene.frames) {
    const e = ctx.entries[f];
    const s = e ? frameState(e, ctx.accepted) : "undecided";
    if (s === "accepted") accepted++; else if (s === "rejected") rejected++; else undecided++;
    const q = ctx.quality[f];
    if (q !== null && q !== undefined) { scored++; if (q > bestQ) { bestQ = q; best = f; } }
  }
  const complete = scored === scene.frames.length && scene.frames.length > 0;
  return {
    accepted, rejected, undecided, scored, frames: scene.frames.length,
    shown: complete && best !== null ? best : scene.frames[0], best: complete ? best : null,
    bestQuality: complete ? bestQ : null,
  };
}

export function sceneState(s: Pick<SceneStats, "accepted" | "rejected" | "undecided" | "frames">): SceneState {
  if (s.frames === 0) return "undecided";
  if (s.accepted === s.frames) return "accepted";
  if (s.rejected === s.frames) return "rejected";
  if (s.undecided === s.frames) return "undecided";
  return "mixed";
}

export const scoreText = (q: number | null) => (q === null ? null : Math.round(q * 100));

export function blockStats<E extends Flags>(model: SceneModel, block: BlockInfo, ctx: Ctx<E>) {
  const out = { accepted: 0, rejected: 0, undecided: 0 };
  for (let i = block.first; i < block.first + block.count; i++) {
    const s = sceneStats(model.scenes[i], ctx);
    out.accepted += s.accepted; out.rejected += s.rejected; out.undecided += s.undecided;
  }
  return out;
}

// the frames of a scene in the order the filmstrip shows them: the sharpest first
// when every frame is scored (ties keep capture order), else capture order
export function filmstripOrder(frames: readonly number[], quality: ReadonlyArray<number | null>, bySharpness: boolean): number[] {
  if (!bySharpness || frames.some((f) => quality[f] === null || quality[f] === undefined)) return [...frames];
  return [...frames].map((f, i) => ({ f, i })).sort((a, b) => (quality[b.f]! - quality[a.f]!) || a.i - b.i).map((x) => x.f);
}

// the library change a decision makes, and whether Scenes now remembers having accepted
export type DecisionPatch = { fields: { rejected?: boolean; labels?: number; rating?: number }; accepted: boolean };
export function decisionPatch(state: FrameState, e: Flags, accepted: ReadonlySet<string>, acceptGivesStar: boolean): DecisionPatch {
  const ours = accepted.has(frameKey(e.path, e.version));
  if (state === "accepted") {
    return { fields: { rejected: false, labels: e.labels | GREEN, ...(acceptGivesStar && e.rating < 1 ? { rating: 1 } : {}) }, accepted: true };
  }
  // an accept's green label goes with it; one the user chose by hand stays
  const labels = ours ? e.labels & ~GREEN : e.labels;
  return state === "rejected"
    ? { fields: { rejected: true, labels }, accepted: false }
    : { fields: { rejected: false, labels }, accepted: false };
}

// ---- the virtual list: rows with heights known from the counts alone ----
export type Row =
  | { kind: "day"; top: number; height: number; block: number }
  | { kind: "block"; top: number; height: number; block: number }
  | { kind: "cards"; top: number; height: number; block: number; first: number; count: number };

export type Layout = { rows: Row[]; total: number; columns: number; visible: number[] };

export const columnsFor = (width: number) => Math.max(1, Math.floor((width - RAIL_W - PAD * 2 + GAP) / (CARD_W + GAP)));

export function visibleSceneIndexes(model: SceneModel, block: BlockInfo, hideSingles: boolean): number[] {
  const out: number[] = [];
  for (let i = block.first; i < block.first + block.count; i++) if (!hideSingles || model.scenes[i].frames.length > 1) out.push(i);
  return out;
}

export function layout(model: SceneModel, width: number, hideSingles: boolean, collapsed: ReadonlySet<number>): Layout {
  const columns = columnsFor(width);
  const rows: Row[] = [];
  const visible: number[] = [];
  let top = 0, lastDay: string | null = null;
  for (const block of model.blocks) {
    if (block.day !== lastDay) {
      rows.push({ kind: "day", top, height: DAY_H, block: block.index }); top += DAY_H; lastDay = block.day;
    }
    rows.push({ kind: "block", top, height: BLOCK_H, block: block.index }); top += BLOCK_H;
    if (collapsed.has(block.index)) { top += GAP; continue; }
    const shown = visibleSceneIndexes(model, block, hideSingles);
    for (let k = 0; k < shown.length; k += columns) {
      const count = Math.min(columns, shown.length - k);
      rows.push({ kind: "cards", top, height: ROW_H, block: block.index, first: k, count });
      top += ROW_H;
    }
    if (!shown.length) top += GAP;
    visible.push(...shown);
  }
  return { rows, total: top, columns, visible };
}

// the rows to mount for a viewport: a binary search over the tops, plus overscan rows
export function windowRows(rows: readonly Row[], scrollTop: number, viewport: number, overscan = 2): [number, number] {
  if (!rows.length) return [0, 0];
  let lo = 0, hi = rows.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (rows[mid].top <= scrollTop) lo = mid; else hi = mid - 1;
  }
  let end = lo;
  while (end < rows.length && rows[end].top < scrollTop + viewport) end++;
  return [Math.max(0, lo - overscan), Math.min(rows.length, end + overscan)];
}

// the row a scene is on: for scrolling to it (its block's header when the block is folded)
export function rowOfScene(lay: Layout, model: SceneModel, scene: number, hideSingles: boolean): Row | undefined {
  const info = model.scenes[scene];
  if (!info) return undefined;
  const block = model.blocks[info.block];
  const at = visibleSceneIndexes(model, block, hideSingles).indexOf(scene);
  const cards = lay.rows.filter((r) => r.kind === "cards" && r.block === block.index);
  return (at >= 0 ? cards[Math.floor(at / lay.columns)] : undefined) ?? lay.rows.find((r) => r.kind === "block" && r.block === block.index);
}

// ---- keyboard ----
export type TimelineAction =
  | { type: "move"; to: number }
  | { type: "open" }
  | { type: "decide"; state: FrameState; whole: boolean }
  | { type: "fold"; fold: boolean }
  | { type: "back" }
  | { type: "none" };

// the scene a navigation key lands on, among the scenes showing (`visible`, in order,
// with `starts` the position of each block's first). Rows wrap across blocks
export function moveFocus(visible: readonly number[], model: SceneModel, from: number, key: string, columns: number, rtl: boolean): number {
  if (!visible.length) return from;
  let at = visible.indexOf(from);
  if (at < 0) at = 0;
  const block = model.scenes[visible[at]].block;
  const inBlock = visible.filter((s) => model.scenes[s].block === block);
  const start = visible.indexOf(inBlock[0]);
  const offset = at - start;
  const blocks = [...new Set(visible.map((s) => model.scenes[s].block))];
  const bi = blocks.indexOf(block);
  const firstOf = (b: number) => visible.find((s) => model.scenes[s].block === b)!;
  const countOf = (b: number) => visible.filter((s) => model.scenes[s].block === b).length;
  const lastRowStart = (n: number) => Math.floor((n - 1) / columns) * columns;
  const column = offset % columns;
  let to = at;
  const prev = rtl ? "ArrowRight" : "ArrowLeft", next = rtl ? "ArrowLeft" : "ArrowRight";
  if (key === prev) to = Math.max(0, at - 1);
  else if (key === next) to = Math.min(visible.length - 1, at + 1);
  else if (key === "ArrowUp") {
    if (offset >= columns) to = at - columns;
    else if (bi > 0) {
      const pb = blocks[bi - 1], n = countOf(pb), base = visible.indexOf(firstOf(pb));
      to = base + Math.min(n - 1, lastRowStart(n) + column);
    }
  } else if (key === "ArrowDown") {
    const lastRow = Math.floor((inBlock.length - 1) / columns);
    if (offset + columns < inBlock.length) to = at + columns;
    else if (Math.floor(offset / columns) < lastRow) to = start + inBlock.length - 1;
    else if (bi < blocks.length - 1) to = visible.indexOf(firstOf(blocks[bi + 1])) + Math.min(column, countOf(blocks[bi + 1]) - 1);
  } else if (key === "Home") to = 0;
  else if (key === "End") to = visible.length - 1;
  else if (key === "PageUp") to = visible.indexOf(firstOf(blocks[Math.max(0, offset > 0 ? bi : bi - 1)]));
  else if (key === "PageDown") to = visible.indexOf(firstOf(blocks[Math.min(blocks.length - 1, bi + 1)]));
  return visible[Math.max(0, Math.min(visible.length - 1, to))];
}

const MOVES = new Set(["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End", "PageUp", "PageDown"]);

// What a key does on the timeline: `onHeader` is a block header having the focus
export function timelineKey(key: string, shift: boolean, onHeader: boolean, rtl: boolean): TimelineAction {
  if (onHeader && (key === "ArrowLeft" || key === "ArrowRight")) return { type: "fold", fold: (key === "ArrowLeft") !== rtl };
  if (MOVES.has(key)) return { type: "move", to: 0 };
  if (key === "Enter" || key === " ") return { type: "open" };
  if (key === "Escape") return { type: "back" };
  const k = key.toLowerCase();
  if (k === "a") return { type: "decide", state: "accepted", whole: shift };
  if (k === "x") return { type: "decide", state: "rejected", whole: shift };
  if (k === "u") return { type: "decide", state: "undecided", whole: shift };
  return { type: "none" };
}

// ---- the camera line ----
export type CameraBits = { model?: string; lens?: string; focal_length?: number; aperture?: number; exposure?: number; iso?: number };
const trim = (n: number, digits: number) => String(Number(n.toFixed(digits)));

export const focalText = (mm?: number) => (mm && mm > 0 ? `${trim(mm, mm < 10 ? 1 : 0)} mm` : "");
export const apertureText = (f?: number) => (f && f > 0 ? `f/${trim(f, 1)}` : "");
export const isoText = (iso?: number) => (iso && iso > 0 ? `ISO ${Math.round(iso)}` : "");
export function shutterText(sec?: number): string {
  if (!sec || sec <= 0) return "";
  if (sec >= 0.5) return `${trim(sec, sec >= 10 ? 0 : 1)} s`;
  return `1/${Math.round(1 / sec)}`;
}

// the card's line collapses by priority as the card narrows: model, then focal and
// aperture and shutter, then ISO, then lens. `room` is how many parts fit
export function cameraLine(c: CameraBits, room = 4): string[] {
  const exposure = [focalText(c.focal_length), apertureText(c.aperture), shutterText(c.exposure)].filter(Boolean).join(" ");
  const parts = [c.model?.trim() ?? "", exposure, isoText(c.iso), c.lens?.trim() ?? ""].filter(Boolean);
  return parts.slice(0, Math.max(1, room));
}

// ---- the sharpest region (P2) ----
// the region clamped to the frame (fractions of the image), so a bad value never draws outside it; null when
// there is none or it has no area (a number may come as null, as every f64 does)
export function clampRegion(r: CullBlurRegion | null | undefined): { x: number; y: number; w: number; h: number } | null {
  if (!r) return null;
  const x = Math.max(0, Math.min(1, r.x ?? 0)), y = Math.max(0, Math.min(1, r.y ?? 0));
  const w = Math.min(r.width ?? 0, 1 - x), h = Math.min(r.height ?? 0, 1 - y);
  return w > 0 && h > 0 ? { x, y, w, h } : null;
}

// the crop's place over the stage photo, in percent
export function regionStyle(r: CullBlurRegion | null | undefined): { left: string; top: string; width: string; height: string } | null {
  const c = clampRegion(r);
  if (!c) return null;
  const pct = (v: number) => `${Math.round(v * 10000) / 100}%`;
  return { left: pct(c.x), top: pct(c.y), width: pct(c.w), height: pct(c.h) };
}

// the point of the frame Z centres the 100% view on (fractions of the image), or null with no region
export function regionCentre(r: CullBlurRegion | null | undefined): { ix: number; iy: number } | null {
  const c = clampRegion(r);
  return c ? { ix: c.x + c.w / 2, iy: c.y + c.h / 2 } : null;
}

// ---- split and merge (P2) ----
// where a frame stands in its scene: Split starts a new scene at it, so it cannot be the first; Merge joins the
// scene to the next one of its block, so the last scene of a block has nothing to join
export function canSplit(model: SceneModel, scene: number, frame: number): boolean {
  const info = model.scenes[scene];
  return !!info && info.frames.indexOf(frame) > 0;
}
export function canMerge(model: SceneModel, scene: number): boolean {
  const info = model.scenes[scene], next = model.scenes[scene + 1];
  return !!info && !!next && next.block === info.block;
}
// the scene holding a frame (by request index), after a rebuild
export const sceneOfFrame = (model: SceneModel, frame: number) => model.scenes.findIndex((s) => s.frames.includes(frame));

// ---- the keep rules (P2) ----
export const DEFAULT_RULES: Rules = { keepBest: 2, minQuality: 0, alwaysKeep: true, ratedAtLeast: 3, singlesKeep: false };
export const clampRules = (r: Rules): Rules => ({
  keepBest: Math.max(0, Math.min(9, Math.round(r.keepBest) || 0)),
  minQuality: Math.max(0, Math.min(100, Math.round(r.minQuality) || 0)),
  alwaysKeep: !!r.alwaysKeep,
  ratedAtLeast: Math.max(1, Math.min(5, Math.round(r.ratedAtLeast) || 1)),
  singlesKeep: !!r.singlesKeep,
});

export type Pile = "keep" | "reject" | "undecided";
export const pileOf = (v: Verdict): Pile => v.startsWith("keep") ? "keep" : v.startsWith("reject") ? "reject" : "undecided";
export type Forced = ReadonlyMap<string, Force>;

// what the page tells Rust about each scene's frames: the library's rating and flag, Scenes' accepts, the
// scores, and the piles review's hand-made calls
export function rulesScenes<E extends Flags>(model: SceneModel, items: ReadonlyArray<{ path: string; version: number }>, ctx: Ctx<E>, forced: Forced): RuleScene[] {
  return model.scenes.map((scene) => ({
    frames: scene.frames.map((f) => {
      const it = items[f], e = ctx.entries[f];
      const key = frameKey(it.path, it.version);
      return {
        path: it.path, version: it.version, quality: ctx.quality[f] ?? null, rating: e?.rating ?? 0, rejected: !!e?.rejected,
        accepted: ctx.accepted.has(key), force: forced.get(key) ?? null,
      };
    }),
  }));
}

// the scenes with something to review: at least one frame the rules keep or reject
export function pileScenes(verdicts: ReadonlyArray<ReadonlyArray<Verdict>>): number[] {
  const out: number[] = [];
  verdicts.forEach((v, i) => { if (v.some((x) => pileOf(x) !== "undecided")) out.push(i); });
  return out;
}

// the piles review's rows are one fixed height, so the list's size follows from the count alone
export const PILE_ROW_H = 120;
export function pileWindow(count: number, scrollTop: number, viewport: number, overscan = 2): [number, number] {
  const first = Math.min(count, Math.max(0, Math.floor(scrollTop / PILE_ROW_H) - overscan));
  const last = Math.min(count, Math.ceil((scrollTop + viewport) / PILE_ROW_H) + overscan);
  return [first, Math.max(first, last)];
}
