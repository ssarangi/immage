// SPDX-License-Identifier: GPL-3.0-or-later
// The Top images section's logic (app/src/statsTop.ts): the paged feed and its cursors, pages that arrive after the
// filter changed, the empty state's way out, the strips of best frames, the grid's keyboard map and the window of
// cards on screen. statsTop.ts imports nothing but types, so it loads as it is. No engine, no browser.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
const require = createRequire(new URL('../app/package.json', import.meta.url));
const { transformSync } = await import(require.resolve('rolldown/experimental'));
const path = new URL('../app/src/statsTop.ts', import.meta.url).pathname;
const T = await import(`data:text/javascript;base64,${Buffer.from(transformSync(path, readFileSync(path, 'utf8')).code).toString('base64')}`);

const photo = (i, extra = {}) => ({ id: String(i), path: `/p/${i}.jpg`, thumb: { kind: 'cache', id: String(i) }, rating: 5, rejected: false, label: null, ...extra });
const range = (a, b) => Array.from({ length: b - a }, (_, i) => photo(a + i));
const req = (min, extra = {}) => ({ scope: { kind: 'library' }, filter: { minRating: min, onlyTop: false, ...extra }, sections: ['top'] });
const labels = { red: 1, yellow: 2, green: 3, blue: 4, purple: 5 };
const tick = () => new Promise((r) => setTimeout(r, 0));

/** a backend of `total` photos: pages of 24 by cursor "c<offset>", each call logged, each answer held until released */
function backend(total, { pages = true } = {}) {
  const log = [], held = [];
  const gate = (v) => (pages ? new Promise((res) => held.push(() => res(v))) : Promise.resolve(v));
  return {
    log, held,
    first: (r) => { log.push(['first', JSON.stringify(r.filter)]); const n = Math.min(24, total); return gate({ total, items: range(0, n), page: { next: n < total ? 'c24' : null }, byRating: [0, 0, 0, 0, 0, total], byLabel: labels }); },
    page: (cursor) => { log.push(['page', cursor]); const at = Number(cursor.slice(1)); const end = Math.min(total, at + 24); return gate({ items: range(at, end), next: end < total ? `c${end}` : null }); },
    release: async (n = held.length) => { for (let i = 0; i < n; i++) held.shift()?.(); await tick(); await tick(); },
  };
}

test('a first request loads the first page, more() follows the cursor to the end', async () => {
  const b = backend(60), feed = new T.TopFeed(b);
  feed.request(req(4));
  assert.equal(feed.get().phase, 'loading');
  await b.release();
  assert.equal(feed.get().phase, 'ready');
  assert.equal(feed.get().items.length, 24);
  assert.equal(feed.get().total, 60);
  feed.more(); assert.equal(feed.get().paging, 'loading');
  feed.more(); // a second call while one is on its way asks nothing
  assert.equal(b.log.filter((l) => l[0] === 'page').length, 1);
  await b.release();
  feed.more(); await b.release();
  const s = feed.get();
  assert.deepEqual([s.items.length, s.next, s.paging], [60, null, 'idle']);
  feed.more(); // nothing left
  assert.equal(b.log.filter((l) => l[0] === 'page').length, 2);
  assert.deepEqual(s.items.map((p) => p.id), range(0, 60).map((p) => p.id));
});

test('the same request again changes nothing; a new one keeps the old photos (stale) until its first page lands', async () => {
  const b = backend(60), feed = new T.TopFeed(b);
  feed.request(req(4)); await b.release();
  feed.request(req(4));
  assert.equal(b.log.length, 1);
  feed.request(req(3));
  const s = feed.get();
  assert.equal(s.stale, true);
  assert.equal(s.items.length, 24, 'the old grid stays while the new one loads');
  feed.more(); assert.equal(b.log.length, 2, 'no paging while stale');
  await b.release();
  assert.equal(feed.get().stale, false);
});

test('a page that lands after the filter changed is dropped', async () => {
  const b = backend(100), feed = new T.TopFeed(b);
  feed.request(req(4)); await b.release();
  feed.more(); // page 2 of the old filter is on its way
  feed.request(req(3, { labels: ['red'] }));
  // the old page lands first, then the new first page
  await b.release(1);
  assert.equal(feed.get().items.length, 24, 'the old page did not join the grid');
  assert.equal(feed.get().stale, true);
  await b.release();
  const s = feed.get();
  assert.equal(s.items.length, 24);
  assert.equal(s.stale, false);
  assert.equal(s.paging, 'idle');
  assert.equal(s.next, 'c24');
});

