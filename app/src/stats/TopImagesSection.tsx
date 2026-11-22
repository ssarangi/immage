// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Top images section: the frames at or above the threshold under the page's filter, best rating first and
// then newest, a page of 24 at a time through the `stats_top_page` cursors (statsTop.ts keeps the pages straight
// when the filter changes mid-scroll). Above the grid: the colour-label facet strip and the best frame of each
// lens, camera or place. Nothing is asked until the section has been near the screen. The grid's cards keep their
// boxes whether or not their image is loaded: only the cards near the view fetch one, so a long grid stays light.
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { t } from "../i18n";
import { useStatsApi } from "../statsApi";
import { chipText, nf, richParts, starsAtLeast } from "../statsFormat";
import { buildRequest, chips, clearAll, setMinRating, setOnlyTop, setValues, toggleValue, selectedSet } from "../statsModel";
import { columnsOf, emptyAdvice, moveFocus, TopFeed, visibleRange, withoutLabels, type StripKind } from "../statsTop";
import type { ColorLabel, TopImages } from "../statsTypes";
import { LabelStrip } from "./LabelStrip";
import { useNear } from "./PlacesSection";
import { Bold, SectionHead } from "./Section";
import { TopCard, TopCardSkeleton } from "./TopCard";
import { TopStrip } from "./TopStrip";
import { useView } from "./ctx";
import "./top.css";

const SKELETONS = 12;
const REVEAL_MARGIN = 700;
const DIRTY_DELAY = 1000;

