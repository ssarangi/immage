// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Dev-only harness for the Library panel's Metadata and Keywording cards while photos open one after
// another: the real cards, with the real hold (cardHold.ts), over a "Folders" marker that stands for
// everything below them. Photo A, B and C each open with a gap in which the entry and keywords are not
// in yet; the cards must stay mounted and the marker must not move. test=1 runs it and writes JSON into
// #cards-report; break=1 renders the cards as before the hold (the test must fail); break=reserve keeps the hold
// but drops the Auto-keywords group's reserved header and body (the arrival check must fail). test=2 runs the
// Auto-keywords group checks (arrival, removal, folding, keys) at the width in w; shot=1 draws one state to
// photograph: w, own, ai (counts), long=1, state=analysing|none|blocked|paused|off, theme=light, dir=rtl.
import "./tauriShim";
import { createRoot } from "react-dom/client";
import { useEffect, useState } from "react";
import "../src/styles.css";
import { BUILTIN_THEMES, THEME_TOKENS } from "../src/themes";
import { KeywordingPanel } from "../src/Keywording";
import { MetadataPanel } from "../src/MetadataPanel";
import { useCardHold } from "../src/cardHold";
import type { ImageEntry } from "../src/library";

const q = new URLSearchParams(location.search);
const theme = BUILTIN_THEMES.find((x) => x.id === (q.get("theme") ?? "dark")) ?? BUILTIN_THEMES[0];
for (const token of THEME_TOKENS) document.documentElement.style.setProperty(`--${token}`, theme.colors[token]);
document.documentElement.style.colorScheme = theme.colorScheme;
document.body.style.background = "var(--window)";
document.body.style.margin = "0";
const broken = q.get("break") === "1";
const noReserve = q.get("break") === "reserve";
if (q.get("dir")) document.documentElement.dir = q.get("dir")!;
// the real column gives Keywording a 224px floor (styles.css), whose spare height sits under the card's content
// and absorbs a reserve lost or a chip row gained, hiding exactly what these checks measure at the Folders
// marker; the floor is checked in the real column by verify/left-panel-shift.test.mjs, so it is off here
document.head.insertAdjacentHTML("beforeend", "<style>.left .panel.keywording:has(> .kw-body){min-height:0!important}</style>");
if (noReserve) document.head.insertAdjacentHTML("beforeend", "<style>.kw-auto-body{min-height:0!important}.kw-auto{min-height:0!important}.kw-auto-head{height:auto!important}</style>");
if (q.get("break") === "undo") document.head.insertAdjacentHTML("beforeend", "<style>.kw-auto-head{height:auto!important}.kw-auto-state{flex-direction:column;white-space:normal!important}</style>");
const W = Number(q.get("w") ?? 320);

// stands in for the three commands the Auto-keywords group reads
const aiOf: Record<string, string[]> = {};
const status = { paused: false, rows: [{ id: "smartSearch", enabled: false, installed: false, pending: 0, running: 0, active: false },
  { id: "keywords", enabled: true, installed: true, pending: 0, running: 0, active: false }] };
