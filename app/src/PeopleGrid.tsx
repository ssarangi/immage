// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The People view's grid: one virtualised grid for every section. Rows are a fixed height, so the scroll
// height is rows x row height with no measuring and only the rows in view (and two each side) are mounted.
// One tab stop and aria-activedescendant (the tiles come and go as they scroll, so none can hold focus);
// click, cmd/ctrl-click, shift-click and a marquee select; the keyboard map is peopleModel's gridKey.
import { Check } from "lucide-react";
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { formatNumber, t } from "./i18n";
import {
  CAPTION, geometry, gridKey, tilePos, tilesInRect, totalHeight, visibleRange,
  type Geometry, type Intent, type Section, type Sel, type SelAction, type Tile,
} from "./peopleModel";
import { cropSizeFor, placeholderStyle, wantCrop } from "./peopleCrops";
import { holdRefresh } from "./peopleStore";

export function Crop({ face, tile, className = "pcrop", style }: { face: number | null; tile: number; className?: string; style?: React.CSSProperties }) {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    setUrl(null);
    if (face === null) return;
    return wantCrop(face, cropSizeFor(tile), setUrl);
  }, [face, tile]);
  return (
    <div className={className} style={style} aria-hidden="true">
      {url ? <img src={url} alt="" draggable={false} /> : <div className="pph" style={placeholderStyle(face ?? 0)} />}
    </div>
  );
}

export const tileLabel = (tile: Tile, byId: ReadonlyMap<number, string>): { name: string; sub: string } => {
  const n = (x: number) => ({ n: x, count: formatNumber(x) });
  switch (tile.kind) {
    case "person": return { name: tile.name, sub: t("people.tile.photos", n(tile.photos)) };
    case "group": return { name: t("people.group.hint"), sub: t("people.tile.faces", n(tile.size)) };
    case "maybe": return { name: t("people.suggest.maybe", { name: tile.name }), sub: t("people.tile.faces", n(tile.size)) };
    case "face": return { name: tile.maybe != null && byId.has(tile.maybe) ? t("people.suggest.maybe", { name: byId.get(tile.maybe)! }) : t("people.face.unnamed"), sub: "" };
  }
};

function TileView({ tile, label, g, index, selected, active, size, rename, onRename }: {
  tile: Tile; label: { name: string; sub: string }; g: Geometry; index: number; selected: boolean; active: boolean; size: number;
  rename: boolean; onRename: (name: string | null) => void;
}) {
  const p = tilePos(index, g);
  const cover = tile.kind === "person" ? tile.cover : tile.kind === "group" ? tile.rep : tile.kind === "maybe" ? tile.rep : tile.id;
  const count = tile.kind === "group" || tile.kind === "maybe" ? tile.size : null;
  const aria = tile.kind === "person" ? `${tile.name}, ${label.sub}${tile.review ? `, ${t("people.tile.toReview", { n: tile.review, count: formatNumber(tile.review) })}` : ""}`
    : `${label.name}${label.sub ? `, ${label.sub}` : ""}`;
  const done = useRef(false);
  return (
    <div role="gridcell" id={`ptile-${tile.key}`} className="ptile" data-tile={index} data-kind={tile.kind} aria-selected={selected} aria-label={aria}
         data-active={active || undefined} data-maybe={(tile.kind === "maybe" || (tile.kind === "face" && tile.maybe != null)) || undefined}
         data-dim={((tile.kind === "person" || tile.kind === "face") && tile.ignored) || undefined}
         style={{ left: p.x, top: p.y, width: size, height: size + CAPTION }} title={label.name}>
      {tile.kind === "group" && tile.members.length > 1 ? (
        <div className="pcrop pcollage" style={{ height: size }} aria-hidden="true">
          {tile.members.slice(0, 4).map((m) => <Crop key={m} face={m} tile={Math.round(size / 2)} className="pcell" />)}
        </div>
      ) : <Crop face={cover} tile={size} style={{ height: size }} />}
      <span className="ptile-badge ptile-check" aria-hidden="true"><Check size={11} /></span>
      {count !== null && <span className="ptile-badge ptile-count" aria-hidden="true">{formatNumber(count)}</span>}
      {tile.kind === "person" && tile.review > 0 && <span className="ptile-badge ptile-flag" aria-hidden="true">!</span>}
      <div className="pcap">
        {rename ? (
          <input autoFocus defaultValue={tile.kind === "person" ? tile.name : ""} aria-label={t("people.rename.label")} spellCheck={false}
                 onFocus={(e) => e.currentTarget.select()}
                 onPointerDown={(e) => e.stopPropagation()}
                 onKeyDown={(e) => {
                   e.stopPropagation();
                   if (e.key === "Enter") { done.current = true; onRename(e.currentTarget.value); }
                   else if (e.key === "Escape") { done.current = true; onRename(null); }
                 }}
                 // the focus leaving ends it like Enter does; the flag stops the second call
                 onBlur={(e) => { if (!done.current) { done.current = true; onRename(e.currentTarget.value); } }} />
        ) : <b>{label.name}</b>}
        <span>{label.sub || " "}</span>
      </div>
    </div>
  );
}

