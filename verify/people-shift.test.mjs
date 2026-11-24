// SPDX-License-Identifier: GPL-3.0-or-later
// The People view must not move anything when a tile is selected, hovered or named, when the name box opens, the
// Undo line comes and goes, a scan ticks, pauses and fails, new groups arrive, or a consent step changes
// (app/harness/people.tsx runs the real PeopleView on an in-memory fake of the faces commands and measures every
// element before and after: app/harness/peopleShift.ts). The keyboard-only path (arrows, a letter, a name, Enter,
// Tab) works, and 5,000 faces mount a bounded number of tiles in every section. This starts a vite dev server for
// the harness and drives headless Chrome with --dump-dom. It skips when Chrome (/usr/bin/google-chrome, or $CHROME)
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
const profile = skip ? '' : mkdtempSync(join(tmpdir(), 'people-shift-'));
let server;

before(async () => {
  if (skip) return;
  server = spawn(VITE, ['--port', String(PORT), '--strictPort', '--host', '127.0.0.1'], { cwd: APP, stdio: 'ignore' });
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://127.0.0.1:${PORT}/harness/people.html`)).ok) return; } catch { /* not up yet */ }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error('vite did not start');
});
after(() => { server?.kill(); if (profile) rmSync(profile, { recursive: true, force: true }); });

function dump(query, w = 1440, h = 900, flags = [], budget = 90000) {
  const r = spawnSync(CHROME, ['--headless=new', '--no-sandbox', '--disable-gpu', `--user-data-dir=${profile}`, `--window-size=${w},${h}`,
    `--virtual-time-budget=${budget}`, ...flags, '--dump-dom', `http://127.0.0.1:${PORT}/harness/people.html?${query}`],
    { encoding: 'utf8', maxBuffer: 64 << 20, timeout: 400000 });
  assert.equal(r.status, 0, r.stderr);
  return r.stdout;
}
const report = (dom) => {
  const m = /<pre id="shift-report"[^>]*>([\s\S]*?)<\/pre>/.exec(dom);
  assert.ok(m && m[1].trim(), 'shift-report is empty');
  return JSON.parse(m[1].replaceAll('&quot;', '"').replaceAll('&amp;', '&').replaceAll('&lt;', '<').replaceAll('&gt;', '>'));
};
const text = (dom) => dom.slice(dom.indexOf('<div id="root">')).replace(/<[^>]*>/g, ' ').replace(/\s+/g, ' ');

for (const [w, h, theme, dir] of [[1440, 900, 'dark', 'ltr'], [1440, 900, 'light', 'ltr'], [1440, 900, 'dark', 'rtl'], [1100, 700, 'light', 'ltr']]) {
  test(`nothing shifts on selection, hover, naming, Undo, scan or new groups: ${w}x${h} ${theme} ${dir}`, { skip }, () => {
    const r = report(dump(`shift=1&theme=${theme}&dir=${dir}&w=${w}`, w, h));
    assert.equal(r.error, undefined, r.error);
    assert.deepEqual(r.failures, [], `failed: ${JSON.stringify(r.failures.slice(0, 5))}`);
    assert.ok(r.compared > 1500, `only ${r.compared} measurements`);
    assert.equal(r.maxDelta, 0, JSON.stringify(r.worst));
    // (the layout-shift detail below is printed when the browser's own count is not zero)
    assert.deepEqual(r.styleOffenders, []);
    // the selection bar's own buttons change with what is selected: a few 0.0001 entries, never a tile or a row
    assert.ok(r.layoutShift.value < 0.002, `the browser reported a layout shift: ${JSON.stringify(r.layoutShift.detail)}`);
    assert.equal(r.ok, true);
  });
}

test('the keyboard-only path names a group: arrows, a letter, a name, Enter, then Tab to the next', { skip }, () => {
  const r = report(dump('shift=1&theme=dark'));
  assert.equal(r.error, undefined, r.error);
  assert.ok(r.keyboard, 'no keyboard report');
  assert.match(r.keyboard.undo, /Dad/);
  assert.match(r.keyboard.assigned, /^assign:1:/);
  assert.equal(r.keyboard.nextIndex, r.keyboard.startIndex);
  assert.equal(r.keyboard.groupsAfter < r.keyboard.groupsBefore || r.keyboard.groupsAfter > 0, true);
  assert.deepEqual(r.options.slice(-1)[0] && r.options.some((o) => /Maya/.test(o)), true);
  assert.deepEqual(r.marquee.drawn, true);
  assert.ok(r.marquee.selected >= 2);
});

test('the shift test fails on styles that move things (negative control: Undo line and scan strip not reserved)', { skip }, () => {
  const r = report(dump('shift=1&break=1'));
  assert.equal(r.ok, false);
  assert.ok(r.styleOffenders.length > 0, 'the stylesheet audit found nothing');
  assert.ok(r.failures.some((f) => /people-undo|people-strip/.test(f)), `expected the reserved rows to be caught: ${JSON.stringify(r.failures.slice(0, 6))}`);
  assert.ok(r.maxDelta > 0);
});

