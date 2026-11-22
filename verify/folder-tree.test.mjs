// SPDX-License-Identifier: GPL-3.0-or-later
// Engine-free: the folder tree's logic shared by the Folders list and the Catalog (folderTreeModel.ts: ARIA
// numbers, keyboard map, type-ahead, the window of rows) and the Catalog's own (catalogTree.ts: merging the
// backend's replies, flattening to rows, keys that survive a remount, count modes, the 2 s throttle).
import test from 'node:test';
import assert from 'node:assert/strict';
import './load-app-ts.mjs';

const M = await import('../app/src/folderTreeModel.ts');
const C = await import('../app/src/catalogTree.ts');

const row = (key, parentKey, depth, over = {}) => ({ key, parentKey, depth, kind: 'folder', path: key, name: key.split('/').pop(), hasChildren: false, open: false, data: {}, ...over });
const sample = () => [
  row('a', null, 0, { hasChildren: true, open: true }),
  row('a/b', 'a', 1, { hasChildren: true, open: false }),
  row('a/c', 'a', 1),
  row('d', null, 0, { hasChildren: true, open: false }),
];

test('place numbers levels, set sizes and positions among the items of one parent; headings are not items', () => {
  const rows = [row('h', null, 0, { kind: 'heading' }), ...sample(), row('m', 'a', 1, { kind: 'message' })];
  const p = M.place(rows);
  assert.deepEqual(p.map((r) => [r.key, r.level, r.setSize, r.posInSet]), [
    ['h', 1, 0, 0], ['a', 1, 2, 1], ['a/b', 2, 2, 1], ['a/c', 2, 2, 2], ['d', 1, 2, 2], ['m', 2, 0, 0]]);
});

test('arrow keys: down and up walk the visible rows, home and end go to the ends', () => {
  const rows = sample();
  assert.deepEqual(M.keyMove(rows, 'a', 'ArrowDown'), { kind: 'focus', key: 'a/b' });
  assert.deepEqual(M.keyMove(rows, 'a/b', 'ArrowUp'), { kind: 'focus', key: 'a' });
  assert.equal(M.keyMove(rows, 'a', 'ArrowUp'), null);
  assert.equal(M.keyMove(rows, 'd', 'ArrowDown'), null);
  assert.deepEqual(M.keyMove(rows, 'a/c', 'Home'), { kind: 'focus', key: 'a' });
  assert.deepEqual(M.keyMove(rows, 'a', 'End'), { kind: 'focus', key: 'd' });
});

test('right opens a closed folder, then moves to its first child; left closes an open one, then moves to the parent', () => {
  const rows = sample();
  assert.deepEqual(M.keyMove(rows, 'd', 'ArrowRight'), { kind: 'toggle', key: 'd', open: true });
  assert.deepEqual(M.keyMove(rows, 'a', 'ArrowRight'), { kind: 'focus', key: 'a/b' });
  assert.equal(M.keyMove(rows, 'a/c', 'ArrowRight'), null, 'a leaf has nothing to open');
  assert.deepEqual(M.keyMove(rows, 'a', 'ArrowLeft'), { kind: 'toggle', key: 'a', open: false });
  assert.deepEqual(M.keyMove(rows, 'a/c', 'ArrowLeft'), { kind: 'focus', key: 'a' });
  assert.equal(M.keyMove(rows, 'd', 'ArrowLeft'), null, 'a closed top row has no parent');
});

test('in a right-to-left tree Left and Right swap', () => {
  const rows = sample();
  assert.deepEqual(M.keyMove(rows, 'd', 'ArrowLeft', true), { kind: 'toggle', key: 'd', open: true });
  assert.deepEqual(M.keyMove(rows, 'a', 'ArrowRight', true), { kind: 'toggle', key: 'a', open: false });
  assert.deepEqual(M.keyMove(rows, 'a/c', 'ArrowRight', true), { kind: 'focus', key: 'a' });
  assert.equal(M.keyMove(rows, 'd', 'ArrowRight', true), null);
});

test('enter and space open the folder, the menu key and shift+F10 ask for the menu, * opens the closed siblings', () => {
  const rows = sample();
  assert.deepEqual(M.keyMove(rows, 'a/c', 'Enter'), { kind: 'open', key: 'a/c' });
  assert.deepEqual(M.keyMove(rows, 'a/c', ' '), { kind: 'open', key: 'a/c' });
  assert.deepEqual(M.keyMove(rows, 'a/c', 'ContextMenu'), { kind: 'menu', key: 'a/c' });
  assert.deepEqual(M.keyMove(rows, 'a/c', 'F10', false, true), { kind: 'menu', key: 'a/c' });
  assert.equal(M.keyMove(rows, 'a/c', 'F10', false, false), null);
  assert.deepEqual(M.closedSiblings(rows, 'a'), ['d']);
  assert.deepEqual(M.closedSiblings(rows, 'a/b'), ['a/b']);
});

