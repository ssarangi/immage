// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Auto cull's pure logic, kept free of React and Tauri so node tests can run it:
// the scope a cull runs over, the chips and counts of the review panel, the
// order and badge of a photo in the grid, keyboard decisions in review, and
// how a natural-language search lands in Find. The calls and the screens are
// CullDialog.tsx, CullPanel.tsx, CullInspector.tsx and cullStore.ts.
import type {
  CullDecision, CullPlan, CullScope, CullScopeItem, CullSession, CullSettings, DuplicateRatio, GeminiSearch,
} from "./bindings";
import type { Filter } from "./filter";
import type { ImageEntry } from "./library";

export type CullChip = "all" | "highlights" | "blurry" | "duplicates" | "rejected";
export const CHIPS: readonly CullChip[] = ["all", "highlights", "blurry", "duplicates", "rejected"];

// a photo as a decision names it: the file and its version (0 for the original)
export const decisionKey = (path: string, version?: number | null) => `${path}\u0000${version ?? 0}`;

export const SHOOT_TYPES = ["general", "portrait", "wildlife"] as const;
export const RATIOS: readonly DuplicateRatio[] = ["more", "moderate", "less"];

// the planner's own defaults (cull_plan.rs); Preferences > Culling stores the user's
export type CullDefaults = {
  shootType: string; duplicateRatio: DuplicateRatio; highlightPercent: number;
  blurThreshold: number; similarityThreshold: number;
  // what Apply gives the highlights: a minimum rating (0 none) and a color label bit (0 none)
  highlightRating: number; highlightLabel: number;
  // faces in the score (pose, closed eyes) and whose eyes count: the main subject's or any face's
  useFaces: boolean; eyeMode: "mainSubject" | "anyFace";
};
export const DEFAULT_SETTINGS: CullDefaults = {
  shootType: "general", duplicateRatio: "moderate", highlightPercent: 15, blurThreshold: 100, similarityThreshold: 28,
  highlightRating: 0, highlightLabel: 0, useFaces: true, eyeMode: "mainSubject",
};

// a stored value, kept to what the dialog could have set; anything else is the default
export function parseDefaults(raw: string | null | undefined): CullDefaults {
  let v: Partial<Record<keyof CullDefaults, unknown>> = {};
  try { v = JSON.parse(raw ?? "null") ?? {}; } catch { /* the defaults */ }
  const num = (x: unknown, lo: number, hi: number, d: number) =>
    typeof x === "number" && Number.isFinite(x) && x >= lo && x <= hi ? x : d;
  const d = DEFAULT_SETTINGS;
  return {
    shootType: (SHOOT_TYPES as readonly unknown[]).includes(v.shootType) ? v.shootType as string : d.shootType,
    duplicateRatio: (RATIOS as readonly unknown[]).includes(v.duplicateRatio) ? v.duplicateRatio as DuplicateRatio : d.duplicateRatio,
    highlightPercent: Math.round(num(v.highlightPercent, 0, 100, d.highlightPercent)),
    blurThreshold: num(v.blurThreshold, 0, 10000, d.blurThreshold),
    similarityThreshold: Math.round(num(v.similarityThreshold, 0, 512, d.similarityThreshold)),
    highlightRating: Math.round(num(v.highlightRating, 0, 5, 0)),
    highlightLabel: [0, 1, 2, 4, 8, 16].includes(v.highlightLabel as number) ? v.highlightLabel as number : 0,
    useFaces: typeof v.useFaces === "boolean" ? v.useFaces : d.useFaces,
    eyeMode: v.eyeMode === "anyFace" ? "anyFace" : "mainSubject",
  };
}


// the photos a cull runs over: the selection when any of the listed photos is
// selected, else everything listed. Immich results are not on this computer, so
// they never go. A RAW's stacked JPEG goes as `backing`, so it takes the
// decision with it
export function scopeFor(
  shown: ImageEntry[], selection: Set<number>, partners: Map<number, ImageEntry[]>,
): { items: CullScopeItem[]; selected: boolean } {
  const local = shown.filter((i) => !i.remote);
  const picked = local.filter((i) => selection.has(i.imgid));
  const list = picked.length ? picked : local;
  return {
    selected: picked.length > 0,
    items: list.map((i) => {
      const backing = (partners.get(i.imgid) ?? []).filter((p) => !p.remote)
        .map((p) => ({ path: p.path, version: p.version ?? 0 }));
      return { path: i.path, version: i.version ?? 0, ...(backing.length ? { backing } : {}) };
    }),
  };
}