const summary = { blocked: null as string | null, health: "ok" };
const kept: Record<string, Set<string>> = {};
const analysed = new Set<string>();     // photos with a stored result; others count as not analysed yet
const calls: string[] = [];
(window as unknown as { __TAURI_INTERNALS__: { invoke: unknown } }).__TAURI_INTERNALS__.invoke = async (cmd: string, args: { paths: string[]; names: string[] }) => {
  await new Promise((r) => setTimeout(r, 10));
  if (cmd === "keywords_photo_states") {
    for (const p of args.paths) if ((aiOf[p] ?? []).length) analysed.add(p);
    return args.paths.map((path) => ({ path, analysed: analysed.has(path) || (aiOf[path] ?? []).length > 0, shared: false,
      keywords: (aiOf[path] ?? []).filter((n) => !kept[path]?.has(n)).map((name, i) => ({ name, confidence: 0.9 - (i % 9) / 10 })) }));
  }
  if (cmd === "keywords_ai_keep") {
    calls.push(`keep ${args.paths.length} ${args.names.join(",")}`);
    for (const p of args.paths) for (const n of args.names.length ? args.names : aiOf[p] ?? []) (kept[p] ??= new Set()).add(n);
    return args.paths.length;
  }
  if (cmd === "keywords_ai_remove_photos") {
    calls.push(`remove ${args.paths.length} ${args.names.join(",")}`);
    for (const p of args.paths) {
      const gone = args.names.length ? args.names : (aiOf[p] ?? []).filter((n) => !kept[p]?.has(n));
      aiOf[p] = (aiOf[p] ?? []).filter((n) => !gone.includes(n));
      setKeywordsOf?.(p, (k) => k.filter((n) => !gone.includes(n)));
    }
    return { photos: args.paths.length, keywords: 0, failed: 0 };
  }
  if (cmd === "keywords_ai_restore") {
    const items = (args as unknown as { items: { path: string; names: string[] }[] }).items;
    calls.push(`restore ${items.map((i) => `${i.names.length}`).join(",")}`);
    for (const i of items) {
      aiOf[i.path] = [...new Set([...(aiOf[i.path] ?? []), ...i.names])].sort((x, y) => WORDS.indexOf(x) - WORDS.indexOf(y));
      setKeywordsOf?.(i.path, (k) => [...new Set([...k, ...i.names])]);
    }
    return { photos: items.length, keywords: 0, failed: 0 };
  }
  if (cmd === "analysis_status") return status;
  if (cmd === "keywords_summary") return summary;
  throw new Error("harness: no Tauri");
};
const WORDS = "beach dog sea sky sunset tree water mountain cloud river forest bird flower snow city street bridge boat sand rock grass field road house window door car train light shadow night moon star lake wave hill path fog rain garden".split(" ");
const LONG = ["Extraordinarily long keyword that has to be cut short", "Places|France|Paris|Montmartre", "Sunset over the mountains and the lake"];
const names = (n: number, long: boolean) => Array.from({ length: n }, (_, i) => (long && i < LONG.length ? LONG[i] : WORDS[i % WORDS.length] + (i >= WORDS.length ? i : "")));

const photo = (n: number): ImageEntry => ({ imgid: n, path: `/p/IMG_${n}.NEF`, filename: `IMG_${n}.NEF`, rating: 0, rejected: false,
  labels: 0, edited: false, width: 6000, height: 4000, exif: { file_width: 6000, file_height: 4000, iso: 100, aperture: 2.8,
    exposure: 0.01, focal_length: 50, datetime: "2026:01:01 10:00:00", lens: "50mm", model: "Z6", maker: "Nikon" } as never });
const noop = () => {};
const removed: string[] = [];

