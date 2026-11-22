// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Library panel's Catalog card: the sources the Catalog keeps (folders Immage has read once, so
// they stay searchable and browsable by their cached data), beside the Folders card. Each source is
// the root of its own folder tree, drawn by the same FolderTreeView as the Folders list: rows with
// photo counts from the catalog (never from the disk), so the tree opens with the drive away. A source
// row has its state glyph; a click opens the folder, a right-click or the "..." button its menu; the
// focused source shows its IndexCard in a footer under the tree.
// Presentational: App gives it the store's state and the actions.
import { useMemo, useState } from "react";
import { FILE_MANAGER } from "./ExportDialog";
import { useMenuClose } from "./FileMenu";
import { FolderTreeView } from "./FolderTreeView";
import { FolderMenu, type FolderTools } from "./FolderMenu";
import { flatten, treeKey, type CatalogRow, type CatalogRowData, type SourceTree } from "./catalogTree";
import type { Placed } from "./folderTreeModel";
import { FolderIcon, DriveIcon, NetworkIcon } from "./icons";
import { Glyph, IndexCard, Ring } from "./IndexCard";
import { SectionStatus } from "./SectionStatus";
import { catalogSummary, overallFraction, phases, sourceRoot, sourceState, type Ended, type Jobs, type SourceState } from "./catalogModel";
import type { SourceStatus } from "./bindings";
import { formatDate, formatNumber, t } from "./i18n";

export type CatalogActions = {
  open(path: string): void;
  add(): void;
  index(id: number): void;
  pause(id: number): void;
  resume(id: number): void;
  stop(id: number): void;
  remove(source: SourceStatus): void;
  reveal(path: string): void;
  dismiss(id: number): void;
};

/** Enter or Space on a card's header folds it, as a click does; keys meant for a button inside it are left alone. */
export function headerKey(e: React.KeyboardEvent<HTMLElement>, onToggle: (alt: boolean) => void) {
  if (e.target !== e.currentTarget || (e.key !== "Enter" && e.key !== " ")) return;
  e.preventDefault();
  onToggle(e.altKey);
}

const stateWord = (s: SourceState) => t(`catalog.state.${s}`);

export function stateTitle(s: SourceStatus, state: SourceState, jobs: Jobs | undefined): string {
  const src = s.source;
  if (state === "indexing" || state === "paused") {
    const fraction = overallFraction(phases(jobs, undefined, src.photos));
    return fraction === null ? t("catalog.tip.finding") : t(state === "paused" ? "catalog.tip.paused" : "catalog.tip.indexing", { pct: Math.round(fraction * 100) });
  }
  if (state === "attention") return t("catalog.tip.attention", { why: src.error ?? t("catalog.problem.unknown") });
  return stateWord(state);
}

const kindIcon = (src: SourceStatus["source"]) => (src.kind === "nas" ? <NetworkIcon /> : src.kind === "drive" ? <DriveIcon /> : <FolderIcon />);

// the count in its 8ch slot: whole up to 99,999, compact above it (the tooltip has the whole number)
const countShown = (n: number) => formatNumber(n, n > 99_999 ? { notation: "compact", maximumFractionDigits: 1 } : undefined);

/** What the tree around the sources shows and does (the Catalog's folder tree; catalogTreeStore.ts feeds it). */
export type CatalogTreeProps = {
  trees: ReadonlyMap<number, SourceTree>;
  expanded: ReadonlySet<string>;
  /** the Folders tree's "Include subfolders": a folder's count is its subtree, or only what is directly in it */
  includeSub: boolean;
  /** the open folder when it was opened from here, or the folder selected while its source is offline */
  selectedKey: string | null;
  /** the folder open from the Folders tree, marked quietly here */
  alsoPath: string | null;
  scrollTo: { key: string; n: number } | null;
  tools: FolderTools;
  onToggle(key: string, open: boolean): void;
  /** an offline folder is selected (nothing opens: the disk is away) */
  onSelect(key: string | null): void;
  /** check whether the offline source answers now */
  onCheck(): void;
};

