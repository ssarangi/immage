// SPDX-License-Identifier: GPL-3.0-or-later
// peopleModel.ts: sections and tiles, stable order, the selection reducer, grid geometry and marquee, the
// keyboard map, the name combobox's ranking, the Undo line and the scan's plain state.
import test from 'node:test';
import assert from 'node:assert/strict';
import { NO_SEL, GAP, PAD, CAPTION, geometry, gridKey, keepSelected, mergeUndo, nameOptions, needsReview, scanView, sectionCounts,
  scanNotes, scanJob, undoMs, suggestRows, capRows, lookAlikes, sectionTiles, select, stableOrder, tilePos, tilesInRect, totalHeight, viewKind, visibleRange } from '../app/src/peopleModel.ts';

const person = (id, name, o = {}) => ({ id, name, state: 'active', faces: 5, photos: 10, cover_face_id: null, cover_manual: false, suggestions: 0, ...o });
const data = (o = {}) => ({ people: [], groups: [], unknown: [], unknownMore: false, ignoredFaces: [], ignoredMore: false, ...o });
const face = (id, o = {}) => ({ id, path: `/p/${id}.jpg`, x: 0, y: 0, w: 1, h: 1, score: 1, frontal: 1, person_id: null, state: 'unreviewed',
  suggested_person: null, suggested_sim: null, cluster_id: null, ...o });
const status = (o = {}) => ({ enabled: true, installed: true, eyes_available: false, scanned: 0, faces_found: 0, pending: null, folders_waiting: 0,
  infer_ms: null, health: 'ok', failure: null, undo: null, offline_folders: 0, totals: { photos: 0, faces: 0, people: 0, unreviewed: 0, groups: 0, suggestions: 0, unknown: 0, ignored_faces: 0, ignored_people: 0, failed_photos: 0 }, ...o });

test('sections: people by photos, ignored apart, review lists "maybe" tiles then groups by size', () => {
  const d = data({ people: [person(1, 'Zed', { photos: 3 }), person(2, 'Amy', { photos: 30, suggestions: 4 }), person(3, 'Old', { state: 'ignored' })],
    groups: [{ id: 7, rep_face_id: 70, size: 3, members: [70, 71, 72] }, { id: 8, rep_face_id: 80, size: 9, members: [80] }], unknown: [face(1), face(2)], unknownMore: true, ignoredFaces: [face(9)] });
  assert.deepEqual(sectionTiles('people', d).map((t) => t.key), ['p:2', 'p:1']);
  assert.deepEqual(sectionTiles('ignored', d).map((t) => t.key), ['p:3', 'f:9']);
  assert.deepEqual(sectionTiles('review', d).find((t) => t.key === 'g:7').members, [70, 71, 72]);
  assert.equal(sectionTiles('ignored', d)[1].ignored, true);
  assert.deepEqual(sectionTiles('review', d).map((t) => t.key), ['s:2', 'g:8', 'g:7']);
  assert.deepEqual(sectionTiles('unassigned', d).map((t) => t.key), ['f:1', 'f:2']);
  assert.deepEqual(sectionCounts(d), { people: 2, review: 3, unassigned: 2, ignored: 2 });
  const totals = { photos: 9, faces: 9, people: 2, unreviewed: 5, groups: 2, suggestions: 4, unknown: 1820, ignored_faces: 40, ignored_people: 3, failed_photos: 0 };
  assert.deepEqual(sectionCounts(d, totals), { people: 2, review: 3, unassigned: 1820, ignored: 43 });
});

test('stable order: kept tiles stay, gone ones drop, new ones append', () => {
  const t = (k) => ({ key: k, kind: 'face', id: 0, path: '', maybe: null });
  assert.deepEqual(stableOrder(['a', 'b', 'c'], [t('c'), t('x'), t('a')]).map((x) => x.key), ['a', 'c', 'x']);
  assert.deepEqual(stableOrder(null, [t('c'), t('a')]).map((x) => x.key), ['c', 'a']);
});

