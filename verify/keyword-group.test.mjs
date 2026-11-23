// SPDX-License-Identifier: GPL-3.0-or-later
// Engine-free: the Keywording card's Auto-keywords group logic (keywordGroup.ts): its header state from the
// library status and the photo's own result, and the split of a photo's or a selection's keywords.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire, registerHooks } from 'node:module';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
const require = createRequire(new URL('../app/package.json', import.meta.url));
const { transformSync } = await import(require.resolve('rolldown/experimental'));
registerHooks({
  load(url, ctx, next) {
    if (!url.endsWith('/keywordGroup.ts')) return next(url, ctx);
    const path = fileURLToPath(url);
    return { format: 'module', source: transformSync(path, readFileSync(path, 'utf8')).code, shortCircuit: true };
  },
});
const { autoGroupState, groupRows } = await import('../app/src/keywordGroup.ts');
const on = { enabled: true, paused: false, busy: false, blocked: false };

test('the header state follows the library status and the photo\'s own result', () => {
  assert.equal(autoGroupState(null, 0), null);
  assert.equal(autoGroupState({ ...on, enabled: false }, 3), 'done');
  assert.equal(autoGroupState({ ...on, enabled: false }, 0), null);
  assert.equal(autoGroupState({ ...on, blocked: true }, 2), 'blocked');
  assert.equal(autoGroupState({ ...on, paused: true }, 0), 'paused');
  assert.equal(autoGroupState({ ...on, busy: true }, 4), 'done');
  // this photo has no result yet: analysing while the queue works, no group when it is idle
  assert.equal(autoGroupState({ ...on, busy: true }, 0, false), 'analysing');
  assert.equal(autoGroupState(on, 0, false), null);
  // the analysis has been through it and found nothing, whatever the queue is doing
  assert.equal(autoGroupState(on, 0, true), 'none');
  assert.equal(autoGroupState({ ...on, busy: true }, 0, true), 'none');
});

const row = (keywords, ai = []) => ({ keywords, ai: new Set(ai) });

test('one photo: its own keywords in order, Auto-keywords by confidence then name', () => {
  const conf = { sea: 90, beach: 90, dog: 60 };
  const g = groupRows([row(['family', 'dog', 'sea', 'Places|Paris', 'beach'], ['dog', 'sea', 'beach'])], (k) => conf[k] ?? 0);
  assert.deepEqual(g.own.map((k) => k.name), ['family', 'Places|Paris']);
  assert.deepEqual(g.auto.map((k) => k.name), ['beach', 'sea', 'dog']);
  assert.ok([...g.own, ...g.auto].every((k) => k.n === 1 && k.total === 1));
});

test('a selection: keywords on every photo first, then the ones on some, most photos first', () => {
  const g = groupRows([row(['a', 'x', 'y']), row(['a', 'y']), row(['a', 'z', 'y'])]);
  assert.deepEqual(g.own.map((k) => [k.name, k.n, k.total]), [['a', 3, 3], ['y', 3, 3], ['x', 1, 3], ['z', 1, 3]]);
  const h = groupRows([row(['a', 'b']), row(['b']), row(['c'])]);
  assert.deepEqual(h.own.map((k) => [k.name, k.n]), [['b', 2], ['a', 1], ['c', 1]]);
});

test('a keyword is Auto-keywords\' only when the analysis added it to every photo that has it', () => {
  const g = groupRows([row(['sea', 'sky'], ['sea', 'sky']), row(['sea', 'sky'], ['sea']), row(['sea'], ['sea'])]);
  assert.deepEqual(g.auto.map((k) => [k.name, k.n, k.total]), [['sea', 3, 3]]);
  assert.deepEqual(g.own.map((k) => k.name), ['sky'], 'one photo\'s own "sky" makes it a keyword of the user');
});

test('the same input always gives the same order', () => {
  const rows = [row(['b', 'a', 'c'], ['b', 'a']), row(['a', 'b'], ['a', 'b'])];
  assert.deepEqual(groupRows(rows), groupRows(rows));
  assert.deepEqual(groupRows(rows).auto.map((k) => k.name), ['a', 'b']);
});
