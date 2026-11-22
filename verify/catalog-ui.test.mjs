// SPDX-License-Identifier: GPL-3.0-or-later
// Engine-free: the Catalog page's model (catalogModel.ts: the one state a source shows, the phases of
// an index, the time left, which source a folder belongs to) and the event reducer that chains the
// three jobs of an index (catalogEvents.ts).
import test from 'node:test';
import assert from 'node:assert/strict';
import './load-app-ts.mjs';

const M = await import('../app/src/catalogModel.ts');
const E = await import('../app/src/catalogEvents.ts');

const source = (over = {}, online = true) => ({
  source: { id: 1, name: 'Archive', kind: 'nas', volume_key: 'smb:h/s', fs_type: 'cifs', host: 'h', host_hash: null, share: 's', mount_path: '/mnt/s',
    rel_root: 'Archive', enabled: true, treat_as_network: null, scan_policy: 'manual', include_subfolders: true, dir_skip: false, state: 'complete',
    error: null, last_scan_start: 1, last_scan_end: 2, last_ok: 2, created_at: 1, photos: 10, missing: 0, ...over },
  availability: online ? { state: 'Online', detail: { mount_point: '/mnt/s', abs_root: '/mnt/s/Archive' } } : { state: 'Offline', detail: { reason: 'not mounted' } },
});
const scanP = (o = {}) => ({ source_id: 1, job_id: 5, run_id: 1, phase: 'finding', dirs_done: 1, dirs_queued: 3, found: 100, new: 100, changed: 0, unchanged: 0, errors: 0, elapsed_s: 3, rate_per_s: 30, eta_s: null, current_dir: null, paused: false, ...o });
const metaP = (o = {}) => ({ source_id: 1, job_id: 6, phase: 'reading', total: 200, done: 50, failed: 0, unparsed: 0, skipped: 0, bytes_read: 0, elapsed_s: 5, rate_per_s: 10, eta_s: [60, 120], workers: 2, p95_ms: 1, paused: false, ...o });
const thumbP = (o = {}) => ({ source_id: 1, job_id: 7, phase: 'making', total: 200, done: 20, failed: 0, deferred: 0, skipped: 0, bytes_read: 0, elapsed_s: 5, rate_per_s: 4, eta_s: null, workers: 2, decoders: 2, p95_ms: 1, paused: false, yielding: false, ...o });

test('a source shows one state, running work first, then where it is, then how it ended', () => {
  assert.equal(M.sourceState(source(), undefined), 'online');
  assert.equal(M.sourceState(source(), { scan: scanP() }), 'indexing');
  assert.equal(M.sourceState(source(), { meta: metaP({ paused: true }) }), 'paused');
  assert.equal(M.sourceState(source(), { pipeline: true }), 'indexing', 'between two jobs it is still indexing');
  assert.equal(M.sourceState(source({}, false), undefined), 'offline');
  assert.equal(M.sourceState(source({}, false), { scan: scanP() }), 'indexing', 'a running job outranks the mount');
  assert.equal(M.sourceState(source({ error: 'x' }), undefined), 'attention');
  assert.equal(M.sourceState(source({ state: 'incomplete' }), undefined), 'attention');
  assert.equal(M.sourceState(source({ state: 'interrupted' }), undefined), 'paused');
  assert.equal(M.sourceState(source({ photos: 0, last_scan_end: null }), undefined), 'never');
  const asleep = { ...source(), availability: { state: 'Asleep', detail: { mount_point: '/mnt/s' } } };
  assert.equal(M.sourceState(asleep, undefined), 'asleep');
});

test('a folder is none, a source root, or inside one; an offline source keeps its path', () => {
  const list = [source(), { ...source({ id: 2, name: 'Gone', rel_root: 'Gone' }, false), source: { ...source().source, id: 2, name: 'Gone', rel_root: 'Gone' } }];
  assert.deepEqual(M.folderCatalog('/mnt/s/Archive', list).kind, 'root');
  assert.deepEqual(M.folderCatalog('/mnt/s/Archive/', list).kind, 'root', 'a trailing slash is the same folder');
  assert.deepEqual(M.folderCatalog('/mnt/s/Archive/2016/06', list).kind, 'inside');
  assert.deepEqual(M.folderCatalog('/mnt/s/Archive2', list).kind, 'none', 'a sibling with the same prefix is not inside');
  assert.deepEqual(M.folderCatalog('/mnt/s/Gone', list).kind, 'root', 'an offline source is found by where it was');
  assert.equal(M.sourceRoot(list[1]), '/mnt/s/Gone');
  assert.deepEqual(M.folderCatalog('/elsewhere', list).kind, 'none');
});

