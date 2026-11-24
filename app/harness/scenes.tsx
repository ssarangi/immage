// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Dev-only harness for Scenes: the real ScenesView, SceneLightbox, CullLightbox and CullSide over synthetic
// photos (generated pictures, fake paths: nothing on disk is read), in a plain browser, no Tauri.
//
//   /harness/scenes.html?state=ready&theme=dark&dir=ltr&view=timeline
//
//   view    timeline (default) | scene | rules (the rules panel beside the timeline) | piles (rules with the piles review open)
//   state   ready | loading | updating | empty | error | notime | scoring | unscored
//   size    small (default, 400 photos) | big (20,000 photos in about 2,500 scenes)
//   theme   a built-in theme id (dark, light, ...)
//   dir     ltr | rtl
//   scroll  pixels to scroll the timeline to (state=notime scrolls to the no-time block itself)
//   fold    a block index to fold
//   break=1 adds a style that moves things on hover and selection (the shift test must catch it)
//   shift=1 runs the no-layout-shift test and writes JSON into #shift-report
import "./tauriShim";
import { createRoot } from "react-dom/client";
import { useEffect, useMemo, useState } from "react";
import "../src/styles.css";
import { BUILTIN_THEMES, THEME_TOKENS } from "../src/themes";
import type { ImageEntry } from "../src/library";
import { CullLightbox, LightboxHead, StageArrows } from "../src/CullLightbox";
import { CullSide, NO_SEL, type Crit, type Tab } from "../src/CullInspector";
import { ScenesView } from "../src/ScenesView";
import { ViewTabs } from "../src/ViewTabs";
import { SceneRules } from "../src/SceneRules";
import { useSceneLightbox } from "../src/SceneLightbox";
import { setThumbSource } from "../src/SceneThumb";
import { buildModel, frameKey, GREEN, layout } from "../src/scenes";
import { openScene, scenesState, setScenes } from "../src/scenesStore";
import { runRulesShift, runSceneViewShift, runScenesShift } from "./scenesShift";
import "../src/scenes.css";

const q = new URLSearchParams(location.search);
const theme = BUILTIN_THEMES.find((t) => t.id === q.get("theme")) ?? BUILTIN_THEMES[0];
for (const token of THEME_TOKENS) document.documentElement.style.setProperty(`--${token}`, theme.colors[token]);
document.documentElement.style.colorScheme = theme.colorScheme;
document.documentElement.dataset.theme = theme.id;
document.documentElement.dataset.scheme = theme.colorScheme;
document.documentElement.dir = q.get("dir") === "rtl" ? "rtl" : "ltr";
document.body.style.background = "var(--window)";
if (q.get("break") === "1") {   // a deliberately bad style: the shift test must catch it (negative control)
  const st = document.createElement("style");
  st.textContent = `.scene-card:hover, .scene-card.active, .scene-card[aria-selected="true"] { margin: 5px; border-width: 4px; } .scene-badge { width: 30px; }
    .scene-decide button[aria-checked="true"] { height: 40px; } .scene-tile[aria-current="true"] { width: 130px; }
    .scene-crop-box:has(.scene-crop-empty) { height: 160px; } .scene-rule-counts[data-stale] { height: 40px; } .scene-pile-tile:hover { margin: 4px; }`;
  document.head.append(st);
}

// ---- fixtures: a deterministic synthetic shoot ----
const CAMERAS = [
  { model: "ILCE-7M4", lens: "FE 70-200mm F2.8 GM OSS II", focal: 200, aperture: 4, exposure: 1 / 1000, iso: 400 },
  { model: "Canon EOS R5 Mark II", lens: "RF 100-500mm F4.5-7.1 L IS USM with a very long name", focal: 500, aperture: 7.1, exposure: 1 / 2000, iso: 3200 },
  { model: "X-T5", lens: "XF 16-55mm F2.8", focal: 23, aperture: 5.6, exposure: 1 / 125, iso: 160 },
  { model: "", lens: "", focal: 0, aperture: 0, exposure: 0, iso: 0 },       // no EXIF at all
];
const pad = (n: number, w = 2) => String(n).padStart(w, "0");
const when = (ms: number) => { const d = new Date(ms); return `${d.getUTCFullYear()}:${pad(d.getUTCMonth() + 1)}:${pad(d.getUTCDate())} ${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}:${pad(d.getUTCSeconds())}`; };

