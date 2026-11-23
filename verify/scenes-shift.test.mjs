// SPDX-License-Identifier: GPL-3.0-or-later
// The Scenes timeline and scene view must not move anything when data, scores, a status line, a decision, a
// selection or the language changes (app/harness/scenes.tsx runs the real ScenesView, SceneLightbox and
// CullLightbox over synthetic photos and measures every element before and after: app/harness/scenesShift.ts),
// and the 20,000-photo timeline mounts a bounded number of cards however far it is scrolled. This starts a vite
// dev server for the harness and drives headless Chrome with --dump-dom. It skips when Chrome
// (/usr/bin/google-chrome, or $CHROME) or app/node_modules is missing.
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
const PORT = 14440 + (process.pid % 50);
const skip = !existsSync(CHROME) ? `${CHROME} is not installed` : !existsSync(VITE) ? 'app/node_modules is missing' : false;
const profile = skip ? '' : mkdtempSync(join(tmpdir(), 'scenes-shift-'));
let server;

before(async () => {
  if (skip) return;
  server = spawn(VITE, ['--port', String(PORT), '--strictPort', '--host', '127.0.0.1'], { cwd: APP, stdio: 'ignore' });
  for (let i = 0; i < 100; i++) {
    try { if ((await fetch(`http://127.0.0.1:${PORT}/harness/scenes.html`)).ok) return; } catch { /* not up yet */ }
    await new Promise((r) => setTimeout(r, 200));
  }
  throw new Error('vite did not start');
});
after(() => { server?.kill(); if (profile) rmSync(profile, { recursive: true, force: true }); });

function dump(query, w = 1440, h = 900, flags = [], budget = 60000) {
  const r = spawnSync(CHROME, ['--headless=new', '--no-sandbox', '--disable-gpu', `--user-data-dir=${profile}`, `--window-size=${w},${h}`,
    `--virtual-time-budget=${budget}`, ...flags, '--dump-dom', `http://127.0.0.1:${PORT}/harness/scenes.html?${query}`],
    { encoding: 'utf8', maxBuffer: 64 << 20, timeout: 170000 });
  assert.equal(r.status, 0, r.stderr);
  return r.stdout;
}
const report = (dom) => {
  const m = /<pre id="shift-report"[^>]*>([\s\S]*?)<\/pre>/.exec(dom);
  assert.ok(m && m[1].trim(), 'shift-report is empty');
  return JSON.parse(m[1].replaceAll('&quot;', '"').replaceAll('&amp;', '&').replaceAll('&lt;', '<').replaceAll('&gt;', '>'));
};
const text = (dom) => dom.slice(dom.indexOf('<div id="root">')).replace(/<[^>]*>/g, ' ').replace(/\s+/g, ' ');

for (const [w, h, theme, dir] of [[1440, 900, 'dark', 'ltr'], [1440, 900, 'light', 'ltr'], [1440, 900, 'dark', 'rtl'], [2560, 1528, 'dark', 'ltr'], [1100, 700, 'light', 'rtl']]) {
  test(`nothing shifts on data, scores, status, decisions, selection or folding: ${w}x${h} ${theme} ${dir}`, { skip }, () => {
    const r = report(dump(`shift=1&theme=${theme}&dir=${dir}`, w, h));
    assert.equal(r.error, undefined, r.error);
    assert.deepEqual(r.failures, [], `moved: ${JSON.stringify(r.failures.slice(0, 5))}`);
    assert.ok(r.compared > 500, `only ${r.compared} measurements`);
    assert.equal(r.maxDelta, 0, r.decision?.worst);
    assert.equal(r.decision.max, 0); assert.equal(r.selection.max, 0); assert.equal(r.scores.max, 0); assert.equal(r.hide.max, 0);
    assert.deepEqual(r.styleOffenders, []);
    assert.equal(r.layoutShift.value, 0, `the browser reported a layout shift: ${JSON.stringify(r.layoutShift.detail)}`);
    assert.equal(r.ok, true);
  });
}

