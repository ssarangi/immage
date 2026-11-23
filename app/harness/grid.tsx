// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Dev-only harness for a large folder: the real Filmstrip and ThumbLoader over synthetic photos (fake
// paths, thumbnails drawn here: nothing on disk is read), in a plain browser, no Tauri.
//
//   /harness/grid.html?n=9653&mode=grid&theme=dark
//
//   n        photos (default 9653)
//   mode     grid (default) | strip
//   latency  milliseconds each cached thumbnail takes to arrive (default 2)
//   preload  1 asks for every photo's thumbnail at once, as the app once did
//   theme    a built-in theme id
//
// window.__h: `select(i)` and `open(i)` change the props the way a click does, `scrollTo(px)` scrolls the
// view, `stats()` reports the DOM's size and the loader's progress, and `longTasks` lists the main thread's
// long tasks. verify/large-grid.test.mjs reads them.
import "./tauriShim";
import { createRoot } from "react-dom/client";
import { useEffect, useMemo, useState } from "react";
import "../src/styles.css";
import { BUILTIN_THEMES, THEME_TOKENS } from "../src/themes";
import { Filmstrip } from "../src/Filmstrip";
import { ThumbLoader, byCaptureTime, type ImageEntry } from "../src/library";

const q = new URLSearchParams(location.search);
const theme = BUILTIN_THEMES.find((t) => t.id === q.get("theme")) ?? BUILTIN_THEMES[0];
for (const token of THEME_TOKENS) document.documentElement.style.setProperty(`--${token}`, theme.colors[token]);
document.documentElement.style.colorScheme = theme.colorScheme;
document.documentElement.dataset.theme = theme.id;
document.documentElement.dataset.scheme = theme.colorScheme;
document.body.style.background = "var(--window)";
document.body.style.margin = "0";

const N = Number(q.get("n") ?? 9653);
const GRID = q.get("mode") !== "strip";
const LATENCY = Number(q.get("latency") ?? 2);
const pad = (n: number, w = 2) => String(n).padStart(w, "0");
const entries: ImageEntry[] = byCaptureTime(Array.from({ length: N }, (_, i) => ({
  imgid: i + 1, path: `/synthetic/f${pad(i % 32)}/IMG_${pad(i + 1, 6)}.ARW`, filename: `IMG_${pad(i + 1, 6)}.ARW`,
  rating: i % 7 === 0 ? 3 : 0, rejected: false, labels: 0, edited: i % 50 === 0, width: 6000, height: 4000, version: 0,
  exif: { maker: "", model: "X", lens: "", exposure: 0.01, aperture: 2.8, iso: 100, focal_length: 35, exposure_bias: 0,
          focus_distance: 0, datetime: `2025:${pad(1 + (i % 12))}:${pad(1 + (i % 28))} 10:${pad(i % 60)}:00` },
} as ImageEntry)));

// every thumbnail is a small drawn JPEG, served after LATENCY ms, as thumb_cached would
const jpeg = (() => {
  const c = document.createElement("canvas"); c.width = 240; c.height = 160;
  const g = c.getContext("2d")!; g.fillStyle = "#789"; g.fillRect(0, 0, 240, 160);
  const bin = atob(c.toDataURL("image/jpeg").split(",")[1]);
  return Uint8Array.from(bin, (ch) => ch.charCodeAt(0)).buffer;
})();
const asked = new Set<number>();
(window as unknown as { __TAURI_INTERNALS__: { invoke: unknown } }).__TAURI_INTERNALS__.invoke = (cmd: string) => {
  if (cmd === "thumb_cached" || cmd === "thumbCached") return new Promise((r) => setTimeout(() => r(jpeg.slice(0)), LATENCY));
  return Promise.reject(new Error("harness: no Tauri"));
};

const longTasks: number[] = [];
new PerformanceObserver((l) => l.getEntries().forEach((e) => longTasks.push(Math.round(e.duration)))).observe({ entryTypes: ["longtask"] });

const byId = new Map(entries.map((i) => [i.imgid, i]));
const loader = new ThumbLoader((id) => byId.get(id), () => 512 * 1024 * 1024);
const h = { loader, longTasks, select: (_: number) => {}, open: (_: number) => {}, scrollTo: (_: number) => {},
  stats: () => ({ nodes: document.getElementsByTagName("*").length, cells: document.querySelectorAll(".thumb").length,
                  ...loader.progress(), asked: asked.size }) };
(window as unknown as { __h: typeof h }).__h = h;

