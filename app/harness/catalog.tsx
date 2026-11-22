// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Dev-only harness for the Catalog UI: the real Library CatalogCard, the add and remove dialogs and
// Preferences' Catalog card with fixture data in a plain browser, no Tauri.
//
//   /harness/catalog.html?state=reading&theme=dark&w=320&h=900
//
//   view    card (default) | prefs | add | addbusy | remove
//   state   finding | findingfast | reading | reading1 | reading99 | thumbs | yielding | paused | done | doneunreadable |
//           offlinejob | failed | partial | diskfull | offline | idle | never | empty        (see catalogFixtures.ts)
//   theme   a built-in theme id (dark, graphite, slate, warm-ash, grey, classic-grey, soft-grey, light)
//   w, h    the pixel size of the page area (the card view defaults to the Library panel's 320 px)
//   break=1 adds a style that lets the card grow with its text, to prove the shift test fails
//   shift=1 runs the no-layout-shift test and writes JSON into #shift-report
import "./tauriShim";
import { createRoot } from "react-dom/client";
import { useEffect, useState } from "react";
import "../src/styles.css";
import { BUILTIN_THEMES, THEME_TOKENS } from "../src/themes";
import { CatalogCard, type CatalogActions } from "../src/CatalogCard";
import { AddToCatalogDialog, RemoveSourceDialog } from "../src/CatalogDialogs";
import { CatalogPrefsView } from "../src/CatalogPrefs";
import { STATES, STATUS, SOURCES, type Fixture } from "./catalogFixtures";
import { runShiftTest } from "./catalogShift";

const q = new URLSearchParams(location.search);
const theme = BUILTIN_THEMES.find((x) => x.id === q.get("theme")) ?? BUILTIN_THEMES[0];
for (const token of THEME_TOKENS) document.documentElement.style.setProperty(`--${token}`, theme.colors[token]);
document.documentElement.style.colorScheme = theme.colorScheme;
document.documentElement.dataset.theme = theme.id;
document.documentElement.dataset.scheme = theme.colorScheme;
document.body.style.background = "var(--window)";
if (q.get("break") === "1") {   // a deliberately bad style: the shift test must catch it (negative control)
  const st = document.createElement("style");
  st.textContent = `.cat-card { height: auto !important; min-height: 0 !important; } .cat-foot-a, .cat-foot-b { height: auto !important; white-space: normal !important; }`;
  document.head.append(st);
}
const noop = () => {};
const actions: CatalogActions = { open: noop, add: noop, index: noop, pause: noop, resume: noop, stop: noop, remove: noop, reveal: noop, dismiss: noop };

function Harness() {
  const [name, setName] = useState(q.get("state") ?? "reading");
  const [confirm, setConfirm] = useState(false);
  const [fx, setFx] = useState<Fixture>(STATES[name] ?? STATES.reading);
  const [open, setOpen] = useState(true);
  useEffect(() => { setFx(STATES[name] ?? STATES.reading); }, [name]);
  useEffect(() => {
    (window as unknown as { __h: unknown }).__h = { set: setName, confirm: setConfirm, states: Object.keys(STATES) };
    document.body.dataset.ready = "1";
    if (q.get("shift") === "1") void runShiftTest().then((r) => r, (e) => ({ ok: false, error: String(e?.stack ?? e) }))
      .then((r) => { const el = document.getElementById("shift-report")!; el.textContent = JSON.stringify(r); el.dataset.done = "1"; });
  }, []);
  const view = q.get("view") ?? "card";
  const w = Number(q.get("w")) || (view === "card" ? 320 : Math.round(window.innerWidth * 0.8) - 190);
  const h = Number(q.get("h")) || (view === "card" ? 760 : Math.round(window.innerHeight * 0.8) - 54);
  const detect = () => view === "addbusy" ? new Promise<never>(() => {}) : Promise.resolve({
    storage: "Network share, SMB (fake-nas, Photos)", kind: "nas", protocol: "SMB", host: "fake-nas", share: "Photos", fs_type: "cifs",
    mount_path: "/fake/nas", readable: true, asleep: false, overlaps: null });
  const body = view === "prefs"
    ? <div className="prefs-scroll" style={{ width: w, height: h, flex: "none", background: "var(--panel)", borderRadius: "var(--radius-panel)" }} data-harness-page>
        <CatalogPrefsView status={STATUS} sources={SOURCES()} confirming={confirm} busy={false} message="" error=""
          onAskRebuild={() => setConfirm(true)} onCancel={() => setConfirm(false)} onRebuild={noop} onTidy={noop} />
      </div>
    : view === "card"
    ? <div style={{ width: w, height: h, flex: "none", background: "var(--panel)", borderRadius: "var(--radius-panel)", padding: 8, overflow: "auto" }} data-harness-page>
        <CatalogCard open={open} onToggle={() => setOpen((o) => !o)} loaded error={null} sources={fx.sources} jobs={fx.jobs} ended={fx.ended}
          focus={fx.sources.length ? 1 : null} current={null} actions={actions} onFocus={noop} />
      </div>
    : <div style={{ width: w, height: h }} data-harness-page>
        {view === "remove"
          ? <RemoveSourceDialog source={SOURCES()[0]} onRemove={() => Promise.resolve()} onClose={noop} preview={() => Promise.resolve({ photos: 98200, dirs: 1240 })} />
          : <AddToCatalogDialog path="/fake/nas/Photos/Archive 2012-2019" sources={[]} onAdd={() => Promise.resolve()} onClose={noop} detect={detect} />}
      </div>;
  return <div style={{ position: "fixed", inset: 0, display: "grid", placeItems: "center" }}>{body}</div>;
}

createRoot(document.getElementById("root")!).render(<Harness />);