let setKeywordsOf: ((path: string, f: (k: string[]) => string[]) => void) | undefined;
type S = { entry: ImageEntry | null; keywords: string[] | null; current: boolean; selected?: { path: string; keywords: string[] }[] };
function Harness() {
  const [s, setS] = useState<S>({ entry: null, keywords: null, current: false });
  const hold = useCardHold(s.entry, s.current, s.keywords);
  const entry = broken ? s.entry : hold.entry, keywords = broken ? (s.entry ? s.keywords : null) : hold.keywords;
  useEffect(() => {
    (window as unknown as { __set: (v: S) => void }).__set = setS;
    setKeywordsOf = (path, f) => setS((v) => (v.entry?.path === path && v.keywords ? { ...v, keywords: f(v.keywords) } : v));
    document.body.dataset.ready = "1";
    if (q.get("shot") === "1") {
      document.head.insertAdjacentHTML("beforeend", "<style>.panel.metadata{display:none}</style>");
      const n = Number(q.get("ai") ?? 7), own = Number(q.get("own") ?? 2), p = photo(1);
      const st = q.get("state");
      if (st === "off") status.rows[1].enabled = false;
      if (st === "analysing") status.rows[1].running = 1;
      if (st === "paused") status.paused = true;
      if (st === "blocked") { status.rows[1].pending = 5; summary.blocked = "free"; }
      const ai = st === "analysing" || st === "none" || st === "blocked" ? [] : names(n, q.get("long") === "1");
      aiOf[p.path] = ai;
      setS({ entry: p, keywords: [...(q.get("long") === "1" ? ["Own keyword with an unreasonably long name that cannot fit"] : []), ...["family", "Places|Paris", "holiday", "friends"].slice(0, own), ...ai], current: true });
      setTimeout(() => { document.body.dataset.shot = "1"; }, 400);
    }
    if (q.get("test") === "1") void run()
      .then((r) => r, (e) => ({ ok: false, error: String(e?.stack ?? e) }))
      .then((r) => { const el = document.getElementById("cards-report")!; el.textContent = JSON.stringify(r); el.dataset.done = "1"; });
    const test = q.get("test");
    if (test === "3" || test === "4" || test === "5") void (test === "3" ? runMulti() : test === "5" ? runUndo() : runActions()).then((r) => r, (e) => ({ ok: false, error: String(e?.stack ?? e) }))
      .then((r) => { const el = document.getElementById("cards-report")!; el.textContent = JSON.stringify(r); el.dataset.done = "1"; });
    if (q.get("test") === "2") void runAuto().then((r) => r, (e) => ({ ok: false, error: String(e?.stack ?? e) }))
      .then((r) => { const el = document.getElementById("cards-report")!; el.textContent = JSON.stringify(r); el.dataset.done = "1"; });
  }, []);
  return (
    <div className="app" style={{ position: "fixed", inset: 0, display: "block" }}>
      <div className="left" style={{ width: W }}>
        <MetadataPanel image={entry} text={null} onText={noop} onMeta={noop} onCaptureTime={noop} onLocationSaved={noop} open onToggle={noop} />
        <KeywordingPanel keywords={keywords} loading={hold.loading} known={[]} photoPath={entry?.path} selected={s.selected} open onToggle={noop} onAdd={noop}
                         onRemove={(k) => { removed.push(k); (window as unknown as { __remove?: (k: string) => void }).__remove?.(k); }} />
        <div id="folders" className="panel" style={{ height: 40 }}>Folders</div>
      </div>
    </div>
  );
}

const wait = () => new Promise<void>((done) => { setTimeout(done, 80); });
async function run() {
  const set = (window as unknown as { __set: (v: S) => void }).__set;
  const y = () => document.getElementById("folders")!.getBoundingClientRect().top;
  const cards = () => [!!document.querySelector(".panel.metadata"), !!document.querySelector(".panel.keywording")];
  const fail: string[] = [];
  const seen: number[] = [];
  const step = async (label: string, v: S) => {
    set(v); await wait();
    seen.push(y());
    if (!cards().every(Boolean)) fail.push(`${label}: a card is gone ${cards()}`);
  };
  // every photo carries some Auto-keywords, a different number each, so the group is there to hold
  const ai = (n: number) => { aiOf[photo(n).path] = names(n * 4 - 1, false); return aiOf[photo(n).path]; };
  await step("A open", { entry: photo(1), keywords: ["a", "b", ...ai(1)], current: true });
  for (const n of [2, 3, 4]) {
    await step(`${n} opening`, { entry: null, keywords: null, current: true });     // not listed, keywords not read
    await step(`${n} entry`, { entry: photo(n), keywords: null, current: true });   // listed, keywords still reading
    await step(`${n} loaded`, { entry: photo(n), keywords: ["c", "d", ...ai(n)], current: true });
  }
  const moved = Math.max(...seen) - Math.min(...seen);
  if (moved !== 0) fail.push(`the Folders marker moved by ${moved}px`);
  set({ entry: null, keywords: null, current: false }); await wait();
  const goneWhenClosed = !cards().some(Boolean);
  set({ entry: photo(5), keywords: null, current: true }); await wait();
  const disabled = document.querySelector<HTMLInputElement>(".kw-add input")?.disabled;
  return { ok: fail.length === 0 && goneWhenClosed && disabled === true, fail, moved, goneWhenClosed, disabled, steps: seen.length };
}