test('a folded card sums the work of every source, and a warning rides beside it', () => {
  const two = (a, b) => [source({ id: 1 }), { ...source({ id: 2, name: 'B' }), source: { ...source({ id: 2, name: 'B' }).source, ...b } }];
  const idle = M.catalogSummary([source()], {});
  assert.deepEqual(idle, { state: 'idle', fraction: null, done: 0, total: null, thumbs: false, attention: 0 });
  assert.equal(M.catalogSummary([source({ state: 'failed' })], {}).attention, 1);
  assert.equal(M.catalogSummary([source({ state: 'failed' })], {}).state, 'idle', 'a problem alone is not work');
  const finding = M.catalogSummary([source()], { 1: { scan: scanP() } });
  assert.equal(finding.state, 'indexing'); assert.equal(finding.total, null); assert.equal(finding.fraction, null);
  const reading = M.catalogSummary([source()], { 1: { meta: metaP() } });
  assert.deepEqual([reading.state, reading.done, reading.total, reading.fraction, reading.thumbs], ['indexing', 50, 200, 0.25, false]);
  const thumbs = M.catalogSummary([source()], { 1: { pipeline: true, thumbs: thumbP() } });
  assert.deepEqual([thumbs.done, thumbs.total, thumbs.thumbs], [20, 200, true]);
  const sum = M.catalogSummary(two(), { 1: { meta: metaP() }, 2: { meta: metaP({ source_id: 2, done: 150 }) } });
  assert.deepEqual([sum.done, sum.total, sum.fraction], [200, 400, 0.5]);
  const paused = M.catalogSummary([source()], { 1: { meta: metaP({ paused: true }) } });
  assert.equal(paused.state, 'paused'); assert.equal(paused.done, 50);
  const mixed = M.catalogSummary(two(), { 1: { meta: metaP({ paused: true }) }, 2: { meta: metaP({ source_id: 2 }) } });
  assert.equal(mixed.state, 'indexing', 'running beats paused');
  const both = M.catalogSummary(two(0, { state: 'failed' }), { 1: { meta: metaP() } });
  assert.deepEqual([both.state, both.attention], ['indexing', 1]);
});

test('phases: finding has no total until it ends; reading and thumbnails follow; done is remembered', () => {
  let p = M.phases({ scan: scanP() }, undefined, 0);
  assert.deepEqual(p.map((x) => x.status), ['active', 'waiting', 'waiting']);
  assert.equal(p[0].total, null);
  assert.equal(M.overallFraction(p), null);
  p = M.phases({ meta: metaP() }, { scan: { found: 200 } }, 0);
  assert.deepEqual(p.map((x) => x.status), ['done', 'active', 'waiting']);
  assert.equal(M.overallFraction(p), 0.25);
  p = M.phases({ thumbs: thumbP() }, { scan: { found: 200 }, meta: { state: 'complete', done: 190, total: 200 } }, 0);
  assert.deepEqual(p.map((x) => x.status), ['done', 'done', 'active']);
  assert.equal(p[2].done, 20);
  p = M.phases(undefined, { scan: { found: 200 }, meta: { state: 'complete', done: 190, total: 200 }, thumbs: { state: 'complete', done: 190, total: 190 } }, 0);
  assert.deepEqual(p.map((x) => x.status), ['done', 'done', 'done']);
});

test('the time left is a range, "soon" under a minute, and nothing without a steady rate', () => {
  assert.deepEqual(M.etaOf(null), { kind: 'none' });
  assert.deepEqual(M.etaOf([20, 40]), { kind: 'soon' });
  assert.deepEqual(M.etaOf([360, 720]), { kind: 'range', lo: 360, hi: 720 });
  assert.deepEqual(M.durationText(45), { unit: 's', a: 45 });
  assert.deepEqual(M.durationText(540), { unit: 'min', a: 9 });
  assert.deepEqual(M.durationText(3900), { unit: 'h', a: 1, b: 5 });
});

