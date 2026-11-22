// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { barHeights } from "../statsExposure";
import { arrowNav } from "./arrowNav";
import "./exposure-time.css";

export interface IsoBar { id: string; label: string; n: number }

const MAX_PX = 58;

/** The seven ISO stops, 100 to 6400+, as short violet bars with mono labels; each is a button. */
export function IsoLadder({ bars, selected, onToggle, loading, title, desc, ariaFor, tipFor }: {
  bars: IsoBar[]; selected: ReadonlySet<string>; onToggle(id: string, additive: boolean): void; loading?: boolean;
  title: string; desc: string; ariaFor(b: IsoBar): string; tipFor(b: IsoBar): { title: string; sub?: string };
}) {
  const heights = barHeights(bars.map((b) => b.n), MAX_PX);
  return (
    <div className="st-iso" role="group" aria-label={title} aria-description={desc} onKeyDown={arrowNav}>
      {loading ? [0.4, 0.6, 1, 0.85, 0.7, 0.45, 0.3].map((h, i) => (
        <div key={i} className="st-iso-col st-iso-sk" aria-hidden="true"><i className="st-sk" style={{ height: h * MAX_PX }} /><span>{" "}</span></div>
      )) : bars.map((b, i) => {
        const tip = tipFor(b);
        return (
          <button key={b.id} type="button" className="st-iso-col" data-id={b.id} aria-pressed={selected.has(b.id)} aria-label={ariaFor(b)} data-nav
                  data-tip={tip.title} data-tip-sub={tip.sub} onClick={(e) => onToggle(b.id, e.shiftKey || e.metaKey || e.ctrlKey)}>
            <i style={{ height: heights[i] }} />
            <span>{b.label}</span>
          </button>
        );
      })}
    </div>
  );
}
