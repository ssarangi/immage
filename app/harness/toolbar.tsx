// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Dev-only harness for the bottom toolbar: the real Toolbar inside the real center column markup
// (section.center > .center-stage + .toolbar), over a grid surface, in a plain browser, no Tauri.
//
//   /harness/toolbar.html?view=grid&w=1280&theme=dark&dir=ltr
//
//   w      width of the center column in px (default 1280); h the height (default 700)
//   view   edit | grid | scenes
//   theme  a built-in theme id; dir ltr | rtl
//   filter 1 shows an active filter; find 1 opens Find
//   report=1 measures the bar and writes JSON into #bar-report
import "./tauriShim";
import { createRoot } from "react-dom/client";
import { useEffect, useRef, useState } from "react";
import "../src/styles.css";
import "../src/scenes.css";
import { BUILTIN_THEMES, THEME_TOKENS } from "../src/themes";
import { Toolbar, GridZoomBar, ZoomBar } from "./../src/Toolbar";
import { NO_FIND, withFilter, type FindState } from "../src/findModel";
import { DEFAULT_LAYOUT } from "../src/layoutPrefs";
import { BeforeAfterIcon, CompareIcon, PinIcon, UndoIcon } from "../src/icons";

const q = new URLSearchParams(location.search);
const theme = BUILTIN_THEMES.find((t) => t.id === q.get("theme")) ?? BUILTIN_THEMES[0];
for (const token of THEME_TOKENS) document.documentElement.style.setProperty(`--${token}`, theme.colors[token]);
document.documentElement.style.colorScheme = theme.colorScheme;
document.documentElement.dataset.theme = theme.id;
document.documentElement.dataset.scheme = theme.colorScheme;
if (q.get("dir")) document.documentElement.dir = q.get("dir") === "rtl" ? "rtl" : "ltr";
document.body.style.background = "var(--window)";
document.body.style.margin = "0";

export type Cfg = { view: "edit" | "grid" | "scenes"; cull: "none" | "pending"; filter: boolean; find: boolean };
const cfg0: Cfg = { view: (q.get("view") as Cfg["view"]) ?? "grid", cull: q.get("cull") === "pending" ? "pending" : "none",
  filter: q.get("filter") === "1", find: q.get("find") === "1" };
const noop = () => {};

function Harness() {
  const [cfg, setCfg] = useState(cfg0);
  const [width, setWidth] = useState(Number(q.get("w") ?? 1280));
  const [view, setView] = useState(cfg0.view);
  const bar = useRef<HTMLDivElement>(null);
  const grid = cfg.view !== "edit";
  const find: FindState = cfg.filter ? withFilter(NO_FIND, { ...NO_FIND.filter, minRating: 3 }) : NO_FIND;
  useEffect(() => {
    (window as unknown as { __h: unknown }).__h = { set: (p: Partial<Cfg>) => { setCfg((c) => ({ ...c, ...p })); setView((v) => p.view ?? v); },
      width: (w: number) => setWidth(w) };
    document.body.dataset.ready = "1";
  }, []);
  void view;
  return (
    <div className="app" style={{ position: "fixed", inset: 0, display: "block" }}>
      <section className="center" style={{ width, height: Number(q.get("h") ?? 700), borderRadius: 0 }}>
        <div className="center-stage">
          <main className="viewer"><div className="activities"><div className="activity">Exporting 3 photos</div></div></main>
          <div className={`strip-slot${grid ? " grid" : ""}`}>
            <div><div className={`filmstrip${grid ? " grid" : ""}`}>
              {grid && Array.from({ length: 60 }, (_, i) => <div key={i} className="thumb" style={{ height: 160, background: "#444" }} />)}
            </div></div>
          </div>
        </div>
        <Toolbar find={find} immichAvailable={false} onFind={noop} findOpen={cfg.find} rightPanel={grid ? "find" : "right"} onToggleFind={noop}
                 savedMenu={(b) => b} layout={DEFAULT_LAYOUT} ref={bar} pin={<button className="tool pin"><PinIcon on={false} /></button>}
                 viewTools={<>
                   <button className="tool"><UndoIcon /></button><button className="tool"><UndoIcon redo /></button>
                   <span className="sep" />
                   <button className="tool view"><BeforeAfterIcon /></button><button className="tool view"><CompareIcon /></button></>}
                 grid={grid} scenes={cfg.view === "scenes"} onScenes={noop} onToggleGrid={noop}
                 zoomBar={grid ? <GridZoomBar size={230} min={100} max={400} standard={230} onSize={noop} />
                   : <ZoomBar disabled={false} register={() => noop} onZoom={noop} onToggle={noop} fit={() => 1} />}
                 onLayout={noop} />
      </section>
    </div>
  );
}
createRoot(document.getElementById("root")!).render(<Harness />);

const rectOf = (e: Element) => { const r = e.getBoundingClientRect(); return { x: r.left, y: r.top, w: r.width, h: r.height, b: r.bottom, r: r.right }; };
if (q.get("report") === "1") setTimeout(() => {
  const col = document.querySelector(".center")!, bar = document.querySelector(".toolbar")!;
  const surf = document.querySelector(".filmstrip.grid");
  const out = { column: rectOf(col), bar: rectOf(bar), surface: surf ? rectOf(surf) : null,
    gap: surf ? bar.getBoundingClientRect().top - surf.getBoundingClientRect().bottom : null,
    below: col.getBoundingClientRect().bottom - bar.getBoundingClientRect().bottom,
    barScroll: [(bar as HTMLElement).scrollWidth, (bar as HTMLElement).clientWidth],
    pageScroll: [document.documentElement.scrollWidth, innerWidth] };
  const el = document.getElementById("bar-report")!; el.textContent = JSON.stringify(out); el.dataset.done = "1";
}, 400);
