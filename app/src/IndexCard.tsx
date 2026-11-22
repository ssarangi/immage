// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The first-index surface of a source: three phases (finding photos, reading them, making
// thumbnails) with their numbers, the time left as a range, and what is already usable. It has one
// fixed height per variant, so the numbers and the words change in place and nothing around it moves.
// Presentational: the Catalog card gives it the jobs and what they reported.
import { formatNumber, t } from "./i18n";
import {
  durationText, etaOf, isReady, overallFraction, phases, problemOf, sourceState,
  type Ended, type Jobs, type PhaseView,
} from "./catalogModel";
import type { SourceStatus } from "./bindings";

export type IndexActions = {
  onPause(): void; onResume(): void; onStop(): void; onRetry(): void; onOpen(): void; onDismiss(): void;
};

const fmt = (n: number) => formatNumber(n);

function duration(seconds: number): string {
  const d = durationText(seconds);
  return d.unit === "s" ? t("catalog.dur.s", { n: d.a }) : d.unit === "min" ? t("catalog.dur.min", { n: d.a }) : t("catalog.dur.h", { n: d.a, m: d.b ?? 0 });
}

// a ring that fills (never a spinner): the fraction as a stroke, or an empty ring while it is not known
export function Ring({ fraction, size = 16 }: { fraction: number | null; size?: number }) {
  const r = size / 2 - 2, c = 2 * Math.PI * r;
  return (
    <svg className="cat-ring" width={size} height={size} viewBox={`0 0 ${size} ${size}`} aria-hidden="true">
      <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="var(--track)" strokeWidth="2" />
      {fraction !== null && <circle cx={size / 2} cy={size / 2} r={r} fill="none" stroke="var(--accent)" strokeWidth="2"
        strokeDasharray={`${c * fraction} ${c}`} transform={`rotate(-90 ${size / 2} ${size / 2})`} />}
    </svg>
  );
}

function phaseLabel(p: PhaseView): string {
  return t(p.id === "finding" ? "catalog.phase.finding" : p.id === "reading" ? "catalog.phase.reading" : "catalog.phase.thumbs");
}

function phaseNumbers(p: PhaseView): string {
  if (p.id === "finding") {
    if (p.status === "waiting") return "";
    return p.status === "active" ? t("catalog.num.soFar", { n: fmt(p.done) }) : t("catalog.num.found", { n: fmt(p.done) });
  }
  if (p.status === "waiting") return "";
  if (p.id === "reading") {
    const base = p.total ? t("catalog.num.of", { done: fmt(p.done), total: fmt(p.total) }) : fmt(p.done);
    return p.status === "active" && p.rate ? `${base} - ${t("catalog.num.rate", { n: fmt(Math.round(p.rate)) })}` : base;
  }
  return p.total && p.status === "active" ? t("catalog.num.of", { done: fmt(p.done), total: fmt(p.total) }) : t("catalog.num.made", { n: fmt(p.done) });
}

function Phase({ p, n }: { p: PhaseView; n: number }) {
  const fraction = p.id === "finding" ? (p.status === "done" ? 1 : 0) : p.total ? Math.min(1, p.done / p.total) : p.status === "done" ? 1 : 0;
  return (
    <div className={`cat-phase ${p.status}`} data-phase={p.id}>
      <span className="cat-badge" aria-hidden="true">{p.status === "done" ? "✓" : n}</span>
      <span className="cat-phase-name">{phaseLabel(p)}</span>
      <span className="cat-phase-num">{phaseNumbers(p) || " "}</span>
      <span className="cat-bar"><i className={p.id === "finding" && p.status === "active" ? "finding" : ""} style={{ width: `${fraction * 100}%` }} /></span>
    </div>
  );
}

