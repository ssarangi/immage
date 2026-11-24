// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { ActivityBar } from "./ActivityBar";
import { type ImageEntry, type Known, LABEL_COLORS, ThumbLoader, fetchThumb, isPrimaryRaw, knownKey, pairKey } from "./library";
import { CullingIcon, SlidersIcon } from "./icons";
import { baseName } from "./paths";
import { captureDate, known, shutter } from "./MetadataPanel";
import { end } from "./activity";
import { isRemote } from "./immich";
import { hintOnScreen } from "./thumbSignals";
import { settings } from "./settings";
import { SecondaryClickGuard } from "./secondaryClick";
import { memo, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { t } from "./i18n";
import { badgeOf } from "./cullModel";
import { SmoothWheel } from "./smoothWheel";
import { useDecision } from "./cullStore";
import { EyeBadge } from "./EyeBadge";

// the gaps between cells (styles.css, .filmstrip and .filmstrip.grid)
const STRIP_GAP = 3, GRID_GAP = 6;
// lines (rows of the grid, cells of the strip) drawn beyond the screen on each side
const DRAW_OVER = 2;

// the pair names (pairKey()) that have a raw among these files
export const pairedRaws = (files: string[]) => new Set(files.filter((p) => isPrimaryRaw(p)).map(pairKey));

// the folder's images along the bottom, as Lightroom's filmstrip: a cell
// each with the file's name and type above its thumbnail, and its reject
// flag and stars below, shown on hover or once it has a rating
export function Filmstrip({ images, partners, pending, empty, current, loader, height, grid, gridSize = 200, onGridZoom, selection, candidate, onPick, onDouble,
                     onOpenKnown, onMenu, onMeta, onPress, more, onBackMenu }: {
  // photos, and those known from the folder's last read (library.ts)
  images: (ImageEntry | Known)[]; current: number | null; loader: ThumbLoader; height: number;
  pending: string[];                      // files not yet read, shown as empty cells
  partners: Map<number, ImageEntry[]>;    // a stacked raw's JPEG or HEIF, by its imgid
  empty: string;                          // what an empty grid says
  grid: boolean;                          // spread over the editing area, in rows
  gridSize?: number;                      // the grid's cell height, CSS pixels
  onGridZoom?: (factor: number) => void;  // a pinch or ⌘ wheel over the grid
  selection: Set<number>;
  candidate: number | null;               // the photo compared with the open one
  onPick: (i: ImageEntry, e: React.MouseEvent) => void;
  onDouble: (i: ImageEntry) => void;
  onOpenKnown: (k: Known, double: boolean) => void;
  onMenu: (e: React.MouseEvent, imgid: number) => void;
  onMeta: (i: ImageEntry, fields: Partial<ImageEntry>) => void;
  onPress?: (i: ImageEntry, e: React.PointerEvent<HTMLElement>) => void; // may become a drag (photoDrag.ts)
  more?: () => void;                      // there are more results: called as the end scrolls into view
  onBackMenu: (e: React.MouseEvent) => void; // right-click beside the photos
}) {
  const strip = useRef<HTMLDivElement>(null);
  // the cell after the last, which asks for the next page of results
  const end = useRef<HTMLDivElement>(null);
  const isEmpty = !images.length && !pending.length;
  // only the cells on or near the screen exist, in a list a folder's thousands
  // of photos long: the rest is two spacers the size of what they stand for.
  // everything below follows from the strip's size and the cells' pitch, so
  // it is computed, not read back from cells that may not be there
  const gap = grid ? GRID_GAP : STRIP_GAP;
  // a grid's cells are a fixed height, and share each row's width
  // (styles.css, .filmstrip.grid); the strip's follow its height
  const cellSize = grid ? Math.round(gridSize) : Math.round((height - 12) * 1.15);
  const track = Math.round(gridSize * 1.15);
  const pitch = cellSize + gap;
  // the strip's measured box: its size along the scroll, its width, and the
  // padding before the first cell and beside the cells
  const [vp, setVp] = useState({ line: 0, main: 800, cross: 1200, padStart: grid ? 10 : 6, padCross: grid ? 20 : 12 });
  const cols = grid ? Math.max(1, Math.floor((vp.cross - vp.padCross + gap) / (track + gap))) : 1;
  const total = images.length + pending.length + (more ? 1 : 0);
  const lines = Math.ceil(total / cols);
  const seen = Math.ceil(vp.main / pitch) + 1;
  // the cells drawn reach two lines past the screen each way, the thumbnails wanted a screen: a cell
  // costs style, layout and (the compositor's layerize, measured) paint in each frame it is drawn,
  // and a thumbnail kept in the loader costs nothing until its cell is
  const over = DRAW_OVER, ahead = Math.max(DRAW_OVER, seen);
  const range = (n: number) => {
    const a = Math.max(0, Math.min(lines - 1, vp.line) - n), b = Math.min(lines - 1, vp.line + seen + n);
    return [a, b] as const;
  };
  const [firstLine, lastLine] = range(over);
  const first = firstLine * cols, last = Math.min(total, (lastLine + 1) * cols);
  const [aheadFirst, aheadLast] = range(ahead);
  const wantFirst = aheadFirst * cols, wantLast = Math.min(total, (aheadLast + 1) * cols);
  const geo = useRef({ cols, pitch, cellSize, track, gap, total, padStart: vp.padStart, padCross: vp.padCross });
  geo.current = { cols, pitch, cellSize, track, gap, total, padStart: vp.padStart, padCross: vp.padCross };
  // the strip's box and place, read again when either changes. the line a
  // render depends on changes only as a row (or column) leaves the screen
  const measure = () => {
    const root = strip.current;
    if (!root) return;
    const st = getComputedStyle(root);
    const g = geo.current;
    const padStart = parseFloat(grid ? st.paddingTop : st.paddingLeft) || 0;
    const padCross = grid ? (parseFloat(st.paddingLeft) || 0) + (parseFloat(st.paddingRight) || 0) : 0;
    const main = grid ? root.clientHeight : root.clientWidth, cross = root.clientWidth;
    const pos = grid ? root.scrollTop : root.scrollLeft;
    const line = Math.max(0, Math.floor((pos - padStart) / g.pitch));
    setVp((v) => (v.line === line && v.main === main && v.cross === cross && v.padStart === padStart && v.padCross === padCross
      ? v : { line, main, cross, padStart, padCross }));
  };
  const measureRef = useRef(measure);
  measureRef.current = measure;
  useLayoutEffect(() => {
    const root = strip.current;
    if (!root) return;
    measureRef.current();
    const on = () => measureRef.current();
    root.addEventListener("scroll", on, { passive: true });
    const ro = new ResizeObserver(on);
    ro.observe(root);
    return () => { root.removeEventListener("scroll", on); ro.disconnect(); };
  }, [grid, isEmpty]);
  // the cell after the last asks for the next page when it comes near
  const endShown = !!more && last >= total;
  useEffect(() => {
    const root = strip.current, el = end.current;
    if (!root || !el || !more) return;
    const watch = new IntersectionObserver((entries) => { if (entries.some((e) => e.isIntersecting)) more(); },
                                           { root, rootMargin: grid ? "600px 0px" : "0px 600px" });
    watch.observe(el);
    return () => watch.disconnect();
  }, [more, images, grid, endShown]);
  // the cells near the screen are the thumbnails wanted, those on it first;
  // what scrolled far away is dropped from the queue
  const onScreen = useRef(new Set<number | string>());
  useEffect(() => {
    const idOf = (k: number) => { const i = images[k]; return i ? ("imgid" in i ? i.imgid : knownKey(i)) : undefined; };
    const near = new Set<number | string>(), on = new Set<number | string>(), paths: string[] = [];
    const stop = Math.min(images.length, wantLast);
    for (let k = wantFirst; k < stop; k++) {
      const id = idOf(k)!;
      near.add(id);
      if (k >= vp.line * cols && k < (vp.line + seen) * cols) {
        on.add(id);
        const i = images[k];
        if ("imgid" in i && !isRemote(i)) paths.push(i.path);
      }
    }
    const gone = [...onScreen.current].filter((id) => !on.has(id));
    loader.show(gone, false);
    loader.show([...on], true);
    onScreen.current = on;
    loader.request([...on], true);
    loader.request([...near].filter((id) => !on.has(id)));
    loader.retain(near);
    // the Catalog's thumbnail job makes these next
    hintOnScreen(paths);
  }, [images, loader, wantFirst, wantLast, vp.line, seen, cols]);
  useEffect(() => () => { loader.show([...onScreen.current], false); onScreen.current = new Set(); }, [loader]);
  // keep the open image in view as the arrows step through the folder.
  // the strip alone scrolls: scrollIntoView would also scroll its
  // ancestors, and a hidden floating strip, parked below the center
  // column, then pushes the whole column up out of its overflow: hidden
  useLayoutEffect(() => {
    const root = strip.current;
    if (!root || current === null) return;
    const at = images.findIndex((i) => "imgid" in i && i.imgid === current);
    if (at < 0) return;
    const g = geo.current;
    const start = g.padStart + Math.floor(at / g.cols) * g.pitch, stop = start + g.cellSize;
    const pos = grid ? root.scrollTop : root.scrollLeft, size = grid ? root.clientHeight : root.clientWidth;
    const to = start < pos ? start : stop > pos + size ? stop - size : pos;
    if (to === pos) return;
    if (grid) root.scrollTop = to; else root.scrollLeft = to;
    measureRef.current();
  }, [current, grid]);
  // a grid resized keeps one photo where it was: the one under the pointer
  // for a pinch, otherwise the open photo when it shows, or the top left one.
  // the column count changes as the size does, so without this the rows
  // under the pointer would run off. a scroll the page makes itself (`set`)
  // keeps the anchor; any other picks a new one, so it is there before the
  // next resize
  const anchor = useRef<{ index: number; offset: number; set: number } | null>(null);
  const takeAnchor = (x?: number, y?: number) => {
    const root = strip.current;
    if (!root) return;
    const g = geo.current;
    if (!g.total) { anchor.current = null; return; }
    const r = root.getBoundingClientRect();
    // the cell at a point, or -1 for a gap between cells when `strict`
    const indexAt = (px: number, py: number, strict = false) => {
      const down = py - r.top + root.scrollTop - g.padStart;
      const left = g.padCross / 2 + (root.clientWidth - g.padCross - (g.cols * g.track + (g.cols - 1) * g.gap)) / 2;
      const across = px - r.left - left;
      const row = Math.max(0, Math.floor(down / g.pitch)), col = Math.min(g.cols - 1, Math.max(0, Math.floor(across / (g.track + g.gap))));
      if (strict && (down < 0 || across < 0 || down - row * g.pitch >= g.cellSize || across - col * (g.track + g.gap) >= g.track
                     || row * g.cols + col >= g.total)) return -1;
      return Math.min(g.total - 1, row * g.cols + col);
    };
    let index = x !== undefined && y !== undefined ? indexAt(x, y, true) : -1;
    if (index < 0) {
      const at = current === null ? -1 : images.findIndex((i) => "imgid" in i && i.imgid === current);
      const top = at < 0 ? 0 : g.padStart + Math.floor(at / g.cols) * g.pitch - root.scrollTop;
      index = at >= 0 && top >= 0 && top + g.cellSize <= root.clientHeight ? at : indexAt(r.left + 30, r.top + 30);
    }
    anchor.current = { index, offset: g.padStart + Math.floor(index / g.cols) * g.pitch - root.scrollTop, set: -1 };
  };
  const zoom = useRef(onGridZoom);
  zoom.current = onGridZoom;
  useEffect(() => {
    const root = strip.current;
    if (!root || !grid) return;
    // about 4% per wheel notch, as the photo's zoom; capped, as macOS wheel
    // acceleration can report several notches in one event. a plain wheel
    // scrolls
    // a mouse notch is eased (smoothWheel.ts); a touchpad and the keys scroll natively
    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)");
    const smooth = new SmoothWheel(root, {
      now: () => performance.now(), raf: (cb) => requestAnimationFrame(cb), cancel: (id) => cancelAnimationFrame(id),
      reduced: () => reduced.matches, dpr: () => window.devicePixelRatio || 1,
    });
    const wheel = (e: WheelEvent) => {
      if (!(e.ctrlKey || e.metaKey) || !zoom.current) {
        if (smooth.handle(e)) e.preventDefault();
        return;
      }
      smooth.cancel();
      e.preventDefault();
      const delta = Math.max(-50, Math.min(50, e.deltaY));
      takeAnchor(e.clientX, e.clientY);
      zoom.current(Math.exp(-delta * 0.006));
    };
    const scroll = () => {
      const a = anchor.current;
      if (!a || Math.abs(root.scrollTop - a.set) > 1) takeAnchor();
    };
    takeAnchor();
    root.addEventListener("wheel", wheel, { passive: false });
    root.addEventListener("scroll", scroll, { passive: true });
    return () => {
      smooth.cancel();
      root.removeEventListener("wheel", wheel);
      root.removeEventListener("scroll", scroll);
    };
  }, [grid, isEmpty]);
  useLayoutEffect(() => {
    const root = strip.current;
    if (!root || !grid) return;
    if (!anchor.current) takeAnchor();
    const a = anchor.current;
    if (!a) return;
    const g = geo.current;
    root.scrollTop = g.padStart + Math.floor(Math.min(a.index, g.total - 1) / g.cols) * g.pitch - a.offset;
    a.set = root.scrollTop;
    measureRef.current();
  }, [gridSize, grid]);
  const cell = useMemo(() => (grid ? { height: cellSize } : { width: cellSize }), [grid, cellSize]);
  // the cells are memoized, and call through this, so a folder's next
  // batch or a new selection redraws only the cells that changed
  const on = useRef({ onPick, onDouble, onMenu, onMeta, onPress });
  on.current = { onPick, onDouble, onMenu, onMeta, onPress };
  const openKnown = useRef(onOpenKnown);
  openKnown.current = onOpenKnown;
  // an empty grid still covers the photo, and says why it is empty
  if (isEmpty) {
    return grid ? <div className="filmstrip grid empty" onContextMenu={onBackMenu}><div className="grid-empty">{empty}</div></div>
      : null;
  }
  const spacer = (lines: number) => {
    if (lines <= 0) return null;
    const size = lines * pitch - gap;
    return grid ? <div aria-hidden style={{ gridColumn: "1 / -1", height: size }} />
                : <div aria-hidden style={{ flex: "none", width: size }} />;
  };
  const cells: React.ReactNode[] = [];
  for (let k = first; k < last; k++) {
    if (k < images.length) {
      const i = images[k];
      cells.push(!("imgid" in i)
        ? <KnownCell key={`k${knownKey(i)}`} entry={i} loader={loader} cell={cell} open={openKnown} doubleOpens={grid} />
        : (
        <FilmCell key={i.imgid} entry={i} loader={loader} cell={cell} on={on}
                  types={[i, ...(partners.get(i.imgid) ?? [])]
                    .map((f) => (f.filename.match(/\.([^.]+)$/)?.[1] ?? "").toUpperCase()).join("+")}
                  active={i.imgid === current} selected={selection.size > 1 && selection.has(i.imgid)}
                  candidate={i.imgid === candidate} doubleOpens={grid || candidate !== null} />
      ));
    } else if (k < images.length + pending.length) {
      const p = pending[k - images.length];
      cells.push(<GhostCell key={p} path={p} cell={cell} />);
    } else cells.push(<div key="more" ref={end} className="thumb ghost more" style={cell} />);
  }
  return (
    <div className={`filmstrip${grid ? " grid" : ""}`} ref={strip}
         style={grid ? { "--grid-column": `${track}px` } as React.CSSProperties : { height }}
         onContextMenu={onBackMenu}
         onWheel={(e) => { if (strip.current && !grid) strip.current.scrollLeft += e.deltaY + e.deltaX; }}>
      {spacer(firstLine)}
      {cells}
      {spacer(lines - 1 - lastLine)}
    </div>
  );
}

