// SPDX-License-Identifier: GPL-3.0-or-later
// The view tabs in the window's top strip (app/harness/viewtabs.tsx runs the real TitleStrip, and
// app/harness/viewtabsTest.ts measures it): the tabs never overlap the menus or leave the strip at
// 700, 900, 1280 and 1600px (three tabs and a fourth, as People will be), nothing moves when the
// view changes, the labels become icons when the strip is narrow, the tablist is one tab stop that
// the arrow keys walk (a disabled tab is skipped), no tab has an underline or bar, a badge moves nothing, every tab is named and has a shortcut tooltip,
// and no drag region covers a tab. Negative controls: a drag region over the tabs and tabs moved
// over the menus must fail. This starts a vite dev server and drives headless Chrome with
// --dump-dom. It skips when Chrome (/usr/bin/google-chrome, or $CHROME) or app/node_modules is missing.
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
const profile = skip ? '' : mkdtempSync(join(tmpdir(), 'view-tabs-'));
let server;

before(async () => {
  if (skip) return;
  server = spawn(VITE, ['--port', String(PORT), '--strictPort', '--host', '127.0.0.1'], { cwd: APP, stdio: 'ignore' });
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://127.0.0.1:${PORT}/harness/viewtabs.html`)).ok) return; } catch { /* not up yet */ }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error('vite did not start');
});
after(() => { server?.kill(); if (profile) rmSync(profile, { recursive: true, force: true }); });

function report(query, flags = []) {
  const r = spawnSync(CHROME, ['--headless=new', '--no-sandbox', '--disable-gpu', `--user-data-dir=${profile}`, '--window-size=1700,300',
    '--virtual-time-budget=60000', ...flags, '--dump-dom', `http://127.0.0.1:${PORT}/harness/viewtabs.html?test=1&${query}`],
    { encoding: 'utf8', maxBuffer: 64 << 20, timeout: 170000 });
  assert.equal(r.status, 0, r.stderr);
  const m = /<pre id="tabs-report"[^>]*>([\s\S]*?)<\/pre>/.exec(r.stdout);
  assert.ok(m && m[1].trim(), 'tabs-report is empty');
  return JSON.parse(m[1].replaceAll('&quot;', '"').replaceAll('&amp;', '&').replaceAll('&lt;', '<').replaceAll('&gt;', '>'));
}

for (const [theme, dir, lang] of [['dark', 'ltr', 'en'], ['light', 'ltr', 'fr'], ['dark', 'rtl', 'en']]) {
  test(`the view tabs: no overlap, no shift, keyboard and names: ${theme} ${dir} ${lang}`, { skip }, () => {
    const r = report(`theme=${theme}&dir=${dir}`, [`--accept-lang=${lang}`]);
    assert.equal(r.error, undefined, r.error);
    assert.deepEqual(r.fail, []);
    assert.equal(r.compared, 8);
    assert.equal(r.shifts, 0);
    if (lang === 'en') assert.deepEqual(r.at900, [true, true], 'labels should still show at 900px in English');
    assert.equal(r.tabStops, 1);
    assert.equal(r.ok, true);
  });
}

for (const theme of ['dark', 'graphite', 'slate', 'warm-ash', 'grey', 'classic-grey', 'soft-grey', 'light']) {
  test(`the view tabs are readable on the strip: ${theme}`, { skip }, () => {
    const r = report(`theme=${theme}`);
    assert.equal(r.error, undefined, r.error);
    assert.deepEqual(r.fail, []);
    assert.ok(r.textRatio >= 4, `${r.textRatio}`);
  });
}

test('reduced motion turns the tab transitions off', { skip }, () => {
  const r = report('theme=dark', ['--force-prefers-reduced-motion']);
  assert.equal(r.error, undefined, r.error);
  assert.match(r.duration, /^0s/);
  assert.doesNotMatch(report('theme=dark').duration, /^0s/);
});

test('the macOS strip (no menus) holds the tabs without overlap', { skip }, () => {
  const r = report('mac=1');
  assert.equal(r.error, undefined, r.error);
  assert.deepEqual(r.fail, []);
});

test('the checks fail on a drag region over the tabs (negative control)', { skip }, () => {
  const r = report('break=drag');
  assert.equal(r.ok, false);
  assert.ok(r.fail.some((f) => /covered|drag region/.test(f)), r.fail.join('; '));
});

test('the checks fail on tabs over the menus (negative control)', { skip }, () => {
  const r = report('break=overlap');
  assert.equal(r.ok, false);
  assert.ok(r.fail.some((f) => /overlap the menus/.test(f)), r.fail.join('; '));
});

test('the checks fail on an underline bar under a tab (negative control)', { skip }, () => {
  const r = report('break=underline');
  assert.equal(r.ok, false);
  assert.ok(r.fail.some((f) => /bottom border or bar/.test(f)), r.fail.join('; '));
});
