// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Scenes timeline of the open folder, shared by the grid (which shows it in
// place of the thumbnails), the lightbox's Scene tab and the toolbar. A small
// external store, as cullStore.ts is. It asks Rust for the grouping (scenes_build),
// keeps the scores that come back, scores a scene's frames on request and, when
// Preferences allows, the rest in the background. Everything derived from flags and
// scores is scenes.ts's, so none of it regroups the timeline.
import { useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";
import { commands } from "./commands";
import { settings } from "./settings";
import type { CullBlurRegion, Force, RulesPreview, Rules } from "./bindings";
import {
  DEFAULT_RULES, buildModel, clampRules, frameKey, listSignature, requestItems, rulesScenes, sceneOfFrame, type Ctx, type Flags,
  type SceneModel, type SceneSource,
} from "./scenes";

export type ScenesStatus = "idle" | "building" | "ready" | "error";
export type ScenesState = {
  on: boolean;                                        // the grid shows scenes
  status: ScenesStatus;
  updating: boolean;                                  // a new build runs behind the timeline on screen
  progress: { done: number; total: number } | null;
  error: string;
  signature: string;
  items: ReturnType<typeof requestItems>;             // the build's photos, by index
  model: SceneModel | null;
  quality: (number | null)[];
  regions: (CullBlurRegion | null)[];                 // each frame's sharpest region, once scored
  accepted: ReadonlySet<string>;
  network: boolean;
  hashed: number;
  unhashed: number;
  open: number | null;                                // the scene shown in the lightbox
  scoring: { done: number; total: number; paused: boolean } | null;
  hideSingles: boolean;
  collapsed: ReadonlySet<number>;
  focus: number;                                      // the scene the keyboard is on
  rules: Rules;                                       // the keep rules
  preview: RulesPreview | null;                       // what the rules would do, as last asked for
  previewStale: boolean;                              // the rules, scores or piles changed since
  previewBusy: boolean;
  forced: ReadonlyMap<string, Force>;                 // frames moved to a pile by hand in the review
  piles: boolean;                                     // the piles review is open
  ruleSession: { id: number; state: "planned" | "applied" | "undone" } | null;
  ruleNote: { text: string; error: boolean } | null;
};

const EMPTY: ScenesState = {
  on: false, status: "idle", updating: false, progress: null, error: "", signature: "", items: [], model: null, quality: [], regions: [],
  accepted: new Set(), network: false, hashed: 0, unhashed: 0, open: null, scoring: null, hideSingles: false,
  collapsed: new Set(), focus: 0, rules: DEFAULT_RULES, preview: null, previewStale: false, previewBusy: false,
  forced: new Map(), piles: false, ruleSession: null, ruleNote: null,
};

let state: ScenesState = EMPTY;
const listeners = new Set<() => void>();
export const scenesState = () => state;
export function setScenes(patch: Partial<ScenesState>) {
  state = { ...state, ...patch };
  listeners.forEach((l) => l());
}
const subscribe = (l: () => void) => { listeners.add(l); return () => { listeners.delete(l); }; };
export const useScenes = () => useSyncExternalStore(subscribe, scenesState);
// the app shell follows only whether Scenes shows, not the timeline's every score and keystroke
export const useScenesOn = () => useSyncExternalStore(subscribe, () => state.on);

// ---- Preferences > Culling ----
export const ACCEPT_STAR_KEY = "immage.scenes.acceptStar";
export const SCORING_KEY = "immage.scenes.scoring";
export type ScoringMode = "auto" | "on" | "off";
export function loadAcceptStar(): boolean {
  try { return settings.get(ACCEPT_STAR_KEY) === "1"; } catch { return false; }
}
export function storeAcceptStar(on: boolean) {
  try { settings.set(ACCEPT_STAR_KEY, on ? "1" : "0"); } catch { /* not kept */ }
}
export function loadScoringMode(): ScoringMode {
  try { const v = settings.get(SCORING_KEY); return v === "on" || v === "off" ? v : "auto"; } catch { return "auto"; }
}
export function storeScoringMode(mode: ScoringMode) {
  try { settings.set(SCORING_KEY, mode); } catch { /* not kept */ }
}
// scoring reads a 1024 px preview of every photo, so by default it runs on local disks only
export const scoresInBackground = (mode: ScoringMode, network: boolean) => mode === "on" || (mode === "auto" && !network);

// ---- building ----
let jobs = 1_000_000;
let current = 0;                 // the build that counts
let scoreRun = 0;                // the scoring run that counts
let listening: Promise<unknown> | null = null;
let lastEntries: readonly SceneSource[] = [];   // the photos of the last build, for a rebuild after a split or merge
let onProgress: (e: { job: number; stage: string; done: number; total: number }) => void = () => {};

function ensureListener() {
  listening ??= listen<{ job: number; stage: string; done: number; total: number }>("scenes-progress", (e) => onProgress(e.payload));
  return listening;
}

// Builds the timeline of `entries` unless it is the one already built. The timeline
// on screen stays until the new one is ready
export async function buildScenes(entries: readonly SceneSource[], force = false, reopen?: string): Promise<void> {
  const local = entries.filter((e) => !e.remote);
  lastEntries = entries;
  const signature = listSignature(local);
  if (!force && signature === state.signature && state.status !== "error") return;
  const job = ++jobs;
  if (current) void commands.scenesCancel(current).catch(() => {});
  current = job;
  scoreRun++;
  const keep = state.model !== null;
  setScenes({ status: keep ? "ready" : "building", updating: keep, progress: { done: 0, total: local.length }, error: "", signature, scoring: null });
  onProgress = (p) => { if (p.job === job && p.stage === "reading") setScenes({ progress: { done: p.done, total: p.total } }); };
  try {
    await ensureListener();
    const items = requestItems(local);
    const [build, accepted] = await Promise.all([commands.scenesBuild(job, { items, params: null }), commands.scenesAccepted().catch(() => [])]);
    if (job !== current) return;
    const quality = build.quality as (number | null)[];
    const model = buildModel(build);
    // after a split or merge the scene holding the frame the user was on stays open
    const at = reopen ? items.findIndex((it) => frameKey(it.path, it.version) === reopen) : -1;
    const kept = at >= 0 ? sceneOfFrame(model, at) : -1;
    setScenes({
      status: "ready", updating: false, progress: null, items, model, quality, regions: build.regions as (CullBlurRegion | null)[],
      accepted: new Set(accepted.map((a) => frameKey(a.path, a.version))), network: build.network,
      hashed: build.hashed, unhashed: build.unhashed, open: kept >= 0 ? kept : null, focus: kept >= 0 ? kept : 0, collapsed: new Set(),
      preview: null, previewStale: false, forced: new Map(), piles: false,
    });
    if (scoresInBackground(loadScoringMode(), build.network)) void scoreAll();
  } catch (e) {
    if (job !== current) return;
    // a stopped build leaves the old timeline; a failed one says so
    if (String(e) === "cancelled") return;
    setScenes({ status: "error", updating: false, progress: null, error: String(e), signature: "" });
  }
}

export function stopScenes() {
  if (current) void commands.scenesCancel(current).catch(() => {});
  current = 0;
  scoreRun++;
  setScenes({ updating: false, progress: null, scoring: null });
}

// ---- scoring ----
function applyScores(scores: { path: string; version: number; quality: number | null; sharpRegion?: CullBlurRegion | null }[]) {
  const index = new Map(state.items.map((it, i) => [frameKey(it.path, it.version), i]));
  const quality = state.quality.slice();
  const regions = state.regions.slice();
  for (const s of scores) {
    const i = index.get(frameKey(s.path, s.version));
    if (i === undefined || s.quality === null) continue;
    quality[i] = s.quality;
    regions[i] = s.sharpRegion ?? null;
  }
  setScenes({ quality, regions, previewStale: state.preview !== null });
}

// Scores a scene's unscored frames (the open scene first: under two seconds on a local disk)
export async function scoreScene(scene: number): Promise<void> {
  const info = state.model?.scenes[scene];
  if (!info) return;
  const targets = info.frames.filter((f) => state.quality[f] === null).map((f) => ({ path: state.items[f].path, version: state.items[f].version }));
  if (!targets.length) return;
  const job = ++jobs;
  try { applyScores(await commands.scenesScore(job, targets)); } catch { /* the frames stay unscored */ }
}

let paused = false;
export function pauseScoring(on: boolean) {
  paused = on;
  if (state.scoring) setScenes({ scoring: { ...state.scoring, paused: on } });
}

// The rest of the folder, a scene at a time from the top, until it is done or stopped
export async function scoreAll(): Promise<void> {
  const model = state.model;
  if (!model) return;
  const run = ++scoreRun;
  const todo = model.scenes.filter((s) => s.frames.some((f) => state.quality[f] === null));
  const total = model.frameCount;
  const scored = () => state.quality.filter((q) => q !== null).length;
  if (!todo.length) return;
  setScenes({ scoring: { done: scored(), total, paused } });
  for (const scene of todo) {
    while (paused && run === scoreRun) await new Promise((r) => setTimeout(r, 400));
    if (run !== scoreRun) return;
    await scoreScene(scene.n - 1);
    if (run !== scoreRun) return;
    setScenes({ scoring: { done: scored(), total, paused } });
  }
  if (run === scoreRun) setScenes({ scoring: null });
}

// ---- decisions ----
export function rememberAccepted(path: string, version: number | undefined, accepted: boolean) {
  const key = frameKey(path, version);
  const next = new Set(state.accepted);
  if (accepted) next.add(key); else next.delete(key);
  setScenes({ accepted: next });
  void commands.scenesSetAccepted(path, version ?? 0, accepted).catch(() => {});
}

// ---- the view ----
export const openScene = (scene: number) => setScenes({ open: scene, focus: scene });
export const closeScene = () => setScenes({ open: null });
export const setScenesOn = (on: boolean) => setScenes(on ? { on } : { on, open: null, piles: false });
export function toggleBlock(block: number) {
  const next = new Set(state.collapsed);
  if (next.has(block)) next.delete(block); else next.add(block);
  setScenes({ collapsed: next });
}

// ---- split and merge ----
// Remembers the user's cut or join (replayed after every grouping) and builds the timeline again; the
// hashes are kept, so the rebuild decodes nothing. The scene holding `frame` stays open
export async function overrideScene(action: "split" | "merge", frame: number): Promise<void> {
  const it = state.items[frame];
  if (!it) return;
  await commands.scenesOverride(it.path, it.version, action);
  await buildScenes(lastEntries, true, frameKey(it.path, it.version));
}

// ---- the keep rules ----
export function setRules(patch: Partial<Rules>) {
  setScenes({ rules: clampRules({ ...state.rules, ...patch }), previewStale: state.preview !== null, ruleNote: null });
}
export const resetRules = () => setScenes({ rules: DEFAULT_RULES, previewStale: state.preview !== null, ruleNote: null });

// asks Rust what the rules would do; nothing is written. A later answer to an earlier question is dropped
let previewRun = 0;
export async function previewRules(ctx: Ctx<Flags>): Promise<void> {
  const model = state.model;
  if (!model) return;
  const run = ++previewRun;
  setScenes({ previewBusy: true });
  try {
    const preview = await commands.scenesRulesPreview({ scenes: rulesScenes(model, state.items, ctx, state.forced), rules: state.rules });
    if (run === previewRun) setScenes({ preview, previewStale: false, previewBusy: false });
  } catch (e) {
    if (run === previewRun) setScenes({ previewBusy: false, ruleNote: { text: String(e), error: true } });
  }
}

// a frame, or every frame of a scene, moved to the Keep or Reject pile by hand (null clears)
export function forceFrames(keys: readonly string[], force: Force | null) {
  const next = new Map(state.forced);
  for (const k of keys) { if (force) next.set(k, force); else next.delete(k); }
  setScenes({ forced: next, previewStale: state.preview !== null });
}
