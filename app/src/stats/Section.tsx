// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import type { ReactNode } from "react";
import { t } from "../i18n";
import { SMALL_AGGREGATE } from "../statsMath";
import { nf } from "../statsFormat";

/** A section's top: a mono eyebrow with a glowing dot, the serif title with one italic amber phrase, a right-hand slot. */
export function SectionHead({ eyebrow, title, em, aside, lede }: { eyebrow: string; title: string; em: string; aside?: ReactNode; lede?: ReactNode }) {
  return (
    <>
      <div className="st-sec-head">
        <div>
          <div className="st-eyebrow">{eyebrow}</div>
          <h2>{title} <em>{em}</em></h2>
        </div>
        {aside && <div className="st-sec-aside">{aside}</div>}
      </div>
      {lede && <p className="st-lede">{lede}</p>}
    </>
  );
}

/** "44,602 / 46,880 photos with camera data": partial fields are never mistaken for the whole. */
export function KnownFact({ known, of, label }: { known: number | undefined; of: number | undefined; label: string }) {
  if (known === undefined || of === undefined) return <span className="st-fact"><i className="st-sk st-sk-fact" /></span>;
  return <span className="st-fact"><b>{nf(known)}</b> / {nf(of)} {label}</span>;
}

export function SmallSample({ n }: { n: number | undefined }) {
  if (n === undefined || n >= SMALL_AGGREGATE) return null;
  return <span className="st-small-tag" title={t("stats.small.tip", { n: SMALL_AGGREGATE })}>{t("stats.small")}</span>;
}

export function Insight({ tone = "cyan", tag, children, onClick }: { tone?: "cyan" | "amber"; tag: string; children: ReactNode; onClick?: () => void }) {
  const body = <><span className="tag">{tag}</span><span>{children}</span></>;
  return onClick
    ? <button type="button" className={`st-insight ${tone}`} onClick={onClick}>{body}</button>
    : <div className={`st-insight ${tone}`}>{body}</div>;
}

export function Bold({ parts }: { parts: { text: string; bold: boolean }[] }) {
  return <>{parts.map((p, i) => (p.bold ? <b key={i}>{p.text}</b> : <span key={i}>{p.text}</span>))}</>;
}

/** A card-level error: the message, Try again, and the support bundle when it is the engine or the index. */
export function ErrorCard({ message, onRetry }: { message: string; onRetry(): void }) {
  return (
    <div className="st-card st-state-card" role="alert">
      <h3>{t("stats.error.title")}</h3>
      <p className="st-sub">{message}</p>
      <button type="button" className="st-btn" onClick={onRetry}>{t("stats.error.retry")}</button>
    </div>
  );
}

/** Nothing matches the filters: said inside the chart's card, with the way out. */
export function NoMatch({ onClear }: { onClear(): void }) {
  return (
    <div className="st-nomatch">
      <span>{t("stats.empty.noMatch")}</span>
      <button type="button" className="st-link" onClick={onClear}>{t("stats.filters.clear")}</button>
    </div>
  );
}