// the Auto-keywords group: nothing below it moves while analysis lands, chips are taken off, the list
// folds, and the keys work; the chips keep their height and stay inside the card
async function runAuto() {
  const set = (window as unknown as { __set: (v: S) => void }).__set;
  const y = () => document.getElementById("folders")!.getBoundingClientRect().top;
  const p = photo(1);
  const fail: string[] = [];
  const own = ["family", "Places|Paris"];
  const open = async (ai: string[], keywords = [...own, ...ai]) => { aiOf[p.path] = ai; set({ entry: p, keywords, current: true }); await wait(); await wait(); };
  await open(names(1, false));
  const base = y();
  const ys: Record<string, number> = {};
  for (const [label, n, long] of [["arrive 12", 12, false], ["arrive 40", 40, false], ["long names", 9, true], ["arrive 3", 3, false], ["arrive 7", 7, false]] as const) {
    await open(names(n, long)); ys[label] = y() - base;
  }
  // taking chips off, one at a time, down to none: the body stays
  let left = names(7, false);
  while (left.length) { left = left.slice(1); await open(left); ys[`left ${left.length}`] = y() - base; }
  const moved = Math.max(...Object.values(ys).map(Math.abs));
  await open(names(40, false));
  const wrapBox = document.querySelector(".kw-auto .kw-chips")!.getBoundingClientRect();
  const chips = Array.from(document.querySelectorAll<HTMLElement>(".kw-auto .kw-chip"));
  const heights = [...new Set(Array.from(document.querySelectorAll<HTMLElement>(".kw-chip")).map((c) => c.getBoundingClientRect().height))];
  const rows = new Set(chips.map((c) => Math.round(c.getBoundingClientRect().top))).size;
  const overflow = chips.filter((c) => { const r = c.getBoundingClientRect(); return r.left < wrapBox.left - 0.5 || r.right > wrapBox.right + 0.5; }).length;
  const more = document.querySelector<HTMLElement>(".kw-more");
  const moreText = more?.textContent ?? "";
  const moreInside = !more || (more.getBoundingClientRect().right <= wrapBox.right + 0.5 && more.getBoundingClientRect().left >= wrapBox.left - 0.5);
  const bodyH = document.querySelector<HTMLElement>(".kw-auto-body")!.getBoundingClientRect().height;
  const headH = document.querySelector<HTMLElement>(".kw-auto-head")!.getBoundingClientRect().height;
  const rowsShown = rows;
  // folding out grows the card (the user asked), folding back restores it
  const y0 = y();
  more?.click(); await wait();
  const grew = y() - y0;
  document.querySelector<HTMLElement>(".kw-more")?.click(); await wait();
  const back = y() - y0;
  // keys: the first Auto-keyword takes the one tab stop; arrows move, swapped when the text runs right to left
  const rtl = getComputedStyle(document.documentElement).direction === "rtl";
  const list = () => Array.from(document.querySelectorAll<HTMLElement>(".kw-auto .kw-chip"));
  const stops = list().filter((c) => c.tabIndex === 0).length;
  const press = (el: Element, key: string) => el.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
  const first = list()[0]; first.focus();
  press(first, rtl ? "ArrowLeft" : "ArrowRight"); await wait();
  const movedOn = document.activeElement === list()[1];
  press(document.activeElement!, rtl ? "ArrowRight" : "ArrowLeft"); await wait();
  const movedBack = document.activeElement === list()[0];
  press(document.activeElement!, "End"); await wait();
  const atEnd = document.activeElement === list()[list().length - 1];
  press(document.activeElement!, "Home"); await wait();
  const atHome = document.activeElement === list()[0];
  const target = (document.activeElement as HTMLElement).dataset.kw;
  calls.length = 0;
  press(document.activeElement!, "Backspace"); await wait();
  const removedByKey = calls.includes(`remove 1 ${target}`);
  const named = list()[0].getAttribute("aria-label") ?? "";
  if (moved !== 0) fail.push(`the Folders marker moved by ${moved}px: ${JSON.stringify(ys)}`);
  if (heights.length !== 1 || heights[0] !== 24) fail.push(`chip heights ${heights}`);
  if (rowsShown > 2) fail.push(`${rowsShown} rows folded`);
  if (overflow) fail.push(`${overflow} chips outside the card`);
  if (!moreInside) fail.push("the +N button is outside the card");
  if (headH !== 22) fail.push(`group header ${headH}px`);
  if (stops !== 1) fail.push(`${stops} tab stops`);
  if (!(movedOn && movedBack && atEnd && atHome && removedByKey)) fail.push(`keys ${[movedOn, movedBack, atEnd, atHome, removedByKey]}`);
  if (!/Auto-keywords|automatiques/.test(named)) fail.push(`chip name "${named}"`);
  if (!(grew > 0 && Math.abs(back) < 1)) fail.push(`fold: grew ${grew}, back ${back}`);
  return { ok: fail.length === 0, fail, moved, ys, bodyH, headH, rows: rowsShown, moreText, grew, back, stops };
}