export function TopImagesSection() {
  const v = useView();
  const api = useStatsApi();
  const { state, update, response, loading } = v;
  const sec = useRef<HTMLElement>(null);
  const grid = useRef<HTMLDivElement>(null);
  const near = useNear(sec);
  const threshold = starsAtLeast(state.filter.minRating);

  const feed = useMemo(() => new TopFeed({
    async first(req) {
      const res = await api.query(req);
      if ("notModified" in res || !res.top) throw new Error("The top images did not come with the answer");
      return res.top;
    },
    page: (cursor) => api.topPage(cursor),
  }), [api]);
  const snap = useSyncExternalStore(feed.subscribe, feed.get, feed.get);

  const request = useMemo(() => buildRequest(state, ["top"]), [state.scope, state.filter]); // eslint-disable-line react-hooks/exhaustive-deps
  const requestKey = JSON.stringify(request);
  useEffect(() => { if (near) feed.request(JSON.parse(requestKey)); }, [feed, near, requestKey]);

  // the cache changed, or Refresh asked for fresh numbers: the first page again (the grid is kept when nothing in it moved)
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const off = api.onDirty(() => { clearTimeout(timer); timer = setTimeout(() => feed.refresh(), DIRTY_DELAY); });
    return () => { clearTimeout(timer); off(); };
  }, [api, feed]);
  const answer = response?.generatedAt;
  useEffect(() => { if (near && answer) feed.refresh(); }, [feed, near, answer]);

  // the label counts: the feed's own, unless a label is picked (that answer counts only the picked ones); then one more small question without the labels
  const labels = selectedSet(state, "labels");
  const [facet, setFacet] = useState<{ key: string; top: TopImages } | null>(null);
  const facetRequest = useMemo(() => { const r = buildRequest(state, ["top"]); r.filter = withoutLabels(r.filter); return r; }, [state.scope, state.filter]); // eslint-disable-line react-hooks/exhaustive-deps
  const facetKey = JSON.stringify(facetRequest);
  const needFacet = labels.size > 0;
  useEffect(() => {
    if (!near || !needFacet) return;
    let alive = true;
    api.query(JSON.parse(facetKey)).then((res) => { if (alive && !("notModified" in res) && res.top) setFacet({ key: facetKey, top: res.top }); }).catch(() => {});
    return () => { alive = false; };
  }, [api, near, needFacet, facetKey, answer]);
  const counts: Record<ColorLabel, number> | null = needFacet ? (facet?.top.byLabel ?? null) : snap.byLabel;

  // which cards are near the view (they load their image), and the next page when the end is near
  const [range, setRange] = useState<[number, number]>([0, -1]);
  const [active, setActive] = useState(0);
  const count = snap.items.length;
  const measure = useCallback(() => {
    const g = grid.current;
    const sc = g?.closest<HTMLElement>(".st-scroll");
    if (!g || !sc || !g.children.length) return;
    const view = sc.getBoundingClientRect();
    const cards = g.children;
    const top = (i: number) => cards[i].getBoundingClientRect().top;
    const bottom = (i: number) => cards[i].getBoundingClientRect().bottom;
    const r = visibleRange(top, bottom, cards.length, view.top, view.bottom, REVEAL_MARGIN);
    setRange((prev) => (prev[0] === r[0] && prev[1] === r[1] ? prev : r));
    const cols = columnsOf(Array.from({ length: Math.min(cards.length, 12) }, (_, i) => top(i)));
    if (r[1] >= cards.length - 1 - cols) feed.more();
  }, [feed]);
  useLayoutEffect(() => { measure(); }, [measure, count, snap.phase, snap.next]);
  useEffect(() => {
    const sc = grid.current?.closest<HTMLElement>(".st-scroll");
    const tick = setInterval(measure, 400);
    sc?.addEventListener("scroll", measure, { passive: true });
    window.addEventListener("resize", measure);
    return () => { clearInterval(tick); sc?.removeEventListener("scroll", measure); window.removeEventListener("resize", measure); };
  }, [measure, snap.phase]);
  useEffect(() => { setActive((a) => Math.min(a, Math.max(0, count - 1))); }, [count]);

  const onKey = (e: React.KeyboardEvent<HTMLDivElement>) => {
    const cur = (e.target as HTMLElement).closest<HTMLElement>("[data-index]");
    const g = grid.current;
    if (!cur || !g) return;
    const cards = g.children;
    const cols = columnsOf(Array.from({ length: Math.min(cards.length, 12) }, (_, i) => cards[i].getBoundingClientRect().top));
    const move = moveFocus(e.key, Number(cur.dataset.index), cols, cards.length, !!snap.next);
    if (!move) return;
    e.preventDefault();
    if (move.more) feed.more();
    g.querySelector<HTMLElement>(`[data-index="${move.index}"]`)?.focus();
  };

  const total = snap.total;
  const ready = snap.phase === "ready";
  const empty = ready && total === 0;
  const showGrid = count > 0 && snap.phase !== "error";
  const skeleton = !showGrid && !empty && snap.phase !== "error";
  const advice = emptyAdvice(state.filter.minRating, chips(state).length);
  const picked = (kind: StripKind, key: string) => update((s) => setValues(s, kind === "lenses" ? "lenses" : kind === "bodies" ? "bodies" : "place", [key]));

  const openGrid = () => {
    const parts = chips(state).map((c) => { const x = chipText(c); return `${x.kind}: ${x.text}`; });
    v.openGrid({ ...state.filter, onlyTop: true }, [t("stats.top.gridTitle", { threshold }), ...parts].join(", "));
  };

  const status = snap.paging === "error" ? snap.pageError
    : count && total ? (count >= total ? t("stats.top.shownAll", { n: nf(total) }) : t("stats.top.shown", { count: nf(count), total: nf(total) })) : " ";
  const capNote = total > 20000;

  return (
    <section className="st-sec" id="top" ref={sec}>
      <SectionHead eyebrow={t("stats.top.eyebrow")} title={t("stats.top.title")} em={t("stats.top.titleEm")}
        aside={<button type="button" className="st-btn" disabled={!total} onClick={openGrid}>{t("stats.top.openGrid", { n: total, count: nf(total) })}</button>}
        lede={<Bold parts={richParts(t("stats.top.lede", { threshold: `**${threshold}**` }))} />} />

      <LabelStrip counts={counts} selected={labels} loading={!counts && (loading || snap.phase !== "error")} onlyTop={state.filter.onlyTop}
                  onToggle={(l, add) => update((s) => toggleValue(s, "labels", l, add))} onOnlyTop={() => update((s) => setOnlyTop(s, !s.filter.onlyTop))}
                  tail={ready ? t("stats.top.total", { n: total, count: nf(total) }) : undefined} />

      <TopStrip state={state} response={response} loading={loading} onOpen={v.openPhoto} onFilter={picked} update={update} />

      <div className={`st-top-body${snap.stale ? " is-stale" : ""}`}>
        {snap.phase === "error" ? (
          <div className="st-card st-state-card st-top-state" role="alert">
            <h3>{t("stats.top.error.title")}</h3>
            <p className="st-sub">{snap.error}</p>
            <button type="button" className="st-btn" onClick={() => feed.retry()}>{t("stats.error.retry")}</button>
          </div>
        ) : empty ? (
          <div className="st-card st-state-card st-top-state" role="status">
            <h3>{t("stats.top.empty.title", { n: state.filter.minRating })}</h3>
            <p className="st-sub">{advice.lowerTo ? t("stats.top.empty.lower") : t("stats.top.empty.none")}</p>
            <div className="st-top-actions">
              {advice.lowerTo && <button type="button" className="st-btn primary" onClick={() => update((s) => setMinRating(s, advice.lowerTo!))}>{t("stats.top.empty.lowerTo", { n: advice.lowerTo })}</button>}
              {advice.canClear && <button type="button" className="st-btn" onClick={() => update(clearAll)}>{t("stats.filters.clear")}</button>}
            </div>
          </div>
        ) : (
          <div className="st-top-grid" ref={grid} onKeyDown={onKey} role="group" aria-label={t("stats.top.grid")} aria-busy={skeleton || snap.paging === "loading"}>
            {skeleton ? Array.from({ length: SKELETONS }, (_, i) => <TopCardSkeleton key={i} more={i >= 8} />)
              : snap.items.map((p, i) => (
                <TopCard key={p.id} photo={p} rank={i + 1} index={i} near={i >= range[0] - 0 && i <= range[1]} tabIndex={i === active ? 0 : -1}
                         onFocus={() => setActive(i)} onOpen={() => v.openPhoto(p, snap.items)} />
              ))}
          </div>
        )}
        <div className="st-tfoot" aria-live="polite">
          <span className="st-fact">{status}</span>
          {capNote && <span className="st-fact">{t("stats.top.cap")}</span>}
          <span className="st-grow" />
          {snap.pageError ? <button type="button" className="st-btn" onClick={() => feed.retry()}>{t("stats.error.retry")}</button>
            : <button type="button" className="st-btn" disabled={!snap.next || snap.paging === "loading"} onClick={() => feed.more()}>{snap.paging === "loading" ? t("stats.top.loading") : t("stats.top.more")}</button>}
        </div>
      </div>
    </section>
  );
}
