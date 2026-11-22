// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Where the critique's numbered badges and their label chips sit over the
// photo. Each badge is a fixed square anchored just inside its region's top-left
// corner; a badge that would land within PIN_GAP of one already placed is
// nudged along the diagonal, and every badge stays inside the photo.
export const PIN = 22;          // the badge's side, px
export const PIN_INSET = 8;     // from the region's corner
export const PIN_GAP = 26;      // the nearest two badges may sit
export const PIN_NUDGE = 24;
const MARGIN = 4;

export type Box = [number, number, number, number];   // x, y, w, h, each 0 to 1
export type Placed = { left: number; top: number };

const area = (b: Box) => (b[2] ?? 0) * (b[3] ?? 0);

// the badges' pixel positions, in the order of `boxes`; the larger regions are
// placed first so a small region's badge is the one that moves
export function placePins(boxes: Box[], width: number, height: number): Placed[] {
  const out: Placed[] = new Array(boxes.length);
  const clampX = (v: number) => Math.max(MARGIN, Math.min(v, Math.max(MARGIN, width - PIN - MARGIN)));
  const clampY = (v: number) => Math.max(MARGIN, Math.min(v, Math.max(MARGIN, height - PIN - MARGIN)));
  const placed: Placed[] = [];
  const order = boxes.map((_, i) => i).sort((a, b) => area(boxes[b]) - area(boxes[a]) || a - b);
  for (const i of order) {
    const [x, y] = boxes[i];
    let left = clampX(x * width + PIN_INSET), top = clampY(y * height + PIN_INSET);
    // along the diagonal, away from the corner, a step at a time
    for (let step = 1; step <= 6 && placed.some((p) => Math.hypot(p.left - left, p.top - top) < PIN_GAP); step++) {
      left = clampX(x * width + PIN_INSET + PIN_NUDGE * step); top = clampY(y * height + PIN_INSET + PIN_NUDGE * step);
    }
    out[i] = { left, top };
    placed.push(out[i]);
  }
  return out;
}

// the label chip: right of its badge, or below it when it would run off the
// photo's right edge (the chip is about 7px a letter)
export function placeLabel(pin: Placed, label: string, width: number): Placed {
  const chip = label.length * 7 + 20;
  if (pin.left + PIN + 6 + chip <= width - MARGIN) return { left: pin.left + PIN + 6, top: pin.top };
  return { left: Math.max(MARGIN, Math.min(pin.left, width - chip - MARGIN)), top: pin.top + PIN + 6 };
}
