// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { createContext, useContext } from "react";
import type { StatsState } from "../statsModel";
import type { PhotoRef, StatsFilter, StatsResponse } from "../statsTypes";

/** What every section of the view shares: the selection, the numbers and the ways out. */
export interface ViewCtx {
  state: StatsState;
  update(fn: (s: StatsState) => StatsState): void;
  response: StatsResponse | null;
  /** no answer yet: skeletons */
  loading: boolean;
  /** an answer is on show while a newer one is on the way: dimmed */
  stale: boolean;
  error: string | null;
  retry(): void;
  /** a photo card: open the photo (and its neighbours in the list) */
  openPhoto(photo: PhotoRef, list: PhotoRef[]): void;
  /** the grid, with the page's filter, or with this part of it replaced (the legend's Grid button) */
  openGrid(override?: Partial<StatsFilter>, title?: string): void;
  scrollTo(section: string): void;
  /** whether a section is on the page yet (the later phases add theirs) */
  has(section: string): boolean;
}
export const ViewContext = createContext<ViewCtx | null>(null);
export const useView = (): ViewCtx => {
  const v = useContext(ViewContext);
  if (!v) throw new Error("Statistics section outside its view");
  return v;
};
