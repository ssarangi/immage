// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Catalog's seam with the editor shell: one hook that gives App the Library card, the folder
// menu's tools and the two dialogs, so App.tsx carries a few lines and the Catalog stays here. The
// store (catalogStore.ts) holds the state; what a click does in the editor (open a folder in the
// grid, start a cull, open Statistics) is passed in.
import { useState } from "react";
import { open as chooseFolder } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { CatalogCard, type CatalogActions } from "./CatalogCard";
import { AddToCatalogDialog, RemoveSourceDialog } from "./CatalogDialogs";
import type { FolderTools } from "./FolderTree";
import { sourceRoot } from "./catalogModel";
import { addToCatalog, dismissEnded, indexSource, pauseSource, removeFromCatalog, resumeSource, stopSource, useCatalog } from "./catalogStore";
import type { SourceStatus } from "./bindings";

export function useCatalogUi(env: {
  /** show a folder's photos in the grid */
  openFolder(path: string): void;
  cullFolder(path: string): void;
  statsFolder(path: string): void;
  current: string | null;
  onError(message: string): void;
}) {
  const cat = useCatalog();
  const [focus, setFocus] = useState<number | null>(null);
  const [adding, setAdding] = useState<string | null>(null);
  const [removing, setRemoving] = useState<SourceStatus | null>(null);
  const fail = (e: unknown) => env.onError(String(e instanceof Error ? e.message : e));
  const choose = async () => {
    const dir = await chooseFolder({ directory: true });
    if (typeof dir === "string") setAdding(dir);
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
  const tools: FolderTools = {
    sources: cat.sources,
    onCull: env.cullFolder,
    onStats: env.statsFolder,
    onAddToCatalog: setAdding,
    onOpenInCatalog: (s) => { setFocus(s.source.id); env.openFolder(sourceRoot(s)); },
    onRemoveFromCatalog: setRemoving,
  };
  const card = (open: boolean, onToggle: (alt: boolean) => void) => (
    <CatalogCard open={open} onToggle={onToggle} loaded={cat.loaded} error={cat.error} sources={cat.sources} jobs={cat.jobs}
                 ended={cat.ended} focus={focus} current={env.current} actions={actions} onFocus={setFocus} />
  );
  const dialogs = <>
    {adding && <AddToCatalogDialog path={adding} sources={cat.sources} onClose={() => setAdding(null)}
                                   onAdd={async (p) => { setFocus(await addToCatalog(p)); }} />}
    {removing && <RemoveSourceDialog source={removing} onClose={() => setRemoving(null)}
                                     onRemove={async (id) => { await removeFromCatalog(id); setFocus((f) => (f === id ? null : f)); }} />}
  </>;
  return { card, tools, dialogs };
}
