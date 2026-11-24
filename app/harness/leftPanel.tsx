// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Dev-only harness for the Library column's fixed height: the real `.left` aside, `.panel` cards, bodies and
// styles.css with fixture content, driven by the real toggleLeftSet and headerKey. With ?test=1 it opens
// combinations of cards and writes JSON into #shift-report (verify/left-panel-shift.test.mjs asserts it).
//   ?h=700    the aside's height, ?break=1 puts the column's own scrolling back (negative control),
//   ?break=2 puts each card's fixed controls back inside its scroller (negative control)
import "./tauriShim";
import { createRoot } from "react-dom/client";
import { useState } from "react";
import "../src/styles.css";
import { BUILTIN_THEMES, THEME_TOKENS } from "../src/themes";
import { headerKey } from "../src/CatalogCard";
import { toggleLeftSet } from "../src/panelToggle";

const q = new URLSearchParams(location.search);
const theme = BUILTIN_THEMES[0];
for (const token of THEME_TOKENS) document.documentElement.style.setProperty(`--${token}`, theme.colors[token]);
document.documentElement.style.colorScheme = theme.colorScheme;
document.body.style.background = "var(--window)";
document.body.style.margin = "0";
if (q.get("break") === "1") {
  const st = document.createElement("style");
  st.textContent = `.left .panel { flex: none !important; min-height: 0 !important; }
    .left .panel > :is(.meta-body, .kw-body, .folders-body, .cat-body, .history-list) { overflow: visible !important; flex: none !important; }`;
  document.head.append(st);
}
if (q.get("break") === "2") {
  const st = document.createElement("style");
  st.textContent = `.left .panel > :is(.folders-body, .cat-body, .kw-body) { display: block !important; overflow-y: auto !important; }
    .left .folders-body > .ftree, .left .cat-body > .ftree, .left .kw-auto-body { overflow: visible !important; flex: none !important; }`;
  document.head.append(st);
}
const H = Number(q.get("h") ?? 700);

const CARDS = ["metadata", "keywording", "people", "folders", "catalog", "history"] as const;
type Card = typeof CARDS[number];
const rows = (n: number) => Array.from({ length: n }, (_, i) => <div key={i} style={{ height: 28, padding: "0 12px" }}>row {i}</div>);
let setOpenFromTest: (s: Set<Card>) => void = () => {};