function SourceMenu({ x, y, s, jobs, actions, tree, onClose }: { x: number; y: number; s: SourceStatus; jobs: Jobs | undefined; actions: CatalogActions; tree: CatalogTreeProps; onClose(): void }) {
  const ref = useMenuClose(onClose);
  const state = sourceState(s, jobs);
  const root = sourceRoot(s), id = s.source.id;
  const left = Math.min(x, window.innerWidth - 240), top = Math.min(y, window.innerHeight - 330);
  const running = state === "indexing", paused = state === "paused";
  const away = state === "offline";
  const go = (f: () => void) => () => { onClose(); f(); };
  const why = away ? t("catalog.notConnected") : undefined;
  return (
    <div ref={ref} className="context-menu cat-menu" style={{ left, top }} role="menu">
      <button role="menuitem" disabled={state === "never"} onClick={go(() => (away ? tree.onSelect(treeKey(id, "")) : actions.open(root)))}>{t("catalog.menu.open")}</button>
      <button role="menuitem" disabled={away || state === "never"} title={why} onClick={go(() => tree.tools.onCull(root))}>{t("catalog.folder.cull")}</button>
      <button role="menuitem" disabled={away || state === "never"} title={why} onClick={go(() => tree.tools.onStats(root))}>{t("catalog.folder.stats")}</button>
      {running && <button role="menuitem" onClick={go(() => actions.pause(id))}>{t("catalog.act.pause")}</button>}
      {paused && <button role="menuitem" onClick={go(() => actions.resume(id))}>{t("catalog.act.resume")}</button>}
      {!running && !paused && <button role="menuitem" disabled={away} title={why} onClick={go(() => actions.index(id))}>{t(s.source.photos > 0 ? "catalog.act.rescan" : "catalog.act.index")}</button>}
      {(running || paused) && <button role="menuitem" onClick={go(() => actions.stop(id))}>{t("catalog.act.stop")}</button>}
      <div className="menu-sep" />
      {tree.tools.inFolders(root) && <button role="menuitem" onClick={go(() => tree.tools.onShowInFolders(root))}>{t("catalog.tree.showInFolders")}</button>}
      <button role="menuitem" disabled={away} title={why} onClick={go(() => actions.reveal(root))}>{t("library.menu.showIn", { manager: FILE_MANAGER })}</button>
      <button role="menuitem" onClick={go(() => void navigator.clipboard?.writeText(root).catch(() => {}))}>{t("catalog.tree.copyPath")}</button>
      <div className="menu-sep" />
      <button role="menuitem" onClick={go(() => actions.remove(s))}>{t("catalog.menu.remove")}</button>
    </div>
  );
}

const samePath = (a: string | null, b: string) => a !== null && a.replace(/[\\/]+$/, "") === b.replace(/[\\/]+$/, "");

/** The folded card's header status: the work across all sources, and a warning if any needs attention. */
export function catalogHeaderStatus(sources: SourceStatus[], jobs: Record<number, Jobs>) {
  const m = catalogSummary(sources, jobs);
  const n = (x: number) => formatNumber(x);
  const text = m.state === "paused" ? t("catalog.sum.paused")
    : m.total === null ? t("catalog.sum.finding")
    : t(m.thumbs ? "catalog.sum.thumbs" : "catalog.sum.indexing", { done: n(m.done), total: n(m.total) });
  const mid = m.state === "paused" ? text : m.total === null ? null : t("catalog.sum.count", { done: n(m.done), total: n(m.total) });
  const short = m.fraction === null ? null : t("catalog.sum.pct", { pct: Math.round(m.fraction * 100) });
  const warn = m.attention > 0 ? t("catalog.sum.attention") : null;
  const work = m.state === "idle" ? null : text;
  const label = t("catalog.sum.fold", { what: [work, warn].filter(Boolean).join(". ") });
  return { kind: m.state, fraction: m.fraction, text, mid, short, label, warn };
}

