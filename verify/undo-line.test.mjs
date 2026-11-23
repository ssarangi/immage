// SPDX-License-Identifier: GPL-3.0-or-later
// Engine-free: the Keywording card's inline Undo line (undoLine.ts): what a removal took off, how removals merge,
// and the 5 s timer that pauses while the pointer or the focus is on the line (driven by a fake clock).
import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire, registerHooks } from 'node:module';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
const require = createRequire(new URL('../app/package.json', import.meta.url));
const { transformSync } = await import(require.resolve('rolldown/experimental'));
registerHooks({
  load(url, ctx, next) {
    if (!url.endsWith('/undoLine.ts')) return next(url, ctx);
    const path = fileURLToPath(url);
    return { format: 'module', source: transformSync(path, readFileSync(path, 'utf8')).code, shortCircuit: true };
  },
});
const { createUndoTimer, mergeUndo, removalItems, undoSummary, UNDO_MS } = await import('../app/src/undoLine.ts');

function fake() {
  let t = 0, id = 0; const timers = new Map();
  return {
    clock: { now: () => t, set: (fn, ms) => { timers.set(++id, { at: t + ms, fn }); return id; }, clear: (h) => timers.delete(h) },
    advance(ms) {
      const to = t + ms;
      for (;;) {
        const next = [...timers].filter(([, v]) => v.at <= to).sort((a, b) => a[1].at - b[1].at)[0];
        if (!next) break;
        timers.delete(next[0]); t = next[1].at; next[1].fn();
      }
      t = to;
    },
  };
}

test('a removal lists, per photo, only what Auto-keywords added', () => {
  const rows = [{ path: 'a', keywords: ['own', 'sea', 'sky'] }, { path: 'b', keywords: ['sea'] }, { path: 'c', keywords: ['sea'] }];
  const ai = new Map([['a', new Set(['sea', 'sky'])], ['b', new Set(['sea'])]]);
  assert.deepEqual(removalItems(rows, ai, ['sea']), [{ path: 'a', names: ['sea'] }, { path: 'b', names: ['sea'] }]);
  assert.deepEqual(removalItems(rows, ai, []), [{ path: 'a', names: ['sea', 'sky'] }, { path: 'b', names: ['sea'] }]);
  assert.deepEqual(removalItems(rows, ai, ['own']), [], 'the user\'s own keyword is never listed');
});

test('removals while the line is up join it; another photo starts a new one', () => {
  const a = mergeUndo(null, 'k', [{ path: 'a', names: ['sea'] }]);
  const b = mergeUndo(a, 'k', [{ path: 'a', names: ['sky'] }, { path: 'b', names: ['sea'] }]);
  assert.deepEqual(b.items, [{ path: 'a', names: ['sea', 'sky'] }, { path: 'b', names: ['sea'] }]);
  assert.deepEqual(undoSummary(a), { n: 1, first: 'sea' });
  assert.equal(undoSummary(b).n, 2);
  assert.deepEqual(mergeUndo(b, 'other', [{ path: 'c', names: ['x'] }]).items, [{ path: 'c', names: ['x'] }]);
  assert.equal(mergeUndo(b, 'other', []), null);
});

test('the line expires after 5 s, once', () => {
  const f = fake(); let n = 0;
  const t = createUndoTimer(() => n++, UNDO_MS, f.clock);
  t.start(); f.advance(4999); assert.equal(n, 0);
  f.advance(1); assert.equal(n, 1);
  f.advance(20000); assert.equal(n, 1);
});

test('hover or focus holds it; it resumes with at least 2 s left when the last hold goes', () => {
  const f = fake(); let n = 0;
  const t = createUndoTimer(() => n++, UNDO_MS, f.clock);
  t.start(); f.advance(4500);
  t.hold('hover'); t.hold('focus'); f.advance(60000); assert.equal(n, 0, 'held');
  t.release('hover'); f.advance(60000); assert.equal(n, 0, 'still held by focus');
  t.release('focus'); assert.ok(t.running);
  f.advance(1999); assert.equal(n, 0);
  f.advance(1); assert.equal(n, 1);
  // plenty left: the rest is kept, not reset
  t.start(); f.advance(1000); t.hold('hover'); t.release('hover');
  f.advance(3999); assert.equal(n, 1); f.advance(1); assert.equal(n, 2);
});

test('a new removal restarts the countdown, also while held; cancel stops it', () => {
  const f = fake(); let n = 0;
  const t = createUndoTimer(() => n++, UNDO_MS, f.clock);
  t.start(); f.advance(4000); t.start(); f.advance(4000); assert.equal(n, 0);
  f.advance(1000); assert.equal(n, 1);
  t.start(); t.hold('hover'); t.start(); assert.ok(!t.running, 'restarted while held waits'); t.release('hover'); assert.ok(t.running);
  t.cancel(); f.advance(60000); assert.equal(n, 1); assert.ok(!t.held);
  t.hold('hover'); assert.ok(!t.held, 'a hold with no line is ignored');
});
