// SPDX-License-Identifier: GPL-3.0-or-later
// Opening a photo: where the time goes, from the app's own Session against the real
// darktable-engine, on generated photos only (no network, no library of yours).
//   node benchmarking/open-photo.mjs [--runs 8] [--scenarios cold,rapid,inflight,background,catalog]
//                                    [--engine build/bin/darktable-engine] [--json out.json] [--keep]
// Build the engine first (cmake --build build). It mirrors `openFileNow` in app/src/App.tsx: the old
// photo's close(), then Session.open, then the fit frame, the idle base (whole image, 3072 px) and the
// idle copy (100%) that App's scheduleBase / scheduleCopy ask for. Per open it reports, in ms from the
// click: close of the previous photo (what close() waited for), the engine's open_image (decode and
// develop history), the rest of Session.open (get_params, history, masks, spots, white balance), the
// first frame, the base frame and the copy frame. Page-side work (the thumbnail placeholder, canvas
// upload, histogram) is not here: it is synchronous in showOpening and costs no IPC.
import { createRequire, registerHooks } from "node:module";
import { copyFileSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir, loadavg, cpus } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const arg = (name, def) => { const i = process.argv.indexOf("--" + name); return i < 0 ? def : (process.argv[i + 1]?.startsWith("--") || i + 1 >= process.argv.length ? true : process.argv[i + 1]); };
const RUNS = Number(arg("runs", 8));
const SCENARIOS = String(arg("scenarios", "cold,rapid,inflight,background,catalog")).split(",");
const ENGINE = arg("engine", join(root, "build/bin/darktable-engine"));
const FILES = Number(arg("catalog", 10000));
const VIEW = { width: 1600, height: 1000 };
// --clear-timers: openFileNow clears the old photo's scheduleBase / scheduleCopy timers when the next
// open begins (the current App); without it they fire on the photo being left, as they did before
const CLEAR_TIMERS = arg("clear-timers", false) === true;