function Harness() {
  const [selection, setSelection] = useState(new Set<number>());
  const [current, setCurrent] = useState<number | null>(null);
  const [gridSize, setGridSize] = useState(200);
  const partners = useMemo(() => new Map(), []);
  useEffect(() => {
    h.select = (i) => setSelection(new Set([entries[i].imgid, entries[i + 1].imgid]));
    h.open = (i) => setCurrent(entries[i].imgid);
    h.scrollTo = (px) => { const r = document.querySelector(".filmstrip")!; if (GRID) r.scrollTop = px; else r.scrollLeft = px; };
    // the app once asked for every photo's thumbnail as it read the folder; now the cells in view do
    if (q.get("preload") === "1") loader.request(entries.map((i) => i.imgid));
  }, []);
  return (
    <div style={{ width: 1200, height: 800, display: "flex", flexDirection: "column" }}>
      <Filmstrip images={entries} partners={partners} pending={[]} empty="" current={current} loader={loader}
                 height={150} grid={GRID} gridSize={gridSize}
                 onGridZoom={(f) => setGridSize((g) => Math.max(100, Math.min(400, g * f)))} selection={selection} candidate={null}
                 onPick={() => {}} onDouble={() => {}} onOpenKnown={() => {}} onMenu={() => {}} onMeta={() => {}}
                 onBackMenu={() => {}} />
    </div>
  );
}
createRoot(document.getElementById("root")!).render(<Harness />);

// check=1: scroll through the list and compare what is drawn with where CSS puts photo k
// (the real column count and gap, read from the grid's computed style), into #grid-report
async function check() {
  const frame = () => new Promise<void>((r) => setTimeout(r, 100));
  for (let k = 0; k < 100 && !document.querySelector(".filmstrip .thumb"); k++) await frame();
  const root = document.querySelector<HTMLElement>(".filmstrip")!;
  const st = getComputedStyle(root);
  const rows = GRID ? st.gridTemplateColumns.split(" ").length : 1;
  const gap = parseFloat(GRID ? st.rowGap : st.columnGap), pad = parseFloat(GRID ? st.paddingTop : st.paddingLeft);
  const first = root.querySelector<HTMLElement>(".thumb")!.getBoundingClientRect();
  const cellMain = GRID ? first.height : first.width;
  const pitch = cellMain + gap;
  const report = { n: N, grid: GRID, cols: rows, scrollExtent: 0, expectedExtent: 0, maxCells: 0, maxNodes: 0, positions: [] as unknown[], missing: 0, misplaced: 0, extra: 0 };
  const total = Math.ceil(N / rows);
  report.expectedExtent = Math.round(2 * pad + total * pitch - gap);
  const main = GRID ? root.clientHeight : root.clientWidth;
  for (const frac of [0, 0.0003, 0.37, 0.999, 1]) {
    const max = (GRID ? root.scrollHeight - root.clientHeight : root.scrollWidth - root.clientWidth);
    const pos = Math.round(max * frac);
    if (GRID) root.scrollTop = pos; else root.scrollLeft = pos;
    let settled = 0;
    let missing = 0, misplaced = 0, firstMissing = -1;
    // the cells follow the scroll a frame or two later, as they do for a person
    for (let tries = 0; tries < 20; tries++) {
      await frame();
      settled = tries;
    const rect = root.getBoundingClientRect();
    const at = GRID ? root.scrollTop : root.scrollLeft;
    report.scrollExtent = GRID ? root.scrollHeight : root.scrollWidth;
    const present = new Map<string, DOMRect>();
    root.querySelectorAll<HTMLElement>(".thumb").forEach((e) => present.set(e.title, e.getBoundingClientRect()));
    report.maxCells = Math.max(report.maxCells, present.size);
    report.maxNodes = Math.max(report.maxNodes, document.getElementsByTagName("*").length);
    missing = 0; misplaced = 0; firstMissing = -1;
    for (let k = 0; k < N; k++) {
      const line = Math.floor(k / rows), top = pad + line * pitch;
      if (top > at + main + pitch) break;
      if (top + cellMain < at) { k = (line + 1) * rows - 1; continue; }
      const seen = top < at + main && top + cellMain > at;
      const c = present.get(entries[k].filename);
      if (!c) { if (seen) { missing++; if (firstMissing < 0) firstMissing = k; } continue; }
      const expectTop = GRID ? rect.top + top - at : rect.left + top - at;
      if (Math.abs((GRID ? c.top : c.left) - expectTop) > 1.5) misplaced++;
    }
      if (!missing) break;
    }
    report.missing += missing; report.misplaced += misplaced;
    report.positions.push({ at: pos, missing, firstMissing, settled });
  }
  const pre = document.createElement("pre");
  pre.id = "grid-report"; pre.hidden = true; pre.textContent = JSON.stringify(report);
  document.body.append(pre);
}
if (q.get("check") === "1") {
  check().catch((e) => {
    const pre = document.createElement("pre");
    pre.id = "grid-report"; pre.hidden = true; pre.textContent = JSON.stringify({ error: String(e) });
    document.body.append(pre);
  });
}
