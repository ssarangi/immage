// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Places map: Leaflet over the app's cached OpenStreetMap tiles (the one tile command the location editor
// uses, behind StatsApi.mapTile), darkened by a CSS filter on the tile pane only. The data is not Leaflet
// markers: one SVG in a Leaflet pane (so it pans with the map) holds a bubble per merged cell, and a canvas
// in another pane draws the Heat mode. Small pins size modestly by count, carry a count label and a dashed ring when they
// are several areas merged, and are buttons: hover and keyboard focus show the shared tooltip, arrow keys move
// between bubbles, Enter selects (or, for a merged one, zooms in). When no tile arrives the map keeps its flat
// background and draws a labelled graticule, so the bubbles still read as places.
import L from "leaflet";
import "leaflet/dist/leaflet.css";
import "./places.css";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent } from "react";
import { createPortal } from "react-dom";
import { t } from "../i18n";
import { nf, photosText } from "../statsFormat";
import type { StatsApi } from "../statsApi";
import type { PlaceCell } from "../statsTypes";
import { Seg } from "./Controls";
import {
  MAX_LAT, MAX_ZOOM, MIN_ZOOM, LABEL_MIN, SOLO_RADIUS, bubbles, clampBbox, compact, coords, debounce, degrees, graticule, inside, neighbour, roundBbox, tileBounds,
  type Bbox, type Bubble, type Dir, type Mode, type View,
} from "./placesGeometry";

const OSM_COPYRIGHT = "https://www.openstreetmap.org/copyright";
/** Leaflet reports a view; the page asks for new cells after this long without another */
export const VIEW_DEBOUNCE_MS = 250;
const FAILED_BEFORE_OFFLINE = 3;
/** the lowest zoom at which the whole-world square (256 * 2^z px) still covers a frame of this size, to the pixel: below it
 *  dark strips show beside or above the tiles, and above it the map would not show the whole world at its widest */
const coverZoom = (w: number, h: number) => Math.max(1, Math.log2(Math.max(w, h, 1) / 256) + 0.002);
/** the next whole zoom level up or down, so a step from the fitted (fractional) zoom lands on a level the tiles are sharp at */
const stepZoom = (z: number, d: number, min: number) => Math.max(min, Math.min(MAX_ZOOM, d > 0 ? Math.floor(z + 1e-6) + 1 : Math.ceil(z - 1e-6) - 1));
const DIR: Record<string, Dir> = { ArrowLeft: "left", ArrowRight: "right", ArrowUp: "up", ArrowDown: "down" };

export interface MapFocus { box: Bbox; maxZoom: number; token: number }

export interface PlacesMapProps {
  api: StatsApi;
  cells: readonly PlaceCell[];
  heat?: readonly [number, number, number][];
  mode: Mode;
  onMode(m: Mode): void;
  /** the place filter's keys: bubbles inside them are drawn selected, with their outline */
  selected: readonly string[];
  /** the extent of the photos: the first view fits it */
  fit: Bbox;
  /** a request to show this box (a quick-jump chip) */
  focus: MapFocus | null;
  /** the viewport changed (debounced); the first one comes at once */
  onView(v: View): void;
  onSelect(key: string, additive: boolean): void;
}

