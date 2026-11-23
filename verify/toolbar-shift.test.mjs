// SPDX-License-Identifier: GPL-3.0-or-later
// The bottom toolbar (app/harness/toolbar.tsx runs the real Toolbar in the real center column markup, and
// app/harness/toolbarTest.ts measures it): the bar is as wide as the grid surface with a 12px gap above it,
// 40px high and 8px above the column's end, never scrolls sideways from 680px to 1600px, moves nothing when
// the view, the filters, Find or the cull state change, needs no script for its gap (a style reload keeps it),
// is one tab stop with the arrow keys reaching every control, and names every button. This starts a vite dev
// server and drives headless Chrome with --dump-dom. It skips when Chrome (/usr/bin/google-chrome, or $CHROME)
// or app/node_modules is missing.
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
const PORT = 14500 + (process.pid % 50);
const skip = !existsSync(CHROME) ? `${CHROME} is not installed` : !existsSync(VITE) ? 'app/node_modules is missing' : false;
const profile = skip ? '' : mkdtempSync(join(tmpdir(), 'toolbar-shift-'));
let server;

before(async () => {
  if (skip) return;
  server = spawn(VITE, ['--port', String(PORT), '--strictPort', '--host', '127.0.0.1'], { cwd: APP, stdio: 'ignore' });
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://127.0.0.1:${PORT}/harness/toolbar.html`)).ok) return; } catch { /* not up yet */ }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error('vite did not start');
});
after(() => { server?.kill(); if (profile) rmSync(profile, { recursive: true, force: true }); });

function report(query, flags = []) {
  const r = spawnSync(CHROME, ['--headless=new', '--no-sandbox', '--disable-gpu', `--user-data-dir=${profile}`, '--window-size=1700,800',
    '--virtual-time-budget=60000', ...flags, '--dump-dom', `http://127.0.0.1:${PORT}/harness/toolbar.html?test=1&${query}`],
    { encoding: 'utf8', maxBuffer: 64 << 20, timeout: 170000 });
  assert.equal(r.status, 0, r.stderr);
  const m = /<pre id="bar-report"[^>]*>([\s\S]*?)<\/pre>/.exec(r.stdout);
  assert.ok(m && m[1].trim(), 'bar-report is empty');
  return JSON.parse(m[1].replaceAll('&quot;', '"').replaceAll('&amp;', '&').replaceAll('&lt;', '<').replaceAll('&gt;', '>'));
}

for (const [theme, dir, lang] of [['dark', 'ltr', 'en'], ['light', 'ltr', 'en'], ['dark', 'ltr', 'fr'], ['dark', 'rtl', 'en'], ['light', 'rtl', 'fr']]) {
  test(`the toolbar: geometry, no shift, keyboard and names: ${theme} ${dir} ${lang}`, { skip }, () => {
    const r = report(`theme=${theme}&dir=${dir}`, [`--accept-lang=${lang}`]);
    assert.equal(r.error, undefined, r.error);
    assert.deepEqual(r.fail, []);
    assert.ok(r.compared > 1000, `only ${r.compared} measurements`);
    assert.equal(r.worst, 0);
    assert.equal(r.layoutShift, 0);
    assert.equal(r.tabStops, 1);
    assert.equal(r.afterReload, 12);
    assert.equal(r.ok, true);
  });
}

test('the toolbar checks fail on a style that moves the bar and its controls (negative control)', { skip }, () => {
  const r = report('theme=dark&break=1');
  assert.equal(r.ok, false);
  assert.ok(r.fail.some((f) => /gap/.test(f)), 'the gap change was not seen');
  assert.ok(r.worst >= 5, `expected a delta, got ${r.worst}`);
});
