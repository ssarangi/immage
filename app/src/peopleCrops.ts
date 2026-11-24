// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Face crops, asked for lazily: a tile that comes into view asks, one that leaves withdraws, at most six are
// in flight, the newest request goes first (what the user is looking at now), and the pictures are kept as
// object URLs in a bounded cache. A crop the backend cannot hand over (null) is remembered as "none" and the
// tile draws its own placeholder (`placeholderStyle`).
import { peopleApi } from "./peopleStore";

const MAX_INFLIGHT = 6;
const KEEP = 1500;

type Want = { face: number; size: number; cbs: Set<(url: string | null) => void> };
const urls = new Map<string, string | null>();           // "id@size" -> object URL, or null for no picture
const wants = new Map<string, Want>();
const queue: string[] = [];
let inflight = 0;

const keyOf = (face: number, size: number) => `${face}@${size}`;

function remember(key: string, url: string | null) {
  urls.set(key, url);
  while (urls.size > KEEP) {
    const [k, u] = urls.entries().next().value as [string, string | null];
    urls.delete(k);
    if (u) URL.revokeObjectURL(u);
  }
}

function pump() {
  while (inflight < MAX_INFLIGHT && queue.length) {
    const key = queue.pop()!;
    const w = wants.get(key);
    if (!w || !w.cbs.size) { wants.delete(key); continue; }      // withdrawn while it waited
    inflight++;
    void (async () => {
      let url: string | null = null;
      try { const blob = await peopleApi().crop(w.face, w.size); url = blob ? URL.createObjectURL(blob) : null; } catch { url = null; }
      remember(key, url);
      wants.delete(key);
      inflight--;
      w.cbs.forEach((cb) => cb(url));
      pump();
    })();
  }
}

// the cached picture now (undefined when it was never asked for), else asks; returns the withdrawal
export function wantCrop(face: number, size: number, cb: (url: string | null) => void): () => void {
  const key = keyOf(face, size);
  if (urls.has(key)) { cb(urls.get(key)!); return () => {}; }
  let w = wants.get(key);
  if (!w) { w = { face, size, cbs: new Set() }; wants.set(key, w); }
  w.cbs.add(cb);
  queue.push(key);
  pump();
  return () => { w!.cbs.delete(cb); };
}

export const cropSizeFor = (tile: number) => (tile > 112 ? 192 : 128);
export const cropsInFlight = () => inflight;
export const clearCrops = () => { urls.forEach((u) => { if (u) URL.revokeObjectURL(u); }); urls.clear(); wants.clear(); queue.length = 0; };

// a placeholder that is not a photo: a gradient from the face's number (never a real person's picture)
export function placeholderStyle(face: number): React.CSSProperties {
  const h = (face * 47) % 360;
  return { background: `linear-gradient(145deg, hsl(${h} 38% 46%), hsl(${(h + 40) % 360} 34% 30%))` };
}
