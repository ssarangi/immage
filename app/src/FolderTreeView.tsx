// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The one folder tree the Library panel draws, for the Folders list and for the Catalog: a flat list of
// rows (folderTreeModel.ts) with the tree's roles, a roving tab stop and the keyboard map, rows of one
// fixed height, and a window of them when the tree is long. It draws no row's content: the Folders
// adapter (FolderTree.tsx) and the Catalog adapter (CatalogCard.tsx) pass a renderer for what sits
// after the twisty, and the attributes a row of theirs carries (drag targets, tooltips, menus).
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import {
  closedSiblings, isItem, keyMove, place, ROW_HEIGHT, scrollToShow, typeAhead, typedSoFar, windowRows,
  type Placed, type TreeRow,
} from "./folderTreeModel";

// points right, and turns down when what it opens opens (and points left in a right-to-left layout)
export const chevron = (
  <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.6"
       strokeLinecap="round" strokeLinejoin="round"><path d="m9 5 7 7-7 7" /></svg>
);

type RowAttrs = React.HTMLAttributes<HTMLDivElement> & { [data: `data-${string}`]: string | undefined };

export interface FolderTreeViewProps<D> {
  rows: readonly TreeRow<D>[];
  /** the tree's accessible name */
  label: string;
  className?: string;
  /** the open folder's row: marked selected, and where the tab stop starts */
  selectedKey?: string | null;
  /** brings a row into view when `n` changes */
  scrollTo?: { key: string; n: number } | null;
  /** the room a long tree gets before it scrolls inside itself, in px */
  maxHeight?: number;
  /** what sits after the twisty of a folder or source row */
  renderItem(row: Placed<D>): React.ReactNode;
  /** what a heading or a message row shows (they are drawn, never focused) */
  renderOther(row: Placed<D>): React.ReactNode;
  /** a row's own class and attributes (data-folder-path, tooltips, pointer and menu handlers) */
  rowAttrs?(row: Placed<D>): RowAttrs;
  onOpen(row: TreeRow<D>): void;
  onToggle(row: TreeRow<D>, open: boolean): void;
  /** the menu key or Shift+F10 on a row; `x`, `y` are where it should open */
  onMenu?(row: TreeRow<D>, x: number, y: number): void;
}

