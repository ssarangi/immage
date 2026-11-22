// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The no-layout-shift test for the critique lightbox. In every state of the
// spec's table it records the rect of every element and every text run inside
// the lightbox, applies each hover, focus, pin and selection it can (CSS :hover
// and :focus-visible are mirrored onto classes, as a script cannot hover),
// records again and reports the largest movement. verify/critique-shift.test.mjs
// runs it in headless Chrome and asserts that the movement is zero.

type H = { setState: (s: string) => void; setTab: (t: string) => void; setSel: (s: unknown) => void };
type R = { x: number; y: number; w: number; h: number };
type Snap = Map<Node, R> & { scroll?: number };

const SKIP = ".cull-label, .cull-bx, .cull-edge, .cull-sweep, .cull-row-pin";   // positioned out of flow by design
// timers, not rAF: headless Chrome's virtual time (--virtual-time-budget) does not run frames on its own
const frames = (n = 3) => new Promise<void>((done) => { setTimeout(done, 25 * n); });
const box = () => document.querySelector<HTMLElement>(".culling-lightbox")!;
const rect = (r: DOMRect): R => ({ x: r.left, y: r.top, w: r.width, h: r.height });

function snapshot(): Snap {
  const out: Snap = new Map();
  out.scroll = document.querySelector(".cull-pane")?.scrollTop ?? 0;
  const root = box();
  out.set(root, rect(root.getBoundingClientRect()));
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_ELEMENT | NodeFilter.SHOW_TEXT);
  for (let n = walker.nextNode(); n; n = walker.nextNode()) {
    if (n.nodeType === Node.ELEMENT_NODE) {
      const el = n as Element;
      if (el.closest(SKIP)) continue;
      out.set(n, rect(el.getBoundingClientRect()));
    } else if ((n.textContent ?? "").trim() && !n.parentElement?.closest(SKIP)) {
      const r = document.createRange(); r.selectNodeContents(n);
      out.set(n, rect(r.getBoundingClientRect()));
    }
  }
  return out;
}

const describe = (n: Node) => {
  const el = n.nodeType === Node.TEXT_NODE ? n.parentElement : (n as Element);
  return `${el?.tagName.toLowerCase()}.${(el?.className ?? "").toString().split(" ").join(".")}${n.nodeType === Node.TEXT_NODE ? " (text)" : ""}`;
};

const describeInPane = (n: Node) => {
  const el = n.nodeType === Node.TEXT_NODE ? n.parentElement : (n as Element);
  return !!el && !el.classList.contains("cull-pane") && !!el.closest(".cull-pane");
};

