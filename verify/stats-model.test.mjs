// SPDX-License-Identifier: GPL-3.0-or-later
// The Statistics view's state and chart arithmetic (app/src/statsModel.ts, statsMath.ts),
// which import nothing, so they load as they are. No engine, no browser.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
const require = createRequire(new URL('../app/package.json', import.meta.url));
const { transformSync } = await import(require.resolve('rolldown/experimental'));
const load = async (name) => {
  const path = new URL(`../app/src/${name}`, import.meta.url).pathname;
  const code = transformSync(path, readFileSync(path, 'utf8')).code;
  return import(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);
};
const M = await load('statsModel.ts');
const A = await load('statsMath.ts');
const keys = (s) => M.chips(s).map((c) => c.key);

test('initial state: library scope, threshold 4, nothing selected', () => {
  const s = M.initialState();
  assert.deepEqual(s.scope, { kind: 'library' });
  assert.equal(s.filter.minRating, 4);
  assert.equal(s.filter.onlyTop, false);
  assert.deepEqual(M.chips(s), []);
  assert.equal(M.initialState(9).filter.minRating, 5);
  assert.equal(M.initialState(0).filter.minRating, 1);
});

test('click sets, click again clears, another mark replaces, shift adds', () => {
  let s = M.initialState();
  s = M.toggleValue(s, 'focalBuckets', 'normal');
  assert.deepEqual(s.filter.focalBuckets, ['normal']);
  s = M.toggleValue(s, 'focalBuckets', 'wide');
  assert.deepEqual(s.filter.focalBuckets, ['wide']);
  s = M.toggleValue(s, 'focalBuckets', 'tele', true);
  assert.deepEqual(s.filter.focalBuckets, ['wide', 'tele']);
  s = M.toggleValue(s, 'focalBuckets', 'wide', true);
  assert.deepEqual(s.filter.focalBuckets, ['tele']);
  s = M.toggleValue(s, 'focalBuckets', 'tele');
  assert.equal('focalBuckets' in s.filter, false);
});

test('numeric dimensions keep numbers, sorted, whatever the mark passes', () => {
  let s = M.initialState();
  s = M.toggleValue(s, 'months', '10');
  s = M.toggleValue(s, 'months', 3, true);
  assert.deepEqual(s.filter.months, [3, 10]);
  assert.deepEqual([...M.selectedSet(s, 'months')], ['3', '10']);
});

test('chips: one per value, years grouped, tones, order of adding', () => {
  let s = M.initialState();
  s = M.toggleValue(s, 'months', 10);
  s = M.toggleValue(s, 'makers', 'Sony');
  s = M.setValues(s, 'years', [2014, 2015, 2016]);
  s = M.toggleValue(s, 'lenses', 'FE 85mm', true);
  assert.deepEqual(keys(s), ['months:10', 'makers:Sony', 'years', 'lenses:FE 85mm']);
  const c = M.chips(s);
  assert.equal(c[0].tone, 'amber');
  assert.equal(c[1].tone, 'cyan');
  assert.deepEqual(c[2].values, [2014, 2015, 2016]);
});

test('removing a chip, Esc removes the last added, clear all keeps scope and threshold', () => {
  let s = M.setMinRating(M.initialState(), 3);
  s = M.setScope(s, { kind: 'folder', folder: '/p', includeSubfolders: true });
  s = M.toggleValue(s, 'makers', 'Canon');
  s = M.toggleValue(s, 'hours', 18);
  s = M.setOnlyTop(s, true);
  assert.deepEqual(keys(s), ['makers:Canon', 'hours:18', 'onlyTop']);
  s = M.clearLast(s);
  assert.deepEqual(keys(s), ['makers:Canon', 'hours:18']);
  s = M.removeChip(s, 'makers:Canon');
  assert.deepEqual(keys(s), ['hours:18']);
  assert.equal(M.clearLast(M.clearAll(s)), null);
  const cleared = M.clearAll(M.toggleValue(s, 'lenses', 'x'));
  assert.equal(cleared.filter.minRating, 3);
  assert.equal(cleared.scope.kind, 'folder');
  assert.deepEqual(M.chips(cleared), []);
});

test('the threshold is not a filter: changing it adds no chip and onlyTop is separate', () => {
  let s = M.setMinRating(M.initialState(), 5);
  assert.deepEqual(M.chips(s), []);
  assert.equal(M.setMinRating(s, 5), s);
  s = M.toggleOnlyTop(s);
  assert.equal(s.filter.onlyTop, true);
  assert.deepEqual(keys(s), ['onlyTop']);
});

