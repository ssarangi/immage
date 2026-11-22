// SPDX-License-Identifier: GPL-3.0-or-later
// Degenerate libraries (app/harness/statsDegenerate.ts): the arithmetic behind every Statistics chart must give
// finite numbers, no division by zero and no insight sentence from nothing, for a library of one photo, one camera,
// no lens data, no GPS, no ratings, all five stars and a single year. Pure files only: no engine, no browser.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
const require = createRequire(new URL('../app/package.json', import.meta.url));
const { transformSync } = await import(require.resolve('rolldown/experimental'));
const load = async (rel) => {
  const path = new URL(`../app/${rel}`, import.meta.url).pathname;
  return import(`data:text/javascript;base64,${Buffer.from(transformSync(path, readFileSync(path, 'utf8')).code).toString('base64')}`);
};
const A = await load('src/statsMath.ts');
const X = await load('src/statsExposure.ts');
const G = await load('harness/statsDegenerate.ts');

const finite = (v, path = '') => {
  if (typeof v === 'number') assert.ok(Number.isFinite(v), `${path} is ${v}`);
  else if (Array.isArray(v)) v.forEach((x, i) => finite(x, `${path}[${i}]`));
  else if (v && typeof v === 'object') for (const k of Object.keys(v)) finite(v[k], `${path}.${k}`);
};

test('nothing divides by zero: empty and all-zero inputs give 0 or null, never NaN', () => {
  assert.equal(A.share(5, 0), 0);
  assert.equal(A.keeperRate({ top: 0, n: 0 }), null);
  assert.deepEqual(A.scaleBars([0, 0, 0]), [0, 0, 0]);
  assert.deepEqual(A.scaleBars([]), []);
  finite(A.barWidths([{ n: 0, top: 0 }, { n: 0, top: 0 }], 'keeper'));
  finite(A.barWidths([], 'most-used'));
  finite(A.donutArcs([], 100));
  finite(A.donutArcs([{ id: 'a', n: 0 }], 100));
  finite(A.fanWedges([], 0, 0));
  assert.deepEqual(A.foldBins([], 8), []);
  assert.deepEqual(A.eraChips([], 'that-year'), []);
  assert.equal(A.groupMakers([], 'Other').length, 0);
  assert.equal(A.keeperBodyInsight([], 100), null);
  assert.equal(A.keeperBodyInsight([{ key: 'a|b', maker: 'a', model: 'b', n: 200, top: 0 }], 100), null);
});

test('a single year is one era chip and one run', () => {
  const chips = A.eraChips([{ year: 2024, n: 7, cumulative: 7 }], 'cumulative');
  assert.equal(chips.length, 1);
  assert.deepEqual(chips[0].years, [2024]);
  assert.deepEqual(A.yearRuns([2024]), [[2024, 2024]]);
});

test('no insight sentence comes from nothing or from a handful of frames', () => {
  const zeros = Array.from({ length: 12 }, () => ({ n: 0, top: 0 }));
  assert.equal(X.monthInsight(zeros), null);
  const one = zeros.map((m, i) => (i === 9 ? { n: 1, top: 1 } : m));
  assert.equal(X.monthInsight(one), null, 'one photo is not a "1.0x the average month" story');
  const many = zeros.map((m, i) => ({ n: i === 9 ? 60 : 10, top: i === 9 ? 6 : 1 }));
  assert.equal(X.monthInsight(many).month, 10);
  assert.equal(X.apertureInsight([{ id: 'a1', n: 0, top: 0 }], 100), null);
  const unrated = X.apertureInsight([{ id: 'a1', n: 500, top: 0 }, { id: 'a2', n: 300, top: 0 }], 100);
  assert.equal(unrated.best, null);
  assert.equal(unrated.bestRate, 0, 'the sentence about the best keeper is left out when nothing is a keeper');
  assert.equal(X.shutterInsight([{ id: 'fast', n: 0 }]), null);
  assert.equal(X.highIsoShare([]), 0);
});

