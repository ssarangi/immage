// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The lightbox's report panel, of a fixed width whatever it shows: a Critique
// tab (Gemini's critique of the photo) and a Cull tab (why the cull decided as
// it did, in plain words, and the buttons to overrule it). The critique's
// numbered badges and the blurry region are drawn by CullOverlay over the photo.
// Nothing here changes the size of a row, badge or tab when it is hovered,
// focused or pinned: those states only paint (background, ring, outline).
import { CircleAlert, CircleMinus, EyeOff, Check, ChevronDown, KeyRound, Minus, Pin, RotateCcw, Sparkles } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { commands, type CullChange, type CullDecision, type CullStatus, type GeminiCritique } from "./bindings";
import { reasonOf, withDecision } from "./cullModel";
import { cullState, updatePlan, useDecision } from "./cullStore";
import type { ImageEntry } from "./library";
import { formatNumber, t } from "./i18n";
import { placeLabel, placePins } from "./critiquePins";

export type Tab = "cull" | "critique" | "scene";
// what the photo's overlay shows: the region under the pointer or keyboard, the
// one a click pinned, and whether the softest region is outlined
export type Sel = { hot: number | null; pinned: number | null; blur: boolean };
export const NO_SEL: Sel = { hot: null, pinned: null, blur: false };
export const activeRegion = (s: Sel) => s.hot ?? s.pinned;

// how long a critique has been asked for, and when the wording says so
export const SLOW_AFTER = 25;       // seconds: "still working"
export const VERY_SLOW_AFTER = 60;  // seconds: "taking longer than usual", with Cancel and Try again

export type Crit = {
  phase: "idle" | "loading" | "error"; error?: string; prefs?: boolean; critique: GeminiCritique | null;
  startedAt?: number;   // Date.now() when the request began, while loading
  run: (force: boolean) => void;
  cancel: () => void;   // stop waiting; a late answer is ignored (the request itself is left to finish)
};

// Gemini's critique of one photo: asked on request, answered from the saved
// copy when there is one. A new photo starts clean.
export function useCritique(image: ImageEntry | undefined): Crit {
  const [state, setState] = useState<Omit<Crit, "run" | "cancel">>({ phase: "idle", critique: null });
  const token = useRef(0);
  const id = image?.imgid;
  useEffect(() => { ++token.current; setState({ phase: "idle", critique: null }); }, [id]);
  const run = (force: boolean) => {
    if (!image) return;
    const mine = ++token.current;
    setState((o) => ({ phase: "loading", startedAt: Date.now(), critique: force ? null : o.critique }));
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
  // a UI-side cancel: the answer, when it comes, is for a request nobody waits on
  const cancel = () => { ++token.current; setState((o) => ({ phase: "idle", critique: o.critique })); };
  return { ...state, run, cancel };
}

type Region = GeminiCritique["regions"][number];
const area = (b: Region["box"]) => (b[2] ?? 0) * (b[3] ?? 0);
const pct = (n: number | null) => `${Math.min(100, Math.max(0, (n ?? 0) * 100))}%`;

// the photo's size as laid out, for the badges' pixel positions
function useSize(ref: React.RefObject<HTMLElement | null>) {
  const [size, setSize] = useState({ w: 0, h: 0 });
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const read = () => setSize((o) => (o.w === el.clientWidth && o.h === el.clientHeight ? o : { w: el.clientWidth, h: el.clientHeight }));
    read();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(read);
    ro.observe(el);
    return () => ro.disconnect();
  }, [ref]);
  return size;
}

