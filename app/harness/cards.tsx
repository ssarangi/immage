// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Dev-only harness for the Library panel's Metadata and Keywording cards while photos open one after
// another: the real cards, with the real hold (cardHold.ts), over a "Folders" marker that stands for
// everything below them. Photo A, B and C each open with a gap in which the entry and keywords are not
// in yet; the cards must stay mounted and the marker must not move. test=1 runs it and writes JSON into
// #cards-report; break=1 renders the cards as before the hold (the test must fail).
import "./tauriShim";
import { createRoot } from "react-dom/client";
import { useEffect, useState } from "react";
import "../src/styles.css";
import { BUILTIN_THEMES, THEME_TOKENS } from "../src/themes";
import { KeywordingPanel } from "../src/Keywording";
import { MetadataPanel } from "../src/MetadataPanel";
import { useCardHold } from "../src/cardHold";
import type { ImageEntry } from "../src/library";

const q = new URLSearchParams(location.search);
const theme = BUILTIN_THEMES[0];
for (const token of THEME_TOKENS) document.documentElement.style.setProperty(`--${token}`, theme.colors[token]);
document.documentElement.style.colorScheme = theme.colorScheme;
document.body.style.background = "var(--window)";
document.body.style.margin = "0";
const broken = q.get("break") === "1";

const photo = (n: number): ImageEntry => ({ imgid: n, path: `/p/IMG_${n}.NEF`, filename: `IMG_${n}.NEF`, rating: 0, rejected: false,
  labels: 0, edited: false, width: 6000, height: 4000, exif: { file_width: 6000, file_height: 4000, iso: 100, aperture: 2.8,
    exposure: 0.01, focal_length: 50, datetime: "2026:01:01 10:00:00", lens: "50mm", model: "Z6", maker: "Nikon" } as never });
const noop = () => {};

type S = { entry: ImageEntry | null; keywords: string[] | null; current: boolean };
function Harness() {
  const [s, setS] = useState<S>({ entry: null, keywords: null, current: false });
  const hold = useCardHold(s.entry, s.current, s.keywords);
  const entry = broken ? s.entry : hold.entry, keywords = broken ? (s.entry ? s.keywords : null) : hold.keywords;
  useEffect(() => {
    (window as unknown as { __set: (v: S) => void }).__set = setS;
    document.body.dataset.ready = "1";
    if (q.get("test") === "1") void run().then((r) => r, (e) => ({ ok: false, error: String(e?.stack ?? e) }))
      .then((r) => { const el = document.getElementById("cards-report")!; el.textContent = JSON.stringify(r); el.dataset.done = "1"; });
  }, []);
  return (
    <div className="app" style={{ position: "fixed", inset: 0, display: "block" }}>
      <div className="left" style={{ width: 320 }}>
        <MetadataPanel image={entry} text={null} onText={noop} onMeta={noop} onCaptureTime={noop} onLocationSaved={noop} open onToggle={noop} />
        <KeywordingPanel keywords={keywords} loading={hold.loading} known={[]} photoPath={entry?.path} open onToggle={noop} onAdd={noop} onRemove={noop} />
        <div id="folders" className="panel" style={{ height: 40 }}>Folders</div>
      </div>
    </div>
  );
}

const wait = () => new Promise<void>((done) => { setTimeout(done, 80); });
async function run() {
  const set = (window as unknown as { __set: (v: S) => void }).__set;
  const y = () => document.getElementById("folders")!.getBoundingClientRect().top;
  const cards = () => [!!document.querySelector(".panel.metadata"), !!document.querySelector(".panel.keywording")];
  const fail: string[] = [];
  const seen: number[] = [];
  const step = async (label: string, v: S) => {
    set(v); await wait();
    seen.push(y());
    if (!cards().every(Boolean)) fail.push(`${label}: a card is gone ${cards()}`);
  };
  await step("A open", { entry: photo(1), keywords: ["a", "b"], current: true });
  for (const n of [2, 3, 4]) {
    await step(`${n} opening`, { entry: null, keywords: null, current: true });     // not listed, keywords not read
    await step(`${n} entry`, { entry: photo(n), keywords: null, current: true });   // listed, keywords still reading
    await step(`${n} loaded`, { entry: photo(n), keywords: ["c", "d"], current: true });
  }
  const moved = Math.max(...seen) - Math.min(...seen);
  if (moved !== 0) fail.push(`the Folders marker moved by ${moved}px`);
  set({ entry: null, keywords: null, current: false }); await wait();
  const goneWhenClosed = !cards().some(Boolean);
  set({ entry: photo(5), keywords: null, current: true }); await wait();
  const disabled = document.querySelector<HTMLInputElement>(".kw-add input")?.disabled;
  return { ok: fail.length === 0 && goneWhenClosed && disabled === true, fail, moved, goneWhenClosed, disabled, steps: seen.length };
}

createRoot(document.getElementById("root")!).render(<Harness />);