// `dogEar` turns the bottom left corner over, as Lightroom marks a virtual
// copy: drawn on the thumbnail, which the cell fits, so it sits on the
// photo's own corner whatever its shape
export type CellHandlers = Pick<Parameters<typeof Filmstrip>[0], "onPick" | "onDouble" | "onMenu" | "onMeta" | "onPress">;

// one photo's filmstrip cell
export const FilmCell = memo(function FilmCell({ entry: i, loader, cell, on, types, active, selected, candidate, doubleOpens }: {
  entry: ImageEntry; loader: ThumbLoader; cell: React.CSSProperties; on: React.RefObject<CellHandlers>;
  types: string; active: boolean; selected: boolean; candidate: boolean; doubleOpens: boolean;
}) {
  const clickGuard = useRef(new SecondaryClickGuard());
  const cull = useDecision(i.path, i.version);
  return (
    <div className={`thumb${active ? " active" : ""}${i.rejected ? " rejected" : ""}`
                    + `${i.rating || i.rejected ? " rated" : ""}${selected ? " selected" : ""}`
                    + `${candidate ? " candidate" : ""}`}
         style={cell}
         title={i.filename} data-imgid={i.imgid} onClick={(e) => {
           if (clickGuard.current.click(e.detail)) return;
           on.current!.onPick(i, e);
         }}
         onPointerDown={(e) => {
           if (clickGuard.current.pointerDown(e.button)) return;
           if (!(e.target as Element).closest(".thumb-foot")) on.current!.onPress?.(i, e);
         }}
         onDoubleClick={() => { if (doubleOpens) on.current!.onDouble(i); }}
         onContextMenu={(e) => { clickGuard.current.contextMenu(e.button); e.stopPropagation(); on.current!.onMenu(e, i.imgid); }}>
      <div className="thumb-head">
        <span className="thumb-name">{i.filename.replace(/\.[^.]+$/, "")}</span>
        {/* altered, as darktable counts it, so its lighttable agrees */}
        {i.edited && <span className="thumb-edited" title={t("library.filmstrip.edited")}><SlidersIcon /></span>}
        <span className="thumb-type">{types}{copyName(i)}</span>
      </div>
      <div className="thumb-image"><ThumbCanvas loader={loader} imgid={i.imgid} dogEar={!!i.copy} />
        {cull && <span className={`cull-badge ${badgeOf(cull)}`}>{{ best: "★", keep: "✓", reject: "✕" }[badgeOf(cull)]} {t(`cull.badge.${badgeOf(cull)}`)}
          {cull.group !== null && <span className="cull-badge-set"> · {t("cull.badge.set", { n: cull.group })}</span>}</span>}
        {!isRemote(i) && <EyeBadge path={i.path} />}
      </div>
      {/* rated without opening the photo, as Lightroom's filmstrip does;
          an Immich result shows Immich's rating, set only once opened */}
      <div className={`thumb-foot${isRemote(i) ? " read-only" : ""}`} onClick={(e) => e.stopPropagation()}>
        {!isRemote(i) && <button className={`thumb-reject${i.rejected ? " on" : ""}`} title={t("library.filmstrip.reject")}
                onClick={() => on.current!.onMeta(i, { rejected: !i.rejected })}>✕</button>}
        <span className="thumb-stars">
          {[1, 2, 3, 4, 5].map((n) => (
            <button key={n} className={!i.rejected && i.rating >= n ? "on" : ""}
                    title={isRemote(i) ? t("library.filmstrip.immichRating") : t("library.filmstrip.stars", { count: n })}
                    disabled={isRemote(i)}
                    onClick={() => on.current!.onMeta(i, { rating: i.rating === n ? 0 : n, rejected: false })}>★</button>
          ))}
        </span>
        <span className="labels">
          {LABEL_COLORS.map((c, k) => (i.labels & (1 << k)
            ? <span key={k} className="dot" style={{ background: c }} /> : null))}
        </span>
        {/* grid only (styles.css): the culling lightbox on this photo */}
        {!isRemote(i) && <button className="thumb-cull" title={t("library.filmstrip.culling")}
                onClick={() => window.dispatchEvent(new CustomEvent("grid-culling-open", { detail: { imgid: i.imgid } }))}><CullingIcon /></button>}
      </div>
    </div>
  );
});