// a small whole-library reply with every block, as the backend would send it
const reply = () => ({
  coverage: { indexed: 400, total: 400, pending: 0, roots: 1, pendingRoots: [], paused: false, maxAgeSec: 0, state: 'ready' },
  overview: { matches: { n: 400, top: 100 }, photos: 400, folders: 4, firstYear: 2022, lastYear: 2026, perYear: [{ year: 2022, n: 100, top: 20 }, { year: 2023, n: 300, top: 80 }],
    ratingDist: [100, 50, 50, 100, 50, 50], rejected: 0, labels: { red: 1, yellow: 0, green: 0, blue: 0, purple: 0 }, gear: { bodies: 3, lenses: 4, makers: 2 },
    favouriteFocal: { mm: 24, share: 0.2, topShare: 0.1 }, medianSetup: { focalMm: 35, fNumber: 2.8, shutterSec: 0.004, iso: 200 }, peak: { hour: 18, month: 10, monthN: 50 },
    fileTypes: [{ type: 'JPG', n: 400 }], edited: 0, insights: [] },
  cameras: { known: 400, makers: [{ maker: 'Canon', n: 300, top: 80, bodies: 2 }, { maker: 'Sony', n: 100, top: 20, bodies: 1 }],
    bodies: [{ key: 'Canon|R5', maker: 'Canon', model: 'R5', n: 200, top: 50, keeperRate: { top: 50, n: 200 }, cover: null, firstShot: null, lastShot: null, serials: 1, topLens: null, topPlace: null },
      { key: 'Sony|A7', maker: 'Sony', model: 'A7', n: 100, top: 20, keeperRate: { top: 20, n: 100 }, cover: null, firstShot: null, lastShot: null, serials: 1, topLens: null, topPlace: null }],
    years: [{ year: 2022, n: 100, cumulative: 100 }, { year: 2023, n: 300, cumulative: 400 }] },
  lenses: { known: 400, lenses: [{ name: 'L1', maker: null, n: 400, top: 100, keeperRate: { top: 100, n: 400 }, focalRange: null, isPrime: true, cover: null }],
    focal: { bins: [[24, 400, 100]], modalMm: 24 }, focalByBucket: [{ id: 'wide', n: 400, top: 100, cover: null, modalMm: 24 }] },
  exposure: { known: { aperture: 400, shutter: 400, iso: 400 }, aperture: [{ id: 'a1', n: 400, top: 100, cover: null }], shutter: [{ id: 'fast', n: 400, top: 100, cover: null }], iso: [{ id: '100', n: 400, top: 100 }],
    median: { fNumber: 2.8, shutterSec: 0.004, iso: 200 } },
  time: { known: 400, hours: Array.from({ length: 24 }, (_, i) => ({ n: i === 18 ? 400 : 0, top: i === 18 ? 100 : 0 })), months: Array.from({ length: 12 }, (_, i) => ({ n: i === 9 ? 400 : 0, top: i === 9 ? 100 : 0 })),
    years: [{ year: 2022, n: 100, top: 20 }, { year: 2023, n: 300, top: 80 }], peak: { hour: 18, month: 10 } },
  places: { known: 400, total: 3, zoom: 2, bbox: [0, 0, 1, 1], cells: [{ key: '0', lat: 0, lng: 0, n: 400, top: 100 }], top: [{ key: '0', lat: 0, lng: 0, n: 400, top: 100 }] },
  top: { total: 100, items: [{ id: '1' }, { id: '2' }], page: { next: 'x' }, byRating: [0, 0, 0, 0, 50, 50], byLabel: { red: 1, yellow: 0, green: 0, blue: 0, purple: 0 } },
});

test('every degenerate shape keeps its numbers finite and consistent', () => {
  for (const kind of G.DEGENERATE) {
    const r = G.degenerate(reply(), kind);
    finite(r, kind);
    assert.ok(r.overview.photos >= 0, kind);
    assert.ok(r.overview.matches.top <= r.overview.matches.n, `${kind}: top images cannot exceed photos`);
  }
});

test('one photo: a single entry per list, a fixed list keeps its slots', () => {
  const r = G.degenerate(reply(), 'one');
  assert.equal(r.overview.photos, 1);
  assert.equal(r.cameras.makers.length, 1);
  assert.equal(r.cameras.makers[0].n, 1);
  assert.equal(r.time.hours.length, 24);
  assert.equal(r.time.hours.reduce((a, h) => a + h.n, 0), 1);
  assert.equal(r.time.months.reduce((a, h) => a + h.n, 0), 1);
  assert.equal(r.top.total, 1);
});

test('no lens data, no GPS, no ratings, all five stars, one year', () => {
  const nl = G.degenerate(reply(), 'nolens');
  assert.equal(nl.lenses.lenses.length, 0);
  assert.equal(nl.overview.favouriteFocal, null);
  const ng = G.degenerate(reply(), 'nogps');
  assert.equal(ng.places.cells.length, 0);
  assert.equal(ng.places.bbox, null);
  const un = G.degenerate(reply(), 'unrated');
  assert.equal(un.overview.matches.top, 0);
  assert.equal(un.top.total, 0);
  assert.equal(un.cameras.bodies[0].top, 0);
  const a5 = G.degenerate(reply(), 'all5');
  assert.equal(a5.overview.matches.top, a5.overview.matches.n);
  assert.equal(a5.cameras.bodies[0].keeperRate.top, a5.cameras.bodies[0].keeperRate.n);
  const oy = G.degenerate(reply(), 'oneyear');
  assert.equal(oy.overview.perYear.length, 1);
  assert.equal(oy.overview.perYear[0].n, 400);
  assert.equal(oy.time.years.length, 1);
});
