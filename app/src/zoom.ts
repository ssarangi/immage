// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Lightroom's navigator zoom levels. a number is image pixels per screen
// (device) pixel, so 1 is 100%
export type Zoom = "fit" | "fill" | number;
export const ZOOMS: [Zoom, string][] = [["fit", "FIT"], ["fill", "FILL"], [1, "1:1"], [2, "2:1"]];
// the levels ⌘= and ⌘- step through, as Lightroom's do past the navigator's,
// up to the wheel's 4:1
export const KEY_ZOOMS: Zoom[] = ["fit", "fill", 1, 2, 3, 4];

// the image as last rendered, kept to place the zoomed window and the
// navigator's rectangle
export type Placement = { fullWidth: number; fullHeight: number; scale: number; x: number; y: number;
                   width: number; height: number; upscale?: number };

// the part of the image the viewer shows, as fractions of the image. A viewer larger
// than the zoomed image shows all of it, so each edge is clamped to the image: clamping
// only the size left the rectangle shifted off the picture.
export function navRect(p: Placement): { x: number; y: number; w: number; h: number } {
  const fw = p.fullWidth * p.scale, fh = p.fullHeight * p.scale;
  const x0 = Math.min(1, Math.max(0, p.x / fw)), y0 = Math.min(1, Math.max(0, p.y / fh));
  const x1 = Math.min(1, Math.max(0, (p.x + p.width) / fw)), y1 = Math.min(1, Math.max(0, (p.y + p.height) / fh));
  return { x: x0, y: y0, w: Math.max(0, x1 - x0), h: Math.max(0, y1 - y0) };
}

// Shared per-frame easing and snap threshold for viewer and lightbox zoom.
export const ZOOM_EASE = 1 - 0.7 * 0.7;
export function easeZoom(value: number, target: number): number {
  const next = value + (target - value) * ZOOM_EASE;
  return Math.abs(target - next) < target * .002 ? target : next;
}
