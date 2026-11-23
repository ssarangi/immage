// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The no-layout-shift test for Scenes. It records the rect of every element (and the toolbar, rail dots,
// block headers, cards, stage and filmstrip by selector) before and after: (a) data arriving (skeleton to
// content), (b) a decision, a selection and a fold, (c) scores arriving, (d) the status line changing. It
// also audits the stylesheet: no hover, focus, selected or pressed rule may touch a property that sizes
// or places a box. A PerformanceObserver counts the browser's own layout-shift entries as a second check.
// verify/scenes-shift.test.mjs runs it in headless Chrome and asserts zero; the break=1 style is the
// negative control. The 20,000-photo run scrolls the whole timeline and reports the most cards mounted.
import { forceFrames, scenesState, setRules, setScenes } from "../src/scenesStore";

type R = { x: number; y: number; w: number; h: number };
type Hooks = { setEntries: (f: (e: unknown[]) => unknown[]) => void; onMeta: (e: unknown, f: unknown) => void; data: { entries: unknown[] } };
const h = () => (window as unknown as { __h: Hooks }).__h;
const frames = (n = 3) => new Promise<void>((done) => { setTimeout(done, 25 * n); });
const rect = (r: DOMRect): R => ({ x: Math.round(r.left * 100) / 100, y: Math.round(r.top * 100) / 100, w: Math.round(r.width * 100) / 100, h: Math.round(r.height * 100) / 100 });
const SKIP = ".scene-img, canvas";

function snapshot(root: Element): Map<Element, R> {
  const out = new Map<Element, R>();
  out.set(root, rect(root.getBoundingClientRect()));
  for (const el of root.querySelectorAll("*")) if (!el.closest(SKIP)) out.set(el, rect(el.getBoundingClientRect()));
  return out;
}
const label = (el: Element) => `${el.tagName.toLowerCase()}.${String(el.className).split(" ").join(".")}`;

function diff(a: Map<Element, R>, b: Map<Element, R>, ignore = "") {
  let max = 0, worst = "", compared = 0;
  const failures: string[] = [];
  for (const [el, r] of a) {
    const s = b.get(el);
    if (!s || (ignore && el.closest(ignore))) continue;
    compared++;
    const d = Math.max(Math.abs(r.x - s.x), Math.abs(r.y - s.y), Math.abs(r.w - s.w), Math.abs(r.h - s.h));
    if (d > 0) failures.push(`${label(el)} ${d}`);
    if (d > max) { max = d; worst = label(el); }
  }
  return { max, worst, compared, failures: failures.slice(0, 8) };
}

