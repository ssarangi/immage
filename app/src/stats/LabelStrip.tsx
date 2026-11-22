// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { t } from "../i18n";
import { LABEL_COLORS } from "../library";
import { nf } from "../statsFormat";
import { LABELS } from "../statsTop";
import type { ColorLabel } from "../statsTypes";

/**
 * The colour-label facet strip above the grid: one chip per label with how many top images carry it (counted
 * with the label filter itself left out, so the others can still be picked). A chip toggles the label filter;
 * each has a permanent border and a fixed height, and says its name in words as well as colour.
 */
export function LabelStrip({ counts, selected, loading, onToggle, onlyTop, onOnlyTop, tail }: {
  counts: Record<ColorLabel, number> | null; selected: ReadonlySet<string>; loading: boolean; onToggle(l: ColorLabel, additive: boolean): void;
  onlyTop: boolean; onOnlyTop(): void; tail?: string;
}) {
  return (
    <div className="st-lbls" role="group" aria-label={t("stats.top.labels")}>
      <span className="st-era-lbl">{t("stats.label.label")}</span>
      {LABELS.map((l, i) => (
        <button key={l} type="button" className="st-lbl" aria-pressed={selected.has(l)} onClick={(e) => onToggle(l, e.shiftKey || e.metaKey || e.ctrlKey)}
                data-tip={t("stats.top.labelTip", { name: t(`stats.label.${l}`) })}>
          <i className="st-lbl-dot" style={{ background: LABEL_COLORS[i] }} aria-hidden="true" />
          {t(`stats.label.${l}`)}
          <span className="st-lbl-n">{loading || !counts ? "–" : nf(counts[l])}</span>
        </button>
      ))}
      <span className="st-grow" />
      {tail && <span className="st-fact st-lbls-tail">{tail}</span>}
      <button type="button" className="st-opt amber st-only" aria-pressed={onlyTop} onClick={onOnlyTop} data-tip={t("stats.top.onlyTip")}>{t("stats.top.only")}</button>
    </div>
  );
}
