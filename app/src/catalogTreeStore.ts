// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The wiring of the Catalog's folder tree: which folders are open (remembered between launches beside
// the Folders list's own), the folder selected while its source is offline, and each source's folders as
// the backend last answered them (catalog_tree_children, from catalog.sqlite alone). While a source is
// scanned the page re-asks about the open folders at most every 2 seconds, using the scan-progress events
// as a tick only. catalogTree.ts is the logic; this file is the state and the calls.
import { useEffect, useRef, useSyncExternalStore } from "react";
import { commands } from "./commands";
import { loadLibrary, saveLibrary } from "./layoutPrefs";
import { applyReply, parentsToAsk, throttle, type SourceTree } from "./catalogTree";
import { toggled } from "./folderTreeModel";
import type { Jobs } from "./catalogModel";
import type { SourceStatus } from "./bindings";

export const TICK_MS = 2000;

interface TreeState {
  trees: ReadonlyMap<number, SourceTree>;
  expanded: ReadonlySet<string>;
  /** the folder selected while its source is not connected (the grid cannot show it) */
  offlineSelected: string | null;
  /** brought into view when `n` changes */
  scrollTo: { key: string; n: number } | null;
  error: string | null;
}

let state: TreeState = {
  trees: new Map(), expanded: new Set(loadLibrary().catalogExpanded ?? []), offlineSelected: null, scrollTo: null, error: null,
};
const listeners = new Set<() => void>();
const set = (next: Partial<TreeState>) => { state = { ...state, ...next }; listeners.forEach((l) => l()); };
export const getCatalogTree = () => state;
export function useCatalogTreeState(): TreeState {
  return useSyncExternalStore((l) => { listeners.add(l); return () => { listeners.delete(l); }; }, getCatalogTree);
}

export function setOpen(key: string, open: boolean) {
  const expanded = toggled(state.expanded, key, open);
  saveLibrary({ catalogExpanded: [...expanded] });
  set({ expanded });
}

/** Opens these folders (and keeps the ones already open): "Show in Catalog" and a new source. */
export function openAll(keys: readonly string[], scrollTo?: string) {
  const expanded = new Set([...state.expanded, ...keys]);
  saveLibrary({ catalogExpanded: [...expanded] });
  set({ expanded, scrollTo: scrollTo ? { key: scrollTo, n: (state.scrollTo?.n ?? 0) + 1 } : state.scrollTo });
}

export const selectOffline = (key: string | null) => set({ offlineSelected: key });

const flying = new Map<number, boolean>();
const again = new Set<number>();

/** Asks the backend about a source's root and open folders. An unchanged reply costs one lookup. */
export async function askTree(id: number, force = false): Promise<void> {
  if (flying.get(id)) { again.add(id); return; }
  flying.set(id, true);
  try {
    const asked = parentsToAsk(id, state.expanded);
    const known = state.trees.get(id);
    // a folder opened since the last answer has no children held, so the unchanged shortcut would hide them
    const fresh = force || !known || asked.some((p) => !known.kids.has(p));
    const reply = await commands.catalogTreeChildren(id, asked, fresh ? null : known.generation);
    const trees = new Map(state.trees);
    trees.set(id, applyReply(state.trees.get(id), asked, reply));
    set({ trees, error: null });
  } catch (e) {
    set({ error: String(e instanceof Error ? e.message : e) });
  } finally {
    flying.set(id, false);
    if (again.delete(id)) void askTree(id);
  }
}

/** Forgets what is held of sources that are gone. */
export function dropTrees(ids: ReadonlySet<number>) {
  if ([...state.trees.keys()].every((id) => ids.has(id))) return;
  set({ trees: new Map([...state.trees].filter(([id]) => ids.has(id))) });
}

/**
 * Keeps the trees current while the card is open: asks about every source that has been indexed when
 * the list, a source's state or an open folder changes, and, while a source is scanned, at most every
 * 2 seconds as the scan reports (a final ask when it ends). With nothing scanning there is no timer.
 */
export function useCatalogTreeSync(open: boolean, sources: readonly SourceStatus[], jobs: Record<number, Jobs>) {
  const { expanded } = useCatalogTreeState();
  const throttles = useRef(new Map<number, ReturnType<typeof throttle>>());
  const scanning = useRef(new Set<number>());
  const real = useRef({ now: () => Date.now(), later: (f: () => void, ms: number) => window.setTimeout(f, ms), cancel: (h: unknown) => window.clearTimeout(h as number) });
  const shape = sources.map((s) => `${s.source.id}:${s.source.state}:${s.source.photos}:${s.availability.state}`).join("|");
  const opened = [...expanded].sort().join("|");
  useEffect(() => {
    if (!open) return;
    dropTrees(new Set(sources.map((s) => s.source.id)));
    for (const s of sources) void askTree(s.source.id);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, shape, opened]);
  useEffect(() => {
    if (!open) return;
    const now = new Set<number>();
    for (const s of sources) {
      const id = s.source.id;
      if (jobs[id]?.scan) {
        now.add(id);
        let tick = throttles.current.get(id);
        if (!tick) { tick = throttle(() => void askTree(id), TICK_MS, real.current); throttles.current.set(id, tick); }
        tick();
      } else if (scanning.current.has(id)) {
        throttles.current.get(id)?.cancel();
        void askTree(id);
      }
    }
    scanning.current = now;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, jobs]);
  useEffect(() => () => { for (const t of throttles.current.values()) t.cancel(); }, []);
}
