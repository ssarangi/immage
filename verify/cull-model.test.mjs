// SPDX-License-Identifier: GPL-3.0-or-later
// cullModel.ts: the scope, chips, grid order, keys and Find mapping of Auto Cull.
import test from 'node:test';
import assert from 'node:assert/strict';
import { applySearch, badgeOf, chipCounts, decisionKey, inChip, matchNamed, nextInReview, parseDefaults, DEFAULT_SETTINGS,
  rejectionReasons, reviewKey, reviewOrder, scopeFor, withDecision, byKey } from '../app/src/cullModel.ts';

const d = (id, path, o = {}) => ({ id, sessionId: 1, path, version: 0, backing: [], proposed: 'keep', finalStatus: null, status: 'keep',
  reason: 'unique', group: null, bestPath: null, best: false, highlight: false, qualityScore: 1, factors: [], blurryRegion: null,
  analysis: {}, fileSize: 1, fileModifiedMs: 1, applied: false, ...o });
const img = (imgid, path, o = {}) => ({ imgid, path, filename: path, ...o });

test('scope is the selection, else every local photo; stacked JPEGs go as backing', () => {
  const a = img(1, 'a.raf', { version: 2 }), b = img(2, 'b.raf'), r = img(3, 'r.jpg', { remote: {} });
  const partners = new Map([[1, [img(9, 'a.jpg', { version: 1 })]]]);
  let s = scopeFor([a, b, r], new Set(), partners);
  assert.equal(s.selected, false);
  assert.deepEqual(s.items, [{ path: 'a.raf', version: 2, backing: [{ path: 'a.jpg', version: 1 }] }, { path: 'b.raf', version: 0 }]);
  s = scopeFor([a, b, r], new Set([2, 3]), partners);
  assert.deepEqual([s.selected, s.items.map((i) => i.path)], [true, ['b.raf']]);
});

test('chips, counts and rejection reasons', () => {
  const ds = [d(1, 'a', { best: true, group: 1, highlight: true, reason: 'bestInGroup' }),
    d(2, 'b', { status: 'reject', proposed: 'reject', reason: 'duplicateOf', group: 1 }),
    d(3, 'c', { status: 'reject', proposed: 'reject', reason: 'blurry' }),
    d(4, 'e', { status: 'reject', proposed: 'keep', finalStatus: 'reject' })];
  assert.deepEqual(chipCounts(ds), { all: 4, highlights: 1, blurry: 1, duplicates: 2, rejected: 3 });
  assert.ok(!inChip(d(5, 'x', { highlight: true, status: 'reject' }), 'highlights'));
  const why = rejectionReasons(ds).map((r) => `${r.reason}:${r.count}`);
  assert.deepEqual(why.sort(), ['blurry:1', 'duplicateOf:1', 'rejectOverride:1']);
  assert.deepEqual(ds.map(badgeOf), ['best', 'reject', 'reject', 'reject']);
  assert.equal(badgeOf(d(6, 'k')), 'keep');
});

test('review order groups a set with its best first and filters by chip', () => {
  const photos = ['a', 'b', 'c', 'e', 'f'].map((p, n) => img(n + 1, p));
  const m = byKey({ session: {}, decisions: [d(1, 'a', { group: 1 }), d(2, 'b'), d(3, 'c', { group: 1, best: true }),
    d(4, 'e', { group: 2, best: true }), d(5, 'f', { group: 1, status: 'reject' })] });
  assert.deepEqual(reviewOrder(photos, m, 'all').map((i) => i.path), ['c', 'a', 'f', 'b', 'e']);
  assert.deepEqual(reviewOrder(photos, m, 'duplicates').map((i) => i.path), ['c', 'a', 'f', 'e']);
  assert.deepEqual(reviewOrder(photos, m, 'rejected').map((i) => i.path), ['f']);
  // a photo the cull never saw shows only under All
  assert.deepEqual(reviewOrder([...photos, img(9, 'new')], m, 'all').at(-1).path, 'new');
  assert.ok(!reviewOrder([img(9, 'new')], m, 'blurry').length);
  assert.equal(decisionKey('a', undefined), decisionKey('a', 0));
});

test('an override keeps the counts and moves the best mark', () => {
  const plan = { session: { keepCount: 2, rejectCount: 0, highlightCount: 0 },
    decisions: [d(1, 'a', { group: 1, best: true }), d(2, 'b', { group: 1 })] };
  const next = withDecision(plan, d(2, 'b', { group: 1, best: true, finalStatus: 'keep' }));
  assert.deepEqual(next.decisions.map((x) => x.best), [false, true]);
  const rej = withDecision(next, d(1, 'a', { group: 1, status: 'reject', finalStatus: 'reject' }));
  assert.deepEqual([rej.session.keepCount, rej.session.rejectCount], [1, 1]);
});