test('headings and messages are skipped by the keys', () => {
  const rows = [row('h', null, 0, { kind: 'heading' }), row('a', null, 0), row('m', 'a', 1, { kind: 'message' }), row('b', null, 0)];
  assert.deepEqual(M.keyMove(rows, 'a', 'ArrowDown'), { kind: 'focus', key: 'b' });
  assert.deepEqual(M.keyMove(rows, 'b', 'Home'), { kind: 'focus', key: 'a' });
});

test('type-ahead jumps to the next row that starts with what was typed, wrapping, and a repeated letter cycles', () => {
  const rows = ['alpha', 'beta', 'Bravo', 'delta'].map((n) => row(n, null, 0));
  assert.equal(M.typeAhead(rows, 'alpha', 'b'), 'beta');
  assert.equal(M.typeAhead(rows, 'beta', 'b'), 'Bravo', 'a single key moves on');
  assert.equal(M.typeAhead(rows, 'Bravo', 'b'), 'beta', 'and wraps');
  assert.equal(M.typeAhead(rows, 'alpha', 'br'), 'Bravo', 'a longer word narrows');
  assert.equal(M.typeAhead(rows, 'Bravo', 'br'), 'Bravo', 'and may stay where it is');
  assert.equal(M.typeAhead(rows, 'alpha', 'bb'), 'beta', 'a repeated letter cycles');
  assert.equal(M.typeAhead(rows, 'alpha', 'zz'), null);
  assert.deepEqual(M.typedSoFar({ text: 'b', at: 1000 }, 'r', 1500), { text: 'br', at: 1500 });
  assert.deepEqual(M.typedSoFar({ text: 'b', at: 1000 }, 'r', 1900), { text: 'r', at: 1900 }, 'after 800 ms it starts again');
});

test('a short tree draws everything; a long one a window of rows with the scroll height of the whole list', () => {
  assert.deepEqual(M.windowRows(400, 0, 560), { from: 0, to: 400, before: 0, after: 0, keep: [] });
  const w = M.windowRows(110_101, 28 * 5000, 560, [0, 5003]);
  assert.equal(w.from, 5000 - M.OVERSCAN);
  assert.equal(w.to, 5000 + 20 + M.OVERSCAN);
  assert.ok(w.to - w.from < 60, `${w.to - w.from} rows drawn`);
  assert.equal(w.before + (w.to - w.from) * M.ROW_HEIGHT + w.after, 110_101 * M.ROW_HEIGHT);
  assert.deepEqual(w.keep, [0], 'the focused row stays mounted outside the window, one inside needs no help');
  assert.equal(M.scrollToShow(100, 0, 560), 100 * 28 + 28 - 560);
  assert.equal(M.scrollToShow(5, 0, 560), null);
  assert.equal(M.scrollToShow(2, 280, 560), 56);
});

// ---- the Catalog's side ----

const src = (id, name, over = {}, online = true) => ({
  source: { id, name, kind: 'nas', volume_key: 'smb:h/s', fs_type: 'cifs', host: 'h', host_hash: null, share: 's', mount_path: '/mnt/s', rel_root: name, enabled: true,
    treat_as_network: null, scan_policy: 'manual', include_subfolders: true, dir_skip: false, state: 'complete', error: null, last_scan_start: 1, last_scan_end: 2, last_ok: 2,
    created_at: 1, photos: 100, missing: 0, ...over },
  availability: online ? { state: 'Online', detail: { mount_point: '/mnt/s', abs_root: `/mnt/s/${name}` } } : { state: 'Offline', detail: { reason: 'x' } },
});
const n = (parent, name, direct, subtree, has = false, status = 'ok') => ({ parent, rel: parent ? `${parent}/${name}` : name, name, has_children: has, direct, subtree, status });
const reply = (nodes, over = {}) => ({ generation: 7, unchanged: false, root: { parent: '', rel: '', name: '', has_children: nodes.some((x) => x.parent === ''), direct: 1, subtree: 100, status: 'ok' }, nodes, ...over });