// a photo known from the folder's last read, not yet read by this one: as
// it was then, and read-only until it is read, when a FilmCell takes its place
export const KnownCell = memo(function KnownCell({ entry: i, loader, cell, open, doubleOpens }: {
  entry: Known; loader: ThumbLoader; cell: React.CSSProperties;
  open: React.RefObject<(k: Known, double: boolean) => void>; doubleOpens: boolean;
}) {
  return (
    <div className={`thumb known${i.rejected ? " rejected" : ""}${i.rating || i.rejected ? " rated" : ""}`} style={cell}
         title={i.filename} data-known={knownKey(i)} onClick={() => { if (!doubleOpens) open.current!(i, false); }}
         onDoubleClick={() => { if (doubleOpens) open.current!(i, true); }}>
      <div className="thumb-head">
        <span className="thumb-name">{i.filename.replace(/\.[^.]+$/, "")}</span>
        {i.edited && <span className="thumb-edited" title={t("library.filmstrip.edited")}><SlidersIcon /></span>}
        <span className="thumb-type">{(i.filename.match(/\.([^.]+)$/)?.[1] ?? "").toUpperCase()}{copyName(i)}</span>
      </div>
      <div className="thumb-image"><ThumbCanvas loader={loader} imgid={knownKey(i)} dogEar={!!i.copy} /></div>
      <div className="thumb-foot read-only">
        {i.rejected && <button className="thumb-reject on" disabled>✕</button>}
        <span className="thumb-stars">
          {[1, 2, 3, 4, 5].map((n) => <button key={n} className={!i.rejected && i.rating >= n ? "on" : ""} disabled>★</button>)}
        </span>
        <span className="labels">
          {LABEL_COLORS.map((c, k) => (i.labels & (1 << k)
            ? <span key={k} className="dot" style={{ background: c }} /> : null))}
        </span>
      </div>
    </div>
  );
});

