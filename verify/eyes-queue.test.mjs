// SPDX-License-Identifier: GPL-3.0-or-later
// eyesQueue.ts: the closed-eye verdicts for the photos on screen are asked for in small batches, after a pause, newest
// window first, never twice, with a cap on the batches in flight, and a failure does not wedge the queue.
import test from 'node:test';
import assert from 'node:assert/strict';
import { EyeQueue } from '../app/src/eyesQueue.ts';

function rig(opts = {}) {
  const calls = [], results = [], timers = [];
  let hold = null, fail = false;
  const fetch = (paths) => { calls.push(paths); if (fail) return Promise.reject(new Error('x')); return hold ? new Promise((r) => { hold.push(() => r(paths.map((path) => ({ path, state: 'open' })))); }) : Promise.resolve(paths.map((path) => ({ path, state: 'open' }))); };
  const clock = { set: (fn, ms) => { const h = { fn, ms }; timers.push(h); return h; }, clear: (h) => { const i = timers.indexOf(h); if (i >= 0) timers.splice(i, 1); } };
  const q = new EyeQueue(fetch, (r) => results.push(...r), { timers: clock, ...opts });
  return { q, calls, results, timers, fire: () => { const t = timers.shift(); t?.fn(); }, hold: (on) => { hold = on ? [] : null; }, setFail: (v) => { fail = v; }, holdList: () => hold };
}
const tick = () => new Promise((r) => setImmediate(r));
const paths = (n, p = 'p') => Array.from({ length: n }, (_, i) => `${p}${i}`);

test('nothing is asked until scrolling pauses, and asking again restarts the pause', async () => {
  const r = rig();
  r.q.want(paths(3));
  assert.equal(r.calls.length, 0);
  assert.equal(r.timers.length, 1);
  assert.equal(r.timers[0].ms, 250);
  r.q.want(paths(4));
  assert.equal(r.timers.length, 1);       // the earlier wait was replaced, not stacked
  r.fire(); await tick();
  assert.deepEqual(r.calls, [paths(4)]);
  assert.equal(r.results.length, 4);
});

test('the photos go in batches of at most 24 (never more than 200), two batches at a time', async () => {
  const r = rig();
  r.hold(true);
  r.q.want(paths(70));
  r.fire(); await tick();
  assert.deepEqual(r.calls.map((c) => c.length), [24, 24]);   // the cap on batches in flight
  r.holdList().shift()(); await tick(); await tick();
  assert.deepEqual(r.calls.map((c) => c.length), [24, 24, 22]);
  assert.equal(new EyeQueue(() => Promise.resolve([]), () => {}, { batch: 500 }).batch, 200);
});

test('a photo scrolled out of the window before its turn is not asked for; one already asked for is kept', async () => {
  const r = rig({ concurrency: 1, batch: 5 });
  r.hold(true);
  r.q.want(paths(12));
  r.fire(); await tick();
  assert.deepEqual(r.calls[0], paths(5));
  r.q.want(paths(3, 'q'));                // scrolled far away: p5..p11 are no longer wanted
  r.holdList().shift()(); await tick();
  r.fire(); await tick();
  assert.deepEqual(r.calls.slice(1).flat(), ['q0', 'q1', 'q2']);
  assert.equal(r.results.filter((x) => x.path.startsWith('p')).length, 5);
});

test('an answer is remembered: the same photo is never asked for twice until the faces change', async () => {
  const r = rig();
  r.q.want(['a', 'b']); r.fire(); await tick();
  r.q.want(['a', 'b', 'c']); r.fire(); await tick();
  assert.deepEqual(r.calls, [['a', 'b'], ['c']]);
  r.q.forget();
  r.q.want(['a']); r.fire(); await tick();
  assert.deepEqual(r.calls.at(-1), ['a']);
});

test('a failing batch does not wedge the queue, and an answer from before a change is dropped', async () => {
  const r = rig({ concurrency: 1, batch: 2 });
  r.setFail(true);
  r.q.want(['a', 'b', 'c', 'd']); r.fire(); await tick(); await tick(); await tick();
  assert.equal(r.calls.length, 2);        // both batches were tried, the second after the first failed
  r.setFail(false);
  r.q.want(['e']); r.fire(); await tick();
  assert.deepEqual(r.results.map((x) => x.path), ['e']);
  const s = rig();
  s.hold(true);
  s.q.want(['x']); s.fire(); await tick();
  s.q.forget();                            // the faces changed while it was on its way
  s.holdList().shift()(); await tick();
  assert.equal(s.results.length, 0);
});

test('nothing is wanted: no wait, no call', async () => {
  const r = rig();
  r.q.want([]);
  assert.equal(r.timers.length, 0);
  assert.equal(r.calls.length, 0);
});
