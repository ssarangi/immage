// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The no-layout-shift test for Preferences > General > AI models (spec section 8). It checks, in the
// harness page with fixture data:
//  1. across every pack state (not installed, downloading at 1/34/99%, verifying, installed, checked
//     flash, repair, stopped, update, unsupported) the pack card, its header, state slot, action row and
//     switch rows have the identical box, and so does the summary strip with every background-work line;
//  2. within a state, hover and keyboard focus on every control move nothing (CSS :hover and
//     :focus-visible are mirrored onto classes, as a script cannot hover).
// verify/models-shift.test.mjs runs it in headless Chrome with ?shift=1 and asserts zero movement.
import { CARD_STATES } from "./modelsFixtures";

type R = { x: number; y: number; w: number; h: number };
type Snap = Map<Node, R>;
const frames = (n = 3) => new Promise<void>((done) => { setTimeout(done, 25 * n); });
const page = () => document.querySelector<HTMLElement>("[data-harness-page]")!;
// net of the page area's own scroll: focusing a control may scroll it, which is not a shift of the layout
const rect = (r: DOMRect): R => ({ x: r.left, y: r.top + (page()?.scrollTop ?? 0), w: r.width, h: r.height });
// painted text and bars that legitimately change with the data; their boxes are fixed, their text runs are not compared
const SKIP = ".pack-bar > i";

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
      if (t.includes(":hover") || t.includes(":focus-visible")) css.push(t.replaceAll(":hover", ".__hover").replaceAll(":focus-visible", ".__fv"));
    }
  }
  const st = document.createElement("style");
  st.textContent = css.join("\n") + "\n[data-harness-page] *, [data-harness-page] *::before, [data-harness-page] *::after { transition: none !important; animation: none !important; }";
  document.head.append(st);
}
const withClass = (el: Element, cls: string, on: boolean) => {
  for (let e: Element | null = el; e && e !== document.body; e = e.parentElement) { if (cls === "__fv" && e !== el) break; e.classList.toggle(cls, on); }
};
const FIXED = [".ai-head", ".ai-lead", ".ai-sum", ".ai-sum > div:nth-child(1)", ".ai-sum > div:nth-child(2)", ".ai-sum > div:nth-child(3)",
  ".ai-sum .bw .pack-bar", ".ai-sum .bw .btn", ".pack", ".pack-h", ".pack-st", ".pack-dl", ".pack-slot", ".pack-msg", ".pack-bar", ".pack-acts", ".pack-an",
  ".pack-an .pref-row:nth-child(1)", ".pack-an .pref-row:nth-child(2)", ".pack-t h5"];

export async function runShiftTest() {
  const h = (window as unknown as { __h: { set: (s: string) => void } }).__h;
  mirrorPseudoClasses();
  const failures: { what: string; delta: number; worst: string }[] = [];
  let interactions = 0, compared = 0, maxDelta = 0;
  const boxes: Record<string, Record<string, R>> = {};

  for (const state of CARD_STATES) {
    h.set(state); await frames();
    boxes[state] = {};
    for (const s of FIXED) { const el = document.querySelector(s); if (el) boxes[state][s] = rect(el.getBoundingClientRect()); }
    // within a state: hover and focus on every control
    const base = snapshot();
    for (const el of Array.from(page().querySelectorAll<HTMLElement>("button, a, input"))) {
      const name = `${el.tagName.toLowerCase()}.${el.className.toString().split(" ").join(".")} "${(el.getAttribute("aria-label") ?? el.textContent ?? "").trim().slice(0, 24)}"`;
      for (const [cls, label] of [["__hover", "hover"], ["__fv", "focus"]] as const) {
        withClass(el, cls, true); if (cls === "__fv") el.focus(); await frames(1);
        const d = diff(base, snapshot()); interactions++; compared += d.compared;
        if (d.max > 0) failures.push({ what: `${state}: ${label} ${name}`, delta: d.max, worst: d.worst });
        maxDelta = Math.max(maxDelta, d.max);
        withClass(el, cls, false); if (cls === "__fv") el.blur();
      }
    }
  }
  // across states: the same boxes, wherever the card is
  const reference = boxes[CARD_STATES[0]];
  let fixedMax = 0, fixedWorst = "";
  for (const state of CARD_STATES) {
    for (const [sel, r0] of Object.entries(reference)) {
      const r = boxes[state][sel];
      if (!r) { fixedMax = Math.max(fixedMax, 1e6); fixedWorst = `${state}: ${sel} is missing`; continue; }
      // a pack's own chips and the status text change with the state; the boxes below must not
      const d = Math.max(Math.abs(r.x - r0.x), Math.abs(r.y - r0.y), Math.abs(r.w - r0.w), Math.abs(r.h - r0.h));
      if (d > fixedMax) { fixedMax = d; fixedWorst = `${state}: ${sel} (${JSON.stringify(r)} vs ${JSON.stringify(r0)})`; }
    }
  }
  fixedMax = Math.round(fixedMax * 100) / 100;
  if (fixedMax > 0) failures.push({ what: "across states", delta: fixedMax, worst: fixedWorst });
  return { ok: failures.length === 0, states: CARD_STATES.length, interactions, compared, maxDelta, fixedMaxDelta: fixedMax, fixedWorst, failures };
}
