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
//   tree    small (default) | big: the folder trees under the sources (catalogTreeFixtures.ts); big is 110,101 folders
//   dir     rtl lays the page out right to left
//   open    a comma list of source ids whose tree starts open (default 1)
//   fold=1  starts with the card folded (the header status), and shift=1 then runs the header test
//   shift=1 runs the no-layout-shift test and writes JSON into #shift-report (add view=tree for the folder tree's)
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
import { allOpen, bigTrees, smallTrees } from "./catalogTreeFixtures";
import { treeKey, type SourceTree } from "../src/catalogTree";
import type { TreeNode } from "../src/bindings";
import type { FolderTools } from "../src/FolderMenu";

const q = new URLSearchParams(location.search);
const theme = BUILTIN_THEMES.find((x) => x.id === q.get("theme")) ?? BUILTIN_THEMES[0];
for (const token of THEME_TOKENS) document.documentElement.style.setProperty(`--${token}`, theme.colors[token]);
document.documentElement.style.colorScheme = theme.colorScheme;
document.documentElement.dataset.theme = theme.id;
document.documentElement.dataset.scheme = theme.colorScheme;
document.body.style.background = "var(--window)";
if (q.get("dir") === "rtl") document.documentElement.dir = "rtl";
if (q.get("break") === "tree") {   // the folder tree's negative control: rows that grow with their text, a count slot that grows with its number
  const st = document.createElement("style");
  st.textContent = `.ftree .folder-row { height: auto !important; padding-block: 5px !important; } .cat-slot { width: auto !important; min-width: 0 !important; } .cat-count { position: static !important; }`;
  document.head.append(st);
}
if (q.get("break") === "1") {   // a deliberately bad style: the shift test must catch it (negative control)
  const st = document.createElement("style");
  st.textContent = `.section-status { flex: 0 0 auto !important; container-type: normal !important; } .cat-card { height: auto !important; min-height: 0 !important; } .cat-foot-a, .cat-foot-b { height: auto !important; white-space: normal !important; }`;
  document.head.append(st);
}
const noop = () => {};
const actions: CatalogActions = { open: noop, add: noop, index: noop, pause: noop, resume: noop, stop: noop, remove: noop, reveal: noop, dismiss: noop };

const tools: FolderTools = { sources: [], onCull: noop, onStats: noop, onAddToCatalog: noop, onShowInCatalog: noop, onRemoveFromCatalog: noop, inFolders: () => true, onShowInFolders: noop };
const bumpCounts = (n: TreeNode, by: number): TreeNode => ({ ...n, direct: n.direct + by, subtree: n.subtree + by });

