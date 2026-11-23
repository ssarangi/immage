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
if (noReserve) document.head.insertAdjacentHTML("beforeend", "<style>.kw-auto-body{min-height:0!important}.kw-auto-head{height:auto!important}</style>");
const W = Number(q.get("w") ?? 320);

// stands in for the three commands the Auto-keywords group reads
const aiOf: Record<string, string[]> = {};
const status = { paused: false, rows: [{ id: "smartSearch", enabled: false, installed: false, pending: 0, running: 0, active: false },
  { id: "keywords", enabled: true, installed: true, pending: 0, running: 0, active: false }] };
const summary = { blocked: null as string | null, health: "ok" };
(window as unknown as { __TAURI_INTERNALS__: { invoke: unknown } }).__TAURI_INTERNALS__.invoke = async (cmd: string, args: { path: string }) => {
  await new Promise((r) => setTimeout(r, 10));
  if (cmd === "keywords_ai_of") return aiOf[args.path] ?? [];
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

type S = { entry: ImageEntry | null; keywords: string[] | null; current: boolean };
function Harness() {
  const [s, setS] = useState<S>({ entry: null, keywords: null, current: false });
  const hold = useCardHold(s.entry, s.current, s.keywords);
  const entry = broken ? s.entry : hold.entry, keywords = broken ? (s.entry ? s.keywords : null) : hold.keywords;
  useEffect(() => {
    (window as unknown as { __set: (v: S) => void }).__set = setS;
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
    if (q.get("test") === "2") void runAuto().then((r) => r, (e) => ({ ok: false, error: String(e?.stack ?? e) }))
      .then((r) => { const el = document.getElementById("cards-report")!; el.textContent = JSON.stringify(r); el.dataset.done = "1"; });
  }, []);
  return (
    <div className="app" style={{ position: "fixed", inset: 0, display: "block" }}>
      <div className="left" style={{ width: W }}>
        <MetadataPanel image={entry} text={null} onText={noop} onMeta={noop} onCaptureTime={noop} onLocationSaved={noop} open onToggle={noop} />
        <KeywordingPanel keywords={keywords} loading={hold.loading} known={[]} photoPath={entry?.path} open onToggle={noop} onAdd={noop}
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
  removed.length = 0;
  press(document.activeElement!, "Backspace");
  const removedByKey = removed.length === 1 && removed[0] === target;
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

createRoot(document.getElementById("root")!).render(<Harness />);