export function FolderTreeView<D>({
  rows, label, className, selectedKey = null, scrollTo = null, maxHeight = 560, renderItem, renderOther, rowAttrs, onOpen, onToggle, onMenu,
}: FolderTreeViewProps<D>) {
  const placed = useMemo(() => place(rows), [rows]);
  const index = useMemo(() => new Map(placed.map((r, i) => [r.key, i])), [placed]);
  const box = useRef<HTMLDivElement>(null);
  const [focusKey, setFocusKey] = useState<string | null>(null);
  const wantFocus = useRef(false);
  const typed = useRef({ text: "", at: 0 });
  const [scrollTop, setScrollTop] = useState(0);
  const long = placed.length > 400;

  // the one row on the tab order: the one focused last if it is still there, else the open one, else the first
  const tab = (focusKey !== null && index.has(focusKey) && isItem(placed[index.get(focusKey)!]) ? focusKey : null)
    ?? (selectedKey !== null && index.has(selectedKey) ? selectedKey : null)
    ?? placed.find(isItem)?.key ?? null;

  const win = windowRows(placed.length, scrollTop, maxHeight, tab !== null && index.has(tab) ? [index.get(tab)!] : []);

  const focusRow = useCallback((key: string) => {
    setFocusKey(key);
    wantFocus.current = true;
    const at = index.get(key);
    const el = box.current;
    if (long && el && at !== undefined) {
      const to = scrollToShow(at, el.scrollTop, el.clientHeight);
      if (to !== null) el.scrollTop = to;
    }
  }, [index, long]);

  useLayoutEffect(() => {
    if (!wantFocus.current || !box.current) return;
    const el = Array.from(box.current.querySelectorAll<HTMLElement>("[data-key]")).find((e) => e.dataset.key === focusKey);
    if (el) { wantFocus.current = false; el.focus({ preventScroll: false }); }
  });

  // a row asked for before its folder's children arrive is scrolled to when it appears
  const wanted = scrollTo ? index.has(scrollTo.key) : false;
  useEffect(() => {
    if (!scrollTo) return;
    const at = index.get(scrollTo.key);
    const el = box.current;
    if (at === undefined || !el) return;
    if (long) {
      const to = scrollToShow(at, el.scrollTop, el.clientHeight);
      if (to !== null) el.scrollTop = to;
    }
    // drawn rows, or the ones the window just brought in on the next frame
    const frame = requestAnimationFrame(() => {
      Array.from(el.querySelectorAll<HTMLElement>("[data-key]")).find((e) => e.dataset.key === scrollTo.key)?.scrollIntoView({ block: "nearest" });
    });
    return () => cancelAnimationFrame(frame);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scrollTo?.n, scrollTo?.key, long, wanted]);

  const onKeyDown = (e: React.KeyboardEvent<HTMLDivElement>) => {
    const rowEl = (e.target as HTMLElement).closest<HTMLElement>("[data-key]");
    // keys meant for a field or a button inside a row stay theirs
    if (!rowEl || e.target !== rowEl || e.ctrlKey || e.metaKey || e.altKey) return;
    const key = rowEl.dataset.key!;
    const rtl = getComputedStyle(box.current!).direction === "rtl";
    const row = rows.find((r) => r.key === key);
    if (!row) return;
    if (e.key.length === 1 && e.key !== " " && e.key !== "*") {
      const now = typedSoFar(typed.current, e.key, Date.now());
      typed.current = now;
      const hit = typeAhead(rows, key, now.text);
      if (hit) { e.preventDefault(); focusRow(hit); }
      return;
    }
    const move = keyMove(rows, key, e.key, rtl, e.shiftKey);
    if (!move) return;
    e.preventDefault();
    switch (move.kind) {
      case "focus": focusRow(move.key); break;
      case "toggle": onToggle(row, move.open); break;
      case "open": onOpen(row); break;
      case "expandSiblings": for (const k of closedSiblings(rows, key)) { const r = rows.find((x) => x.key === k); if (r) onToggle(r, true); } break;
      case "menu": { const b = rowEl.getBoundingClientRect(); onMenu?.(row, rtl ? b.right - 24 : b.left + 24, b.bottom); break; }
    }
  };

  const draw = (r: Placed<D>) => {
    if (!isItem(r)) {
      return (
        <div key={r.key} role="presentation" className={`ftree-other ${r.kind === "heading" ? "folder-volume" : "folder-message"}`}
             style={{ "--depth": r.depth } as React.CSSProperties}>
          {renderOther(r)}
        </div>
      );
    }
    const extra = rowAttrs?.(r) ?? {};
    const { className: cls, style, onClick, onFocus, ...rest } = extra;
    return (
      <div key={r.key} role="treeitem" data-key={r.key} aria-level={r.level} aria-setsize={r.setSize} aria-posinset={r.posInSet}
           aria-expanded={r.hasChildren ? r.open : undefined} aria-selected={r.key === selectedKey}
           tabIndex={r.key === tab ? 0 : -1}
           className={`folder-row${r.key === selectedKey ? " active" : ""}${cls ? ` ${cls}` : ""}`}
           style={{ ...style, "--depth": r.depth } as React.CSSProperties}
           onFocus={(e) => { if (e.target === e.currentTarget) setFocusKey(r.key); onFocus?.(e); }}
           onClick={(e) => { onClick?.(e); if (!e.defaultPrevented) onOpen(r); }}
           {...rest}>
        <button className={`twisty${r.open ? " open" : ""}${r.hasChildren ? "" : " none"}`} tabIndex={-1} aria-hidden="true"
                onClick={(e) => { e.stopPropagation(); if (r.hasChildren) onToggle(r, !r.open); }}>
          {chevron}
        </button>
        {renderItem(r)}
      </div>
    );
  };

  const shown = placed.slice(win.from, win.to);
  const kept = win.keep.map((i) => placed[i]);
  return (
    <div ref={box} role="tree" aria-label={label} className={`ftree ${className ?? ""}${long ? " long" : ""}`}
         style={long ? { height: Math.min(placed.length * ROW_HEIGHT, maxHeight) } : undefined}
         onKeyDown={onKeyDown} onScroll={long ? (e) => setScrollTop(e.currentTarget.scrollTop) : undefined}>
      {long && <div style={{ height: win.before }} aria-hidden="true" />}
      {shown.map(draw)}
      {long && <div style={{ height: win.after }} aria-hidden="true" />}
      {kept.length > 0 && <div className="ftree-kept">{kept.map(draw)}</div>}
    </div>
  );
}
