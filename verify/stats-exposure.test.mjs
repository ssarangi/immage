// SPDX-License-Identifier: GPL-3.0-or-later
// The Exposure and Time charts' arithmetic (app/src/statsExposure.ts): iris geometry, bar heights, dayparts, the polar
// clock's wedges and the client-side insights. The file imports nothing, so it loads as it is. No engine, no browser.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
const require = createRequire(new URL('../app/package.json', import.meta.url));
const { transformSync } = await import(require.resolve('rolldown/experimental'));
const path = new URL('../app/src/statsExposure.ts', import.meta.url).pathname;
const X = await import(`data:text/javascript;base64,${Buffer.from(transformSync(path, readFileSync(path, 'utf8')).code).toString('base64')}`);

test('the iris hole shrinks as the f-number grows and never exceeds the blade ring', () => {
  const radii = ['a1', 'a2', 'a3', 'a4', 'a5'].map((id) => X.irisRadius(X.IRIS_F[id]));
  for (let i = 1; i < radii.length; i++) assert.ok(radii[i] < radii[i - 1]);
  assert.ok(radii[0] <= 34 + 1e-9 && radii[4] > 5);
  assert.equal(X.irisRadius(0.9), 34, 'faster than f/1.6 is clamped to the ring');
});

test('a regular polygon: seven points, the first straight up, all on the circle', () => {
  const pts = X.polygonPoints(7, 38, 60, 60).split(' ').map((p) => p.split(',').map(Number));
  assert.equal(pts.length, 7);
  assert.deepEqual(pts[0], [60, 22]);
  for (const [x, y] of pts) assert.ok(Math.abs(Math.hypot(x - 60, y - 60) - 38) < 0.02);
});

test('aperture labels get an en dash between numbers only', () => {
  assert.equal(X.dashed('f/1.2-1.8'), 'f/1.2–1.8');
  assert.equal(X.dashed('f/16+'), 'f/16+');
  assert.equal(X.dashed('f/8-11'), 'f/8–11');
});

test('bar heights: scaled to the largest, a non-empty bar keeps a visible sliver, an empty one none', () => {
  assert.deepEqual(X.barHeights([100, 50, 0, 1], 200), [200, 100, 0, 3]);
  assert.deepEqual(X.barHeights([0, 0], 200), [0, 0]);
  assert.deepEqual(X.barHeights([], 200), []);
});

test('dayparts by fixed hours: night 21-04, early 05-08, day 09-15, late 16-20', () => {
  const got = Array.from({ length: 24 }, (_, h) => X.daypart(h));
  assert.deepEqual(got.slice(0, 5), Array(5).fill('night'));
  assert.deepEqual(got.slice(5, 9), Array(4).fill('early'));
  assert.deepEqual(got.slice(9, 16), Array(7).fill('day'));
  assert.deepEqual(got.slice(16, 21), Array(5).fill('late'));
  assert.deepEqual(got.slice(21), Array(3).fill('night'));
  assert.equal(X.DAYPART_ACCENT.early, 'amber');
  assert.equal(X.DAYPART_ACCENT.late, 'amber');
  assert.equal(X.DAYPART_ACCENT.night, 'violet');
});

test('polar clock: hour 0 is at 12 o\'clock, hour 6 at 3, wedges are 13 degrees wide', () => {
  assert.deepEqual(X.hourPoint(0, 100, 190, 190), { x: 190, y: 90 });
  assert.deepEqual(X.hourPoint(6, 100, 190, 190), { x: 290, y: 190 });
  assert.deepEqual(X.hourPoint(12, 100, 190, 190), { x: 190, y: 290 });
  assert.deepEqual(X.hourPoint(18, 100, 190, 190), { x: 90, y: 190 });
  const d = X.wedgePath(0, 58, 128, 190, 190);
  const nums = [...d.matchAll(/[ML] ?(-?[\d.]+) (-?[\d.]+)/g)].map((m) => [Number(m[1]), Number(m[2])]);
  // M inner-left, L outer-left, (arc to) outer-right comes from the A command, then L inner-right
  const [inL, outL, inR] = [nums[0], nums[1], nums[2]];
  const ang = (p) => (Math.atan2(p[1] - 190, p[0] - 190) * 180) / Math.PI;
  assert.ok(Math.abs(ang(inL) - (-90 - 6.5)) < 0.2 && Math.abs(ang(outL) - (-90 - 6.5)) < 0.2);
  assert.ok(Math.abs(ang(inR) - (-90 + 6.5)) < 0.2);
  assert.ok(Math.abs(Math.hypot(outL[0] - 190, outL[1] - 190) - 128) < 0.2);
  assert.ok(d.endsWith('Z'));
});