const require = createRequire(new URL("../app/package.json", import.meta.url));
const { transformSync } = await import(require.resolve("rolldown/experimental"));
const shim = new URL("./lib/open-shim.mjs", import.meta.url).href;
registerHooks({
  load(url, ctx, next) {
    if (!url.startsWith("file:") || !/\.tsx?$/.test(url)) return next(url, ctx);
    const path = fileURLToPath(url);
    return { format: "module", source: transformSync(path, readFileSync(path, "utf8"), { jsx: { runtime: "automatic" } }).code, shortCircuit: true };
  },
  resolve(spec, ctx, next) {
    if (spec.startsWith("@tauri-apps/")) return next(shim, ctx);
    if (/^\.\.?\//.test(spec) && !/\.[cm]?[jt]sx?$/.test(spec))
      for (const ext of [".ts", ".tsx"]) { const u = new URL(spec + ext, ctx.parentURL); if (existsSync(fileURLToPath(u))) return next(u.href, ctx); }
    return next(spec, ctx);
  },
});
globalThis.window = globalThis;
const store = new Map();
globalThis.localStorage = { getItem: (k) => store.get(k) ?? null, setItem: (k, v) => store.set(k, String(v)), removeItem: (k) => store.delete(k) };
globalThis.addEventListener ??= () => {};
globalThis.requestAnimationFrame = (f) => setTimeout(() => f(performance.now()), 0);
globalThis.cancelAnimationFrame = clearTimeout;
// --src DIR: another copy of app/src (a checkout of an earlier commit) to compare against
const SRC = arg("src", false) && arg("src") !== true ? arg("src") : join(root, "app/src");
const { Session } = await import(join(SRC, "session.ts"));
const { startEngine, stopEngine, request, requestTagged, log, trace, setHome, failures } = await import(shim);
// a rejection nobody awaits (a fire-and-forget command the engine refused) is counted, not fatal
const unhandled = [];
process.on("unhandledRejection", (e) => { unhandled.push(String(e?.message ?? e)); });

// ---- generated photos -------------------------------------------------------------------------
const work = mkdtempSync(join(tmpdir(), "immage-open-bench-"));
const photos = join(work, "photos");
mkdirSync(photos);
function tiff(path, width, height) {
  // uncompressed baseline RGB, with a gradient and a hash-like texture so decode and demosaic-free develop do real work
  const head = 8 + 2 + 12 * 11 + 4 + 6 + 8, data = head, size = width * height * 3;
  const b = Buffer.alloc(data + size);
  b.write("II"); b.writeUInt16LE(42, 2); b.writeUInt32LE(8, 4);
  const tags = [[256, 4, 1, width], [257, 4, 1, height], [258, 3, 3, head - 14], [259, 3, 1, 1], [262, 3, 1, 2], [273, 4, 1, data],
                [274, 3, 1, 1], [277, 3, 1, 3], [278, 4, 1, height], [279, 4, 1, size], [284, 3, 1, 1]];
  b.writeUInt16LE(tags.length, 8);
  tags.forEach(([t, ty, n, v], i) => { const o = 10 + 12 * i; b.writeUInt16LE(t, o); b.writeUInt16LE(ty, o + 2); b.writeUInt32LE(n, o + 4); ty === 3 && n === 1 ? b.writeUInt16LE(v, o + 8) : b.writeUInt32LE(v, o + 8); });
  b.writeUInt32LE(0, 10 + 12 * tags.length);
  [8, 8, 8].forEach((v, i) => b.writeUInt16LE(v, head - 14 + 2 * i));
  let s = 12345;
  for (let y = 0; y < height; y++) for (let x = 0; x < width; x++) {
    const o = data + 3 * (y * width + x); s = (s * 1103515245 + 12345) >>> 0;
    b[o] = (x * 255 / width + (s >>> 28)) & 255; b[o + 1] = (y * 255 / height + ((s >>> 24) & 15)) & 255; b[o + 2] = ((x + y) * 255 / (width + height) + ((s >>> 20) & 15)) & 255;
  }
  writeFileSync(path, b);
}
function jpeg(path, width, height) {
  // through the same textured TIFF, converted by ImageMagick
  const t = path + ".tmp.tif";
  tiff(t, width, height);
  const r = spawnSync("convert", [t, "-quality", "92", path]);
  rmSync(t, { force: true });
  if (r.status) throw Error(String(r.stderr));
}
const made = {};
function make(kind, count) {
  const ext = kind === "tiff" ? "tif" : "jpg", first = join(photos, `${kind}-0.${ext}`);
  if (!made[kind]) {
    if (kind === "tiff") tiff(first, 6000, 4000); else jpeg(first, kind === "big" ? 6000 : 1200, kind === "big" ? 4000 : 800);
    made[kind] = [first];
  }
  // distinct paths, so no open is a hit in the engine's own caches
  for (let i = made[kind].length; i < count; i++) { const p = join(photos, `${kind}-${i}.${ext}`); copyFileSync(first, p); made[kind].push(p); }
  return made[kind];
}

// ---- measuring --------------------------------------------------------------------------------
const pct = (a, p) => { const s = [...a].sort((x, y) => x - y); return s.length ? s[Math.min(s.length - 1, Math.ceil(s.length * p) - 1)] : NaN; };
const stat = (a) => { a = a.filter((x) => Number.isFinite(x)); return { n: a.length, med: +pct(a, .5).toFixed(1), p95: +pct(a, .95).toFixed(1), min: +Math.min(...a).toFixed(1), max: +Math.max(...a).toFixed(1) }; };
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const load = () => loadavg()[0].toFixed(2);
const until = async (cond, ms = 60000) => { const end = performance.now() + ms; while (!cond()) { if (performance.now() > end) throw Error("timeout"); await sleep(5); } };

// what App.openFileNow does for the pixels: the old photo closes, the new one opens, the fit frame
// paints, and the idle base (scheduleBase 500 ms) and copy (scheduleCopy 600 ms) follow once idle
const dbg = (...a) => process.env.OPEN_DEBUG && console.error(Math.round(performance.now()), ...a);
async function openPhoto(path, prev, { baseAndCopy = true } = {}) {
  const rec = { path };
  const t0 = performance.now();
  log.length = 0; trace.on = true;
  if (prev && CLEAR_TIMERS) prev.__stopTimers = true;
  if (prev) { const c0 = performance.now(); await prev.close(); rec.close = performance.now() - c0; } else rec.close = 0;
  dbg("close done", path); const opened0 = performance.now();
  let firstFrame = null, base = null, copy = null, full = null;
  const s = await Session.open(path, null, () => ({ ...VIEW }), (f) => { if (firstFrame === null) { firstFrame = performance.now(); full = { w: f.fullWidth, h: f.fullHeight }; rec.firstPixels = f.width + "x" + f.height; } }, (e) => { rec.error = e; });
  rec.sessionOpen = performance.now() - opened0; dbg("session open", rec.sessionOpen);
  s.thumbSize = () => ({ width: 3072, height: 3072 });
  s.onThumb = () => { base ??= performance.now(); };
  s.onCopy = (f) => { if (f) copy ??= performance.now(); };
  s.previewMode = () => "balanced";
  dbg("A"); const openCmd = log.find((e) => e.cmd === "open_image");
  rec.openImage = openCmd ? openCmd.t1 - openCmd.t0 : NaN;
  rec.openImageEngine = openCmd?.engineMs ?? NaN;
  rec.setup = rec.sessionOpen - rec.openImage;
  dbg("B"); await until(() => firstFrame !== null, 15000).catch((e) => { throw Error(e.message + " err=" + rec.error + " log=" + JSON.stringify(log.map((l) => [l.cmd, Math.round(l.t0), l.t1 && Math.round(l.t1)]))); });
  rec.firstFrame = firstFrame - t0;
  rec.firstFrameAfterOpen = firstFrame - (opened0 + rec.sessionOpen);
  rec.session = s;
  rec.t0 = t0;
  // the engine-side render time of the first frame
  const rc = log.find((e) => e.cmd === "render");
  rec.firstRender = rc ? rc.t1 - rc.t0 : NaN;
  rec.start = () => {
    // App's paintFrame: scheduleBase(500), scheduleCopy(600), each waiting for idle
    (async () => { await sleep(500); await until(() => s.idle); if (!s.__stopTimers) s.requestThumb(); })();
    (async () => { await sleep(600); await until(() => s.idle || base !== null); await until(() => s.idle); if (!s.__stopTimers && Math.min(full.w / VIEW.width, full.h / VIEW.height) > 1) s.requestCopy(1, full); })();
  };
  // a photo that fits the viewer at 100% has no copy: nothing more is asked for
  const wantsCopy = () => Math.min(full.w / VIEW.width, full.h / VIEW.height) > 1 && full.w * full.h <= 64e6;
  rec.waitAll = async () => {
    await until(() => base !== null);
    if (wantsCopy()) await until(() => copy !== null);
    await sleep(50); await s.settled();
    rec.base = base - t0; rec.copy = copy === null ? NaN : copy - t0;
  };
  rec.times = () => ({ base: base === null ? NaN : base - t0, copy: copy === null ? NaN : copy - t0 });
  rec.total = rec.firstFrame;
  return rec;
}
const phases = ["close", "openImage", "setup", "firstFrame", "total"];
function summarize(name, recs, extra = ["base", "copy"]) {
  const out = { scenario: name, load: load(), n: recs.length, refused: failures.length + unhandled.length };
  failures.length = unhandled.length = 0;
  for (const k of [...phases, "openImageEngine", "firstRender", ...extra]) out[k] = stat(recs.map((r) => r[k]));
  return out;
}
const results = [];
const note = (r) => { results.push(r); console.log(JSON.stringify(r)); };
const fresh = async (tag) => {
  const home = join(work, "engine-" + tag);
  startEngine(ENGINE, home, null, { gpu: arg("gpu", false) === true });
  setHome(home);
  await request({ cmd: "version" });
};
const settle = async (s) => { if (s) { await s.settled(); await sleep(50); } };

console.log(JSON.stringify({ host: `${cpus().length} cpus`, engine: ENGINE, runs: RUNS, loadStart: load() }));

// ---- scenarios --------------------------------------------------------------------------------
if (SCENARIOS.includes("cold")) for (const [kind, label] of [["small", "jpeg 1200x800"], ["big", "jpeg 6000x4000"], ["tiff", "tiff 6000x4000"]]) {
  const files = make(kind, RUNS + 1);
  await fresh("cold-" + kind);
  const recs = [];
  for (let i = 0; i < RUNS; i++) {
    // cold: no photo open before it, a path the engine has not seen; a run's base and copy are waited for
    const r = await openPhoto(files[i], null); r.start(); await r.waitAll(); recs.push(r); await r.session.close(); await sleep(200);
  }
  note(summarize(`cold ${label}`, recs));
  await stopEngine();
}

if (SCENARIOS.includes("rapid")) {
  for (const [kind, label] of [["big", "jpeg 6000x4000"], ["tiff", "tiff 6000x4000"]]) {
    const files = make(kind, 5 * RUNS + 3);
    await fresh("rapid-" + kind);
    const settled = [], rapid = [];
    let k = 0;
    for (let i = 0; i < RUNS; i++) {
      // A settled (base and copy done), then B: the normal case
      const a = await openPhoto(files[k++], null); a.start(); await a.waitAll();
      const b = await openPhoto(files[k++], a.session); settled.push(b); b.start(); await b.waitAll(); await b.session.close();
    }
    for (let i = 0; i < RUNS; i++) {
      // A->B->C each as soon as the one before has its first frame: the arrow key held down
      const a = await openPhoto(files[k++], null); a.start();
      const b = await openPhoto(files[k++], a.session); b.start();
      const c = await openPhoto(files[k++], b.session); c.start();
      rapid.push(b, c); await c.waitAll(); await c.session.close();
    }
    note(summarize(`switch ${label}, previous settled`, settled));
    note(summarize(`switch ${label}, rapid A>B>C (B and C)`, rapid, []));
    await stopEngine();
  }
}

if (SCENARIOS.includes("inflight")) for (const [kind, label] of [["big", "jpeg 6000x4000"], ["tiff", "tiff 6000x4000"]]) {
  const files = make(kind, 4 * RUNS + 2);
  await fresh("inflight-" + kind);
  for (const [what, ask] of [["base", (s, full) => s.requestThumb()], ["copy", (s, full) => s.requestCopy(1, full)]]) {
    const recs = [];
    for (let i = 0; i < RUNS; i++) {
      const a = await openPhoto(files[(what === "base" ? 0 : RUNS) + i], null);
      ask(a.session, { w: 6000, h: 4000 });
      // wait until the engine is really rendering it, then click on the next photo
      await until(() => a.session.baseRendering, 20000).catch(() => {});
      await sleep(150);
      const b = await openPhoto(files[2 * RUNS + (what === "base" ? 0 : RUNS) + i], a.session);
      recs.push(b); await settle(b.session); await b.session.close();
    }
    note(summarize(`open while ${what} render of the previous photo is in flight, ${label}`, recs, []));
  }
  await stopEngine();
}

async function background(files, thumbCount, stopFlag) {
  // the grid's thumbnail loader (a tagged thumb per cell, 12 at a time), the catalog scan (import batches)
  // and CLIP's analysis_preview, all off the main loop, as the app sends them
  const ids = [];
  for (let i = 0; i < files.length; i += 200) {
    const r = await requestTagged({ cmd: "import", files: files.slice(i, i + 200), job: 1, budget: 4000 });
    for (const im of r.images ?? []) ids.push(im.imgid);
    if (stopFlag.stop) break;
  }
  return ids;
}

if (SCENARIOS.includes("background")) {
  const files = make("big", RUNS * 4 + 4), small = make("small", 400);
  await fresh("background");
  const flag = { stop: false };
  const ids = await background(small, 400, flag);
  let thumbs = 0;
  const worker = async (w) => { for (let i = w; !flag.stop; i += 12) {
    const id = ids[i % ids.length]; if (id == null) { await sleep(10); continue; }
    await requestTagged({ cmd: "thumb", imgid: id, size: 256, proof: false, foreground: false, out: join(work, `t${w}.rgba`) }).catch(() => {}); thumbs++; } };
  const analysis = async () => { for (let i = 0; !flag.stop; i++) { await requestTagged({ cmd: "analysis_preview", imgid: ids[i % ids.length], subject: "", out: join(work, "a.rgba") }).catch(() => {}); } };
  const idle = [], busy = [];
  for (const [bucket, label] of [[idle, "idle"], [busy, "busy"]]) {
    if (bucket === busy) { flag.stop = false; for (let w = 0; w < 12; w++) void worker(w); void analysis(); void (async () => { while (!flag.stop) { await background(small, 0, flag); } })(); await sleep(1500); }
    const off = bucket === busy ? 2 * RUNS : 0;
    for (let i = 0; i < RUNS; i++) {
      const a = await openPhoto(files[off + 2 * i], null); a.start(); await a.waitAll().catch(() => {});
      const b = await openPhoto(files[off + 2 * i + 1], a.session); bucket.push(b); await settle(b.session); await b.session.close();
    }
  }
  flag.stop = true; await sleep(500);
  note(summarize("switch jpeg 6000x4000, engine idle", idle, []));
  note({ ...summarize("switch jpeg 6000x4000, thumbnails + import + analysis_preview running", busy, []), thumbsDuring: thumbs });
  await stopEngine();
}

if (SCENARIOS.includes("catalog")) {
  const big = make("big", RUNS + 2);
  await fresh("catalog");
  const tiny = join(photos, "tiny"); mkdirSync(tiny);
  jpeg(join(tiny, "t0.jpg"), 320, 213);
  for (let i = 1; i < FILES; i++) copyFileSync(join(tiny, "t0.jpg"), join(tiny, `t${i}.jpg`));
  const names = Array.from({ length: FILES }, (_, i) => join(tiny, `t${i}.jpg`));
  const t0 = performance.now();
  let n = 0;
  for (let i = 0; i < names.length; i += 500) { const r = await requestTagged({ cmd: "import", files: names.slice(i, i + 500), job: 2, budget: 8000 }); n += (r.images ?? []).length; }
  console.log(JSON.stringify({ catalogImport: { files: n, ms: Math.round(performance.now() - t0) } }));
  const recs = [];
  for (let i = 0; i < RUNS; i++) {
    const a = await openPhoto(big[i], null); a.start(); await a.waitAll().catch(() => {});
    const b = await openPhoto(big[i + 1], a.session); recs.push(b); await settle(b.session); await b.session.close();
  }
  note(summarize(`switch jpeg 6000x4000 with ${FILES} photos in the catalog`, recs, []));
  await stopEngine();
}

if (arg("json", false) && arg("json") !== true) writeFileSync(arg("json"), JSON.stringify({ runs: RUNS, results }, null, 1));
console.log(JSON.stringify({ loadEnd: load() }));
if (!arg("keep", false)) rmSync(work, { recursive: true, force: true });
process.exit(0);
