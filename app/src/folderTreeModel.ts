// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The folder tree's logic with no React: rows as one flat list (what FolderTreeView draws, and what
// both the Folders and the Catalog adapters produce), the ARIA numbers a tree needs, the keyboard map,
// type-ahead, and the window of rows a long tree draws. Tested under Node (verify/folder-tree.test.mjs).

/** The height of every row, in px: what makes a window of rows an arithmetic, and a row never shift. */
export const ROW_HEIGHT = 28;
/** Below this many visible rows everything is drawn; above it, a window. */
export const WINDOW_AT = 400;
export const OVERSCAN = 10;

export type RowKind = "source" | "folder" | "heading" | "message";

export interface TreeRow<D = unknown> {
  /** Opaque, unique in one tree. */
  key: string;
  /** The key of the row this one is under, or null at the top. */
  parentKey: string | null;
  /** 0 for the top of a tree. */
  depth: number;
  kind: RowKind;
  /** The folder a click opens (empty for headings and messages). */
  path: string;
  name: string;
  hasChildren: boolean;
  open: boolean;
  /** What the adapter's renderer needs to draw the row. */
  data: D;
}

export interface Placed<D = unknown> extends TreeRow<D> {
  /** aria-level, 1 based */
  level: number;
  /** aria-setsize and aria-posinset among the siblings that are tree items */
  setSize: number;
  posInSet: number;
}

/** Whether a row can take focus and be opened: headings and messages are only drawn. */
export const isItem = (r: { kind: RowKind }) => r.kind === "source" || r.kind === "folder";

/** The rows with their ARIA numbers. Siblings are the items with the same parent, in list order. */
export function place<D>(rows: readonly TreeRow<D>[]): Placed<D>[] {
  const sizes = new Map<string | null, number>();
  for (const r of rows) if (isItem(r)) sizes.set(r.parentKey, (sizes.get(r.parentKey) ?? 0) + 1);
  const seen = new Map<string | null, number>();
  return rows.map((r) => {
    const pos = isItem(r) ? (seen.set(r.parentKey, (seen.get(r.parentKey) ?? 0) + 1), seen.get(r.parentKey)!) : 0;
    return { ...r, level: r.depth + 1, setSize: isItem(r) ? sizes.get(r.parentKey) ?? 1 : 0, posInSet: pos };
  });
}

export type Move =
  | { kind: "focus"; key: string }
  | { kind: "toggle"; key: string; open: boolean }
  | { kind: "open"; key: string }
  | { kind: "menu"; key: string }
  | { kind: "expandSiblings"; key: string }
  | null;

const items = (rows: readonly TreeRow[]) => rows.filter(isItem);

/**
 * What a key does with focus on row `at`. In a right-to-left tree Left and Right swap, so Right
 * (toward the end of a line) is always "back".
 */
export function keyMove(rows: readonly TreeRow[], at: string, key: string, rtl = false, shift = false): Move {
  const list = items(rows);
  const i = list.findIndex((r) => r.key === at);
  if (i < 0) return list.length && ["ArrowDown", "ArrowUp", "Home", "End"].includes(key) ? { kind: "focus", key: list[0].key } : null;
  const row = list[i];
  const forward = rtl ? "ArrowLeft" : "ArrowRight";
  const back = rtl ? "ArrowRight" : "ArrowLeft";
  switch (key) {
    case "ArrowDown": return i + 1 < list.length ? { kind: "focus", key: list[i + 1].key } : null;
    case "ArrowUp": return i > 0 ? { kind: "focus", key: list[i - 1].key } : null;
    case "Home": return { kind: "focus", key: list[0].key };
    case "End": return { kind: "focus", key: list[list.length - 1].key };
    case "Enter": case " ": return { kind: "open", key: row.key };
    case "*": return { kind: "expandSiblings", key: row.key };
    case "ContextMenu": return { kind: "menu", key: row.key };
    case "F10": return shift ? { kind: "menu", key: row.key } : null;
    case forward:
      if (row.hasChildren && !row.open) return { kind: "toggle", key: row.key, open: true };
      if (row.hasChildren && row.open && list[i + 1]?.parentKey === row.key) return { kind: "focus", key: list[i + 1].key };
      return null;
    case back:
      if (row.hasChildren && row.open) return { kind: "toggle", key: row.key, open: false };
      return row.parentKey !== null && list.some((r) => r.key === row.parentKey) ? { kind: "focus", key: row.parentKey } : null;
    default: return null;
  }
}

