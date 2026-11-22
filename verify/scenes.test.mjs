// SPDX-License-Identifier: GPL-3.0-or-later
// scenes.ts: the timeline model, frame and scene state, the virtual list's heights and window,
// the keyboard map and the camera line. Pure functions; no engine, no DOM.
import test from 'node:test';
import assert from 'node:assert/strict';
import {
  CARD_H, DAY_H, BLOCK_H, GAP, GREEN, ROW_H, apertureText, buildModel, captureText, cameraLine, columnsFor, dayOf, decisionPatch,
  filmstripOrder, frameKey, frameState, layout, listSignature, moveFocus, requestItems, rowOfScene, sceneState, sceneStats, scoreText,
  shutterText, timelineKey, visibleSceneIndexes, windowRows,
} from '../app/src/scenes.ts';
import { cullingKey } from '../app/src/culling.ts';

const T = Date.UTC(2026, 2, 5, 7, 10, 0);
const build = (blocks) => buildModel({ blocks: blocks.map((scenes, i) => ({ startMs: T + i * 86400000, endMs: T + i * 86400000 + 1000, noTime: false, approx: false,
  scenes: scenes.map((frames) => ({ frames, unhashed: 0 })) })) });
const flags = (path, o = {}) => ({ path, version: 0, rejected: false, labels: 0, rating: 0, ...o });

test('the model numbers scenes across blocks and names each block by day', () => {
  const m = build([[[0, 1], [2]], [[3, 4, 5]]]);
  assert.equal(m.scenes.length, 3);
  assert.deepEqual(m.scenes.map((s) => s.n), [1, 2, 3]);
  assert.deepEqual(m.blocks.map((b) => [b.first, b.count, b.frames]), [[0, 2, 3], [2, 1, 3]]);
  assert.equal(m.blocks[0].day, '2026-03-05');
  assert.equal(m.blocks[1].day, '2026-03-06');
  assert.equal(m.frameCount, 6);
  assert.equal(dayOf(T), '2026-03-05');
});

test('capture text keeps the camera fraction only for the same second, else falls back to the file time', () => {
  assert.deepEqual(captureText({ datetime: '2026:03:05 07:10:04', capture_time_raw: '2026:03:05 07:10:04.123456' }), { takenAt: '2026:03:05 07:10:04.123456', fromFile: false });
  assert.deepEqual(captureText({ datetime: '2026:03:05 07:10:09', capture_time_raw: '2026:03:05 07:10:04.5' }), { takenAt: '2026:03:05 07:10:09', fromFile: false });
  assert.deepEqual(captureText({ datetime: '', file_datetime: '2026:03:05 09:00:00' }), { takenAt: '2026:03:05 09:00:00', fromFile: true });
  assert.deepEqual(captureText({}), { takenAt: '', fromFile: false });
  const items = requestItems([{ path: '/a.jpg', filename: 'a.jpg', exif: { datetime: '2026:03:05 07:10:04' } }]);
  assert.deepEqual(items, [{ path: '/a.jpg', version: 0, name: 'a.jpg', takenAt: '2026:03:05 07:10:04', fromFile: false }]);
});

test('the list signature ignores flags and changes with the photos', () => {
  const a = { path: '/a.jpg', filename: 'a', exif: { datetime: 'x' }, rating: 1 }, b = { path: '/b.jpg', filename: 'b', exif: {} };
  const s = listSignature([a, b]);
  assert.equal(listSignature([{ ...a, rating: 5, rejected: true }, b]), s);
  assert.notEqual(listSignature([a]), s);
  assert.notEqual(listSignature([b, a]), s);
});

test('a frame is accepted only with both the green label and Scenes\' own mark', () => {
  const key = frameKey('/a.jpg', 0);
  assert.equal(frameState(flags('/a.jpg', { labels: GREEN }), new Set()), 'undecided');
  assert.equal(frameState(flags('/a.jpg'), new Set([key])), 'undecided');
  assert.equal(frameState(flags('/a.jpg', { labels: GREEN }), new Set([key])), 'accepted');
  assert.equal(frameState(flags('/a.jpg', { labels: GREEN, rejected: true }), new Set([key])), 'rejected');
});

test('the best frame and the score show only once every frame is scored', () => {
  const m = build([[[0, 1, 2]]]);
  const ctx = (quality, entries) => ({ entries, quality, accepted: new Set() });
  const entries = [flags('/0'), flags('/1', { rejected: true }), flags('/2')];
  let s = sceneStats(m.scenes[0], ctx([0.5, null, 0.9], entries));
  assert.equal(s.best, null); assert.equal(s.bestQuality, null); assert.equal(s.shown, 0); assert.equal(s.scored, 2);
  s = sceneStats(m.scenes[0], ctx([0.5, 0.7, 0.9], entries));
  assert.equal(s.best, 2); assert.equal(s.shown, 2); assert.equal(scoreText(s.bestQuality), 90);
  assert.deepEqual([s.accepted, s.rejected, s.undecided], [0, 1, 2]);
  assert.equal(sceneState(s), 'mixed');
  assert.equal(sceneState({ accepted: 0, rejected: 0, undecided: 3, frames: 3 }), 'undecided');
  assert.equal(sceneState({ accepted: 3, rejected: 0, undecided: 0, frames: 3 }), 'accepted');
  assert.equal(sceneState({ accepted: 0, rejected: 3, undecided: 0, frames: 3 }), 'rejected');
  // ties keep the earlier frame
  s = sceneStats(m.scenes[0], ctx([0.8, 0.8, 0.8], entries));
  assert.equal(s.best, 0);
});