test('review keys never take typing or chords, and step stays within the list', () => {
  assert.deepEqual(reviewKey({ key: 'R' }), { type: 'reject' });
  assert.deepEqual(reviewKey({ key: 'p' }), { type: 'keep' });
  assert.deepEqual(reviewKey({ key: 'b' }), { type: 'best' });
  assert.equal(reviewKey({ key: 'k' }), null);
  assert.equal(reviewKey({ key: 'r', typing: true }), null);
  assert.equal(reviewKey({ key: 'r', metaKey: true }), null);
  const l = [{ imgid: 1 }, { imgid: 2 }];
  assert.equal(nextInReview(l, 2, 1), 2);
  assert.equal(nextInReview(l, 1, -1), 1);
  assert.equal(nextInReview(l, 9, 1), 1);
});

test('stored defaults keep only valid values', () => {
  assert.deepEqual(parseDefaults(null), DEFAULT_SETTINGS);
  assert.deepEqual(parseDefaults('nope'), DEFAULT_SETTINGS);
  const p = parseDefaults(JSON.stringify({ shootType: 'wildlife', duplicateRatio: 'x', highlightPercent: 250, blurThreshold: 60,
    highlightRating: 4, highlightLabel: 3 }));
  assert.deepEqual([p.shootType, p.duplicateRatio, p.highlightPercent, p.blurThreshold, p.highlightRating, p.highlightLabel],
    ['wildlife', 'moderate', 15, 60, 4, 0]);
});

test('named camera and lens matching', () => {
  const cams = ['Fujifilm X-T5', 'Sony ILCE-7M4'];
  assert.equal(matchNamed('fujifilm x-t5', cams), 'Fujifilm X-T5');
  assert.equal(matchNamed('Sony', cams), 'Sony ILCE-7M4');
  assert.equal(matchNamed('Canon R5', cams), null);
  assert.equal(matchNamed(null, cams), null);
});

test('a Gemini search sets the words, the filter, a matching camera and the labels', () => {
  const state = { search: { text: 'old', mode: 'smart' }, filter: { minRating: 0, rejected: 'all', edited: 'all', labels: 0,
    unlabeled: true, kinds: [], cameras: [], bodies: [], lenses: [], keywords: ['keep'], takenFrom: '', takenTo: '', iso: [], aperture: [], focal: [], shutter: [] } };
  const found = { summary: 's', text: 'sunset', filter: { minRating: 4, kinds: ['RAW'], takenFrom: '2024-01-01', rejected: 'bogus' },
    camera: 'Fujifilm', lens: 'Zeiss 50', labels: 4, unmapped: ['people'] };
  const { state: next, unmapped } = applySearch(state, found, ['Fujifilm X-T5'], ['XF 16mm']);
  assert.equal(next.search.text, 'sunset'); assert.equal(next.search.mode, 'smart');
  assert.deepEqual([next.filter.minRating, next.filter.kinds, next.filter.takenFrom, next.filter.rejected], [4, ['RAW'], '2024-01-01', 'all']);
  assert.deepEqual([next.filter.cameras, next.filter.lenses, next.filter.labels, next.filter.unlabeled], [['Fujifilm X-T5'], [], 4, false]);
  assert.deepEqual(next.filter.keywords, ['keep']);
  assert.deepEqual(unmapped, ['people', 'Zeiss 50']);
});

test('the cull defaults carry the faces options: on by default, main subject, anything else falls back', () => {
  assert.equal(DEFAULT_SETTINGS.useFaces, true);
  assert.equal(DEFAULT_SETTINGS.eyeMode, 'mainSubject');
  const kept = parseDefaults(JSON.stringify({ useFaces: false, eyeMode: 'anyFace' }));
  assert.deepEqual([kept.useFaces, kept.eyeMode], [false, 'anyFace']);
  const odd = parseDefaults(JSON.stringify({ useFaces: 'yes', eyeMode: 'both' }));
  assert.deepEqual([odd.useFaces, odd.eyeMode], [true, 'mainSubject']);
  assert.deepEqual([parseDefaults(null).useFaces, parseDefaults('not json').eyeMode], [true, 'mainSubject']);
});
