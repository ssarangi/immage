// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Statistics view: a full-window overlay below the title strip, like the importer, loaded on first
// use. The editor stays mounted underneath. Header (scope, top-images threshold, labels), then a scroll body
// with the hero, the coverage banner, six stat cards, a sticky nav with the filter bar, and the sections.
// The numbers come from the search index through one StatsApi; every chart element filters (statsModel.ts),
// photos open. Fonts and styles load with this chunk only.
import "@fontsource/instrument-serif/latin-400.css";
import "@fontsource/instrument-serif/latin-400-italic.css";
import "@fontsource/jetbrains-mono/latin-500.css";
import "@fontsource/jetbrains-mono/latin-600.css";
import "./statistics.css";
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { t } from "./i18n";
import { StatsApiContext, commandsApi, type StatsApi } from "./statsApi";
import { chips, clearAll, clearLast, removeChip } from "./statsModel";
import { chipText, nf } from "./statsFormat";
import { useStatsData, useStatsState, viewMemory } from "./statsStore";
import type { PhotoRef, StatsFilter } from "./statsTypes";
import { CamerasSection } from "./stats/CamerasSection";
import { CoverageBanner } from "./stats/CoverageBanner";
import { FilterBar } from "./stats/FilterBar";
import { FirstRun } from "./stats/FirstRun";
import { LensesSection } from "./stats/LensesSection";
import { ErrorCard } from "./stats/Section";
import { StatCards } from "./stats/StatCards";
import { StatsHeader } from "./stats/StatsHeader";
import { StatsHero } from "./stats/StatsHero";
import { StatsNav, type Tab } from "./stats/StatsNav";
import { TipLayer } from "./stats/Tip";
import { ViewContext, type ViewCtx } from "./stats/ctx";

/** The sections on the page; the later phases add Exposure, Time, Places and Top images here. */
const SECTION_IDS = ["cameras", "lenses"] as const;
const ANCHOR: Record<string, string> = { overview: "st-top", cameras: "cameras", lenses: "lenses" };

export interface StatisticsProps {
  onClose(): void;
  /** the grid, showing the photos with these handles (ids from stats_open_in_grid), titled with the filter */
  onOpenInGrid(ids: string[], title: string): void;
  onOpenPhoto(photo: PhotoRef, list: PhotoRef[]): void;
  onAddFolders(): void;
  onLearnMore?(): void;
  /** the Folders panel's current folder, for the "This folder" scope */
  currentFolder?: string | null;
  api?: StatsApi;
}

