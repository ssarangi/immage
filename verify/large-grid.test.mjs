// SPDX-License-Identifier: GPL-3.0-or-later
// A folder of tens of thousands of photos draws only the cells near the screen, in the places CSS
// would put them all (app/harness/grid.tsx: the real Filmstrip and ThumbLoader over synthetic
// photos, no engine). Starts a vite dev server and drives headless Chrome with --dump-dom; skips
// when Chrome (/usr/bin/google-chrome, or $CHROME) or app/node_modules is missing.
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
const profile = skip ? '' : mkdtempSync(join(tmpdir(), 'large-grid-'));
let server;

before(async () => {
  if (skip) return;
  server = spawn(VITE, ['--port', String(PORT), '--strictPort', '--host', '127.0.0.1'], { cwd: APP, stdio: 'ignore' });
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://127.0.0.1:${PORT}/harness/grid.html`)).ok) return; } catch { /* not up yet */ }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error('vite did not start');
});
after(() => { server?.kill(); if (profile) rmSync(profile, { recursive: true, force: true }); });

function report(query, w = 1300, h = 900) {
  const r = spawnSync(CHROME, ['--headless=new', '--no-sandbox', '--disable-gpu', `--user-data-dir=${profile}`, `--window-size=${w},${h}`,
    '--virtual-time-budget=20000', '--dump-dom', `http://127.0.0.1:${PORT}/harness/grid.html?check=1&${query}`],
    { encoding: 'utf8', maxBuffer: 64 << 20, timeout: 170000 });
  assert.equal(r.status, 0, r.stderr);
  const m = /<pre id="grid-report"[^>]*>([\s\S]*?)<\/pre>/.exec(r.stdout);
  assert.ok(m, 'the harness wrote no report');
  return JSON.parse(m[1].replaceAll('&quot;', '"').replaceAll('&amp;', '&'));
}

for (const [mode, n] of [['grid', 25000], ['grid', 700], ['strip', 25000]]) {
  test(`${n} photos as a ${mode}: only the cells near the screen exist, each where CSS puts it`, { skip }, () => {
    const r = report(`mode=${mode}&n=${n}`);
    assert.equal(r.missing, 0, `${r.missing} cells that should show are not drawn: ${JSON.stringify(r.positions)}`);
    assert.equal(r.misplaced, 0, `${r.misplaced} cells are off their place`);
    assert.ok(Math.abs(r.scrollExtent - r.expectedExtent) <= 2, `scroll extent ${r.scrollExtent}, expected ${r.expectedExtent}`);
    // a few screens of cells, not the folder: 25,000 cells would be 600,000 elements
    assert.ok(r.maxCells < 200, `${r.maxCells} cells drawn at once`);
    if (n > 5000) assert.ok(r.maxNodes < 4000, `${r.maxNodes} elements in the page`);
  });
}