function diff(a: Snap, b: Snap) {
  let max = 0, worst = "", compared = 0;
  for (const [n, r] of a) {
    const s = b.get(n);
    if (!s) continue;
    compared++;
    // a badge hovered on the photo scrolls its row into view when it is out of sight: that is
    // the panel's scroll, not a shift, so the rows inside it are compared net of it
    const scrolled = describeInPane(n) ? (b.scroll ?? 0) - (a.scroll ?? 0) : 0;
    const d = Math.max(Math.abs(r.x - s.x), Math.abs(r.y - s.y - scrolled), Math.abs(r.w - s.w), Math.abs(r.h - s.h));
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
  st.textContent = css.join("\n") + "\n.culling-lightbox *, .culling-lightbox *::before, .culling-lightbox *::after { transition: none !important; animation: none !important; }\n/* layout is measured: a paint-only transform (a swatch's hover scale, a centring translate) is not a shift */\n.culling-lightbox *, .culling-lightbox *::before, .culling-lightbox *::after { transform: none !important; }";
  document.head.append(st);
}

const withClass = (el: Element, cls: string, on: boolean) => {
  for (let e: Element | null = el; e && e !== document.body; e = e.parentElement) { if (cls === "__fv" && e !== el) break; e.classList.toggle(cls, on); }
};
const pointer = (el: Element, type: string) => el.dispatchEvent(new PointerEvent(type, { bubbles: true, pointerType: "mouse" }));

type Result = { state: string; action: string; max: number; worst: string; compared: number };

export async function runShiftTest() {
  const h = (window as unknown as { __h: H }).__h;
  mirrorPseudoClasses();
  const results: Result[] = [];
  const fixed: Record<string, Record<string, R>> = {};
  const FIXED = [".culling-toolbar", ".culling-stage", ".culling-photo", ".cull-side", ".cull-tabs", ".culling-bottom-bar", ".cull-tab", ".culling-lightbox"];
  const sel = (hot: number | null, pinned: number | null, blur: boolean) => h.setSel({ hot, pinned, blur });
  const note = (state: string, action: string, base: Snap) => { const d = diff(base, snapshot()); results.push({ state, action, max: d.max, worst: d.worst, compared: d.compared }); };

  for (const state of ["result", "loading", "error", "errorkept", "nokey", "empty", "cull"]) {
    document.body.dataset.shiftState = state;
    h.setState(state); h.setTab(state === "cull" ? "cull" : "critique");
    const blur = state === "cull";
    sel(null, null, blur);
    await frames();
    // 1. the stage, photo, bars and tabs are where they were in every other state
    fixed[state] = {};
    for (const s of FIXED) { const el = document.querySelector(s); if (el) fixed[state][s] = rect(el.getBoundingClientRect()); }
    const base = snapshot();
    const lb = box();

    // 2. selection driven from the photo or a row: hover, pin, pin then hover another
    const regionCount = lb.querySelectorAll(".cull-pin").length;
    for (let i = 0; i < regionCount; i++) {
      sel(i, null, blur); await frames(); note(state, `hover region ${i + 1}`, base);
      sel(null, i, blur); await frames(); note(state, `pin region ${i + 1}`, base);
      for (let j = 0; j < regionCount; j++) if (j !== i) { sel(j, i, blur); await frames(); note(state, `pin ${i + 1}, hover ${j + 1}`, base); }
      sel(null, null, blur); await frames();
    }
    sel(null, null, !blur); await frames(); note(state, "toggle softest outline", base);
    sel(null, null, blur); await frames();

    // 3. real pointer events and CSS hover on the badges and rows
    for (const pin of Array.from(lb.querySelectorAll<HTMLElement>(".cull-pin"))) {
      pointer(pin, "pointerover"); withClass(pin, "__hover", true); await frames(); note(state, `badge pointer+:hover ${pin.textContent}`, base);
      withClass(pin, "__hover", false); pointer(pin, "pointerout"); await frames();
    }
    for (const row of Array.from(lb.querySelectorAll<HTMLElement>(".cull-row"))) {
      pointer(row, "pointerover"); withClass(row, "__hover", true); await frames(); note(state, `row pointer+:hover ${row.id}`, base);
      withClass(row, "__hover", false); pointer(row, "pointerout"); await frames();
      const main = row.querySelector<HTMLElement>(".cull-row-main")!;
      main.focus(); withClass(main, "__fv", true); await frames(); note(state, `row focus ${row.id}`, base);
      withClass(main, "__fv", false); main.blur(); await frames();
      main.click(); await frames(); note(state, `row click (pin) ${row.id}`, base);
      main.click(); await frames();
    }

    // 4. every other control: hover and keyboard focus
    for (const el of Array.from(lb.querySelectorAll<HTMLElement>("button, [role=tab], [role=switch], a, input"))) {
      if (el.closest(".cull-row, .cull-pin") || el.classList.contains("cull-pin")) continue;
      const name = `${el.tagName.toLowerCase()}.${el.className.toString().split(" ").join(".")} "${(el.getAttribute("aria-label") ?? el.textContent ?? "").trim().slice(0, 24)}"`;
      withClass(el, "__hover", true); await frames(); note(state, `hover ${name}`, base); withClass(el, "__hover", false);
      el.focus(); withClass(el, "__fv", true); await frames(); note(state, `focus ${name}`, base); withClass(el, "__fv", false); el.blur();
    }
    // the stage's hover shows the previous and next buttons
    const stage = lb.querySelector(".culling-stage")!;
    withClass(stage, "__hover", true); await frames(); note(state, "hover stage (arrows)", base); withClass(stage, "__hover", false);
    await frames();
  }

  // the Critique tab and the Cull tab share one stage
  h.setState("result"); h.setTab("cull"); sel(null, null, false); await frames();
  const base = snapshot(), tabCull: Record<string, R> = {};
  for (const s of FIXED) { const el = document.querySelector(s); if (el) tabCull[s] = rect(el.getBoundingClientRect()); }
  fixed["result-on-cull-tab"] = tabCull;
  void base;

  // fixed regions: identical across every state
  let fixedMax = 0, fixedWorst = "";
  const ref = fixed.result;
  for (const [state, rs] of Object.entries(fixed)) for (const s of FIXED) {
    const a = ref[s], b = rs[s];
    if (!a || !b) continue;
    const d = Math.max(Math.abs(a.x - b.x), Math.abs(a.y - b.y), Math.abs(a.w - b.w), Math.abs(a.h - b.h));
    if (d > fixedMax) { fixedMax = d; fixedWorst = `${s} in ${state}`; }
  }
  const max = results.reduce((m, r) => Math.max(m, r.max), 0);
  return { ok: max === 0 && fixedMax === 0, maxDelta: max, fixedMaxDelta: Math.round(fixedMax * 100) / 100, fixedWorst,
    interactions: results.length, compared: results.reduce((m, r) => m + r.compared, 0), failures: results.filter((r) => r.max > 0), results };
}