// Keep, the group menu and removing an Auto-keywords chip: the keyword moves to the user's own chips (or
// goes), the card's chip count follows, and nothing below the card moves, down to the last chip
const must = (sel: string) => { const el = document.querySelector<HTMLElement>(sel); if (!el) throw new Error(`no ${sel}`); return el; };
async function runActions() {
  const set = (window as unknown as { __set: (v: S) => void }).__set;
  const y = () => document.getElementById("folders")!.getBoundingClientRect().top;
  const p = photo(1);
  const fail: string[] = [];
  const own = ["family"], ai = names(8, false);
  aiOf[p.path] = ai; set({ entry: p, keywords: [...own, ...ai], current: true }); await wait(); await wait();
  // a kept keyword joins the user's own chips, which may add a row above: only what is below the add box
  // (the group and Recent) must keep its height
  const below = () => y() - must(".kw-add").getBoundingClientRect().bottom;
  const base = below();
  const autoNames = () => Array.from(document.querySelectorAll<HTMLElement>(".kw-auto .kw-chip")).map((c) => c.dataset.kw!);
  const ownNames = () => Array.from(document.querySelectorAll<HTMLElement>(".kw-body > .kw-chips .kw-chip")).map((c) => c.dataset.kw!);
  const ys: number[] = [];
  const settle = async () => { await wait(); await wait(); ys.push(below() - base); };
  const tip = must(".kw-auto .kw-chip").title;
  if (!/% sure/.test(tip) || !/keep/i.test(tip)) fail.push(`tooltip "${tip}"`);
  const aria = must(".kw-auto .kw-chip").getAttribute("aria-label") ?? "";
  if (!/% sure/.test(aria)) fail.push(`name "${aria}"`);
  const sorted = autoNames();
  // confidence order: the shim gives the first state keyword the highest confidence
  if (sorted[0] !== ai[0]) fail.push(`order ${sorted.slice(0, 2)}`);
  // click keeps one
  if (autoNames().length < 2) fail.push(`auto chips ${autoNames()} own ${ownNames()} head ${document.querySelector(".kw-auto-head")?.textContent}`);
  const target = autoNames()[1] ?? autoNames()[0];
  must(`.kw-auto .kw-chip[data-kw="${target}"] .kw-leaf`).click(); await settle();
  if (autoNames().includes(target) || !ownNames().includes(target)) fail.push(`click kept ${target}: auto ${autoNames().includes(target)}, own ${ownNames().includes(target)}`);
  // Enter keeps the focused one
  const second = autoNames()[0];
  const chip = must(`.kw-auto .kw-chip[data-kw="${second}"]`); chip.focus();
  chip.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })); await settle();
  if (autoNames().includes(second) || !ownNames().includes(second)) fail.push(`Enter kept ${second}`);
  // the x of an Auto-keywords chip removes it from this photo only
  const rm = autoNames()[0];
  must(`.kw-auto .kw-chip[data-kw="${rm}"] button`).click(); await settle();
  if (autoNames().includes(rm) || ownNames().includes(rm)) fail.push(`x removed ${rm}`);
  if (!calls.some((c) => c === `remove 1 ${rm}`)) fail.push(`calls ${calls}`);
  // the menu: Keep all
  const headState = () => document.querySelector(".kw-auto-state")?.textContent;
  const before = autoNames().length;
  const menuBtn = must(".kw-auto-more");
  menuBtn.click(); await wait();
  const items = Array.from(document.querySelectorAll<HTMLElement>("[role=menuitem]")).map((b) => b.textContent);
  if (items.length < 2) fail.push(`menu items ${items}`);
  document.querySelectorAll<HTMLElement>("[role=menuitem]")[0].click(); await settle();
  if (autoNames().length !== 0 || ownNames().length < before) fail.push(`keep all: auto ${autoNames().length}, own ${ownNames().length}`);
  // the group stays (header and body) with nothing left
  const head = document.querySelector<HTMLElement>(".kw-auto-head")?.getBoundingClientRect().height;
  const bodyH = document.querySelector<HTMLElement>(".kw-auto-body")?.getBoundingClientRect().height ?? 0;
  const moved = Math.max(...ys.map(Math.abs));
  if (moved !== 0) fail.push(`the card below the add box changed height by ${moved}px: ${ys}`);
  if (head !== 22 || bodyH < 52) fail.push(`group kept? head ${head}, body ${bodyH}, state ${headState()}`);
  // an open photo the analysis has been through with nothing found says so; one it has not does not
  const q2 = photo(2);
  aiOf[q2.path] = []; analysed.add(q2.path);
  set({ entry: q2, keywords: ["x"], current: true }); await wait(); await wait();
  const nothing = document.querySelector(".kw-auto-state")?.textContent ?? "";
  if (!/nothing found|rien trouvé/.test(nothing)) fail.push(`analysed, none found says "${nothing}"`);
  const q3 = photo(3);
  set({ entry: q3, keywords: ["x"], current: true }); await wait(); await wait();
  const unseen = document.querySelector(".kw-auto");
  if (unseen) fail.push("a photo that was never analysed, with an idle queue, has a group");
  return { ok: fail.length === 0, fail, moved, ys, items, calls };
}