const cssColor = (ctx: CanvasRenderingContext2D, value: string, fallback: string) => {
  ctx.fillStyle = "#000";
  ctx.fillStyle = value.trim() || fallback;
  const v = String(ctx.fillStyle);
  const hex = /^#([0-9a-f]{6})$/i.exec(v);
  if (hex) return [parseInt(hex[1].slice(0, 2), 16), parseInt(hex[1].slice(2, 4), 16), parseInt(hex[1].slice(4), 16)] as const;
  const rgb = /rgba?\((\d+),\s*(\d+),\s*(\d+)/.exec(v);
  return rgb ? ([Number(rgb[1]), Number(rgb[2]), Number(rgb[3])] as const) : ([92, 214, 208] as const);
};

export function PlacesMap(props: PlacesMapProps) {
  const { api, cells, heat, mode, selected, fit, focus } = props;
  const latest = useRef(props);
  latest.current = props;
  const host = useRef<HTMLDivElement>(null);
  const stage = useRef<HTMLDivElement>(null);
  const [map, setMap] = useState<L.Map | null>(null);
  const [pane, setPane] = useState<HTMLElement | null>(null);
  const [rev, setRev] = useState(0);
  const [tilesOk, setTilesOk] = useState(true);
  const [zoom, setZoom] = useState(MIN_ZOOM);
  const [minZoom, setMinZoom] = useState(MIN_ZOOM);
  const tileStats = useRef({ ok: 0, bad: 0 });
  const layer = useRef<L.GridLayer | null>(null);
  const heatCanvas = useRef<HTMLCanvasElement | null>(null);
  const dragged = useRef(0);

  // the map, once; the data layers follow props
  useEffect(() => {
    const el = host.current;
    if (!el) return;
    let alive = true;
    const view = L.map(el, {
      attributionControl: false, zoomControl: false, scrollWheelZoom: false, zoomAnimation: false, fadeAnimation: false, markerZoomAnimation: false,
      inertia: !window.matchMedia("(prefers-reduced-motion: reduce)").matches, minZoom: coverZoom(el.clientWidth, el.clientHeight), maxZoom: MAX_ZOOM, zoomSnap: 0,
      maxBounds: [[-MAX_LAT, -180], [MAX_LAT, 180]], maxBoundsViscosity: 1, worldCopyJump: false,
    });
    const box = fit;
    view.fitBounds([[box[1], box[0]], [box[3], box[2]]], { animate: false, padding: [76, 76], maxZoom: 9 });
    el.setAttribute("aria-label", t("stats.places.map"));
    const overlay = view.createPane("statsPlaces");
    overlay.style.zIndex = "450";
    const heatPane = view.createPane("statsHeat");
    heatPane.style.zIndex = "440";
    const canvas = document.createElement("canvas");
    canvas.className = "st-pl-heat";
    heatPane.appendChild(canvas);
    heatCanvas.current = canvas;

    // tiles: only the visible ones, no look-ahead, fetched by the app's own cached command (two at a time)
    const Tiles = L.GridLayer.extend({
      createTile(coords: L.Coords, done: L.DoneCallback) {
        const tile = document.createElement("img");
        tile.alt = "";
        api.mapTile(coords.z, coords.x, coords.y).then((bytes) => {
          if (!alive) return;
          const url = URL.createObjectURL(new Blob([bytes], { type: "image/png" }));
          tile.onload = () => { URL.revokeObjectURL(url); tileStats.current.ok++; setTilesOk(true); done(undefined, tile); };
          tile.onerror = () => { URL.revokeObjectURL(url); fail(); done(new Error("tile"), tile); };
          tile.src = url;
        }).catch(() => { if (!alive) return; fail(); done(new Error("tile"), tile); });
        return tile;
      },
    });
    const fail = () => {
      tileStats.current.bad++;
      if (tileStats.current.ok === 0 && tileStats.current.bad >= FAILED_BEFORE_OFFLINE) setTilesOk(false);
    };
    const tiles = new Tiles() as L.GridLayer;
    L.setOptions(tiles, { minZoom: 0, maxZoom: MAX_ZOOM, keepBuffer: 0, updateWhenIdle: true, updateWhenZooming: false, noWrap: true });
    tiles.addTo(view);
    layer.current = tiles;

    const current = (): View => ({ zoom: Math.round(view.getZoom()), bbox: roundBbox(clampBbox([view.getBounds().getWest(), view.getBounds().getSouth(), view.getBounds().getEast(), view.getBounds().getNorth()])) });
    const settle = debounce<View>((v) => latest.current.onView(v), VIEW_DEBOUNCE_MS);
    const changed = () => { setRev((r) => r + 1); setZoom(view.getZoom()); settle.call(current()); };
    view.on("moveend zoomend resize", changed);
    view.on("dragend", () => { dragged.current = Date.now(); });
    // a plain wheel scrolls the page; the wheel zooms with Ctrl or Cmd held (which is also how a trackpad pinch arrives)
    let acc = 0;
    const wheel = (e: WheelEvent) => {
      if (!e.ctrlKey && !e.metaKey) return;
      e.preventDefault();
      acc += e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY;
      if (Math.abs(acc) < 40) return;
      const dir = acc < 0 ? 1 : -1;
      acc = 0;
      view.setZoomAround(view.mouseEventToContainerPoint(e), stepZoom(view.getZoom(), dir, view.getMinZoom()), { animate: false });
    };
    el.addEventListener("wheel", wheel, { passive: false });
    // the frame changes size without a window resize (the panel loads, the sidebar toggles, the page is resized):
    // Leaflet only listens to the window, so without this tiles keep covering the old size and strips show.
    // The lowest zoom follows the frame, so the world always reaches every edge.
    let size = { w: el.clientWidth, h: el.clientHeight };
    const observer = new ResizeObserver(() => {
      const w = el.clientWidth, h = el.clientHeight;
      if (w === size.w && h === size.h) return;
      const was = size;
      size = { w, h };
      if (!w || !h) return;
      view.invalidateSize({ animate: false, pan: false });
      const min = coverZoom(w, h);
      view.setMinZoom(min);
      setMinZoom(min);
      if (!was.w || !was.h) view.fitBounds([[box[1], box[0]], [box[3], box[2]]], { animate: false, padding: [76, 76], maxZoom: 9 });
      else if (view.getZoom() < min) view.setZoom(min, { animate: false });
      view.panInsideBounds(view.options.maxBounds as L.LatLngBounds, { animate: false });
    });
    observer.observe(el);
    setMinZoom(view.getMinZoom());
    setMap(view);
    setPane(overlay);
    setZoom(view.getZoom());
    latest.current.onView(current());
    return () => {
      alive = false;
      settle.cancel();
      observer.disconnect();
      el.removeEventListener("wheel", wheel);
      view.remove();
      layer.current = null;
      heatCanvas.current = null;
      setMap(null);
      setPane(null);
    };
    // the first fit is the one the map was made with
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api]);

  // a quick-jump chip: show its box
  useEffect(() => {
    if (!map || !focus) return;
    map.fitBounds([[focus.box[1], focus.box[0]], [focus.box[3], focus.box[2]]], { animate: false, padding: [40, 40], maxZoom: focus.maxZoom });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [map, focus?.token]);

  // the heat canvas follows the map while it moves
  const drawHeat = useCallback(() => {
    const view = map, canvas = heatCanvas.current;
    if (!view || !canvas) return;
    const size = view.getSize();
    if (canvas.width !== size.x || canvas.height !== size.y) { canvas.width = size.x; canvas.height = size.y; }
    L.DomUtil.setPosition(canvas, view.containerPointToLayerPoint([0, 0]));
    const g = canvas.getContext("2d")!;
    g.clearRect(0, 0, size.x, size.y);
    if (mode !== "heat" || !heat?.length) return;
    const styles = getComputedStyle(host.current!);
    const light = document.documentElement.dataset.scheme === "light";
    const cyan = cssColor(g, styles.getPropertyValue("--stat-cyan"), "#5cd6d0"), amber = cssColor(g, styles.getPropertyValue("--stat-amber"), "#f4b84a");
    const max = heat.reduce((m, h) => Math.max(m, h[2]), 1);
    g.globalCompositeOperation = light ? "source-over" : "lighter";
    const blob = (x: number, y: number, r: number, c: readonly number[], a: number) => {
      const grad = g.createRadialGradient(x, y, 0, x, y, r);
      grad.addColorStop(0, `rgba(${c[0]},${c[1]},${c[2]},${a})`);
      grad.addColorStop(1, `rgba(${c[0]},${c[1]},${c[2]},0)`);
      g.fillStyle = grad;
      g.fillRect(x - r, y - r, r * 2, r * 2);
    };
    for (const [lat, lng, w] of heat) {
      const p = view.latLngToContainerPoint([lat, lng]);
      if (p.x < -80 || p.y < -80 || p.x > size.x + 80 || p.y > size.y + 80) continue;
      const k = Math.sqrt(w / max);
      blob(p.x, p.y, 16 + 34 * k, cyan, light ? 0.3 : 0.28 + 0.3 * k);
    }
    for (const [lat, lng, w] of heat) {
      const p = view.latLngToContainerPoint([lat, lng]);
      const k = Math.sqrt(w / max);
      if (k < 0.35) continue;
      blob(p.x, p.y, 8 + 18 * k, amber, light ? 0.3 : 0.2 + 0.25 * k);
    }
  }, [map, heat, mode]);
  useEffect(() => {
    if (!map) return;
    drawHeat();
    map.on("move", drawHeat);
    return () => { map.off("move", drawHeat); };
  }, [map, drawHeat, rev]);

  // bubbles at this zoom, in the layer's pixels
  const list = useMemo<Bubble[]>(() => {
    if (!map) return [];
    return bubbles(cells, (lat, lng) => { const p = map.latLngToLayerPoint([lat, lng]); return { x: p.x, y: p.y }; }, mode === "heat" ? "photos" : mode);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [map, cells, mode, rev]);

  const outlines = useMemo(() => {
    if (!map) return [];
    const out: { key: string; x: number; y: number; w: number; h: number }[] = [];
    for (const key of selected) {
      const b = tileBounds(key);
      if (!b) continue;
      const a = map.latLngToLayerPoint([b[3], b[0]]), c = map.latLngToLayerPoint([b[1], b[2]]);
      out.push({ key, x: a.x, y: a.y, w: Math.max(2, c.x - a.x), h: Math.max(2, c.y - a.y) });
    }
    return out;
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [map, selected, rev]);

  const lines = useMemo(() => {
    if (!map || tilesOk) return null;
    const b = map.getBounds().pad(1);
    const box: Bbox = [b.getWest(), b.getSouth(), b.getEast(), b.getNorth()];
    const span = map.getBounds();
    const g = graticule([span.getWest(), span.getSouth(), span.getEast(), span.getNorth()]);
    const wide = graticule(box, g.step);
    const lat = (v: number) => map.latLngToLayerPoint([v, box[0]]).y;
    const lng = (v: number) => map.latLngToLayerPoint([box[1], v]).x;
    const corner = map.containerPointToLayerPoint([8, 14]);
    return {
      step: g.step,
      h: wide.lats.map((v) => ({ v, y: lat(v), x1: lng(box[0]), x2: lng(box[2]) })),
      v: wide.lngs.map((v) => ({ v, x: lng(v), y1: lat(box[3]), y2: lat(box[1]) })),
      labelX: corner.x,
      labelY: map.containerPointToLayerPoint([0, map.getSize().y - 30]).y,
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [map, tilesOk, rev]);

  const activate = (b: Bubble, e: { shiftKey: boolean; metaKey: boolean; ctrlKey: boolean }) => {
    if (Date.now() - dragged.current < 250) return;
    if (b.merged && map && map.getZoom() < MAX_ZOOM) {
      const bounds = L.latLngBounds([b.box[1], b.box[0]], [b.box[3], b.box[2]]);
      const to = Math.min(MAX_ZOOM, Math.max(map.getZoom() + 1, map.getBoundsZoom(bounds, false, L.point(70, 70))));
      map.setView(bounds.getCenter(), to, { animate: false });
      return;
    }
    latest.current.onSelect(b.key, e.shiftKey || e.metaKey || e.ctrlKey);
  };

  const onKey = (b: Bubble) => (e: KeyboardEvent<SVGGElement>) => {
    const dir = DIR[e.key];
    if (dir) {
      const next = neighbour(list, b, dir);
      e.preventDefault();
      e.stopPropagation();
      if (next) stage.current?.querySelector<SVGGElement>(`[data-cell="${CSS.escape(next.key)}"]`)?.focus();
    } else if (e.key === "Enter" || e.key === " ") { e.preventDefault(); e.stopPropagation(); activate(b, e); }
  };

  const retry = () => {
    tileStats.current = { ok: 0, bad: 0 };
    setTilesOk(true);
    layer.current?.redraw();
  };
  const zoomBy = (d: number) => map?.setZoom(stepZoom(map.getZoom(), d, map.getMinZoom()), { animate: false });
  const refit = () => map?.fitBounds([[fit[1], fit[0]], [fit[3], fit[2]]], { animate: false, padding: [76, 76], maxZoom: 9 });
  const first = list[0]?.key;
  const isOn = (b: Bubble) => b.keys.some((k) => selected.some((s) => inside(k, s)));
  // a coarser bubble that holds the selected area gets the ring, not the fill
  const holds = (b: Bubble) => !isOn(b) && b.keys.some((k) => selected.some((s) => inside(s, k)));

  return (
    <div className="st-pl-map" ref={stage}>
      <div className="st-pl-leaflet" ref={host} />
      {map && pane && createPortal(
        <svg className="st-pl-svg" width="1" height="1" role="group" aria-label={t("stats.places.bubbles")}>
          {lines && (
            <g className="st-pl-grid" aria-hidden="true">
              {lines.h.map((l) => <line key={`h${l.v}`} x1={l.x1} x2={l.x2} y1={l.y} y2={l.y} />)}
              {lines.v.map((l) => <line key={`v${l.v}`} x1={l.x} x2={l.x} y1={l.y1} y2={l.y2} />)}
              {lines.h.map((l) => <text key={`ht${l.v}`} x={lines.labelX} y={l.y - 4}>{degrees(l.v, "lat", lines.step)}</text>)}
              {lines.v.map((l) => <text key={`vt${l.v}`} x={l.x + 4} y={lines.labelY}>{degrees(l.v, "lng", lines.step)}</text>)}
            </g>
          )}
          {list.map((b) => {
            const on = isOn(b);
            const label = b.r >= 14 && (b.n >= LABEL_MIN || b.merged) ? compact(mode === "top" ? b.top : b.n) : "";
            const where = coords(b.lat, b.lng);
            const solo = !!b.solo && !b.merged;
            const heatDot = mode === "heat";
            const r = heatDot ? Math.min(b.r, 5) : b.r;
            const tip = solo ? t("stats.places.solo") : photosText(b.n);
            const sub = b.merged ? t("stats.places.merged", { areas: nf(b.keys.length) }) : `${t("stats.places.tipTop", { n: nf(b.top) })} · ${where}`;
            return (
              <g key={b.key} className={`st-pl-b${on ? " on" : ""}${holds(b) ? " holds" : ""}${solo ? " solo" : ""}${b.merged ? " merged" : ""}${heatDot ? " heat" : ""}`}
                 transform={`translate(${b.x} ${b.y})`} role="button" tabIndex={b.key === first ? 0 : -1} data-cell={b.key}
                 aria-pressed={b.merged ? undefined : on} aria-label={b.merged ? `${tip}. ${sub}` : t("stats.places.bubble.aria", { photos: tip, top: nf(b.top), where })}
                 data-tip={tip} data-tip-sub={sub} onClick={(e: MouseEvent<SVGGElement>) => activate(b, e)} onKeyDown={onKey(b)}>
                <circle className="ring" r={r + 4} />
                <circle className="dot" r={solo ? SOLO_RADIUS : r} />
                {label && !heatDot && <text className="lbl" dy="0.36em">{label}</text>}
              </g>
            );
          })}
          {outlines.map((o) => <rect key={o.key} className="st-pl-outline" x={o.x} y={o.y} width={o.w} height={o.h} rx="3" />)}
        </svg>, pane)}

      <div className="st-pl-modes"><Seg<Mode> value={mode} label={t("stats.places.mode")} onChange={props.onMode}
        options={[{ id: "photos", label: t("stats.places.mode.photos") }, { id: "top", label: t("stats.places.mode.top") }, { id: "heat", label: t("stats.places.mode.heat") }]} /></div>
      <div className="st-pl-zoom" role="group" aria-label={t("stats.places.zoom")}>
        <button type="button" onClick={() => zoomBy(1)} disabled={zoom >= MAX_ZOOM - 0.01} aria-label={t("stats.places.zoomIn")} data-tip={t("stats.places.zoomIn")}>+</button>
        <button type="button" onClick={() => zoomBy(-1)} disabled={zoom <= minZoom + 0.01} aria-label={t("stats.places.zoomOut")} data-tip={t("stats.places.zoomOut")}>−</button>
        <button type="button" onClick={refit} aria-label={t("stats.places.fit")} data-tip={t("stats.places.fit")}>
          <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M2.5 6V2.5H6M10 2.5h3.5V6M13.5 10v3.5H10M6 13.5H2.5V10" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg>
        </button>
      </div>

      <div className="st-pl-notes">
        {!tilesOk && (
          <p className="st-pl-offline" role="status">
            <span>{t("stats.places.offline")}</span>
            <button type="button" className="st-link" onClick={retry}>{t("stats.error.retry")}</button>
          </p>
        )}
        <p className="st-pl-attr">©{" "}
          <button type="button" onClick={() => { void openUrl(OSM_COPYRIGHT).catch(() => {}); }}>{t("stats.places.attr")}</button>
        </p>
      </div>
    </div>
  );
}
