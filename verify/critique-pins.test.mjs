// SPDX-License-Identifier: GPL-3.0-or-later
// placePins (app/src/critiquePins.ts): fixed-size badges inside the photo, nudged apart.
import test from 'node:test';
import assert from 'node:assert/strict';
import { PIN, PIN_GAP, placeLabel, placePins } from '../app/src/critiquePins.ts';

const W = 1000, H = 560;
const apart = (ps) => ps.every((p, i) => ps.every((q, j) => i === j || Math.hypot(p.left - q.left, p.top - q.top) >= PIN_GAP - 0.01));

test('a badge sits 8px inside its region corner', () => {
  const [p] = placePins([[.2, .3, .3, .3]], W, H);
  assert.deepEqual(p, { left: 208, top: 176 });
});

test('badges stay inside the photo', () => {
  for (const p of placePins([[0, 0, .1, .1], [.99, .99, .01, .01], [1, 1, 0, 0]], W, H)) {
    assert.ok(p.left >= 0 && p.left + PIN <= W && p.top >= 0 && p.top + PIN <= H, JSON.stringify(p));
  }
});

test('badges closer than the gap are nudged apart, the smaller region moving', () => {
  const boxes = [[.1, .1, .5, .5], [.1, .1, .2, .2], [.1, .1, .1, .1]];
  const ps = placePins(boxes, W, H);
  assert.ok(apart(ps), JSON.stringify(ps));
  assert.deepEqual(ps[0], { left: 108, top: 64 }, 'the largest region keeps its corner');
});

test('without a size there is nothing to place, and results follow the input order', () => {
  assert.deepEqual(placePins([], W, H), []);
  const ps = placePins([[.5, .5, .1, .1], [.1, .1, .5, .5]], W, H);
  assert.equal(ps[1].left, 108);
});

test('the label goes right of its badge, or below it near the right edge', () => {
  const near = placeLabel({ left: 100, top: 50 }, 'Stone pathway', W);
  assert.deepEqual(near, { left: 100 + PIN + 6, top: 50 });
  const far = placeLabel({ left: 960, top: 50 }, 'Foreground heather', W);
  assert.equal(far.top, 50 + PIN + 6);
  assert.ok(far.left + 18 * 7 + 20 <= W);
});
