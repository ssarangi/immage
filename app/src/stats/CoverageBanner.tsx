// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { t } from "../i18n";
import { nf } from "../statsFormat";
import type { Coverage } from "../statsTypes";
import { Bold } from "./Section";
import { richParts } from "../statsFormat";

/** What every number is based on: how many photos are analysed, how many are still being read. One fixed-height row. */
export function CoverageBanner({ coverage, onAddFolders, onTogglePause }: { coverage: Coverage | null; onAddFolders(): void; onTogglePause(paused: boolean): void }) {
  const c = coverage;
  const total = c?.total ?? null;
  const frac = !c ? 0 : total && total > 0 ? Math.min(1, c.indexed / total) : c.pending > 0 ? c.indexed / Math.max(1, c.indexed + c.pending) : 1;
  const state = c?.state ?? "ready";
  const early = !!c && state === "indexing" && frac < 0.1;
  const folders = c?.pendingRoots.length ?? 0;
  return (
    <div className={`st-coverage${c ? "" : " is-loading"}`} role="status" aria-live="polite">
      {!c ? <i className="st-sk st-sk-line cov" /> : <>
        <span className={`st-cov-icon ${state}`} aria-hidden="true" />
        <span className="st-cov-text">
          {state === "ready" ? <Bold parts={richParts(t("stats.coverage.ready", { n: `**${nf(c.indexed)}**` }))} />
            : <Bold parts={richParts(total !== null ? t("stats.coverage.based", { n: `**${nf(c.indexed)}**`, total: `**${nf(total)}**` }) : t("stats.coverage.basedNoTotal", { n: `**${nf(c.indexed)}**` }))} />}
        </span>
        <span className="st-cov-bar" aria-hidden="true"><i style={{ width: `${frac * 100}%` }} /></span>
        <span className="st-cov-text st-cov-rest">
          {state === "indexing" && <Bold parts={richParts(t("stats.coverage.indexing", { n: c.pending, count: `**${nf(c.pending)}**`, folders: folders || c.roots }))} />}
          {state === "paused" && t("stats.coverage.paused")}
        </span>
        {early && <span className="st-tag amber">{t("stats.coverage.early")}</span>}
        <span className="st-grow" />
        {(state === "indexing" || state === "paused") && (
          <button type="button" className="st-link" onClick={() => onTogglePause(state === "indexing")}>{state === "paused" ? t("stats.coverage.resume") : t("stats.coverage.pause")}</button>
        )}
        <button type="button" className="st-link" onClick={onAddFolders}>{t("stats.coverage.add")}</button>
      </>}
    </div>
  );
}
