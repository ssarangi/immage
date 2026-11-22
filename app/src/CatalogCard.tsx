// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Library panel's Catalog card: the sources the Catalog keeps (folders Immage has read once, so
// they stay searchable and browsable by their cached data), beside the Folders card, which is
// unchanged. A row is a source with its photo count and one state glyph; a click opens it, a
// right-click or the "..." button its menu; the focused source shows its IndexCard under the list.
// Presentational: App gives it the store's state and the actions.
import { useState } from "react";
import { FILE_MANAGER } from "./ExportDialog";
import { useMenuClose } from "./FileMenu";
import { FolderIcon, DriveIcon, NetworkIcon } from "./icons";
import { Glyph, IndexCard, Ring } from "./IndexCard";
import { groupByVolume, isOnline, overallFraction, phases, sourceRoot, sourceState, type Ended, type Jobs, type SourceState } from "./catalogModel";
import type { SourceStatus } from "./bindings";
import { formatNumber, t } from "./i18n";

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

function Row({ s, jobs, focused, active, actions, onFocus, onMenu }: {
  s: SourceStatus; jobs: Jobs | undefined; focused: boolean; active: boolean; actions: CatalogActions;
  onFocus(): void; onMenu(x: number, y: number): void;
}) {
  const src = s.source, state = sourceState(s, jobs);
  const online = isOnline(s) || s.availability.state === "Asleep";
  const fraction = state === "indexing" || state === "paused" ? overallFraction(phases(jobs, undefined, src.photos)) : null;
  const root = sourceRoot(s);
  const open = () => { onFocus(); if (online && state !== "never") actions.open(root); };
  return (
    <li>
      <div className={`cat-row${active ? " active" : ""}${focused ? " focused" : ""}${state === "offline" ? " offline" : ""}`}
           role="button" tabIndex={0} data-source={src.id} title={`${root}\n${stateTitle(s, state, jobs)}`}
           onClick={open}
           onKeyDown={(e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); open(); } if (e.key === "ContextMenu") onMenu(e.currentTarget.getBoundingClientRect().left + 24, e.currentTarget.getBoundingClientRect().bottom); }}
           onContextMenu={(e) => { e.preventDefault(); e.stopPropagation(); onFocus(); onMenu(e.clientX, e.clientY); }}>
        <span className="cat-row-icon">{src.kind === "nas" ? <NetworkIcon /> : src.kind === "drive" ? <DriveIcon /> : <FolderIcon />}</span>
        <span className="name">{src.name}</span>
        <span className="cat-slot">
          <span className="cat-count">{state === "never" ? "-" : formatNumber(src.photos)}</span>
          <button className="cat-more" aria-label={t("catalog.row.more", { name: src.name })} title={t("catalog.row.more", { name: src.name })}
                  onClick={(e) => { e.stopPropagation(); const r = e.currentTarget.getBoundingClientRect(); onFocus(); onMenu(r.left, r.bottom + 2); }}>{"…"}</button>
        </span>
        <span className="cat-state" role="img" aria-label={stateTitle(s, state, jobs)} title={stateTitle(s, state, jobs)}>
          {state === "indexing" ? <Ring fraction={fraction} /> : <Glyph state={state} fraction={fraction} />}
        </span>
      </div>
    </li>
  );
}