function Harness() {
  const [open, setOpen] = useState<Set<Card>>(new Set(["folders", "catalog"]));
  setOpenFromTest = setOpen;
  const toggle = (id: Card) => setOpen((o) => toggleLeftSet(o, id));
  const card = (id: Card, extra: string, body: React.ReactNode) => (
    <div className={`panel ${extra}`} data-card={id}>
      <div className={`panel-title${open.has(id) ? " open" : ""}`} onClick={() => toggle(id)}
           role="button" tabIndex={0} aria-expanded={open.has(id)} onKeyDown={(e) => headerKey(e, () => toggle(id))}>
        <span>{id}</span>
      </div>
      {open.has(id) && body}
    </div>
  );
  return (
    <aside className="left" style={{ width: 300, height: H }} data-aside>
      <div className="panel-head"><h1>Library</h1></div>
      <div className="navigator"><div className="nav-head"><span>Navigator</span></div><div className="nav-image"><div className="nav-pic" style={{ width: 100, height: 60 }} /></div></div>
      {card("metadata", "metadata", <div className="meta-body"><div className="meta-grid">{rows(7)}</div><div style={{ height: 80 }}>location</div></div>)}
      {card("keywording", "keywording", <div className="kw-body">
        <div className="kw-chips" data-own>{Array.from({ length: 3 }, (_, i) => <span key={i} className="kw-chip"><span className="kw-leaf">own {i}</span></span>)}</div>
        <div className="kw-add"><input className="field" defaultValue="" /></div>
        <div className="kw-auto" role="group">
          <div className="kw-auto-head"><span className="kw-auto-name">Auto-keywords</span><span className="kw-auto-state">60</span><span className="kw-auto-menu"><button type="button" className="kw-auto-more">...</button></span></div>
          <div className="kw-auto-body"><div className="kw-chips">{Array.from({ length: 60 }, (_, i) => <span key={i} className="kw-chip ai"><span className="kw-leaf">auto {i}</span></span>)}</div></div>
        </div>
        <div className="kw-label">Recent</div>
        <div className="kw-recent">{["a", "b", "c"].map((k) => <button key={k}>{k}</button>)}</div></div>)}
      {card("people", "faces-panel", <div className="faces-body">
        <div className="faces-chips" role="list">{Array.from({ length: 30 }, (_, i) => <button key={i} type="button" role="listitem" className="faces-chip"><span className="faces-crop" /><span>Person {i}</span></button>)}</div>
        <label className="faces-switch"><span>Show faces on photo</span><input type="checkbox" className="switch" /></label>
        <div className="faces-status" role="status">Up to date</div></div>)}
      {card("folders", "folders-panel", <div className="folders-body">
        <label className="sub-toggle"><span>Include subfolders</span><input type="checkbox" className="switch" defaultChecked /></label>
        <div className="ftree folders" role="tree">{Array.from({ length: 60 }, (_, i) => <div key={i} className="folder-row" style={{ height: 28 }}>folder {i}</div>)}</div>
        <button className="folder add"><span>Add folder</span></button></div>)}
      {card("catalog", "catalog-panel", <div className="cat-body">
        <div className="ftree cat-list" role="tree">{Array.from({ length: 60 }, (_, i) => <div key={i} className="folder-row" style={{ height: 28 }}>source {i}</div>)}</div>
        <button className="folder add"><span>Add folder</span></button>
        <div className="cat-footer"><div className="cat-card" style={{ height: 62 }}>index card</div></div></div>)}
      {card("history", "history", <ol className="history-list">{Array.from({ length: 30 }, (_, i) => <li key={i} style={{ height: 28 }}>step {i}</li>)}</ol>)}
    </aside>
  );
}
createRoot(document.getElementById("root")!).render(<Harness />);

const tick = (n = 3) => new Promise<void>((done) => { setTimeout(done, 40 * n); });
const aside = () => document.querySelector<HTMLElement>("[data-aside]")!;
const head = (id: string) => document.querySelector<HTMLElement>(`[data-card=${id}] .panel-title`)!;
const body = (id: string) => document.querySelector<HTMLElement>(`[data-card=${id}] > :not(.panel-title)`);
const top = (el: Element) => Math.round(el.getBoundingClientRect().top * 100) / 100;
const openSet = () => CARDS.filter((c) => head(c).getAttribute("aria-expanded") === "true");

const COMBOS: Card[][] = [[], ["folders"], ["catalog"], ["folders", "catalog"], ["metadata", "keywording"],
  ["people"], ["metadata", "people", "keywording"], ["metadata", "keywording", "folders", "catalog"], [...CARDS]];