test('request: empty fields are left out, sections in a fixed order, equal states are equal', () => {
  let s = M.toggleValue(M.initialState(), 'makers', 'Sony');
  s = M.toggleValue(s, 'makers', 'Sony');
  const a = M.buildRequest(s, ['lenses', 'overview', 'cameras']);
  assert.deepEqual(a.sections, ['overview', 'cameras', 'lenses']);
  assert.deepEqual(a.filter, { minRating: 4, onlyTop: false });
  const b = M.buildRequest(M.initialState(), ['overview', 'cameras', 'lenses'], 'abc');
  assert.equal(b.etag, 'abc');
  assert.ok(M.sameRequest(a, b));
  const c = M.buildRequest(M.toggleValue(M.initialState(), 'bodies', 'Sony|ILCE-7M4'), ['overview']);
  assert.deepEqual(c.filter.bodies, ['Sony|ILCE-7M4']);
  assert.ok(!M.sameRequest(a, c));
});

test('prune drops what a new scope cannot have', () => {
  let s = M.toggleValue(M.initialState(), 'years', 2014);
  s = M.toggleValue(s, 'makers', 'Sony');
  const { state, removed } = M.prune(s, (dim, v) => !(dim === 'years' && v === 2014));
  assert.equal(removed, 1);
  assert.deepEqual(keys(state), ['makers:Sony']);
});

test('donut: top five plus Other, arcs sum to the circle', () => {
  const rows = ['A', 'B', 'C', 'D', 'E', 'F', 'G'].map((maker, i) => ({ maker, n: 100 - i * 10, top: i }));
  const marks = A.groupMakers(rows, 'Other');
  assert.equal(marks.length, 6);
  assert.equal(marks[5].label, 'Other');
  assert.equal(marks[5].n, 50 + 40);
  assert.deepEqual(marks[5].members, ['F', 'G']);
  assert.equal(new Set(marks.map((m) => m.accent)).size, 6);
  const arcs = A.donutArcs(marks, 84, 3);
  const circ = 2 * Math.PI * 84;
  const sum = arcs.reduce((a, x) => a + x.len + 3, 0);
  assert.ok(Math.abs(sum - circ) < 1e-6);
  assert.ok(Math.abs(arcs.reduce((a, x) => a + x.fraction, 0) - 1) < 1e-9);
  assert.equal(A.groupMakers(rows.slice(0, 3), 'Other').length, 3);
});

test('rankings and two-layer bars', () => {
  const rows = [{ n: 100, top: 10 }, { n: 400, top: 20 }, { n: 50, top: 20 }];
  assert.deepEqual(A.rankRows(rows, 'most-used').map((r) => r.n), [400, 100, 50]);
  assert.deepEqual(A.rankRows(rows, 'keeper').map((r) => r.n), [50, 100, 400]);
  assert.deepEqual(A.rankRows(rows, 'top').map((r) => r.n), [400, 50, 100]);
  assert.deepEqual(A.rankRows(rows, 'keeper', 100).map((r) => r.n), [100, 400]);
  const w = A.barWidths(A.rankRows(rows, 'most-used'), 'most-used');
  assert.equal(w[0].all, 1);
  assert.equal(w[0].top, 20 / 400);
  const k = A.barWidths(A.rankRows(rows, 'keeper'), 'keeper');
  assert.equal(k[0].top, 1);
  assert.ok(k[2].top < k[1].top);
  assert.equal(A.keeperRate({ top: 1, n: 0 }), null);
});

test('field of view: 35 mm full frame angles and a wedge that grows with its count', () => {
  assert.ok(Math.abs(A.fovDegrees(18) - 90) < 1e-9);
  assert.ok(Math.abs(A.fovDegrees(50) - 39.6) < 0.1);
  const w = A.fanWedges([{ id: 'a', modalMm: 85, n: 100 }, { id: 'b', modalMm: 24, n: 400 }], 24, 170);
  assert.deepEqual(w.map((x) => x.id), ['b', 'a']);
  assert.equal(w[0].len, 300);
  assert.ok(w[1].len < w[0].len);
  assert.ok(w[0].y1 < 170 && w[0].y2 > 170);
  assert.ok(Math.abs((170 - w[0].y1) - (w[0].y2 - 170)) < 1e-9);
});

