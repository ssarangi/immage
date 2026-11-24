// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The no-layout-shift, keyboard and scale test for People. It records the rect of every element of the view
// and compares it after: a click, a ctrl-click, a shift-click, select all, a marquee, hover, the name box
// opening and filtering, naming (the Undo line appearing and going), a scan's progress, pause, failure and
// idle, new groups arriving, and the consent card's every step. It drives the keyboard-only path (arrows, a
// letter, a name, Enter, Tab), scrolls every section to count what is mounted, and audits the stylesheet: no
// hover, focus or selected rule may touch a property that sizes or places a box. A PerformanceObserver counts
// the browser's own layout-shift entries. verify/people-shift.test.mjs runs it in headless Chrome and asserts
// zero; break=1 is the negative control.
import { clearUndo, peopleState, refreshAll, setPeople } from "../src/peopleStore";

type R = { x: number; y: number; w: number; h: number };
type H = { calls: string[]; scan: { offline: number; net: number; failed: number; enabled: boolean; installed: boolean; scanned: number; pending: number | null; health: string; failure: string | null; paused: boolean };
           db: () => { faces: { id: number; person: number | null; state: string; cluster: number | null; maybe: number | null; path: string }[] };
           setPack: (m: string) => void; tick: () => void };
const h = () => (window as unknown as { __h: H }).__h;
const frames = (n = 3) => new Promise<void>((done) => { setTimeout(done, 25 * n); });
const rect = (r: DOMRect): R => ({ x: Math.round(r.left * 100) / 100, y: Math.round(r.top * 100) / 100, w: Math.round(r.width * 100) / 100, h: Math.round(r.height * 100) / 100 });
const label = (el: Element) => (el.tagName ? `${el.tagName.toLowerCase()}.${String(el.className).split(" ").join(".")}` : "text");
const $ = <T extends HTMLElement>(s: string) => document.querySelector<T>(s);
const $$ = <T extends HTMLElement>(s: string) => Array.from(document.querySelectorAll<T>(s));

function snapshot(root: Element): Map<Element, R> {
  const out = new Map<Element, R>();
  out.set(root, rect(root.getBoundingClientRect()));
  for (const el of root.querySelectorAll("*")) if (!el.closest("img, svg, canvas")) out.set(el, rect(el.getBoundingClientRect()));
  return out;
}
function diff(a: Map<Element, R>, b: Map<Element, R>, ignore = "", self = "") {
  let max = 0, worst = "", compared = 0;
  const failures: string[] = [];
  for (const [el, r] of a) {
    const s = b.get(el);
    if (!s || (ignore && el.closest(ignore)) || (self && el.matches(self))) continue;
    compared++;
    const d = Math.max(Math.abs(r.x - s.x), Math.abs(r.y - s.y), Math.abs(r.w - s.w), Math.abs(r.h - s.h));
    if (d > 0) failures.push(`${label(el)} ${d}`);
    if (d > max) { max = d; worst = `${label(el)} ${JSON.stringify(r)} > ${JSON.stringify(s)}`; }
  }
  return { max, worst, compared, failures: failures.slice(0, 6) };
}

