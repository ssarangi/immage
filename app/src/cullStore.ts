// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The cull under review, shared by App (which runs it), the Cull panel, the
// grid's thumbnails and the lightbox's inspector. A small external store so a
// thumbnail re-renders only when its own decision changes.
import { useSyncExternalStore } from "react";
import type { CullDecision, CullPlan } from "./bindings";
import { byKey, decisionKey, type CullChip } from "./cullModel";

export type CullState = {
  plan: CullPlan | null;
  decisions: Map<string, CullDecision>;
  chip: CullChip;
  open: boolean;      // the Cull panel takes Find's place in the grid
  dialog: boolean;    // the setup dialog
};

let state: CullState = { plan: null, decisions: new Map(), chip: "all", open: false, dialog: false };
const listeners = new Set<() => void>();
export const cullState = () => state;
export function setCull(patch: Partial<CullState>) {
  state = { ...state, ...patch };
  listeners.forEach((l) => l());
}
const subscribe = (l: () => void) => { listeners.add(l); return () => { listeners.delete(l); }; };

// a plan to review, or none (closing the review)
export function showPlan(plan: CullPlan | null) {
  setCull({ plan, decisions: byKey(plan), chip: "all", open: plan !== null });
}
// the same session with a decision or two changed (an override, an apply)
export function updatePlan(plan: CullPlan) {
  setCull({ plan, decisions: byKey(plan) });
}

export const useCull = () => useSyncExternalStore(subscribe, cullState);
export const useDecision = (path: string, version?: number) =>
  useSyncExternalStore(subscribe, () => state.decisions.get(decisionKey(path, version)));
// the grid and lightbox review only while the panel is open
export const useReviewing = () => useSyncExternalStore(subscribe, () => state.open && state.plan !== null);
