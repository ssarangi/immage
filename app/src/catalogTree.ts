// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Catalog's folder tree with no React: what the backend's catalog_tree_children answers kept per
// source, merged as scan ticks arrive, and flattened into the rows FolderTreeView draws (spec:
// notes/catalog-folder-tree-spec.md). Tested under Node (verify/catalog-tree.test.mjs).
import type { SourceStatus, TreeNode, TreeReply } from "./bindings";
import { groupByVolume, sourceRoot, sourceState, type Jobs, type SourceState } from "./catalogModel";
import type { TreeRow } from "./folderTreeModel";

/** The key of a source's folder: stable across a remount, which changes the mount path and not the source or rel. */
export const treeKey = (sourceId: number, rel: string) => `src:${sourceId}:${rel}`;

export function parseKey(key: string): { sourceId: number; rel: string } | null {
  const m = /^src:(\d+):([\s\S]*)$/.exec(key);
  return m ? { sourceId: Number(m[1]), rel: m[2] } : null;
}

/** What is known of one source's folders. */
export interface SourceTree {
  generation: number | null;
  root: TreeNode | null;
  /** children by parent rel ("" is the source root), each in the backend's order */
  kids: Map<string, TreeNode[]>;
}

export const emptyTree = (): SourceTree => ({ generation: null, root: null, kids: new Map() });

/** The parents to ask about: the root (for its chevron and count) and every open folder of this source. */
export function parentsToAsk(sourceId: number, expanded: ReadonlySet<string>): string[] {
  const out = [""];
  for (const k of expanded) {
    const p = parseKey(k);
    if (p && p.sourceId === sourceId && p.rel !== "") out.push(p.rel);
  }
  return out;
}

/** Folds a reply in: an unchanged reply keeps everything; otherwise each asked parent's children are replaced. */
export function applyReply(prev: SourceTree | undefined, asked: readonly string[], reply: TreeReply): SourceTree {
  const base = prev ?? emptyTree();
  if (reply.unchanged) return base;
  const kids = new Map(base.kids);
  for (const p of asked) kids.set(p, []);
  const by = new Map<string, TreeNode[]>();
  for (const n of reply.nodes) {
    const list = by.get(n.parent) ?? [];
    list.push(n);
    by.set(n.parent, list);
  }
  for (const [p, list] of by) kids.set(p, list);
  return { generation: reply.generation, root: reply.root, kids };
}

/** A folder's absolute path from its source's root, as the grid opens it. */
export function folderPath(root: string, rel: string): string {
  if (!rel) return root;
  const sep = root.includes("\\") && !root.includes("/") ? "\\" : "/";
  return `${root.endsWith("/") || root.endsWith("\\") ? root : root + sep}${sep === "\\" ? rel.replaceAll("/", "\\") : rel}`;
}

/** The rel of `path` under a source's root, or null when it is not inside it. */
export function relUnder(root: string, path: string): string | null {
  const norm = (p: string) => (p.length > 1 ? p.replace(/[\\/]+$/, "") : p).replaceAll("\\", "/");
  const r = norm(root), p = norm(path);
  if (p === r) return "";
  return p.startsWith(r.endsWith("/") ? r : r + "/") ? p.slice(r.endsWith("/") ? r.length : r.length + 1) : null;
}

/** The expanded keys that show `rel`: the source and each ancestor folder. */
export function revealKeys(sourceId: number, rel: string): string[] {
  const out = [treeKey(sourceId, "")];
  const parts = rel ? rel.split("/") : [];
  for (let i = 1; i < parts.length; i++) out.push(treeKey(sourceId, parts.slice(0, i).join("/")));
  // the folder itself is shown, not opened
  return out;
}

/** What the grid will show for a folder: the subtree, or only what is directly in it. */
export const countOf = (n: Pick<TreeNode, "direct" | "subtree">, includeSub: boolean) => (includeSub ? n.subtree : n.direct);

export interface CatalogRowData {
  sourceId: number;
  rel: string;
  /** a source row carries its status; a folder row only its own exception */
  source: SourceStatus | null;
  state: SourceState;
  offline: boolean;
  count: number | null;
  /** "attention" while its own directory did not answer, or "" */
  folderStatus: string;
  heading?: { name: string; host: string | null; online: boolean };
  message?: "finding";
}

