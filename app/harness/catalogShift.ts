// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The no-layout-shift test for the Catalog card (spec section 12). In the harness page it checks:
//  1. across every tall state (finding, reading at 1/43/99%, thumbnails, paused, done, problems) the
//     source rows, the card and its header, phase rows, bars and footer have the identical box; the
//     short states (offline, idle, never indexed) share theirs;
//  2. within a state, hover and keyboard focus on every control move nothing (CSS :hover and
//     :focus-visible are mirrored onto classes, as a script cannot hover);
//  3. the Rebuild row of Preferences keeps its box when it asks for the second click (view=prefs).
// verify/catalog-shift.test.mjs runs it in headless Chrome with ?shift=1 and asserts zero movement.
import { SHORT, TALL } from "./catalogFixtures";

type R = { x: number; y: number; w: number; h: number };
type Snap = Map<Node, R>;
const frames = (n = 3) => new Promise<void>((done) => { setTimeout(done, 25 * n); });
const page = () => document.querySelector<HTMLElement>("[data-harness-page]")!;
const rect = (r: DOMRect): R => ({ x: r.left, y: r.top + (page()?.scrollTop ?? 0), w: r.width, h: r.height });
// bars fill and text runs change with the data; their boxes are fixed and measured on the containers
const SKIP = ".cat-bar > i";

function snapshot(root: Element = page()): Snap {
  const out: Snap = new Map();
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT);
  for (let n: Node | null = walker.nextNode(); n; n = walker.nextNode()) {
    const el = n as Element;
    if (el.closest(SKIP)) continue;
    out.set(n, rect(el.getBoundingClientRect()));
  }
  return out;
}
const describe = (n: Node) => { const el = n as Element; return `${el.tagName.toLowerCase()}.${(el.className ?? "").toString().split(" ").join(".")}`; };
function diff(a: Snap, b: Snap) {
  let max = 0, worst = "", compared = 0;
  for (const [n, r] of a) {
    const s = b.get(n);
    if (!s) continue;
    compared++;
    const d = Math.max(Math.abs(r.x - s.x), Math.abs(r.y - s.y), Math.abs(r.w - s.w), Math.abs(r.h - s.h));
    if (d > max) { max = d; worst = describe(n); }
  }
  return { max: Math.round(max * 100) / 100, worst, compared };
}
function mirrorPseudoClasses() {
  const css: string[] = [];
  for (const sheet of Array.from(document.styleSheets)) {
    let rules: CSSRuleList;
    try { rules = sheet.cssRules; } catch { continue; }
    for (const rule of Array.from(rules)) {
      const t = rule.cssText;
      if (t.includes(":hover") || t.includes(":focus-visible") || t.includes(":focus-within")) {
        css.push(t.replaceAll(":hover", ".__hover").replaceAll(":focus-visible", ".__fv").replaceAll(":focus-within", ".__fw"));
      }
    }
  }
  const st = document.createElement("style");
  st.textContent = css.join("\n") + "\n[data-harness-page] *, [data-harness-page] *::before, [data-harness-page] *::after { transition: none !important; animation: none !important; }";
  document.head.append(st);
}
const withClass = (el: Element, cls: string, on: boolean) => {
  for (let e: Element | null = el; e && e !== document.body; e = e.parentElement) { if (cls === "__fv" && e !== el) break; e.classList.toggle(cls, on); }
};
const ROWS = [".cat-list li:nth-child(1) .cat-row", ".cat-list li:nth-child(1) .cat-slot", ".cat-list li:nth-child(1) .cat-state",
  ".cat-list li:nth-child(2) .cat-row", ".cat-list li:nth-child(3) .cat-row", ".cat-list li:nth-child(4) .cat-row", ".cat-list li:nth-child(5) .cat-row",
  ".cat-list li:nth-child(1) .cat-volume", ".cat-list li:nth-child(2) .cat-volume"];
const CARD = [".cat-card", ".cat-head", ".cat-head-ring", ".cat-head-text", ".cat-acts", ".cat-phases", ".cat-phase:nth-child(1)", ".cat-phase:nth-child(2)",
  ".cat-phase:nth-child(3)", ".cat-phase:nth-child(1) .cat-bar", ".cat-phase:nth-child(2) .cat-bar", ".cat-phase:nth-child(3) .cat-bar", ".cat-foot", ".cat-foot-a", ".cat-foot-b"];
const SHORT_CARD = [".cat-card", ".cat-head", ".cat-acts", ".cat-foot", ".cat-foot-a", ".cat-foot-b"];

type H = { set: (s: string) => void; confirm: (on: boolean) => void; open: (on: boolean) => void };
const boxesOf = (sels: string[]) => {
  const out: Record<string, R> = {};
  for (const s of sels) { const el = document.querySelector(s); if (el) out[s] = rect(el.getBoundingClientRect()); }
  return out;
};