test('keys are stable across a remount: they name the source and the rel, not the mount path', () => {
  assert.equal(C.treeKey(3, '2019/Iceland'), 'src:3:2019/Iceland');
  assert.deepEqual(C.parseKey('src:3:2019/Iceland'), { sourceId: 3, rel: '2019/Iceland' });
  assert.deepEqual(C.parseKey('src:3:'), { sourceId: 3, rel: '' });
  assert.equal(C.parseKey('/mnt/x'), null);
  const tree = C.applyReply(undefined, [''], reply([n('', '2019', 5, 50, true)]));
  const at = (mount) => C.flatten({ sources: [src(1, 'A', { mount_path: mount })], jobs: {}, trees: new Map([[1, tree]]), expanded: new Set([C.treeKey(1, '')]), includeSub: true }).map((r) => r.key);
  assert.deepEqual(at('/mnt/s'), at('/Volumes/other'));
});

test('a reply replaces the asked folders only, an unchanged reply keeps everything, an asked folder with no children empties', () => {
  let t = C.applyReply(undefined, [''], reply([n('', 'a', 1, 11, true), n('', 'b', 2, 2)]));
  t = C.applyReply(t, ['', 'a'], reply([n('', 'a', 1, 11, true), n('', 'b', 2, 2), n('a', 'x', 10, 10)], { generation: 8 }));
  assert.equal(t.generation, 8);
  assert.deepEqual(t.kids.get('a').map((k) => k.rel), ['a/x']);
  const same = C.applyReply(t, ['', 'a'], { generation: 8, unchanged: true, root: null, nodes: [] });
  assert.equal(same, t);
  const gone = C.applyReply(t, ['', 'a'], reply([n('', 'a', 1, 1), n('', 'b', 2, 2)], { generation: 9 }));
  assert.deepEqual(gone.kids.get('a'), []);
  assert.deepEqual(C.parentsToAsk(1, new Set([C.treeKey(1, ''), C.treeKey(1, 'a'), C.treeKey(2, 'z')])), ['', 'a']);
});

test('flatten: a source is the root of its tree, open folders show their children, counts follow Include subfolders', () => {
  let t = C.applyReply(undefined, ['', '2019'], reply([n('', '2018', 0, 30, true), n('', '2019', 4, 10, true), n('2019', 'Iceland', 6, 6)]));
  const input = (over = {}) => ({ sources: [src(1, 'Archive')], jobs: {}, trees: new Map([[1, t]]), expanded: new Set([C.treeKey(1, ''), C.treeKey(1, '2019')]), includeSub: true, ...over });
  const rows = C.flatten(input());
  assert.deepEqual(rows.map((r) => [r.kind, r.name, r.depth, r.open, r.data.count]), [
    ['source', 'Archive', 0, true, 100], ['folder', '2018', 1, false, 30], ['folder', '2019', 1, true, 10], ['folder', 'Iceland', 2, false, 6]]);
  assert.equal(rows[3].path, '/mnt/s/Archive/2019/Iceland');
  assert.equal(rows[3].parentKey, 'src:1:2019');
  assert.deepEqual(C.flatten(input({ includeSub: false })).map((r) => r.data.count), [100, 0, 4, 6], 'direct photos only');
  assert.equal(C.flatten(input({ expanded: new Set() })).length, 1, 'closed: the source row alone');
  assert.equal(C.flatten(input({ expanded: new Set() }))[0].hasChildren, true);
});

test('offline sources show their tree, dimmed by data; a source never indexed has no chevron; an empty folder shows 0', () => {
  const t = C.applyReply(undefined, [''], reply([n('', 'DCIM', 0, 0)]));
  const rows = C.flatten({ sources: [src(1, 'SD', {}, false), src(2, 'New', { photos: 0, last_scan_end: null, state: 'new' })], jobs: {}, trees: new Map([[1, t], [2, C.emptyTree()]]),
    expanded: new Set([C.treeKey(1, ''), C.treeKey(2, '')]), includeSub: true }).filter((r) => r.kind !== 'heading');
  assert.deepEqual(rows.map((r) => [r.name, r.data.offline, r.hasChildren, r.data.count]), [['SD', true, true, 100], ['DCIM', true, false, 0], ['New', false, false, null]]);
  assert.equal(rows[2].open, false, 'never indexed stays shut, whatever was saved');
});

test('while a source is being indexed it stays open with a "finding folders" line until the first folder arrives', () => {
  const jobs = { 1: { pipeline: true, scan: { source_id: 1, job_id: 5, run_id: 1, phase: 'finding', dirs_done: 1, dirs_queued: 3, found: 10, new: 10, changed: 0, unchanged: 0, errors: 0, elapsed_s: 1, rate_per_s: 1, eta_s: null, current_dir: null, paused: false } } };
  const make = (tree) => C.flatten({ sources: [src(1, 'W', { state: 'indexing', photos: 0, last_scan_end: null })], jobs, trees: new Map([[1, tree]]), expanded: new Set([C.treeKey(1, '')]), includeSub: true });
  const before = make(C.emptyTree());
  assert.deepEqual(before.map((r) => [r.kind, r.depth]), [['source', 0], ['message', 1]]);
  assert.equal(before[0].open, true);
  const after = make(C.applyReply(undefined, [''], reply([n('', 'first', 3, 3)])));
  assert.deepEqual(after.map((r) => [r.kind, r.depth]), [['source', 0], ['folder', 1]], 'the folder takes the line\'s place: one row either way');
  assert.equal(before.length, after.length);
  assert.equal(before[0].data.count, 0, 'nothing is counted before the first folder');
  assert.equal(after[0].data.count, 100, 'while it scans the source shows the tree\'s own live total, not the last refresh of the list');
});