test('the filmstrip is sharpest first only when every frame is scored', () => {
  const q = [0.2, 0.9, 0.9, 0.5];
  assert.deepEqual(filmstripOrder([0, 1, 2, 3], q, true), [1, 2, 3, 0]);
  assert.deepEqual(filmstripOrder([0, 1, 2, 3], q, false), [0, 1, 2, 3]);
  assert.deepEqual(filmstripOrder([0, 1, 2, 3], [0.2, null, 0.9, 0.5], true), [0, 1, 2, 3]);
});

test('a decision writes the green label for accept and takes back only what Scenes wrote', () => {
  const e = flags('/a.jpg', { labels: 1 });
  let p = decisionPatch('accepted', e, new Set(), false);
  assert.deepEqual(p, { fields: { rejected: false, labels: 1 | GREEN }, accepted: true });
  p = decisionPatch('accepted', { ...e, rating: 0 }, new Set(), true);
  assert.equal(p.fields.rating, 1);
  assert.equal(decisionPatch('accepted', { ...e, rating: 3 }, new Set(), true).fields.rating, undefined);
  const ours = new Set([frameKey('/a.jpg', 0)]);
  assert.equal(decisionPatch('undecided', flags('/a.jpg', { labels: GREEN | 1 }), ours, false).fields.labels, 1);
  // a green label the user chose by hand stays
  assert.equal(decisionPatch('undecided', flags('/a.jpg', { labels: GREEN }), new Set(), false).fields.labels, GREEN);
  p = decisionPatch('rejected', flags('/a.jpg', { labels: GREEN }), ours, false);
  assert.deepEqual(p, { fields: { rejected: true, labels: 0 }, accepted: false });
});

test('row heights come from counts alone, so nothing moves when data arrives', () => {
  const m = build([[[0], [1, 2], [3, 4], [5], [6]], [[7, 8]]]);
  const width = 1200;
  const cols = columnsFor(width);
  assert.ok(cols >= 3);
  const lay = layout(m, width, false, new Set());
  // day, block, cards x ceil(5/cols); day, block, cards
  const first = Math.ceil(5 / cols);
  assert.equal(lay.total, DAY_H + BLOCK_H + first * ROW_H + DAY_H + BLOCK_H + ROW_H);
  for (let i = 1; i < lay.rows.length; i++) assert.equal(lay.rows[i].top, lay.rows[i - 1].top + lay.rows[i - 1].height);
  assert.equal(ROW_H, CARD_H + GAP);
  // folding a block removes its cards and keeps its header
  const folded = layout(m, width, false, new Set([0]));
  assert.equal(folded.total, lay.total - first * ROW_H + GAP);
  // hiding single-frame scenes drops them from the rows
  assert.deepEqual(visibleSceneIndexes(m, m.blocks[0], true), [1, 2]);
  assert.ok(layout(m, width, true, new Set()).total <= lay.total);
});

test('20,000 scenes: the layout is quick and only a few rows mount', () => {
  const scenes = [];
  for (let b = 0; b < 400; b++) scenes.push(Array.from({ length: 50 }, (_, i) => [b * 400 + i * 8, b * 400 + i * 8 + 1]));
  const m = build(scenes);
  assert.equal(m.scenes.length, 20000);
  const started = performance.now();
  const lay = layout(m, 1440, false, new Set());
  assert.ok(performance.now() - started < 250, 'layout took too long');
  for (const top of [0, 12345, 400000, lay.total - 900]) {
    const [a, b] = windowRows(lay.rows, top, 900);
    assert.ok(b - a <= 14, `mounted ${b - a} rows at ${top}`);
    assert.ok(lay.rows[a].top <= top || a === 0);
  }
  const [a, b] = windowRows(lay.rows, lay.total - 900, 900);
  assert.equal(b, lay.rows.length);
  assert.ok(rowOfScene(lay, m, 19999, false));
});

test('windowRows covers the viewport and overscans', () => {
  const m = build([[[0], [1], [2], [3], [4], [5], [6], [7], [8], [9]]]);
  const lay = layout(m, 300, false, new Set());   // one column
  const [a, b] = windowRows(lay.rows, 500, 300, 0);
  for (let i = a; i < b; i++) assert.ok(lay.rows[i].top + lay.rows[i].height > 500 && lay.rows[i].top < 800);
  const [c, d] = windowRows(lay.rows, 500, 300, 2);
  assert.ok(c <= a && d >= b);
  assert.deepEqual(windowRows([], 0, 100), [0, 0]);
});

