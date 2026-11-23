// SPDX-License-Identifier: GPL-3.0-or-later
// The Library panel's cards expand independently (Folders and Catalog together), the two trees are capped at
// about 38% of the panel and scroll inside themselves when both are open, and toggling a card moves neither its
// header nor the panel's scroll (app/harness/leftPanel.tsx runs the real styles, headerKey and toggle logic).
// Starts a vite dev server and drives headless Chrome with --dump-dom; skips without Chrome or app/node_modules.
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
const profile = skip ? '' : mkdtempSync(join(tmpdir(), 'left-panel-'));
let server;

before(async () => {
  if (skip) return;
  server = spawn(VITE, ['--port', String(PORT), '--strictPort', '--host', '127.0.0.1'], { cwd: APP, stdio: 'ignore' });
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://127.0.0.1:${PORT}/harness/leftPanel.html`)).ok) return; } catch { /* not up yet */ }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error('vite did not start');
});
after(() => { server?.kill(); if (profile) rmSync(profile, { recursive: true, force: true }); });

function run(query) {
  const r = spawnSync(CHROME, ['--headless=new', '--no-sandbox', '--disable-gpu', `--user-data-dir=${profile}`, '--window-size=600,900',
    '--virtual-time-budget=30000', '--dump-dom', `http://127.0.0.1:${PORT}/harness/leftPanel.html?test=1&${query}`],
    { encoding: 'utf8', maxBuffer: 64 << 20, timeout: 120000 });
  assert.equal(r.status, 0, r.stderr);
  const m = /<pre id="shift-report"[^>]*>([\s\S]*?)<\/pre>/.exec(r.stdout);
  assert.ok(m && m[1].trim(), 'report is empty');
  return JSON.parse(m[1].replaceAll('&quot;', '"').replaceAll('&amp;', '&'));
}

for (const h of [700, 500, 900]) {
  test(`Folders and Catalog stay open together, capped and scrolling inside, and no header or scroll moves: panel ${h}px`, { skip }, () => {
    const r = run(`h=${h}`);
    assert.equal(r.error, undefined, r.error);
    assert.deepEqual(r.both.open, ['folders', 'catalog']);
    assert.ok(r.both.folders <= r.cap + 0.5 && r.both.catalog <= r.cap + 0.5, JSON.stringify(r.both));
    assert.ok(r.both.folders >= 139 && r.both.catalog >= 139, 'not below the minimum');
    assert.ok(r.both.foldersScrolls && r.both.catalogScrolls);
    assert.ok(r.innerScrollable);
    assert.deepEqual(r.afterOpenTwo, ['metadata', 'keywording', 'folders', 'catalog']);
    assert.equal(r.headerMoves, 0);
    assert.equal(r.scrollDelta, 0);
    assert.deepEqual(r.keys, ['true', 'false']);
    assert.ok(r.roles);
    assert.ok(r.single.open.includes('folders') && !r.single.open.includes('catalog'));
    assert.ok(r.single.clientHeight > r.cap + 0.5, 'a single open tree is not capped');
    assert.deepEqual(r.reopened.filter((c) => c === 'folders' || c === 'catalog'), ['folders', 'catalog']);
  });
}

test('the cap check fails without the height rule (negative control)', { skip }, () => {
  const r = run('h=700&break=1');
  assert.equal(r.error, undefined, r.error);
  assert.ok(r.both.folders > r.cap + 0.5, `expected an uncapped tree, got ${r.both.folders} vs ${r.cap}`);
});