const SIZING = /^(width|height|min-width|max-width|min-height|max-height|margin.*|padding.*|border(-\w+)*-width|border-width|border|font-size|font-weight|letter-spacing|line-height|display|position|top|left|right|bottom|inset.*|flex.*|gap|row-gap|column-gap|grid.*|content|text-transform)$/;
const STATE = /:hover|:focus|:active|\.active|\[data-active|\[aria-(selected|checked|current|pressed|expanded)/;
function auditStyles(): string[] {
  const bad: string[] = [];
  for (const sheet of Array.from(document.styleSheets)) {
    let rules: CSSRuleList;
    try { rules = sheet.cssRules; } catch { continue; }
    const walk = (list: CSSRuleList) => {
      for (const rule of Array.from(list)) {
        if (rule instanceof CSSMediaRule || rule instanceof CSSSupportsRule) { walk(rule.cssRules); continue; }
        if (!(rule instanceof CSSStyleRule)) continue;
        const sel = rule.selectorText;
        if (!/\.people|\.ptile|\.pcrop|\.pcap|\.pmarquee/.test(sel) || !STATE.test(sel)) continue;
        for (const prop of Array.from(rule.style)) if (SIZING.test(prop)) bad.push(`${sel} { ${prop} }`);
      }
    };
    walk(rules);
  }
  return bad;
}

// CSS :hover cannot be driven from a script, so its rules are mirrored onto a class
function mirrorHover() {
  const css: string[] = [];
  for (const sheet of Array.from(document.styleSheets)) {
    let rules: CSSRuleList;
    try { rules = sheet.cssRules; } catch { continue; }
    for (const rule of Array.from(rules)) if (rule.cssText.includes(":hover")) css.push(rule.cssText.replaceAll(":hover", ".__hover"));
  }
  const st = document.createElement("style");
  st.textContent = css.join("\n") + "\n.people-view *, .people-view *::before { transition: none !important; animation: none !important; }";
  document.head.append(st);
}

const press = (el: Element, key: string, o: KeyboardEventInit = {}) => el.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...o }));
const click = (el: Element, o: MouseEventInit = {}) => { el.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, ...o })); };
function typeInto(input: HTMLInputElement, value: string) {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}
const tiles = () => $$(".ptile");
const grid = () => $<HTMLElement>(".people-scroll")!;

async function consentRun(out: Record<string, unknown>, fail: (m: string) => void) {
  const card = () => $(".people-consent")!;
  const steps: Record<string, R> = {};
  const all: Map<Element, R>[] = [];
  const SKIP = ".people-slot > *, .people-bar-track i, .people-consent .check, .people-radio";
  for (const m of ["off", "download", "installed"]) {
    h().setPack(m);
    await frames(3);
    steps[m] = rect(card().getBoundingClientRect());
    all.push(snapshot(card()));
  }
  const d1 = diff(all[0], all[1], SKIP), d2 = diff(all[1], all[2], SKIP);
  out.consent = { rects: steps, max: Math.max(d1.max, d2.max), worst: d1.worst || d2.worst, compared: d1.compared + d2.compared };
  const button = () => $<HTMLButtonElement>(".people-consent .big")!;
  if (!button().disabled) fail("consent: Turn on is enabled before the box is ticked");
  const box = $<HTMLInputElement>(".people-consent .check input")!;
  const before = snapshot(card());
  click(box);
  await frames(2);
  if (button().disabled) fail("consent: Turn on stays disabled after the box is ticked");
  const d3 = diff(before, snapshot(card()), SKIP);
  (out.consent as { max: number }).max = Math.max((out.consent as { max: number }).max, d3.max);
  // scope: the radio for the open folder, then back
  click($$<HTMLInputElement>(".people-consent input[type=radio]")[1]);
  await frames(2);
  const d4 = diff(before, snapshot(card()), SKIP);
  (out.consent as { max: number }).max = Math.max((out.consent as { max: number }).max, d4.max);
  click(button());
  await frames(8);
  out.turnedOn = { calls: h().calls.filter((c) => c.startsWith("enabled") || c.startsWith("scanFolder")), rail: !!$(".people-rail") };
  if (!h().calls.includes("enabled:true")) fail("consent: Turn on did not turn People on");
  if (!h().calls.some((c) => c.startsWith("scanFolder:"))) fail("consent: the folder scope did not scan the folder");
  if (!$(".people-rail")) fail("consent: the workspace did not replace the card");
}