// the critique's numbered badges and the blurry region over the photo's canvas.
// Only the region in use (hovered, else pinned) draws its outline, a spotlight
// and its label chip; the badges never change size, so nothing under the
// pointer moves. Larger regions are drawn first so the smallest wins where they
// overlap.
export function CullOverlay({ critique, sel, onSel, decision, loading }: {
  critique: GeminiCritique | null; sel: Sel; onSel: (s: Sel) => void; decision?: CullDecision; loading?: boolean;
}) {
  const box = useRef<HTMLDivElement>(null);
  const { w, h } = useSize(box);
  const r = decision?.blurryRegion;
  const active = activeRegion(sel);
  const regions = critique?.regions ?? [];
  const order = regions.map((g, i) => ({ g, i })).sort((a, b) => area(b.g.box) - area(a.g.box));
  const placed = w > 0 ? placePins(regions.map((g) => g.box as [number, number, number, number]), w, h) : [];
  const hover = (i: number | null) => ({ onPointerEnter: () => onSel({ ...sel, hot: i }), onPointerLeave: () => onSel({ ...sel, hot: null }) });
  const toggle = (i: number) => (e: React.MouseEvent) => { e.stopPropagation(); onSel({ ...sel, pinned: sel.pinned === i ? null : i }); };
  return <div ref={box} className={`cull-overlay${active !== null ? " has-on" : ""}`}>
    {loading && <div className="cull-sweep" aria-hidden="true" />}
    {sel.blur && r && <div className="cull-reg warn" aria-hidden="true"
      style={{ left: pct(r.x), top: pct(r.y), width: pct(r.width), height: pct(r.height) }}>
      <div className="cull-bx" /><span className="cull-label" style={{ left: PIN_INSET_CHIP, top: PIN_INSET_CHIP }}>{t("cull.inspector.softest")}</span></div>}
    {order.map(({ g, i }) => {
      const on = active === i;
      const [x, y, bw, bh] = g.box;
      return <div key={`b${i}`} className={`cull-reg ${g.positive ? "pos" : "neg"}${on ? " on" : ""}`}
        style={{ left: pct(x), top: pct(y), width: pct(bw), height: pct(bh) }}>
        <div className="cull-bx" />
        {/* only the outline takes the pointer, so clicking inside the region still zooms the photo */}
        {on && (["t", "r", "b", "l"] as const).map((side) => <span key={side} className={`cull-edge ${side}`}
          {...hover(i)} onPointerDown={(e) => e.stopPropagation()} onClick={toggle(i)} />)}
      </div>;
    })}
    {order.map(({ g, i }) => {
      const on = active === i;
      const [x, y] = g.box;
      const at = placed[i];
      const style: CSSProperties = at ? { left: at.left, top: at.top } : { left: `calc(${pct(x)} + 8px)`, top: `calc(${pct(y)} + 8px)` };
      const chip = at ? placeLabel(at, g.label, w) : { left: 36, top: 8 };
      return <div key={`p${i}`} className={`cull-mark ${g.positive ? "pos" : "neg"}${on ? " on" : ""}`}>
        <button type="button" className="cull-pin" tabIndex={-1} style={style}
          aria-label={t(g.positive ? "cull.critique.pinStrength" : "cull.critique.pinImprove", { n: i + 1, label: g.label })}
          {...hover(i)} onPointerDown={(e) => e.stopPropagation()} onClick={toggle(i)}>{i + 1}</button>
        <span className="cull-label" aria-hidden="true" style={at ? { left: chip.left, top: chip.top } : { left: `calc(${pct(x)} + 36px)`, top: `calc(${pct(g.box[1])} + 8px)` }}>{g.label}</span>
      </div>;
    })}
  </div>;
}
const PIN_INSET_CHIP = 8;

// the user's call on one photo: the plan keeps it, and a session already applied
// rewrites the library's entries (reported to `onChanges`)
export async function overrideDecision(d: CullDecision, status: CullStatus, makeBest: boolean | null,
                                       onChanges: (changes: CullChange[]) => void) {
  const r = await commands.cullOverride(d.id, status, makeBest);
  const plan = cullState().plan;
  if (plan) updatePlan(withDecision(plan, r.decision));
  if (r.changes.length) onChanges(r.changes);
}

