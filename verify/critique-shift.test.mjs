// SPDX-License-Identifier: GPL-3.0-or-later
// The critique lightbox must not move anything when a row, badge, tab or button is
// hovered, focused or pinned, in any of its states (app/harness/critique.tsx runs the
// real CullLightbox, CullSide and CullOverlay with fixture data and measures every
// element and text run before and after: app/harness/shiftTest.ts). This starts a vite
// dev server for the harness and drives headless Chrome with --dump-dom. It skips when
// Chrome (/usr/bin/google-chrome, or $CHROME) or app/node_modules is missing.
import test, { after, before } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const CHROME = process.env.CHROME ?? '/usr/bin/google-chrome';
const APP = fileURLToPath(new URL('../app/', import.meta.url));
const VITE = join(APP, 'node_modules/.bin/vite');
const PORT = 14390 + (process.pid % 50);
const skip = !existsSync(CHROME) ? `${CHROME} is not installed` : !existsSync(VITE) ? 'app/node_modules is missing' : false;
const profile = skip ? '' : mkdtempSync(join(tmpdir(), 'critique-shift-'));
let server;

before(async () => {
  if (skip) return;
  server = spawn(VITE, ['--port', String(PORT), '--strictPort', '--host', '127.0.0.1'], { cwd: APP, stdio: 'ignore' });
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://127.0.0.1:${PORT}/harness/critique.html`)).ok) return; } catch { /* not up yet */ }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error('vite did not start');
});
after(() => { server?.kill(); if (profile) rmSync(profile, { recursive: true, force: true }); });

// the virtual time budget is also how far Date.now() runs on, so a state that reads the clock gets a short one
function dump(query, w = 1440, h = 900, flags = [], budget = 100000) {
  const r = spawnSync(CHROME, ['--headless=new', '--no-sandbox', '--disable-gpu', `--user-data-dir=${profile}`, `--window-size=${w},${h}`,
    `--virtual-time-budget=${budget}`, ...flags, '--dump-dom', `http://127.0.0.1:${PORT}/harness/critique.html?${query}`],
    { encoding: 'utf8', maxBuffer: 64 << 20, timeout: 170000 });
  assert.equal(r.status, 0, r.stderr);
  return r.stdout;
}
const report = (dom, id) => {
  const m = new RegExp(`<pre id="${id}"[^>]*>([\\s\\S]*?)</pre>`).exec(dom);
  assert.ok(m && m[1].trim(), `${id} is empty`);
  return JSON.parse(m[1].replaceAll('&quot;', '"').replaceAll('&amp;', '&').replaceAll('&lt;', '<').replaceAll('&gt;', '>'));
};
const text = (dom) => dom.slice(dom.indexOf('<div id="root">')).replace(/<[^>]*>/g, ' ').replace(/\s+/g, ' ');

for (const [w, h, theme] of [[1440, 900, 'dark'], [1920, 1080, 'dark'], [1100, 700, 'dark'], [1440, 900, 'light'], [2560, 1528, 'grey']]) {
  test(`nothing shifts on hover, focus, pin or selection, in every state: ${w}x${h} ${theme}`, { skip }, () => {
    const r = report(dump(`shift=1&theme=${theme}`, w, h), 'shift-report');
    assert.equal(r.error, undefined, r.error);
    assert.ok(r.interactions > 300, `only ${r.interactions} interactions ran`);
    assert.ok(r.compared > 10000, `only ${r.compared} measurements`);
    assert.deepEqual(r.failures, [], `moved: ${JSON.stringify(r.failures.slice(0, 5))}`);
    assert.equal(r.maxDelta, 0);
    // the stage, photo, header, tabs, footer and panel are where they were in every state and on both tabs
    assert.equal(r.fixedMaxDelta, 0, r.fixedWorst);
    assert.equal(r.ok, true);
  });
}

test('the shift test fails on a style that moves things (negative control)', { skip }, () => {
  const r = report(dump('shift=1&break=1'), 'shift-report');
  assert.equal(r.ok, false);
  assert.ok(r.maxDelta >= 6, `expected a large delta, got ${r.maxDelta}`);
});

test('loading: honest wording at 25 s and 60 s, Cancel, and no animation under reduced motion', { skip }, () => {
  const early = text(dump('state=loading&elapsed=5', 1440, 900, [], 3000));
  assert.match(early, /Gemini is studying this photo/);
  assert.match(early, /10 to 40 seconds/);
  assert.match(early, /Cancel/);
  assert.doesNotMatch(early, /Try again/);
  assert.match(text(dump('state=loading&elapsed=30', 1440, 900, [], 3000)), /Still working\. Large photos take longer\./);
  const late = text(dump('state=loading&elapsed=65', 1440, 900, [], 3000));
  assert.match(late, /Taking longer than usual/);
  assert.match(late, /Try again/);
  assert.match(late, /Cancel/);
  const still = dump('state=loading&elapsed=65', 1440, 900, ['--force-prefers-reduced-motion'], 3000);
  assert.equal(report(still, 'motion-report').animations, 0, 'reduced motion leaves no running animation');
  assert.ok(report(dump('state=loading', 1440, 900, [], 3000), 'motion-report').animations > 0, 'the loading state animates without the preference');
});

test('no key, empty, error and cull states carry their spec text and actions', { skip }, () => {
  assert.match(text(dump('state=nokey', 1440, 900, [], 3000)), /Add a Gemini key to get critiques[\s\S]*Open Preferences/);
  assert.match(text(dump('state=empty', 1440, 900, [], 3000)), /Get a critique of this photo[\s\S]*Critique this photo/);
  const err = dump('state=error', 1440, 900, [], 3000);
  assert.match(text(err), /Gemini did not answer/);
  assert.match(err, /role="alert"/);
  assert.match(text(dump('state=errorkept', 1440, 900, [], 3000)), /The earlier critique is still shown below/);
  const cull = text(dump('state=cull', 1440, 900, [], 3000));
  assert.match(cull, /Why/);
  assert.match(cull, /Outline the softest area/);
  assert.match(cull, /Keep \(P\)[\s\S]*Reject \(R\)[\s\S]*Make best \(B\)/);
});
