// SPDX-License-Identifier: GPL-3.0-or-later
// Engine-free unit tests of what a folder of tens of thousands of photos asks of the page: the
// thumbnail loader's queue (library.ts, ThumbLoader), the batch merge that keeps a read folder in
// capture order, and the signals that make the Catalog's thumbnail job wait while the page is used
// (thumbSignals.ts). benchmarking/large-catalog-js.mjs times the same code.
import test from 'node:test';
import assert from 'node:assert/strict';
import { registerHooks } from 'node:module';
import { existsSync, readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { fileURLToPath } from 'node:url';
import { withNodeLocales } from './vite-locales.mjs';
const require = createRequire(new URL('../app/package.json', import.meta.url));
const { transformSync } = await import(require.resolve('rolldown/experimental'));

// every Tauri command goes to this, which the tests set
const calls = [];
globalThis.window = globalThis;
globalThis.createImageBitmap = async () => ({ width: 240, height: 160, close() {} });
globalThis.ImageData = class { constructor(d, w, h) { this.width = w; this.height = h; } };
globalThis.invoke = async (cmd, args) => { calls.push([cmd, args]); return globalThis.reply?.(cmd, args) ?? {}; };
registerHooks({
  resolve(spec, ctx, next) {
    if (spec === '@tauri-apps/api/core') return { url: 'data:text/javascript,export const invoke=(...a)=>globalThis.invoke(...a);export class Channel{}', shortCircuit: true };
    if (spec === '@tauri-apps/api/event') return { url: 'data:text/javascript,export const listen=async()=>()=>{};', shortCircuit: true };
    if (spec === './immich' && ctx.parentURL?.endsWith('/library.ts')) return { url: 'data:text/javascript,export const fetchImmichThumb=async()=>null;', shortCircuit: true };
    if (spec.startsWith('./') || spec.startsWith('../')) for (const ext of ['.ts', '.tsx']) {
      const url = new URL(spec + ext, ctx.parentURL);
      if (existsSync(url)) return next(url.href, ctx);
    }
    return next(spec, ctx);
  },
  load(url, ctx, next) {
    if (!url.startsWith('file:') || !/\.tsx?$/.test(url)) return next(url, ctx);
    const path = fileURLToPath(url);
    return { format: 'module', source: transformSync(path, withNodeLocales(url, readFileSync(path, 'utf8')), { jsx: { runtime: 'automatic' } }).code, shortCircuit: true };
  },
});
const { ThumbLoader, byCaptureTime, mergeByCaptureTime } = await import('../app/src/library.ts');
const signals = await import('../app/src/thumbSignals.ts');

const entry = (i, datetime) => ({ imgid: i, path: `/p/IMG_${i}.ARW`, filename: `IMG_${i}.ARW`, rating: 0, rejected: false, labels: 0,
  edited: false, width: 1, height: 1, exif: { datetime: datetime ?? `2024:01:01 10:${String(i % 60).padStart(2, '0')}:00` } });
const settle = async (ms = 20) => { await new Promise((r) => setTimeout(r, ms)); };
// a disk that has none of the thumbnails and an engine that renders each at once
const engineOnly = () => { calls.length = 0; globalThis.reply = (cmd) => (cmd === 'thumb_cached' ? new ArrayBuffer(0) : cmd === 'thumb' ? new Uint32Array([240, 160, 0, 0]).buffer : {}); };
const rendered = () => calls.filter(([c]) => c === 'thumb').map(([, a]) => a.imgid);

test('merging a batch gives what sorting everything gives, ties and missing times included', () => {
  let seed = 7;
  const rnd = () => (seed = (seed * 1103515245 + 12345) & 0x7fffffff) / 0x7fffffff;
  for (let round = 0; round < 40; round++) {
    let sorted = [], all = [], id = 1;
    for (let b = 0; b < 12; b++) {
      const batch = Array.from({ length: Math.floor(rnd() * 20) }, () => {
        const r = rnd();
        return entry(id++, r < 0.15 ? '' : `2024:01:0${1 + Math.floor(r * 3)} 10:00:0${Math.floor(rnd() * 3)}`);
      });
      sorted = mergeByCaptureTime(sorted, batch);
      all = [...all, ...batch];
      assert.deepEqual(sorted.map((i) => i.imgid), byCaptureTime(all).map((i) => i.imgid), `round ${round} batch ${b}`);
    }
  }
});

test('the photos on screen load before the rest, whatever the order they were asked for', async () => {
  engineOnly();
  const all = Array.from({ length: 400 }, (_, i) => entry(i + 1));
  const byId = new Map(all.map((i) => [i.imgid, i]));
  const loader = new ThumbLoader((id) => byId.get(id), () => 1 << 30);
  loader.show([390, 391, 392], true);
  loader.request(all.map((i) => i.imgid));
  await settle(200);
  assert.deepEqual(rendered().slice(0, 3).sort((a, b) => a - b), [390, 391, 392]);
  assert.equal(new Set(rendered()).size, 400, 'each is rendered once');
});

test('what scrolled away before it was reached is never fetched', async () => {
  engineOnly();
  const all = Array.from({ length: 300 }, (_, i) => entry(i + 1));
  const byId = new Map(all.map((i) => [i.imgid, i]));
  const loader = new ThumbLoader((id) => byId.get(id), () => 1 << 30);
  loader.request(all.map((i) => i.imgid));
  loader.retain(new Set([10, 11, 12]));
  await settle(100);
  const got = new Set(rendered());
  assert.ok([10, 11, 12].every((id) => got.has(id)), 'the cells kept load');
  assert.ok(got.size < 30, `${got.size} fetched; the dropped ones should not be (only those already in flight)`);
  assert.equal(loader.progress().total <= 30, true);
});

test('asking for tens of thousands of thumbnails costs a time that grows with the count, not its square', async () => {
  globalThis.reply = (cmd) => (cmd === 'thumb_cached' ? new ArrayBuffer(8) : {});
  const n = 60000, all = Array.from({ length: n }, (_, i) => entry(i + 1));
  const byId = new Map(all.map((i) => [i.imgid, i]));
  const loader = new ThumbLoader((id) => byId.get(id), () => 1 << 30);
  const t = performance.now();
  for (let k = 0; k < n; k += 64) loader.request(all.slice(k, k + 64).map((i) => i.imgid));
  while (loader.progress().done < n) await settle(5);
  const took = performance.now() - t;
  // the linear queue took 20 s at 30,000 photos and four times that at 60,000
  assert.ok(took < 5000, `${n} thumbnails took ${took.toFixed(0)} ms`);
});

test('the Catalog job is told the page is in use only while a thumbnail job runs, and to go on after', async () => {
  const sent = [];
  globalThis.reply = (cmd, args) => { if (cmd.startsWith('thumbnails_')) sent.push([cmd, args]); return {}; };
  signals.probe.running = () => false;
  signals.noteUse();
  await settle();
  assert.deepEqual(sent, [], 'nothing is sent with no job');
  signals.probe.running = () => true;
  signals.noteUse();
  signals.noteUse();
  await settle();
  assert.deepEqual(sent, [['thumbnails_busy', { busy: true }]], 'busy once, however much use');
  await settle(signals.QUIET_MS + 200);
  assert.deepEqual(sent.at(-1), ['thumbnails_busy', { busy: false }]);
  sent.length = 0;
  signals.hintOnScreen(['/a.ARW', '/b.ARW']);
  signals.hintOnScreen(['/a.ARW', '/b.ARW', '/c.ARW']);
  await settle(450);
  assert.deepEqual(sent, [['thumbnails_hint', { paths: ['/a.ARW', '/b.ARW', '/c.ARW'] }]], 'the latest set, once');
  signals.hintOnScreen(['/a.ARW', '/b.ARW', '/c.ARW']);
  await settle(450);
  assert.equal(sent.length, 1, 'an unchanged set is not sent again');
});
