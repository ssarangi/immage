// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// "Open N in grid": the handles stats_open_in_grid returned become the grid's entries through
// stats_grid_entries, the rows local search shows. Opening one goes the normal photo-open path (the
// handle is its imgid), so edits land in the photo's sidecar like any other photo's.
import type { LibraryImage } from "./bindings";
import type { ImageEntry } from "./library";

/** the most photos one result holds (stats_open_in_grid and stats_grid_entries both stop here) */
export const GRID_CAP = 20_000;

/** entries in the order of `ids`; a handle the backend no longer knows is skipped, and counted in `missing` */
export async function gridEntries(ids: string[], fetch: (ids: string[]) => Promise<LibraryImage[]>): Promise<{ entries: ImageEntry[]; missing: number }> {
  const wanted = ids.slice(0, GRID_CAP);
  if (!wanted.length) return { entries: [], missing: 0 };
  const rows = await fetch(wanted);
  return { entries: rows.map((r) => ({ ...r } satisfies ImageEntry)), missing: wanted.length - rows.length };
}