// several photos: keywords on all of them are chips, ones on some are dimmed with n/m, a click adds a mixed
// one to all, and going between selections moves nothing below
async function runMulti() {
  const set = (window as unknown as { __set: (v: S) => void }).__set;
  const y = () => document.getElementById("folders")!.getBoundingClientRect().top;
  const fail: string[] = [];
  const a = photo(1), b = photo(2), c = photo(3);
  const rowsOf = (list: [ImageEntry, string[], string[]][]) => list.map(([e, own, ai]) => { aiOf[e.path] = ai; return { path: e.path, keywords: [...own, ...ai] }; });
  const open = async (list: [ImageEntry, string[], string[]][]) => {
    const rows = rowsOf(list);
    set({ entry: list[0][0], keywords: rows[0].keywords, current: true, selected: rows.length > 1 ? rows : undefined }); await wait(); await wait();
  };
  await open([[a, ["family"], names(5, false)]]);
  const base = y();
  const ys: Record<string, number> = {};
  const chipsOf = (sel: string) => Array.from(document.querySelectorAll<HTMLElement>(sel));
  await open([[a, ["family", "trip"], ["beach", "dog"]], [b, ["family"], ["beach", "dog"]], [c, ["family", "trip"], ["dog", "extra"]]]);
  const own = chipsOf(".kw-body > .kw-chips .kw-chip");
  const mixedOwn = own.filter((x) => x.classList.contains("mixed"));
  if (own[0]?.dataset.kw !== "family" || own[0].classList.contains("mixed")) fail.push(`common first: ${own.map((x) => x.dataset.kw)}`);
  if (mixedOwn.length !== 1 || !/2\/3/.test(mixedOwn[0].textContent ?? "")) fail.push(`mixed own: ${mixedOwn.map((x) => x.textContent)}`);
  // a narrow card folds the list; open it to see every chip
  const more = document.querySelector<HTMLElement>(".kw-more");
  if (more && /^\+/.test(more.textContent ?? "")) { more.click(); await wait(); }
  const auto = chipsOf(".kw-auto .kw-chip");
  const mixedAuto = auto.filter((x) => x.classList.contains("mixed"));
  if (!mixedAuto.some((x) => x.dataset.kw === "extra" && /1\/3/.test(x.textContent ?? ""))) fail.push(`mixed auto: ${mixedAuto.map((x) => x.textContent)} of ${auto.map((x) => x.textContent)}, state ${document.querySelector(".kw-auto-state")?.textContent} widths ${auto.map((x) => Math.round(x.getBoundingClientRect().width))} wrap ${document.querySelector(".kw-auto .kw-chips")?.getBoundingClientRect().width} more ${document.querySelector(".kw-more")?.textContent}`);
  if (auto.filter((x) => !x.classList.contains("mixed")).length < 1) fail.push("common Auto-keywords chips missing");
  const rm = mixedOwn[0].querySelector("button")!.title;
  if (!/3|2/.test(rm)) fail.push(`remove title "${rm}"`);
  // Keep on a common Auto-keywords chip keeps it on every photo
  calls.length = 0;
  chipsOf(".kw-auto .kw-chip:not(.mixed) .kw-leaf")[0].click(); await wait(); await wait();
  if (!calls.some((x) => /^keep 3 /.test(x))) fail.push(`keep calls ${calls}`);
  document.querySelector<HTMLElement>(".kw-more[aria-expanded=true]")?.click(); await wait();
  await open([[a, ["family"], []], [b, ["family"], []]]);
  ys.none = y() - base;
  await open([[a, ["family"], names(30, false)], [b, ["family"], names(12, false)]]);
  ys.many = y() - base;
  await open([[a, ["family"], names(1, false)]]);
  ys.one = y() - base;
  const moved = Math.max(...Object.values(ys).map(Math.abs));
  if (moved !== 0) fail.push(`the Folders marker moved by ${moved}px: ${JSON.stringify(ys)}`);
  return { ok: fail.length === 0, fail, moved, ys, calls };
}

