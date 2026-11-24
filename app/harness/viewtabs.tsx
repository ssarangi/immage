// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Dev-only harness for the window's top strip: the real TitleStrip (title, menus, view tabs)
// in a plain browser, no Tauri.
//
//   /harness/viewtabs.html?w=900&theme=dark&dir=ltr
//
//   w       width of the window in px (default 1280)
//   mac=1   the macOS strip (no menus)
//   four=1  a 4th tab, as People will be
//   test=1  runs the checks and writes JSON into #tabs-report
//   break=drag     puts a drag region over the tabs (the checks must fail)
//   break=underline  adds a bar under the tabs (the checks must fail)
//   break=overlap  moves the tabs over the menus (the checks must fail)
import "./tauriShim";
import { createRoot } from "react-dom/client";
import { useEffect, useState } from "react";
import "../src/styles.css";
import { BUILTIN_THEMES, THEME_TOKENS } from "../src/themes";
import { TitleStrip, VIEW_TABS, type ViewMode, type ViewTab } from "../src/ViewTabs";
import { MENUS } from "../src/menu";
import { EditViewIcon } from "../src/icons";

const q = new URLSearchParams(location.search);
const theme = BUILTIN_THEMES.find((t) => t.id === q.get("theme")) ?? BUILTIN_THEMES[0];
for (const token of THEME_TOKENS) document.documentElement.style.setProperty(`--${token}`, theme.colors[token]);
document.documentElement.style.colorScheme = theme.colorScheme;
document.documentElement.dataset.theme = theme.id;
document.documentElement.dataset.scheme = theme.colorScheme;
if (q.get("dir")) document.documentElement.dir = q.get("dir") === "rtl" ? "rtl" : "ltr";
document.body.style.cssText = "margin:0;background:var(--window)";
const mac = q.get("mac") === "1";
const menus = MENUS.map((m) => ({ ...m, items: m.items.filter((i) => i.kind !== "role") }));
const noop = () => {};
const brk = q.get("break");
if (brk === "underline") {
  const st = document.createElement("style");
  st.textContent = ".view-tab::after { content: \"\"; position: absolute; inset-inline: 10px; bottom: 0; height: 2px; background: currentColor; }";
  document.head.append(st);
}
if (brk === "overlap") {
  const st = document.createElement("style");
  st.textContent = ".titlebar-tabs { position: static; } .view-tabs { position: absolute; left: 120px; }";
  document.head.append(st);
}

type H = { set: (p: { view?: ViewMode; canScenes?: boolean }) => void; width: (w: number) => void; four: (on: boolean) => void; badge: (on: boolean) => void };
const four = (badge: boolean): ViewTab[] => [...VIEW_TABS, { badge, id: "people" as ViewMode, label: "viewTabs.grid", title: "viewTabs.gridTitle", icon: EditViewIcon, shortcut: "⇧P" }];

function Harness() {
  const [view, setView] = useState<ViewMode>("grid");
  const [canScenes, setCan] = useState(true);
  const [width, setWidth] = useState(Number(q.get("w") ?? 1280));
  const [badge, setBadge] = useState(false);
  const [more, setMore] = useState(q.get("four") === "1");
  useEffect(() => {
    const h: H = { set: (p) => { if (p.view) setView(p.view); if (p.canScenes !== undefined) setCan(p.canScenes); }, width: setWidth, four: setMore, badge: setBadge };
    (window as unknown as { __h: H }).__h = h;
    document.body.dataset.ready = "1";
    if (q.get("test") === "1") void import("./viewtabsTest").then((m) => m.runViewTabsTest(h))
      .then((r) => r, (e) => ({ ok: false, error: String(e?.stack ?? e) }))
      .then((r) => { const el = document.getElementById("tabs-report")!; el.textContent = JSON.stringify(r); el.dataset.done = "1"; });
  }, []);
  return (
    <div className="window" style={{ position: "fixed", top: 0, left: 0, width, height: 120 }}>
      <TitleStrip mac={mac} menus={menus} disabled={new Set()} onAction={noop} view={view} onView={setView} canScenes={canScenes} tabs={more ? four(badge) : undefined} />
      {brk === "drag" && <span data-tauri-drag-region style={{ position: "absolute", top: 0, left: 0, right: 0, height: 34 }} />}
    </div>
  );
}
createRoot(document.getElementById("root")!).render(<Harness />);