test('era chips fold early years and honour cumulative mode', () => {
  const years = [2018, 2019, 2020, 2021, 2022, 2023, 2024, 2025].map((year, i) => ({ year, n: 10 * (i + 1), cumulative: 10 * ((i + 1) * (i + 2)) / 2 }));
  const t = A.eraChips(years, 'that-year', 5);
  assert.deepEqual(t.map((c) => c.id), ['<=2020', '2021', '2022', '2023', '2024', '2025']);
  assert.deepEqual(t[0].years, [2018, 2019, 2020]);
  assert.equal(t[0].n, 60);
  const c = A.eraChips(years, 'cumulative', 5);
  assert.equal(c[2].n, years[4].cumulative);
  assert.deepEqual(c[2].years, [2018, 2019, 2020, 2021, 2022]);
  assert.deepEqual(A.yearRuns([2016, 2014, 2015, 2019]), [[2014, 2016], [2019, 2019]]);
});

test('insights come from the counts; compact numbers', () => {
  const bodies = [{ key: 'a', maker: 'Sony', model: 'A7', n: 1000, top: 50 }, { key: 'b', maker: 'Fuji', model: 'XT5', n: 500, top: 80 },
    { key: 'c', maker: 'Tiny', model: 'T', n: 10, top: 10 }];
  const ins = A.keeperBodyInsight(bodies);
  assert.equal(ins.key, 'b');
  assert.ok(ins.ratio > 1);
  assert.equal(A.keeperBodyInsight([{ key: 'a', maker: 'S', model: 'x', n: 5, top: 1 }]), null);
  assert.equal(A.compact(950), '950');
  assert.equal(A.compact(1204), '1.2k');
  assert.equal(A.compact(46880), '47k');
  assert.equal(A.compact(2_400_000), '2.4M');
});

// ---- the boundary with the backend (statsNormalize.ts) and "Open N in grid" (statsGrid.ts)
const N = await load('statsNormalize.ts');
const G = await load('statsGrid.ts');

test('normaliser: Rust nulls for optional blocks and filters become absent, real nulls stay', () => {
  const raw = {
    etag: 'e', coverage: { indexed: 3 },
    overview: { insights: [{ id: 'keeper-lens', params: { lens: 'x' }, apply: { minRating: 4, onlyTop: false, lenses: ['x'], makers: null, years: null } }] },
    cameras: { known: 1, timeline: null },
    lenses: { zoomUsage: null },
    exposure: { apertureByShutter: null },
    time: { weekdays: null, monthByHour: null },
    places: { bbox: null, heat: null, cells: [{ key: 'a', solo: null, name: null }] },
  };
  const out = N.normalizeReply(structuredClone(raw));
  assert.deepEqual(out.overview.insights[0].apply, { minRating: 4, onlyTop: false, lenses: ['x'] });
  for (const [block, key] of [['cameras', 'timeline'], ['lenses', 'zoomUsage'], ['exposure', 'apertureByShutter'], ['time', 'weekdays'], ['time', 'monthByHour'], ['places', 'heat']]) {
    assert.ok(!(key in out[block]), `${block}.${key} is absent`);
  }
  assert.equal(out.places.bbox, null, 'bbox stays a real null: no positioned photos');
  assert.ok(!('solo' in out.places.cells[0]) && !('name' in out.places.cells[0]));
  assert.deepEqual(N.normalizeReply({ notModified: true }), { notModified: true });
  const place = N.normalizePlace({ key: 'k', name: null, lat: 1, lng: 2, topBody: null, photos: [] });
  assert.equal(place.name, null, 'a place without a name keeps its null');
});

test('open in grid: entries keep the order, a vanished handle is counted, the cap is 20,000', async () => {
  const rows = (ids) => ids.map((id) => ({ imgid: Number(id), path: `/p/${id}.jpg`, filename: `${id}.jpg` }));
  const seen = [];
  const got = await G.gridEntries(['3', '1', '2'], async (ids) => { seen.push(ids); return rows(ids.filter((i) => i !== '1')); });
  assert.deepEqual(got.entries.map((e) => e.imgid), [3, 2]);
  assert.equal(got.missing, 1);
  assert.deepEqual(await G.gridEntries([], async () => { throw new Error('not asked'); }), { entries: [], missing: 0 });
  const many = Array.from({ length: 25_000 }, (_, i) => String(i));
  const all = await G.gridEntries(many, async (ids) => rows(ids));
  assert.equal(G.GRID_CAP, 20_000);
  assert.equal(all.entries.length, 20_000);
  assert.equal(seen.length, 1);
});