// a file not yet read, shown as an empty cell
export const GhostCell = memo(function GhostCell({ path, cell }: { path: string; cell: React.CSSProperties }) {
  const name = baseName(path);
  return (
    <div className="thumb ghost" style={cell} title={name}>
      <div className="thumb-head">
        <span className="thumb-name">{name.replace(/\.[^.]+$/, "")}</span>
        <span className="thumb-type">{(name.match(/\.([^.]+)$/)?.[1] ?? "").toUpperCase()}</span>
      </div>
      <div className="thumb-image"><ThumbCanvas /></div>
      <div className="thumb-foot" />
    </div>
  );
});

// the folder's thumbnails loading, redrawn a few times a second rather
// than for each one
export function ThumbProgress({ loader }: { loader: ThumbLoader }) {
  const [, setTick] = useState(0);
  useEffect(() => {
    let timer = 0;
    const off = loader.watchProgress(() => {
      if (!timer) timer = window.setTimeout(() => { timer = 0; setTick((n) => n + 1); }, 250);
    });
    return () => { off(); window.clearTimeout(timer); };
  }, [loader]);
  const progress = loader.progress();
  return progress.done < progress.total ? <ActivityBar what={t("library.filmstrip.loading")} done={progress.done} total={progress.total} /> : null;
}

