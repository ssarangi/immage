// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Dev-only harness for the Statistics view: the real component on a mock of the stats commands (statsMock.ts),
// in a plain browser, no Tauri.
//
//   /harness/statistics.html?state=ready&theme=light&filter=focal:normal,month:10&scrollTo=lenses
//
//   state   ready | loading | empty (a filter that matches nothing) | error | firstrun
//   cov     indexing (default) | ready | paused: the coverage banner
//   theme   a built-in theme id (dark default, graphite, slate, warm-ash, grey, classic-grey, soft-grey, light)
//   filter  key:value pairs, comma separated: maker, body, lens, year, month, hour, focal, aperture, shutter, iso, label, top
//   scope   library (default) | range (2024-01-01 to 2024-12-31)
//   min     the top-images threshold (default 4)
//   lib     big (default: about 47,000 photos) | gt (the 400 photos of the ground-truth library); both are the
//           real backend's replies, recorded into stats-fixtures/ (statsMock.ts)
//   delay   milliseconds each query takes (default 0)
//   scrollTo  cameras | lenses | exposure | time | places | top | overview;  y=<px>
//   top     the Top images section: error | loading | pagefail (a later page fails once) | changed (a cursor is refused once) | slow (pages take 600 ms);
//           thumbs=none: no thumbnails (the offline placeholder)
//   gps     none: no photo has a position (Places' empty state);  tiles=offline: every map tile fails (the grid fallback)
//           filter=place:<quadkey> selects a map area (the biggest at the first zooms were recorded)
//   shift=1 runs the no-layout-shift test and writes JSON into #report (see shiftTest.ts)
import "./tauriShim";
import { createRoot } from "react-dom/client";
import { Suspense, lazy, useEffect } from "react";
import "@fontsource/poppins/400.css";
import "@fontsource/poppins/500.css";
import "@fontsource/poppins/600.css";
import "@fontsource/poppins/700.css";
import "../src/styles.css";
import { BUILTIN_THEMES, THEME_TOKENS } from "../src/themes";
import { LOCALES } from "../src/i18n";
import { initialState, setScope, setValues, setOnlyTop } from "../src/statsModel";
import { resetStatsState } from "../src/statsStore";
import { createMockApi } from "./statsMock";
import { runStatsShiftTest } from "./statsShift";

// the catalog keys are merged into en.json at integration; until then they are read from here
const staged = import.meta.glob("./stats-en.json", { eager: true, import: "default" });
for (const dict of Object.values(staged)) Object.assign(LOCALES.en.dict, dict as Record<string, string>);

const q = new URLSearchParams(location.search);
const theme = BUILTIN_THEMES.find((t) => t.id === q.get("theme")) ?? BUILTIN_THEMES[0];
for (const token of THEME_TOKENS) document.documentElement.style.setProperty(`--${token}`, theme.colors[token]);
document.documentElement.style.colorScheme = theme.colorScheme;
document.documentElement.dataset.theme = theme.id;
document.documentElement.dataset.scheme = theme.colorScheme;
document.body.style.background = "var(--window)";
document.body.style.overflow = "hidden";
// negative control for verify/stats-shift.test.mjs: a hover and a selected style that move layout, which the test must catch
if (q.get("bad") === "1") {
  const st = document.createElement("style");
  st.textContent = ".st-stat:hover, .st-stat:focus-visible { padding-top: 22px !important; } .st-row:hover, .st-row.on, .st-pc:hover { margin-top: 3px !important; }";
  document.head.appendChild(st);
}

// the same for the Exposure and Time charts (bad=2): a ring, a shutter bar, an ISO bin and a month that grow on hover
if (q.get("bad") === "2") {
  const st = document.createElement("style");
  st.textContent = ".st-iris-cell:hover, .st-shutter-col:hover, .st-iso-col:hover, .st-month:hover { padding-top: 12px !important; } .st-month[aria-pressed=true] { border-width: 3px !important; }";
  document.head.appendChild(st);
}

// the Top images section (bad=3): a card that moves on hover, a label chip that grows when picked, a strip card that grows on focus
if (q.get("bad") === "3") {
  const st = document.createElement("style");
  st.textContent = ".st-tc:hover { margin-top: 4px !important; } .st-lbl[aria-pressed=true] { border-width: 4px !important; } .st-tcs-im:focus-visible { padding-top: 9px !important; }";
  document.head.appendChild(st);
}

const DIM: Record<string, Parameters<typeof setValues>[1]> = { maker: "makers", body: "bodies", lens: "lenses", year: "years", month: "months", hour: "hours",
  focal: "focalBuckets", aperture: "apertureGroups", shutter: "shutterRegimes", iso: "isoBins", label: "labels", place: "place" };
let state = initialState(Number(q.get("min") ?? 4));
if (q.get("scope") === "range") state = setScope(state, { kind: "dateRange", from: "2024-01-01", to: "2024-12-31" });
const picked: Record<string, string[]> = {};
for (const pair of (q.get("filter") ?? "").split(",").filter(Boolean)) {
  const i = pair.indexOf(":");
  const k = pair.slice(0, i), v = pair.slice(i + 1);
  if (k === "top") state = setOnlyTop(state, true);
  else if (DIM[k]) (picked[k] ??= []).push(v);
}
for (const [k, v] of Object.entries(picked)) state = setValues(state, DIM[k], ["year", "month", "hour"].includes(k) ? v.map(Number) : v);
// a state that matches nothing
if (q.get("state") === "empty") state = setValues(state, "makers", ["Nobody"]);
resetStatsState(state);

const api = createMockApi({ lib: q.get("lib") ?? "big", state: q.get("state") ?? "ready", cov: q.get("cov") ?? "indexing", delayMs: Number(q.get("delay") ?? 0), gps: q.get("gps") ?? "", top: q.get("top") ?? "", thumbs: q.get("thumbs") ?? "" });
const Statistics = lazy(() => import("../src/Statistics"));

function Harness() {
  useEffect(() => {
    const at = q.get("scrollTo"), y = q.get("y");
    const go = () => {
      const sc = document.querySelector<HTMLElement>(".st-scroll");
      if (!sc) return;
      if (at && at !== "overview") { const nav = document.querySelector<HTMLElement>(".st-nav"); const el = document.getElementById(at);
        if (el) sc.scrollTop += el.getBoundingClientRect().top - sc.getBoundingClientRect().top - (nav?.offsetHeight ?? 0) + Number(q.get("o") ?? 0); }
      else if (y) sc.scrollTop = Number(y);
    };
    const t = setTimeout(go, 700);
    if (q.get("shift") === "1") setTimeout(() => { void runStatsShiftTest().then((r) => { document.getElementById("report")!.textContent = JSON.stringify(r); }); }, 1200);
    return () => clearTimeout(t);
  }, []);
  return (
    <>
      <div style={{ height: 34, display: "flex", alignItems: "center", gap: 16, padding: "0 12px", color: "var(--text-2)", fontSize: 13 }}>
        <b style={{ color: "var(--text)" }}>Immage</b><span>File</span><span>Edit</span><span>Photo</span><span>View</span><span>Help</span>
      </div>
      <Suspense fallback={null}>
        <Statistics api={api} onClose={() => {}} onOpenInGrid={(ids, title) => { document.title = `grid:${ids.length}:${title}`; }}
                    onOpenPhoto={(p) => { document.title = `photo:${p.path}`; }} onAddFolders={() => { document.title = "add-folders"; }} currentFolder="/photos/2024" />
      </Suspense>
    </>
  );
}
createRoot(document.getElementById("root")!).render(<Harness />);