// seconds since the request began, ticking while it runs
function useElapsed(startedAt: number | undefined) {
  const [s, setS] = useState(0);
  useEffect(() => {
    if (startedAt === undefined) { setS(0); return; }
    const tick = () => setS(Math.max(0, Math.floor((Date.now() - startedAt) / 1000)));
    tick();
    const timer = window.setInterval(tick, 500);
    return () => window.clearInterval(timer);
  }, [startedAt]);
  return s;
}
export const clock = (s: number) => `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;

// the summary, three lines of it, with a reserved line below for Read more so
// the panel does not jump when the text turns out to be short or long
function Summary({ text }: { text: string }) {
  const [more, setMore] = useState(false);
  const [long, setLong] = useState(false);
  const p = useRef<HTMLParagraphElement>(null);
  useLayoutEffect(() => {
    const el = p.current;
    if (el && !more) setLong(el.scrollHeight > el.clientHeight + 1);
  }, [text, more]);
  return <div className="cull-summary-wrap">
    <p ref={p} className={`cull-summary${more ? "" : " clamp"}`}>{text}</p>
    <div className="cull-more-slot">{(long || more) && <button type="button" className="cull-more" aria-expanded={more}
      onClick={() => setMore(!more)}>{t(more ? "cull.critique.less" : "cull.critique.more")}<ChevronDown size={12} aria-hidden="true" className={more ? "up" : ""} /></button>}</div>
  </div>;
}

function Rows({ list, sel, onSel }: { list: { g: Region; i: number }[]; sel: Sel; onSel: (s: Sel) => void }) {
  // Up and Down move between the rows; Left and Right stay with the photos
  const move = (e: React.KeyboardEvent<HTMLUListElement>) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    const all = Array.from(e.currentTarget.closest(".cull-pane")?.querySelectorAll<HTMLElement>(".cull-row-main") ?? []);
    const at = all.indexOf(document.activeElement as HTMLElement);
    const next = all[at + (e.key === "ArrowDown" ? 1 : -1)];
    if (at >= 0 && next) { e.preventDefault(); next.focus(); }
  };
  return <ul className="cull-rows" onKeyDown={move}>
    {list.map(({ g, i }) => <li key={i} id={`cull-row-${i}`}
      className={`cull-row ${g.positive ? "pos" : "neg"}${sel.hot === i ? " on" : ""}${sel.pinned === i ? " pinned" : ""}`}
      onPointerEnter={() => onSel({ ...sel, hot: i })} onPointerLeave={() => onSel({ ...sel, hot: null })}>
      <button type="button" className="cull-row-main" aria-pressed={sel.pinned === i}
        aria-label={t(g.positive ? "cull.critique.rowStrength" : "cull.critique.rowImprove", { n: i + 1, label: g.label, note: g.note })}
        onFocus={() => onSel({ ...sel, hot: i })} onBlur={() => onSel({ ...sel, hot: null })}
        onClick={() => onSel({ ...sel, pinned: sel.pinned === i ? null : i })}>
        <span className="cull-num" aria-hidden="true">{i + 1}</span>
        <strong className="cull-row-label">{g.label}</strong>
        <span className="cull-row-note">{g.note}</span>
        {sel.pinned === i && <Pin size={13} className="cull-row-pin" aria-hidden="true" />}
      </button>
    </li>)}
  </ul>;
}

// the stand-in for the result while Gemini works: the same shape, so the
// answer arrives where the stubs were
function Skeleton() {
  const row = (w: number) => <div className="cull-skel-row"><span className="cull-sk m" /><div><span className="cull-sk" style={{ height: 13, width: `${w}%` }} />
    <span className="cull-sk" style={{ height: 11, width: "92%", marginTop: 8 }} /></div></div>;
  return <div className="cull-skel" aria-hidden="true">
    <span className="cull-sk" style={{ height: 22, width: "94%" }} /><span className="cull-sk" style={{ height: 22, width: "62%", marginTop: 8 }} />
    <span className="cull-sk" style={{ height: 12, width: "100%", marginTop: 16 }} /><span className="cull-sk" style={{ height: 12, width: "96%", marginTop: 8 }} />
    <span className="cull-sk" style={{ height: 12, width: "70%", marginTop: 8 }} />
    <span className="cull-sk" style={{ height: 13, width: 90, margin: "26px 0 8px" }} />{row(55)}{row(62)}
    <span className="cull-sk" style={{ height: 13, width: 120, margin: "22px 0 8px" }} />{row(58)}
  </div>;
}

function Empty({ icon, title, body, action }: { icon: React.ReactNode; title: string; body: string; action: React.ReactNode }) {
  return <div className="cull-empty-state"><span className="cull-tile" aria-hidden="true">{icon}</span>
    <h3>{title}</h3><p className="cull-note">{body}</p>{action}</div>;
}

export function CullSide({ image, tab, onTab, crit, sel, onSel, onChanges, onOpenPrefs, scenePanel }: {
  image: ImageEntry; tab: Tab; onTab: (t: Tab) => void; crit: Crit; sel: Sel; onSel: (s: Sel) => void;
  onChanges: (changes: CullChange[]) => void; onOpenPrefs: () => void;
  scenePanel?: React.ReactNode;   // the scene view's Scene tab, first among the tabs
}) {
  const d = useDecision(image.path, image.version);
  const [busy, setBusy] = useState(false);
  const [problem, setProblem] = useState("");
  const seconds = useElapsed(crit.phase === "loading" ? crit.startedAt : undefined);
  useEffect(() => { setProblem(""); }, [image.imgid]);
  // the region under the pointer on the photo scrolls its row into view, inside
  // the panel only (never smooth: it would fight the pointer)
  useEffect(() => {
    if (sel.hot === null) return;
    const row = document.getElementById(`cull-row-${sel.hot}`);
    const pane = row?.closest<HTMLElement>(".cull-pane");
    if (!row || !pane) return;
    const a = row.getBoundingClientRect(), b = pane.getBoundingClientRect();
    if (a.top < b.top) pane.scrollTop -= b.top - a.top;
    else if (a.bottom > b.bottom) pane.scrollTop += a.bottom - b.bottom;
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
  const noKey = crit.phase === "error" && !!crit.prefs;
  const failed = crit.phase === "error" && !crit.prefs;
  const tabs: { id: Tab; icon?: React.ReactNode }[] = [...(scenePanel ? [{ id: "scene" as Tab }] : []),
    { id: "critique", icon: <Sparkles size={14} aria-hidden="true" /> }, { id: "cull" }];
  const verdictKey = d ? (d.status === "reject" ? "reject" : d.best ? "best" : "keep") : "keep";

  const foot = tab === "scene" ? null : tab === "cull"
    ? d && <div className="cull-override" role="group" aria-label={t("cull.inspector.override")}>
        <button type="button" className="cull-btn" aria-pressed={d.status === "keep"} disabled={busy || d.status === "keep"}
                title={t("cull.inspector.keepHint")} onClick={() => void decide("keep", null)}>{t("cull.inspector.keep")}</button>
        <button type="button" className="cull-btn" aria-pressed={d.status === "reject"} disabled={busy || d.status === "reject"}
                title={t("cull.inspector.rejectHint")} onClick={() => void decide("reject", null)}>{t("cull.inspector.reject")}</button>
        <button type="button" className="cull-btn" aria-pressed={d.best} disabled={busy || d.best || d.group === null}
                title={d.group === null ? t("cull.inspector.bestNone") : t("cull.inspector.bestHint")}
                onClick={() => void decide("keep", true)}>{t("cull.inspector.best")}</button>
      </div>
    : loading
      ? <><span className="cull-spacer" />
          {seconds >= VERY_SLOW_AFTER && <button type="button" className="cull-btn quiet" onClick={() => crit.run(true)}><RotateCcw size={13} aria-hidden="true" />{t("cull.critique.retry")}</button>}
          <button type="button" className="cull-btn quiet" onClick={crit.cancel}>{t("cull.critique.cancel")}</button></>
      : c && <><span className="cull-model" title={c.cached ? t("cull.critique.cachedHint") : undefined}>{c.model}</span>
          <button type="button" className="cull-btn quiet" onClick={() => crit.run(true)}><RotateCcw size={13} aria-hidden="true" />{t("cull.critique.again")}</button></>;

  return <aside className="cull-side" aria-label={t("cull.inspector.title")}>
    <div className="cull-tabs" role="tablist" aria-label={t("cull.inspector.title")}>
      {tabs.map(({ id, icon }) => <button key={id} type="button" role="tab" id={`cull-tab-${id}`} aria-selected={tab === id} aria-controls={`cull-panel-${id}`}
        className="cull-tab" onClick={() => onTab(id)}>{icon}{t(`cull.tab.${id}`)}</button>)}
    </div>
    <section id={`cull-panel-${tab}`} role="tabpanel" aria-labelledby={`cull-tab-${tab}`} className="cull-pane">
      {tab === "scene" && scenePanel}
      {tab === "cull" && (d ? <>
        <div className="cull-verdict">
          <span className={`cull-tile ${d.status}`} aria-hidden="true">{d.status === "reject" ? <Minus size={18} /> : <Check size={18} />}</span>
          <div><h2>{t(`cull.verdict.${verdictKey}`)}</h2>
            <div className="cull-sub">{d.group !== null ? t("cull.inspector.set", { n: formatNumber(d.group) }) : t("cull.inspector.noSet")}
              {d.highlight && d.status === "keep" && ` · ${t("cull.verdict.highlight")}`}{d.applied && ` · ${t("cull.inspector.applied")}`}</div></div>
        </div>
        <p className="cull-reason">{t(`cull.reason.${reasonOf(d)}`)}</p>
        <h3 className="cull-group">{t("cull.inspector.why")} <span className="cull-count">{formatNumber(d.factors.length)}</span></h3>
        <ul className="cull-factors">
          {d.factors.map((f) => {
            // closed eyes (backend: impact "supporting") lower the score: a warning glyph, never a tick; the text is the backend's
            const eye = f.id === "eye_state" && f.impact === "supporting";
            const kind = f.impact === "reject" ? "no" : f.impact === "supporting" && !eye ? "ok" : "cx";
            return <li key={f.id}>
              <span className={`cull-glyph ${eye ? "cx warn" : kind}`} role="img" aria-label={eye ? f.label : t(`cull.impact.${kind === "no" ? "reject" : kind === "ok" ? "supporting" : "context"}`)}>
                {eye ? <EyeOff size={18} /> : kind === "no" ? <CircleMinus size={18} /> : kind === "ok" ? <Check size={18} /> : <Minus size={18} />}</span>
              <div><strong>{f.label}</strong><span className="cull-note">{f.detail}</span></div>
            </li>;
          })}
        </ul>
        {d.blurryRegion && <button type="button" role="switch" aria-checked={sel.blur} className="cull-switch"
          onClick={() => onSel({ ...sel, blur: !sel.blur })}>{t("cull.inspector.showSoft")}<span className="cull-track" aria-hidden="true" /></button>}
        {problem && <p role="alert" className="cull-note warn">{problem}</p>}
      </> : <p className="cull-note">{t("cull.inspector.noDecision")}</p>)}
      {tab === "critique" && <>
        {loading && <div className="cull-wait" role="status">
          <div className="cull-wait-top"><strong>{t("cull.critique.studying")}</strong><span className="cull-elapsed">{clock(seconds)}</span></div>
          <div className="cull-bar" aria-hidden="true" />
          <p className="cull-note">{t(seconds >= VERY_SLOW_AFTER ? "cull.critique.verySlow" : seconds >= SLOW_AFTER ? "cull.critique.slow" : "cull.critique.wait")}</p>
        </div>}
        {loading && !c && <Skeleton />}
        {failed && <div className="cull-alert" role="alert"><CircleAlert size={18} aria-hidden="true" />
          <div><strong>{t("cull.critique.errorTitle")}</strong><span className="cull-note">{crit.error}</span>
            {c && <span className="cull-note">{t("cull.critique.errorKept")}</span>}</div></div>}
        {failed && !c && <button type="button" className="cull-btn" onClick={() => crit.run(false)}><RotateCcw size={13} aria-hidden="true" />{t("cull.critique.retry")}</button>}
        {noKey && <Empty icon={<KeyRound size={18} />} title={t("cull.critique.noKeyTitle")} body={t("cull.critique.noKeyBody")}
          action={<button type="button" className="cull-btn primary" onClick={onOpenPrefs}>{t("cull.critique.openPrefs")}</button>} />}
        {!c && !loading && crit.phase === "idle" && <Empty icon={<Sparkles size={18} />} title={t("cull.critique.emptyTitle")}
          body={t("cull.critique.intro")}
          action={<button type="button" className="cull-btn primary" onClick={() => crit.run(false)}><Sparkles size={13} aria-hidden="true" />{t("cull.critique.emptyRun")}</button>} />}
        {c && <div className="cull-result">
          <h2 className="cull-headline">{c.headline || t("cull.critique.title")}</h2>
          {c.overallSummary && <Summary text={c.overallSummary} />}
          {good.length > 0 && <><h3 className="cull-group">{t("cull.critique.strengths")} <span className="cull-count">{formatNumber(good.length)}</span></h3>
            <Rows list={good} sel={sel} onSel={onSel} /></>}
          {bad.length > 0 && <><h3 className="cull-group">{t("cull.critique.improve")} <span className="cull-count">{formatNumber(bad.length)}</span></h3>
            <Rows list={bad} sel={sel} onSel={onSel} /></>}
        </div>}
      </>}
    </section>
    {foot && <div className="cull-foot">{foot}</div>}
  </aside>;
}
