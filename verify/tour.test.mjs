// SPDX-License-Identifier: GPL-3.0-or-later
import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire, registerHooks } from 'node:module';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
const require = createRequire(new URL('../app/package.json', import.meta.url));
const { transformSync } = await import(require.resolve('rolldown/experimental'));
registerHooks({
  resolve(spec, ctx, next) {
    if (spec.startsWith('./') || spec.startsWith('../')) for (const ext of ['.ts', '.tsx']) {
      const url = new URL(spec + ext, ctx.parentURL);
      if (existsSync(url)) return next(url.href, ctx);
    }
    return next(spec, ctx);
  },
  load(url, ctx, next) {
    if (!url.startsWith('file:') || !/\.tsx?$/.test(url)) return next(url, ctx);
    const path = fileURLToPath(url);
    let source = readFileSync(path, 'utf8');
    // Vite expands eager locale imports in the app; expose the same catalogs in Node.
    if (url.endsWith('/i18n.ts')) {
      const dir = new URL('./locales/', url);
      const catalogs = Object.fromEntries(readdirSync(dir).filter(f => f.endsWith('.json'))
        .map(f => [`./locales/${f}`, JSON.parse(readFileSync(new URL(f, dir), 'utf8'))]));
      source = source.replace(/import\.meta\.glob<Dict>\([^;]+\)/, JSON.stringify(catalogs));
    }
    return { format: 'module', source: transformSync(path, source, { jsx: { runtime: 'automatic' } }).code, shortCircuit: true };
  },
});
const { STEPS, actAt } = await import('../app/src/tourSteps.ts');
const read = (p) => readFileSync(new URL(`../app/src/${p}`, import.meta.url), 'utf8');
const en = JSON.parse(read('locales/en.json'));
const fr = JSON.parse(read('locales/fr.json'));

test('a press on a tour button is found by position, not by its target', () => {
  const boxes = [
    { act: 'skip', left: 10, top: 10, right: 50, bottom: 30 },
    { act: 'next', left: 60, top: 10, right: 100, bottom: 30 },
  ];
  assert.equal(actAt(boxes, 20, 20), 'skip');
  assert.equal(actAt(boxes, 100, 30), 'next');
  assert.equal(actAt(boxes, 55, 20), null);
  assert.equal(actAt([], 20, 20), null);
});

test('the tour acts on the buttons by position and keeps its clicks', () => {
  const tour = read('Tour.tsx');
  for (const a of ['skip', 'back', 'next', 'hide']) assert.ok(tour.includes(`data-act="${a}"`), a);
  assert.match(tour, /window\.addEventListener\("pointerup", up, true\)/);
  assert.ok(read("styles.css").includes("prefers-reduced-motion: reduce) {\n  .tour,"));
});

test('every step points at a part of the window that carries its data-tour', () => {
  const app = read('App.tsx') + read('Toolbar.tsx');
  const targets = new Set();
  for (const s of STEPS) {
    if (s.target) targets.add(s.target);
    (s.also ?? []).forEach(a => targets.add(a));
    (s.callouts ?? []).forEach(c => targets.add(c.target));
  }
  for (const t of targets) assert.ok(app.includes(`data-tour="${t}"`), t);
});

test('the tour covers Find, Culling, Statistics and AI models, in the real labels', () => {
  const titles = STEPS.map(s => s.title);
  for (const k of ['find', 'cull', 'stats']) assert.ok(titles.includes(en[`dialogs.tour.${k}.title`]), k);
  const find = en['dialogs.tour.find.body'];
  for (const w of [en['library.search.smart'], en['library.search.filename'], en['cull.find.ask']]) assert.ok(find.includes(w), w);
  const cull = en['dialogs.tour.cull.body'];
  assert.ok(cull.includes('Cull') && cull.includes('Preferences > Culling'));
  // the keys it names are the ones the menus and chords carry
  const menu = read('menu.ts');
  assert.match(menu, /item\("critique", t\("menu\.critique"\), "⇧⌘G"\)/);
  assert.match(menu, /item\("grid", t\("menu\.grid"\), "G"\)/);
  assert.match(menu, /item\("culling-preview", t\("menu\.culling-preview"\), "F"\)/);
  assert.ok(cull.includes('(G)') && cull.includes('F ') && cull.includes('⇧⌘G'));
  assert.match(read('actions.ts'), /e\.code === "KeyG", \(\) => "critique"/);
  assert.match(en['dialogs.tour.stats.body'], /View > Statistics/);
  assert.ok(en['menu.stats'] === 'Statistics' && menu.includes('item("stats"'));
  assert.match(en['dialogs.tour.preferences.body'], /AI models/);
  assert.equal(en['aimodels.title'], 'AI models');
});

test('the tour stays short and is translated', () => {
  assert.ok(STEPS.length <= 11);
  const keys = [...read('tourSteps.ts').matchAll(/t\("(dialogs\.tour\.[\w.]+)"\)/g)].map(m => m[1]);
  assert.ok(keys.length > 20);
  for (const k of keys) {
    assert.ok(en[k], `en ${k}`);
    assert.ok(fr[k], `fr ${k}`);
    if (k.endsWith('.body')) assert.ok(en[k].length <= 260, `${k} is ${en[k].length} characters`);
  }
});