function fixture(photos: number, noTime = 0) {
  const entries: ImageEntry[] = [];
  const blocks: { start: number; scenes: number[][] }[] = [];
  let id = 1;
  let t = Date.UTC(2026, 2, 5, 7, 10, 0);
  let block: { start: number; scenes: number[][] } | null = null;
  let sceneNo = 0;
  while (entries.length < photos - noTime) {
    if (!block || sceneNo % 6 === 0) { t += 45 * 60_000 + (sceneNo % 24 === 0 ? 20 * 3_600_000 : 0); block = { start: t, scenes: [] }; blocks.push(block); }
    const size = [1, 7, 8, 3, 7, 2][sceneNo % 6];
    const cam = CAMERAS[(sceneNo >> 1) % CAMERAS.length];
    const frames: number[] = [];
    for (let k = 0; k < size && entries.length < photos - noTime; k++) {
      const i = entries.length;
      frames.push(i);
      entries.push({
        imgid: id++, path: `/synthetic/shoot/IMG_${pad(i + 1, 5)}.jpg`, filename: `IMG_${pad(i + 1, 5)}.jpg`, rating: i % 7 === 0 ? 3 : 0,
        rejected: false, labels: 0, edited: false, width: 6000, height: 4000, version: 0,
        exif: { maker: "", model: cam.model, lens: cam.lens, exposure: cam.exposure, aperture: cam.aperture, iso: cam.iso, focal_length: cam.focal,
          exposure_bias: 0, focus_distance: 0, datetime: when(t + k * 400), capture_time_raw: when(t + k * 400) + ".25" },
      } as ImageEntry);
    }
    block.scenes.push(frames);
    t += 90_000 + sceneNo * 1000;
    sceneNo++;
  }
  const withoutTime: number[][] = [];
  for (let i = 0; i < noTime; i++) {
    const n = entries.length;
    entries.push({ imgid: id++, path: `/synthetic/shoot/scan_${pad(i, 4)}.jpg`, filename: `scan_${pad(i, 4)}.jpg`, rating: 0, rejected: false, labels: 0,
      edited: false, width: 3000, height: 2000, version: 0, exif: { datetime: "" } as ImageEntry["exif"] } as ImageEntry);
    withoutTime.push([n]);
  }
  const all = blocks.map((b) => ({ startMs: b.start, endMs: b.start + 1000, noTime: false, approx: false, scenes: b.scenes.map((f) => ({ frames: f, unhashed: 0 })) }));
  if (noTime) all.push({ startMs: null as unknown as number, endMs: null as unknown as number, noTime: true, approx: false, scenes: withoutTime.map((f) => ({ frames: f, unhashed: 0 })) });
  // real times of each block, for the header: the first and last frame
  all.forEach((b) => { if (!b.noTime) { const fs = b.scenes.flatMap((s) => s.frames); b.startMs = parseWhen(entries[fs[0]].exif.datetime); b.endMs = parseWhen(entries[fs[fs.length - 1]].exif.datetime); } });
  return { entries, build: { blocks: all } };
}
function parseWhen(s: string) { const m = /^(\d+):(\d+):(\d+) (\d+):(\d+):(\d+)/.exec(s)!; return Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]); }

// a picture per frame, from its name: a gradient and a few shapes, in a 4:3 or 3:2 frame
function picture(entry: ImageEntry): Promise<ImageBitmap | null> {
  const n = entry.imgid;
  const w = 240, h = n % 5 === 0 ? 240 : 160;
  const c = new OffscreenCanvas(w, h);
  const g = c.getContext("2d")!;
  const hue = (n * 47) % 360;
  const grad = g.createLinearGradient(0, 0, w, h);
  grad.addColorStop(0, `hsl(${hue} 55% 38%)`); grad.addColorStop(1, `hsl(${(hue + 60) % 360} 60% 62%)`);
  g.fillStyle = grad; g.fillRect(0, 0, w, h);
  g.fillStyle = `hsla(${(hue + 180) % 360} 50% 85% / .8)`;
  g.beginPath(); g.arc(w * (0.3 + (n % 4) * 0.12), h * 0.55, 18 + (n % 3) * 6, 0, 7); g.fill();
  g.fillStyle = "rgba(0,0,0,.35)"; g.fillRect(0, h * 0.78, w, h * 0.22);
  return createImageBitmap(c);
}
setThumbSource(picture);

