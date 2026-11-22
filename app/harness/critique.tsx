// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Dev-only harness for the critique lightbox: renders the real CullLightbox,
// CullSide and CullOverlay with fixture data in a plain browser, no Tauri.
//
//   /harness/critique.html?state=result&theme=dark&hover=row2
//
//   state  result | loading | error | errorkept | nokey | empty | cull
//   frame  mat (default) | flush
//   hover  row<N> | badge<N> (N is the region's number, 1 to 3; both set the same state)
//   pin    <N>: the region pinned by a click
//   theme  a built-in theme id (dark, graphite, slate, warm-ash, grey, classic-grey, soft-grey, light)
//   elapsed  seconds already waited, for state=loading (default 14)
//   break=1  adds a style that moves things on hover, to prove the shift test fails on it
//   #motion-report   {"animations": n}: document.getAnimations().length 1.5 s after load
//   shift=1   runs the no-layout-shift test and writes JSON into #shift-report
import "./tauriShim";
import { Camera } from "lucide-react";
import { createRoot } from "react-dom/client";
import { useEffect, useMemo, useState } from "react";
import "../src/styles.css";
import { BUILTIN_THEMES, THEME_TOKENS } from "../src/themes";
import type { CullDecision, GeminiCritique } from "../src/bindings";
import { CullLightbox, LightboxHead, StageArrows } from "../src/CullLightbox";
import { CullOverlay, CullSide, NO_SEL, type Crit, type Sel, type Tab } from "../src/CullInspector";
import { setCull } from "../src/cullStore";
import { decisionKey } from "../src/cullModel";
import type { ImageEntry } from "../src/library";
import { ZoomBar } from "../src/Toolbar";
import { BanIcon, StarIcon } from "../src/icons";
import { runShiftTest } from "./shiftTest";

const q = new URLSearchParams(location.search);
const state0 = q.get("state") ?? "result";
const theme = BUILTIN_THEMES.find((t) => t.id === q.get("theme")) ?? BUILTIN_THEMES[0];
for (const token of THEME_TOKENS) document.documentElement.style.setProperty(`--${token}`, theme.colors[token]);
document.documentElement.style.colorScheme = theme.colorScheme;
document.documentElement.dataset.theme = theme.id;
document.documentElement.dataset.scheme = theme.colorScheme;
document.body.style.background = "var(--window)";
if (q.get("break") === "1") {   // a deliberately bad style: the shift test must catch it (negative control)
  const st = document.createElement("style");
  st.textContent = `.cull-row:hover, .cull-row.on { padding-block: 6px; } .cull-pin:hover { width: 30px; }`;
  document.head.append(st);
}
if (q.get("frame") === "flush") {
  const st = document.createElement("style");
  st.textContent = `.culling-stage-inner { inset: 0 !important; } .culling-photo { border-radius: 0 !important; box-shadow: none !important; }`;
  document.head.append(st);
}

const IMAGE: ImageEntry = { imgid: 1, path: "/photos/burst_a2.jpg", filename: "burst_a2.jpg", rating: 0, rejected: false, labels: 0,
  edited: false, width: 2400, height: 1350, exif: {} as ImageEntry["exif"], version: 0 };

const CRITIQUE: GeminiCritique = {
  imgid: 1, model: "gemini-3.8-flash", cached: false,
  headline: "A strong leading line, let down by an artificial haze band",
  overallSummary: "The stone path and blooming heather make an inviting way into the scene, and the peaks give it scale. The midground haze looks layered on rather than natural, which pulls attention from the otherwise clean tonal range.",
  regions: [
    { label: "Foreground heather", positive: true, note: "Saturated purple blooms give the frame depth and a warm anchor.", box: [.01, .55, .40, .44] },
    { label: "Stone pathway", positive: true, note: "Textured steps lead the eye from foreground to the peaks.", box: [.46, .50, .30, .49] },
    { label: "Unnatural fog seam", positive: false, note: "The horizontal haze band looks artificial against the ridge.", box: [.10, .33, .52, .15] },
  ],
} as GeminiCritique;

const DECISION = {
  id: 1, sessionId: 1, path: IMAGE.path, version: 0, backing: [], proposed: "keep", finalStatus: null, status: "keep", reason: "bestInGroup",
  group: 2, bestPath: null, best: false, highlight: false, qualityScore: 0.8,
  factors: [
    { id: "sharp", label: "Sharpness", detail: "Sharp across the frame, softest at the top right.", impact: "supporting" },
    { id: "exp", label: "Exposure", detail: "No clipped highlights on the snow.", impact: "supporting" },
    { id: "dup", label: "Similar frames", detail: "Two near-duplicates; this one wins on detail.", impact: "context" },
  ],
  blurryRegion: { x: .6, y: .06, width: .3, height: .26 }, analysis: {} as CullDecision["analysis"], fileSize: 1, fileModifiedMs: 1, applied: false,
} as CullDecision;
setCull({ decisions: new Map([[decisionKey(IMAGE.path, 0), DECISION]]) });

