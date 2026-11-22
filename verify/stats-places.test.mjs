// SPDX-License-Identifier: GPL-3.0-or-later
// The Statistics Places map's arithmetic (app/src/stats/placesGeometry.ts), which imports only types, so it
// loads as it is: quadkeys to bounds and centroids, Mercator pixels, bubble sizes and the merge of overlapping
// bubbles at a zoom, the offline graticule, which viewport changes need a request, and the debounce.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
const require = createRequire(new URL('../app/package.json', import.meta.url));
const { transformSync } = await import(require.resolve('rolldown/experimental'));
const path = new URL('../app/src/stats/placesGeometry.ts', import.meta.url).pathname;
const code = transformSync(path, readFileSync(path, 'utf8')).code;
const G = await import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);
const near = (a, b, eps = 1e-6) => assert.ok(Math.abs(a - b) < eps, `${a} is not ${b}`);

test('a quadkey is a tile: parse, round trip and rejection', () => {
  assert.deepEqual(G.parseKey('213'), { level: 3, x: 3, y: 5 }); // 2 -> y, 1 -> x, 3 -> both
  assert.deepEqual(G.parseKey(''), { level: 0, x: 0, y: 0 });
  assert.equal(G.parseKey('12a'), null);
  assert.equal(G.parseKey('4'), null);
  assert.equal(G.parseKey('1'.repeat(23)), null);
  for (const key of ['0', '3', '021', '3120312', '0123012301230123']) {
    const t = G.parseKey(key);
    assert.equal(G.keyOf(t.level, t.x, t.y), key);
  }
});

test('a tile covers its box and the boxes of its children tile it', () => {
  assert.deepEqual(G.tileBounds('').map(Math.round), [-180, -85, 180, 85]);
  const [w, s, e, n] = G.tileBounds('1'); // the north-east quarter
  near(w, 0); near(e, 180); near(s, 0); near(n, G.MAX_LAT, 1e-4);
  const sw = G.tileBounds('2');
  near(sw[0], -180); near(sw[2], 0); near(sw[3], 0); near(sw[1], -G.MAX_LAT, 1e-4);
  // the four children of a tile share its edges
  const parent = G.tileBounds('12');
  const kids = ['120', '121', '122', '123'].map(G.tileBounds);
  near(Math.min(...kids.map((k) => k[0])), parent[0]); near(Math.max(...kids.map((k) => k[2])), parent[2]);
  near(Math.min(...kids.map((k) => k[1])), parent[1]); near(Math.max(...kids.map((k) => k[3])), parent[3]);
  assert.equal(G.tileBounds('x'), null);
});

test('the centroid of a tile is inside it; a cell is inside the area it starts with', () => {
  for (const key of ['0', '1203', '3333']) {
    const [lat, lng] = G.tileCentroid(key);
    const [w, s, e, n] = G.tileBounds(key);
    assert.ok(lng > w && lng < e && lat > s && lat < n);
  }
  assert.ok(G.inside('1203', '12'));
  assert.ok(G.inside('12', '12'));
  assert.ok(!G.inside('12', '1203'));
  assert.ok(!G.inside('13', '12'));
});

test('a key round trips through its centroid: the pixel of a centroid lies in the tile at that zoom', () => {
  const key = '12031';
  const [lat, lng] = G.tileCentroid(key);
  const p = G.project(lat, lng, key.length);
  const t = G.parseKey(key);
  assert.equal(Math.floor(p.x / 256), t.x);
  assert.equal(Math.floor(p.y / 256), t.y);
  assert.deepEqual(G.project(0, 0, 0), { x: 128, y: 128 });
  near(G.project(0, -180, 3).x, 0); near(G.project(0, 180, 3).x, 256 * 8);
  near(G.project(G.MAX_LAT, 0, 3).y, 0, 1e-3);
  assert.ok(G.project(100, 0, 2).y >= 0); // clamped, not off the world
});

test('bubble sizes grow with the count relative to the biggest cell, with the area following it, and stop at the cap', () => {
  assert.ok(G.bubbleRadius(1) < G.bubbleRadius(100));
  assert.ok(G.bubbleRadius(100) < G.bubbleRadius(1000));
  assert.equal(G.bubbleRadius(1e9), G.RADIUS_CAP);
  assert.equal(G.bubbleRadius(0), G.RADIUS_MIN);
  // the same count is a bigger bubble where it is the biggest cell, and the area (radius minus the minimum, squared) follows the count
  assert.ok(G.bubbleRadius(500, 600) > G.bubbleRadius(500, 50000));
  const a = (v) => (G.bubbleRadius(v, 10000) - G.RADIUS_MIN) ** 2;
  near(a(2500) / a(10000), 0.25, 1e-9);
  assert.deepEqual([999, 1000, 1204, 12340, 1_230_000].map(G.compact), ['999', '1k', '1.2k', '12k', '1.2M']);
});

