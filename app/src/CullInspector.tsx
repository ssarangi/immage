// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The lightbox's right column, of a fixed width whatever it shows: a Cull tab
// (why the cull decided as it did, in plain words, and the buttons to overrule
// it) and a Critique tab (Gemini's critique of the photo). The critique's
// numbered badges and the blurry region are drawn by CullOverlay over the photo.
import { AlertCircle, Check, Minus, Sparkles } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { commands, type CullChange, type CullDecision, type CullStatus, type GeminiCritique } from "./bindings";
import { reasonOf, withDecision } from "./cullModel";
import { cullState, updatePlan, useDecision } from "./cullStore";
import type { ImageEntry } from "./library";
import { formatNumber, t } from "./i18n";

export type Tab = "cull" | "critique";
// what the photo's overlay shows: the region under the pointer or keyboard, the
// one a click pinned, and whether the softest region is outlined
export type Sel = { hot: number | null; pinned: number | null; blur: boolean };
export const NO_SEL: Sel = { hot: null, pinned: null, blur: false };
export const activeRegion = (s: Sel) => s.hot ?? s.pinned;

export type Crit = {
  phase: "idle" | "loading" | "error"; error?: string; prefs?: boolean; critique: GeminiCritique | null;
  run: (force: boolean) => void;
};

// Gemini's critique of one photo: asked on request, answered from the saved
// copy when there is one. A new photo starts clean.
export function useCritique(image: ImageEntry | undefined): Crit {
  const [state, setState] = useState<Omit<Crit, "run">>({ phase: "idle", critique: null });
  const token = useRef(0);
  const id = image?.imgid;
  useEffect(() => { ++token.current; setState({ phase: "idle", critique: null }); }, [id]);
  const run = (force: boolean) => {
    if (!image) return;
    const mine = ++token.current;
    setState((o) => ({ phase: "loading", critique: force ? null : o.critique }));
    void (async () => {
      try {
        const status = await commands.geminiKeyStatus();
        if (!status.hasKey) { if (mine === token.current) setState({ phase: "error", prefs: true, error: t("cull.critique.noKey"), critique: null }); return; }
        const c = await commands.geminiCritique(image.imgid, image.path, force);
        if (mine === token.current) setState({ phase: "idle", critique: c });
      } catch (e) {
        if (mine === token.current) setState((o) => ({ phase: "error", error: t("cull.critique.failed", { error: String(e) }), critique: o.critique }));
      }
    })();
  };
  return { ...state, run };
}

const area = (b: GeminiCritique["regions"][number]["box"]) => (b[2] ?? 0) * (b[3] ?? 0);
const pct = (n: number | null) => `${Math.min(100, Math.max(0, (n ?? 0) * 100))}%`;