async function interactions(failures: { what: string; delta: number; worst: string }[], label: string) {
  const base = snapshot();
  let n = 0, compared = 0, maxDelta = 0;
  for (const el of Array.from(page().querySelectorAll<HTMLElement>("button, a, input, [role=button]"))) {
    const name = `${el.tagName.toLowerCase()}.${el.className.toString().split(" ").join(".")} "${(el.getAttribute("aria-label") ?? el.textContent ?? "").trim().slice(0, 24)}"`;
    for (const [cls, what] of [["__hover", "hover"], ["__fv", "focus"]] as const) {
      withClass(el, cls, true); if (cls === "__fv") el.focus(); await frames(1);
      const d = diff(base, snapshot()); n++; compared += d.compared;
      if (d.max > 0) failures.push({ what: `${label}: ${what} ${name}`, delta: d.max, worst: d.worst });
      maxDelta = Math.max(maxDelta, d.max);
      withClass(el, cls, false); if (cls === "__fv") el.blur();
    }
  }
  return { n, compared, maxDelta };
}

function compareAcross(group: string[], boxes: Record<string, Record<string, R>>, failures: { what: string; delta: number; worst: string }[]) {
  const reference = boxes[group[0]];
  let max = 0, worst = "";
  for (const state of group) {
    for (const [sel, r0] of Object.entries(reference)) {
      const r = boxes[state][sel];
      if (!r) { max = Math.max(max, 1e6); worst = `${state}: ${sel} is missing`; continue; }
      const d = Math.max(Math.abs(r.x - r0.x), Math.abs(r.y - r0.y), Math.abs(r.w - r0.w), Math.abs(r.h - r0.h));
      if (d > max) { max = d; worst = `${state}: ${sel} (${JSON.stringify(r)} vs ${JSON.stringify(r0)})`; }
    }
  }
  max = Math.round(max * 100) / 100;
  if (max > 0) failures.push({ what: "across states", delta: max, worst });
  return { max, worst };
}

// the folded card's header: every state, folded or open, keeps the title, the status slot, the add
// button and the chevron where they were (the status takes only free space)
const HEADER = [".catalog-panel .panel-title", ".catalog-panel .panel-title > span:first-child", ".section-status", ".cat-add", ".chevron"];
async function headerTest(failures: { what: string; delta: number; worst: string }[]) {
  const h = (window as unknown as { __h: H }).__h;
  const names = ["reading", "finding", "thumbs", "paused", "yielding", "idle", "done", "offline", "never"];
  const boxes: Record<string, Record<string, R>> = {};
  const shown: Record<string, string> = {};
  for (const open of [false, true]) {
    h.open(open);
    for (const state of names) {
      h.set(state); await frames();
      const key = `${open ? "open" : "folded"}:${state}`;
      boxes[key] = boxesOf(HEADER);
      shown[key] = document.querySelector(".section-status")?.textContent ?? "";
      if (!open && (state === "reading" || state === "thumbs") && !shown[key]) failures.push({ what: `${key}: the folded header shows no status`, delta: 1, worst: key });
      if (open && shown[key]) failures.push({ what: `${key}: the open header shows a status`, delta: 1, worst: key });
    }
  }
  const keys = Object.keys(boxes);
  const c = compareAcross(keys, boxes, failures);
  h.open(false); h.set("reading"); await frames();
  const r = await interactions(failures, "folded");
  return { ok: failures.length === 0, states: keys.length, interactions: r.n, compared: r.compared, maxDelta: r.maxDelta, fixedMaxDelta: c.max, fixedWorst: c.worst, shown, failures };
}

export async function runShiftTest() {
  const h = (window as unknown as { __h: H }).__h;
  mirrorPseudoClasses();
  const failures: { what: string; delta: number; worst: string }[] = [];
  let interactionsRun = 0, compared = 0, maxDelta = 0, fixedMax = 0, fixedWorst = "";
  const view = new URLSearchParams(location.search).get("view") ?? "card";

  if (new URLSearchParams(location.search).get("fold") === "1") return headerTest(failures);

  if (view === "prefs") {
    const boxes: Record<string, Record<string, R>> = {};
    for (const [name, on] of [["ask", false], ["sure", true]] as const) {
      h.confirm(on); await frames();
      boxes[name] = boxesOf([".pref-card", "[data-pref-label='Rebuild catalog']", "[data-pref-label='Catalog disk']", "[data-pref-label='Catalog sources']"]);
      const r = await interactions(failures, `prefs ${name}`);
      interactionsRun += r.n; compared += r.compared; maxDelta = Math.max(maxDelta, r.maxDelta);
    }
    const c = compareAcross(["ask", "sure"], boxes, failures);
    fixedMax = c.max; fixedWorst = c.worst;
    return { ok: failures.length === 0, states: 2, interactions: interactionsRun, compared, maxDelta, fixedMaxDelta: fixedMax, fixedWorst, failures };
  }

  const boxes: Record<string, Record<string, R>> = {};
  for (const state of [...TALL, ...SHORT]) {
    h.set(state); await frames();
    boxes[state] = boxesOf([...ROWS, ...(TALL.includes(state) ? CARD : SHORT_CARD)]);
    const r = await interactions(failures, state);
    interactionsRun += r.n; compared += r.compared; maxDelta = Math.max(maxDelta, r.maxDelta);
  }
  // the rows and the card are where they were in every state of their variant
  for (const group of [TALL, SHORT]) {
    const c = compareAcross(group, boxes, failures);
    if (c.max > fixedMax) { fixedMax = c.max; fixedWorst = c.worst; }
  }
  return { ok: failures.length === 0, states: TALL.length + SHORT.length, interactions: interactionsRun, compared, maxDelta, fixedMaxDelta: fixedMax, fixedWorst, failures };
}