const big = q.get("size") === "big";
const data = fixture(big ? 20_000 : 400, q.get("state") === "notime" ? 40 : 0);

function seed() {
  const state = q.get("state") ?? "ready";
  const items = data.entries.map((e) => ({ path: e.path, version: 0, name: e.filename, takenAt: e.exif.datetime, fromFile: false }));
  const model = buildModel(data.build as Parameters<typeof buildModel>[0]);
  const quality = data.entries.map((_, i) => (state === "unscored" || state === "scoring" ? (i % 3 === 0 ? ((i * 37) % 100) / 100 : null) : ((i * 37) % 100) / 100));
  // a few decisions, so the badges and counts show
  const accepted = new Set<string>();
  data.entries.forEach((e, i) => {
    if (i % 11 === 1) { e.labels |= GREEN; accepted.add(frameKey(e.path, 0)); }
    if (i % 13 === 2) e.rejected = true;
  });
  // the sharpest regions: most frames have one, every third has none (as an unscored frame)
  const regions = data.entries.map((_, i) => (i % 3 === 1 ? null : { x: 0.5 + (i % 4) * 0.05, y: 0.25 + (i % 3) * 0.1, width: 0.3, height: 0.3 }));
  const base = { on: true, items, model, quality, regions, accepted, status: "ready" as const, network: false, hashed: items.length, unhashed: state === "scoring" ? 12 : 0 };
  if (state === "loading") setScenes({ on: true, status: "building", model: null, progress: { done: 7200, total: 20400 } });
  else if (state === "empty") setScenes({ ...base, items: [], model: buildModel({ blocks: [] }), quality: [] });
  else if (state === "error") setScenes({ on: true, status: "error", model: null, error: "scenes build failed: the cached thumbnails could not be read (synthetic harness error)" });
  else setScenes({
    ...base,
    updating: state === "updating", progress: state === "updating" ? { done: 10, total: 100 } : null,
    scoring: state === "scoring" ? { done: 3100, total: 20400, paused: false } : null,
  });
  if (q.get("fold")) setScenes({ collapsed: new Set([Number(q.get("fold"))]) });
}
seed();

const noop = () => {};
const CRIT: Crit = { phase: "idle", critique: null, run: noop, cancel: noop };

// what the rules would propose, without Rust: a scene's first frame is kept as its best, the second kept, the rest rejected
function fakeVerdicts() {
  const model = scenesState().model!;
  return model.scenes.map((sc) => sc.frames.map((_, k) => (sc.frames.length === 1 ? "undecidedSingle" : k === 0 ? "keepBest" : k === 1 ? "keepKept" : "rejectDuplicate")));
}
function fakePreview() {
  const verdicts = fakeVerdicts();
  const flat = verdicts.flat();
  const n = (p: string) => flat.filter((v) => v.startsWith(p)).length;
  return { counts: { keep: n("keep"), reject: n("reject"), undecided: n("undecided"), alreadyRejected: 0 }, verdicts } as never;
}
(window as unknown as { __fake: unknown }).__fake = { fakeVerdicts, fakePreview };

function SceneView({ entries, onMeta }: { entries: ImageEntry[]; onMeta: (e: ImageEntry, f: Partial<ImageEntry>) => void }) {
  const scene = useSceneLightbox(entries, onMeta);
  const [photo, setPhoto] = useState<number | null>(null);
  const [tab, setTab] = useState<Tab>("scene");
  useEffect(() => { if (scene) setPhoto((p) => (p !== null && scene.frames.some((f) => f.imgid === p) ? p : scene.startId)); }, [scene?.key]);   // eslint-disable-line react-hooks/exhaustive-deps
  const current = scene?.frames.find((f) => f.imgid === photo) ?? scene?.frames[0];
  if (!scene || !current) return null;
  return <div className="modal-backdrop culling-backdrop" style={{ position: "fixed", inset: 0, display: "grid", placeItems: "center" }}>
    <CullLightbox
      head={<LightboxHead filename={current.filename} index={scene.frames.indexOf(current) + 1} total={scene.frames.length} onEdit={noop} onKeywords={noop} onClose={noop} context={scene.context} />}
      stage={<>
        <div style={{ flex: 1, minWidth: 0, minHeight: 0, display: "grid", placeItems: "center", containerType: "size" }}>
          <div className="culling-photo" style={{ position: "relative", width: "min(100cqw, 100cqh * 1.5)", aspectRatio: "1.5" }}>
            <div style={{ width: "100%", height: "100%", background: "linear-gradient(135deg, #3a6ea5, #d98b4a)", borderRadius: 3 }} />
            {scene.overlay(current)}
          </div>
        </div>
        <StageArrows hasPrevious={scene.hasPrevious} hasNext={scene.hasNext} onStep={(d) => scene.onScene(d)} />
      </>}
      side={<CullSide image={current} tab={tab} onTab={setTab} crit={CRIT} sel={NO_SEL} onSel={noop} onChanges={noop} onOpenPrefs={noop} scenePanel={scene.panel(current)} />}
      foot={<>{scene.strip(current, setPhoto)}<div className="culling-bottom-bar"><span className="culling-status" /></div></>} />
  </div>;
}