const cell = (key, lat, lng, n, top = 0, solo) => ({ key, lat, lng, n, top, solo });
const px = (lat, lng) => G.project(lat, lng, 5);

test('overlapping cells merge into the biggest, keep the sum and move to the weighted centre', () => {
  const cells = [cell('a', 47.6, -122.3, 900, 90), cell('b', 47.61, -122.31, 100, 10), cell('c', 47.62, -122.29, 1, 1, { id: '1' }), cell('far', -33.9, 151.2, 40, 4)];
  const out = G.bubbles(cells, px);
  assert.equal(out.length, 2);
  const seattle = out.find((b) => b.keys.includes('a'));
  assert.equal(seattle.key, 'a');
  assert.equal(seattle.n, 1001);
  assert.equal(seattle.top, 101);
  assert.equal(seattle.merged, true);
  assert.equal(seattle.solo, undefined);
  assert.deepEqual(seattle.keys.sort(), ['a', 'b', 'c']);
  assert.ok(seattle.lat > 47.6 && seattle.lat < 47.61, 'the centre moves toward the heavier member');
  assert.ok(seattle.box[1] === 47.6 && seattle.box[3] === 47.62);
  assert.equal(out.reduce((a, b) => a + b.n, 0), 1041);
});

test('zooming in separates what zooming out merged', () => {
  const cells = [cell('a', 47.60, -122.30, 500), cell('b', 47.64, -122.36, 300)];
  assert.equal(G.bubbles(cells, (la, ln) => G.project(la, ln, 6)).length, 1);
  assert.equal(G.bubbles(cells, (la, ln) => G.project(la, ln, 13)).length, 2);
});

test('a lone photo stays a small dot with its photo, and the order does not matter', () => {
  const cells = [cell('x', 10, 10, 1, 0, { id: '7' }), cell('y', -40, 100, 300, 30)];
  const a = G.bubbles(cells, px), b = G.bubbles([...cells].reverse(), px);
  assert.deepEqual(a.map((v) => v.key), b.map((v) => v.key));
  const dot = a.find((v) => v.key === 'x');
  assert.equal(dot.r, G.SOLO_RADIUS);
  assert.deepEqual(dot.solo, { id: '7' });
  assert.equal(dot.merged, false);
});

test('top mode sizes by top photos and leaves out cells with none', () => {
  const cells = [cell('a', 0, 0, 1000, 0), cell('b', 20, 20, 50, 40)];
  const out = G.bubbles(cells, px, 'top');
  assert.deepEqual(out.map((b) => b.key), ['b']);
  assert.equal(out[0].r, G.bubbleRadius(40, 40));
});

test('many cells in one place still give a bounded number of bubbles', () => {
  const cells = [];
  for (let i = 0; i < 1500; i++) cells.push(cell(`k${i}`, 40 + (i % 40) * 0.01, -74 + Math.floor(i / 40) * 0.01, 1 + (i % 7)));
  const out = G.bubbles(cells, (la, ln) => G.project(la, ln, 7));
  assert.ok(out.length < 60, `${out.length} bubbles`);
  assert.equal(out.reduce((a, b) => a + b.n, 0), cells.reduce((a, c) => a + c.n, 0));
});

test('the graticule picks 4 to 8 lines on round degrees', () => {
  assert.equal(G.graticuleStep(360), 90);
  assert.equal(G.graticuleStep(40), 10);
  assert.equal(G.graticuleStep(2), 0.5);
  const g = G.graticule([-10, 35, 30, 60]);
  assert.equal(g.step, 10);
  assert.deepEqual(g.lngs, [-10, 0, 10, 20, 30]);
  assert.deepEqual(g.lats, [40, 50, 60]);
  assert.ok(G.graticule([-180, -90, 180, 90]).lats.every((v) => Math.abs(v) <= G.MAX_LAT));
  assert.equal(G.degrees(38.72, 'lat', 0.1), '38.7°N');
  assert.equal(G.degrees(-9, 'lng'), '9°W');
  assert.equal(G.degrees(0, 'lat'), '0°');
  assert.equal(G.coords(38.7223, -9.1393), '38.72° N, 9.14° W');
});

