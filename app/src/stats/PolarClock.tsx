// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { useState } from "react";
import { DAYPART_ACCENT, daypart, hourPoint, peakIndex, wedgePath, wedgeRadius } from "../statsExposure";
import { arrowNav, pressKey } from "./arrowNav";
import { accentVar } from "./Shapes";
import "./exposure-time.css";

const CX = 190, CY = 190, INNER = 58, OUTER = 128;

/**
 * 24 wedges, one per hour, hour 0 at 12 o'clock, clockwise. The length is the photo count; early and late light
 * are amber, night violet, the day cyan. The centre shows the peak (or the one picked) hour, and the hour under the
 * pointer or focus replaces it by CSS alone, so no text is added or moved. Arrow keys move between wedges.
 */
export function PolarClock({ counts, selected, onToggle, loading, title, desc, peakCaption, pickedCaption, hourText, ariaFor, tipFor, captionFor }: {
  counts: number[]; selected: ReadonlySet<string>; onToggle(hour: number, additive: boolean): void; loading?: boolean;
  title: string; desc: string; peakCaption: string; pickedCaption: string; hourText(h: number): string;
  ariaFor(h: number): string; tipFor(h: number): { title: string; sub?: string }; captionFor(h: number): string;
}) {
  const max = Math.max(0, ...counts);
  const peak = peakIndex(counts);
  const picked = selected.size === 1 ? Number([...selected][0]) : -1;
  const shown = picked >= 0 ? picked : peak;
  const [cur, setCur] = useState(0);
  const any = selected.size > 0;
  return (
    <svg className={`st-clock${loading ? " is-loading" : ""}`} viewBox="0 0 380 390" width="100%" role="group" aria-label={title} onKeyDown={arrowNav}>
      <title>{title}</title><desc>{desc}</desc>
      {[0.33, 0.66, 1].map((f) => <circle key={f} className="st-clock-ring" cx={CX} cy={CY} r={INNER + f * (OUTER - INNER)} />)}
      {[0, 6, 12, 18].map((h) => { const p = hourPoint(h, OUTER + 22, CX, CY); return <text key={h} x={p.x} y={p.y + 4} textAnchor="middle">{hourText(h)}</text>; })}
      {loading ? Array.from({ length: 24 }, (_, h) => <path key={h} className="st-sk-wedge" d={wedgePath(h, INNER, INNER + 18 + ((h * 7) % 9) * 6, CX, CY)} />)
        : counts.map((n, h) => {
          const on = selected.has(String(h));
          const d = wedgePath(h, INNER, wedgeRadius(n, max, INNER, OUTER), CX, CY);
          return (
            <g key={h} className={`st-hit st-hour${any && !on ? " dim" : ""}`} data-id={h} tabIndex={h === cur ? 0 : -1} role="button" aria-pressed={on} aria-label={ariaFor(h)} data-nav
               data-tip={tipFor(h).title} data-tip-sub={tipFor(h).sub} onFocus={() => setCur(h)}
               onClick={(e) => onToggle(h, e.shiftKey || e.metaKey || e.ctrlKey)} onKeyDown={(e) => pressKey(e, () => onToggle(h, e.shiftKey))}>
              <path className={`st-wedge-fill dp-${daypart(h)}`} d={d} fill={accentVar(DAYPART_ACCENT[daypart(h)])} />
              <path className="st-fx" d={d} />
              <g className="st-clock-c" aria-hidden="true">
                <text x={CX} y={CY - 2} textAnchor="middle" className="st-clock-big">{hourText(h)}</text>
                <text x={CX} y={CY + 16} textAnchor="middle">{captionFor(h)}</text>
              </g>
            </g>
          );
        })}
      {!loading && shown >= 0 && (
        <g className="st-clock-peak" aria-hidden="true">
          <text x={CX} y={CY - 2} textAnchor="middle" className="st-clock-big">{hourText(shown)}</text>
          <text x={CX} y={CY + 16} textAnchor="middle">{picked >= 0 ? pickedCaption : peakCaption}</text>
        </g>
      )}
    </svg>
  );
}
