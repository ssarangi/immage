// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The no-layout-shift test for the Catalog card (spec section 12). In the harness page it checks:
//  1. across every tall state (finding, reading at 1/43/99%, thumbnails, paused, done, problems) the
//     source rows, the card and its header, phase rows, bars and footer have the identical box; the
//     short states (offline, idle, never indexed) share theirs;
//  2. within a state, hover and keyboard focus on every control move nothing (CSS :hover and
//     :focus-visible are mirrored onto classes, as a script cannot hover);
//  3. the Rebuild row of Preferences keeps its box when it asks for the second click (view=prefs);
//  4. the folder tree (view=tree): expanding moves only the rows below by exactly the rows added, and a state
//     change, a count from 12 to 1,000,000, the first folder arriving and an error triangle appearing move
//     nothing; rows are 28 px; the tree's roles and ARIA numbers are right; a 110,101-folder tree draws a window.
// verify/catalog-shift.test.mjs runs it in headless Chrome with ?shift=1 and asserts zero movement.
import { SHORT, TALL } from "./catalogFixtures";

type R = { x: number; y: number; w: number; h: number };
type Snap = Map<Node, R>;
const frames = (n = 3) => new Promise<void>((done) => { setTimeout(done, 25 * n); });
const page = () => document.querySelector<HTMLElement>("[data-harness-page]")!;
const rect = (r: DOMRect): R => ({ x: r.left, y: r.top + (page()?.scrollTop ?? 0), w: r.width, h: r.height });
// bars fill and text runs change with the data; their boxes are fixed and measured on the containers
const SKIP = ".cat-bar > i, .twisty > svg, .twisty > svg *";

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
const describe = (n: Node) => {
  const el = n as Element;
  return `${el.tagName.toLowerCase()}.${(el.className ?? "").toString().split(" ").join(".")}${el.closest("[data-key]") ? ` in ${el.closest("[data-key]")!.getAttribute("data-key")}` : ""}`;
};
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
const ROWS = ['[data-key="src:1:"]', '[data-key="src:1:"] .cat-slot', '[data-key="src:1:"] .cat-state',
  '[data-key="src:2:"]', '[data-key="src:3:"]', '[data-key="src:4:"]', '[data-key="src:5:"]', ".cat-volume"];
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
  for (const el of Array.from(page().querySelectorAll<HTMLElement>("button, a, input, [role=button], [role=treeitem]"))) {
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

type Failure = { what: string; delta: number; worst: string };
type TreeH = H & {
  expand(keys: string[], on?: boolean): void; select(key: string | null): void; scrollTo(key: string): void;
  setCount(id: number, rel: string, parent: string, count: number): void;
  addFolder(id: number, parent: string, name: string, count: number, status?: string): void;
  clear(id: number): void; setStatus(id: number, rel: string, parent: string, status: string): void; grow(id: number): void;
};
// the footer is sticky (its place depends on the scroll) and its own fixed boxes are tested with the card
const TREE_SKIP = ".cat-footer, .cat-add, .panel-title";
// the containers around the rows grow with the rows; it is the rows that must not move
const GROWS = (el: Element) => el.matches(".catalog-panel, .cat-body, .ftree") || el.contains(document.querySelector(".ftree"));
const treeSnap = (): Snap => {
  const out = snapshot();
  for (const n of Array.from(out.keys())) if ((n as Element).closest(TREE_SKIP) || GROWS(n as Element)) out.delete(n);
  return out;
};

/** Nodes in both snapshots: above `from` they must be where they were, at or below it exactly `by` lower. */
function shifted(a: Snap, b: Snap, from: number, by: number, label: string, failures: Failure[]) {
  let compared = 0;
  for (const [n, r] of a) {
    const s = b.get(n);
    if (!s) continue;
    compared++;
    const want = r.y >= from ? by : 0;
    const d = Math.max(Math.abs(r.x - s.x), Math.abs(r.w - s.w), Math.abs(r.h - s.h), Math.abs(s.y - r.y - want));
    if (d > 0.01) { failures.push({ what: label, delta: Math.round(d * 100) / 100, worst: describe(n) }); break; }
  }
  return compared;
}

async function treeTest(failures: Failure[]) {
  const h = (window as unknown as { __h: TreeH }).__h;
  const checks: Record<string, unknown> = {};
  let compared = 0;
  const key = (rel: string, id = 1) => `src:${id}:${rel}`;
  const rowTop = (k: string) => (document.querySelector(`[data-key="${k}"]`) as HTMLElement | null)?.getBoundingClientRect().top ?? NaN;
  const pageTop = () => page().scrollTop;

  // 1. expanding a folder moves the rows below it by exactly the rows added, and nothing else
  h.set("idle"); await frames();
  for (const [what, k, rows] of [["expand 2018", key("2018"), 2], ["expand 2019", key("2019"), 4]] as const) {
    const before = treeSnap();
    const at = rowTop(k) + pageTop() + 28;      // the first added row's top
    h.expand([k]); await frames();
    const after = treeSnap();
    compared += shifted(before, after, at, rows * 28, what, failures);
    const before2 = treeSnap();
    h.expand([k], false); await frames();
    compared += shifted(after, treeSnap(), at, -rows * 28, `${what} back`, failures);
    void before2;
  }
  h.expand([key("2019"), key("2019/Iceland")]); await frames();

  // 2. a source going from online to indexing to paused to offline and back moves no row
  const states = ["idle", "reading", "thumbs", "paused", "offline", "partial", "idle"];
  const base = treeSnap();
  const rowsOf = (m: Snap) => { const o = new Map<Node, R>(); for (const [n, r] of m) if ((n as Element).closest(".folder-row, .ftree-other")) o.set(n, r); return o; };
  const baseRows = rowsOf(base);
  let stateDelta = 0;
  for (const st of states) {
    h.set(st); await frames();
    const d = diff(baseRows, rowsOf(treeSnap()));
    compared += d.compared; stateDelta = Math.max(stateDelta, d.max);
    if (d.max > 0) failures.push({ what: `state ${st}`, delta: d.max, worst: d.worst });
  }
  checks.stateDelta = stateDelta;
  h.set("idle"); await frames();

  // 3. a count from 12 to 999 to 1,000,000 and back moves no box of any row
  const day = "2019/Iceland";
  let countDelta = 0;
  const reference = treeSnap();
  for (const n of [12, 999, 99_999, 100_000, 1_000_000, 12]) {
    h.setCount(1, `${day}/Day 1`, day, n); await frames();
    const d = diff(reference, treeSnap());
    compared += d.compared; countDelta = Math.max(countDelta, d.max);
    if (d.max > 0) failures.push({ what: `count ${n}`, delta: d.max, worst: d.worst });
  }
  checks.countDelta = countDelta;
  // the count slot says the compact number above 99,999 and the whole number in its tooltip
  h.setCount(1, `${day}/Day 1`, day, 1_234_567); await frames();
  const big = document.querySelector<HTMLElement>(`[data-key="${key(`${day}/Day 1`)}"] .cat-count`);
  checks.compactCount = big?.textContent ?? null;
  if (!big || big.textContent === "1,234,567" || !big.getAttribute("aria-label")?.includes("1,234,567")) failures.push({ what: "count above 99,999 is not compact, or lost its full number", delta: 1, worst: String(big?.textContent) });
  h.setCount(1, `${day}/Day 1`, day, 12); await frames();

  // 5. a folder's error triangle appears in its own slot
  const reference2 = treeSnap();
  h.setStatus(1, "2019/Misc", "2019", "denied"); await frames();
  const d5 = diff(reference2, treeSnap());
  compared += d5.compared;
  if (d5.max > 0) failures.push({ what: "an error triangle appeared and moved something", delta: d5.max, worst: d5.worst });
  const tri = document.querySelector(`[data-key="${key("2019/Misc")}"] .cat-state`);
  if (!tri?.querySelector("svg") || !tri.getAttribute("aria-label")) failures.push({ what: "the error triangle has no shape or no label", delta: 1, worst: String(tri?.outerHTML) });
  checks.triangle = tri?.getAttribute("aria-label") ?? null;
  checks.hoverFocus = await interactions(failures, "tree");

  // 6. the rows are 28 px, and the tree has its roles
  const heights = new Set(Array.from(document.querySelectorAll<HTMLElement>(".folder-row, .ftree-other")).map((e) => Math.round(e.getBoundingClientRect().height * 100) / 100));
  checks.rowHeights = [...heights];
  if (heights.size !== 1 || !heights.has(28)) failures.push({ what: `rows are not all 28 px high: ${[...heights].join(", ")}`, delta: 1, worst: "" });
  const tree = document.querySelector('[role="tree"]');
  const items = Array.from(document.querySelectorAll<HTMLElement>('[role="treeitem"]'));
  const problems: string[] = [];
  if (!tree?.getAttribute("aria-label")) problems.push("tree has no name");
  if (items.filter((e) => e.tabIndex === 0).length !== 1) problems.push(`${items.filter((e) => e.tabIndex === 0).length} rows on the tab order`);
  for (const e of items) {
    for (const a of ["aria-level", "aria-setsize", "aria-posinset"]) if (!e.getAttribute(a)) problems.push(`${e.dataset.key}: no ${a}`);
    const kids = e.querySelector(".twisty:not(.none)") !== null;
    if (kids !== (e.getAttribute("aria-expanded") !== null)) problems.push(`${e.dataset.key}: aria-expanded does not match its chevron`);
    if (!e.closest('[role="tree"]')) problems.push(`${e.dataset.key}: outside the tree`);
    if (e.querySelector(".cat-count") && !e.querySelector(".cat-count")!.getAttribute("aria-label")) problems.push(`${e.dataset.key}: count has no label`);
  }
  const lvl = (k: string) => document.querySelector(`[data-key="${k}"]`)?.getAttribute("aria-level");
  if (lvl(key("")) !== "1" || lvl(key("2019")) !== "2" || lvl(key("2019/Iceland")) !== "3") problems.push("aria-level does not follow the depth");
  checks.aria = { items: items.length, problems };
  for (const p of problems) failures.push({ what: `aria: ${p}`, delta: 1, worst: p });

  // 7. keys: down moves focus, right opens, left closes (the tree's own handler, on real key events)
  const first = document.querySelector<HTMLElement>(`[data-key="${key("")}"]`)!;
  first.focus(); await frames();
  const press = async (k: string) => { (document.activeElement as HTMLElement).dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true })); await frames(); };
  await press("ArrowDown");
  const afterDown = (document.activeElement as HTMLElement).dataset.key;
  await press("ArrowDown");
  const second = (document.activeElement as HTMLElement).dataset.key;
  const rtl = document.documentElement.dir === "rtl";   // Left and Right swap in a right-to-left tree
  await press(rtl ? "ArrowLeft" : "ArrowRight");     // 2018 is a closed folder: it opens
  const opened = document.querySelector(`[data-key="${second}"]`)?.getAttribute("aria-expanded");
  await press(rtl ? "ArrowRight" : "ArrowLeft");
  const closed = document.querySelector(`[data-key="${second}"]`)?.getAttribute("aria-expanded");
  await press("End");
  const last = (document.activeElement as HTMLElement).dataset.key;
  checks.keys = { afterDown, second, opened, closed, last };
  if (afterDown !== key("2017") || second !== key("2018") || opened !== "true" || closed !== "false" || !last?.startsWith("src:4")) failures.push({ what: "keyboard navigation", delta: 1, worst: JSON.stringify(checks.keys) });
  // 4. a source found while indexing: the first folder takes the "finding folders" line's place
  h.set("finding"); await frames(); h.clear(1); await frames();
  const message = document.querySelector(".folder-message");
  checks.finding = message?.textContent ?? null;
  if (!message) failures.push({ what: "no 'finding folders' line while a source is indexed with no folder yet", delta: 1, worst: "" });
  const lineBox = message ? rect(message.getBoundingClientRect()) : null;
  const waiting = treeSnap();
  h.addFolder(1, "", "first", 3); await frames();
  const arrived = document.querySelector(`[data-key="${key("first")}"]`);
  if (!arrived) failures.push({ what: "the first folder did not appear", delta: 1, worst: "" });
  else if (lineBox) {
    const r = rect(arrived.getBoundingClientRect());
    const d = Math.max(Math.abs(r.y - lineBox.y), Math.abs(r.h - lineBox.h));
    if (d > 0.01) failures.push({ what: "the first folder is not where the line was", delta: d, worst: "first" });
    checks.arrivalDelta = d;
  }
  compared += shifted(waiting, treeSnap(), 1e9, 0, "first folder arrives", failures);
  for (let i = 0; i < 3; i++) { h.grow(1); await frames(); }   // counts grow in place while the scan reads on
  h.set("idle"); await frames();

  return { ok: failures.length === 0, compared, checks, failures };
}