export const scopeOf = (label: string, items: CullScopeItem[]): CullScope => ({ label, items });

// how the decision reads: an override that flipped the verdict is its own reason
export type ReasonKey = "unique" | "bestInGroup" | "keptInGroup" | "blurry" | "duplicateOf" | "lowQuality" | "keepOverride" | "rejectOverride";
export function reasonOf(d: CullDecision): ReasonKey {
  if (d.finalStatus && d.finalStatus !== d.proposed) return d.finalStatus === "keep" ? "keepOverride" : "rejectOverride";
  return d.reason;
}

export function inChip(d: CullDecision, chip: CullChip): boolean {
  switch (chip) {
    case "all": return true;
    case "highlights": return d.highlight && d.status === "keep";
    case "blurry": return d.reason === "blurry";
    case "duplicates": return d.group !== null;
    case "rejected": return d.status === "reject";
  }
}

export function chipCounts(ds: CullDecision[]): Record<CullChip, number> {
  const n: Record<CullChip, number> = { all: ds.length, highlights: 0, blurry: 0, duplicates: 0, rejected: 0 };
  for (const d of ds) for (const c of CHIPS) if (c !== "all" && inChip(d, c)) n[c]++;
  return n;
}

// why the rejects were rejected, most common first (ties in a fixed order)
export function rejectionReasons(ds: CullDecision[]): { reason: ReasonKey; count: number }[] {
  const counts = new Map<ReasonKey, number>();
  for (const d of ds) if (d.status === "reject") counts.set(reasonOf(d), (counts.get(reasonOf(d)) ?? 0) + 1);
  const order: ReasonKey[] = ["blurry", "duplicateOf", "lowQuality", "rejectOverride", "unique", "bestInGroup", "keptInGroup", "keepOverride"];
  return [...counts].map(([reason, count]) => ({ reason, count }))
    .sort((a, b) => b.count - a.count || order.indexOf(a.reason) - order.indexOf(b.reason));
}

export type Badge = "best" | "keep" | "reject";
export const badgeOf = (d: CullDecision): Badge => d.status === "reject" ? "reject" : d.best ? "best" : "keep";

export function byKey(plan: CullPlan | null): Map<string, CullDecision> {
  return new Map((plan?.decisions ?? []).map((d) => [decisionKey(d.path, d.version), d]));
}

// the grid in review: the chip's photos only, a set's photos together with its
// best frame first, others where they were. A photo the cull did not see (added
// since) is kept only under All
export function reviewOrder<T extends { path: string; version?: number }>(
  shown: T[], decisions: Map<string, CullDecision>, chip: CullChip,
): T[] {
  const seen = shown.map((i) => ({ i, d: decisions.get(decisionKey(i.path, i.version)) }));
  const kept = seen.filter(({ d }) => (d ? inChip(d, chip) : chip === "all"));
  const firstAt = new Map<number, number>();
  kept.forEach(({ d }, n) => { if (d?.group != null && !firstAt.has(d.group)) firstAt.set(d.group, n); });
  const place = (n: number, d?: CullDecision) => (d?.group != null ? firstAt.get(d.group)! : n);
  return kept.map((x, n) => ({ ...x, n, p: place(n, x.d) }))
    .sort((a, b) => a.p - b.p || (a.d?.group != null ? (a.d.best === b.d?.best ? a.n - b.n : a.d.best ? -1 : 1) : a.n - b.n))
    .map(({ i }) => i);
}

// the next photo after a decision in review: forward, staying on the last
export function nextInReview<T extends { imgid: number }>(list: T[], current: number, by: 1 | -1): number | null {
  const at = list.findIndex((i) => i.imgid === current);
  if (at < 0) return list[0]?.imgid ?? null;
  return list[Math.min(list.length - 1, Math.max(0, at + by))]?.imgid ?? null;
}