function Harness() {
  const [entries, setEntries] = useState(data.entries);
  const view = q.get("view") ?? "timeline";
  const onMeta = (e: ImageEntry, f: Partial<ImageEntry>) => setEntries((all) => all.map((x) => (x === e ? { ...x, ...f } : x)));
  const shown = useMemo(() => entries, [entries]);
  useEffect(() => {
    (window as unknown as { __h: unknown }).__h = { setEntries, scenesState, setScenes, onMeta, data };
    document.body.dataset.ready = "1";
    const scroller = document.querySelector<HTMLElement>(".scenes-scroll");
    if (q.get("scroll") && scroller) scroller.scrollTop = Number(q.get("scroll"));
    // no-time state: bring the no-time block's header into view
    if (q.get("state") === "notime" && scroller) {
      const model = scenesState().model!;
      const row = layout(model, scroller.clientWidth, false, new Set()).rows.find((r) => r.kind === "block" && model.blocks[r.block].noTime);
      if (row) scroller.scrollTop = row.top - 40;
    }
    if (view === "scene") openScene(Number(q.get("scene") ?? 1));
    if (view === "rules" || view === "piles") setScenes({ preview: fakePreview(), previewStale: false, piles: view === "piles" });
    // a preview where the rules change nothing but some frames were already rejected: the panel says so and Apply stays off
    if (view === "nothing") setScenes({ preview: { counts: { keep: 0, reject: 0, undecided: 394, alreadyRejected: 6, demotedForEyes: 0 }, verdicts: [] }, previewStale: false });
    if (q.get("shift") === "1") void (view === "scene" ? runSceneViewShift() : view === "rules" || view === "piles" ? runRulesShift(view === "piles") : runScenesShift()).then((r) => r, (e) => ({ ok: false, error: String(e?.stack ?? e) }))
      .then((r) => { const el = document.getElementById("shift-report")!; el.textContent = JSON.stringify(r); el.dataset.done = "1"; });
  }, []);   // eslint-disable-line react-hooks/exhaustive-deps
  return <>
    <div className="harness-app" style={{ position: "fixed", inset: 0, display: "flex", flexDirection: "column" }}>
      <div className="harness-toolbar" style={{ flex: "none", height: 44, display: "flex", alignItems: "center", gap: 12, padding: "0 16px", background: "var(--panel)" }}>
        <ViewTabs view="scenes" onView={noop} canScenes />
      </div>
      <div className="strip-slot grid" style={{ position: "relative", inset: "auto", flex: 1, minHeight: 0, display: "block" }}>
        <div style={{ position: "relative", height: "100%" }}>
          <div className="scenes-host"><ScenesView entries={shown} onMeta={onMeta} autoBuild={false} /></div>
        </div>
        {(view === "rules" || view === "piles" || view === "nothing") && <aside className="right" style={{ position: "absolute", insetBlock: 0, insetInlineEnd: 0, width: 340, display: "flex", flexDirection: "column", background: "var(--panel)", zIndex: 5 }}>
          <SceneRules entries={shown} onChanges={noop} /></aside>}
      </div>
    </div>
    {view === "scene" && <SceneView entries={shown} onMeta={onMeta} />}
  </>;
}

createRoot(document.getElementById("root")!).render(<Harness />);