async function run() {
  await tick(5);
  const out: Record<string, unknown> = { H };
  const combos = [];
  for (const set of COMBOS) {
    setOpenFromTest(new Set(set)); await tick();
    const a = aside();
    const bodies = set.map((id) => {
      const b = body(id)!, cs = getComputedStyle(b);
      const inner = b.querySelector<HTMLElement>(".ftree, .kw-auto-body, .faces-chips");
      const ht = head(id);
      return { id, clientHeight: b.clientHeight, scrollHeight: b.scrollHeight, overflowY: cs.overflowY,
        headerTop: top(ht), scrolls: b.scrollHeight > b.clientHeight + 1 || (!!inner && inner.scrollHeight > inner.clientHeight + 1) };
    });
    // a scrolled body keeps its header where it was
    const tall = set.find((id) => id === "folders" || id === "catalog" || id === "keywording" || id === "people");
    let headerStaysOnScroll = true;
    if (tall) { const t0 = top(head(tall)); body(tall)!.scrollTop = 200; body(tall)!.querySelector<HTMLElement>(".ftree, .kw-auto-body, .faces-chips")!.scrollTop = 200; headerStaysOnScroll = top(head(tall)) === t0; }
    combos.push({ set, columnScrollHeight: a.scrollHeight, columnClientHeight: a.clientHeight, bodies, headerStaysOnScroll });
  }
  out.combos = combos;
  // fixed controls stay put while the one scrolling region scrolls (each card open alone, and all three open)
  const fixedCase = async (open: Card[], id: Card) => {
    setOpenFromTest(new Set(open)); await tick();
    const card = document.querySelector<HTMLElement>(`[data-card=${id}]`)!, b = body(id)!;
    const sels = id === "folders" ? [".sub-toggle", "button.folder.add"] : id === "catalog" ? ["button.folder.add", ".cat-footer"]
      : id === "people" ? [".faces-switch", ".faces-status"] : ["[data-own]", ".kw-add", ".kw-auto-head", ".kw-recent"];
    const scroller = b.querySelector<HTMLElement>(id === "keywording" ? ".kw-auto-body" : id === "people" ? ".faces-chips" : ".ftree")!;
    const rel = () => sels.map((sel) => top(b.querySelector(sel)!) - top(card));
    const before = rel();
    scroller.scrollTop = scroller.scrollHeight; await tick(2);
    const after = rel();
    const last = b.querySelector(sels.at(-1)!)!.getBoundingClientRect();
    return { id, open, scrolls: scroller.scrollHeight > scroller.clientHeight + 1, scrolled: scroller.scrollTop > 0,
      bodyScrolls: b.scrollHeight > b.clientHeight + 1, bodyTop: b.scrollTop,
      moved: Math.max(...before.map((t, i) => Math.abs(t - after[i]))), inCard: last.bottom <= card.getBoundingClientRect().bottom + 1,
      scrollerHeight: scroller.clientHeight, bodyH: b.clientHeight, bodySH: b.scrollHeight, cardH: card.clientHeight, asideH: aside().clientHeight };
  };
  const fixedRuns = [];
  for (const id of ["folders", "catalog", "keywording", "people"] as Card[]) fixedRuns.push(await fixedCase([id], id));
  for (const id of ["folders", "catalog", "keywording"] as Card[]) fixedRuns.push(await fixedCase(["keywording", "folders", "catalog"], id));
  fixedRuns.push(await fixedCase(["people", "keywording", "folders", "catalog"], "people"));
  out.fixed = fixedRuns;
  // toggling a card moves no header above it while the column has room (cards above keep their size); when it is
  // full, cards above share the height and their sizes change, so only the Navigator and the first header are fixed
  let moves = 0;
  const order = [...CARDS];
  setOpenFromTest(new Set()); await tick();
  for (const id of order.flatMap((c) => [c, c])) {
    const above = order.slice(0, order.indexOf(id));
    const t0 = above.map((c) => top(head(c)));
    head(id).click(); await tick();
    const t1 = above.map((c) => top(head(c)));
    moves = Math.max(moves, ...t0.map((t, i) => Math.abs(t - t1[i])));
  }
  out.headerMoves = moves;
  setOpenFromTest(new Set(["folders", "catalog"])); await tick();
  let fixed = 0;
  const nav = document.querySelector(".navigator")!;
  for (const id of ["history", "keywording", "metadata", "history", "keywording", "metadata", "catalog", "folders"] as Card[]) {
    const t0 = [top(nav), top(head("metadata"))];
    head(id).click(); await tick();
    fixed = Math.max(fixed, Math.abs(top(nav) - t0[0]), id === "metadata" ? 0 : Math.abs(top(head("metadata")) - t0[1]));
  }
  out.fixedMoves = fixed;
  // keyboard: Enter and Space toggle a header
  setOpenFromTest(new Set()); await tick();
  const h = head("history");
  h.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); await tick();
  const afterEnter = h.getAttribute("aria-expanded");
  h.dispatchEvent(new KeyboardEvent("keydown", { key: " ", bubbles: true })); await tick();
  out.keys = [afterEnter, h.getAttribute("aria-expanded")];
  out.roles = CARDS.every((c) => head(c).getAttribute("role") === "button" && head(c).hasAttribute("aria-expanded"));
  out.open = openSet();
  document.getElementById("shift-report")!.textContent = JSON.stringify(out);
}
if (q.get("test") === "1") void run().catch((e) => { document.getElementById("shift-report")!.textContent = JSON.stringify({ error: String(e) }); });