// what a key does in the lightbox while a cull is under review. P keeps (K is Keywords), R
// rejects, B makes the photo its set's best, arrows move. Typing is never taken
export type ReviewAction = { type: "keep" | "reject" | "best" } | { type: "step"; by: 1 | -1 };
export function reviewKey(e: { key: string; ctrlKey?: boolean; metaKey?: boolean; altKey?: boolean; typing?: boolean }): ReviewAction | null {
  if (e.typing || e.ctrlKey || e.metaKey || e.altKey) return null;
  switch (e.key.toLowerCase()) {
    case "p": return { type: "keep" };
    case "r": return { type: "reject" };
    case "b": return { type: "best" };
    default: return null;
  }
}

// replaces one decision in a plan (after an override), keeping the session's
// counts in step the way the backend counts them
export function withDecision(plan: CullPlan, next: CullDecision): CullPlan {
  const decisions = plan.decisions.map((d) => {
    if (d.id === next.id) return next;
    // making a frame the set's best takes the mark from the old one
    if (next.best && next.group !== null && d.group === next.group && d.best) return { ...d, best: false };
    return d;
  });
  const session: CullSession = { ...plan.session,
    keepCount: decisions.filter((d) => d.status === "keep").length,
    rejectCount: decisions.filter((d) => d.status === "reject").length,
    highlightCount: decisions.filter((d) => d.highlight && d.status === "keep").length };
  return { session, decisions };
}

// a number the setup dialog shows and sends: clamped, whole
export const clampPercent = (n: number) => Math.min(100, Math.max(0, Math.round(Number.isFinite(n) ? n : 0)));

// ---- Find's natural-language search ------------------------------------------------------

// the stored name the words name best: an exact match, else one that contains
// the words (or the other way round); no guess otherwise
export function matchNamed(want: string | null, names: string[]): string | null {
  const w = want?.trim().toLowerCase();
  if (!w) return null;
  const norm = (s: string) => s.toLowerCase().replace(/\s+/g, " ").trim();
  const exact = names.find((n) => norm(n) === w);
  if (exact) return exact;
  const partial = names.filter((n) => norm(n).includes(w) || w.includes(norm(n)));
  return partial.length === 1 ? partial[0] : partial.sort((a, b) => a.length - b.length)[0] ?? null;
}

// Gemini's reading of a sentence, as the Find state it sets: the words, the
// folder Filter (merged over the one in force, lists replaced), a camera and
// lens only when the folder has a matching one, and the color labels. What it
// could not set is `unmapped` and `notFound` for the panel to say
export function applySearch<S extends { search: { text: string }; filter: Filter }>(
  state: S, found: GeminiSearch, cameras: string[], lenses: string[],
): { state: S; unmapped: string[] } {
  const f = found.filter;
  const unmapped = [...found.unmapped];
  const filter: Filter = { ...state.filter };
  if (f.minRating !== undefined) filter.minRating = f.minRating;
  if (f.rejected === "all" || f.rejected === "hide" || f.rejected === "only") filter.rejected = f.rejected;
  if (f.edited === "all" || f.edited === "edited" || f.edited === "unedited") filter.edited = f.edited;
  if (f.kinds) filter.kinds = f.kinds;
  if (f.keywords) filter.keywords = f.keywords;
  if (f.takenFrom !== undefined) filter.takenFrom = f.takenFrom;
  if (f.takenTo !== undefined) filter.takenTo = f.takenTo;
  for (const k of ["iso", "aperture", "focal", "shutter"] as const) if (f[k]) filter[k] = f[k];
  if (found.labels) { filter.labels = found.labels; filter.unlabeled = false; }
  const cam = matchNamed(found.camera, cameras);
  if (found.camera) { if (cam) filter.cameras = [cam]; else unmapped.push(found.camera); }
  const lens = matchNamed(found.lens, lenses);
  if (found.lens) { if (lens) filter.lenses = [lens]; else unmapped.push(found.lens); }
  return { state: { ...state, search: { ...state.search, text: found.text }, filter }, unmapped };
}
