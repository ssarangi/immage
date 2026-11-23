// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Dev-only harness for the Library panel's expand behaviour: the real `.left` aside, `.panel` cards, tree bodies
// and styles.css with fixture content, driven by the real toggleLeftSet, headerKey and useHeightVar. With ?test=1
// it clicks the headers and writes JSON into #shift-report (verify/left-panel-shift.test.mjs asserts it).
//   ?h=700    the aside's height, ?break=1 removes the height cap (negative control)
import "./tauriShim";
import { createRoot } from "react-dom/client";
import { useRef, useState } from "react";
import "../src/styles.css";
import { BUILTIN_THEMES, THEME_TOKENS } from "../src/themes";
import { headerKey } from "../src/CatalogCard";
import { toggleLeftSet, useHeightVar } from "../src/panelToggle";

const q = new URLSearchParams(location.search);
const theme = BUILTIN_THEMES[0];
for (const token of THEME_TOKENS) document.documentElement.style.setProperty(`--${token}`, theme.colors[token]);
document.documentElement.style.colorScheme = theme.colorScheme;
document.body.style.background = "var(--window)";
document.body.style.margin = "0";
if (q.get("break") === "1") {
  const st = document.createElement("style");
  st.textContent = `.folders-body, .cat-body { max-height: none !important; overflow: visible !important; }`;
  document.head.append(st);
}
const H = Number(q.get("h") ?? 700);

const CARDS = ["metadata", "keywording", "folders", "catalog", "history"] as const;
type Card = typeof CARDS[number];
const rows = (n: number) => Array.from({ length: n }, (_, i) => <div key={i} style={{ height: 28, padding: "0 12px" }}>row {i}</div>);

function Harness() {
  const aside = useRef<HTMLElement>(null);
  const [open, setOpen] = useState<Set<Card>>(new Set(["folders", "catalog"]));
  useHeightVar(aside, "--left-h");
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
    <aside className="left" ref={aside} style={{ width: 300, height: H }} data-aside>
      {card("metadata", "", <div style={{ height: 160 }}>meta</div>)}
      {card("keywording", "", <div style={{ height: 120 }}>kw</div>)}
      {card("folders", "folders-panel", <div className="folders-body">{rows(60)}</div>)}
      {card("catalog", "catalog-panel", <div className="cat-body"><div style={{ height: 262 }}>index card</div>{rows(60)}</div>)}
      {card("history", "", <div style={{ height: 200 }}>history</div>)}
    </aside>
  );
}
createRoot(document.getElementById("root")!).render(<Harness />);

const tick = (n = 3) => new Promise<void>((done) => { setTimeout(done, 40 * n); });
const aside = () => document.querySelector<HTMLElement>("[data-aside]")!;
const head = (id: string) => document.querySelector<HTMLElement>(`[data-card=${id}] .panel-title`)!;
const body = (id: string) => document.querySelector<HTMLElement>(`[data-card=${id}] :is(.folders-body, .cat-body)`);
const top = (el: Element) => Math.round(el.getBoundingClientRect().top * 100) / 100;
const openSet = () => CARDS.filter((c) => head(c).getAttribute("aria-expanded") === "true");

async function run() {
  await tick(5);
  const out: Record<string, unknown> = {};
  const cap = Math.max(140, aside().clientHeight * 0.38);
  const fb = body("folders")!, cb = body("catalog")!;
  out.cap = cap;
  out.both = { open: openSet(), folders: fb.clientHeight, catalog: cb.clientHeight,
    foldersScrolls: fb.scrollHeight > fb.clientHeight, catalogScrolls: cb.scrollHeight > cb.clientHeight };
  fb.scrollTop = 120;
  out.innerScrollable = fb.scrollTop === 120;
  // a plain click on a card opens only that card, and never shuts Folders or Catalog
  head("metadata").click(); await tick();
  head("keywording").click(); await tick();
  out.afterOpenTwo = openSet();
  // each header stays where it was under the pointer, and the panel does not scroll
  const moves: Record<string, number> = {};
  let scrollDelta = 0;
  for (const id of ["history", "keywording", "metadata", "history", "keywording", "metadata"]) {
    const s0 = aside().scrollTop, t0 = top(head(id));
    head(id).click(); await tick();
    moves[`${id}:${openSet().includes(id as Card) ? "open" : "shut"}`] = Math.abs(top(head(id)) - t0);
    scrollDelta = Math.max(scrollDelta, Math.abs(aside().scrollTop - s0));
  }
  out.headerMoves = Object.values(moves).reduce((a, b) => Math.max(a, b), 0);
  out.scrollDelta = scrollDelta;
  // Enter and Space toggle, with no Alt
  const h = head("history");
  h.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true })); await tick();
  const afterEnter = h.getAttribute("aria-expanded");
  h.dispatchEvent(new KeyboardEvent("keydown", { key: " ", bubbles: true })); await tick();
  out.keys = [afterEnter, h.getAttribute("aria-expanded")];
  out.roles = CARDS.every((c) => head(c).getAttribute("role") === "button" && head(c).hasAttribute("aria-expanded"));
  // one open tree takes the panel's own scroll, no cap
  head("catalog").click(); await tick();
  const single = body("folders")!;
  out.single = { open: openSet(), capped: single.clientHeight <= cap + 0.5, scrollHeight: single.scrollHeight, clientHeight: single.clientHeight };
  head("catalog").click(); await tick();
  out.reopened = openSet();
  document.getElementById("shift-report")!.textContent = JSON.stringify(out);
}
if (q.get("test") === "1") void run().catch((e) => { document.getElementById("shift-report")!.textContent = JSON.stringify({ error: String(e) }); });
