// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { barHeights, type ExpAccent } from "../statsExposure";
import { arrowNav } from "./arrowNav";
import { accentVar } from "./Shapes";
import "./exposure-time.css";

export interface ShutterBar { id: string; name: string; range: string; n: number; count: string; accent: ExpAccent }

const MAX_PX = 236;
const SK = [0.34, 0.5, 0.86, 0.6, 0.3, 0.14];

/** The six shutter regimes in time order: a gradient bar per regime, its count above, its name and range below. */
export function ShutterBars({ bars, selected, onToggle, loading, title, desc, ariaFor, tipFor }: {
  bars: ShutterBar[]; selected: ReadonlySet<string>; onToggle(id: string, additive: boolean): void; loading?: boolean;
  title: string; desc: string; ariaFor(b: ShutterBar): string; tipFor(b: ShutterBar): { title: string; sub?: string };
}) {
  const heights = barHeights(bars.map((b) => b.n), MAX_PX);
  return (
    <div className="st-shutter" role="group" aria-label={title} aria-description={desc} onKeyDown={arrowNav}>
      {loading ? SK.map((h, i) => (
        <div key={i} className="st-shutter-col st-shutter-sk" aria-hidden="true"><span className="st-shutter-bar"><i className="st-sk" style={{ height: h * MAX_PX }} /></span><i className="st-sk st-sk-line short" /></div>
      )) : bars.map((b, i) => {
        const tip = tipFor(b);
        return (
          <button key={b.id} type="button" className="st-shutter-col" data-id={b.id} style={{ ["--c" as string]: accentVar(b.accent) }} aria-pressed={selected.has(b.id)}
                  aria-label={ariaFor(b)} data-nav data-tip={tip.title} data-tip-sub={tip.sub} onClick={(e) => onToggle(b.id, e.shiftKey || e.metaKey || e.ctrlKey)}>
            <span className="st-shutter-bar"><i style={{ height: heights[i] }}><b>{b.count}</b></i></span>
            <span className="st-shutter-nm">{b.name}</span>
            <span className="st-shutter-rg">{b.range}</span>
          </button>
        );
      })}
    </div>
  );
}