export function IndexCard({ source, jobs, ended, actions }: {
  source: SourceStatus; jobs: Jobs | undefined; ended: Ended | undefined; actions: IndexActions;
}) {
  const state = sourceState(source, jobs);
  const src = source.source;
  const host = src.host ?? (src.kind === "nas" ? t("catalog.storage.network") : src.kind === "drive" ? t("catalog.storage.drive") : t("catalog.storage.local"));
  const view = phases(jobs, ended, src.photos);
  const running = state === "indexing" || state === "paused";
  const problem = running ? null : problemOf(ended);
  const ready = !running && isReady(ended);
  const offline = !running && (state === "offline" || state === "asleep");
  // one variant per situation; two heights only
  const tall = running || ready || (!!problem && !offline);
  const variant = running ? (state === "paused" ? "paused" : "indexing") : problem && !offline ? "problem" : ready ? "done" : offline ? "offline" : "idle";
  const fraction = overallFraction(view);
  const eta = etaOf(jobs?.meta?.eta_s ?? jobs?.thumbs?.eta_s ?? jobs?.scan?.eta_s);
  const reading = view[1];
  const unreadable = (ended?.meta?.failed ?? 0) + (ended?.meta?.unparsed ?? 0);
  const startedAgo = ended?.at && ended.scan ? ended.scan.elapsed_s + (ended.meta?.elapsed_s ?? 0) + (ended.thumbs?.elapsed_s ?? 0) : null;

  let title: string, sub: string;
  if (running) { title = t("catalog.card.indexing", { name: src.name }); sub = t(state === "paused" ? "catalog.card.pausedSub" : "catalog.card.indexingSub", { host }); }
  else if (variant === "done") { title = t("catalog.card.ready", { name: src.name }); sub = t("catalog.card.readySub", { n: fmt(ended?.scan?.found ?? src.photos), time: duration(startedAgo ?? 0) }); }
  else if (variant === "problem") { title = t("catalog.card.problem", { name: src.name }); sub = t("catalog.card.neverChanged"); }
  else if (offline) { title = t("catalog.card.offline", { name: src.name }); sub = t("catalog.card.offlineSub", { n: fmt(src.photos) }); }
  else { title = src.name; sub = src.photos > 0 ? t("catalog.card.idleSub", { n: fmt(src.photos) }) : t("catalog.card.notIndexed"); }

  const buttons = (() => {
    if (running) return <>
      <button className="btn small" onClick={state === "paused" ? actions.onResume : actions.onPause}>{t(state === "paused" ? "catalog.act.resume" : "catalog.act.pause")}</button>
      <button className="btn small" onClick={actions.onStop} title={t("catalog.act.stopHint")}>{t("catalog.act.stop")}</button>
    </>;
    if (variant === "done") return <button className="btn small primary" onClick={actions.onOpen}>{t("catalog.act.open")}</button>;
    if (variant === "problem" || offline) return <button className="btn small" disabled={offline} title={offline ? t("catalog.notConnected") : undefined} onClick={actions.onRetry}>{t("catalog.act.tryAgain")}</button>;
    return <button className="btn small" onClick={actions.onRetry}>{t(src.photos > 0 ? "catalog.act.rescan" : "catalog.act.index")}</button>;
  })();

  const problemText = problem === "offline" ? t("catalog.problem.offline", { host })
    : problem === "disk_full" ? t("catalog.problem.diskFull")
    : problem === "partial" ? t("catalog.problem.partial", { n: ended?.scan?.incomplete_dirs.length ?? 0 })
    : problem === "failed" ? t("catalog.problem.failed", { why: ended?.scan?.error ?? ended?.meta?.error ?? ended?.thumbs?.error ?? t("catalog.problem.unknown") })
    : "";

  let foot1: string, foot2: string;
  if (state === "paused") { foot1 = t("catalog.foot.paused", { done: fmt(reading.done), total: reading.total ? fmt(reading.total) : "-" }); foot2 = t("catalog.foot.safe"); }
  else if (running) {
    foot1 = eta.kind === "range" ? t("catalog.foot.eta", { t: duration((eta.lo + eta.hi) / 2), lo: duration(eta.lo), hi: duration(eta.hi) })
      : eta.kind === "soon" ? t("catalog.foot.soon")
      : t("catalog.foot.working");
    foot2 = jobs?.thumbs?.yielding ? t("catalog.foot.yield") : reading.done > 0 ? t("catalog.foot.usable", { n: fmt(reading.done) }) : t("catalog.foot.keepUsing");
  } else if (variant === "done") {
    foot1 = unreadable > 0 ? t("catalog.foot.unreadable", { read: fmt(ended?.meta?.done ?? 0), n: fmt(unreadable) }) : t("catalog.foot.allRead", { n: fmt(ended?.meta?.done ?? 0) });
    foot2 = t("catalog.foot.next");
  } else if (variant === "problem") { foot1 = problemText; foot2 = t("catalog.foot.again"); }
  else if (offline) { foot1 = t("catalog.foot.offlineStay"); foot2 = t("catalog.foot.offlineBrowse"); }
  else { foot1 = t("catalog.foot.idle"); foot2 = " "; }

  return (
    <div className={`cat-card ${tall ? "tall" : "short"}`} data-variant={variant} role="group" aria-label={title}>
      <div className="cat-head">
        <span className="cat-head-ring">{running ? <Ring fraction={fraction} size={22} /> : <Glyph state={state} />}</span>
        <span className="cat-head-text">
          <span className="cat-title" title={title}>{title}</span>
          <span className="cat-sub" title={sub}>{sub}</span>
        </span>
      </div>
      <div className="cat-phases" aria-hidden={!tall}>
        {view.map((p, i) => <Phase key={p.id} p={p} n={i + 1} />)}
      </div>
      <div className="cat-foot">
        <span className="cat-foot-a" title={foot1}>{foot1}</span>
        <span className="cat-foot-b" title={foot2}>{foot2}</span>
      </div>
      <div className="cat-acts">{buttons}</div>
      <button className="cat-x" aria-label={t("catalog.act.dismiss")} title={t("catalog.act.dismiss")} onClick={actions.onDismiss}>{"×"}</button>
    </div>
  );
}