// a tree of 110,101 folders, every folder open: only a window of rows is in the page, and scrolling moves it
async function bigTest(failures: Failure[]) {
  const h = (window as unknown as { __h: TreeH }).__h;
  h.set("idle"); await frames(6);
  const count = () => document.querySelectorAll('[role="treeitem"]').length;
  const box = document.querySelector<HTMLElement>('[role="tree"]')!;
  const out: Record<string, unknown> = { drawnAtTop: count(), scrollHeight: box.scrollHeight, clientHeight: box.clientHeight };
  const firstKey = () => (document.querySelector('[role="treeitem"]') as HTMLElement | null)?.dataset.key;
  const top = firstKey();
  const t0 = performance.now();
  const paint = () => new Promise<void>((done) => requestAnimationFrame(() => requestAnimationFrame(() => done())));
  box.scrollTop = 28 * 50_000; await paint(); await frames(4);
  out.scrollMs = Math.round(performance.now() - t0);
  out.scrollTopSet = box.scrollTop; out.firstAfter = firstKey();
  out.drawnMiddle = count(); out.topKeyMoved = firstKey() !== top;
  box.scrollTop = box.scrollHeight; await paint(); await frames(4);
  out.drawnEnd = count();
  const items = Array.from(document.querySelectorAll<HTMLElement>('[role="treeitem"]'));
  out.lastKey = items[items.length - 1]?.dataset.key;
  if (count() > 60) failures.push({ what: `${count()} rows in the page at the end of a 110,101-row tree`, delta: count(), worst: "" });
  if ((out.drawnAtTop as number) > 60 || (out.drawnMiddle as number) > 60) failures.push({ what: "more than 60 rows drawn", delta: 1, worst: JSON.stringify(out) });
  if (box.scrollHeight < 28 * 110_000) failures.push({ what: `scroll height ${box.scrollHeight} is not the whole list's`, delta: 1, worst: "" });
  if (!out.topKeyMoved) failures.push({ what: "scrolling did not move the window", delta: 1, worst: "" });
  // the tab stop stays mounted when it scrolls out of the window
  const tabbable = Array.from(document.querySelectorAll<HTMLElement>('[role="treeitem"]')).filter((e) => e.tabIndex === 0).length;
  out.tabbableMounted = tabbable;
  if (tabbable !== 1) failures.push({ what: `${tabbable} rows on the tab order in the big tree`, delta: 1, worst: "" });
  return { ok: failures.length === 0, big: out, failures };
}

export async function runShiftTest() {
  const h = (window as unknown as { __h: H }).__h;
  mirrorPseudoClasses();
  const failures: { what: string; delta: number; worst: string }[] = [];
  let interactionsRun = 0, compared = 0, maxDelta = 0, fixedMax = 0, fixedWorst = "";
  const view = new URLSearchParams(location.search).get("view") ?? "card";

  if (new URLSearchParams(location.search).get("fold") === "1") return headerTest(failures);
  if (view === "tree") return new URLSearchParams(location.search).get("tree") === "big" ? bigTest(failures) : treeTest(failures);

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