function Harness() {
  const [name, setName] = useState(q.get("state") ?? "reading");
  const [confirm, setConfirm] = useState(false);
  const [fx, setFx] = useState<Fixture>(STATES[name] ?? STATES.reading);
  const [open, setOpen] = useState(q.get("fold") !== "1");
  const big = q.get("tree") === "big";
  const [trees, setTrees] = useState<Map<number, SourceTree>>(() => (big ? bigTrees() : smallTrees()));
  const [expanded, setExpanded] = useState<Set<string>>(() => {
    if (big && q.get("expand") === "all") return allOpen(bigTrees().get(1)!, 1);
    const ids = (q.get("open") ?? "1").split(",").filter(Boolean).map(Number);
    return new Set(ids.map((id) => treeKey(id, "")).concat(q.get("deep") === "1" ? [treeKey(1, "2019"), treeKey(1, "2019/Iceland")] : []));
  });
  const [selected, setSelected] = useState<string | null>(q.get("select"));
  const [scrollTo, setScrollTo] = useState<{ key: string; n: number } | null>(null);
  useEffect(() => { setFx(STATES[name] ?? STATES.reading); }, [name]);
  useEffect(() => {
    (window as unknown as { __h: unknown }).__h = {
      set: setName, confirm: setConfirm, open: setOpen, states: Object.keys(STATES),
      // the tree: open or close a folder, select one, change a count in place, add a folder under a source's root
      expand: (keys: string[], on = true) => setExpanded((old) => { const n = new Set(old); for (const k of keys) { if (on) n.add(k); else n.delete(k); } return n; }),
      select: setSelected,
      scrollTo: (key: string) => setScrollTo((o) => ({ key, n: (o?.n ?? 0) + 1 })),
      setCount: (id: number, rel: string, parent: string, count: number) => setTrees((old) => {
        const t = new Map(old), src = old.get(id)!, kids = new Map(src.kids);
        kids.set(parent, (kids.get(parent) ?? []).map((n) => (n.rel === rel ? { ...n, direct: count, subtree: count } : n)));
        t.set(id, { ...src, kids });
        return t;
      }),
      addFolder: (id: number, parent: string, name: string, count: number, status = "ok") => setTrees((old) => {
        const t = new Map(old), src = old.get(id)!, kids = new Map(src.kids);
        const rel = parent ? `${parent}/${name}` : name;
        kids.set(parent, [...(kids.get(parent) ?? []), { parent, rel, name, has_children: false, direct: count, subtree: count, status }]);
        const root = src.root ? { ...bumpCounts(src.root, count), has_children: true } : null;
        t.set(id, { ...src, kids, root });
        return t;
      }),
      clear: (id: number) => setTrees((old) => new Map(old).set(id, { generation: null, root: null, kids: new Map() })),
      setStatus: (id: number, rel: string, parent: string, status: string) => setTrees((old) => {
        const t = new Map(old), src = old.get(id)!, kids = new Map(src.kids);
        kids.set(parent, (kids.get(parent) ?? []).map((n) => (n.rel === rel ? { ...n, status } : n)));
        t.set(id, { ...src, kids });
        return t;
      }),
      grow: (id: number) => setTrees((old) => {
        const t = new Map(old), src = old.get(id)!, kids = new Map(src.kids);
        kids.set("", (kids.get("") ?? []).map((n) => bumpCounts(n, 7)));
        t.set(id, { ...src, kids, generation: (src.generation ?? 0) + 1 });
        return t;
      }),
    };
    document.body.dataset.ready = "1";
    if (q.get("shift") === "1") void runShiftTest().then((r) => r, (e) => ({ ok: false, error: String(e?.stack ?? e) }))
      .then((r) => { const el = document.getElementById("shift-report")!; el.textContent = JSON.stringify(r); el.dataset.done = "1"; });
  }, []);
  const view = q.get("view") ?? "card";
  const cardView = view === "card" || view === "tree";
  const w = Number(q.get("w")) || (cardView ? 320 : Math.round(window.innerWidth * 0.8) - 190);
  const h = Number(q.get("h")) || (cardView ? 760 : Math.round(window.innerHeight * 0.8) - 54);
  const detect = () => view === "addbusy" ? new Promise<never>(() => {}) : Promise.resolve({
    storage: "Network share, SMB (fake-nas, Photos)", kind: "nas", protocol: "SMB", host: "fake-nas", share: "Photos", fs_type: "cifs",
    mount_path: "/fake/nas", readable: true, asleep: false, overlaps: null });
  const body = view === "prefs"
    ? <div className="prefs-scroll" style={{ width: w, height: h, flex: "none", background: "var(--panel)", borderRadius: "var(--radius-panel)" }} data-harness-page>
        <CatalogPrefsView status={STATUS} sources={SOURCES()} confirming={confirm} busy={false} message="" error=""
          onAskRebuild={() => setConfirm(true)} onCancel={() => setConfirm(false)} onRebuild={noop} onTidy={noop} />
      </div>
    : cardView
    ? <div style={{ width: w, height: h, flex: "none", background: "var(--panel)", borderRadius: "var(--radius-panel)", padding: 8, overflow: "auto" }} data-harness-page>
        <CatalogCard open={open} onToggle={() => setOpen((o) => !o)} loaded error={null} sources={fx.sources} jobs={fx.jobs} ended={fx.ended}
          focus={fx.sources.length ? 1 : null} actions={actions} onFocus={noop}
          tree={{
            trees, expanded, includeSub: q.get("sub") !== "0", selectedKey: selected, alsoPath: q.get("also"), scrollTo, tools,
            onToggle: (key, on) => setExpanded((old) => { const n = new Set(old); if (on) n.add(key); else n.delete(key); return n; }),
            onSelect: setSelected, onCheck: noop,
          }} />
      </div>
    : <div style={{ width: w, height: h }} data-harness-page>
        {view === "remove"
          ? <RemoveSourceDialog source={SOURCES()[0]} onRemove={() => Promise.resolve()} onClose={noop} preview={() => Promise.resolve({ photos: 98200, dirs: 1240 })} />
          : <AddToCatalogDialog path="/fake/nas/Photos/Archive 2012-2019" sources={[]} onAdd={() => Promise.resolve()} onClose={noop} detect={detect} />}
      </div>;
  return <div style={{ position: "fixed", inset: 0, display: "grid", placeItems: "center" }}>{body}</div>;
}

createRoot(document.getElementById("root")!).render(<Harness />);