// the state slot's glyph: a shape and a word (the title), never only a colour
export function Glyph({ state, fraction = null }: { state: ReturnType<typeof sourceState>; fraction?: number | null }) {
  switch (state) {
    case "online": return <svg className="cat-glyph ok" viewBox="0 0 16 16" aria-hidden="true"><circle cx="8" cy="8" r="3.5" fill="currentColor" /></svg>;
    case "indexing": return <Ring fraction={fraction} />;
    case "paused": return <svg className="cat-glyph" viewBox="0 0 16 16" aria-hidden="true"><rect x="4.5" y="3.5" width="2.6" height="9" rx="0.8" fill="currentColor" /><rect x="8.9" y="3.5" width="2.6" height="9" rx="0.8" fill="currentColor" /></svg>;
    case "attention": return <svg className="cat-glyph warn" viewBox="0 0 16 16" aria-hidden="true"><path d="M8 2.2 14.4 13H1.6z" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round" /><path d="M8 6.3v3.1M8 11v.1" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" /></svg>;
    case "offline": return <svg className="cat-glyph" viewBox="0 0 16 16" aria-hidden="true"><circle cx="8" cy="8" r="4" fill="none" stroke="currentColor" strokeWidth="1.6" /></svg>;
    case "asleep": return <svg className="cat-glyph" viewBox="0 0 16 16" aria-hidden="true"><circle cx="8" cy="8" r="4" fill="none" stroke="currentColor" strokeWidth="1.6" strokeDasharray="1.2 2.1" /></svg>;
    case "never": return <svg className="cat-glyph" viewBox="0 0 16 16" aria-hidden="true"><circle cx="8" cy="8" r="4" fill="none" stroke="currentColor" strokeWidth="1.6" strokeDasharray="2.2 1.8" /></svg>;
  }
}