test('keyboard: arrows wrap across blocks, Home/End, PageUp/PageDown, mirrored in RTL', () => {
  const m = build([[[0], [1], [2], [3], [4]], [[5], [6]]]);
  const vis = m.scenes.map((_, i) => i);
  const go = (from, key, rtl = false) => moveFocus(vis, m, from, key, 3, rtl);
  assert.equal(go(0, 'ArrowRight'), 1);
  assert.equal(go(4, 'ArrowRight'), 5);          // wraps into the next block
  assert.equal(go(0, 'ArrowLeft'), 0);
  assert.equal(go(0, 'ArrowRight', true), 0);   // RTL: right goes back
  assert.equal(go(1, 'ArrowLeft', true), 2);
  assert.equal(go(1, 'ArrowDown'), 4);           // same column, next row
  assert.equal(go(0, 'ArrowDown'), 3);
  assert.equal(go(3, 'ArrowDown'), 5);           // last row: the next block's first row
  assert.equal(go(4, 'ArrowDown'), 6);           // column 1, clamped to the block's size
  assert.equal(go(5, 'ArrowUp'), 3);             // up into the previous block's last row
  assert.equal(go(6, 'ArrowUp'), 4);
  assert.equal(go(3, 'ArrowUp'), 0);
  assert.equal(go(3, 'Home'), 0);
  assert.equal(go(3, 'End'), 6);
  assert.equal(go(3, 'PageDown'), 5);
  assert.equal(go(1, 'PageDown'), 5);
  assert.equal(go(6, 'PageUp'), 5);              // to the block's start, then the one before
  assert.equal(go(5, 'PageUp'), 0);
});

test('keyboard: the timeline map', () => {
  assert.deepEqual(timelineKey('Enter', false, false, false), { type: 'open' });
  assert.deepEqual(timelineKey(' ', false, false, false), { type: 'open' });
  assert.deepEqual(timelineKey('Escape', false, false, false), { type: 'back' });
  assert.deepEqual(timelineKey('a', false, false, false), { type: 'decide', state: 'accepted', whole: false });
  assert.deepEqual(timelineKey('X', true, false, false), { type: 'decide', state: 'rejected', whole: true });
  assert.deepEqual(timelineKey('u', true, false, false), { type: 'decide', state: 'undecided', whole: true });
  // on a block header, Left folds and Right unfolds; RTL swaps them
  assert.deepEqual(timelineKey('ArrowLeft', false, true, false), { type: 'fold', fold: true });
  assert.deepEqual(timelineKey('ArrowRight', false, true, false), { type: 'fold', fold: false });
  assert.deepEqual(timelineKey('ArrowLeft', false, true, true), { type: 'fold', fold: false });
  assert.equal(timelineKey('ArrowLeft', false, false, false).type, 'move');
  assert.equal(timelineKey('q', false, false, false).type, 'none');
});

test('keyboard: the lightbox keys A, U and N exist only in the scene view', () => {
  const base = { photo: 1, keywordsOpen: false, writing: false, blocked: false, modalOpen: false, hasTarget: true, hasCurrent: true };
  for (const k of ['a', 'u', 'n']) {
    assert.equal(cullingKey(k, base).kind, 'swallow', `${k} outside Scenes only swallows`);
    assert.equal(cullingKey(k, { ...base, scene: false }).kind, 'swallow');
  }
  assert.deepEqual(cullingKey('a', { ...base, scene: true }).action, { type: 'decide', state: 'accepted' });
  assert.deepEqual(cullingKey('u', { ...base, scene: true }).action, { type: 'decide', state: 'undecided' });
  assert.deepEqual(cullingKey('n', { ...base, scene: true }).action, { type: 'scene', by: 1 });
  assert.deepEqual(cullingKey('n', { ...base, scene: true, shift: true }).action, { type: 'scene', by: -1 });
  // the keys that were there still are
  assert.deepEqual(cullingKey('x', { ...base, scene: true }).action, { type: 'toggleReject' });
  assert.deepEqual(cullingKey('arrowright', { ...base, scene: true }).action, { type: 'step', by: 1 });
  // no scene, no photo open: ignored, so the grid's own keys keep working
  assert.equal(cullingKey('a', { ...base, photo: null, scene: true }).kind, 'ignore');
});

test('the camera line collapses by priority and formats exposure', () => {
  const full = { model: 'ILCE-7M4', lens: 'FE 70-200mm F4', focal_length: 200, aperture: 4, exposure: 0.001, iso: 400 };
  assert.deepEqual(cameraLine(full), ['ILCE-7M4', '200 mm f/4 1/1000', 'ISO 400', 'FE 70-200mm F4']);
  assert.deepEqual(cameraLine(full, 2), ['ILCE-7M4', '200 mm f/4 1/1000']);
  assert.deepEqual(cameraLine({}), []);
  assert.deepEqual(cameraLine({ iso: 100 }), ['ISO 100']);
  assert.equal(shutterText(2), '2 s'); assert.equal(shutterText(0.5), '0.5 s'); assert.equal(shutterText(1 / 60), '1/60'); assert.equal(shutterText(0), '');
  assert.equal(apertureText(5.6), 'f/5.6'); assert.equal(apertureText(0), '');
});
