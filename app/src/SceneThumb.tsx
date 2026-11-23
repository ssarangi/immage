// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// A scene's thumbnail: the grid's cached one, drawn into a canvas that fills its
// box and letterboxes the picture inside it (object-fit: contain), so the box never
// changes size with the picture. Bitmaps are kept in a small LRU, so scrolling back
// does not decode again; the box shows --inset until the picture is there.
import { useEffect, useRef } from "react";
import type { CullBlurRegion } from "./bindings";
import { clampRegion } from "./scenes";
import { fetchCachedThumb, fetchThumb, type ImageEntry } from "./library";

const KEEP = 160;
const bitmaps = new Map<string, Promise<ImageBitmap | null>>();

// the harness supplies synthetic pictures here; the app reads the grid's cache
let override: ((entry: ImageEntry) => Promise<ImageBitmap | null>) | null = null;
export function setThumbSource(fn: typeof override) { override = fn; bitmaps.clear(); }

function load(entry: ImageEntry): Promise<ImageBitmap | null> {
  const key = `${entry.version ?? 0}:${entry.path}`;
  const hit = bitmaps.get(key);
  if (hit) { bitmaps.delete(key); bitmaps.set(key, hit); return hit; }
  const made = (async () => {
    try { return override ? await override(entry) : (await fetchCachedThumb(entry, 240)) ?? await fetchThumb(entry.imgid, 240, entry); } catch { return null; }
  })();
  bitmaps.set(key, made);
  while (bitmaps.size > KEEP) {
    const oldest = bitmaps.keys().next().value!;
    // a bitmap is freed once it is no longer the one a canvas is waiting on
    void bitmaps.get(oldest)!.then((b) => b?.close());
    bitmaps.delete(oldest);
  }
  return made;
}

export function SceneThumb({ entry, className }: { entry: ImageEntry | undefined; className?: string }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const id = entry ? `${entry.version ?? 0}:${entry.path}` : "";
  useEffect(() => {
    const el = canvas.current;
    if (!el || !entry) return;
    let stale = false;
    el.width = 1; el.height = 1;
    void load(entry).then((bitmap) => {
      if (stale || !bitmap || !canvas.current) return;
      try {
        canvas.current.width = bitmap.width; canvas.current.height = bitmap.height;
        canvas.current.getContext("2d")?.drawImage(bitmap, 0, 0);
      } catch { /* a bitmap freed under a fast scroll: the box stays empty */ }
    });
    return () => { stale = true; };
  }, [id]);   // eslint-disable-line react-hooks/exhaustive-deps
  return <canvas ref={canvas} className={className ?? "scene-img"} aria-hidden="true" />;
}

// the picture the sharpest-region crop is cut from: the 1024 px preview the grid and the lightbox share
// (the harness supplies one too)
async function loadLarge(entry: ImageEntry): Promise<ImageBitmap | null> {
  try { return override ? await override(entry) : (await fetchCachedThumb(entry, 1024)) ?? await fetchThumb(entry.imgid, 1024, entry); } catch { return null; }
}

// The sharpest region of a frame, cut from its preview and drawn into a box of one fixed size (the picture is
// fitted inside it, as a thumbnail is), so the box never changes with the picture or while it loads
export function SharpCrop({ entry, region }: { entry: ImageEntry | undefined; region: CullBlurRegion | null | undefined }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  const id = entry ? `${entry.version ?? 0}:${entry.path}` : "";
  const r = region ? `${region.x},${region.y},${region.width},${region.height}` : "";
  useEffect(() => {
    const el = canvas.current;
    if (!el) return;
    el.width = 1; el.height = 1;
    const c = clampRegion(region);
    if (!entry || !c) return;
    let stale = false;
    void loadLarge(entry).then((bitmap) => {
      if (stale || !bitmap || !canvas.current) return;
      try {
        const sx = Math.round(c.x * bitmap.width), sy = Math.round(c.y * bitmap.height);
        const sw = Math.max(1, Math.min(bitmap.width - sx, Math.round(c.w * bitmap.width)));
        const sh = Math.max(1, Math.min(bitmap.height - sy, Math.round(c.h * bitmap.height)));
        canvas.current.width = sw; canvas.current.height = sh;
        canvas.current.getContext("2d")?.drawImage(bitmap, sx, sy, sw, sh, 0, 0, sw, sh);
      } catch { /* a bitmap freed under a fast change: the box stays empty */ }
      bitmap.close();
    });
    return () => { stale = true; };
  }, [id, r]);   // eslint-disable-line react-hooks/exhaustive-deps
  return <canvas ref={canvas} className="scene-crop-img" aria-hidden="true" />;
}
