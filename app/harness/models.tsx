// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Dev-only harness for Preferences > General > AI models: renders the real AiModelsView (PackCard, the
// dialogs, the summary strip) with fixture data in a plain browser, no Tauri.
//
//   /harness/models.html?state=downloading&theme=dark&w=1000&h=700
//
//   state   notinstalled | diskfull | downloading | downloading1 | downloading99 | verifying | installed |
//           installedoff | installedpaused | installedidle | checking | checked | repair | offline | stopped |
//           update | unsupported | consent | remove | kwnotinstalled | kwdownloading | kwoff | kwworking | kwidle |
//           kwpaused | kwrepair | kwreview | kwreviewempty          (see modelsFixtures.ts)
//   theme   a built-in theme id (dark, graphite, slate, warm-ash, grey, classic-grey, soft-grey, light)
//   w, h    the pixel size of the page area (default: 80% of the window less the 190 px sidebar)
//   break=1 adds a style that lets the state slot grow with its message, to prove the shift test fails
//   shift=1 runs the no-layout-shift test and writes JSON into #shift-report
import "./tauriShim";
import { createRoot } from "react-dom/client";
import { useEffect, useState } from "react";
import "../src/styles.css";
import { BUILTIN_THEMES, THEME_TOKENS } from "../src/themes";
import { AiModelsView } from "../src/AiModelsPrefs";
import { STATES, type Fixture } from "./modelsFixtures";
import { runShiftTest } from "./modelsShift";

const q = new URLSearchParams(location.search);
const theme = BUILTIN_THEMES.find((t) => t.id === q.get("theme")) ?? BUILTIN_THEMES[0];
for (const token of THEME_TOKENS) document.documentElement.style.setProperty(`--${token}`, theme.colors[token]);
document.documentElement.style.colorScheme = theme.colorScheme;
document.documentElement.dataset.theme = theme.id;
document.documentElement.dataset.scheme = theme.colorScheme;
document.body.style.background = "var(--window)";
if (q.get("break") === "1") {   // a deliberately bad style: the shift test must catch it (negative control)
  const st = document.createElement("style");
  st.textContent = `.pack-slot { height: auto !important; } .pack-msg { height: auto !important; } .pack-msg-text { display: block !important; }`;
  document.head.append(st);
}
const noop = () => {};

function Harness() {
  const [name, setName] = useState(q.get("state") ?? "installed");
  const [fx, setFx] = useState<Fixture | null>(null);
  useEffect(() => { setFx(STATES[name] ?? STATES.installed); }, [name]);
  useEffect(() => {
    (window as unknown as { __h: unknown }).__h = { set: setName, states: Object.keys(STATES) };
    document.body.dataset.ready = "1";
    if (q.get("shift") === "1") void runShiftTest().then((r) => r, (e) => ({ ok: false, error: String(e?.stack ?? e) }))
      .then((r) => { const el = document.getElementById("shift-report")!; el.textContent = JSON.stringify(r); el.dataset.done = "1"; });
  }, []);
  const w = Number(q.get("w")) || Math.round(window.innerWidth * 0.8) - 190;
  const h = Number(q.get("h")) || Math.round(window.innerHeight * 0.8) - 54;
  if (!fx) return null;
  return <div style={{ position: "fixed", inset: 0, display: "grid", placeItems: "center" }}>
    <div className="prefs-scroll" style={{ width: w, height: h, flex: "none", background: "var(--panel)", borderRadius: "var(--radius-panel)" }} data-harness-page>
      <AiModelsView packs={fx.packs} work={fx.work} smartOn={fx.smartOn} keywordsOn={!!fx.keywordsOn} keywordsWork={fx.keywordsWork ?? null}
        keywords={fx.keywords ?? null} aiTags={fx.aiTags ?? []} onThreshold={noop} onReview={noop} onRemoveTags={() => Promise.resolve({ photos: 0, keywords: 0, failed: 0 })} flash={fx.flash ? { clip: fx.flash } : {}} busy={false} error=""
        dialog={fx.dialog ?? null} onAction={noop} onSwitch={noop} onPause={noop} onShowFolder={noop} onCulling={noop} onConsent={noop}
        onRemove={noop} onClose={noop} license={() => Promise.resolve("MIT License\n\nCopyright (c) 2021 OpenAI\n\nPermission is hereby granted, free of charge, to any person obtaining a copy of this software...")}
        onTest={noop} testLine="" />
    </div>
  </div>;
}

createRoot(document.getElementById("root")!).render(<Harness />);
