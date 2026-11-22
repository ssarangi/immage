// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The no-layout-shift test for the Statistics view. It records the rect of every element and text run in the
// view, applies hover, keyboard focus and a selected state to every interactive element in turn (a script
// cannot hover, so CSS :hover and :focus-visible are mirrored onto classes), records again and reports the
// largest movement. Then it really clicks a chart mark and checks that the frame around the content (header,
// hero, strip, nav, filter bar, the clicked card) did not move. verify/stats-shift.test.mjs runs it in
// headless Chrome with ?shift=1 and asserts that every movement is zero.
type R = { x: number; y: number; w: number; h: number };
type Snap = Map<Node, R>;

const SKIP = ".st-tip, .st-pop, .st-progress";
// timers, not rAF: headless Chrome's virtual time does not run frames on its own
const frames = (n = 2) => new Promise<void>((done) => { setTimeout(done, 20 * n); });
const root = () => document.querySelector<HTMLElement>(".stats-view")!;
const rect = (r: DOMRect): R => ({ x: r.left, y: r.top, w: r.width, h: r.height });

function snapshot(): Snap {
  const out: Snap = new Map();
  const sc = document.querySelector(".st-scroll")!;
  const origin = 0;void sc;
  const walker = document.createTreeWalker(root(), NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if (n.nodeType === Node.ELEMENT_NODE) {
      const el = n as Element;
      if (el.closest(SKIP)) continue;
      const r = el.getBoundingClientRect();
      out.set(n, rect(new DOMRect(r.left, r.top + origin, r.width, r.height)));
    } else if ((n.textContent ?? "").trim() && !n.parentElement?.closest(SKIP)) {
      const range = document.createRange(); range.selectNodeContents(n);
      const r = range.getBoundingClientRect();
      out.set(n, rect(new DOMRect(r.left, r.top + origin, r.width, r.height)));
    }
  }
  return out;
}

const label = (n: Node) => {
  const el = n.nodeType === Node.TEXT_NODE ? n.parentElement : (n as Element);
  return `${el?.tagName.toLowerCase()}.${(el?.className ?? "").toString().split(" ").join(".")}${n.nodeType === Node.TEXT_NODE ? " (text)" : ""}`;
};

function diff(a: Snap, b: Snap) {
  let max = 0, worst = "", compared = 0;
  for (const [n, r] of a) {
    const s = b.get(n);
    if (!s) continue;
    compared++;
    const d = Math.max(Math.abs(r.x - s.x), Math.abs(r.y - s.y), Math.abs(r.w - s.w), Math.abs(r.h - s.h));
    if (d > max) { max = d; worst = label(n); }
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
      if ((t.includes(":hover") || t.includes(":focus-visible") || t.includes(":focus-within")) && t.includes("st-")) {
        css.push(t.replaceAll(":hover", ".__hover").replaceAll(":focus-visible", ".__fv").replaceAll(":focus-within", ".__fw"));
      }
    }
  }
  const st = document.createElement("style");
  st.textContent = css.join("\n") + `
.stats-view *, .stats-view *::before, .stats-view *::after { transition: none !important; animation: none !important; }
/* layout is measured: a paint-only transform (a photo's hover scale) is not a shift */
.stats-view .st-pc-im img, .stats-view .st-grid-btn { transform: none !important; }`;
  document.head.append(st);
}

const withClass = (el: Element, cls: string, on: boolean) => {
  for (let e: Element | null = el; e && e !== document.body; e = e.parentElement) { if (cls === "__fv" && e !== el) break; e.classList.toggle(cls, on); }
};

type Result = { action: string; max: number; worst: string; compared: number };

export async function runStatsShiftTest() {
  mirrorPseudoClasses();
  const results: Result[] = [];
  await frames(4);
  const base = snapshot();
  const sc0 = document.querySelector<HTMLElement>(".st-scroll")!, top0 = sc0.scrollTop;
  const note = (action: string) => { sc0.scrollTop = top0; const d = diff(base, snapshot()); results.push({ action, max: d.max, worst: d.worst, compared: d.compared }); };
  const name = (el: Element) => `${el.tagName.toLowerCase()}.${el.className.toString().split(" ").join(".")} "${(el.getAttribute("aria-label") ?? el.textContent ?? "").trim().slice(0, 24)}"`;

  const marks = Array.from(root().querySelectorAll<HTMLElement | SVGElement>("button, [role=button], [data-nav], .st-pc, input"));
  for (const el of marks) {
    if (el.closest(SKIP)) continue;
    // hover and the hover of its ancestors (a card is hovered when its button is)
    withClass(el, "__hover", true); await frames(); note(`hover ${name(el)}`); withClass(el, "__hover", false);
    // keyboard focus
    (el as HTMLElement).focus?.({ preventScroll: true }); withClass(el, "__fv", true); withClass(el, "__fw", true); await frames(); note(`focus ${name(el)}`);
    withClass(el, "__fv", false); withClass(el, "__fw", false); (el as HTMLElement).blur?.();
    // a selected state, as the page sets it
    if (el.hasAttribute("aria-pressed")) {
      const was = el.getAttribute("aria-pressed");
      el.setAttribute("aria-pressed", was === "true" ? "false" : "true"); await frames(); note(`select ${name(el)}`);
      el.setAttribute("aria-pressed", was!); await frames();
    }
  }
  // a donut with one slice picked: the rest dim
  const hits = Array.from(root().querySelectorAll<SVGElement>(".st-donut .st-hit"));
  hits.forEach((h, i) => h.classList.toggle("dim", i !== 0)); await frames(); note("donut dims");
  hits.forEach((h) => h.classList.remove("dim"));

  // a real selection: click a donut slice, then compare the frame around the content
  const frame = [".st-head", ".st-hero", ".st-coverage", ".st-stats", ".st-stat", ".st-nav", ".st-nav-row", ".st-fbar", ".st-era", ".st-donut-wrap", ".st-donut", ".st-card"];
  const before = new Map<Element, R>();
  const nodesOf = () => frame.flatMap((s) => Array.from(root().querySelectorAll(s)).slice(0, s === ".st-card" ? 2 : 12));
  const sc = sc0;
  const first = nodesOf();
  first.forEach((e) => { const r = e.getBoundingClientRect(); before.set(e, rect(new DOMRect(r.left, r.top + top0, r.width, r.height))); });
  const slice = root().querySelector<SVGElement>(".st-donut .st-hit");
  let click = { max: 0, worst: "", checked: 0, clicked: false };
  if (slice) {
    slice.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await new Promise((r) => setTimeout(r, 400));
    const top1 = sc.scrollTop;
    for (const [e, a] of before) {
      if (!e.isConnected) continue;
      const r = e.getBoundingClientRect(); const b = rect(new DOMRect(r.left, r.top + top1, r.width, r.height));
      const d = Math.max(Math.abs(a.x - b.x), Math.abs(a.y - b.y), Math.abs(a.w - b.w), Math.abs(a.h - b.h));
      click.checked++;
      if (d > click.max) { click.max = Math.round(d * 100) / 100; click.worst = label(e); }
    }
    click.clicked = true;
  }
  const max = results.reduce((m, r) => Math.max(m, r.max), 0);
  return { ok: max === 0 && click.max === 0, maxDelta: max, interactions: results.length, compared: results.reduce((m, r) => m + r.compared, 0),
    click, failures: results.filter((r) => r.max > 0).slice(0, 12) };
}
