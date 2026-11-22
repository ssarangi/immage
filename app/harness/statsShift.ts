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
.stats-view .st-pc-im img, .stats-view .st-tc-im img, .stats-view .st-tcs-im img, .stats-view .st-grid-btn { transform: none !important; }`;
  document.head.append(st);
}

const withClass = (el: Element, cls: string, on: boolean) => {
  for (let e: Element | null = el; e && e !== document.body; e = e.parentElement) { if (cls === "__fv" && e !== el) break; e.classList.toggle(cls, on); }
};

type Result = { action: string; max: number; worst: string; compared: number };

/** The Places map loads when its section comes near the view: scroll there and wait for its bubbles (or its empty card). */
async function openPlaces() {
  const sec = document.getElementById("places"), sc = document.querySelector<HTMLElement>(".st-scroll");
  if (!sec || !sc) return;
  sc.scrollTop += sec.getBoundingClientRect().top - sc.getBoundingClientRect().top - 130;
  for (let i = 0; i < 100 && !sec.hasAttribute("data-ready"); i++) await new Promise((r) => setTimeout(r, 50));
  await new Promise((r) => setTimeout(r, 500));
}

/** A real click on a bubble selects an area: every box of the section, from the section's corner, stays where it was (by selector and position, as the panel swaps its contents). */
async function clickMapBubble() {
  const out = { max: 0, worst: "", checked: 0, clicked: false };
  const sec = document.getElementById("places");
  const bubble = sec?.querySelector<SVGElement>(".st-pl-b:not(.merged)") ?? sec?.querySelector<SVGElement>(".st-pl-b");
  if (!sec || !bubble) return out;
  const sc = document.querySelector<HTMLElement>(".st-scroll")!;
  sc.scrollTop += sec.getBoundingClientRect().top - sc.getBoundingClientRect().top - 130;
  await new Promise((r) => setTimeout(r, 100));
  const q = [".st-sec-head", ".st-lede", ".st-pl", ".st-pl-stage", ".st-pl-map", ".st-pl-modes", ".st-pl-zoom", ".st-pl-panel", ".st-pl-main", ".st-pl-head", ".st-pl-name", ".st-pl-coords",
    ".st-pl-tiles", ".st-pl-tiles > div", ".st-pl-sum", ".st-pl-photos", ".st-pl-photos > *", ".st-pl-act", ".st-pl-act .st-btn", ".st-pl-jump", ".st-pl-chip", ".st-pl-foot", ".st-pl-b"];
  const measure = () => {
    const c = sec.getBoundingClientRect(), m = new Map<string, R>();
    for (const sel of q) Array.from(sec.querySelectorAll(sel)).slice(0, 14).forEach((e, i) => { const r = e.getBoundingClientRect(); m.set(`${sel}#${i}`, rect(new DOMRect(r.left - c.left, r.top - c.top, r.width, r.height))); });
    return m;
  };
  const before = measure();
  bubble.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  await new Promise((r) => setTimeout(r, 30));
  const mid = measure(); // the panel's skeletons, when the numbers are still on their way
  await new Promise((r) => setTimeout(r, 900));
  const after = measure();
  for (const [from, to] of [[before, after], [mid, after]] as const) {
    for (const [k, a] of from) {
      const b = to.get(k);
      if (!b) continue;
      out.checked++;
      const d = Math.max(Math.abs(a.x - b.x), Math.abs(a.y - b.y), Math.abs(a.w - b.w), Math.abs(a.h - b.h));
      if (d > out.max) { out.max = Math.round(d * 100) / 100; out.worst = k; }
    }
  }
  out.clicked = true;
  bubble.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  await new Promise((r) => setTimeout(r, 400));
  return out;
}

