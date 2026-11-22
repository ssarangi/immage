// SPDX-License-Identifier: GPL-3.0-or-later
// The Library's Catalog card must not move anything when a source changes state, an index progresses,
// or a control is hovered or focused (app/harness/catalog.tsx runs the real CatalogCard with fixture
// data and measures every element: app/harness/catalogShift.ts), and Preferences' Rebuild row keeps its
// box when it asks for its second click. This starts a vite dev server for the harness and drives
// headless Chrome with --dump-dom. It skips when Chrome (/usr/bin/google-chrome, or $CHROME) or
// app/node_modules is missing.
import test, { after, before } from 'node:test';
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { existsSync, mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const CHROME = process.env.CHROME ?? '/usr/bin/google-chrome';
const APP = fileURLToPath(new URL('../app/', import.meta.url));
const VITE = join(APP, 'node_modules/.bin/vite');
const PORT = 14500 + (process.pid % 50);
const skip = !existsSync(CHROME) ? `${CHROME} is not installed` : !existsSync(VITE) ? 'app/node_modules is missing' : false;
const profile = skip ? '' : mkdtempSync(join(tmpdir(), 'catalog-shift-'));
let server;

before(async () => {
  if (skip) return;
  server = spawn(VITE, ['--port', String(PORT), '--strictPort', '--host', '127.0.0.1'], { cwd: APP, stdio: 'ignore' });
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://127.0.0.1:${PORT}/harness/catalog.html`)).ok) return; } catch { /* not up yet */ }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error('vite did not start');
});
after(() => { server?.kill(); if (profile) rmSync(profile, { recursive: true, force: true }); });

function dump(query, w = 1440, h = 900, budget = 60000, shot) {
  const r = spawnSync(CHROME, ['--headless=new', '--no-sandbox', '--disable-gpu', `--user-data-dir=${profile}`, `--window-size=${w},${h}`,
    `--virtual-time-budget=${budget}`, ...(shot ? [`--screenshot=${shot}`] : ['--dump-dom']), `http://127.0.0.1:${PORT}/harness/catalog.html?${query}`],
    { encoding: 'utf8', maxBuffer: 64 << 20, timeout: 170000 });
  assert.equal(r.status, 0, r.stderr);
  return r.stdout;
}
const report = (dom, id) => {
  const m = new RegExp(`<pre id="${id}"[^>]*>([\\s\\S]*?)</pre>`).exec(dom);
  assert.ok(m && m[1].trim(), `${id} is empty`);
  return JSON.parse(m[1].replaceAll('&quot;', '"').replaceAll('&amp;', '&').replaceAll('&lt;', '<').replaceAll('&gt;', '>'));
};
const text = (dom) => dom.slice(dom.indexOf('<div id="root">')).replace(/<[^>]*>/g, ' ').replace(/&middot;/g, '·').replace(/\s+/g, ' ');

for (const [w, h, theme, size] of [[1440, 900, 'dark', 320], [1440, 900, 'light', 320], [1000, 700, 'dark', 260], [2560, 1528, 'grey', 420]]) {
  test(`the Catalog card keeps its boxes in every state, and nothing moves on hover or focus: ${w}x${h} ${theme}, panel ${size}px`, { skip }, () => {
    const r = report(dump(`shift=1&theme=${theme}&w=${size}`, w, h), 'shift-report');
    assert.equal(r.error, undefined, r.error);
    assert.ok(r.states >= 15, `only ${r.states} states`);
    assert.ok(r.interactions > 100, `only ${r.interactions} interactions ran`);
    assert.ok(r.compared > 3000, `only ${r.compared} measurements`);
    assert.deepEqual(r.failures, [], `moved: ${JSON.stringify(r.failures.slice(0, 5))}`);
    assert.equal(r.maxDelta, 0);
    assert.equal(r.fixedMaxDelta, 0, r.fixedWorst);
    assert.equal(r.ok, true);
  });
}

for (const [theme, size] of [['dark', 320], ['light', 320], ['dark', 240], ['grey', 420]]) {
  test(`the folded Catalog header shows the work and moves nothing as it starts, ends, folds or opens: ${theme}, panel ${size}px`, { skip }, () => {
    const r = report(dump(`shift=1&fold=1&theme=${theme}&w=${size}`), 'shift-report');
    assert.equal(r.error, undefined, r.error);
    assert.ok(r.states >= 18, `only ${r.states} states`);
    assert.deepEqual(r.failures, [], `moved: ${JSON.stringify(r.failures.slice(0, 5))}`);
    assert.equal(r.fixedMaxDelta, 0, r.fixedWorst);
    assert.match(r.shown['folded:reading'], /Indexing 41,860 \/ 98,200|41,860 \/ 98,200|%/);
    assert.match(r.shown['folded:paused'], /Paused|%/);
    assert.equal(r.shown['folded:done'].includes('Indexing'), false);
  });
}

test('the folded header test fails when the status slot sizes from its text (negative control)', { skip }, () => {
  const r = report(dump('shift=1&fold=1&break=1'), 'shift-report');
  assert.equal(r.ok, false);
  assert.ok(r.fixedMaxDelta > 0, `expected a delta, got ${r.fixedMaxDelta}`);
});

test('the Rebuild row keeps its box when it asks for the second click', { skip }, () => {
  const r = report(dump('view=prefs&shift=1'), 'shift-report');
  assert.equal(r.error, undefined, r.error);
  assert.deepEqual(r.failures, [], JSON.stringify(r.failures.slice(0, 5)));
  assert.equal(r.fixedMaxDelta, 0, r.fixedWorst);
});

test('the shift test fails on a style that lets the card grow with its text (negative control)', { skip }, () => {
  const r = report(dump('shift=1&break=1&w=260'), 'shift-report');
  assert.equal(r.ok, false);
  assert.ok(r.fixedMaxDelta >= 6, `expected a large delta, got ${r.fixedMaxDelta}`);
});

test('every state says what is happening in plain words', { skip }, () => {
  const cases = [
    ['finding', [/Indexing Archive 2012-2019/, /12,400 photos so far/, /Working out the time/, /You can keep using Immage/]],
    ['reading', [/41,860 of 98,200 - 118 a second/, /About 9 min left \(could be 6 min to 12 min\)/, /Already usable: 41,860 photos/]],
    ['paused', [/Paused/, /Nothing is lost/, /Resume/]],
    ['done', [/Archive 2012-2019 is ready/, /Your files were only read, never changed/, /Open/]],
    ['doneunreadable', [/98,106 read, 94 could not be read/, /Unreadable files stay listed/]],
    ['offlinejob', [/Lost the connection to fake-nas\. Waiting for it\. Nothing was marked missing\./]],
    ['failed', [/Indexing stopped: the catalog writer stopped/]],
    ['partial', [/2 folders did not answer in time, so nothing was marked missing\./]],
    ['diskfull', [/disk is full/]],
    ['offline', [/Archive 2012-2019 is not connected/, /98,200 photos stay in the Catalog/, /Browsing them offline is not available yet/]],
    ['empty', [/The Catalog keeps what Immage reads from a folder once/, /Add folder to Catalog/, /It never changes, moves or deletes your files/]],
  ];
  for (const [state, patterns] of cases) {
    const t = text(dump(`state=${state}`, 1440, 900, 6000));
    for (const p of patterns) assert.match(t, p, `${state}: ${p}`);
  }
});

test('the add dialog says what it will do before anything is read, and remove says no file is touched', { skip }, () => {
  const add = text(dump('view=add', 1440, 900, 6000));
  assert.match(add, /Add a folder to the Catalog/);
  assert.match(add, /Network share, SMB \(fake-nas, Photos\)/);
  assert.match(add, /Immage reads file headers and makes thumbnails on this computer\. It never changes, moves or deletes your files\./);
  assert.match(add, /Cancel[\s\S]*Add and index/);
  const rm = text(dump('view=remove', 1440, 900, 6000));
  assert.match(rm, /Remove Archive 2012-2019 from the Catalog\?/);
  assert.match(rm, /98,200 photos leave the Catalog/);
  assert.match(rm, /Your files, sidecars and edits are not touched/);
});

test('the harness is not part of the production build', () => {
  const config = readFileSync(join(APP, 'vite.config.ts'), 'utf8');
  assert.doesNotMatch(config, /harness/);
});