test('selection: click, toggle, range, keyboard extend, all, clear, marquee', () => {
  const order = ['a', 'b', 'c', 'd', 'e'];
  let s = select(NO_SEL, { type: 'click', index: 1 }, order);
  assert.deepEqual([...s.keys], ['b']);
  s = select(s, { type: 'toggle', index: 3 }, order);
  assert.deepEqual([...s.keys].sort(), ['b', 'd']);
  s = select(s, { type: 'toggle', index: 1 }, order);
  assert.deepEqual([...s.keys], ['d']);
  s = select(select(NO_SEL, { type: 'click', index: 1 }, order), { type: 'range', index: 3 }, order);
  assert.deepEqual([...s.keys], ['b', 'c', 'd']);
  s = select(s, { type: 'range', index: 0 }, order);   // the anchor stays where the first click was
  assert.deepEqual([...s.keys], ['a', 'b']);
  s = select(select(NO_SEL, { type: 'click', index: 2 }, order), { type: 'move', index: 4, extend: true }, order);
  assert.deepEqual([...s.keys], ['c', 'd', 'e']);
  assert.equal(select(s, { type: 'all' }, order).keys.size, 5);
  assert.equal(select(s, { type: 'clear' }, order).keys.size, 0);
  const base = new Set(['a']);
  assert.deepEqual([...select(NO_SEL, { type: 'marquee', indexes: [2, 3], additive: true, base }, order).keys].sort(), ['a', 'c', 'd']);
  assert.deepEqual([...select(NO_SEL, { type: 'marquee', indexes: [2], additive: false, base }, order).keys], ['c']);
  assert.deepEqual([...keepSelected(s, ['c', 'x']).keys], ['c']);
});

test('geometry: columns follow the width, rows are fixed, the window is bounded', () => {
  const g = geometry(1000, 112);
  assert.equal(g.cols, Math.floor((1000 - 2 * PAD + GAP) / (112 + GAP)));
  assert.equal(g.rowH, 112 + CAPTION + GAP);
  assert.equal(geometry(10, 112).cols, 1);
  assert.equal(geometry(1000, 500).tile, 160);
  const r = visibleRange(0, 600, 5000, g);
  assert.equal(r.from, 0);
  assert.ok(r.to < 200, `mounts ${r.to}`);
  const far = visibleRange(totalHeight(5000, g) - 600, 600, 5000, g);
  assert.equal(far.to, 5000);
  assert.ok(far.to - far.from < 200);
  assert.deepEqual(tilePos(g.cols + 1, g), { x: PAD + (112 + GAP), y: PAD + g.rowH });
});

test('marquee: tiles that are not mounted still count, gaps do not', () => {
  const g = geometry(560, 100);      // 4 columns
  assert.equal(g.cols, 4);
  const a = tilePos(0, g), b = tilePos(5, g);
  const hit = tilesInRect({ x: a.x + 50, y: a.y + 50, w: b.x - a.x, h: b.y - a.y }, 100, g);
  assert.deepEqual(hit.sort((x, y) => x - y), [0, 1, 4, 5]);
  assert.deepEqual(tilesInRect({ x: a.x + 100 + 1, y: a.y + 1, w: 5, h: 5 }, 100, g), []);          // in the gap
  assert.deepEqual(tilesInRect({ x: a.x, y: a.y + 100 + CAPTION - 1, w: 5, h: 2 }, 100, g), [0]);   // the caption belongs to the tile
  assert.deepEqual(tilesInRect({ x: 0, y: 100000, w: 500, h: 50 }, 8, g), []);
});

test('keys: arrows move, letters name in review, M and I are People shortcuts', () => {
  const k = (key, o = {}) => ({ key, shift: false, meta: false, alt: false, ...o });
  const c = (o = {}) => ({ section: 'review', open: false, count: 20, cols: 5, active: 7, dir: 'ltr', ...o });
  assert.deepEqual(gridKey(k('ArrowRight'), c()), { kind: 'move', index: 8, extend: false });
  assert.deepEqual(gridKey(k('ArrowDown', { shift: true }), c()), { kind: 'move', index: 12, extend: true });
  assert.deepEqual(gridKey(k('ArrowRight'), c({ dir: 'rtl' })), { kind: 'move', index: 6, extend: false });
  assert.deepEqual(gridKey(k('ArrowUp'), c({ active: 2 })), { kind: 'move', index: 0, extend: false });
  assert.deepEqual(gridKey(k('End'), c()), { kind: 'move', index: 19, extend: false });
  assert.deepEqual(gridKey(k('n'), c()), { kind: 'name', seed: '' });
  assert.deepEqual(gridKey(k('d'), c()), { kind: 'name', seed: 'd' });
  assert.equal(gridKey(k('m'), c()).kind, 'name');
  assert.deepEqual(gridKey(k('m'), c({ section: 'people' })), { kind: 'merge' });
  assert.deepEqual(gridKey(k('i'), c({ section: 'people' })), { kind: 'ignore' });
  assert.equal(gridKey(k('d'), c({ section: 'people' })), null);
  assert.deepEqual(gridKey(k('a', { meta: true }), c()), { kind: 'all' });
  assert.deepEqual(gridKey(k('Escape'), c()), { kind: 'clear' });
  assert.deepEqual(gridKey(k('Enter'), c()), { kind: 'open' });
  assert.equal(gridKey(k('Backspace'), c({ section: 'people' })), null);
  assert.equal(gridKey(k('1'), c()), null);
  assert.equal(gridKey(k('ArrowRight'), c({ count: 0 })), null);
});