export async function runStatsShiftTest() {
  mirrorPseudoClasses();
  const results: Result[] = [];
  await openPlaces();
  await frames(4);
  const base = snapshot();
  const sc0 = document.querySelector<HTMLElement>(".st-scroll")!, top0 = sc0.scrollTop;
  const note = (action: string) => { sc0.scrollTop = top0; const d = diff(base, snapshot()); results.push({ action, max: d.max, worst: d.worst, compared: d.compared }); };
  const name = (el: Element) => `${el.tagName.toLowerCase()}.${el.className.toString().split(" ").join(".")} "${(el.getAttribute("aria-label") ?? el.textContent ?? "").trim().slice(0, 24)}"`;

  // the map's bubbles are many: the first six stand for them
  let bubbleCount = 0;
  const marks = Array.from(root().querySelectorAll<HTMLElement | SVGElement>("button, [role=button], [data-nav], .st-pc, input")).filter((el) => !el.matches(".st-pl-b") || ++bubbleCount <= 6);
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
  // the clicks below are measured from the top of the page, as they were before the map section was opened
  sc0.scrollTop = 0; await frames();

  // a real selection: click a mark of each chart that has a recorded reply (a donut slice, an aperture ring, a shutter bar,
  // an ISO bin, a clock wedge, a month), compare the frame around the content, then click it again to clear it. The frame is
  // measured as before (absolute); the chart's own boxes are measured from their section's corner, because a filter changes
  // the data, and so the height, of the sections above (a different number of photo cards), which is not the click's doing.
  const frame = [".st-head", ".st-hero", ".st-coverage", ".st-stats", ".st-stat", ".st-nav", ".st-nav-row", ".st-fbar", ".st-era", ".st-donut-wrap", ".st-donut", ".st-card"];
  const chartBoxes = [".st-iris", ".st-iris-cell", ".st-shutter", ".st-shutter-col", ".st-iso", ".st-iso-col", ".st-clock-wrap", ".st-clock", ".st-months", ".st-month"];
  const sc = sc0;
  const targets = [".st-donut .st-hit", '.st-iris-cell[data-id="a3"]', '.st-shutter-col[data-id="handheld"]', '.st-iso-col[data-id="400"]', '.st-hour[data-id="18"]', '.st-month[data-id="10"]'];
  const click = { max: 0, worst: "", checked: 0, clicked: false, marks: 0 };
  const measure = (e: Element, own: boolean): R => {
    const r = e.getBoundingClientRect();
    const sec = own ? e.closest("section.st-sec")?.getBoundingClientRect() : null;
    return sec ? rect(new DOMRect(r.left - sec.left, r.top - sec.top, r.width, r.height)) : rect(new DOMRect(r.left, r.top + sc.scrollTop, r.width, r.height));
  };
  for (const sel of targets) {
    const mark = root().querySelector<SVGElement | HTMLElement>(sel);
    if (!mark) continue;
    const own = !sel.startsWith(".st-donut");
    const nodes: [Element, boolean][] = [
      // the Cameras section's cards, era row and legend change height with the data (fewer bodies match), so an own-chart click leaves them out
      ...frame.filter((q) => !own || !/card|era|donut/.test(q)).flatMap((q) => Array.from(root().querySelectorAll(q)).slice(0, q === ".st-card" ? 2 : 12).map((e): [Element, boolean] => [e, false])),
      ...(own ? [...chartBoxes.flatMap((q) => Array.from(root().querySelectorAll(q)).slice(0, 12)), ...Array.from(mark.closest("section.st-sec")?.querySelectorAll(".st-card") ?? [])].map((e): [Element, boolean] => [e, true]) : []),
    ];
    const before = new Map<Element, R>(nodes.map(([e, o]) => [e, measure(e, o)]));
    const owns = new Map(nodes);
    mark.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await new Promise((r) => setTimeout(r, 400));
    for (const [e, a] of before) {
      if (!e.isConnected) continue;
      const b = measure(e, owns.get(e)!);
      const d = Math.max(Math.abs(a.x - b.x), Math.abs(a.y - b.y), Math.abs(a.w - b.w), Math.abs(a.h - b.h));
      click.checked++;
      if (d > click.max) { click.max = Math.round(d * 100) / 100; click.worst = `${sel}: ${label(e)}`; }
    }
    click.clicked = true; click.marks++;
    // clear it again (the same mark, the same chart) so the next target starts from the recorded base state
    root().querySelector<SVGElement | HTMLElement>(sel)?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    await new Promise((r) => setTimeout(r, 400));
  }
  const map = await clickMapBubble();
  const max = results.reduce((m, r) => Math.max(m, r.max), 0);
  return { ok: max === 0 && click.max === 0 && map.max === 0, map, maxDelta: max, interactions: results.length, compared: results.reduce((m, r) => m + r.compared, 0),
    click, failures: results.filter((r) => r.max > 0).slice(0, 12) };
}
