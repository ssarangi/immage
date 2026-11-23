// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The bottom toolbar's checks, run in the page by harness/toolbar.tsx (?test=1) and read by
// verify/toolbar-shift.test.mjs: its geometry against the grid surface, that it never scrolls
// sideways, that nothing moves when the view, the filters, Find or the cull state change, that the
// 12px gap needs no script, and its keyboard and accessible names.

export type H = { set: (p: Record<string, unknown>) => void; width: (w: number) => void };
const frames = (n = 3) => new Promise<void>((done) => { setTimeout(done, 40 * n); });
const R = (e: Element) => { const r = e.getBoundingClientRect(); return [r.left, r.top, r.width, r.height]; };
const near = (a: number[], b: number[], eps = 0.5) => a.every((v, i) => Math.abs(v - b[i]) <= eps);

function geometry() {
  const col = document.querySelector(".center")!, bar = document.querySelector(".toolbar")!;
  const surf = document.querySelector(".filmstrip.grid");
  const b = bar.getBoundingClientRect(), c = col.getBoundingClientRect();
  const out: Record<string, unknown> = { height: b.height, below: c.bottom - b.bottom, scroll: [(bar as HTMLElement).scrollWidth, (bar as HTMLElement).clientWidth] };
  if (surf) {
    const s = surf.getBoundingClientRect();
    out.left = b.left - s.left; out.width = b.width - s.width; out.gap = b.top - s.bottom;
  }
  return out;
}

// every control, by identity: what is in the bar in two states can be compared
function snapshot() {
  const out = new Map<Element, number[]>();
  const bar = document.querySelector(".toolbar")!;
  out.set(bar, R(bar));
  for (const e of bar.querySelectorAll("*")) if (e.getClientRects().length) out.set(e, R(e));
  return out;
}

const accessibleName = (e: Element) => (e.getAttribute("aria-label") ?? e.getAttribute("title") ?? e.textContent ?? "").trim();
const items = () => [...document.querySelectorAll<HTMLElement>(".toolbar button:not(:disabled)")].filter((e) => e.getClientRects().length > 0);

