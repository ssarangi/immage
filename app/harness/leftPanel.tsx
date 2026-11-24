// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Dev-only harness for the Library column's fixed height: the real `.left` aside, `.panel` cards, bodies and
// styles.css with fixture content, driven by the real toggleLeftSet and headerKey. With ?test=1 it opens
// combinations of cards and writes JSON into #shift-report (verify/left-panel-shift.test.mjs asserts it).
//   ?h=700    the aside's height, ?break=1 puts the column's own scrolling back (negative control)
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
const H = Number(q.get("h") ?? 700);

const CARDS = ["metadata", "keywording", "folders", "catalog", "history"] as const;
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
        <div style={{ display: "flex", flexWrap: "wrap", gap: 4 }}>{Array.from({ length: 60 }, (_, i) => <span key={i} style={{ padding: "2px 8px", background: "var(--inset)" }}>chip {i}</span>)}</div></div>)}
      {card("folders", "folders-panel", <div className="folders-body">{rows(60)}</div>)}
      {card("catalog", "catalog-panel", <div className="cat-body"><div style={{ height: 262 }}>index card</div>{rows(60)}</div>)}
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
  ["metadata", "keywording", "folders", "catalog"], [...CARDS]];

async function run() {
  await tick(5);
  const out: Record<string, unknown> = { H };
  const combos = [];
  for (const set of COMBOS) {
    setOpenFromTest(new Set(set)); await tick();
    const a = aside();
    const bodies = set.map((id) => {
      const b = body(id)!, cs = getComputedStyle(b);
      const ht = head(id);
      return { id, clientHeight: b.clientHeight, scrollHeight: b.scrollHeight, overflowY: cs.overflowY,
        headerTop: top(ht), scrolls: b.scrollHeight > b.clientHeight + 1 };
    });
    // a scrolled body keeps its header where it was
    const tall = set.find((id) => id === "folders" || id === "catalog" || id === "keywording");
    let headerStaysOnScroll = true;
    if (tall) { const t0 = top(head(tall)); body(tall)!.scrollTop = 200; headerStaysOnScroll = top(head(tall)) === t0; }
    combos.push({ set, columnScrollHeight: a.scrollHeight, columnClientHeight: a.clientHeight, bodies, headerStaysOnScroll });
  }
  out.combos = combos;
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