test('a first page that lands after a newer request is dropped too', async () => {
  const b = backend(30), feed = new T.TopFeed(b);
  feed.request(req(4));
  feed.request(req(5));
  const [a, c] = [b.held[0], b.held[1]];
  c(); await tick(); await tick();
  assert.equal(feed.get().phase, 'ready');
  a(); await tick(); await tick();
  assert.equal(feed.get().key, JSON.stringify(req(5)));
  assert.equal(feed.get().items.length, 24);
});

test('"stats-changed" on a page asks for the first page again, quietly', async () => {
  const b = backend(100, { pages: false });
  let refuse = true;
  const source = { first: b.first, page: (c) => (refuse ? (refuse = false, Promise.reject(new Error('stats-changed: the statistics changed; reload them'))) : b.page(c)) };
  const feed = new T.TopFeed(source);
  feed.request(req(4)); await tick();
  feed.more(); await tick(); await tick();
  assert.equal(b.log.filter((l) => l[0] === 'first').length, 2, 'the first page was asked for again');
  assert.equal(feed.get().pageError, null);
  assert.equal(feed.get().phase, 'ready');
  assert.equal(feed.get().items.length, 24);
});

test('another page error is kept and Try again retries that page', async () => {
  const b = backend(100, { pages: false });
  let fail = true;
  const feed = new T.TopFeed({ first: b.first, page: (c) => (fail ? (fail = false, Promise.reject(new Error('boom'))) : b.page(c)) });
  feed.request(req(4)); await tick();
  feed.more(); await tick(); await tick();
  assert.deepEqual([feed.get().paging, feed.get().pageError, feed.get().items.length], ['error', 'boom', 24]);
  feed.retry(); await tick(); await tick();
  assert.deepEqual([feed.get().paging, feed.get().pageError, feed.get().items.length], ['idle', null, 48]);
});

test('a first page that fails is an error; retry asks again', async () => {
  let fail = true;
  const b = backend(10, { pages: false });
  const feed = new T.TopFeed({ first: (r) => (fail ? (fail = false, Promise.reject(new Error('locked'))) : b.first(r)), page: b.page });
  feed.request(req(4)); await tick(); await tick();
  assert.deepEqual([feed.get().phase, feed.get().error, feed.get().items.length], ['error', 'locked', 0]);
  feed.retry(); await tick(); await tick();
  assert.deepEqual([feed.get().phase, feed.get().total], ['ready', 10]);
});

test('refresh keeps the grid, and what was paged in, when the first page is unchanged', async () => {
  const b = backend(100, { pages: false }), feed = new T.TopFeed(b);
  feed.request(req(4)); await tick();
  feed.more(); await tick(); await tick();
  assert.equal(feed.get().items.length, 48);
  feed.refresh(); await tick(); await tick();
  assert.equal(b.log.filter((l) => l[0] === 'first').length, 2);
  assert.equal(feed.get().items.length, 48, 'nothing moved: the pages stay');
  assert.equal(feed.get().next, 'c48');
});

test('a refresh that finds a different first page resets the grid and drops the page on its way', async () => {
  let n = 0;
  const held = [];
  const source = {
    first: async () => { n++; return n === 1 ? { total: 100, items: range(0, 24), page: { next: 'c24' }, byRating: [0, 0, 0, 0, 0, 100], byLabel: labels }
      : { total: 99, items: range(1, 25), page: { next: 'c25' }, byRating: [0, 0, 0, 0, 0, 99], byLabel: labels }; },
    page: () => new Promise((res) => held.push(() => res({ items: range(24, 48), next: 'c48' }))),
  };
  const feed = new T.TopFeed(source);
  feed.request(req(4)); await tick(); await tick();
  feed.more();
  feed.refresh(); await tick(); await tick();
  assert.deepEqual([feed.get().total, feed.get().items[0].id, feed.get().paging], [99, '1', 'idle']);
  held.shift()(); await tick(); await tick();
  assert.equal(feed.get().items.length, 24, 'the old cursor\'s page was dropped');
});