function SourceMenu({ x, y, s, jobs, actions, onClose }: { x: number; y: number; s: SourceStatus; jobs: Jobs | undefined; actions: CatalogActions; onClose(): void }) {
  const ref = useMenuClose(onClose);
  const state = sourceState(s, jobs);
  const root = sourceRoot(s), id = s.source.id;
  const left = Math.min(x, window.innerWidth - 240), top = Math.min(y, window.innerHeight - 190);
  const running = state === "indexing", paused = state === "paused";
  const away = state === "offline";
  const go = (f: () => void) => () => { onClose(); f(); };
  return (
    <div ref={ref} className="context-menu cat-menu" style={{ left, top }} role="menu">
      <button disabled={away || state === "never"} title={away ? t("catalog.notConnected") : undefined} onClick={go(() => actions.open(root))}>{t("catalog.menu.open")}</button>
      {running && <button onClick={go(() => actions.pause(id))}>{t("catalog.act.pause")}</button>}
      {paused && <button onClick={go(() => actions.resume(id))}>{t("catalog.act.resume")}</button>}
      {!running && !paused && <button disabled={away} title={away ? t("catalog.notConnected") : undefined} onClick={go(() => actions.index(id))}>{t(s.source.photos > 0 ? "catalog.act.rescan" : "catalog.act.index")}</button>}
      {(running || paused) && <button onClick={go(() => actions.stop(id))}>{t("catalog.act.stop")}</button>}
      <div className="menu-sep" />
      <button disabled={away} title={away ? t("catalog.notConnected") : undefined} onClick={go(() => actions.reveal(root))}>{t("library.menu.showIn", { manager: FILE_MANAGER })}</button>
      <div className="menu-sep" />
      <button onClick={go(() => actions.remove(s))}>{t("catalog.menu.remove")}</button>
    </div>
  );
}

export function CatalogCard({ open, onToggle, loaded, error, sources, jobs, ended, focus, current, actions, onFocus, pin }: {
  open: boolean; onToggle(alt: boolean): void;
  loaded: boolean; error: string | null;
  sources: SourceStatus[]; jobs: Record<number, Jobs>; ended: Record<number, Ended>;
  focus: number | null; current: string | null;
  actions: CatalogActions; onFocus(id: number | null): void;
  pin?: React.ReactNode;
}) {
  const [menu, setMenu] = useState<{ x: number; y: number; id: number } | null>(null);
  const groups = groupByVolume(sources);
  const focused = sources.find((s) => s.source.id === focus) ?? null;
  const menuSource = menu ? sources.find((s) => s.source.id === menu.id) : undefined;
  const rows = (items: SourceStatus[]) => items.map((s) => (
    <Row key={s.source.id} s={s} jobs={jobs[s.source.id]} focused={focus === s.source.id}
         active={current !== null && sourceRoot(s) === current}
         actions={actions} onFocus={() => onFocus(s.source.id)} onMenu={(x, y) => setMenu({ x, y, id: s.source.id })} />
  ));
  return (
    <div className="panel catalog-panel" data-tour="catalog">
      <div className={`panel-title${open ? " open" : ""}`} onClick={(e) => onToggle(e.altKey)}>
        <span>{t("catalog.title")}</span>
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
              <ul className="cat-list">
                {groups.length < 2 ? rows(sources) : groups.map((g) => (
                  <li key={g.key}>
                    <div className="cat-volume" title={g.host ?? undefined}>
                      <span className="name">{g.name}</span>
                      {g.host && g.host !== g.name && <span className="host">{g.host}</span>}
                      <span className="cat-volume-state">{t(g.online ? "catalog.state.online" : "catalog.state.offline")}</span>
                    </div>
                    <ul>{rows(g.items)}</ul>
                  </li>
                ))}
              </ul>
              <button className="folder add" onClick={actions.add}><span>{t("catalog.addFolder")}</span></button>
            </>
          )}
          {focused && (
            <IndexCard key={focused.source.id} source={focused} jobs={jobs[focused.source.id]} ended={ended[focused.source.id]}
                       actions={{
                         onPause: () => actions.pause(focused.source.id), onResume: () => actions.resume(focused.source.id),
                         onStop: () => actions.stop(focused.source.id), onRetry: () => actions.index(focused.source.id),
                         onOpen: () => actions.open(sourceRoot(focused)), onDismiss: () => { actions.dismiss(focused.source.id); onFocus(null); },
                       }} />
          )}
          {error && <p role="alert" className="cat-error">{t("catalog.error", { why: error })}</p>}
        </div>
      )}
      {menu && menuSource && <SourceMenu x={menu.x} y={menu.y} s={menuSource} jobs={jobs[menu.id]} actions={actions} onClose={() => setMenu(null)} />}
    </div>
  );
}