export async function runToolbarTest(h: H) {
  const fail: string[] = [];
  const out: Record<string, unknown> = { fail };
  let shifts = 0;
  // only the state changes of step 2 count: the harness resizing its own column is a shift by design
  // and only what moves inside the bar: a view change replaces the stage's content, which the page may shift
  const inBar = (e: PerformanceEntry) => ((e as unknown as { sources?: { node?: Node | null }[] }).sources ?? []).some((s) => !!s.node && !!document.querySelector(".toolbar")?.contains(s.node));
  const shiftSeen: string[] = [];
  const add = (e: PerformanceEntry) => { if (inBar(e)) { shifts += (e as unknown as { value: number }).value; (e as unknown as { sources: { node?: Element; previousRect: DOMRect; currentRect: DOMRect }[] }).sources.forEach((s) => shiftSeen.push(`${s.node?.className}:${s.previousRect.x},${s.previousRect.y},${s.previousRect.width}->${s.currentRect.x},${s.currentRect.y},${s.currentRect.width}`)); } };
  const po = new PerformanceObserver(() => {});   // entries are read with takeRecords, in the step that made them
  po.observe({ type: "layout-shift", buffered: false });

  // 1. geometry, in every view and width
  const geo: Record<string, unknown>[] = [];
  const dir = document.documentElement.dir;
  for (const view of ["edit", "grid", "scenes"]) {
    h.set({ view }); await frames();
    for (const w of [1600, 1280, 1000, 800, 680]) {
      h.width(w); await frames();
      const g = geometry(); geo.push({ view, w, ...g });
      if (g.height !== 40) fail.push(`${view} ${w}: bar height ${g.height}`);
      if (Math.abs((g.below as number) - 8) > 0.5) fail.push(`${view} ${w}: ${g.below}px below the bar`);
      if (view !== "edit") {
        if (Math.abs(g.left as number) > 0.5 || Math.abs(g.width as number) > 0.5) fail.push(`${view} ${w}: bar differs from the grid surface by ${g.left}, ${g.width}`);
        if (Math.abs((g.gap as number) - 12) > 0.5) fail.push(`${view} ${w}: gap ${g.gap}`);
      }
      const [sw, cw] = g.scroll as number[];
      if (sw > cw + 0.5) fail.push(`${view} ${w}: bar scrolls sideways (${sw} > ${cw})`);
      if (document.documentElement.scrollWidth > innerWidth + 1) fail.push(`${view} ${w}: the page scrolls sideways`);
    }
  }
  out.geometry = geo.length;
  h.width(1280); h.set({ view: "grid", cull: "none", filter: false, find: false }); await frames();

  // 2. nothing moves when the state changes
  let compared = 0, worst = 0;
  const steps: Record<string, unknown>[] = [{ view: "edit" }, { view: "scenes" }, { view: "grid" }, { cull: "pending" }, { filter: true }, { find: true }, { cull: "none" }, { filter: false }, { find: false }];
  for (const w of [1600, 1280, 1000, 800]) {
    h.width(w); h.set({ view: "grid", cull: "none", filter: false, find: false }); await frames();
    for (const step of steps) {
      const a = snapshot();
      po.takeRecords();
      h.set(step); await frames();
      po.takeRecords().forEach(add);
      const b = snapshot();
      for (const [e, r] of a) {
        const s = b.get(e);
        if (!s) continue;
        compared++;
        // right-anchored controls stay put; the controls after the filter group's end (Find's count, Clear) may
        // grow, so a control that only moves with the group's own width is the one allowed to change
        const d = Math.max(...r.map((v, i) => Math.abs(v - s[i])));
        const filterGrows = !!(e as Element).closest?.(".filter") && (step.filter !== undefined || step.find !== undefined);
        if (d > 0.5 && !filterGrows && e !== document.querySelector(".toolbar")) {
          worst = Math.max(worst, d);
          if (fail.length < 12) fail.push(`${w} ${JSON.stringify(step)}: ${(e as Element).tagName.toLowerCase()}.${(e as Element).className} moved ${d.toFixed(1)}`);
        }
      }
    }
  }
  out.compared = compared; out.worst = worst;
  h.width(1280); h.set({ view: "grid", cull: "none", filter: false, find: false }); await frames();
  out.layoutShift = shifts; out.shiftSources = shiftSeen.slice(0, 6);
  if (shifts !== 0) fail.push(`layout-shift ${shifts}`);

  // 3. the gap needs no script: strip every inline style but the column's size, reload the stylesheets
  const col = document.querySelector<HTMLElement>(".center")!;
  for (let p: HTMLElement | null = col.parentElement; p; p = p.parentElement) p.removeAttribute("style");
  for (const st of [...document.querySelectorAll<HTMLStyleElement>("style[data-vite-dev-id]")]) { const n = st.cloneNode(true) as HTMLStyleElement; st.replaceWith(n); }
  await frames();
  const g = geometry();
  if (Math.abs((g.gap as number) - 12) > 0.5) fail.push(`after a style reload the gap is ${g.gap}`);
  out.afterReload = g.gap;

  // 4. keyboard and names
  const bar = document.querySelector<HTMLElement>(".toolbar")!;
  const stops = items().filter((e) => e.tabIndex === 0);
  out.tabStops = stops.length;
  if (stops.length !== 1) fail.push(`${stops.length} tab stops in the bar`);
  const nameless = items().filter((e) => !accessibleName(e));
  if (nameless.length) fail.push(`${nameless.length} buttons without a name`);
  const swatches = [...bar.querySelectorAll<HTMLElement>(".swatch")].map(accessibleName);
  if (new Set(swatches).size !== swatches.length) fail.push(`label swatches share names: ${swatches.join(", ")}`);
  const first = items()[0]; first.focus();
  const seen = new Set<HTMLElement>([first]);
  const key = (el: HTMLElement, k: string) => el.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true }));
  const rtl = getComputedStyle(bar).direction === "rtl";
  for (let i = 0; i < 60; i++) { key(document.activeElement as HTMLElement, rtl ? "ArrowLeft" : "ArrowRight"); seen.add(document.activeElement as HTMLElement); }
  if (seen.size !== items().length) fail.push(`arrow keys reached ${seen.size} of ${items().length} controls`);
  key(document.activeElement as HTMLElement, "Home");
  if (document.activeElement !== items()[0]) fail.push("Home does not go to the first control");
  key(document.activeElement as HTMLElement, "End");
  if (document.activeElement !== items().at(-1)) fail.push("End does not go to the last control");
  // 5. the menus that stand in for hidden controls open, hold them, and close on Escape
  h.width(800); h.set({ view: "grid" }); await frames();
  for (const [sel, want] of [[".collapsed-labels", 6], [".collapsed-more", 2], [".collapsed-stars", 6], [".overflow", 3]] as const) {
    const btn = document.querySelector<HTMLElement>(`.toolbar ${sel} > button`);
    if (!btn || !btn.getClientRects().length) { fail.push(`${sel} is not shown at 800px`); continue; }
    btn.click(); await frames();
    const menu = document.querySelector(".toolbar-menu");
    const n = menu ? menu.querySelectorAll("button").length : 0;
    if (n < want) fail.push(`${sel} menu holds ${n} buttons, want ${want}`);
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })); await frames();
    if (document.querySelector(".toolbar-menu")) fail.push(`${sel} menu stays after Escape`);
  }
  out.items = items().length; out.dir = dir;
  out.ok = fail.length === 0;
  return out;
}
