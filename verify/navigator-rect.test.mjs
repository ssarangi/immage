// SPDX-License-Identifier: GPL-3.0-or-later
import test from 'node:test';
import assert from 'node:assert/strict';
import { navRect } from '../app/src/zoom.ts';

const pl = (o) => ({ fullWidth: 800, fullHeight: 600, scale: 1, x: 0, y: 0, width: 400, height: 300, ...o });
const close = (a, b) => assert.ok(Math.abs(a - b) < 1e-9, `${a} vs ${b}`);

test('a zoomed window inside the image maps to its fractions', () => {
  const r = navRect(pl({ x: 200, y: 150 }));
  close(r.x, .25); close(r.y, .25); close(r.w, .5); close(r.h, .5);
});

test('a viewer larger than the image shows all of it, not a shifted box', () => {
  // 800x600 at 1:1 in a 1700x1000 viewer: the window starts at -450, -200
  const r = navRect(pl({ x: -450, y: -200, width: 1700, height: 1000 }));
  assert.deepEqual(r, { x: 0, y: 0, w: 1, h: 1 });
});

test('a window hanging off one edge is cut at the image edge', () => {
  const r = navRect(pl({ x: 600, y: 0, width: 400, height: 300 })); // 200 px past the right edge
  close(r.x, .75); close(r.w, .25); close(r.y, 0); close(r.h, .5);
});

test('the rectangle never leaves the picture', () => {
  for (const x of [-900, -100, 0, 300, 700, 1500]) for (const w of [100, 800, 2000]) {
    const r = navRect(pl({ x, y: x, width: w, height: w }));
    assert.ok(r.x >= 0 && r.y >= 0 && r.x + r.w <= 1 + 1e-9 && r.y + r.h <= 1 + 1e-9 && r.w >= 0 && r.h >= 0);
  }
});