export async function runPeopleShift() {
  const out: Record<string, unknown> = { failures: [] as string[] };
  const fail = (m: string) => (out.failures as string[]).push(m);
  let shifts = 0, supported = false;
  const detail: string[] = [];
  const pending: PerformanceEntry[] = [];
  let observer: PerformanceObserver | null = null;
  try { observer = new PerformanceObserver((l) => { pending.push(...l.getEntries()); }); observer.observe({ type: "layout-shift", buffered: true }); supported = true; } catch { /* not supported */ }
  const drain = (keep: boolean, phase = "") => {
    pending.push(...(observer?.takeRecords() ?? []));
    for (const e of pending.splice(0)) {
      const v = e as unknown as { value: number; hadRecentInput: boolean; sources?: { node?: Element; previousRect: DOMRect; currentRect: DOMRect }[] };
      if (!keep || v.hadRecentInput) continue;
      shifts += v.value;
      if (detail.length < 6) detail.push(`${phase} ${v.value.toFixed(4)} ${(v.sources ?? []).map((x) => `${x.node ? label(x.node) : "?"} x${Math.round(x.previousRect.x)}>${Math.round(x.currentRect.x)} y${Math.round(x.previousRect.y)}>${Math.round(x.currentRect.y)}`).join(", ")}`);
    }
  };
  mirrorHover();
  await frames(5);
  drain(false);
  out.styleOffenders = auditStyles();
  if ((out.styleOffenders as string[]).length) fail(`state rules touch a size: ${(out.styleOffenders as string[]).slice(0, 3).join("; ")}`);

  if (!peopleState().status?.enabled) {
    await consentRun(out, fail);
    drain(true);
    out.layoutShift = { supported, value: Math.round(shifts * 10000) / 10000, detail };
    const cm = (out.consent as { max: number }).max;
    out.maxDelta = cm;
    out.ok = (out.failures as string[]).length === 0 && cm === 0;
    return out;
  }

  const view = $(".people-view")!;
  const FIXED = [".people-strip", ".people-bar", ".people-selbar", ".people-body", ".people-rail", ".people-main", ".people-scroll", ".people-inspector", ".people-undo"];
  const fixed = () => Object.fromEntries(FIXED.map((s) => [s, $(s) ? rect($(s)!.getBoundingClientRect()) : null]));
  const rows0 = fixed();
  out.fixed = rows0;
  const checkFixed = (where: string) => {
    const now = fixed();
    for (const s of FIXED) if (JSON.stringify(now[s]) !== JSON.stringify(rows0[s])) fail(`${where}: ${s} moved ${JSON.stringify(rows0[s])} -> ${JSON.stringify(now[s])}`);
  };
  const worst: Record<string, number> = {};
  const note = (name: string, d: ReturnType<typeof diff>) => {
    worst[name] = Math.max(worst[name] ?? 0, d.max);
    if (d.max > 0) fail(`${name}: ${d.worst} moved ${d.max} (${d.failures.join("; ")})`);
    out.compared = ((out.compared as number) ?? 0) + d.compared;
  };

  const phase = (n: string) => { document.body.dataset.phase = n; };
  phase("selection");
  // 1. selection: click, ctrl-click, shift-click, select all, clear, a marquee, hover
  let base = snapshot(view);
  const t = tiles();
  const step = async (name: string, act: () => void, ignore = "") => { act(); await frames(3); note(name, diff(base, snapshot(view), ignore)); checkFixed(name); };
  await step("click", () => click(t[0]));
  await step("ctrl-click", () => click(t[2], { ctrlKey: true }));
  await step("shift-click", () => click(t[4], { shiftKey: true }));
  const picked = tiles().filter((x) => x.getAttribute("aria-selected") === "true").length;
  if (picked < 3) fail(`selection: only ${picked} tiles selected after click, ctrl-click and shift-click`);
  await step("select all", () => press(grid(), "a", { ctrlKey: true }));
  const all = tiles().filter((x) => x.getAttribute("aria-selected") === "true").length;
  if (all < 10) fail(`selection: select all selected ${all} mounted tiles`);
  await step("clear", () => press(grid(), "Escape"));
  const r = grid().getBoundingClientRect();
  grid().dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, button: 0, clientX: r.left + 4, clientY: r.top + 4, pointerId: 7 }));
  grid().dispatchEvent(new PointerEvent("pointermove", { bubbles: true, clientX: r.left + 480, clientY: r.top + Math.min(300, r.height - 60), pointerId: 7 }));
  await frames(3);
  const marquee = !!$(".pmarquee");
  note("marquee", diff(base, snapshot(view), ".pmarquee"));
  grid().dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerId: 7 }));
  await frames(2);
  const swept = tiles().filter((x) => x.getAttribute("aria-selected") === "true").length;
  out.marquee = { drawn: marquee, selected: swept };
  if (!marquee || swept < 2) fail(`marquee: drew ${marquee}, selected ${swept}`);
  press(grid(), "Escape");
  await frames(2);
  base = snapshot(view);
  for (const target of [tiles()[1], $(".people-rail-item")!, $(".people-inspector .people-btn")!]) {
    target.classList.add("__hover");
    await frames(2);
    note("hover", diff(base, snapshot(view)));
    target.classList.remove("__hover");
  }
  drain(true, "selection");

  phase("naming");
  // 2. the name box: opening floats over the grid, typing filters it, Escape closes it
  click(tiles().find((x) => x.dataset.kind === "group") ?? tiles()[0]);
  await frames(2);
  base = snapshot(view);
  press(grid(), "n");
  await frames(3);
  const box = $(".people-name");
  if (!box) fail("naming: N did not open the name box");
  note("name box opens", diff(base, snapshot(view), ".people-name"));
  const input = $<HTMLInputElement>(".people-name input");
  if (input) { typeInto(input, "ma"); await frames(2); note("name box filters", diff(base, snapshot(view), ".people-name")); press(input, "ArrowDown"); await frames(2); }
  out.options = $$(".people-name li").map((x) => x.textContent);
  if (input) { press(input, "Escape"); await frames(2); }
  if ($(".people-name")) fail("naming: Escape left the name box open");
  note("name box closes", diff(base, snapshot(view)));
  const calls0 = h().calls.length;
  if (h().calls.length !== calls0 || h().calls.some((c) => c.startsWith("assign") || c.startsWith("create"))) fail("naming: Escape changed something");

  phase("keyboard");
  // 3. the keyboard-only path: arrows to a group, a letter, a name, Enter; then N, a new name, Tab
  grid().focus();
  press(grid(), "Home");
  await frames(2);
  const walk: string[] = [];
  for (let i = 0; i < 400; i++) {
    const active = document.getElementById(grid().getAttribute("aria-activedescendant") ?? "");
    if (walk.length < 6) walk.push(`${grid().getAttribute("aria-activedescendant")}:${active?.dataset.kind}`);
    if (active?.dataset.kind === "group") break;
    press(grid(), "ArrowRight");
    await frames(1);
  }
  await frames(2);
  const target = document.getElementById(grid().getAttribute("aria-activedescendant") ?? "");
  const startIndex = Number(target?.dataset.tile ?? -1);
  if (target?.dataset.kind !== "group") fail(`keyboard: arrows did not reach a group (${walk.join(" ")}; section ${$(".people-rail-item[aria-selected=true]")?.textContent})`);
  const countBefore = $$(".ptile[data-kind=group]").length;
  base = snapshot(view);
  press(grid(), "d");
  await frames(3);
  let kbd = $<HTMLInputElement>(".people-name input");
  if (!kbd) fail("keyboard: a letter did not open the name box");
  if (kbd && kbd.value !== "d") fail(`keyboard: the box opened with "${kbd.value}", not the letter`);
  if (kbd) { typeInto(kbd, "Dad"); await frames(2); press(kbd, "Enter"); }
  await frames(6);
  const assigned = h().calls.find((c) => c.startsWith("assign:1:"));
  if (!assigned) fail(`keyboard: Enter did not name the group Dad (${h().calls.join(",")})`);
  const undoText = $(".people-undo-text")?.textContent ?? "";
  if (!/Dad/.test(undoText)) fail(`keyboard: the Undo line says "${undoText}"`);
  const next = document.getElementById(grid().getAttribute("aria-activedescendant") ?? "");
  out.keyboard = { startIndex, nextIndex: Number(next?.dataset.tile ?? -1), undo: undoText, assigned, groupsBefore: countBefore, groupsAfter: $$(".ptile[data-kind=group]").length };
  if (Number(next?.dataset.tile ?? -1) !== startIndex) fail("keyboard: the focus did not move on to the next group");
  note("naming and the Undo line appear", diff(base, snapshot(view), ".people-canvas, .people-canvas *, .people-selbar > *"));
  checkFixed("naming");
  press(grid(), "n");
  await frames(3);
  kbd = $<HTMLInputElement>(".people-name input");
  if (kbd) { typeInto(kbd, "Zed Test"); await frames(2); press(kbd, "Tab"); }
  await frames(6);
  if (!h().calls.some((c) => c.startsWith("create:Zed Test"))) fail("keyboard: Tab did not create Zed Test");
  if (!$(".people-name")) fail("keyboard: Tab did not open the name box for the next group");
  press($<HTMLInputElement>(".people-name input") ?? grid(), "Escape");
  await frames(2);
  out.undoAfterTab = $(".people-undo-text")?.textContent ?? "";

  phase("undo");
  // 4. the Undo line: Undo takes the change back, and it goes after its time
  base = snapshot(view);
  const created = h().calls.length;
  click($(".people-undo .people-btn")!);
  await frames(8);
  if (!h().calls.slice(created).includes("undo")) fail("undo: the button did not call undo");
  note("undo", diff(base, snapshot(view), ".people-canvas, .people-canvas *, .people-selbar > *"));
  clearUndo();
  await frames(3);
  checkFixed("undo line gone");
  drain(false);   // Undo puts the tile back: on purpose

  await refreshAll(true); await frames(4);
  drain(false);   // naming and Undo reflow the tiles on purpose
  phase("scan");
  // 5. the scan: progress ticks, pause, a failure, idle: only the strip's own text may change
  base = snapshot(view);
  const strip = ".people-strip > *";
  h().scan.pending = 4000; h().scan.scanned = 100;
  await refreshAll(true); await frames(3);
  note("scan starts", diff(base, snapshot(view), strip));
  for (let i = 0; i < 5; i++) { h().tick(); await frames(1); note("scan tick", diff(base, snapshot(view), strip)); }
  out.stripText = $(".people-strip-text")?.textContent;
  if (!/Finding faces/.test(out.stripText as string)) fail(`scan: the strip says "${out.stripText}"`);
  h().scan.paused = true; await refreshAll(true); await frames(3);
  note("scan paused", diff(base, snapshot(view), strip));
  h().scan.paused = false; h().scan.health = "blocked"; h().scan.failure = "the engine stopped"; await refreshAll(true); await frames(3);
  note("scan blocked", diff(base, snapshot(view), strip));
  h().scan.health = "ok"; h().scan.failure = null; h().scan.pending = 0; await refreshAll(true); await frames(3);
  note("scan idle", diff(base, snapshot(view), strip));
  // the quiet lines: photos not read, folders offline, network sources left out
  h().scan.failed = 5; h().scan.offline = 2; h().scan.net = 3; await refreshAll(true); await frames(3);
  note("strip notes", diff(base, snapshot(view), strip));
  const notesText = $(".people-strip")?.textContent ?? "";
  out.notes = notesText;
  if (!/5 photos could not be read/.test(notesText) || !/2 folders offline/.test(notesText) || !/3 network folders not scanned/.test(notesText)) fail(`strip notes: "${notesText}"`);
  click($(".people-strip .people-link")!);
  await frames(6);
  const listed = $$(".people-failed li").length;
  if (listed !== 5) fail(`failed photos: the list shows ${listed}`);
  document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true }));
  await frames(3);
  note("strip notes after the list", diff(base, snapshot(view), strip));
  checkFixed("scan");
  drain(true, "scan");

  phase("newgroups");
  // 6. new groups arrive: the tiles on screen stay where they are, the new ones append, the counts change
  setPeople({ error: "" });
  const db = h().db();
  const topId = Math.max(...db.faces.map((f) => f.id));
  for (let c = 0; c < 30; c++) for (let k = 0; k < 3; k++) db.faces.push({ id: topId + 1 + c * 3 + k, path: `/synthetic/new_${c}_${k}.jpg`, person: null, state: "unreviewed", cluster: 9000 + c, maybe: null });
  base = snapshot(view);
  const keysBefore = tiles().map((x) => x.id);
  await refreshAll(true); await frames(4);
  note("new groups arrive", diff(base, snapshot(view), "", ".people-canvas"));
  const keysAfter = tiles().map((x) => x.id);
  if (!keysBefore.every((k) => keysAfter.includes(k))) fail("new groups: a tile on screen went away");
  checkFixed("new groups");
  drain(true, "new groups");
  out.layoutShift = { supported, value: Math.round(shifts * 10000) / 10000, detail };

  phase("counts");
  // 6b. real counts, the collage and the Ignored section
  const railCount = (i: number) => $$(".people-rail-item")[i].querySelector(".people-rail-count")?.textContent?.split(",").join("") ?? "";
  const totals = peopleState().status!.totals;
  out.counts = { unassigned: railCount(2), unknown: totals.unknown, ignored: railCount(3), ignoredTotal: totals.ignored_faces + totals.ignored_people };
  if (railCount(2) !== String(totals.unknown) || railCount(2).includes("+")) fail(`counts: Unassigned shows ${railCount(2)}, the total is ${totals.unknown}`);
  if (railCount(3) !== String(totals.ignored_faces + totals.ignored_people)) fail(`counts: Ignored shows ${railCount(3)}`);
  click($$(".people-rail-item")[1]);
  await frames(4);
  const collage = $$(".pcollage");
  out.collage = { groups: collage.length, cells: collage[0]?.children.length ?? 0 };
  if (!collage.length || collage[0].children.length < 2) fail("collage: a group tile has no 2 x 2 collage");
  click($$(".people-rail-item")[3]);
  await frames(4);
  const ignoredFaces = tiles().filter((x) => x.dataset.kind === "face");
  if (!ignoredFaces.length) fail("ignored: no ignored faces are listed");
  click(ignoredFaces[0]);
  await frames(2);
  base = snapshot(view);
  const was = ignoredFaces.length;
  press(grid(), "r");
  await frames(6);
  if (!h().calls.some((c) => c.startsWith("unignore:1"))) fail(`ignored: Restore did not call unignore (${h().calls.slice(-3).join(",")})`);
  out.ignored = { before: was, after: tiles().filter((x) => x.dataset.kind === "face").length, undo: $(".people-undo-text")?.textContent };
  note("restore an ignored face", diff(base, snapshot(view), ".people-canvas, .people-canvas *, .people-selbar > *"));
  checkFixed("ignored");
  drain(false);

  phase("scale");
  // 7. scale: scroll every section end to end; what is mounted stays bounded
  const section = async (name: string) => {
    phase(`scale-${name}`);
    click($$(".people-rail-item")[["people", "review", "unassigned", "ignored"].indexOf(name)]);
    await frames(4);
    const el = grid();
    let maxTiles = 0, maxNodes = 0;
    for (let pass = 0; pass < 2; pass++) {
      const totalH = el.scrollHeight;
      for (let y = 0; y <= totalH; y += Math.max(400, Math.floor(totalH / 40))) {
        el.scrollTop = y;
        await frames(1);
        maxTiles = Math.max(maxTiles, tiles().length);
        maxNodes = Math.max(maxNodes, view.querySelectorAll("*").length);
      }
      await frames(3);
    }
    const count = $$(".people-rail-item")[["people", "review", "unassigned", "ignored"].indexOf(name)].querySelector(".people-rail-count")?.textContent ?? "";
    return { maxTiles, maxNodes, height: el.scrollHeight, count };
  };
  out.virt = { people: await section("people"), review: await section("review"), unassigned: await section("unassigned") };
  await section("review");

  const w = Math.max(...Object.values(worst), 0);
  out.maxDelta = w;
  out.worst = worst;
  out.ok = (out.failures as string[]).length === 0 && w === 0 && (shifts < 0.002);
  return out;
}