test('appendPage skips photos that are already there; samePage compares ids, ratings and labels', () => {
  assert.deepEqual(T.appendPage(range(0, 3), range(2, 5)).map((p) => p.id), ['0', '1', '2', '3', '4']);
  assert.equal(T.samePage(range(0, 30), range(0, 24)), true);
  assert.equal(T.samePage(range(0, 30), [photo(0, { rating: 4 })]), false);
  assert.equal(T.samePage(range(0, 3), range(0, 24)), false);
});

test('empty state: lowering the threshold sets threshold - 1, to one star; clearing is offered when filters are on', () => {
  assert.deepEqual(T.emptyAdvice(4, 0), { lowerTo: 3, canClear: false });
  assert.deepEqual(T.emptyAdvice(5, 2), { lowerTo: 4, canClear: true });
  assert.deepEqual(T.emptyAdvice(1, 1), { lowerTo: null, canClear: true });
});

test('the facet request leaves the labels out and keeps the rest', () => {
  const f = T.withoutLabels({ minRating: 4, onlyTop: false, labels: ['red'], makers: ['Sony'] });
  assert.deepEqual(f, { minRating: 4, onlyTop: false, makers: ['Sony'] });
});

test('strips: groups with a best frame and a top image, most top first, capped', () => {
  const cover = (i) => photo(i);
  const r = {
    lenses: { lenses: [
      { name: 'A', n: 100, top: 10, cover: cover(1) }, { name: 'B', n: 50, top: 30, cover: cover(2) },
      { name: 'C', n: 500, top: 0, cover: cover(3) }, { name: 'D', n: 80, top: 30, cover: null }, { name: 'E', n: 70, top: 30, cover: cover(5) },
    ] },
    cameras: { bodies: [{ key: 'Sony|A7', n: 9, top: 2, cover: cover(7) }, { key: 'Canon|R5', n: 4, top: 3, cover: cover(8) }] },
  };
  assert.deepEqual(T.stripGroups('lenses', r, (k) => k).map((g) => g.key), ['E', 'B', 'A'], 'C has no top image, D no frame; E beats B on photos');
  assert.deepEqual(T.stripGroups('bodies', r, (k) => k.replace('|', ' ')).map((g) => g.name), ['Canon R5', 'Sony A7']);
  assert.equal(T.stripGroups('lenses', r, (k) => k, 2).length, 2);
  assert.deepEqual(T.stripGroups('lenses', null, (k) => k), []);
});

test('keyboard: arrows move over the grid, clamp at the ends, and ask for the next page near the end', () => {
  assert.equal(T.columnsOf([10, 10, 10.5, 10, 300, 300]), 4);
  assert.equal(T.columnsOf([]), 1);
  assert.deepEqual(T.moveFocus('ArrowRight', 0, 4, 24, true), { index: 1, more: false });
  assert.deepEqual(T.moveFocus('ArrowLeft', 0, 4, 24, true), { index: 0, more: false });
  assert.deepEqual(T.moveFocus('ArrowDown', 5, 4, 24, true), { index: 9, more: false });
  assert.deepEqual(T.moveFocus('ArrowUp', 2, 4, 24, true), { index: 0, more: false });
  assert.deepEqual(T.moveFocus('ArrowDown', 22, 4, 24, true), { index: 23, more: true });
  assert.deepEqual(T.moveFocus('ArrowDown', 22, 4, 24, false), { index: 23, more: false });
  assert.deepEqual(T.moveFocus('End', 3, 4, 24, true), { index: 23, more: true });
  assert.deepEqual(T.moveFocus('Home', 9, 4, 24, true), { index: 0, more: false });
  assert.equal(T.moveFocus('a', 0, 4, 24, true), null);
  assert.equal(T.moveFocus('ArrowDown', 0, 4, 0, true), null);
});

test('visible range: the cards whose rows are within the margin of the view', () => {
  // 40 cards, 4 per row, rows 300 px apart, each 280 px tall
  const top = (i) => Math.floor(i / 4) * 300, bottom = (i) => top(i) + 280;
  assert.deepEqual(T.visibleRange(top, bottom, 40, 0, 900, 0), [0, 15]);
  assert.deepEqual(T.visibleRange(top, bottom, 40, 1500, 2400, 0), [20, 35]);
  assert.deepEqual(T.visibleRange(top, bottom, 40, 1500, 2400, 300), [16, 39]);
  assert.deepEqual(T.visibleRange(top, bottom, 40, 5000, 6000, 0), [40, 39], 'nothing near: an empty range');
  assert.deepEqual(T.visibleRange(top, bottom, 0, 0, 900, 700), [0, -1]);
});
