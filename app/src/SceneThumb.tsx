// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// A scene's thumbnail: the grid's cached one, drawn into a canvas that fills its
// box and letterboxes the picture inside it (object-fit: contain), so the box never
// changes size with the picture. Bitmaps are kept in a small LRU, so scrolling back
// does not decode again; the box shows --inset until the picture is there.
import { useEffect, useRef } from "react";
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