// the critique's numbered badges and the blurry region over the photo's canvas.
// Only the region in use (hovered, else pinned) draws its rectangle, the others
// dim; larger regions are drawn first so the smallest wins where they overlap
export function CullOverlay({ critique, sel, onSel, decision }: {
  critique: GeminiCritique | null; sel: Sel; onSel: (s: Sel) => void; decision?: CullDecision;
}) {
  const r = decision?.blurryRegion;
  const active = activeRegion(sel);
  const order = (critique?.regions ?? []).map((g, i) => ({ g, i })).sort((a, b) => area(b.g.box) - area(a.g.box));
  return <div className="cull-overlay">
    {sel.blur && r && <div className="cull-box blur" aria-hidden="true"
      style={{ left: pct(r.x), top: pct(r.y), width: pct(r.width), height: pct(r.height) }}><span>{t("cull.inspector.softest")}</span></div>}
    {order.map(({ g, i }) => {
      const on = active === i;
      const [x, y, w, h] = g.box;
      return <div key={i} className={`cull-region-box ${g.positive ? "good" : "bad"}${on ? " on" : ""}${active !== null && !on ? " dim" : ""}`}
        style={{ left: pct(x), top: pct(y), width: pct(w), height: pct(h) }}>
        <button type="button" className="cull-pin" tabIndex={-1}
          aria-label={t(g.positive ? "cull.critique.pinStrength" : "cull.critique.pinImprove", { n: i + 1, label: g.label })}
          onPointerEnter={() => onSel({ ...sel, hot: i })} onPointerLeave={() => onSel({ ...sel, hot: null })}
          onPointerDown={(e) => e.stopPropagation()}
          onClick={(e) => { e.stopPropagation(); onSel({ ...sel, pinned: sel.pinned === i ? null : i }); }}>
          {i + 1}{g.positive ? <Check size={9} aria-hidden="true" /> : <AlertCircle size={9} aria-hidden="true" />}
        </button>
        {on && <>
          {/* only the outline takes the pointer, so clicking inside the region still zooms the photo */}
          {(["t", "r", "b", "l"] as const).map((side) => <span key={side} className={`cull-edge ${side}`}
            onPointerEnter={() => onSel({ ...sel, hot: i })} onPointerLeave={() => onSel({ ...sel, hot: null })}
            onPointerDown={(e) => e.stopPropagation()}
            onClick={(e) => { e.stopPropagation(); onSel({ ...sel, pinned: sel.pinned === i ? null : i }); }} />)}
          <span className="cull-region-label">{g.label}</span></>}
      </div>;
    })}
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


function useElapsed(running: boolean) {
  const [s, setS] = useState(0);
  useEffect(() => {
    if (!running) { setS(0); return; }
    const start = Date.now();
    const timer = window.setInterval(() => setS(Math.floor((Date.now() - start) / 1000)), 500);
    return () => window.clearInterval(timer);
  }, [running]);
  return s;
}

function Summary({ text }: { text: string }) {
  const [more, setMore] = useState(false);
  const [long, setLong] = useState(false);
  const p = useRef<HTMLParagraphElement>(null);
  useEffect(() => {
    const el = p.current;
    if (el && !more) setLong(el.scrollHeight > el.clientHeight + 1);
  }, [text, more]);
  return <div className="cull-summary-wrap">
    <p ref={p} className={`cull-summary${more ? "" : " clamp"}`}>{text}</p>
    {(long || more) && <button type="button" className="link-button" aria-expanded={more}
      onClick={() => setMore(!more)}>{t(more ? "cull.critique.less" : "cull.critique.more")}</button>}
  </div>;
}

function Rows({ list, sel, onSel }: { list: { g: GeminiCritique["regions"][number]; i: number }[]; sel: Sel; onSel: (s: Sel) => void }) {
  const active = activeRegion(sel);
  return <ul className="cull-rows">
    {list.map(({ g, i }) => <li key={i} id={`cull-row-${i}`} className={`cull-row ${g.positive ? "good" : "bad"}${active === i ? " on" : ""}`}
      title={g.note} onPointerEnter={() => onSel({ ...sel, hot: i })} onPointerLeave={() => onSel({ ...sel, hot: null })}>
      <button type="button" className="cull-row-main" aria-pressed={sel.pinned === i}
        aria-label={t(g.positive ? "cull.critique.rowStrength" : "cull.critique.rowImprove", { n: i + 1, label: g.label, note: g.note })}
        onFocus={() => onSel({ ...sel, hot: i })} onBlur={() => onSel({ ...sel, hot: null })}
        onClick={() => onSel({ ...sel, pinned: sel.pinned === i ? null : i })}>
        <span className="cull-num" aria-hidden="true">{i + 1}{g.positive ? <Check size={10} /> : <AlertCircle size={10} />}</span>
        <strong className="cull-row-label">{g.label}</strong>
        <span className="cull-row-note">{g.note}</span>
      </button>
    </li>)}
  </ul>;
}

export function CullSide({ image, tab, onTab, crit, sel, onSel, onChanges, onOpenPrefs }: {
  image: ImageEntry; tab: Tab; onTab: (t: Tab) => void; crit: Crit; sel: Sel; onSel: (s: Sel) => void;
  onChanges: (changes: CullChange[]) => void; onOpenPrefs: () => void;
}) {
  const d = useDecision(image.path, image.version);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState("");
  const seconds = useElapsed(crit.phase === "loading");
  useEffect(() => { setProblem(""); }, [image.imgid]);
  // the region under the pointer on the photo scrolls its row into view
  useEffect(() => {
    if (sel.hot !== null) document.getElementById(`cull-row-${sel.hot}`)?.scrollIntoView({ block: "nearest" });
  }, [sel.hot]);

  async function decide(status: CullStatus, makeBest: boolean | null) {
    if (!d || busy) return;
    setBusy(true); setProblem("");
    try { await overrideDecision(d, status, makeBest, onChanges); }
    catch (e) { setProblem(String(e)); }
    finally { setBusy(false); }
  }

  const c = crit.critique;
  const all = (c?.regions ?? []).map((g, i) => ({ g, i }));
  const good = all.filter((r) => r.g.positive), bad = all.filter((r) => !r.g.positive);
  const loading = crit.phase === "loading";
  const tabs: Tab[] = ["cull", "critique"];

  return <aside className="cull-side" aria-label={t("cull.inspector.title")}>
    <div className="segmented small cull-tabs" role="tablist" aria-label={t("cull.inspector.title")}>
      {tabs.map((k) => <button key={k} type="button" role="tab" id={`cull-tab-${k}`} aria-selected={tab === k} aria-controls={`cull-panel-${k}`}
        className={tab === k ? "active" : ""} onClick={() => onTab(k)}>{t(`cull.tab.${k}`)}</button>)}
    </div>
    {tab === "cull" && <section id="cull-panel-cull" role="tabpanel" aria-labelledby="cull-tab-cull" className="cull-pane">
      {d ? <>
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
        {d.blurryRegion && <label className="row check"><input type="checkbox" checked={sel.blur}
          onChange={(e) => onSel({ ...sel, blur: e.target.checked })} />{t("cull.inspector.showSoft")}</label>}
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
      </> : <p className="note-plain">{t("cull.inspector.noDecision")}</p>}
    </section>}
    {tab === "critique" && <section id="cull-panel-critique" role="tabpanel" aria-labelledby="cull-tab-critique" className="cull-pane">
      {loading && <div className="cull-wait" role="status">
        <span className="cull-spinner" aria-hidden="true" />
        <span><strong>{t("cull.critique.loading")}</strong> <span className="cull-elapsed">{t("cull.critique.elapsed", { s: formatNumber(seconds) })}</span>
          <span className="note-plain cull-wait-note">{t("cull.critique.wait")}</span></span>
      </div>}
      {crit.phase === "error" && <p role="alert" className="note-plain warn">{crit.error}
        {crit.prefs && <> <button type="button" className="link-button" onClick={onOpenPrefs}>{t("cull.dialog.aiSetUp")}</button></>}</p>}
      {!c && !loading && <>
        <p className="note-plain">{t("cull.critique.intro")}</p>
        <button type="button" className="btn small" onClick={() => crit.run(false)}><Sparkles size={12} aria-hidden="true" /> {t("cull.critique.run")}</button>
      </>}
      {c && <>
        <h2 className="cull-headline">{c.headline || t("cull.critique.title")}</h2>
        {c.overallSummary && <Summary text={c.overallSummary} />}
        {good.length > 0 && <><h3 className="cull-group">{t("cull.critique.strengths")} <span className="cull-count">{formatNumber(good.length)}</span></h3>
          <Rows list={good} sel={sel} onSel={onSel} /></>}
        {bad.length > 0 && <><h3 className="cull-group">{t("cull.critique.improve")} <span className="cull-count">{formatNumber(bad.length)}</span></h3>
          <Rows list={bad} sel={sel} onSel={onSel} /></>}
        <div className="cull-foot">
          <span className="cull-model">{t(c.cached ? "cull.critique.footCached" : "cull.critique.footFresh", { model: c.model })}</span>
          <button type="button" className="btn small" disabled={loading} onClick={() => crit.run(true)}>{t("cull.critique.again")}</button>
        </div>
      </>}
    </section>}
  </aside>;
}