// the image's thumbnail as the loader has it, redrawn when that changes
// a print cell's photo: the engine's thumbnail at the size the cell shows it
// (the filmstrip's is 240 pixels, soft in a big cell), soft proofed when the
// proof is on. it lasts as long as the cell, so it is not kept in the loader
export function CellCanvas({ loader, imgid, proof, proofKey, pixels }:
                    { loader?: ThumbLoader; imgid: number; proof: boolean; proofKey: string; pixels: number }) {
  const ref = useRef<HTMLCanvasElement>(null);
  const [ready, setReady] = useState(false);
  // the engine could not make it: the library's thumbnail stands in, unproofed
  const [failed, setFailed] = useState(false);
  // the cached sizes are steps; asking in steps keeps a resize from refetching
  const size = Math.min(1440, Math.max(256, Math.ceil(pixels / 128) * 128));
  useEffect(() => {
    let stale = false;
    fetchThumb(imgid, size, undefined, proof).then((bitmap) => {
      const c = ref.current;
      if (stale || !c) return;
      c.width = bitmap.width;
      c.height = bitmap.height;
      c.getContext("2d", { willReadFrequently: true })!.drawImage(bitmap, 0, 0);
      setReady(true);
      setFailed(false);
    }).catch((e) => { if (!stale) { console.error(`print cell thumbnail ${imgid}: ${e}`); setFailed(true); } });
    return () => { stale = true; };
  }, [imgid, size, proofKey]);
  return <>
    <canvas ref={ref} style={ready ? undefined : { display: "none" }} />
    {!ready && <ThumbCanvas loader={failed ? loader : undefined} imgid={failed ? imgid : undefined} />}
  </>;
}