test('the shift test fails on a style that moves things on hover and selection (negative control)', { skip }, () => {
  const r = report(dump('shift=1&break=1'));
  assert.equal(r.ok, false);
  assert.ok(r.styleOffenders.length > 0, 'the stylesheet audit found nothing');
  assert.ok(r.maxDelta >= 5 || r.failures.length > 0, `expected a delta, got ${r.maxDelta}`);
});

test('French moves no card, rail dot, block header or list box (en versus fr)', { skip }, () => {
  const en = report(dump('shift=1&theme=dark')).fixed;
  const fr = report(dump('shift=1&theme=dark', 1440, 900, ['--accept-lang=fr'])).fixed;
  for (const k of ['cards', 'dots', 'heads', 'thumbs', 'scroll']) assert.deepEqual(fr[k], en[k], `${k} differ between en and fr`);
  assert.ok(en.cards.length >= 5);
});

test('20,000 photos: scrolling the whole timeline mounts a bounded number of cards and rows', { skip }, () => {
  const r = report(dump('shift=1&size=big&theme=dark', 1440, 900, [], 120000));
  assert.equal(r.error, undefined, r.error);
  assert.equal(r.scroll.scenes > 4000, true, `${r.scroll.scenes} scenes`);
  assert.ok(r.scroll.total > 100000, `scroll height ${r.scroll.total}`);
  assert.ok(r.scroll.maxCards <= 80, `${r.scroll.maxCards} cards mounted`);
  assert.ok(r.scroll.maxRows <= 24, `${r.scroll.maxRows} rows mounted`);
  assert.equal(r.maxDelta, 0);
});

test('states: loading shows the grouping status, empty and error explain themselves, no-time notes its grouping', { skip }, () => {
  assert.match(text(dump('state=loading', 1440, 900, [], 4000)), /Grouping 7,200 of 20,400 photos/);
  assert.match(text(dump('state=empty', 1440, 900, [], 4000)), /No photos in this folder/);
  const err = text(dump('state=error', 1440, 900, [], 4000));
  assert.match(err, /Scenes could not be built/); assert.match(err, /Retry/); assert.match(err, /Show details/);
  assert.match(text(dump('state=notime', 1440, 900, [], 8000)), /Grouped by look and file name only/);
  assert.match(text(dump('state=scoring', 1440, 900, [], 8000)), /Scoring 3,100 of 20,400/);
  assert.match(text(dump('state=ready', 1440, 900, ['--accept-lang=fr'], 8000)), /Masquer les images seules/);
});

test('scene view: the Scene tab, frames, decision buttons and camera rows', { skip }, () => {
  const t = text(dump('view=scene&scene=1', 1440, 900, [], 8000));
  for (const word of ['Scene 2 of', 'Accept', 'Undecided', 'Reject', 'Frames of the scene', 'Sharpest first', 'Camera', 'Lens', 'Focal length', 'Aperture', 'Shutter', 'Next scene']) assert.match(t, new RegExp(word));
});

for (const [theme, dir, lang] of [['dark', 'ltr', []], ['light', 'ltr', []], ['dark', 'rtl', []], ['dark', 'ltr', ['--accept-lang=fr']]]) {
  test(`scene view: nothing moves on a frame, decision, score, status or scene change: ${theme} ${dir} ${lang.join('') || 'en'}`, { skip }, () => {
    const r = report(dump(`shift=1&view=scene&scene=1&theme=${theme}&dir=${dir}`, 1440, 900, lang));
    assert.equal(r.error, undefined, r.error);
    assert.deepEqual(r.failures, []);
    assert.ok(r.compared > 500, `only ${r.compared} measurements`);
    assert.equal(r.maxDelta, 0);
    assert.ok(r.log.length >= 10);
    assert.equal(r.ok, true);
  });
}

test('scene view: the shift test fails on a style that moves things (negative control)', { skip }, () => {
  const r = report(dump('shift=1&view=scene&scene=1&break=1&breakview=1'));
  assert.equal(r.ok, false);
});

