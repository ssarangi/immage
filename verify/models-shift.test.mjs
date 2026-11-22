// SPDX-License-Identifier: GPL-3.0-or-later
// Preferences > General > AI models must not move anything when a pack changes state, a download
// progresses, or a control is hovered or focused (app/harness/models.tsx runs the real AiModelsView
// with fixture data and measures every element: app/harness/modelsShift.ts). This starts a vite dev
// server for the harness and drives headless Chrome with --dump-dom. It skips when Chrome
// (/usr/bin/google-chrome, or $CHROME) or app/node_modules is missing.
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
const PORT = 14440 + (process.pid % 50);
const skip = !existsSync(CHROME) ? `${CHROME} is not installed` : !existsSync(VITE) ? 'app/node_modules is missing' : false;
const profile = skip ? '' : mkdtempSync(join(tmpdir(), 'models-shift-'));
let server;

before(async () => {
  if (skip) return;
  server = spawn(VITE, ['--port', String(PORT), '--strictPort', '--host', '127.0.0.1'], { cwd: APP, stdio: 'ignore' });
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://127.0.0.1:${PORT}/harness/models.html`)).ok) return; } catch { /* not up yet */ }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error('vite did not start');
});
after(() => { server?.kill(); if (profile) rmSync(profile, { recursive: true, force: true }); });

function dump(query, w = 1440, h = 900, budget = 60000, shot) {
  const r = spawnSync(CHROME, ['--headless=new', '--no-sandbox', '--disable-gpu', `--user-data-dir=${profile}`, `--window-size=${w},${h}`,
    `--virtual-time-budget=${budget}`, ...(shot ? [`--screenshot=${shot}`] : ['--dump-dom']), `http://127.0.0.1:${PORT}/harness/models.html?${query}`],
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

for (const [w, h, theme] of [[1440, 900, 'dark'], [1920, 1080, 'dark'], [1000, 700, 'dark'], [1440, 900, 'light'], [2560, 1528, 'grey']]) {
  test(`a pack card and the summary strip keep their boxes in every state, and nothing moves on hover or focus: ${w}x${h} ${theme}`, { skip }, () => {
    const r = report(dump(`shift=1&theme=${theme}`, w, h), 'shift-report');
    assert.equal(r.error, undefined, r.error);
    assert.ok(r.states >= 16, `only ${r.states} states`);
    assert.ok(r.interactions > 60, `only ${r.interactions} interactions ran`);
    assert.ok(r.compared > 2000, `only ${r.compared} measurements`);
    assert.deepEqual(r.failures, [], `moved: ${JSON.stringify(r.failures.slice(0, 5))}`);
    assert.equal(r.maxDelta, 0);
    // card, header, slot, action row, switch rows and the strip's cells are where they were in every state
    assert.equal(r.fixedMaxDelta, 0, r.fixedWorst);
    assert.equal(r.ok, true);
  });
}

test('the shift test fails on a style that lets the state slot grow (negative control)', { skip }, () => {
  const r = report(dump('shift=1&break=1'), 'shift-report');
  assert.equal(r.ok, false);
  assert.ok(r.fixedMaxDelta >= 6, `expected a large delta, got ${r.fixedMaxDelta}`);
});

test('every state shows its pill, its one message and its buttons; nothing starts a download by itself', { skip }, () => {
  const cases = [
    ['notinstalled', [/Not installed/, /Needs 608 MB of free space\. Nothing is downloaded until you press Download\./, /Download \(608 MB\)\.\.\./]],
    ['downloading', [/Downloading 34%/, /of 608 MB/, /6\.2 MB\/s/, /Checking files after the download/, /Cancel/]],
    ['installed', [/Installed · verified/, /checked Nov 14, every file matches its known checksum/, /Check files/, /Remove\.\.\./, /Analyse photos for Smart search/]],
    ['repair', [/Needs repair/, /Analysis is off until it is repaired/, /Repair \(608 MB\)/, /Not available until the model is repaired\./]],
    ['offline', [/Download stopped/, /No connection\. 0\.6 GB of 1\.7 GB kept\. Immage will not retry on its own\./, /Resume \(1\.1 GB left\)/, /Remove partial files/]],
    ['diskfull', [/Needs 608 MB free\. This disk has 900 MB\./, /Download \(608 MB\)\.\.\./]],
    ['unsupported', [/Cannot run on this computer/, /AVX2/]],
  ];
  for (const [state, patterns] of cases) {
    const t = text(dump(`state=${state}`, 1440, 900, 6000));
    for (const p of patterns) assert.match(t, p, `${state}: ${p}`);
    assert.match(t, /What it does/);
    assert.match(t, /What happens to your photos/);
  }
  // the not-installed switch is disabled and says why
  const dom = dump('state=notinstalled', 1440, 900, 6000);
  assert.match(dom, /class="switch"[^>]*disabled/);
  assert.match(text(dom), /Turn this on after the model is installed\./);
});

test('the consent dialog carries the spec text, and Cancel is the focused button', { skip }, () => {
  const dom = dump('state=consent', 1440, 900, 6000);
  const t = text(dom);
  assert.match(t, /Download the Smart search \(CLIP\) model\?/);
  assert.match(t, /Immage will download this model pack to this computer\./);
  assert.match(t, /7 files/);
  assert.match(t, /608 MB\. You have 212\.0 GB free\./);
  assert.match(t, /huggingface\.co, a fixed version\. Immage checks every file against a known checksum/);
  assert.match(t, /MIT\s+Read the license/);
  assert.match(t, /It runs on this computer\. Your photos are never uploaded\./);
  assert.match(t, /Downloading does not look at your photos\. You turn analysis on separately\./);
  assert.match(t, /You can cancel and resume the download, and remove the model later\./);
  assert.match(t, /Cancel[\s\S]*Download 608 MB/);
});

test('the harness is not part of the production build', () => {
  const config = readFileSync(join(APP, 'vite.config.ts'), 'utf8');
  assert.match(config, /input: \{ main: "index\.html", splash: "splash\.html" \}/);
  assert.doesNotMatch(config, /harness/);
});
