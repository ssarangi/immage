// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Dev-only harness for the Preferences window: the real PreferencesDialog in a plain browser, no Tauri
// (every command fails, so the cards that read the engine show their "not available" states).
//
//   /harness/prefs.html?tab=general&theme=dark&dir=ltr
//
//   tab      the page to open at (default general); card: a data-pref-card to scroll to
//   theme    a built-in theme id; dir ltr | rtl
//   test=1   runs prefsTest.ts over every page and writes JSON into #prefs-report
//   break=1  restores the old structure (the scroller is the pane, flush to the modal's top edge, no header):
//            the containment test must fail
//   break=shift  lets the well take its content's height, so the footer moves from page to page: the check must fail
// The window size is the browser window's (headless Chrome --window-size).
import "./tauriShim";
import { createRoot } from "react-dom/client";
import { useEffect } from "react";
import "../src/styles.css";
import { BUILTIN_THEMES, THEME_TOKENS } from "../src/themes";
import { PreferencesDialog, type PrefsTab } from "../src/Preferences";
import { runPrefsTest } from "./prefsTest";

const q = new URLSearchParams(location.search);
const theme = BUILTIN_THEMES.find((x) => x.id === (q.get("theme") ?? "dark")) ?? BUILTIN_THEMES[0];
for (const token of THEME_TOKENS) document.documentElement.style.setProperty(`--${token}`, theme.colors[token]);
document.documentElement.style.colorScheme = theme.colorScheme;
document.documentElement.dataset.theme = theme.id;
document.documentElement.dataset.scheme = theme.colorScheme;
if (q.get("dir")) document.documentElement.dir = q.get("dir")!;
document.body.style.background = "var(--window)";
document.body.style.margin = "0";
const bad = q.get("break");
if (bad === "1") {   // the structure before the fix: the pane scrolls, flush to the modal's top and right edges
  document.head.insertAdjacentHTML("beforeend", `<style>.prefs-head { display: none !important; }
    .prefs-well { margin: 0 !important; border: 0 !important; border-radius: 0 !important; overflow: visible !important; display: flex; }
    .prefs-scroll { position: static !important; flex: 1; padding: 22px 24px 8px !important; }</style>`);
} else if (bad === "shift") {
  document.head.insertAdjacentHTML("beforeend", `<style>.prefs-well { flex: none !important; } .prefs-scroll { position: static !important; }</style>`);
}
const noop = () => {};

function Harness() {
  useEffect(() => {
    document.body.dataset.ready = "1";
    if (q.get("test") === "1") void runPrefsTest().then((r) => r, (e) => ({ ok: false, error: String(e?.stack ?? e) }))
      .then((r) => { const el = document.getElementById("prefs-report")!; el.textContent = JSON.stringify(r); el.dataset.done = "1"; });
  }, []);
  return <PreferencesDialog presets={[]} newPhotos={null} onNewPhotos={noop} onSaveCurrent={null} focus={(q.get("tab") ?? "general") as PrefsTab}
    card={q.get("card") ?? undefined} canvas={"dark" as never} onCanvas={noop} overlayColor={"red" as never} onOverlayColor={noop}
    guideColor={"white" as never} onGuideColor={noop} openGenerated={false} onOpenGenerated={noop} loupe={false} onLoupe={noop}
    focusMode={false} onFocusMode={noop} editMode={"simple" as never} onEditMode={noop} autoHide={false} onAutoHide={noop}
    stackPairs={false} onStackPairs={noop} accel={null} plugins={[]} onPlugins={noop} perfHud={false} onPerfHud={noop}
    onBenchmark={null} onGpuCheck={null} onTune={null} onRelaunch={() => Promise.resolve()} onClose={noop} />;
}

createRoot(document.getElementById("root")!).render(<Harness />);
