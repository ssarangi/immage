// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { barHeights } from "../statsExposure";
import { compact } from "../statsMath";
import { arrowNav } from "./arrowNav";
import "./exposure-time.css";

export interface MonthBar { month: number; label: string; n: number; top: number }

const MAX_PX = 176;
const SK = [0.3, 0.26, 0.4, 0.46, 0.52, 0.58, 0.62, 0.56, 0.7, 0.86, 0.45, 0.38];

/**
 * Twelve month columns, all years pooled: a faint layer of every frame and a solid layer of the top images on the
 * same scale. The chart is a row of buttons with a permanent border, so a picked month is tinted and nothing moves.
 */
export function Seasonality({ months, selected, onToggle, loading, title, desc, ariaFor, tipFor }: {
  months: MonthBar[]; selected: ReadonlySet<string>; onToggle(month: number, additive: boolean): void; loading?: boolean;
  title: string; desc: string; ariaFor(m: MonthBar): string; tipFor(m: MonthBar): { title: string; sub?: string };
}) {
  const all = barHeights(months.map((m) => m.n), MAX_PX);
  const max = Math.max(0, ...months.map((m) => m.n));
  const top = months.map((m) => (m.top > 0 && max > 0 ? Math.max(2, Math.round((m.top / max) * MAX_PX * 10) / 10) : 0));
  return (
    <div className="st-months" role="group" aria-label={title} aria-description={desc} onKeyDown={arrowNav}>
      {loading ? SK.map((h, i) => (
        <div key={i} className="st-month st-month-sk" aria-hidden="true"><span className="st-month-col"><i className="st-sk" style={{ height: h * MAX_PX }} /></span><i className="st-sk st-sk-line short" /></div>
      )) : months.map((m, i) => {
        const tip = tipFor(m);
        return (
          <button key={m.month} type="button" className="st-month" data-id={m.month} aria-pressed={selected.has(String(m.month))} aria-label={ariaFor(m)} data-nav
                  data-tip={tip.title} data-tip-sub={tip.sub} onClick={(e) => onToggle(m.month, e.shiftKey || e.metaKey || e.ctrlKey)}>
            <span className="st-month-col" style={{ height: all[i] }}>
              <i className="all" style={{ height: "100%" }} />
              <i className="top" style={{ height: top[i] }} />
              <span className="v">{compact(m.n)}</span>
            </span>
            <span className="m">{m.label}</span>
          </button>
        );
      })}
    </div>
  );
}
