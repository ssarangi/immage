// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Auto-keywords part of its AI models card: the confidence setting, the English note with the
// measured speed, the results row (Review and Remove), and the review dialog that lists the keywords
// the analysis added and removes them. Every row has the fixed height of the card's other switch
// rows, so a count, a speed line or a change of confidence moves nothing (aiModels.css).
import { useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { KeywordsSummary, RemoveReport, TagCount } from "./bindings";
import { PrefRow, PrefSwitch } from "./PrefsUI";
import { burstLine, resultsLine, speedLine, uiFmt } from "./aiPacks";
import { t } from "./i18n";

export type KeywordsRowsProps = {
  summary: KeywordsSummary | null;
  busy: boolean;
  onThreshold: (percent: number) => void;
  onReview: () => void;
  onBurst: (on: boolean) => void;
};

export function KeywordsRows({ summary, busy, onThreshold, onReview, onBurst }: KeywordsRowsProps) {
  const stored = summary?.threshold ?? 70;
  const [value, setValue] = useState(stored);
  const dragging = useRef(false);
  useEffect(() => { if (!dragging.current) setValue(stored); }, [stored]);
  const commit = (v: number) => { dragging.current = false; if (v !== stored) onThreshold(v); };
  const none = !summary || summary.keywords === 0;
  return <>
    <PrefRow label={t("aimodels.kw.confidence")} hint={t("aimodels.kw.confidenceHint", { pct: value })}>
      <input type="range" className="kw-slider" min={summary?.min_threshold ?? 40} max={summary?.max_threshold ?? 95} step={5} value={value}
             aria-label={t("aimodels.kw.confidence")} aria-valuetext={`${value}%`} disabled={!summary}
             onPointerDown={() => { dragging.current = true; }}
             onChange={(e) => setValue(Number(e.target.value))}
             onPointerUp={(e) => commit(Number((e.target as HTMLInputElement).value))}
             onKeyUp={(e) => commit(Number((e.target as HTMLInputElement).value))} />
      <span className="kw-pct" aria-hidden="true">{value}%</span>
    </PrefRow>
    <PrefRow label={t("aimodels.kw.english")} hint={speedLine(summary, t, uiFmt)}><span /></PrefRow>
    <PrefRow label={t("aimodels.kw.burst")} hint={burstLine(summary, t, uiFmt)}>
      <PrefSwitch checked={summary?.burst ?? true} disabled={!summary || busy} aria-label={t("aimodels.kw.burst")} onChange={onBurst} />
    </PrefRow>
    <PrefRow label={t("aimodels.kw.resultsLabel")} hint={resultsLine(summary, t, uiFmt)}>
      <button type="button" className="btn small kw-btn" disabled={busy || none} onClick={onReview}>{t("aimodels.kw.review")}</button>
    </PrefRow>
  </>;
}

export function KeywordsReviewDialog({ list, keywords, photos, onRemove, onClose }: {
  list: TagCount[];
  /** every keyword the analysis added, and the photos that carry them: what "Remove all" will touch */
  keywords: number; photos: number;
  /** names empty means every AI keyword; resolves with what was removed */
  onRemove: (names: string[]) => Promise<RemoveReport>;
  onClose: () => void;
}) {
  const [confirm, setConfirm] = useState(false);
  const [working, setWorking] = useState(false);
  const [done, setDone] = useState<RemoveReport | null>(null);
  const [error, setError] = useState("");
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    box.current?.querySelector<HTMLButtonElement>("button[data-first]")?.focus();
    const key = (e: KeyboardEvent) => { if (e.key === "Escape") { e.stopPropagation(); e.preventDefault(); if (!working) onClose(); } };
    document.addEventListener("keydown", key, true);
    return () => document.removeEventListener("keydown", key, true);
  }, [onClose, working]);
  const run = async (names: string[]) => {
    setWorking(true); setError("");
    try { setDone(await onRemove(names)); setConfirm(false); } catch (e) { setError(String(e)); } finally { setWorking(false); }
  };
  return createPortal(
    <div className="modal-backdrop pack-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget && !working) onClose(); }}>
      <div className="modal pack-dialog kw-review" role="dialog" aria-modal="true" aria-label={t("aimodels.kw.review.title")} ref={box}>
        <div className="modal-head"><h2>{t("aimodels.kw.review.title")}</h2></div>
        <div className="modal-body">
          <p className="pack-lead">{t("aimodels.kw.review.lead")}</p>
          <ul className="kw-list" aria-label={t("aimodels.kw.review.title")}>
            {list.length === 0 && <li className="kw-empty">{t("aimodels.kw.review.empty")}</li>}
            {list.map((x) => (
              <li key={x.name}>
                <span className="kw-name">{x.name}</span>
                <span className="kw-n">{uiFmt.num(x.photos)}</span>
                <button type="button" className="btn small" disabled={working} onClick={() => void run([x.name])}>{t("aimodels.kw.review.remove")}</button>
              </li>
            ))}
          </ul>
          <p className="kw-status" role="status">
            {done ? <>{t("aimodels.kw.review.done", { keywords: uiFmt.num(done.keywords), photos: uiFmt.num(done.photos) })}
              {done.failed > 0 ? ` ${t("aimodels.kw.review.failed", { n: done.failed })}` : ""}</>
              : confirm ? t("aimodels.kw.review.confirm", { keywords: uiFmt.num(keywords), photos: uiFmt.num(photos) }) : ""}
            {error && <span className="pack-bad"> {error}</span>}
          </p>
        </div>
        <div className="modal-foot">
          <button className="btn" data-first disabled={working} onClick={onClose}>{t("aimodels.kw.review.close")}</button>
          {confirm
            ? <button className="btn primary" disabled={working} onClick={() => void run([])}>{t("aimodels.kw.review.removeAllNow")}</button>
            : <button className="btn" disabled={working || list.length === 0} onClick={() => setConfirm(true)}>{t("aimodels.kw.review.removeAll")}</button>}
        </div>
      </div>
    </div>,
    document.body,
  );
}
