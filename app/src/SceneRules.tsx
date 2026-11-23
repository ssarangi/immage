// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Scenes rules panel, in the grid's right-hand column while Scenes shows: keep the best N frames of each
// scene, a minimum quality, ratings that always stay, and what to do with single frames. Nothing is written
// until Apply: Preview asks Rust what the rules would do (scenes_rules_preview), Review piles shows it scene by
// scene, and Apply records it as a cull session and applies it with the same cull_apply as Auto Cull, so Undo,
// the history and the photos' badges are the ones Auto Cull has. Every box is the same size before and after a
// preview, so numbers arriving move nothing.
import { useState } from "react";
import { commands, type CullChange } from "./bindings";
import type { ImageEntry } from "./library";
import { rulesScenes } from "./scenes";
import { previewRules, resetRules, setRules, setScenes, useScenes } from "./scenesStore";
import { useCtx } from "./ScenesView";
import { formatNumber, t } from "./i18n";

export function SceneRules({ entries, onChanges }: { entries: ImageEntry[]; onChanges: (changes: CullChange[]) => void }) {
  const s = useScenes();
  const { ctx } = useCtx(entries);
  const [busy, setBusy] = useState(false);
  const r = s.rules;
  const ready = s.model !== null && s.status === "ready" && s.model.frameCount > 0;
  const fresh = s.preview !== null && !s.previewStale;
  const session = s.ruleSession;
  const applied = session?.state === "applied";
  const counts = s.preview?.counts;
  const num = (n: number | undefined) => (n === undefined ? "-" : formatNumber(n));
  const folder = (s.items[0]?.path ?? "").split(/[\\/]/).slice(-2, -1)[0] ?? "";

  async function apply() {
    if (!s.model || !fresh || busy || applied) return;
    setBusy(true); setScenes({ ruleNote: null });
    try {
      const plan = await commands.scenesRulesPlan(t("cull.scenes.rules.sessionLabel", { folder }),
        { scenes: rulesScenes(s.model, s.items, ctx, s.forced), rules: s.rules });
      const done = await commands.cullApply(plan.session.id, { reject: true, highlightRating: null, highlightLabel: null });
      onChanges(done.changes);
      setScenes({ ruleSession: { id: plan.session.id, state: "applied" }, previewStale: true,
        ruleNote: { text: t("cull.scenes.rules.applied", { n: done.rejected, count: formatNumber(done.rejected) }), error: false } });
    } catch (e) { setScenes({ ruleNote: { text: t("cull.panel.failed", { error: String(e) }), error: true } }); }
    finally { setBusy(false); }
  }
  async function undo() {
    if (!session || !applied || busy) return;
    setBusy(true); setScenes({ ruleNote: null });
    try {
      const done = await commands.cullUndo(session.id);
      onChanges(done.changes);
      setScenes({ ruleSession: { id: session.id, state: "undone" }, previewStale: true,
        ruleNote: { text: t("cull.panel.undone", { n: formatNumber(done.changes.length) }), error: false } });
    } catch (e) { setScenes({ ruleNote: { text: t("cull.panel.failed", { error: String(e) }), error: true } }); }
    finally { setBusy(false); }
  }

  return <div className="find-pop cull-panel scene-rules" role="region" aria-label={t("cull.scenes.rules.title")}>
    <div className="panel-head"><h1>{t("cull.scenes.rules.title")}</h1>
      <div className="head-tools">
        <button type="button" className="btn small" onClick={resetRules}>{t("cull.scenes.rules.reset")}</button>
      </div></div>
    <div className="find-scroll cull-scroll">
      <div className="cull-block">
        <div className="cull-session-row">
          <label htmlFor="scene-keep-best">{t("cull.scenes.rules.keepBest")}</label>
          <input id="scene-keep-best" className="field scene-num" type="number" min={0} max={9} step={1} value={r.keepBest}
            onChange={(e) => setRules({ keepBest: Number(e.target.value) })} />
          <span className="scene-rule-unit">{t("cull.scenes.rules.perScene")}</span>
        </div>
        <p className="note-plain">{t("cull.scenes.rules.keepBestHint")}</p>
        <div className="cull-session-row">
          <label htmlFor="scene-min-quality">{t("cull.scenes.rules.minQuality")}</label>
          <input id="scene-min-quality" className="scene-range" type="range" min={0} max={100} step={1} value={r.minQuality}
            onChange={(e) => setRules({ minQuality: Number(e.target.value) })} />
          <output className="scene-rule-value" htmlFor="scene-min-quality"><bdi dir="ltr">{r.minQuality === 0 ? t("cull.scenes.rules.off") : formatNumber(r.minQuality)}</bdi></output>
        </div>
        <div className="cull-session-row">
          <label className="scene-check"><input type="checkbox" checked={r.alwaysKeep} onChange={(e) => setRules({ alwaysKeep: e.target.checked })} />
            <span>{t("cull.scenes.rules.alwaysKeep")}</span></label>
          <select className="field scene-rating" aria-label={t("cull.scenes.rules.ratedAtLeast")} value={r.ratedAtLeast} disabled={!r.alwaysKeep}
            onChange={(e) => setRules({ ratedAtLeast: Number(e.target.value) })}>
            {[1, 2, 3, 4, 5].map((n) => <option key={n} value={n}>{t("library.culling.stars", { count: n })}</option>)}
          </select>
        </div>
        <div className="cull-session-row" role="radiogroup" aria-label={t("cull.scenes.rules.singles")}>
          <span className="scene-rule-label">{t("cull.scenes.rules.singles")}</span>
          <label className="scene-check"><input type="radio" name="scene-singles" checked={r.singlesKeep} onChange={() => setRules({ singlesKeep: true })} />
            <span>{t("cull.scenes.rules.singlesKeep")}</span></label>
          <label className="scene-check"><input type="radio" name="scene-singles" checked={!r.singlesKeep} onChange={() => setRules({ singlesKeep: false })} />
            <span>{t("cull.scenes.rules.singlesSkip")}</span></label>
        </div>
      </div>
      <div className="cull-block">
        <button type="button" className="btn" disabled={!ready || s.previewBusy || busy} onClick={() => void previewRules(ctx)}>{t("cull.scenes.rules.preview")}</button>
      </div>
      <div className="cull-block">
        <h2 className="cull-h">{t("cull.scenes.rules.previewHeading")}</h2>
        <p className="note-plain scene-rule-counts" data-stale={s.previewStale || undefined} role="status" aria-live="polite">
          <span><bdi dir="ltr">{t("cull.scenes.rules.keep")} {num(counts?.keep)}</bdi></span>
          <span><bdi dir="ltr">{t("cull.scenes.rules.reject")} {num(counts?.reject)}</bdi></span>
          <span><bdi dir="ltr">{t("cull.scenes.rules.undecided")} {num(counts?.undecided)}</bdi></span>
        </p>
        <p className="note-plain scene-rule-state">{t(s.preview === null ? "cull.scenes.rules.nothingYet" : s.previewStale ? "cull.scenes.rules.stale" : "cull.scenes.rules.promise")}</p>
        <div className="cull-actions">
          <button type="button" className="btn" disabled={!fresh || busy} onClick={() => setScenes({ piles: true })}>{t("cull.scenes.rules.review")}</button>
          <button type="button" className="btn primary" disabled={!fresh || busy || applied} onClick={() => void apply()}>{t("cull.scenes.rules.apply")}</button>
          <button type="button" className="btn" disabled={!applied || busy} onClick={() => void undo()}>{t("cull.scenes.rules.undo")}</button>
        </div>
      </div>
      <div className="cull-block scene-rule-notes" role="status">
        <p className={`note-plain${s.ruleNote?.error ? " warn" : ""}`}>{s.ruleNote?.text ?? ""}</p>
      </div>
    </div>
  </div>;
}
