// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Places section: the map (PlacesMap.tsx) and the selected area's panel (PlacePanel.tsx) in one card of fixed
// height. Nothing is asked of the backend, and no tile is fetched, until the section has been near the screen.
// The map's viewport (zoom and box) drives `stats_query`'s `places` request, debounced by the map; a pan inside
// the box already answered, plus the backend's margin, asks nothing. A click on a bubble sets the `place` filter
// (a chip, like every chart element) and the panel loads `stats_place` for it. The place filter itself is left
// out of the map's request, as the contract says (the map shows the alternatives), so selecting does not redraw it.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { t } from "../i18n";
import { buildRequest, chips, clearAll, clearDim, setValues, toggleValue, valuesOf } from "../statsModel";
import { nf } from "../statsFormat";
import { useStatsApi } from "../statsApi";
import type { Places, PlaceCell, PlaceDetail, StatsFilter } from "../statsTypes";
import { PlacePanel } from "./PlacePanel";
import { PlacesMap, type MapFocus } from "./PlacesMap";
import { ErrorCard, SectionHead } from "./Section";
import { MIN_ZOOM, needsRequest, tileBounds, type Mode, type View } from "./placesGeometry";
import { useView } from "./ctx";
import "./places.css";

const WORLD: View = { zoom: MIN_ZOOM, bbox: null };

/** true once the element has been within 400 px of the scroller's view; never goes back, so the map stays */
export function useNear(ref: React.RefObject<HTMLElement | null>): boolean {
  const [near, setNear] = useState(false);
  useEffect(() => {
    const el = ref.current;
    if (!el || near) return;
    const scroller = el.closest<HTMLElement>(".st-scroll");
    const check = () => {
      const r = el.getBoundingClientRect(), s = scroller?.getBoundingClientRect() ?? { top: 0, bottom: window.innerHeight };
      if (r.bottom > s.top - 400 && r.top < s.bottom + 400) setNear(true);
    };
    // the scroll event, and a slow tick for a scroll made by a script (no event comes before the next frame)
    const first = setTimeout(check, 0), tick = setInterval(check, 400);
    scroller?.addEventListener("scroll", check, { passive: true });
    window.addEventListener("resize", check);
    return () => { clearTimeout(first); clearInterval(tick); scroller?.removeEventListener("scroll", check); window.removeEventListener("resize", check); };
  }, [ref, near]);
  return near;
}

const withoutPlace = (f: StatsFilter): StatsFilter => { const { place: _p, ...rest } = f; void _p; return rest as StatsFilter; };