test('wedge radius: the busiest hour reaches the rim, an empty hour is the inner ring, a tiny one a stub', () => {
  assert.equal(X.wedgeRadius(10, 10, 58, 128), 128);
  assert.equal(X.wedgeRadius(0, 10, 58, 128), 58);
  assert.equal(X.wedgeRadius(5, 10, 58, 128), 93);
  assert.ok(X.wedgeRadius(1, 10000, 58, 128) >= 60);
  assert.equal(X.wedgeRadius(3, 0, 58, 128), 58);
});

test('peak index: the first of equal peaks, -1 when nothing', () => {
  assert.equal(X.peakIndex([1, 5, 5, 2]), 1);
  assert.equal(X.peakIndex([0, 0]), -1);
  assert.equal(X.peakIndex([]), -1);
});

test('light share counts only the early and late hours', () => {
  const hours = Array.from({ length: 24 }, (_, h) => ({ n: h === 6 || h === 18 ? 50 : h === 12 ? 100 : 0 }));
  assert.equal(X.lightShare(hours), 0.5);
  assert.equal(X.lightShare([]), 0);
});

test('aperture insight: the most-used group, and a better-keeping group among those with enough frames', () => {
  const groups = [{ id: 'a1', n: 50, top: 50 }, { id: 'a2', n: 1000, top: 112 }, { id: 'a3', n: 3300, top: 165 }, { id: 'a4', n: 1000, top: 40 }, { id: 'a5', n: 100, top: 1 }];
  const r = X.apertureInsight(groups);
  assert.equal(r.most, 'a3');
  assert.ok(Math.abs(r.mostShare - 3300 / 5450) < 1e-9);
  assert.equal(r.best, 'a2', 'a1 keeps 100% but has fewer than 100 frames');
  assert.ok(Math.abs(r.bestRate - 0.112) < 1e-9);
  assert.equal(X.apertureInsight([{ id: 'a1', n: 0, top: 0 }]), null);
  assert.equal(X.apertureInsight(groups.map((g) => ({ ...g, top: 0 }))).best, null, 'no keepers anywhere, nothing to praise');
});

test('shutter insight: the biggest regime and the share of exposures over a second', () => {
  const r = X.shutterInsight([{ id: 'freeze', n: 10 }, { id: 'handheld', n: 70 }, { id: 'long', n: 20 }]);
  assert.deepEqual(r, { id: 'handheld', share: 0.7, longShare: 0.2 });
  assert.equal(X.shutterInsight([{ id: 'freeze', n: 0 }]), null);
});

test('high ISO share is 3200 and up', () => {
  assert.equal(X.highIsoShare([{ id: '100', n: 50 }, { id: '3200', n: 25 }, { id: '6400+', n: 25 }]), 0.5);
  assert.equal(X.highIsoShare([]), 0);
});

test('month insight: the busiest month against the average month, with its top-image rate', () => {
  const months = Array.from({ length: 12 }, (_, i) => ({ n: i === 9 ? 6000 : 3000, top: i === 9 ? 600 : 100 }));
  const r = X.monthInsight(months);
  assert.equal(r.month, 10);
  assert.equal(r.n, 6000);
  assert.ok(Math.abs(r.ratio - 6000 / (39000 / 12)) < 1e-9);
  assert.equal(r.topRate, 0.1);
  assert.equal(X.monthInsight(months.map(() => ({ n: 0, top: 0 }))), null);
});