const SIZING = /^(width|height|min-width|max-width|min-height|max-height|margin.*|padding.*|border(-\w+)*-width|border-width|border|font-size|font-weight|letter-spacing|line-height|display|position|top|left|right|bottom|inset.*|flex.*|gap|row-gap|column-gap|grid.*|content|text-transform)$/;
const STATE = /:hover|:focus|:active|\.active|\[aria-(selected|checked|current|pressed|expanded)/;
function auditStyles(): string[] {
  const bad: string[] = [];
  for (const sheet of Array.from(document.styleSheets)) {
    let rules: CSSRuleList;
    try { rules = sheet.cssRules; } catch { continue; }
    const walk = (list: CSSRuleList) => {
      for (const rule of Array.from(list)) {
        if (rule instanceof CSSMediaRule) { walk(rule.cssRules); continue; }
        if (!(rule instanceof CSSStyleRule)) continue;
        const sel = rule.selectorText;
        if (!/\.scene|\.scenes-/.test(sel) || !STATE.test(sel)) continue;
        for (const prop of Array.from(rule.style)) if (SIZING.test(prop)) bad.push(`${sel} { ${prop} }`);
      }
    };
    walk(rules);
  }
  return bad;
}

const sel = (s: string) => {
  const el = document.querySelector(s);
  return el ? rect(el.getBoundingClientRect()) : null;
};
const same = (a: R | null, b: R | null) => !!a && !!b && a.x === b.x && a.y === b.y && a.w === b.w && a.h === b.h;

export async function runScenesShift() {
  const out: Record<string, unknown> = { failures: [] as string[] };
  const fail = (m: string) => (out.failures as string[]).push(m);
  let shifts = 0, supported = false;
  const detail: string[] = [];
  // the browser's own count. entries are collected as they come and judged at the end of each phase, so a
  // phase that reflows the list on purpose (the Hide single frames toggle, scrolling) can be left out
  const pending: PerformanceEntry[] = [];
  let observer: PerformanceObserver | null = null;
  try {
    observer = new PerformanceObserver((list) => { pending.push(...list.getEntries()); });
    observer.observe({ type: "layout-shift", buffered: true });
    supported = true;
  } catch { /* not supported */ }
  const drain = (keep: boolean) => {
    pending.push(...(observer?.takeRecords() ?? []));
    for (const e of pending.splice(0)) {
      const v = e as unknown as { value: number; hadRecentInput: boolean; sources?: { node?: Element; previousRect: DOMRect; currentRect: DOMRect }[] };
      if (!keep || v.hadRecentInput) continue;
      shifts += v.value;
      if (detail.length < 8) detail.push(`${v.value.toFixed(4)} ${(v.sources ?? []).map((x) => `${(x.node as Element | undefined)?.tagName ? label(x.node as Element) : "?"} ${Math.round(x.previousRect.y)}>${Math.round(x.currentRect.y)}`).join(", ")}`);
    }
  };
  await frames();
  drain(false);
  const view = document.querySelector(".scenes-view")!;
  const ready = { ...scenesState() };

  // (a) data arrives: toolbar, body and the first block and card sit where the skeleton had them
  setScenes({ status: "building", model: null, progress: { done: 7200, total: 20400 }, scoring: null });
  await frames();
  const skeleton = { bar: sel(".scenes-bar"), body: sel(".scenes-body"), head: sel(".scene-skeleton .scene-block-head"), card: sel(".scene-skel-card"), dot: sel(".scene-skeleton .scene-dot") };
  setScenes({ ...ready, progress: null });
  await frames();
  const content = { bar: sel(".scenes-bar"), body: sel(".scenes-body"), head: sel(".scene-block-head"), card: sel(".scene-card"), dot: sel(".scene-dot") };
  drain(true);
  out.arrival = { skeleton, content };
  for (const k of ["bar", "body", "head", "dot"] as const) if (!same(skeleton[k], content[k])) fail(`arrival: ${k} moved`);
  if (!skeleton.card || !content.card || skeleton.card.x !== content.card.x || skeleton.card.y !== content.card.y || skeleton.card.w !== content.card.w || skeleton.card.h !== content.card.h) fail("arrival: first card moved");

  // (b) a decision (the library flags change, a badge appears), a selection, a fold
  const entries = h().data.entries as { rejected: boolean; labels: number }[];
  let before = snapshot(view);
  const target = document.querySelector(".scene-card")!;
  const scene = Number(target.id.replace("scene-card-", ""));
  h().setEntries((all) => (all as { rejected: boolean }[]).map((e, i) => (i < 40 ? { ...e, rejected: !e.rejected } : e)));
  await frames();
  let after = snapshot(view);
  const decision = diff(before, after);
  before = after;
  setScenes({ focus: scene + 1 });
  await frames();
  after = snapshot(view);
  const selection = diff(before, after);
  before = after;
  void entries;
  out.decision = decision; out.selection = selection;

  // (c) scores arrive
  setScenes({ quality: scenesState().quality.map(() => null) });
  await frames();
  before = snapshot(view);
  setScenes({ quality: scenesState().quality.map((_, i) => ((i * 37) % 100) / 100) });
  await frames();
  after = snapshot(view);
  const scores = diff(before, after);
  drain(true);
  out.scores = scores;

  // (d) the status line changes: nothing but its own text moves
  const fixedBefore = { toggle: sel(".scenes-toggle"), summary: sel(".scenes-summary"), scroll: sel(".scenes-scroll"), bar: sel(".scenes-bar") };
  const texts: [string, unknown][] = [["a", { scoring: { done: 1, total: 20400, paused: false } }], ["b", { scoring: { done: 20399, total: 20400, paused: true } }],
    ["c", { scoring: null, updating: true, progress: { done: 5, total: 9 } }], ["d", { updating: false, progress: null }]];
  const statusRects: R[] = [];
  for (const [, patch] of texts) {
    setScenes(patch as never);
    await frames();
    const now = { toggle: sel(".scenes-toggle"), summary: sel(".scenes-summary"), scroll: sel(".scenes-scroll"), bar: sel(".scenes-bar") };
    for (const k of Object.keys(now) as (keyof typeof now)[]) if (!same(now[k], fixedBefore[k])) fail(`status: ${k} moved`);
    const st = sel(".scenes-status"); if (st) statusRects.push(st);
  }
  drain(true);
  out.statusRects = statusRects.length;
  // the browser's own count is final here: what follows reflows the list on purpose
  out.layoutShift = { supported, value: Math.round(shifts * 10000) / 10000, detail };

  // Hide single frames drops cards, so the rows below them move; the elements that stay are unchanged
  before = snapshot(view);
  setScenes({ hideSingles: true });
  await frames();
  setScenes({ hideSingles: false });
  await frames(4);
  const hide = diff(before, snapshot(view));
  out.hide = hide;

  // the stylesheet: no state rule changes a size
  const styles = auditStyles();
  out.styleOffenders = styles;
  if (styles.length) fail(`state rules touch a size: ${styles.slice(0, 4).join("; ")}`);

  // 20,000 photos: scroll the whole timeline and count what is mounted
  const scroller = document.querySelector<HTMLElement>(".scenes-scroll");
  if (scroller) {
    let maxCards = 0, maxRows = 0;
    const total = scroller.scrollHeight;
    for (let y = 0; y < total; y += Math.max(700, Math.floor(total / 80))) {
      scroller.scrollTop = y;
      await frames(1);
      maxCards = Math.max(maxCards, document.querySelectorAll(".scene-card").length);
      maxRows = Math.max(maxRows, document.querySelectorAll(".scene-row, .scene-day").length);
    }
    out.scroll = { total, maxCards, maxRows, scenes: scenesState().model?.scenes.length };
  }

  // what a language must not move: the cards, rail dots, block headers and the list itself
  const boxes = (css: string, n: number) => Array.from(document.querySelectorAll(css)).slice(0, n).map((el) => rect(el.getBoundingClientRect()));
  setScenes({ scoring: null, updating: false, progress: null });
  scroller && (scroller.scrollTop = 0);
  await frames(3);
  out.fixed = { cards: boxes(".scene-card", 12), dots: boxes(".scene-dot", 4), heads: boxes(".scene-block-head", 4), thumbs: boxes(".scene-thumb", 12), scroll: boxes(".scenes-scroll", 1), bar: boxes(".scenes-bar", 1) };

  await frames(4);
  const worst = Math.max(decision.max, selection.max, hide.max, scores.max);
  out.maxDelta = worst;
  out.compared = decision.compared + selection.compared + scores.compared;
  out.ok = (out.failures as string[]).length === 0 && worst === 0;
  return out;
}

// The scene view: the stage, header, tabs, side panel, decision buttons, filmstrip and bottom bar stay where
// they are when another frame is picked, a decision is made, scores arrive or another scene is opened. What
// changes by design (the texts and the picture) is left out; every other element is compared.
const SCENE_SKIP = ".scene-img, canvas, bdi, .scene-info-sub, .scene-info-title, .culling-filename, .culling-photo-count, .scene-info-note, .scene-tile-cap span, .scene-tile, .scene-info-stars, .scene-sharp-rect, .scene-crop-img";
const fixedSelectors = [".culling-lightbox", ".culling-toolbar", ".culling-stage", ".culling-stage-inner", ".cull-side", ".cull-tabs", ".cull-pane", ".scene-strip", ".scene-decide",
  ".scene-decide button", ".scene-tiles", ".scene-tile-list", ".scene-tiles-head", ".scene-info", ".scene-nav", ".scene-edit", ".scene-edit button", ".scene-crop", ".scene-crop-head", ".scene-crop-box", ".culling-bottom-bar"];

export async function runSceneViewShift() {
  const out: Record<string, unknown> = { failures: [] as string[] };
  const fail = (m: string) => (out.failures as string[]).push(m);
  await frames(4);
  const box = document.querySelector(".culling-lightbox")!;
  const snap = () => {
    const m = new Map<Element, R>();
    for (const el of [box, ...box.querySelectorAll("*")]) if (!el.closest(SCENE_SKIP) && !el.matches(SCENE_SKIP)) m.set(el, rect(el.getBoundingClientRect()));
    return m;
  };
  const fixed = () => fixedSelectors.map((s) => [s, Array.from(document.querySelectorAll(s)).map((e) => rect(e.getBoundingClientRect()))] as const);
  const click = (css: string, nth = 0) => (document.querySelectorAll<HTMLElement>(css)[nth])?.click();
  const steps: [string, () => void][] = [
    ["pick another frame", () => click(".scene-tile", 2)],
    ["accept", () => click(".scene-decide button", 0)],
    ["reject", () => click(".scene-decide button", 2)],
    ["undecided", () => click(".scene-decide button", 1)],
    ["sort by time", () => click(".scene-sort")],
    ["scores go", () => setScenes({ quality: scenesState().quality.map(() => null) })],
    ["scores arrive", () => setScenes({ quality: scenesState().quality.map((_, i) => ((i * 53) % 100) / 100) })],
    ["regions go", () => setScenes({ regions: scenesState().regions.map(() => null) })],
    ["regions arrive", () => setScenes({ regions: scenesState().regions.map((_, i) => ({ x: 0.2 + (i % 5) * 0.1, y: 0.3, width: 0.25, height: 0.3 })) })],
    ["scoring status", () => setScenes({ scoring: { done: 5, total: 100, paused: false } })],
    ["next scene", () => click(".scene-nav button", 1)],
    ["previous scene", () => click(".scene-nav button", 0)],
  ];
  let before = snap(), fixedBefore = fixed(), maxDelta = 0, compared = 0;
  const log: { step: string; max: number; worst: string }[] = [];
  for (const [name, act] of steps) {
    act();
    await frames(4);
    const after = snap();
    const d = diff(before, after);
    const f = fixed();
    // the fixed boxes keep their number and place (a frame count may change the number of tiles, not the list's box)
    f.forEach(([s, rs], i) => {
      const prev = fixedBefore[i][1];
      if (s === ".scene-tile-list" || s === ".scene-decide button" || s === ".scene-tiles-head") {
        if (rs.length === prev.length && rs.some((r, k) => !same(r, prev[k]))) fail(`${name}: ${s} moved`);
      } else if (rs.length === prev.length && rs.some((r, k) => !same(r, prev[k]))) fail(`${name}: ${s} moved`);
    });
    log.push({ step: name, max: d.max, worst: d.worst });
    compared += d.compared;
    maxDelta = Math.max(maxDelta, d.max);
    if (d.max > 0) fail(`${name}: ${d.worst} moved by ${d.max}`);
    before = after; fixedBefore = f;
  }
  out.log = log; out.maxDelta = maxDelta; out.compared = compared;
  out.styleOffenders = auditStyles();
  if ((out.styleOffenders as string[]).length) fail("state rules touch a size");
  out.ok = (out.failures as string[]).length === 0 && maxDelta === 0;
  return out;
}

// The rules panel and the piles review: a preview arriving, the rules changing (the preview goes stale), Apply and
// Undo and their notes, a frame moved to the other pile, and scrolling the piles move nothing but text and the
// tiles themselves. The panel, the toolbar, the timeline and the piles' rows, heads and columns are compared.
const RULES_SKIP = ".scene-img, canvas, bdi, .scene-rule-counts span, .scene-rule-state, .scene-rule-notes p, .scene-pile-tile, .scene-pile-strip *, .scene-pile-head > span, .scene-pile-title, .scene-rule-value, .scene-piles-bar .scenes-summary";
export async function runRulesShift(piles: boolean) {
  const out: Record<string, unknown> = { failures: [] as string[] };
  const fail = (m: string) => (out.failures as string[]).push(m);
  await frames(4);
  const root = document.querySelector(".harness-app")!;
  // scrolling moves the scrolled content by definition, and a preview going or arriving empties or fills the list, so those steps leave the list's own rows out; their
  // height is the one fixed PILE_ROW_H, which the 20,000-photo run checks through the list's total height
  let scrolling = false;
  const snap = () => {
    const m = new Map<Element, R>();
    for (const el of [root, ...root.querySelectorAll("*")]) {
      if (el.closest(RULES_SKIP) || el.matches(RULES_SKIP)) continue;
      if (scrolling && el.closest(".scene-piles-scroll")) continue;
      m.set(el, rect(el.getBoundingClientRect()));
    }
    return m;
  };
  const fake = (window as unknown as { __fake: { fakePreview: () => never } }).__fake;
  const keysOf = (n: number) => (scenesState().model!.scenes[n].frames.map((f) => { const it = scenesState().items[f]; return `${it.version}:${it.path}`; }));
  const steps: [string, () => void][] = [
    ["preview goes", () => setScenes({ preview: null, previewStale: false })],
    ["preview arrives", () => setScenes({ preview: fake.fakePreview(), previewStale: false })],
    ["keep best changes", () => setRules({ keepBest: 3 })],
    ["minimum quality", () => setRules({ minQuality: 60 })],
    ["always keep off", () => setRules({ alwaysKeep: false })],
    ["singles are kept", () => setRules({ singlesKeep: true })],
    ["preview again", () => setScenes({ preview: fake.fakePreview(), previewStale: false })],
    ["applied", () => setScenes({ ruleSession: { id: 1, state: "applied" }, ruleNote: { text: "Applied: 1,204 photos rejected.", error: false } })],
    ["a long error note", () => setScenes({ ruleNote: { text: "Cull failed: " + "the file changed after it was analysed. ".repeat(12), error: true } })],
    ["undone", () => setScenes({ ruleSession: { id: 1, state: "undone" }, ruleNote: { text: "Undone: 1,204 photos put back as they were.", error: false } })],
    ["busy", () => setScenes({ previewBusy: true })],
    ["not busy", () => setScenes({ previewBusy: false })],
  ];
  if (!piles) steps.push(["piles open", () => setScenes({ piles: true })]);
  steps.push(
    ["a frame moves to the other pile", () => { forceFrames([keysOf(1)[2]], "keep"); setScenes({ preview: fake.fakePreview() }); }],
    ["a scene is rejected", () => { forceFrames(keysOf(2), "reject"); setScenes({ preview: fake.fakePreview() }); }],
    ["scroll the piles", () => { const el = document.querySelector<HTMLElement>(".scene-piles-scroll"); if (el) el.scrollTop = 1500; }],
    ["scroll back", () => { const el = document.querySelector<HTMLElement>(".scene-piles-scroll"); if (el) el.scrollTop = 0; }],
  );
  let before = snap(), maxDelta = 0, compared = 0, maxRows = 0;
  const log: { step: string; max: number; worst: string }[] = [];
  for (const [name, act] of steps) {
    scrolling = name.startsWith("scroll") || name.startsWith("preview");
    before = snap();
    act();
    await frames(4);
    maxRows = Math.max(maxRows, document.querySelectorAll(".scene-pile-row").length);
    const after = snap();
    const d = diff(before, after);
    log.push({ step: name, max: d.max, worst: d.worst });
    compared += d.compared;
    maxDelta = Math.max(maxDelta, d.max);
    if (d.max > 0) fail(`${name}: ${d.worst} moved by ${d.max}`);
    before = after;
  }
  // the piles list: scroll the whole of it and count the rows mounted
  const scroller = document.querySelector<HTMLElement>(".scene-piles-scroll");
  if (scroller) {
    const total = scroller.scrollHeight;
    for (let y = 0; y < total; y += Math.max(400, Math.floor(total / 60))) {
      scroller.scrollTop = y;
      await frames(1);
      maxRows = Math.max(maxRows, document.querySelectorAll(".scene-pile-row").length);
    }
    out.piles = { total, maxRows, scenes: scenesState().preview?.verdicts.length };
  }
  out.log = log; out.maxDelta = maxDelta; out.compared = compared;
  out.styleOffenders = auditStyles();
  if ((out.styleOffenders as string[]).length) fail("state rules touch a size");
  out.ok = (out.failures as string[]).length === 0 && maxDelta === 0;
  return out;
}