export function PlacesSection() {
  const v = useView();
  const api = useStatsApi();
  const { state, update, response } = v;
  const sec = useRef<HTMLElement>(null);
  const near = useNear(sec);
  const [mode, setMode] = useState<Mode>("photos");
  const [asked, setAsked] = useState<View>(WORLD);
  const [places, setPlaces] = useState<Places | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);
  const [focus, setFocus] = useState<MapFocus | null>(null);
  const [focusKey, setFocusKey] = useState<string | null>(null);
  const asking = useRef<View>(WORLD);
  const heat = mode === "heat";

  // the map's answer: the filter without its place selection, at the viewport last asked for
  const request = useMemo(() => {
    const r = buildRequest(state, ["places"]);
    r.filter = withoutPlace(r.filter);
    return r;
  }, [state.scope, state.filter]); // eslint-disable-line react-hooks/exhaustive-deps
  const requestKey = JSON.stringify([request, heat, asked]);
  useEffect(() => {
    if (!near) return;
    let alive = true;
    asking.current = asked;
    api.query({ ...request, places: { zoom: asked.zoom, ...(asked.bbox ? { bbox: asked.bbox } : {}), ...(heat ? { heat: true } : {}) } }).then((res) => {
      if (!alive || "notModified" in res) return;
      setPlaces(res.places ?? null);
      setError(null);
    }).catch((e) => { if (alive) setError(String(e instanceof Error ? e.message : e)); });
    return () => { alive = false; };
  }, [api, near, requestKey, tick]); // eslint-disable-line react-hooks/exhaustive-deps

  // the cache changed: ask again once things have been quiet for a second
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const off = api.onDirty(() => { clearTimeout(timer); timer = setTimeout(() => setTick((n) => n + 1), 1000); });
    return () => { clearTimeout(timer); off(); };
  }, [api]);

  const onView = useCallback((view: View) => { setAsked((prev) => (needsRequest(prev, view) ? view : prev)); }, []);

  // the selected area
  const selected = valuesOf(state.filter, "place").map(String);
  const activeKey = focusKey && selected.includes(focusKey) ? focusKey : selected[selected.length - 1] ?? null;
  const detailFilter = JSON.stringify(request.filter) + JSON.stringify(state.scope);
  const [detail, setDetail] = useState<{ key: string; filter: string; d: PlaceDetail } | null>(null);
  const [detailError, setDetailError] = useState<{ key: string; message: string } | null>(null);
  const [retry, setRetry] = useState(0);
  useEffect(() => {
    if (!activeKey) return;
    let alive = true;
    api.place(activeKey, request.filter, state.scope).then((d) => { if (alive) { setDetail({ key: activeKey, filter: detailFilter, d }); setDetailError(null); } })
      .catch((e) => { if (alive) setDetailError({ key: activeKey, message: String(e instanceof Error ? e.message : e) }); });
    return () => { alive = false; };
  }, [api, activeKey, detailFilter, retry, tick]); // eslint-disable-line react-hooks/exhaustive-deps
  const have = detail && detail.key === activeKey ? detail.d : null;
  const failed = detailError && detailError.key === activeKey ? detailError.message : null;

  const onSelect = useCallback((key: string, additive: boolean) => {
    setFocusKey(key);
    update((s) => toggleValue(s, "place", key, additive));
  }, [update]);
  const onJump = useCallback((c: PlaceCell) => {
    setFocusKey(c.key);
    update((s) => setValues(s, "place", [c.key]));
    const box = tileBounds(c.key);
    if (box) setFocus((f) => ({ box, maxZoom: Math.max(MIN_ZOOM, c.key.length - 2), token: (f?.token ?? 0) + 1 }));
  }, [update]);
  const clear = useCallback(() => { setFocusKey(null); update((s) => clearDim(s, "place")); }, [update]);

  const known = places?.known;
  const photos = selected.length ? undefined : response?.overview?.photos;
  const without = known !== undefined && photos !== undefined ? Math.max(0, photos - known) : undefined;
  const filtered = chips(state).some((c) => c.dim !== "place");
  const ready = !!places && places.known > 0 && !!places.bbox;
  const none = !!places && places.known === 0;

  const head = (
    <SectionHead eyebrow={t("stats.places.eyebrow")} title={t("stats.places.title")} em={t("stats.places.titleEm")} tail={t("stats.places.titleTail")}
      aside={<span className="st-fact">{known === undefined ? <i className="st-sk st-sk-fact" /> : <>
        <b>{nf(known)}</b>{photos !== undefined && <> / {nf(photos)}</>} {t("stats.places.known")} {"·"} <b>{t("stats.places.count", { n: places!.total, count: nf(places!.total) })}</b></>}</span>}
      lede={t("stats.places.lede")} />
  );

  // no photo has a position at all (and nothing is filtered): a short card
  if (none && !filtered) {
    return (
      <section className="st-sec st-places" id="places" ref={sec} data-ready="1">
        {head}
        <div className="st-card st-pl-empty">
          <h3>{t("stats.places.empty.title")}</h3>
          <p className="st-sub">{t("stats.places.empty.body")}</p>
        </div>
      </section>
    );
  }
  if (error && !places) {
    return <section className="st-sec st-places" id="places" ref={sec} data-ready="1">{head}<ErrorCard message={error} onRetry={() => setTick((n) => n + 1)} /></section>;
  }

  return (
    <section className="st-sec st-places" id="places" ref={sec} data-ready={ready || none ? "1" : undefined}>
      {head}
      <div className="st-pl">
        <div className="st-pl-stage">
          {ready ? (
            <PlacesMap api={api} cells={places!.cells} heat={places!.heat} mode={mode} onMode={setMode} selected={selected} fit={places!.bbox!} focus={focus}
                       onView={onView} onSelect={onSelect} />
          ) : (
            <div className="st-pl-map st-pl-wait">
              {none ? <div className="st-pl-none" role="status"><span>{t("stats.places.noMatch")}</span>
                <button type="button" className="st-link" onClick={() => update(clearAll)}>{t("stats.filters.clear")}</button></div>
                : <i className="st-sk st-pl-sk" />}
            </div>
          )}
        </div>
        <PlacePanel detail={have} loading={!!activeKey && !have && !failed} error={failed} selected={!!activeKey} minRating={state.filter.minRating}
                    top={places?.top ?? []} activeKeys={selected} known={known} total={places?.total} onJump={onJump} onClear={clear}
                    onOpenGrid={() => activeKey && v.openGrid({ ...state.filter, place: [activeKey] })} onOpenPhoto={v.openPhoto}
                    onRetry={() => setRetry((n) => n + 1)} />
      </div>
      <p className="st-pl-foot">{without !== undefined && without > 0 ? t("stats.places.without", { n: without, count: nf(without) }) : " "}</p>
    </section>
  );
}