export function ThumbCanvas({ loader, imgid, dogEar = false }: { loader?: ThumbLoader; imgid?: number | string; dogEar?: boolean }) {
  const [bitmap, setBitmap] = useState(() => (imgid === undefined ? undefined : loader?.get(imgid)));
  useEffect(() => {
    if (!loader || imgid === undefined) return;
    setBitmap(loader.get(imgid));
    return loader.watch(imgid, () => setBitmap(loader.get(imgid)));
  }, [loader, imgid]);
  const ref = useRef<HTMLCanvasElement>(null);
  useEffect(() => {
    const c = ref.current;
    if (!c || !bitmap?.width) return;
    c.width = bitmap.width;
    c.height = bitmap.height;
    // WebKitGTK stops showing GPU-backed thumbnail canvases once a photo is
    // open, though their pixels are intact; unaccelerated ones are painted
    // with the page
    const g = c.getContext("2d", { willReadFrequently: true })!;
    g.drawImage(bitmap, 0, 0);
    if (!dogEar) return;
    const d = Math.round(Math.max(c.width, c.height) * 0.09), h = c.height;
    // the corner cut away, showing the cell behind, and the flap folded over
    g.globalCompositeOperation = "destination-out";
    g.beginPath(); g.moveTo(0, h - d); g.lineTo(d, h); g.lineTo(0, h); g.closePath(); g.fill();
    g.globalCompositeOperation = "source-over";
    g.fillStyle = "#d9d9d9";
    g.beginPath(); g.moveTo(0, h - d); g.lineTo(d, h - d); g.lineTo(d, h); g.closePath(); g.fill();
  }, [bitmap, dogEar]);
  // a photo's outline until its thumbnail is built
  return bitmap ? <canvas ref={ref} /> : (
    <div className="placeholder">
      <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round"
           strokeLinejoin="round"><rect x="3.5" y="5" width="17" height="14" rx="2" />
        <circle cx="9" cy="10" r="1.6" /><path d="m4 17 5-4.5 3.5 3 3-2.5 4.5 4" /></svg>
    </div>
  );
}