test('problems are named in plain categories; ready needs every job complete', () => {
  assert.equal(M.problemOf(undefined), null);
  assert.equal(M.problemOf({ scan: { state: 'offline' } }), 'offline');
  assert.equal(M.problemOf({ scan: { state: 'complete' }, thumbs: { state: 'disk_full' } }), 'disk_full');
  assert.equal(M.problemOf({ scan: { state: 'failed' } }), 'failed');
  assert.equal(M.problemOf({ scan: { state: 'incomplete' } }), 'partial');
  assert.equal(M.problemOf({ scan: { state: 'complete' } }), null);
  assert.equal(M.isReady({ scan: { state: 'complete' }, meta: { state: 'complete' }, thumbs: { state: 'complete' } }), true);
  assert.equal(M.isReady({ scan: { state: 'complete' }, meta: { state: 'cancelled' } }), false);
});

test('sources group by volume in the order met; sizes read as one unit', () => {
  const a = source(), b = source({ id: 2 }), c = source({ id: 3, volume_key: 'uuid:x', kind: 'drive', host: null, share: null, mount_path: '/media/sd' }, false);
  const g = M.groupByVolume([a, c, b]);
  assert.deepEqual(g.map((x) => [x.key, x.items.length, x.online]), [['smb:h/s', 2, true], ['uuid:x', 1, false]]);
  assert.equal(g[1].name, 'sd');
  assert.deepEqual(M.sizeOf(500), { n: 500, unit: 'B' });
  assert.deepEqual(M.sizeOf(5 * 1024 ** 2), { n: 5, unit: 'MB' });
  assert.deepEqual(M.sizeOf(18.4 * 1024 ** 3), { n: 18.4, unit: 'GB' });
});

test('an index is find, read, thumbnails in turn; a failure or a stop ends it where it is', () => {
  let st = E.EMPTY, r;
  r = E.apply(st, { kind: 'begin', id: 1 }); st = r.state;
  assert.equal(st.jobs[1].pipeline, true);
  r = E.apply(st, { kind: 'scan-progress', p: scanP() }); st = r.state;
  assert.equal(M.sourceState(source(), st.jobs[1]), 'indexing');
  r = E.apply(st, { kind: 'scan-finished', s: { source_id: 1, state: 'complete', found: 200 } }); st = r.state;
  assert.deepEqual(r.next, { start: 'metadata', id: 1 });
  assert.equal(st.jobs[1].pipeline, true, 'still indexing between two jobs');
  assert.equal(st.jobs[1].scan, undefined);
  r = E.apply(st, { kind: 'metadata-progress', p: metaP() }); st = r.state;
  r = E.apply(st, { kind: 'metadata-finished', s: { source_id: 1, state: 'complete', done: 190, total: 200 } }); st = r.state;
  assert.deepEqual(r.next, { start: 'thumbnails', id: 1 });
  r = E.apply(st, { kind: 'thumbnails-finished', s: { source_id: 1, state: 'complete', done: 190, total: 190 } }); st = r.state;
  assert.deepEqual(r.next, { end: 1 });
  assert.equal(st.jobs[1].pipeline, false);
  assert.equal(M.isReady(st.ended[1]), true);
  assert.equal(Object.keys(E.tidy(st).jobs).length, 0, 'finished jobs leave no entry');

  // offline ends the chain and says so
  st = E.apply(E.EMPTY, { kind: 'begin', id: 1 }).state;
  r = E.apply(st, { kind: 'scan-finished', s: { source_id: 1, state: 'offline' } });
  assert.deepEqual(r.next, { end: 1 });
  assert.equal(M.problemOf(r.state.ended[1]), 'offline');
  // a metadata run that was cancelled does not start thumbnails
  st = E.apply(E.EMPTY, { kind: 'begin', id: 1 }).state;
  r = E.apply(st, { kind: 'metadata-finished', s: { source_id: 1, state: 'cancelled' } });
  assert.deepEqual(r.next, { end: 1 });
  // a finish for a source this page did not start (the command line) chains nothing
  r = E.apply(E.EMPTY, { kind: 'scan-finished', s: { source_id: 9, state: 'complete' } });
  assert.deepEqual(r.next, { end: 9 });
  // beginning again forgets how the last one ended
  st = { jobs: {}, ended: { 1: { scan: { state: 'failed' } } } };
  assert.equal(E.apply(st, { kind: 'begin', id: 1 }).state.ended[1], undefined);
});
