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
import { peopleApi, resetPeople, setPeopleApi, setPeople, type PeopleApi } from "../src/peopleStore";
import type { FacesSettings, FacesStats, FacesStatus } from "../src/bindings";



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

// the Faces page on a fake face store whose numbers change (window.__faces.bump), in a plain browser
let bumped = 0;
const stat = (): FacesStats => ({
  store: { files: 4210 + bumped * 1000, photos: 4100 + bumped * 1000, faces: 9120 + bumped * 5000, people: 14 + bumped, ignored_people: 1, groups: 37 + bumped * 100, unassigned: 1820, rejected: 3,
           failed: 34, db_bytes: 38_000_000 + bumped * 4_000_000, models: [] },
  crops_bytes: 120_000_000 + bumped * 60_000_000, crops_files: 9000, recognizer: "sface", model_version: "sface-2021dec", pack_installed: true, eyes_available: false, arcface_available: false,
});
const status = (): FacesStatus => ({
  enabled: true, recognizer: "sface", installed: true, eyes_available: false,
  totals: { photos: stat().store.photos, faces: stat().store.faces, people: stat().store.people, unreviewed: 40, groups: 37, suggestions: 3, unknown: 1820, ignored_faces: 3, ignored_people: 1, failed_photos: 34 },
  scanned: 0, faces_found: 0, pending: 0, folders_waiting: 0, offline_folders: 0, skipped_network_sources: 2, infer_ms: null, health: "ok", failure: null, undo: null,
});
let cfg: FacesSettings = { enabled: true, scope: "folder", strictness: "balanced", recognizer: "sface", detect_size: 1024 };
const calls: string[] = [];
const fakes: Partial<PeopleApi> = {
  settings: () => Promise.resolve({ ...cfg }),
  setStrictness: (level) => { calls.push(`strictness:${level}`); cfg = { ...cfg, strictness: level }; return Promise.resolve({ ...cfg }); },
  setDetectSize: (size) => { calls.push(`detect:${size}`); cfg = { ...cfg, detect_size: size }; return Promise.resolve({ ...cfg }); },
  reprocess: () => { calls.push("reprocess"); return Promise.resolve(4100); },
  status: () => Promise.resolve(status()), paused: () => Promise.resolve(false), stats: () => Promise.resolve(stat()), people: () => Promise.resolve([]), groups: () => Promise.resolve([]),
  list: () => Promise.resolve([]), mergeSuggestions: () => Promise.resolve([]), failedPhotos: (n) => Promise.resolve(Array.from({ length: Math.min(n, 34) }, (_, i) => `/synthetic/broken_${i}.jpg`)),
  deleteAll: () => { (window as unknown as { __faces: { deleted: number } }).__faces.deleted++; return Promise.resolve(); }, setEnabled: () => Promise.resolve(),
  subscribe: () => () => {},
};
if (q.get("faces") === "1") {
  setPeopleApi(fakes as PeopleApi);
  (window as unknown as { __faces: unknown }).__faces = { bump: () => { bumped++; setPeople({ tick: Math.random() }); }, deleted: 0, calls, api: peopleApi };
  resetPeople({ on: false });
  void fakes.status!().then((s) => setPeople({ status: s, loaded: true }));
}

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
