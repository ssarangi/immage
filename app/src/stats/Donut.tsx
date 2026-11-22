// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { donutArcs, type Mark } from "../statsMath";
import { arrowNav, pressKey } from "./arrowNav";
import { accentVar } from "./Shapes";

/**
 * A ring of arcs on one circle (dash arrays), 3 px gaps, biggest first from 12 o'clock. A selected
 * slice changes nothing in size: the others dim, and the centre shows what was picked.
 */
export function Donut({ marks, center, size = 236, selected, onToggle, title, desc, ariaFor, tipFor, loading }: {
  marks: Mark[]; center: { value: string; caption: string }; size?: number; selected: ReadonlySet<string>;
  onToggle(id: string, additive: boolean): void; title: string; desc: string; ariaFor(m: Mark): string;
  tipFor(m: Mark): { title: string; sub?: string }; loading?: boolean;
}) {
  const R = 84, C = 118, STROKE = 26;
  const circ = 2 * Math.PI * R;
  const arcs = donutArcs(marks, R, 3);
  const any = selected.size > 0;
  return (
    <svg className={`st-donut${loading ? " is-loading" : ""}`} viewBox="0 0 236 236" width={size} height={size} role="group" aria-label={title}
         onKeyDown={arrowNav}>
      <title>{title}</title><desc>{desc}</desc>
      {loading && <circle cx={C} cy={C} r={R} fill="none" stroke="var(--inset)" strokeWidth={STROKE} className="st-sk-ring" />}
      {!loading && marks.map((m, i) => {
        const a = arcs[i];
        const on = selected.has(m.id);
        const dash = `${a.len} ${circ}`;
        const tip = tipFor(m);
        return (
          <g key={m.id} className={`st-hit${any && !on ? " dim" : ""}`} tabIndex={0} role="button" aria-pressed={on} aria-label={ariaFor(m)}
             data-nav data-tip={tip.title} data-tip-sub={tip.sub} onClick={(e) => onToggle(m.id, e.shiftKey || e.metaKey || e.ctrlKey)}
             onKeyDown={(e) => pressKey(e, () => onToggle(m.id, e.shiftKey))}>
            <circle cx={C} cy={C} r={R} fill="none" stroke={accentVar(m.accent)} strokeWidth={STROKE} strokeDasharray={dash}
                    strokeDashoffset={-a.offset} transform={`rotate(-90 ${C} ${C})`} />
            <circle className="st-fx" cx={C} cy={C} r={R + STROKE / 2 + 1} fill="none" strokeDasharray={dash} strokeDashoffset={-a.offset}
                    transform={`rotate(-90 ${C} ${C})`} />
            <circle className="st-fx" cx={C} cy={C} r={R - STROKE / 2 - 1} fill="none" strokeDasharray={dash} strokeDashoffset={-a.offset}
                    transform={`rotate(-90 ${C} ${C})`} />
          </g>
        );
      })}
      {!loading && <>
        <text x={C} y={C - 2} textAnchor="middle" className="st-donut-center">{center.value}</text>
        <text x={C} y={C + 18} textAnchor="middle" className="st-donut-sub">{center.caption}</text>
      </>}
    </svg>
  );
}
