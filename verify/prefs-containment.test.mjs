// SPDX-License-Identifier: GPL-3.0-or-later
// Preferences keeps its settings in their own scroll well: the header and footer stay fixed, only the well's
// scroller moves, and nothing is painted over the header or past the modal's edge (app/harness/prefsTest.ts over
// the real PreferencesDialog). break=1 restores the old pane that scrolled flush to the modal's top edge and must
// fail. Starts a vite dev server and drives headless Chrome with --dump-dom; skips when Chrome or app/node_modules
// is missing.
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
const PORT = 14600 + (process.pid % 50);
const skip = !existsSync(CHROME) ? `${CHROME} is not installed` : !existsSync(VITE) ? 'app/node_modules is missing' : false;
const profile = skip ? '' : mkdtempSync(join(tmpdir(), 'prefs-containment-'));
let server;

before(async () => {
  if (skip) return;
  server = spawn(VITE, ['--port', String(PORT), '--strictPort', '--host', '127.0.0.1'], { cwd: APP, stdio: 'ignore' });
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://127.0.0.1:${PORT}/harness/prefs.html`)).ok) return; } catch { /* not up yet */ }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error('vite did not start');
});
after(() => { server?.kill(); if (profile) rmSync(profile, { recursive: true, force: true }); });

function report(query, size) {
  const r = spawnSync(CHROME, ['--headless=new', '--no-sandbox', '--disable-gpu', `--user-data-dir=${profile}`, `--window-size=${size}`,
    '--virtual-time-budget=60000', '--dump-dom', `http://127.0.0.1:${PORT}/harness/prefs.html?test=1&${query}`],
    { encoding: 'utf8', maxBuffer: 64 << 20, timeout: 170000 });
  assert.equal(r.status, 0, r.stderr);
  const m = /<pre id="prefs-report"[^>]*>([\s\S]*?)<\/pre>/.exec(r.stdout);
  assert.ok(m && m[1].trim(), 'prefs-report is empty');
  return JSON.parse(m[1].replaceAll('&quot;', '"').replaceAll('&amp;', '&').replaceAll('&lt;', '<').replaceAll('&gt;', '>'));
}

for (const [label, size, extra] of [['1100x720', '1100,720', ''], ['800x600 (icon rail)', '800,600', ''], ['1100x720 right to left', '1100,720', '&dir=rtl'], ['light theme', '1100,720', '&theme=light']]) {
  test(`Preferences scrolls only inside its well at ${label}`, { skip }, () => {
    const r = report(`x=1${extra}`, size);
    assert.equal(r.error, undefined, r.error);
    assert.deepEqual(r.fail, []);
    assert.equal(r.moved, 0);
    assert.equal(r.ok, true);
  });
}

test('the containment check fails on the old pane that scrolled through the modal edge (negative control)', { skip }, () => {
  const r = report('break=1', '1100,720');
  assert.equal(r.ok, false);
  assert.ok(r.fail.some((f) => /top edge|overlaps|outside/.test(f)), `the original complaint was not seen: ${r.fail.join('; ')}`);
});

test('the containment check fails when the well takes the height of its content and the footer moves (negative control)', { skip }, () => {
  const r = report('break=shift', '1100,720');
  assert.equal(r.ok, false);
});
