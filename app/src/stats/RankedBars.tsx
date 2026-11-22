// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import type { Accent } from "../statsMath";
import { accentVar } from "./Shapes";

export interface RankRow {
  id: string;
  name: string;
  sub?: string;
  /** widths 0..1 of the faint (all frames) and bright (top frames) layers */
  all: number;
  top: number;
  v1: string;
  v2: string;
  /** a hatched top layer and a "small sample" tooltip */
  small?: boolean;
  tip: { title: string; sub?: string };
}

/**
 * Rows of rank, name, a two-layer bar and two values. Each row is a button that filters; rows have a
 * fixed height, a permanent 1 px border, and hover changes only the background.
 */
export function RankedBars({ rows, accent, selected, onToggle, v2Kind, loadingRows = 0, ariaFor }: {
  rows: RankRow[]; accent: Accent; selected: ReadonlySet<string>; onToggle(id: string, additive: boolean): void; v2Kind: "pill" | "muted";
  loadingRows?: number; ariaFor(r: RankRow, i: number): string;
}) {
  return (
    <div className="st-rows st-rank" style={{ ["--c" as string]: accentVar(accent) }}>
      {loadingRows > 0 && Array.from({ length: loadingRows }, (_, i) => (
        <div key={i} className="st-row st-row-sk" aria-hidden="true"><span className="st-no">{i + 1}</span><i className="st-sk st-sk-line" /><i className="st-sk st-sk-bar" /><i className="st-sk st-sk-val" /><i className="st-sk st-sk-val" /></div>
      ))}
      {rows.map((r, i) => (
        <button key={r.id} type="button" className="st-row" aria-pressed={selected.has(r.id)} aria-label={ariaFor(r, i)}
                data-tip={r.tip.title} data-tip-sub={r.tip.sub} onClick={(e) => onToggle(r.id, e.shiftKey || e.metaKey || e.ctrlKey)}>
          <span className="st-no">{i + 1}</span>
          <span className="st-nm">{r.name}{r.sub && <small>{r.sub}</small>}</span>
          <span className="st-bar2" aria-hidden="true">
            <i className="all" style={{ width: `${r.all * 100}%` }} />
            <i className={`top${r.small ? " hatch" : ""}`} style={{ width: `${r.top * 100}%` }} />
          </span>
          <span className="st-num">{r.v1}</span>
          <span className={v2Kind === "pill" ? "st-keep" : "st-pc-muted"}>{r.v2}</span>
        </button>
      ))}
    </div>
  );
}
