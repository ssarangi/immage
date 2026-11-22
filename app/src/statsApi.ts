// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// What the Statistics view asks of the outside world, as one small interface so the view
// can be mounted on fixtures (harness/statistics.tsx) and so no component calls a command.
// The real one goes through commands.ts; thumbnails come from the existing thumbnail cache.
import { createContext, useContext } from "react";
import { commands } from "./commands";
import { normalizePlace, normalizeReply } from "./statsNormalize";
import type { Coverage, PhotoRef, PlaceDetail, StatsFilter, StatsRequest, StatsResponse, StatsScope } from "./statsTypes";

export interface StatsApi {
  query(req: StatsRequest): Promise<StatsResponse | { notModified: true }>;
  /** cheap; polled every 2 s while indexing */
  coverage(): Promise<Coverage>;
  /** the photos that match, as the handles the grid takes */
  openInGrid(scope: StatsScope, filter: StatsFilter): Promise<{ ids: string[] }>;
  /** the search cache changed (a rating edit, indexing finished); returns the unsubscribe */
  onDirty(cb: () => void): () => void;
  /** an image address for a photo's thumbnail (a blob or object URL), null when it has none */
  thumb(photo: PhotoRef, size: number): Promise<string | null>;
  /** pause or resume the background indexing */
  setPaused(paused: boolean): Promise<void>;
  /** the Places side panel: one map cell under the filter (its own place selection ignored) */
  place(key: string, filter: StatsFilter, scope: StatsScope): Promise<PlaceDetail>;
  /** the next page of top images from a cursor `query` or this gave; rejects with "stats-changed" when the index was rebuilt since */
  topPage(cursor: string): Promise<{ items: PhotoRef[]; next: string | null }>;
  /** a map tile (PNG bytes) through the app's cached OpenStreetMap command; rejects when offline */
  mapTile(z: number, x: number, y: number): Promise<ArrayBuffer>;
}

/** the one size the disk cache holds (the grid's THUMB_SIZE, the catalog thumbnail job's GRID_SIZE) */
const CACHED_THUMB_SIZE = 240;

export const commandsApi: StatsApi = {
  async query(req) { return normalizeReply(await commands.statsQuery({ yearMode: null, etag: null, ...req, places: req.places ? { bbox: null, heat: null, ...req.places } : null })); },
  coverage: () => commands.statsCoverage(),
  openInGrid: (scope, filter) => commands.statsOpenInGrid(scope, filter),
  onDirty(cb) {
    let off: (() => void) | undefined;
    let gone = false;
    void import("@tauri-apps/api/event").then(({ listen }) => listen("stats_dirty", cb)).then((un) => { if (gone) un(); else off = un; }).catch(() => {});
    return () => { gone = true; off?.(); };
  },
  async thumb(photo, _size) {
    // thumb_cached only answers for sizes the grid (and the catalog's thumbnail job) wrote
    // to the disk cache, which is GRID_SIZE: asking for a larger one always misses and
    // leaves every card an empty box. The cards scale the 240 px picture to their box.
    const buf = await commands.thumbCached(photo.path, 0, CACHED_THUMB_SIZE, false, false);
    return buf.byteLength ? URL.createObjectURL(new Blob([buf], { type: "image/jpeg" })) : null;
  },
  async setPaused(paused) { await commands.searchPause(paused); },
  async place(key, filter, scope) { return normalizePlace(await commands.statsPlace(key, filter, scope)); },
  topPage: (cursor) => commands.statsTopPage(cursor),
  mapTile: (z, x, y) => commands.mapTile(z, x, y),
};

export const StatsApiContext = createContext<StatsApi>(commandsApi);
export const useStatsApi = () => useContext(StatsApiContext);