test('a viewport asks again for another zoom or when it leaves the box and margin it was answered for', () => {
  const asked = { zoom: 5, bbox: [-10, 30, 10, 50] };
  assert.equal(G.needsRequest(null, asked), true);
  assert.equal(G.needsRequest(asked, asked), false);
  assert.equal(G.needsRequest(asked, { zoom: 6, bbox: [-1, 35, 1, 45] }), true);
  // a small pan: still inside the box plus 10 %
  assert.equal(G.needsRequest(asked, { zoom: 5, bbox: [-9, 31, 11, 51] }), false);
  // a big pan: past the margin
  assert.equal(G.needsRequest(asked, { zoom: 5, bbox: [5, 30, 25, 50] }), true);
  // the whole world covers every view at its zoom, a box does not cover the world
  assert.equal(G.needsRequest({ zoom: 3, bbox: null }, { zoom: 3, bbox: [100, 0, 120, 10] }), false);
  assert.equal(G.needsRequest(asked, { zoom: 5, bbox: null }), true);
});

test('bboxes are clamped to the world and rounded', () => {
  assert.deepEqual(G.clampBbox([-200, -90, 190, 91]), [-180, -G.MAX_LAT, 180, G.MAX_LAT]);
  assert.deepEqual(G.roundBbox([1.234567, 2.345678, 3.456789, 4.567891]), [1.2346, 2.3457, 3.4568, 4.5679]);
});

test('zoomFor fits a box in a pixel size', () => {
  const z = G.zoomFor([-124, 45, -122, 49], 800, 500);
  const a = G.project(49, -124, z), b = G.project(45, -122, z);
  assert.ok(b.x - a.x <= 800 && b.y - a.y <= 500);
  assert.equal(G.zoomFor([-180, -80, 180, 80], 100, 100), G.MIN_ZOOM);
  assert.equal(G.zoomFor([1, 1, 1, 1], 800, 500), G.MAX_ZOOM);
});

function fakeTimers() {
  let now = 0, next = 1;
  const jobs = new Map();
  return {
    set: (fn, ms) => { const id = next++; jobs.set(id, { fn, at: now + ms }); return id; },
    clear: (id) => { jobs.delete(id); },
    tick(ms) { now += ms; for (const [id, j] of [...jobs]) if (j.at <= now) { jobs.delete(id); j.fn(); } },
    get pending() { return jobs.size; },
  };
}

test('the debounce sends only the last value after the quiet time, and can be flushed or cancelled', () => {
  const t = fakeTimers();
  const got = [];
  const d = G.debounce((v) => got.push(v), 250, t);
  d.call(1); t.tick(100); d.call(2); t.tick(100); d.call(3);
  assert.equal(d.waiting, true);
  t.tick(249);
  assert.deepEqual(got, []);
  t.tick(1);
  assert.deepEqual(got, [3]);
  assert.equal(d.waiting, false);
  d.call(4); d.flush();
  assert.deepEqual(got, [3, 4]);
  assert.equal(t.pending, 0);
  d.call(5); d.cancel(); t.tick(1000);
  assert.deepEqual(got, [3, 4]);
  d.flush(); // nothing waiting: nothing sent
  assert.deepEqual(got, [3, 4]);
});

test('an arrow key goes to the nearest bubble in its direction, and nowhere at the edge', () => {
  const at = (x, y) => ({ x, y });
  const c = at(100, 100), r = at(180, 105), l = at(20, 98), u = at(104, 30), d = at(95, 190), diag = at(160, 160);
  const all = [c, r, l, u, d, diag];
  assert.equal(G.neighbour(all, c, 'right'), r);
  assert.equal(G.neighbour(all, c, 'left'), l);
  assert.equal(G.neighbour(all, c, 'up'), u);
  assert.equal(G.neighbour(all, c, 'down'), d);
  assert.equal(G.neighbour(all, r, 'right'), null);
  assert.equal(G.neighbour(all, u, 'up'), null);
  assert.equal(G.neighbour([c], c, 'down'), null);
});

test('a place chip says where the tile is centred', () => {
  assert.match(G.placeText('0'), /^\d+\.\d° N, \d+\.\d° W$/);
  const [lat, lng] = G.tileCentroid('1203');
  assert.equal(G.placeText('1203'), G.coords(lat, lng, 1));
  assert.equal(G.placeText('x'), 'x');
});
