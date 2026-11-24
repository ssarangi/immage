// SPDX-License-Identifier: GPL-3.0-or-later
// The Library column is as tall as the window and never scrolls itself: every open card shares the height and a
// card taller than its share scrolls inside its own body, under a header that stays put; toggling a card moves no
// header above it (app/harness/leftPanel.tsx runs the real styles, headerKey and toggle logic).
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

// aside heights for the windows 900x640, 1280x800 and 1600x1000 (the window less the title strip, toolbar and gaps)
for (const [win, h] of [['900x640', 544], ['1280x800', 704], ['1600x1000', 904]]) {
  test(`the column has no scrollbar and tall bodies scroll inside themselves: ${win}`, { skip }, () => {
    const r = run(`h=${h}`);
    assert.equal(r.error, undefined, r.error);
    for (const c of r.combos) {
      // four or more open cards in a window this short cannot all keep their floors (Keywording keeps its fixed rows), so the
      // column's own scroll is the last resort there; everywhere else it must not scroll
      if (!(h < 600 && c.set.length >= 4)) assert.equal(c.columnScrollHeight, c.columnClientHeight, `column scrolls with ${c.set.join('+') || 'nothing'} open: ${JSON.stringify(c)}`);
      assert.ok(c.headerStaysOnScroll);
      for (const b of c.bodies) {
        assert.equal(b.overflowY, 'auto');
        assert.ok(b.clientHeight > 0, `${b.id} has no room in ${c.set.join('+')}`);
        if (b.id === 'folders' || b.id === 'catalog' || b.id === 'keywording') assert.ok(b.scrolls, `${b.id} should scroll inside itself`);
      }
    }
    const all = r.combos.at(-1);
    assert.equal(all.set.length, 5);
    assert.equal(r.headerMoves, 0, 'a header above the toggled card moved while the column had room');
    assert.equal(r.fixedMoves, 0, 'the Navigator or the first header moved');
    // inside a card the controls stay put and only the tree / Auto-keywords chips scroll
    for (const f of r.fixed) {
      const what = `${f.id} with ${f.open.join('+')}: ${JSON.stringify(f)}`;
      assert.ok(f.scrolls && f.scrolled, `the scrolling region does not scroll: ${what}`);
      assert.equal(f.moved, 0, `a fixed control moved while scrolling: ${what}`);
      // the scrolling region gives way first: the body only scrolls itself once it is at its floor
      if (f.bodyScrolls) assert.ok(f.scrollerHeight <= 57, `the body scrolls before the region shrank: ${what}`);
      else assert.ok(f.inCard, `a fixed control fell outside its card: ${what}`);
      if (f.open.length === 1 && f.scrollerHeight > 57) assert.ok(!f.bodyScrolls, `the card body scrolls itself: ${what}`);
    }
    assert.deepEqual(r.keys, ['true', 'false']);
    assert.ok(r.roles);
  });
}

test('the check fails when the column scroll is put back (negative control)', { skip }, () => {
  const r = run('h=544&break=1');
  assert.equal(r.error, undefined, r.error);
  assert.ok(r.combos.some((c) => c.columnScrollHeight > c.columnClientHeight), 'expected the column to scroll');
});

test('the fixed-controls check fails when the controls go back inside the scroller (negative control)', { skip }, () => {
  const r = run('h=544&break=2');
  assert.equal(r.error, undefined, r.error);
  assert.ok(r.fixed.some((f) => f.moved > 0 || f.bodyScrolls), 'expected a fixed control to move or the body to scroll');
});