test('name options: prefix before contains, existing before create, no second Maya', () => {
  const people = [person(1, 'Maya', { photos: 400 }), person(2, 'Marcus', { photos: 90 }), person(3, 'Dad', { photos: 300 }),
    person(4, 'Old', { state: 'ignored' }), person(5, 'Annemarie', { photos: 5 })];
  const o = (q) => nameOptions(q, people).map((x) => x.kind === 'create' ? `+${x.name}` : x.name);
  assert.deepEqual(o('ma'), ['Maya', 'Marcus', 'Annemarie', '+ma']);
  assert.deepEqual(o('maya'), ['Maya']);                // an exact match offers no second Maya
  assert.deepEqual(o('MAYA '), ['Maya']);
  assert.deepEqual(o('zoe'), ['+zoe']);
  assert.deepEqual(o('old'), ['+old']);                 // an ignored person is not offered
  assert.deepEqual(o('').map((n) => n), ['Maya', 'Dad', 'Marcus', 'Annemarie']);
  assert.deepEqual(nameOptions('é', [person(9, 'Élodie')]).map((x) => x.kind === 'create' ? '+' : x.name), ['Élodie', '+']);
});

test('undo line: the same action joins, others replace', () => {
  const e = (kind, n, name = '', o = {}) => ({ kind, n, name, other: '', steps: 1, ...o });
  const a = mergeUndo(null, e('ignored', 3));
  const b = mergeUndo(a, e('ignored', 2));
  assert.deepEqual([b.n, b.steps], [5, 2]);
  assert.equal(mergeUndo(b, e('assigned', 4, 'Dad')).kind, 'assigned');
  const c = mergeUndo(e('assigned', 4, 'Dad'), e('assigned', 1, 'Dad'));
  assert.equal(c.n, 5);
  assert.equal(mergeUndo(e('assigned', 4, 'Dad'), e('assigned', 1, 'Mum')).n, 1);
  assert.equal(mergeUndo(e('merged', 1, 'Dad', { other: 'Dan' }), e('merged', 1, 'Dad', { other: 'Dan' })).steps, 1);
});

test('scan view and the centre state', () => {
  assert.equal(scanView(status({ enabled: false }), false).kind, 'off');
  const busy = scanView(status({ scanned: 100, pending: 900, infer_ms: 200 }), false);
  assert.deepEqual([busy.kind, busy.done, busy.total, busy.left], ['scanning', 100, 1000, 180]);
  assert.equal(scanView(status({ scanned: 100, pending: 900 }), true).kind, 'paused');
  assert.equal(scanView(status({ scanned: 100, pending: 0 }), true).kind, 'idle');
  assert.equal(scanView(status({ health: 'retrying', failure: 'x' }), false).kind, 'retrying');
  assert.equal(scanView(status({ health: 'blocked', failure: 'engine' }), false).reason, 'engine');
  const none = status();
  assert.equal(viewKind(null, false, scanView(none, false), false), 'loading');
  assert.equal(viewKind(null, true, scanView(none, false), false), 'error');
  assert.equal(viewKind(status({ enabled: false }), false, scanView(none, false), false), 'consent');
  assert.equal(viewKind(status({ installed: false }), false, scanView(none, false), false), 'consent');
  const scanning = status({ scanned: 3, pending: 40 });
  assert.equal(viewKind(scanning, false, scanView(scanning, false), true), 'scanning');
  const done = status({ scanned: 50, pending: 0 });
  assert.equal(viewKind(none, false, scanView(none, false), true), 'scanning');   // nothing counted yet
  assert.equal(viewKind(done, false, scanView(done, false), true), 'none');
  const faces = status({ totals: { photos: 1, faces: 4, people: 0, unreviewed: 4, groups: 1, suggestions: 0 } });
  assert.equal(viewKind(faces, false, scanView(faces, false), true), 'ready');
  assert.equal(needsReview(faces), true);
  assert.equal(needsReview(status({ ...faces, enabled: false })), false);
  assert.equal(needsReview(null), false);
});

