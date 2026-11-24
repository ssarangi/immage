// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The face-box overlay's geometry and words, without React (notes/faces-ux-spec.md 3.2). Boxes arrive as fractions of
// the displayed picture (its orientation applied), so the overlay is a layer exactly as big as the picture and a box
// is a percentage of it: zoom and pan move the layer, never the boxes' meaning.
import type { PhotoFace } from "./peopleStore";

const clamp01 = (n: number) => Math.min(1, Math.max(0, Number.isFinite(n) ? n : 0));
const MIN = 0.006;   // a box never vanishes: 0.6% of the picture

export type BoxStyle = { left: string; top: string; width: string; height: string };
const pct = (n: number) => `${Math.round(n * 10000) / 100}%`;

// the box kept inside the picture, however the detector rounded it
export function boxStyle(f: Pick<PhotoFace, "x" | "y" | "w" | "h">): BoxStyle {
  const x = clamp01(f.x), y = clamp01(f.y);
  const w = Math.min(1 - x, Math.max(MIN, clamp01(f.w))), h = Math.min(1 - y, Math.max(MIN, clamp01(f.h)));
  return { left: pct(x), top: pct(y), width: pct(w), height: pct(h) };
}

// left to right (then top to bottom), the order of the tab stops and of "Face 2 of 4"
export const orderFaces = (list: readonly PhotoFace[]): PhotoFace[] =>
  [...list].filter((f) => f.state !== "rejected").sort((a, b) => a.x - b.x || a.y - b.y || a.face - b.face);

// what a face is called: the person, "Maybe Maya" for a suggestion, or nobody yet
export function faceTitle(f: Pick<PhotoFace, "name" | "state">, tr: (k: string, v?: Record<string, string | number>) => string): string {
  if (f.state === "confirmed" && f.name) return f.name;
  if (f.state === "suggested" && f.name) return tr("people.suggest.maybe", { name: f.name });
  return tr("people.face.unnamed");
}

export const faceAria = (f: Pick<PhotoFace, "name" | "state">, i: number, n: number, tr: (k: string, v?: Record<string, string | number>) => string) =>
  tr("people.face.aria", { i: i + 1, n, name: faceTitle(f, tr) });

// the name sits above the box, and below it when the box is at the top edge
export const labelSide = (f: Pick<PhotoFace, "y">): "above" | "below" => (f.y < 0.08 ? "below" : "above");

// the face under a point (fractions of the picture); the smallest box wins, so a face inside a bigger one is reachable
export function faceAt(list: readonly PhotoFace[], x: number, y: number): PhotoFace | null {
  let best: PhotoFace | null = null;
  for (const f of list) {
    if (x < f.x || x > f.x + f.w || y < f.y || y > f.y + f.h) continue;
    if (!best || f.w * f.h < best.w * best.h) best = f;
  }
  return best;
}

// a face as faces_photo_faces answers (fractions of the displayed picture, any of them null when the detector kept no
// box), as the overlay draws it; a face without a box cannot be drawn and is left out
export type BackendFace = {
  face: number; x: number | null; y: number | null; w: number | null; h: number | null; person: number | null; name: string | null;
  state: string; detector_confidence: number | null;
};
export function fromBackend(list: readonly BackendFace[]): PhotoFace[] {
  const out: PhotoFace[] = [];
  for (const f of list) {
    if (f.x == null || f.y == null || f.w == null || f.h == null || !(f.w > 0) || !(f.h > 0)) continue;
    const state = f.state === "confirmed" || f.state === "suggested" || f.state === "rejected" ? f.state : "unassigned";
    out.push({ face: f.face, x: f.x, y: f.y, w: f.w, h: f.h, person: f.person, name: f.name, state, confidence: f.detector_confidence ?? 0 });
  }
  return out;
}
