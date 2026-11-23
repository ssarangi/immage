// SPDX-License-Identifier: GPL-3.0-or-later
// The grid's mouse wheel (app/src/smoothWheel.ts) moves a notch over several frames, not in one: the
// jump-detection test counts the biggest single-frame move of the scroll position against a whole notch.
// The negative control is the wheel left to the browser (reduced motion), where a notch is one jump.
import test from 'node:test';
import assert from 'node:assert/strict';
import { SmoothWheel } from '../app/src/smoothWheel.ts';

const wheel = (o = {}) => ({ deltaX: 0, deltaY: 100, deltaMode: 0, ctrlKey: false, metaKey: false, shiftKey: false, altKey: false, ...o });

// a 60 Hz clock, the page's scroll position, and what a browser does with a wheel event it is not given
function rig({ reduced = false, dpr = 1, height = 1e6 } = {}) {
  let time = 1000, next = 1, queue = new Map();
  const el = { scrollTop: 0, clientHeight: 800, scrollHeight: height };
  const host = { now: () => time, raf: (cb) => { queue.set(next, cb); return next++; }, cancel: (id) => queue.delete(id),
                 reduced: () => reduced, dpr: () => dpr };
  const sw = new SmoothWheel(el, host);
  const moves = [];
  // one frame of 16.67 ms: the due callbacks run, then the position is read
  const frame = () => {
    time += 1000 / 60;
    const due = [...queue]; queue = new Map();
    const before = el.scrollTop;
    for (const [, cb] of due) cb(time);
    moves.push(el.scrollTop - before);
  };
  // the event as the page gets it: taken, or applied natively in one jump
  const send = (e) => { const before = el.scrollTop; if (!sw.handle(e)) el.scrollTop += e.deltaY; moves.push(el.scrollTop - before); };
  return { el, sw, moves, frame, send, run: (n) => { for (let i = 0; i < n; i++) frame(); } };
}

test('a mouse notch is spread over frames and lands exactly', () => {
  const r = rig();
  r.send(wheel());
  r.run(60);
  assert.equal(r.el.scrollTop, 100);
  assert.ok(Math.max(...r.moves) < 40, `biggest frame move ${Math.max(...r.moves)}`);
  assert.ok(r.moves.filter((m) => m > 0).length > 8, 'the move took several frames');
});

test('negative control: with the wheel left to the browser a notch is one jump the test flags', () => {
  const r = rig({ reduced: true });
  r.send(wheel());
  r.run(60);
  assert.equal(r.el.scrollTop, 100);
  assert.equal(Math.max(...r.moves), 100);
});

test('a spinning wheel adds up: notches 40 ms apart end exactly at their sum, no frame jumps', () => {
  const r = rig();
  for (let i = 0; i < 30; i++) { r.send(wheel()); r.run(2); }
  r.run(90);
  assert.equal(r.el.scrollTop, 3000);
  assert.ok(Math.max(...r.moves) < 100, `biggest frame move ${Math.max(...r.moves)}`);
  assert.ok(r.moves.every((m) => m >= 0), 'never steps back');
});

test('the end of the list clamps, and a reverse notch eases back', () => {
  const r = rig({ height: 1000 });
  r.send(wheel({ deltaY: 900 }));
  r.run(90);
  assert.equal(r.el.scrollTop, 200);
  r.send(wheel({ deltaY: -100 }));
  r.run(90);
  assert.equal(r.el.scrollTop, 100);
});

test('device pixels: a fractional ratio lands on a whole device pixel', () => {
  const r = rig({ dpr: 1.25 });
  r.send(wheel({ deltaY: 53 }));
  r.run(90);
  assert.equal(r.el.scrollTop * 1.25, Math.round(r.el.scrollTop * 1.25));
  assert.ok(Math.abs(r.el.scrollTop - 53) <= 0.4);
});

test('a touchpad, a modifier and a line-mode wheel', () => {
  let r = rig();
  r.send(wheel({ deltaY: 3.5 }));       // fractional pixels: a touchpad, left alone
  r.send(wheel({ deltaY: 120 }));       // its fling's big event, still the touchpad's
  r.run(30);
  assert.equal(r.el.scrollTop, 123.5);
  r = rig();
  r.send(wheel({ ctrlKey: true }));     // zoom
  r.send(wheel({ shiftKey: true }));
  assert.equal(r.el.scrollTop, 200);
  r = rig();
  r.send(wheel({ deltaY: 3, deltaMode: 1 }));   // three lines
  r.run(90);
  assert.equal(r.el.scrollTop, 120);
});

test('a scrollbar drag or key scroll during an ease ends it', () => {
  const r = rig();
  r.send(wheel({ deltaY: 400 }));
  r.run(5);
  r.el.scrollTop = 5000;                // someone else moved the view
  r.run(60);
  assert.equal(r.el.scrollTop, 5000);
  r.send(wheel());                      // the next notch starts from there
  r.run(60);
  assert.equal(r.el.scrollTop, 5100);
});