test('scan notes: unreadable photos, offline folders, network sources left out', () => {
  assert.deepEqual(scanNotes(null), { failed: 0, offline: 0, network: 0 });
  const s = status({ offline_folders: 2, skipped_network_sources: 4 });
  s.totals.failed_photos = 34;
  assert.deepEqual(scanNotes(s), { failed: 34, offline: 2, network: 4 });
  assert.deepEqual(scanNotes(status()), { failed: 0, offline: 0, network: 0 });
});

test('scan view in the folder scope: photos of the open folder, no ETA without a rate, strip and activity agree', () => {
  const folder = { folderScope: true, folderTotal: 333 };
  const s = status({ scanned: 0, pending: 1, folders_waiting: 1, totals: { photos: 36, faces: 37, people: 0, unreviewed: 37, groups: 0, suggestions: 0, unknown: 0, ignored_faces: 0, ignored_people: 0, failed_photos: 0 } });
  const v = scanView(s, false, folder);
  assert.deepEqual([v.kind, v.done, v.total, v.left], ['scanning', 36, 333, null]);   // pending counts folders there: ignored
  assert.deepEqual(scanJob(v), { done: 36, total: 333, paused: false });
  assert.equal(scanView({ ...s, infer_ms: 400 }, false, folder).left, 119);            // (333 - 36) photos at 0.4 s
  assert.equal(scanView({ ...s, infer_ms: null }, false, folder).left, null);
  assert.equal(scanView(s, true, folder).kind, 'paused');
  const unknownTotal = scanView(s, false, { folderScope: true, folderTotal: null });
  assert.deepEqual([unknownTotal.kind, unknownTotal.total], ['scanning', null]);
  const end = scanView({ ...s, folders_waiting: 0, totals: { ...s.totals, photos: 333 } }, false, folder);
  assert.equal(end.kind, 'idle');
  assert.equal(scanJob(end), null);
  const cat = scanView(status({ scanned: 100, pending: 900, infer_ms: 200 }), false);
  assert.deepEqual([cat.done, cat.total, cat.left], [100, 1000, 180]);
  assert.equal(scanView(status({ scanned: 100, pending: 900 }), false).left, null);
});

test('the Undo line stays longer after a change to many faces', () => {
  assert.equal(undoMs(1), 5000);
  assert.equal(undoMs(20), 5000);
  assert.equal(undoMs(21), 8000);
  assert.equal(undoMs(88), 8000);
});

test('suggestions: only what is still there, strongest first, low left out, dismissed hidden, capped', () => {
  const d = data({ people: [person(1, 'Dad'), person(2, 'Maya'), person(3, 'Sam'), person(4, 'Old', { state: 'ignored' })],
    groups: [1, 2, 3, 4].map((id) => ({ id, rep_face_id: id * 10, size: 5, members: [id * 10] })) });
  const s = (kind, a, b, similarity, confidence) => ({ kind, a, b, similarity, confidence });
  const list = [s('groups', 1, 2, 0.7, 'medium'), s('people', 2, 3, 0.6, 'high'), s('group_person', 3, 1, 0.9, 'high'), s('groups', 3, 4, 0.5, 'low'),
    s('groups', 1, 9, 0.99, 'high'), s('group_person', 4, 4, 0.99, 'high'), s('people', 1, 4, 0.99, 'high'), s('groups', 2, 2, 0.99, 'high'), s('other', 1, 2, 0.99, 'high'),
    s('people', 2, 3, 0.6, 'high'), s('groups', 1, 3, 0.95, 'medium')];
  const rows = suggestRows(list, d);
  assert.deepEqual(rows.map((r) => r.key), ['group_person:3:1', 'people:2:3', 'groups:1:3', 'groups:1:2']);   // high first, then by similarity
  assert.deepEqual(suggestRows(list, d, new Set(['people:2:3'])).map((r) => r.key), ['group_person:3:1', 'groups:1:3', 'groups:1:2']);
  assert.deepEqual(suggestRows(list, data({ people: d.people, groups: d.groups.filter((g) => g.id !== 3) })).map((r) => r.key), ['people:2:3', 'groups:1:2']);   // a named group is gone
  const cap = capRows(rows, 3);
  assert.deepEqual([cap.shown.length, cap.more], [3, 1]);
  assert.deepEqual(capRows(rows.slice(0, 2), 3), { shown: rows.slice(0, 2), more: 0 });
  assert.deepEqual([...lookAlikes(rows)], [['g:3', 1]]);
});