export function CatalogCard({ open, onToggle, loaded, error, sources, jobs, ended, focus, actions, onFocus, pin, tree }: {
  open: boolean; onToggle(alt: boolean): void;
  loaded: boolean; error: string | null;
  sources: SourceStatus[]; jobs: Record<number, Jobs>; ended: Record<number, Ended>;
  focus: number | null;
  actions: CatalogActions; onFocus(id: number | null): void;
  pin?: React.ReactNode;
  tree: CatalogTreeProps;
}) {
  const [menu, setMenu] = useState<{ x: number; y: number; row: CatalogRow } | null>(null);
  const focused = sources.find((s) => s.source.id === focus) ?? null;
  const rows = useMemo(
    () => flatten({ sources, jobs, trees: tree.trees, expanded: tree.expanded, includeSub: tree.includeSub }),
    [sources, jobs, tree.trees, tree.expanded, tree.includeSub]);
  const sourceOf = (id: number) => sources.find((s) => s.source.id === id);
  // a click opens the folder in the grid (an offline one is only selected: the disk is away)
  const openRow = (r: CatalogRow) => {
    const d = r.data;
    onFocus(d.sourceId);
    if (d.state === "never") return;
    if (d.offline) { tree.onSelect(r.key); return; }
    tree.onSelect(null);
    actions.open(r.path);
  };
  const offlineRow = tree.selectedKey && rows.find((r) => r.key === tree.selectedKey && r.data.offline);
  const offlineSource = offlineRow ? sourceOf(offlineRow.data.sourceId) : undefined;
  const menuSource = menu ? sourceOf(menu.row.data.sourceId) : undefined;
  const rowAttrs = (r: Placed<CatalogRowData>) => {
    const d = r.data;
    const tip = d.source ? stateTitle(d.source, d.state, jobs[d.sourceId]) : "";
    return {
      className: `${d.offline ? "offline" : ""}${focus === d.sourceId && r.kind === "source" ? " focused" : ""}${samePath(tree.alsoPath, r.path) ? " also" : ""}`,
      "data-source": String(d.sourceId), "data-folder-refuse": "1",
      title: `${r.path}\n${d.offline ? t("catalog.tree.offlineTip") : r.kind === "source" ? tip : ""}`.trim(),
      onContextMenu: (e: React.MouseEvent) => { e.preventDefault(); e.stopPropagation(); onFocus(d.sourceId); setMenu({ x: e.clientX, y: e.clientY, row: r }); },
    };
  };
  const renderItem = (r: Placed<CatalogRowData>) => {
    const d = r.data, src = d.source?.source;
    const fraction = d.source && (d.state === "indexing" || d.state === "paused") ? overallFraction(phases(jobs[d.sourceId], undefined, src!.photos)) : null;
    const status = d.source ? stateTitle(d.source, d.state, jobs[d.sourceId]) : d.folderStatus ? t(`catalog.tree.dir.${d.folderStatus}`) : "";
    const full = d.count === null ? undefined : t("catalog.tree.photos", { n: d.count, count: formatNumber(d.count) });
    const also = samePath(tree.alsoPath, r.path);
    return <>
      <span className="ftree-icon">{src ? kindIcon(src) : <FolderIcon />}</span>
      <span className="name">{r.name}</span>
      {also && <span className="photo-dot" role="img" aria-label={t("catalog.tree.alsoFolders")} title={t("catalog.tree.alsoFolders")} />}
      <span className="cat-slot">
        <span className={`cat-count${d.count === 0 ? " zero" : ""}`} aria-label={full ?? t("catalog.state.never")} title={d.count === 0 ? t("catalog.tree.empty") : full}>
          {d.count === null ? "-" : countShown(d.count)}
        </span>
        <button className="cat-more" tabIndex={-1} aria-label={t("catalog.row.more", { name: r.name })} title={t("catalog.row.more", { name: r.name })}
                onClick={(e) => { e.stopPropagation(); const b = e.currentTarget.getBoundingClientRect(); onFocus(d.sourceId); setMenu({ x: b.left, y: b.bottom + 2, row: r }); }}>{"…"}</button>
      </span>
      <span className="cat-state" role={status ? "img" : undefined} aria-label={status || undefined} title={status || undefined}>
        {d.source
          ? (d.state === "indexing" ? <Ring fraction={fraction} /> : <Glyph state={d.state} fraction={fraction} />)
          : d.folderStatus ? <Glyph state="attention" /> : null}
      </span>
    </>;
  };
  const renderOther = (r: Placed<CatalogRowData>) => r.data.heading ? (
    <div className="cat-volume" title={r.data.heading.host ?? undefined}>
      <span className="name">{r.data.heading.name}</span>
      {r.data.heading.host && r.data.heading.host !== r.data.heading.name && <span className="host">{r.data.heading.host}</span>}
      <span className="cat-volume-state">{t(r.data.heading.online ? "catalog.state.online" : "catalog.state.offline")}</span>
    </div>
  ) : <span className="cat-finding">{t("catalog.tree.finding")}</span>;
  return (
    <div className="panel catalog-panel" data-tour="catalog">
      <div className={`panel-title${open ? " open" : ""}`} onClick={(e) => onToggle(e.altKey)}
           role="button" tabIndex={0} aria-expanded={open} onKeyDown={(e) => headerKey(e, onToggle)}>
        <span>{t("catalog.title")}</span>
        <SectionStatus {...(open ? { kind: "idle", fraction: null, text: "", mid: null, short: null, label: "", warn: null } : catalogHeaderStatus(sources, jobs))} />
        <button className="cat-add" title={t("catalog.add")} aria-label={t("catalog.add")} onClick={(e) => { e.stopPropagation(); actions.add(); }}>+</button>
        {pin}
        <svg className={`chevron${open ? " open" : ""}`} viewBox="0 0 24 24" fill="none" stroke="currentColor"
             strokeWidth="2.2" strokeLinecap="round" strokeLinejoin="round"><path d="m6 15 6-6 6 6" /></svg>
      </div>
      {open && (
        <div className="cat-body">
          {sources.length === 0 ? (
            <div className="cat-empty">
              <p>{loaded ? t("catalog.empty") : t("catalog.loading")}</p>
              <button className="folder add" onClick={actions.add}><span>{t("catalog.addFolder")}</span></button>
              <p className="cat-reassure">{t("catalog.readOnly")}</p>
            </div>
          ) : (
            <>
              <FolderTreeView<CatalogRowData> rows={rows} label={t("catalog.tree.label")} className="cat-list" selectedKey={tree.selectedKey}
                scrollTo={tree.scrollTo} rowAttrs={rowAttrs} renderItem={renderItem} renderOther={renderOther}
                onOpen={openRow} onToggle={(r, on) => tree.onToggle(r.key, on)}
                onMenu={(r, x, y) => { onFocus(r.data.sourceId); setMenu({ x, y, row: r as CatalogRow }); }} />
              <button className="folder add" onClick={actions.add}><span>{t("catalog.addFolder")}</span></button>
            </>
          )}
          {error && <p role="alert" className="cat-error">{t("catalog.error", { why: error })}</p>}
          {/* under the tree whatever its length: the focused source's work, or what an offline folder holds */}
          {(offlineSource || focused) && (
            <div className="cat-footer">
              {offlineSource && offlineRow ? <OfflineCard source={offlineSource} count={offlineRow.data.count} onCheck={tree.onCheck} />
                : focused && (
                  <IndexCard key={focused.source.id} source={focused} jobs={jobs[focused.source.id]} ended={ended[focused.source.id]}
                             actions={{
                               onPause: () => actions.pause(focused.source.id), onResume: () => actions.resume(focused.source.id),
                               onStop: () => actions.stop(focused.source.id), onRetry: () => actions.index(focused.source.id),
                               onOpen: () => actions.open(sourceRoot(focused)), onDismiss: () => { actions.dismiss(focused.source.id); onFocus(null); },
                             }} />
                )}
            </div>
          )}
        </div>
      )}
      {menu && menuSource && (menu.row.kind === "source"
        ? <SourceMenu x={menu.x} y={menu.y} s={menuSource} jobs={jobs[menu.row.data.sourceId]} actions={actions} tree={tree} onClose={() => setMenu(null)} />
        : <FolderMenu variant="catalog" x={menu.x} y={menu.y} path={menu.row.path} offline={menu.row.data.offline} tools={tree.tools} source={menuSource}
                      onOpen={() => openRow(menu.row)} onClose={() => setMenu(null)} />)}
    </div>
  );
}

// an offline folder cannot be opened in the grid yet (its photos are only in the Catalog): what is known
// of it, and a way to check whether the drive is back
function OfflineCard({ source, count, onCheck }: { source: SourceStatus; count: number | null; onCheck(): void }) {
  const at = source.source.last_scan_end;
  const where = source.source.host ?? source.source.name;
  return (
    <div className="cat-card cat-offline" role="status">
      <span className="cat-title">{t("catalog.tree.offlineTitle", { n: count ?? 0, count: formatNumber(count ?? 0) })}</span>
      <span className="cat-sub">{at ? t("catalog.tree.offlineChecked", { when: formatDate(at * 1000, { dateStyle: "medium" }) }) : t("catalog.tree.offlineNever")}</span>
      <span className="cat-sub">{t("catalog.tree.offlineBody", { where })}</span>
      <div className="cat-acts"><button className="btn" onClick={onCheck}>{t("catalog.tree.check")}</button></div>
    </div>
  );
}
