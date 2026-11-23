// SPDX-License-Identifier: GPL-3.0-or-later
// Page-side cost of opening a large folder, with no engine: the work
// showFolder() (App.tsx) does per progress batch, and ThumbLoader (library.ts)
// draining its queue. usage: node benchmarking/large-catalog-js.mjs [N ...]
import { createRequire, registerHooks } from 'node:module';
import { existsSync, readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { withNodeLocales } from '../verify/vite-locales.mjs';
const require = createRequire(new URL('../app/package.json', import.meta.url));
const { transformSync } = await import(require.resolve('rolldown/experimental'));
globalThis.window = globalThis;
globalThis.createImageBitmap = async () => ({ width: 240, height: 160, close() {} });
globalThis.ImageData = class { constructor(d, w, h) { this.width = w; this.height = h; } };
globalThis.invoke = async (cmd) => (cmd === 'thumb_cached' ? new ArrayBuffer(8) : {});
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
const { byCaptureTime, ThumbLoader } = await import('../app/src/library.ts');

const entry = (i) => ({ imgid: i + 1, path: `/p/f${i % 40}/IMG_${i}.ARW`, filename: `IMG_${i}.ARW`, rating: 0, rejected: false,
  labels: 0, edited: false, width: 6000, height: 4000,
  exif: { maker: 'x', model: 'y', lens: 'z', exposure: 0.01, aperture: 2.8, iso: 100, focal_length: 35, exposure_bias: 0, focus_distance: 0,
          datetime: `2024:${String(1 + (i * 7919) % 12).padStart(2, '0')}:${String(1 + i % 28).padStart(2, '0')} 10:${String(i % 60).padStart(2, '0')}:00` } });
const ms = (t) => (performance.now() - t).toFixed(0);

for (const n of (process.argv.length > 2 ? process.argv.slice(2).map(Number) : [9653, 100000])) {
  const all = Array.from({ length: n }, (_, i) => entry(i));
  const batch = 64;
  // showFolder: per batch, copy and sort the whole list so far
  let t = performance.now(), got = [];
  for (let k = 0; k < n; k += batch) got = byCaptureTime([...got, ...all.slice(k, k + batch)]);
  console.log(`N=${n} accumulate+sort per batch of ${batch}: ${ms(t)} ms`);
  // the loader's request() per batch, then run() draining the disk path
  const byId = new Map(got.map((i) => [i.imgid, i]));
  // App.tsx indexes the list by imgid; LINEAR=1 measures the scan it replaced
  const loader = new ThumbLoader(process.env.LINEAR ? (id) => got.find((i) => i.imgid === id) : (id) => byId.get(id), () => 1 << 30);
  t = performance.now();
  for (let k = 0; k < n; k += batch) loader.request(all.slice(k, k + batch).map((i) => i.imgid));
  console.log(`N=${n} ThumbLoader.request per batch: ${ms(t)} ms`);
  t = performance.now();
  while (loader.progress().done < n) await new Promise((r) => setTimeout(r, 5));
  console.log(`N=${n} ThumbLoader drain of cached thumbnails: ${ms(t)} ms`);
}
