// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { fanWedges, fovDegrees } from "../statsMath";
import { arrowNav, pressKey } from "./arrowNav";

export interface FanBucket { id: string; label: string; modalMm: number; n: number }

const OX = 24, OY = 170, W = 480, H = 340;

/**
 * The field of view as a fan: one wedge per focal range at that range's angle of view on a 35 mm frame,
 * its reach by photo count. Presets above the fan are the keyboard route to the same selection.
 */
export function FovFan({ buckets, selected, onToggle, title, ariaFor, tipFor, loading }: {
  buckets: FanBucket[]; selected: ReadonlySet<string>; onToggle(id: string, additive: boolean): void; title: string;
  ariaFor(b: FanBucket, degrees: number): string; tipFor(b: FanBucket, degrees: number): { title: string; sub?: string }; loading?: boolean;
}) {
  const wedges = fanWedges(buckets, OX, OY);
  const byId = new Map(buckets.map((b) => [b.id, b]));
  return (
    <svg className={`st-fan${loading ? " is-loading" : ""}`} viewBox={`0 0 ${W} ${H}`} role="group" aria-label={title} onKeyDown={arrowNav}>
      <title>{title}</title>
      {[100, 200, 300].map((r) => <path key={r} d={`M${OX + r} ${OY - 8} A${r} ${r} 0 0 1 ${OX + r} ${OY + 8}`} className="st-arc" />)}
      {!loading && wedges.map((w) => {
        const b = byId.get(w.id)!;
        const on = selected.has(w.id);
        const deg = fovDegrees(w.mm);
        const tip = tipFor(b, deg);
        return (
          <g key={w.id} className={`st-hit st-wedge${on ? " on" : ""}`} tabIndex={0} role="button" aria-pressed={on} aria-label={ariaFor(b, deg)}
             data-nav data-tip={tip.title} data-tip-sub={tip.sub} onClick={(e) => onToggle(w.id, e.shiftKey || e.metaKey || e.ctrlKey)}
             onKeyDown={(e) => pressKey(e, () => onToggle(w.id, e.shiftKey))}>
            <path className="st-fx w" d={`M${OX} ${OY} L${w.x.toFixed(1)} ${w.y1.toFixed(1)} A${w.len.toFixed(1)} ${w.len.toFixed(1)} 0 0 1 ${w.x.toFixed(1)} ${w.y2.toFixed(1)}Z`} />
          </g>
        );
      })}
      {!loading && wedges.map((w) => (
        <text key={w.id} x={(w.labelX - 4).toFixed(0)} y={(w.labelY - 5).toFixed(0)} className={`st-fan-label${selected.has(w.id) ? " on" : ""}`}>{Math.round(w.mm)}mm</text>
      ))}
      <rect x={OX - 16} y={OY - 9} width="18" height="18" rx="4" className="st-cam" />
      <circle cx={OX - 7} cy={OY} r="4" className="st-cam-lens" />
    </svg>
  );
}
