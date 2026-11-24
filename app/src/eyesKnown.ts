// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The closed-eye verdicts the page has so far, by photo path, readable without React (the folder filter, the glyphs and the
// Find scan all use them). A photo with no entry has not been looked at; null is "looked at, cannot tell".
import type { EyeLabel } from "./eyesQueue";

const known = new Map<string, EyeLabel | null>();
let version = 0;
const listeners = new Set<() => void>();

export const eyeVerdict = (path: string): EyeLabel | null | undefined => known.get(path);
export const eyesVersion = () => version;
export const subscribeEyes = (l: () => void) => { listeners.add(l); return () => { listeners.delete(l); }; };
export function setEyeVerdicts(list: readonly { path: string; state: EyeLabel | null }[]): string[] {
  const changed: string[] = [];
  for (const r of list) if (known.get(r.path) !== r.state || !known.has(r.path)) { known.set(r.path, r.state); changed.push(r.path); }
  if (changed.length) { version++; listeners.forEach((l) => l()); }
  return changed;
}
/** a photo whose primary face is closed or half closed (what Find's Closed eyes keeps) */
export const eyesClosed = (path: string) => { const v = known.get(path); return v === "closed" || v === "semi"; };
export function forgetEyes() { known.clear(); version++; listeners.forEach((l) => l()); }