test('scene view: the sharpest-region box, split and merge buttons and the crop outline exist and keep their size', { skip }, () => {
  const t = text(dump('view=scene&scene=1', 1440, 900, [], 8000));
  for (const word of ['Sharpest region', 'Z: 100%', 'Split here', 'Merge with next']) assert.match(t, new RegExp(word));
  assert.match(text(dump('view=scene&scene=1', 1440, 900, ['--accept-lang=fr'], 8000)), /Zone la plus nette/);
  const dom = dump('view=scene&scene=1', 1440, 900, [], 8000);
  assert.match(dom, /class="scene-sharp-rect"/, 'the crop outline is not on the stage photo');
});

for (const [theme, dir, lang] of [['dark', 'ltr', []], ['light', 'ltr', []], ['dark', 'rtl', []], ['dark', 'ltr', ['--accept-lang=fr']]]) {
  for (const view of ['rules', 'piles']) {
    test(`${view}: nothing moves on a preview, rule change, apply, undo, note, pile move or scroll: ${theme} ${dir} ${lang.join('') || 'en'}`, { skip }, () => {
      const r = report(dump(`shift=1&view=${view}&theme=${theme}&dir=${dir}`, 1440, 900, lang));
      assert.equal(r.error, undefined, r.error);
      assert.deepEqual(r.failures, []);
      assert.ok(r.compared > 300, `only ${r.compared} measurements`);
      assert.equal(r.maxDelta, 0);
      assert.ok(r.log.length >= 14);
      assert.equal(r.ok, true);
    });
  }
}

test('rules: the shift test fails on a style that moves things (negative control)', { skip }, () => {
  assert.equal(report(dump('shift=1&view=rules&break=1')).ok, false);
  assert.equal(report(dump('shift=1&view=piles&break=1')).ok, false);
  assert.equal(report(dump('shift=1&view=scene&scene=1&break=1')).ok, false, 'the crop box growing when there is no region must be caught');
});

test('rules panel texts: the panel, its preview counts and the piles review say what they do (en and fr)', { skip }, () => {
  const en = text(dump('view=piles', 1440, 900, [], 8000));
  for (const word of ['Scene rules', 'Keep best', 'per scene', 'Minimum quality', 'Single frames', 'Preview rules', 'Preview \\(nothing written yet\\)', 'Review piles', 'Apply', 'Undo', 'Keep all', 'Reject all']) assert.match(en, new RegExp(word));
  const fr = text(dump('view=piles', 1440, 900, ['--accept-lang=fr'], 8000));
  for (const word of ['Règles de scène', 'Garder les meilleures', 'Qualité minimale', 'Aperçu des règles', 'Revoir les piles', 'Appliquer', 'Tout garder', 'Tout rejeter']) assert.match(fr, new RegExp(word));
});

test('rules panel: a preview that changes nothing says so plainly and leaves Apply off (en and fr)', { skip }, () => {
  const en = dump('view=nothing', 1440, 900, [], 8000);
  assert.match(text(en), /would not keep or reject any frame, so there is nothing to apply/);
  assert.match(text(en), /6 frames are already rejected and stay as they are/);
  assert.match(en, /<button[^>]*class="btn primary"[^>]*disabled[^>]*>Apply</, 'Apply is enabled with nothing to apply');
  assert.match(text(dump('view=nothing', 1440, 900, ['--accept-lang=fr'], 8000)), /rien à appliquer/);
});

test('20,000 photos: the piles review mounts a bounded number of rows however far it is scrolled', { skip }, () => {
  const r = report(dump('shift=1&size=big&view=piles&theme=dark', 1440, 900, [], 170000));
  assert.equal(r.error, undefined, r.error);
  assert.ok(r.piles.scenes > 4000, `${r.piles.scenes} scenes`);
  assert.ok(r.piles.total > 400000, `piles height ${r.piles.total}`);
  assert.ok(r.piles.maxRows <= 14, `${r.piles.maxRows} rows mounted`);
  assert.equal(r.maxDelta, 0);
});