// a virtual copy's name after its file's, as Lightroom's "Copy 1"
// the info over the photo, as Lightroom's I cycles it: 0 the file (its
// folder, capture time and size), 1 the exposure and lens, 2 none
export const INFO_MODE_KEY = "immage.infoMode";
export function loadInfoMode(): number {
  try { const v = Number(settings.get(INFO_MODE_KEY)); return v >= 0 && v <= 2 ? v : 0; } catch { return 0; }
}
export function InfoOverlay({ mode, name, dir, entry }: { mode: number; name: string; dir: string; entry: ImageEntry | null }) {
  const e = entry?.exif;
  const lines = mode === 0
    ? [dir, e?.datetime ? captureDate(e.datetime) : "",
       entry && entry.width > 0 && entry.height > 0 ? `${entry.width} × ${entry.height}` : ""]
    : [[shutter(e?.exposure ?? 0),
        e && known(e.aperture) && e.aperture > 0 ? `ƒ / ${e.aperture.toFixed(1)}` : "",
        e && known(e.iso) && e.iso > 0 ? `ISO ${Math.round(e.iso)}` : ""]
         .filter(Boolean).join(", ").replace(/^(.*sec), (ƒ)/, "$1 at $2"),
       [e && known(e.focal_length) && e.focal_length > 0 ? `${+e.focal_length.toFixed(0)} mm` : "",
        e?.lens ? `(${e.lens})` : ""].filter(Boolean).join(" ")];
  return (
    <div className="info-overlay">
      <div className="info-name">{name}</div>
      {lines.filter(Boolean).map((l, k) => <div key={k}>{l}</div>)}
    </div>
  );
}

export const copyName = (i: Known | null | undefined) => (i?.copy ? ` · Copy ${i.version}` : "");
