// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Catalog's seam with the editor shell: one hook that gives App the Library card, the folder
// menu's tools and the two dialogs, so App.tsx carries a few lines and the Catalog stays here. The
// store (catalogStore.ts) holds the state; what a click does in the editor (open a folder in the
// grid, start a cull, open Statistics) is passed in.
import { useMemo, useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { browseFolder } from "./FolderBrowser";
import { t } from "./i18n";
import { CatalogCard, type CatalogActions } from "./CatalogCard";
import { AddToCatalogDialog, RemoveSourceDialog } from "./CatalogDialogs";
import type { FolderTools } from "./FolderMenu";
import { folderCatalog, sourceRoot } from "./catalogModel";
import { folderPath, parseKey, relUnder, revealKeys, treeKey } from "./catalogTree";
import { getCatalogTree, openAll, selectOffline, setOpen, useCatalogTreeState, useCatalogTreeSync } from "./catalogTreeStore";
import { addToCatalog, dismissEnded, getCatalog, indexSource, pauseSource, refresh, removeFromCatalog, resumeSource, stopSource, useCatalog } from "./catalogStore";
import type { SourceStatus } from "./bindings";

export function useCatalogUi(env: {
  /** show a folder's photos in the grid (opened from the Catalog tree) */
  openFolder(path: string): void;
  cullFolder(path: string): void;
  statsFolder(path: string): void;
  /** the open folder, and which tree it was opened from */
  current: string | null;
  origin: "folders" | "catalog";
  /** the Folders tree's "Include subfolders": a folder's count follows it */
  includeSub: boolean;
  /** the Folders list has a root above this path / brings its row into view */
  inFolders(path: string): boolean;
  showInFolders(path: string): void;
  /** notes that the folder a tool is about to open came from the Catalog tree */
  fromCatalog(path: string): void;
  /** opens the Catalog card when it is folded */
  showCard(): void;
  /** the card is open (the trees are only kept current while it is) */
  cardOpen: boolean;
  onError(message: string): void;
}) {
  const cat = useCatalog();
  const tree = useCatalogTreeState();
  useCatalogTreeSync(env.cardOpen, cat.sources, cat.jobs);
  const [focus, setFocus] = useState<number | null>(null);
  const [adding, setAdding] = useState<string | null>(null);
  const [removing, setRemoving] = useState<SourceStatus | null>(null);
  const fail = (e: unknown) => env.onError(String(e instanceof Error ? e.message : e));
  const choose = async () => {
    const dir = await browseFolder({ title: t("catalog.add.title"), added: cat.sources.map(sourceRoot) });
    if (dir) setAdding(dir);
  };
  const actions: CatalogActions = {
    open: env.openFolder,
    add: () => void choose().catch(fail),
    index: (id) => void indexSource(id).catch(fail),
    pause: (id) => void pauseSource(id).catch(fail),
    resume: (id) => void resumeSource(id).catch(fail),
    stop: (id) => void stopSource(id).catch(fail),
    remove: setRemoving,
    reveal: (p) => void revealItemInDir(p).catch(() => {}),
    dismiss: dismissEnded,
  };
  // "Show in Catalog": opens what is above the folder in its source's tree and brings its row into view; no folder is opened
  const showInCatalog = (s: SourceStatus, path: string) => {
    const rel = relUnder(sourceRoot(s), path) ?? "";
    env.showCard();
    setFocus(s.source.id);
    openAll(revealKeys(s.source.id, rel), treeKey(s.source.id, rel));
  };
  const tools: FolderTools = {
    sources: cat.sources,
    onCull: env.cullFolder,
    onStats: env.statsFolder,
    onAddToCatalog: setAdding,
    onShowInCatalog: showInCatalog,
    onRemoveFromCatalog: setRemoving,
    inFolders: env.inFolders,
    onShowInFolders: env.showInFolders,
  };
  // the same tools from a Catalog row: the folder they open counts as opened from here
  const catalogTools = useMemo<FolderTools>(() => ({
    ...tools,
    onCull: (p) => { env.fromCatalog(p); env.cullFolder(p); },
    onStats: (p) => { env.fromCatalog(p); env.statsFolder(p); },
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }), [cat.sources, env.cullFolder, env.statsFolder, env.fromCatalog, env.inFolders, env.showInFolders]);
  // the row selected in the tree: the open folder when it came from here, else the offline one picked
  const openKey = (() => {
    if (env.origin !== "catalog" || !env.current) return null;
    const hit = folderCatalog(env.current, cat.sources);
    if (hit.kind === "none") return null;
    return treeKey(hit.source.source.id, relUnder(sourceRoot(hit.source), env.current) ?? "");
  })();
  // "Check again" on an offline folder: look at the mounts once more, and if its source answers now, open the folder live
  const check = async () => {
    await refresh();
    const p = parseKey(getCatalogTree().offlineSelected ?? "");
    const s = p && getCatalog().sources.find((x) => x.source.id === p.sourceId);
    if (p && s && s.availability.state === "Online") { selectOffline(null); env.openFolder(folderPath(sourceRoot(s), p.rel)); }
  };
  const card = (open: boolean, onToggle: (alt: boolean) => void) => (
    <CatalogCard open={open} onToggle={onToggle} loaded={cat.loaded} error={cat.error} sources={cat.sources} jobs={cat.jobs}
                 ended={cat.ended} focus={focus} actions={actions} onFocus={setFocus}
                 tree={{
                   trees: tree.trees, expanded: tree.expanded, includeSub: env.includeSub,
                   selectedKey: tree.offlineSelected ?? openKey,
                   alsoPath: env.origin === "folders" ? env.current : null,
                   scrollTo: tree.scrollTo, tools: catalogTools,
                   onToggle: setOpen, onSelect: selectOffline,
                   onCheck: () => void check(),
                 }} />
  );
  const dialogs = <>
    {adding && <AddToCatalogDialog path={adding} sources={cat.sources} onClose={() => setAdding(null)}
                                   onAdd={async (p) => { const id = await addToCatalog(p); setFocus(id); openAll([treeKey(id, "")]); }} />}
    {removing && <RemoveSourceDialog source={removing} onClose={() => setRemoving(null)}
                                     onRemove={async (id) => { await removeFromCatalog(id); setFocus((f) => (f === id ? null : f)); }} />}
  </>;
  return { card, tools, dialogs };
}