// the Undo line: it takes the state row's place, so nothing below the card moves when it appears or goes;
// Undo puts the keyword back through keywords_ai_restore; it goes by itself after 5 s but waits while the
// pointer or the focus is on it
async function runUndo() {
  const set = (window as unknown as { __set: (v: S) => void }).__set;
  const y = () => document.getElementById("folders")!.getBoundingClientRect().top;
  const p = photo(1);
  const fail: string[] = [];
  const ai = names(8, false);
  aiOf[p.path] = ai; set({ entry: p, keywords: ["family", ...ai], current: true }); await wait(); await wait();
  const base = y();
  const ys: number[] = [];
  const settle = async () => { await wait(); await wait(); ys.push(y() - base); };
  const autoNames = () => Array.from(document.querySelectorAll<HTMLElement>(".kw-auto .kw-chip")).map((c) => c.dataset.kw!);
  const line = () => document.querySelector<HTMLElement>(".kw-auto-state");
  const undoBtn = () => document.querySelector<HTMLButtonElement>(".kw-undo");
  const sleep = (ms: number) => new Promise<void>((d) => { setTimeout(d, ms); });
  if (undoBtn()) fail.push("an Undo line before any removal");
  const live = line();
  if (live?.getAttribute("role") !== "status" || live?.getAttribute("aria-live") !== "polite") fail.push("the state row is not a polite status");
  const rm = autoNames()[0];
  must(`.kw-auto .kw-chip[data-kw="${rm}"] button`).click(); await settle();
  if (!undoBtn()) fail.push("no Undo line after removing a chip");
  if (!line()?.textContent?.includes(rm.slice(0, 3))) fail.push(`line text "${line()?.textContent}"`);
  if (line() !== live) fail.push("the Undo line is not the announced status element");
  if (autoNames().includes(rm)) fail.push("the chip stayed");
  const head = () => document.querySelector<HTMLElement>(".kw-auto-head")!.getBoundingClientRect().height;
  if (head() !== 22) fail.push(`header ${head()}px with the line`);
  // Undo puts it back
  undoBtn()?.click(); await settle();
  if (!autoNames().includes(rm)) fail.push("Undo did not put the chip back");
  if (undoBtn()) fail.push("the line stayed after Undo");
  if (!calls.some((c) => c === "restore 1")) fail.push(`calls ${calls}`);
  // it goes by itself
  must(`.kw-auto .kw-chip[data-kw="${rm}"] button`).click(); await settle();
  if (!undoBtn()) fail.push("no line for the timer check");
  await sleep(4000); if (!undoBtn()) fail.push("the line went before 5 s");
  await sleep(1500); await settle();
  if (undoBtn()) fail.push("the line stayed past 5 s");
  // hovering holds it, leaving lets it go
  const second = autoNames()[0];
  must(`.kw-auto .kw-chip[data-kw="${second}"] button`).click(); await settle();
  line()!.dispatchEvent(new PointerEvent("pointerover", { bubbles: true, pointerType: "mouse" }));
  await sleep(6000);
  if (!undoBtn()) fail.push("the line went while hovered");
  line()!.dispatchEvent(new PointerEvent("pointerout", { bubbles: true, pointerType: "mouse" }));
  await sleep(1000); if (!undoBtn()) fail.push("the line went at once when the pointer left");
  await sleep(4500); await settle();
  if (undoBtn()) fail.push("the line stayed after the pointer left");
  // taking off the rest: the line takes the state row's place with the group kept
  const shownBefore = autoNames().length;
  document.querySelectorAll<HTMLElement>(".kw-auto-more")[0].click(); await wait();
  document.querySelectorAll<HTMLElement>("[role=menuitem]")[1].click(); await settle();
  if (!undoBtn() || !/\d/.test(line()?.textContent ?? "")) fail.push(`Remove all line "${line()?.textContent}"`);
  if (autoNames().length) fail.push("Remove all left chips");
  if (head() !== 22) fail.push(`header ${head()}px after Remove all`);
  undoBtn()?.click(); await settle();
  if (autoNames().length !== shownBefore) fail.push(`Undo of Remove all put back ${autoNames().length}`);
  // another photo drops the line
  must(`.kw-auto .kw-chip[data-kw="${autoNames()[0]}"] button`).click(); await settle();
  const q2 = photo(2); aiOf[q2.path] = ["x"];
  set({ entry: q2, keywords: ["x"], current: true }); await wait(); await wait();
  if (undoBtn()) fail.push("the line followed to another photo");
  const moved = Math.max(...ys.map(Math.abs));
  if (moved !== 0) fail.push(`moved ${moved}px: ${ys}`);
  return { ok: fail.length === 0, fail, moved, ys, calls };
}

createRoot(document.getElementById("root")!).render(<Harness />);