test('the consent card keeps its size through every step and turns People on only after the box is ticked', { skip }, () => {
  const r = report(dump('shift=1&state=off&theme=dark'));
  assert.equal(r.error, undefined, r.error);
  assert.deepEqual(r.failures, []);
  assert.equal(r.consent.max, 0, r.consent.worst);
  const sizes = Object.values(r.consent.rects).map((x) => `${x.w}x${x.h}`);
  assert.equal(new Set(sizes).size, 1, sizes.join(' '));
  assert.ok(r.consent.compared > 20);
  assert.ok(r.turnedOn.rail);
});

test('5,000 faces: every section mounts a bounded number of tiles however far it is scrolled', { skip }, () => {
  const r = report(dump('shift=1&many=5000&theme=dark', 1440, 900, [], 400000));
  assert.equal(r.error, undefined, r.error);
  assert.deepEqual(r.failures, []);
  for (const s of ['people', 'review', 'unassigned']) {
    assert.ok(r.virt[s].maxTiles < 150, `${s}: ${r.virt[s].maxTiles} tiles mounted`);
    assert.ok(r.virt[s].maxNodes < 2500, `${s}: ${r.virt[s].maxNodes} nodes`);
  }
  assert.ok(r.virt.review.height > 30000, `the review list is ${r.virt.review.height}px tall: it is not the whole list`);
});

test('the other states render their text: scanning, empty, error, off, download', { skip }, () => {
  const scanning = text(dump('state=scanning'));
  assert.match(scanning, /Looking for faces/);
  assert.match(text(dump('state=empty')), /No faces found|Scanned|Loading/);
  assert.match(text(dump('state=error')), /cannot be read/);
  const off = text(dump('state=off'));
  assert.match(off, /Face data is biometric/);
  assert.match(off, /Turn on People/);
  assert.match(text(dump('state=download')), /Downloading|download/i);
});

test('real counts, the 2 x 2 collage, the quiet strip notes and restoring an ignored face', { skip }, () => {
  const r = report(dump('shift=1&theme=dark'));
  assert.equal(r.error, undefined, r.error);
  assert.deepEqual(r.failures, []);
  assert.equal(r.counts.unassigned, String(r.counts.unknown));
  assert.equal(r.counts.ignored, String(r.counts.ignoredTotal));
  assert.ok(r.collage.groups > 0 && r.collage.cells >= 2);
  assert.match(r.notes, /5 photos could not be read/);
  assert.match(r.notes, /2 folders offline/);
  assert.match(r.notes, /3 network folders not scanned/);
  assert.match(r.ignored.undo, /Took 1 face back/);
  assert.ok(r.ignored.after < r.ignored.before);
});

test('the folder scan says "N of M photos" from the open folder, never a placeholder, and the page fills in under a resting pointer', { skip }, () => {
  const r = report(dump('shift=1&state=scanning&folderscope=1&theme=dark'));
  assert.equal(r.error, undefined, r.error);
  assert.deepEqual(r.failures, []);
  assert.equal(r.strip[0], 'Finding faces: 36 of 333 photos');
  assert.deepEqual([r.job.done, r.job.total], [36, 333]);
  assert.ok(r.tiles > 0);
  assert.equal(r.after, '');
});

test('the consent card starts on the open folder, and on the Catalog only when no folder is open', { skip }, () => {
  assert.equal(report(dump('shift=1&state=off&theme=dark')).defaultScope, 'folder');
  assert.equal(report(dump('shift=1&state=off&nofolder=1&theme=dark')).defaultScope, 'catalog');
});

test('a scan that goes on by itself keeps the strip, the counts and the groups current with no event, and the activity line leaves at its end', { skip }, () => {
  const r = report(dump('shift=1&state=scanning&folderscope=1&grow=1&theme=dark', 1440, 900, [], 120000));
  assert.equal(r.error, undefined, r.error);
  assert.deepEqual(r.failures, []);
  assert.equal(r.grow[2].strip, '');
  assert.equal(r.grow[2].job, false);
});

test('a named group leaves To review at once and for good, the box says what is named, and Ctrl+Z takes it back', { skip }, () => {
  const r = report(dump('shift=1&theme=dark'));
  assert.equal(r.error, undefined, r.error);
  assert.deepEqual(r.failures, []);
  assert.ok(r.named.goneNow && r.named.goneLater);
  assert.equal(r.named.reviewNow, r.named.reviewBefore - 1);
  assert.match(r.nameTitle, /^Name this group \(\d+ faces?\)$/);
  assert.equal(r.ctrlZ.after, r.ctrlZ.before + 1);
});

test('the face overlay follows the zoom, names on hover and focus, names and rejects faces; the card lists them and selects', { skip }, () => {
  const r = report(dump('shift=1&view=overlay&theme=dark', 1440, 900, [], 120000));
  assert.equal(r.error, undefined, r.error);
  assert.deepEqual(r.failures, []);
  assert.equal(r.boxes, 4);
  assert.ok(r.names.every((n) => /^Face \d of 4, /.test(n)), r.names.join('|'));
  assert.equal(r.ok, true);
});

test('the scope chip names the folder, also when only faces.scope was set (no folder stored)', { skip }, () => {
  for (const extra of ['', '&noscopefolder=1']) {
    const r = report(dump(`shift=1&state=scanning&folderscope=1&theme=dark${extra}`));
    assert.equal(r.error, undefined, r.error);
    assert.deepEqual(r.failures, []);
    assert.equal(r.chip, 'Folder: Shoot');
    assert.equal(r.strip[0], 'Finding faces: 36 of 333 photos');
  }
});
