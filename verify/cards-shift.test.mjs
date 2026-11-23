// SPDX-License-Identifier: GPL-3.0-or-later
// The Library panel's Metadata and Keywording cards must stay mounted, and move nothing below them, while
// the user keeps opening photos (app/harness/cards.tsx mounts the real cards with the real hold from
// app/src/cardHold.ts, opens photos with a gap where the entry and keywords are not in yet, and measures a
// Folders marker under them). A negative control renders the cards as before the hold. Starts a vite dev
// server and drives headless Chrome with --dump-dom; skips when Chrome or app/node_modules is missing.
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
const profile = skip ? '' : mkdtempSync(join(tmpdir(), 'cards-shift-'));
let server;

before(async () => {
  if (skip) return;
  server = spawn(VITE, ['--port', String(PORT), '--strictPort', '--host', '127.0.0.1'], { cwd: APP, stdio: 'ignore' });
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://127.0.0.1:${PORT}/harness/cards.html`)).ok) return; } catch { /* not up yet */ }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error('vite did not start');
});
after(() => { server?.kill(); if (profile) rmSync(profile, { recursive: true, force: true }); });

function report(query) {
  const r = spawnSync(CHROME, ['--headless=new', '--no-sandbox', '--disable-gpu', `--user-data-dir=${profile}`, '--window-size=1000,900',
    '--virtual-time-budget=60000', '--dump-dom', `http://127.0.0.1:${PORT}/harness/cards.html?test=1&${query}`],
    { encoding: 'utf8', maxBuffer: 64 << 20, timeout: 170000 });
  assert.equal(r.status, 0, r.stderr);
  const m = /<pre id="cards-report"[^>]*>([\s\S]*?)<\/pre>/.exec(r.stdout);
  assert.ok(m && m[1].trim(), 'cards-report is empty');
  return JSON.parse(m[1].replaceAll('&quot;', '"').replaceAll('&amp;', '&').replaceAll('&lt;', '<').replaceAll('&gt;', '>'));
}

test('Metadata and Keywording stay mounted and move nothing while photos keep opening', { skip }, () => {
  const r = report('');
  assert.equal(r.error, undefined, r.error);
  assert.deepEqual(r.fail, []);
  assert.equal(r.moved, 0);
  assert.equal(r.disabled, true, 'the keyword box takes input before the keywords are in');
  assert.equal(r.goneWhenClosed, true);
  assert.equal(r.ok, true);
});

test('the cards check fails without the hold (negative control)', { skip }, () => {
  const r = report('break=1');
  assert.equal(r.ok, false);
  assert.ok(r.fail.some((f) => /gone/.test(f)), 'the vanishing was not seen');
  assert.ok(r.moved > 0, 'the marker did not move');
});
