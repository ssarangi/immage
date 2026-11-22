// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The lightbox's inspector: why the cull decided as it did, in plain words, the
// buttons to overrule it, and Gemini's critique of the photo. The critique's
// boxes and the blurry region are drawn by CullOverlay over the photo.
import { Check, Minus, Plus } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { commands, type CullChange, type CullDecision, type CullStatus, type GeminiCritique } from "./bindings";
import { reasonOf, withDecision } from "./cullModel";
import { cullState, updatePlan, useDecision } from "./cullStore";
import type { ImageEntry } from "./library";
import { formatNumber, t } from "./i18n";

export type Overlay = { critique: GeminiCritique | null; hot: number | null; blur: boolean };
export const NO_OVERLAY: Overlay = { critique: null, hot: null, blur: false };

// the decision's blurry region and the critique's boxes, over the photo's canvas
export function CullOverlay({ overlay, decision }: { overlay: Overlay; decision?: CullDecision }) {
  const r = decision?.blurryRegion;
  const box = (x: number | null, y: number | null, w: number | null, h: number | null) =>
    ({ left: `${(x ?? 0) * 100}%`, top: `${(y ?? 0) * 100}%`, width: `${(w ?? 0) * 100}%`, height: `${(h ?? 0) * 100}%` });
  return <div className="cull-overlay" aria-hidden="true">
    {overlay.blur && r && <div className="cull-box blur" style={box(r.x, r.y, r.width, r.height)}><span>{t("cull.inspector.softest")}</span></div>}
    {overlay.critique?.regions.map((g, i) => <div key={i} className={`cull-box ${g.positive ? "good" : "bad"}${overlay.hot === i ? " hot" : ""}`}
      style={box(g.box[0], g.box[1], g.box[2], g.box[3])}><span>{g.positive ? "+ " : "− "}{g.label}</span></div>)}
  </div>;
}

// the user's call on one photo: the plan keeps it, and a session already applied
// rewrites the library's entries (reported to `onChanges`)
export async function overrideDecision(d: CullDecision, status: CullStatus, makeBest: boolean | null,
                                       onChanges: (changes: CullChange[]) => void) {
  const r = await commands.cullOverride(d.id, status, makeBest);
  const plan = cullState().plan;
  if (plan) updatePlan(withDecision(plan, r.decision));
  if (r.changes.length) onChanges(r.changes);
}

type Ask = { state: "idle" } | { state: "loading" } | { state: "error"; text: string; prefs?: boolean };