const noop = () => {};
function critFor(state: string): Crit {
  const base = { run: noop, cancel: noop };
  switch (state) {
    case "loading": return { ...base, phase: "loading", critique: null, startedAt: Date.now() - Number(q.get("elapsed") ?? 14) * 1000 };
    case "error": return { ...base, phase: "error", critique: null, error: "Gemini: the request timed out after 60 seconds. Your photo was not changed. Check your connection, then try again." };
    case "errorkept": return { ...base, phase: "error", critique: CRITIQUE, error: "Gemini: the request timed out after 60 seconds. Check your connection, then try again." };
    case "nokey": return { ...base, phase: "error", prefs: true, critique: null, error: "no key" };
    case "empty": case "cull": return { ...base, phase: "idle", critique: null };
    default: return { ...base, phase: "idle", critique: CRITIQUE };
  }
}

function selFrom(): Sel {
  const h = q.get("hover") ?? "", p = Number(q.get("pin") ?? 0);
  const m = /^(?:row|badge|pin)(\d)$/.exec(h);
  return { hot: m ? Number(m[1]) - 1 : null, pinned: p ? p - 1 : null, blur: state0 === "cull" };
}

function Harness() {
  const [state, setState] = useState(state0);
  const [tab, setTab] = useState<Tab>(state0 === "cull" ? "cull" : "critique");
  const [sel, setSel] = useState<Sel>(selFrom);
  const [aspect, setAspect] = useState(16 / 9);
  const crit = useMemo(() => critFor(state), [state]);
  useEffect(() => {
    const img = new Image();
    img.onload = () => setAspect(img.naturalWidth / img.naturalHeight);
    img.src = "/harness/burst_a2.jpg";
  }, []);
  useEffect(() => {
    (window as unknown as { __h: unknown }).__h = { setState, setTab, setSel, NO_SEL };
    document.body.dataset.ready = "1";
    if (q.get("shift") === "1") void runShiftTest().then((r) => r, (e) => ({ ok: false, error: String(e?.stack ?? e) }))
      .then((r) => { const el = document.getElementById("shift-report")!; el.textContent = JSON.stringify(r); el.dataset.done = "1"; });
    const m = document.createElement("pre"); m.id = "motion-report"; m.hidden = true; document.body.append(m);
    const t = setTimeout(() => { m.textContent = JSON.stringify({ animations: document.getAnimations().length }); }, 1500);
    return () => clearTimeout(t);
  }, []);
  return <div className="modal-backdrop culling-backdrop" style={{ position: "fixed", inset: 0, display: "grid", placeItems: "center" }}>
    <CullLightbox
      head={<LightboxHead filename={IMAGE.filename} index={3} total={6} onEdit={noop} onKeywords={noop} onClose={noop} />}
      stage={<>
        <div style={{ flex: 1, minWidth: 0, minHeight: 0, display: "grid", placeItems: "center", containerType: "size" }}>
          <div className="culling-photo" style={{ position: "relative", width: `min(100cqw, 100cqh * ${aspect})`, aspectRatio: String(aspect) }}>
            <img src="/harness/burst_a2.jpg" alt="" style={{ display: "block", width: "100%", height: "100%", borderRadius: 3 }} />
            <CullOverlay critique={crit.critique} sel={sel} onSel={setSel} decision={DECISION} loading={state === "loading" && tab === "critique"} />
          </div>
        </div>
        <StageArrows hasPrevious hasNext onStep={noop} />
      </>}
      side={<CullSide image={IMAGE} tab={tab} onTab={setTab} crit={crit} sel={sel} onSel={setSel} onChanges={noop} onOpenPrefs={noop} />}
      foot={<div className="culling-bottom-bar">
        <div className="culling-rating-controls">
          <span className="tool-group"><button className="tool" aria-label="Reject"><BanIcon /></button>
            {[1, 2, 3, 4, 5].map((n) => <button key={n} className="tool" aria-label={`${n} stars`}><StarIcon filled={false} /></button>)}</span>
          <span className="tool-group">{["#d9534f", "#e0b020", "#3f9a52", "#3f7be0", "#9a5be0"].map((c) => <button key={c} className="swatch" style={{ background: c }} aria-label={c} />)}</span>
        </div>
        <span className="culling-status" />
        <div className="culling-view-controls">
          <button type="button" className="tool" aria-pressed="false" aria-label="Camera previews"><Camera size={16} aria-hidden="true" /></button>
          <ZoomBar disabled={false} register={noop} fit={() => 1} onZoom={noop} onToggle={noop} />
        </div>
      </div>} />
  </div>;
}

createRoot(document.getElementById("root")!).render(<Harness />);