export type CatalogRow = TreeRow<CatalogRowData>;

export interface FlattenInput {
  sources: readonly SourceStatus[];
  jobs: Record<number, Jobs>;
  trees: ReadonlyMap<number, SourceTree>;
  expanded: ReadonlySet<string>;
  includeSub: boolean;
}

const attention = (status: string) => status === "denied" || status === "stalled" || status === "error";

/** Volume headings, each source as the root of its tree, and the rows of every open folder under it. */
export function flatten(input: FlattenInput): CatalogRow[] {
  const out: CatalogRow[] = [];
  const groups = groupByVolume(input.sources);
  const addSource = (s: SourceStatus) => {
    const id = s.source.id;
    const state = sourceState(s, input.jobs[id]);
    const offline = state === "offline";
    const tree = input.trees.get(id);
    const key = treeKey(id, "");
    const scanning = state === "indexing" || state === "paused";
    const hasChildren = state !== "never" && (tree?.root?.has_children ?? false);
    // an expanded source that is still being indexed stays open before its first folder is found, so
    // the "finding folders" line has its place and the folder takes it
    const open = state !== "never" && input.expanded.has(key) && (hasChildren || scanning);
    const root = sourceRoot(s);
    out.push({
      key, parentKey: null, depth: 0, kind: "source", path: root, name: s.source.name, hasChildren, open,
      data: { sourceId: id, rel: "", source: s, state, offline, count: state === "never" ? null : scanning && tree?.root ? tree.root.subtree : s.source.photos, folderStatus: "" },
    });
    if (!open) return;
    const walk = (parent: string, parentKey: string, depth: number) => {
      for (const n of input.trees.get(id)?.kids.get(parent) ?? []) {
        const k = treeKey(id, n.rel);
        const isOpen = n.has_children && input.expanded.has(k);
        out.push({
          key: k, parentKey, depth, kind: "folder", path: folderPath(root, n.rel), name: n.name, hasChildren: n.has_children, open: isOpen,
          data: { sourceId: id, rel: n.rel, source: null, state, offline, count: countOf(n, input.includeSub), folderStatus: attention(n.status) ? n.status : "" },
        });
        if (isOpen) walk(n.rel, k, depth + 1);
      }
    };
    walk("", key, 1);
    if (scanning && !(tree?.kids.get("")?.length)) {
      // one quiet line at row height until the first folder arrives, replaced in place
      out.push({
        key: `${key}:message`, parentKey: key, depth: 1, kind: "message", path: "", name: "", hasChildren: false, open: false,
        data: { sourceId: id, rel: "", source: null, state, offline, count: null, folderStatus: "", message: "finding" },
      });
    }
  };
  if (groups.length < 2) {
    for (const s of input.sources) addSource(s);
  } else {
    for (const g of groups) {
      out.push({
        key: `volume:${g.key}`, parentKey: null, depth: 0, kind: "heading", path: "", name: g.name, hasChildren: false, open: false,
        data: { sourceId: 0, rel: "", source: null, state: "online", offline: !g.online, count: null, folderStatus: "", heading: { name: g.name, host: g.host, online: g.online } },
      });
      for (const s of g.items) addSource(s);
    }
  }
  return out;
}

/**
 * Calls `fn` at most once in `ms`: the first call at once, the last one after the wait if more came
 * in meanwhile. The clock and timers are passed in so a test runs without waiting.
 */
export function throttle(fn: () => void, ms: number, clock: { now(): number; later(f: () => void, ms: number): unknown; cancel(h: unknown): void }) {
  let last = -Infinity;
  let pending: unknown = null;
  const run = () => { pending = null; last = clock.now(); fn(); };
  const call = () => {
    const wait = last + ms - clock.now();
    if (wait <= 0) { if (pending !== null) { clock.cancel(pending); pending = null; } run(); return; }
    if (pending === null) pending = clock.later(run, wait);
  };
  call.cancel = () => { if (pending !== null) clock.cancel(pending); pending = null; };
  return call;
}