/** The keys of the closed siblings of a row (what `*` opens). */
export function closedSiblings(rows: readonly TreeRow[], key: string): string[] {
  const row = rows.find((r) => r.key === key);
  if (!row) return [];
  return items(rows).filter((r) => r.parentKey === row.parentKey && r.hasChildren && !r.open).map((r) => r.key);
}

/** Type-ahead: the next item after `from` whose name starts with `typed` (case blind), wrapping; one letter repeated cycles. */
export function typeAhead(rows: readonly TreeRow[], from: string | null, typed: string): string | null {
  const list = items(rows);
  if (!list.length || !typed) return null;
  const word = typed.toLowerCase();
  const cycling = word.length > 1 && [...word].every((c) => c === word[0]);
  const needle = cycling ? word[0] : word;
  const start = Math.max(0, list.findIndex((r) => r.key === from));
  // a growing word may stay where it is; a single repeated key moves on
  for (let step = cycling || word.length === 1 ? 1 : 0; step <= list.length; step++) {
    const r = list[(start + step) % list.length];
    if (r.name.toLowerCase().startsWith(needle)) return r.key;
  }
  return null;
}

/** Collects what is typed within `ms` of the previous key. Pure: the clock is passed in. */
export function typedSoFar(prev: { text: string; at: number }, char: string, now: number, ms = 800): { text: string; at: number } {
  return { text: now - prev.at > ms ? char : prev.text + char, at: now };
}

export interface Windowed {
  /** the first and one past the last index drawn */
  from: number;
  to: number;
  /** px above and below the drawn rows, so the scroll height is the whole list's */
  before: number;
  after: number;
}

/** The rows to draw for a viewport. `keep` are indexes drawn whatever the scroll (the focused row). */
export function windowRows(count: number, scrollTop: number, viewport: number, keep: readonly number[] = []): Windowed & { keep: number[] } {
  if (count <= WINDOW_AT) return { from: 0, to: count, before: 0, after: 0, keep: [] };
  const first = Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN);
  const last = Math.min(count, Math.ceil((scrollTop + viewport) / ROW_HEIGHT) + OVERSCAN);
  return {
    from: first, to: last, before: first * ROW_HEIGHT, after: (count - last) * ROW_HEIGHT,
    keep: keep.filter((i) => i >= 0 && i < count && (i < first || i >= last)),
  };
}

/** The scroll offset that brings row `index` into a viewport, or null when it is already inside. */
export function scrollToShow(index: number, scrollTop: number, viewport: number): number | null {
  const top = index * ROW_HEIGHT, bottom = top + ROW_HEIGHT;
  if (top < scrollTop) return top;
  if (bottom > scrollTop + viewport) return bottom - viewport;
  return null;
}

/** Counts in the 8ch slot: whole up to 99,999, compact above (the full number is the tooltip). */
export function countText(n: number, format: (n: number, compact: boolean) => string): string {
  return format(n, n > 99_999);
}

/** Drops from `expanded` every key whose row no longer exists is not done: a closed set is kept as it was, so a remount restores it. */
export function toggled(set: ReadonlySet<string>, key: string, open?: boolean): Set<string> {
  const next = new Set(set);
  const want = open ?? !next.has(key);
  if (want) next.add(key); else next.delete(key);
  return next;
}
