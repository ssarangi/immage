// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The structure of the culling lightbox, with no behaviour of its own: a small
// Immage window with one surface and two floating panels on it, the stage that
// holds the matted photo and the report panel, between a head and a foot. The
// stage is the same size whatever the report shows. GridCulling fills it with
// the live photo; app/harness/critique.tsx fills it with a fixture.
import { ChevronLeft, ChevronRight, SlidersHorizontal, Tag, X } from "lucide-react";
import type { ReactNode } from "react";
import { formatNumber, t } from "./i18n";

export function LightboxHead({ filename, index, total, onEdit, onKeywords, onClose, context }: {
  filename: string; index: number; total: number; onEdit: () => void; onKeywords: () => void; onClose: () => void;
  context?: string;   // the scene view: the scene's time and frame count
}) {
  return <div className="culling-toolbar">
    <div className="culling-photo-info">
      <bdi className="culling-filename" title={filename}>{filename}</bdi>
      <span className="culling-photo-count">{t("library.culling.position", { n: formatNumber(index), total: formatNumber(total) })}</span>
      {context && <span className="culling-photo-count">{context}</span>}
    </div>
    <div className="culling-actions">
      <button type="button" className="tool labeled" title={t("library.culling.edit")} onClick={onEdit}>
        <SlidersHorizontal size={16} aria-hidden="true" /><span>{t("library.culling.edit")}</span></button>
      <button type="button" className="tool" aria-label={t("library.culling.keywords")} title={t("library.culling.keywords")}
        onClick={onKeywords}><Tag size={16} aria-hidden="true" /></button>
      <button type="button" className="tool close" aria-label={t("library.culling.close")} title={t("library.culling.close")}
        onClick={onClose}><X size={16} aria-hidden="true" /></button>
    </div>
  </div>;
}

// previous and next sit in the stage and show on its hover or focus
export function StageArrows({ hasPrevious, hasNext, onStep }: { hasPrevious: boolean; hasNext: boolean; onStep: (dir: -1 | 1) => void }) {
  return <>{([-1, 1] as const).map((dir) => {
    const ok = dir < 0 ? hasPrevious : hasNext;
    const label = t(dir < 0 ? "library.culling.previous" : "library.culling.next");
    return <button key={dir} type="button" className={`culling-arrow ${dir < 0 ? "previous" : "next"}`} disabled={!ok}
      aria-label={label} title={label} onClick={() => onStep(dir)}>
      {dir < 0 ? <ChevronLeft size={18} aria-hidden="true" /> : <ChevronRight size={18} aria-hidden="true" />}</button>;
  })}</>;
}

export function CullLightbox({ head, stage, side, foot, dialogLabel }: {
  head: ReactNode; stage: ReactNode; side: ReactNode; foot: ReactNode; dialogLabel?: string;
}) {
  return <div className="culling-lightbox" role={dialogLabel ? "dialog" : undefined} aria-label={dialogLabel}>
    {head}
    <section className="culling-stage"><div className="culling-stage-inner">{stage}</div></section>
    {side}
    {foot}
  </div>;
}
