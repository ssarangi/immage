// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Thumbnails for Statistics' photo cards: a few at a time, kept per path while the view is open.
import { useEffect, useState } from "react";
import { useStatsApi, type StatsApi } from "./statsApi";
import type { PhotoRef } from "./statsTypes";

const MAX_PARALLEL = 4;
const MAX_KEPT = 160;
const kept = new Map<string, string | null>();
const waiting = new Map<string, Promise<string | null>>();
const queue: (() => void)[] = [];
let running = 0;

function pump() {
  while (running < MAX_PARALLEL && queue.length) { running++; queue.shift()!(); }
}

export function loadThumb(api: StatsApi, photo: PhotoRef, size = 480): Promise<string | null> {
  const key = `${photo.path}|${size}`;
  if (kept.has(key)) return Promise.resolve(kept.get(key)!);
  let p = waiting.get(key);
  if (!p) {
    p = new Promise<string | null>((resolve) => {
      queue.push(() => {
        api.thumb(photo, size).catch(() => null).then((url) => {
          if (kept.size >= MAX_KEPT) {
            const oldest = kept.keys().next().value as string;
            const url0 = kept.get(oldest);
            if (url0 && url0.startsWith("blob:")) URL.revokeObjectURL(url0);
            kept.delete(oldest);
          }
          kept.set(key, url);
          waiting.delete(key);
          running--;
          resolve(url);
          pump();
        });
      });
      pump();
    });
    waiting.set(key, p);
  }
  return p;
}

/** The thumbnail's address once it has loaded; the box it fills is sized by the caller, so nothing moves. */
export function useThumb(photo: PhotoRef | null | undefined, size = 480): string | null {
  const api = useStatsApi();
  const key = photo ? `${photo.path}|${size}` : "";
  const [url, setUrl] = useState<string | null>(() => (key && kept.has(key) ? kept.get(key)! : null));
  useEffect(() => {
    if (!photo) { setUrl(null); return; }
    let alive = true;
    if (kept.has(key)) setUrl(kept.get(key)!);
    else { setUrl(null); void loadThumb(api, photo, size).then((u) => { if (alive) setUrl(u); }); }
    return () => { alive = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api, key]);
  return url;
}