export default function Statistics({ onClose, onOpenInGrid, onOpenPhoto, onAddFolders, onLearnMore, currentFolder = null, api = commandsApi }: StatisticsProps) {
  const [state, update] = useStatsState();
  const data = useStatsData(api, state);
  const root = useRef<HTMLDivElement>(null);
  const scroller = useRef<HTMLDivElement>(null);
  const nav = useRef<HTMLElement | null>(null);
  const [active, setActive] = useState("overview");
  const [gridBusy, setGridBusy] = useState(false);
  const [gridError, setGridError] = useState<string | null>(null);

  const response = data.response;
  const cov = data.coverage;
  const firstRun = cov?.state === "no-roots" || cov?.state === "empty" ? cov.state : null;
  const loading = data.phase === "loading" && !response;
  const stale = data.phase === "refreshing";
  const list = useMemo(() => chips(state), [state]);
  const defs = response?.defs;
  const has = useCallback((id: string) => id === "overview" || (SECTION_IDS as readonly string[]).includes(id), []);

  const scrollTo = useCallback((id: string) => {
    const sc = scroller.current;
    const el = document.getElementById(ANCHOR[id] ?? id);
    if (!sc || !el) return;
    if (id === "overview") { sc.scrollTop = 0; return; }
    const navH = nav.current?.offsetHeight ?? 0;
    sc.scrollTop += el.getBoundingClientRect().top - sc.getBoundingClientRect().top - navH;
  }, []);

  const openGrid = useCallback(async (override?: Partial<StatsFilter>) => {
    setGridBusy(true);
    setGridError(null);
    try {
      const filter: StatsFilter = override ? { minRating: state.filter.minRating, onlyTop: false, ...override } : state.filter;
      const { ids } = await api.openInGrid(state.scope, filter);
      const title = override ? "" : list.map((c) => { const x = chipText(c, defs); return `${x.kind}: ${x.text}`; }).join(", ");
      onOpenInGrid(ids, title);
    } catch (e) {
      setGridError(String(e instanceof Error ? e.message : e));
    } finally { setGridBusy(false); }
  }, [api, state, list, defs, onOpenInGrid]);

  const ctx: ViewCtx = {
    state, update, response, loading, stale, error: data.phase === "error" ? data.error : null, retry: data.reload,
    openPhoto: onOpenPhoto, openGrid: (o) => { void openGrid(o); }, scrollTo, has,
  };

  // Esc: an open popover closes itself first (it listens in the capture phase and stops the event), then the
  // last chip goes, then the view closes
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || e.defaultPrevented) return;
      e.preventDefault();
      e.stopPropagation();
      const next = clearLast(state);
      if (next) update(next); else onClose();
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  }, [state, update, onClose]);

  // scroll position is kept while the app is open; the tab follows the scroll
  useLayoutEffect(() => { if (scroller.current) scroller.current.scrollTop = viewMemory.scrollTop; root.current?.focus({ preventScroll: true }); }, []);
  const onScroll = () => {
    const sc = scroller.current;
    if (!sc) return;
    viewMemory.scrollTop = sc.scrollTop;
    const navH = nav.current?.offsetHeight ?? 0;
    const top = sc.getBoundingClientRect().top + navH + 40;
    let cur = "overview";
    for (const id of SECTION_IDS) { const el = document.getElementById(id); if (el && el.getBoundingClientRect().top <= top) cur = id; }
    setActive(cur);
  };

  const overview = response?.overview;
  const tabs: Tab[] = [
    { id: "overview", label: t("stats.nav.overview") },
    { id: "cameras", label: t("stats.nav.cameras"), count: overview?.gear.bodies },
    { id: "lenses", label: t("stats.nav.lenses"), count: overview?.gear.lenses },
  ];
  const matches = overview?.matches ?? null;

  return (
    <StatsApiContext.Provider value={api}>
      <ViewContext.Provider value={ctx}>
        <div className="stats-view" role="dialog" aria-label={t("stats.title")} ref={root} tabIndex={-1}>
          <StatsHeader state={state} update={update} folder={currentFolder} onRefresh={data.reload} onClose={onClose} busy={data.phase === "loading" || stale} />
          <div className="st-body">
            <div className="st-scroll" ref={scroller} onScroll={onScroll}>
              <div className="st-wrap">
                <StatsHero response={response} loading={loading} firstRun={!!firstRun} />
                {firstRun ? <FirstRun kind={firstRun} onAdd={onAddFolders} onLearn={() => onLearnMore?.()} /> : <>
                  <CoverageBanner coverage={cov} onAddFolders={onAddFolders} onTogglePause={(p) => { void api.setPaused(p).catch(() => {}).then(data.reload); }} />
                  {data.phase === "error" && !response ? <div className="st-stats-error"><ErrorCard message={data.error ?? ""} onRetry={data.reload} /></div>
                    : <div className={`st-dimmable${stale ? " is-stale" : ""}`}><StatCards /></div>}
                </>}
              </div>
              {!firstRun && <>
                <StatsNav tabs={tabs} active={active} onJump={scrollTo} refreshing={stale} busy={gridBusy} openCount={matches?.n ?? null} onOpenGrid={() => { void openGrid(); }}
                          ref={nav}
                          filterBar={<FilterBar chips={list} defs={defs} matches={matches} total={cov?.indexed ?? null} loading={loading}
                                                 onRemove={(k) => update(removeChip(state, k))} onClear={() => update(clearAll)} />} />
                <div className={`st-wrap st-dimmable${stale ? " is-stale" : ""}`}>
                  <CamerasSection />
                  <LensesSection />
                  {gridError && <p className="st-grid-error" role="alert">{gridError}</p>}
                  <div className="st-foot"><span>{overview ? t("stats.foot.computed", { n: nf(overview.photos) }) : " "}</span><span>{t("stats.foot.private")}</span></div>
                </div>
              </>}
            </div>
            <TipLayer root={root} />
          </div>
        </div>
      </ViewContext.Provider>
    </StatsApiContext.Provider>
  );
}