test('a folder whose own directory did not answer carries a status; two volumes get headings', () => {
  const t = C.applyReply(undefined, [''], reply([n('', 'Locked', 0, 0, false, 'denied'), n('', 'Slow', 0, 0, false, 'stalled'), n('', 'Fine', 1, 1)]));
  const rows = C.flatten({ sources: [src(1, 'A')], jobs: {}, trees: new Map([[1, t]]), expanded: new Set([C.treeKey(1, '')]), includeSub: true });
  assert.deepEqual(rows.slice(1).map((r) => r.data.folderStatus), ['denied', 'stalled', '']);
  const two = C.flatten({ sources: [src(1, 'A'), src(2, 'B', { host: null, volume_key: 'uuid:1', mount_path: '/mnt/x', kind: 'drive', share: null }, false)], jobs: {}, trees: new Map(), expanded: new Set(), includeSub: true });
  assert.deepEqual(two.map((r) => r.kind), ['heading', 'source', 'heading', 'source']);
});

test('paths: a rel joins to the source root, and a path under a root gives its rel and the keys to open', () => {
  assert.equal(C.folderPath('/mnt/s/A', '2019/x'), '/mnt/s/A/2019/x');
  assert.equal(C.folderPath('/mnt/s/A/', '2019'), '/mnt/s/A/2019');
  assert.equal(C.folderPath('/mnt/s/A', ''), '/mnt/s/A');
  assert.equal(C.folderPath('C:\\Photos', '2019/x'), 'C:\\Photos\\2019\\x');
  assert.equal(C.relUnder('/mnt/s/A', '/mnt/s/A/2019/x/'), '2019/x');
  assert.equal(C.relUnder('/mnt/s/A', '/mnt/s/A'), '');
  assert.equal(C.relUnder('/mnt/s/A', '/mnt/s/AB'), null);
  assert.equal(C.relUnder('C:\\Photos', 'C:\\Photos\\2019'), '2019');
  assert.deepEqual(C.revealKeys(4, '2019/Iceland/Day 1'), ['src:4:', 'src:4:2019', 'src:4:2019/Iceland']);
  assert.deepEqual(C.revealKeys(4, ''), ['src:4:']);
  assert.equal(C.countOf({ direct: 2, subtree: 9 }, true), 9);
  assert.equal(C.countOf({ direct: 2, subtree: 9 }, false), 2);
});

test('toggled keeps a closed set as it was, so a remount restores the open folders', () => {
  const s = M.toggled(new Set(['a']), 'b', true);
  assert.deepEqual([...s].sort(), ['a', 'b']);
  assert.deepEqual([...M.toggled(s, 'a')], ['b']);
  assert.deepEqual([...M.toggled(s, 'zzz', false)].sort(), ['a', 'b']);
});

test('the scan tick is throttled to one ask in 2 s: the first at once, the last after the wait, nothing when idle', () => {
  let now = 0, asks = 0;
  const timers = [];
  const clock = { now: () => now, later: (f, ms) => { const h = { f, at: now + ms }; timers.push(h); return h; }, cancel: (h) => { timers.splice(timers.indexOf(h), 1); } };
  const advance = (ms) => { now += ms; for (const h of [...timers]) if (h.at <= now) { timers.splice(timers.indexOf(h), 1); h.f(); } };
  const tick = C.throttle(() => { asks++; }, 2000, clock);
  assert.equal(timers.length, 0, 'no timer before anything scans');
  tick(); assert.equal(asks, 1, 'the leading call runs at once');
  now = 100; tick(); now = 500; tick(); now = 900; tick();
  assert.equal(asks, 1);
  assert.equal(timers.length, 1, 'one trailing call is waiting');
  advance(1100); assert.equal(asks, 2, 'the trailing call runs at the end of the window');
  assert.equal(timers.length, 0, 'and leaves no timer behind');
  advance(5000); tick(); assert.equal(asks, 3, 'after a quiet spell the next call is immediate again');
  now += 10; tick(); tick.cancel(); assert.equal(timers.length, 0);
});