export function CullInspector({ image, overlay, onOverlay, onChanges, onOpenPrefs }: {
  image: ImageEntry; overlay: Overlay; onOverlay: (o: Overlay) => void;
  onChanges: (changes: CullChange[]) => void; onOpenPrefs: () => void;
}) {
  const d = useDecision(image.path, image.version);
  const [ask, setAsk] = useState<Ask>({ state: "idle" });
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState("");
  const token = useRef(0);
  // a new photo starts clean: its own critique, if it is cached, is one click away
  useEffect(() => { ++token.current; setAsk({ state: "idle" }); setProblem(""); }, [image.imgid]);

  async function decide(status: CullStatus, makeBest: boolean | null) {
    if (!d || busy) return;
    setBusy(true); setProblem("");
    try {
      await overrideDecision(d, status, makeBest, onChanges);
    } catch (e) { setProblem(String(e)); }
    finally { setBusy(false); }
  }

  async function critique(force: boolean) {
    const mine = ++token.current;
    setAsk({ state: "loading" });
    onOverlay({ ...overlay, critique: force ? null : overlay.critique, hot: null });
    try {
      const status = await commands.geminiKeyStatus();
      if (!status.hasKey || !status.enabled) {
        if (mine === token.current) setAsk({ state: "error", prefs: true, text: t(status.hasKey ? "cull.critique.disabled" : "cull.critique.noKey") });
        return;
      }
      const c = await commands.geminiCritique(image.imgid, image.path, force);
      if (mine !== token.current) return;
      onOverlay({ ...overlay, critique: c, hot: null });
      setAsk({ state: "idle" });
    } catch (e) {
      if (mine === token.current) setAsk({ state: "error", text: t("cull.critique.failed", { error: String(e) }) });
    }
  }
  const c = overlay.critique;

  return <aside className="cull-inspector" aria-label={t("cull.inspector.title")}>
    <h2>{t("cull.inspector.title")}</h2>
    {d ? <section aria-label={t("cull.inspector.decision")}>
      <p className={`cull-verdict ${d.status}`}>
        {d.status === "reject" ? <Minus size={14} aria-hidden="true" /> : <Check size={14} aria-hidden="true" />}
        <strong>{t(d.status === "reject" ? "cull.verdict.reject" : d.best ? "cull.verdict.best" : "cull.verdict.keep")}</strong>
        {d.highlight && d.status === "keep" && <span className="cull-tag">{t("cull.verdict.highlight")}</span>}
      </p>
      <p className="note-plain">{t(`cull.reason.${reasonOf(d)}`)}{d.group !== null && ` · ${t("cull.inspector.set", { n: formatNumber(d.group) })}`}
        {d.applied && ` · ${t("cull.inspector.applied")}`}</p>
      <ul className="cull-factors">
        {d.factors.map((f) => <li key={f.id}>
          <span className="cull-impact">{t(`cull.impact.${f.impact === "reject" || f.impact === "supporting" ? f.impact : "context"}`)}</span>
          <strong>{f.label}</strong><span className="note-plain">{f.detail}</span>
        </li>)}
      </ul>
      {d.blurryRegion && <label className="row check"><input type="checkbox" checked={overlay.blur}
        onChange={(e) => onOverlay({ ...overlay, blur: e.target.checked })} />{t("cull.inspector.showSoft")}</label>}
      <div className="cull-actions" role="group" aria-label={t("cull.inspector.override")}>
        <button type="button" className="btn small" aria-pressed={d.status === "keep"} disabled={busy || d.status === "keep"}
                title={t("cull.inspector.keepHint")} onClick={() => void decide("keep", null)}>{t("cull.inspector.keep")}</button>
        <button type="button" className="btn small" aria-pressed={d.status === "reject"} disabled={busy || d.status === "reject"}
                title={t("cull.inspector.rejectHint")} onClick={() => void decide("reject", null)}>{t("cull.inspector.reject")}</button>
        <button type="button" className="btn small" disabled={busy || d.best || d.group === null}
                title={d.group === null ? t("cull.inspector.bestNone") : t("cull.inspector.bestHint")}
                onClick={() => void decide("keep", true)}>{t("cull.inspector.best")}</button>
      </div>
      {problem && <p role="alert" className="note-plain warn">{problem}</p>}
    </section> : <p className="note-plain">{t("cull.inspector.noDecision")}</p>}
    <section aria-label={t("cull.critique.title")}>
      <h3>{t("cull.critique.title")}</h3>
      {!c && <button type="button" className="btn small" disabled={ask.state === "loading"} onClick={() => void critique(false)}>
        {ask.state === "loading" ? t("cull.critique.loading") : t("cull.critique.run")}</button>}
      {ask.state === "error" && <p role="alert" className="note-plain warn">{ask.text}
        {ask.prefs && <> <button type="button" className="link-button" onClick={onOpenPrefs}>{t("cull.dialog.aiSetUp")}</button></>}</p>}
      {c && <>
        <p className="cull-summary">{c.overallSummary}</p>
        <ul className="cull-regions">
          {c.regions.map((g, i) => <li key={i} className={g.positive ? "good" : "bad"}
            onMouseEnter={() => onOverlay({ ...overlay, hot: i })} onMouseLeave={() => onOverlay({ ...overlay, hot: null })}>
            <button type="button" className="cull-region" onFocus={() => onOverlay({ ...overlay, hot: i })} onBlur={() => onOverlay({ ...overlay, hot: null })}>
              {g.positive ? <Plus size={12} aria-hidden="true" /> : <Minus size={12} aria-hidden="true" />}
              <span className="cull-impact">{t(g.positive ? "cull.critique.strength" : "cull.critique.weakness")}</span>
              <strong>{g.label}</strong></button>
            <span className="note-plain">{g.note}</span>
          </li>)}
        </ul>
        <p className="note-plain">{c.cached ? t("cull.critique.cached", { model: c.model }) : t("cull.critique.fresh", { model: c.model })}</p>
        <button type="button" className="btn small" disabled={ask.state === "loading"} onClick={() => void critique(true)}>
          {ask.state === "loading" ? t("cull.critique.loading") : t("cull.critique.again")}</button>
      </>}
    </section>
  </aside>;
}