export type GridOverlay = { pos: (index: number) => { x: number; y: number }; scrollTop: number; width: number; height: number; g: Geometry };

export function PeopleGrid({ tiles, section, open, size, sel, byId, renameKey, label, dir, onSel, onIntent, onRename, onNearEnd, overlay, empty }: {
  tiles: Tile[]; section: Section; open: boolean; size: number; sel: Sel; byId: ReadonlyMap<number, string>; renameKey: string | null; label: string;
  dir: "ltr" | "rtl"; onSel: (a: SelAction) => void; onIntent: (i: Intent) => void; onRename: (key: string, name: string | null) => void;
  onNearEnd?: () => void; overlay?: (o: GridOverlay) => ReactNode; empty?: ReactNode;
}) {
  const scroller = useRef<HTMLDivElement>(null);
  const [box, setBox] = useState({ w: 800, h: 500 });
  const [top, setTop] = useState(0);
  const [marquee, setMarquee] = useState<{ x: number; y: number; w: number; h: number } | null>(null);
  const holding = useRef(false);
  useLayoutEffect(() => {
    const el = scroller.current!;
    const read = () => setBox({ w: el.clientWidth, h: el.clientHeight });
    read();
    const ro = new ResizeObserver(read);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  useEffect(() => () => { if (holding.current) { holding.current = false; holdRefresh(false); } }, []);
  const g = geometry(box.w, size);
  const range = visibleRange(top, box.h, tiles.length, g);
  const height = totalHeight(tiles.length, g);
  const active = Math.min(sel.active, Math.max(0, tiles.length - 1));

  useEffect(() => { if (onNearEnd && range.to >= tiles.length - g.cols * 3) onNearEnd(); }, [range.to, tiles.length, g.cols, onNearEnd]);

  // keep the keyboard's tile in view
  const reveal = useCallback((index: number) => {
    const el = scroller.current;
    if (!el) return;
    const y = tilePos(index, g).y;
    if (y < el.scrollTop) el.scrollTop = Math.max(0, y - g.pad);
    else if (y + g.rowH > el.scrollTop + el.clientHeight) el.scrollTop = y + g.rowH - el.clientHeight + g.pad;
  }, [g.cols, g.rowH, g.tile, g.pad]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    if ((e.target as HTMLElement).tagName === "INPUT") return;
    const intent = gridKey({ key: e.key, shift: e.shiftKey, meta: e.metaKey || e.ctrlKey, alt: e.altKey },
      { section, open, count: tiles.length, cols: g.cols, active, dir });
    if (!intent) return;
    e.preventDefault();
    e.stopPropagation();   // the app's own letters (G, L, Y...) must not fire while naming
    if (intent.kind === "move") { onSel({ type: "move", index: intent.index, extend: intent.extend }); reveal(intent.index); return; }
    if (intent.kind === "all") { onSel({ type: "all" }); return; }
    if (intent.kind === "clear") { onSel({ type: "clear" }); onIntent(intent); return; }
    onIntent(intent);
  };

  const indexAt = (e: React.PointerEvent | React.MouseEvent) => Number((e.target as HTMLElement).closest<HTMLElement>("[data-tile]")?.dataset.tile ?? -1);
  const onClick = (e: React.MouseEvent) => {
    const i = indexAt(e);
    if (i < 0) return;
    scroller.current?.focus({ preventScroll: true });
    if (e.metaKey || e.ctrlKey) onSel({ type: "toggle", index: i });
    else if (e.shiftKey) onSel({ type: "range", index: i });
    else onSel({ type: "click", index: i });
  };

  // a drag on empty space draws a rectangle; the tiles it touches are computed from the geometry, so ones that
  // are not mounted count; near the top or bottom edge it scrolls
  const drag = useRef<{ x: number; y: number; base: ReadonlySet<string>; additive: boolean; cx: number; cy: number; raf: number } | null>(null);
  const rectOf = (d: NonNullable<typeof drag.current>) => {
    const el = scroller.current!, r = el.getBoundingClientRect();
    const x = d.cx - r.left, y = d.cy - r.top + el.scrollTop;
    return { x: Math.min(d.x, x), y: Math.min(d.y, y), w: Math.abs(x - d.x), h: Math.abs(y - d.y) };
  };
  const apply = () => {
    const d = drag.current;
    if (!d) return;
    const rc = rectOf(d);
    setMarquee(rc);
    onSel({ type: "marquee", indexes: tilesInRect(rc, tiles.length, g), additive: d.additive, base: d.base });
  };
  const onPointerDown = (e: React.PointerEvent) => {
    if (e.button !== 0 || indexAt(e) >= 0 || e.target !== e.currentTarget && !(e.target as HTMLElement).classList.contains("people-canvas")) return;
    const el = scroller.current!, r = el.getBoundingClientRect();
    if (e.clientX > r.left + el.clientWidth) return;     // the scrollbar
    try { el.setPointerCapture(e.pointerId); } catch { /* a synthetic pointer (the harness) has none to capture */ }
    const additive = e.metaKey || e.ctrlKey || e.shiftKey;
    drag.current = { x: e.clientX - r.left, y: e.clientY - r.top + el.scrollTop, base: additive ? sel.keys : new Set(), additive, cx: e.clientX, cy: e.clientY, raf: 0 };
    if (!additive) onSel({ type: "clear" });
  };
  const onPointerMove = (e: React.PointerEvent) => {
    const d = drag.current;
    if (!d) return;
    d.cx = e.clientX; d.cy = e.clientY;
    const el = scroller.current!, r = el.getBoundingClientRect();
    const edge = e.clientY < r.top + 24 ? -18 : e.clientY > r.bottom - 24 ? 18 : 0;
    cancelAnimationFrame(d.raf);
    const tick = () => { if (edge && drag.current) { el.scrollTop += edge; apply(); d.raf = requestAnimationFrame(tick); } };
    if (edge) d.raf = requestAnimationFrame(tick);
    apply();
  };
  const end = () => { if (drag.current) { cancelAnimationFrame(drag.current.raf); drag.current = null; setMarquee(null); } };

  const items: ReactNode[] = [];
  for (let i = range.from; i < range.to && i < tiles.length; i++) {
    const tile = tiles[i];
    items.push(<TileView key={tile.key} tile={tile} label={tileLabel(tile, byId)} g={g} index={i} size={g.tile} selected={sel.keys.has(tile.key)} active={i === active}
                         rename={renameKey === tile.key} onRename={(name) => onRename(tile.key, name)} />);
  }
  const activeTile = tiles[active];
  return (
    <div className="people-gridwrap" style={{ position: "relative", flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
      <div ref={scroller} className="people-scroll" role="grid" tabIndex={0} aria-label={label} aria-multiselectable="true" dir={dir}
           aria-activedescendant={activeTile ? `ptile-${activeTile.key}` : undefined} aria-rowcount={Math.ceil(tiles.length / g.cols)}
           onScroll={(e) => setTop(e.currentTarget.scrollTop)} onKeyDown={onKeyDown} onClick={onClick}
           onDoubleClick={(e) => { if (indexAt(e) >= 0) onIntent({ kind: "open" }); }}
           onPointerDown={onPointerDown} onPointerMove={onPointerMove} onPointerUp={end} onPointerCancel={end}
           onPointerEnter={() => { if (!holding.current) { holding.current = true; holdRefresh(true); } }}
           onPointerLeave={() => { if (holding.current) { holding.current = false; holdRefresh(false); } }}>
        <div className="people-canvas" style={{ height }}>
          {items}
          {marquee && <div className="pmarquee" style={{ left: marquee.x, top: marquee.y, width: marquee.w, height: marquee.h }} />}
        </div>
      </div>
      {empty}
      {overlay?.({ pos: (i) => tilePos(i, g), scrollTop: top, width: box.w, height: box.h, g })}
    </div>
  );
}
