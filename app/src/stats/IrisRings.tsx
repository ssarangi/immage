// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { irisRadius, polygonPoints, type ExpAccent } from "../statsExposure";
import { arrowNav } from "./arrowNav";
import { accentVar } from "./Shapes";
import "./exposure-time.css";

export interface IrisGroup { id: string; label: string; name: string; fNumber: number; n: number; share: number; accent: ExpAccent; count: string; percent: string }

const R = 50, CIRC = 2 * Math.PI * R;

/**
 * Five aperture groups as rings: a share arc around a seven-blade outline, with a seven-sided hole that shrinks
 * as the f-number grows. Each ring is a button with a permanent border; picking one tints it and glows its arc.
 */
export function IrisRings({ groups, selected, onToggle, loading, title, desc, ariaFor, tipFor }: {
  groups: IrisGroup[]; selected: ReadonlySet<string>; onToggle(id: string, additive: boolean): void; loading?: boolean;
  title: string; desc: string; ariaFor(g: IrisGroup): string; tipFor(g: IrisGroup): { title: string; sub?: string };
}) {
  return (
    <div className="st-iris" role="group" aria-label={title} aria-description={desc} onKeyDown={arrowNav}>
      {loading ? Array.from({ length: 5 }, (_, i) => (
        <div key={i} className="st-iris-cell st-iris-sk" aria-hidden="true"><i className="st-sk st-sk-disc" /><i className="st-sk st-sk-line" /><i className="st-sk st-sk-line short" /></div>
      )) : groups.map((g) => {
        const on = selected.has(g.id);
        const tip = tipFor(g);
        return (
          <button key={g.id} type="button" className="st-iris-cell" data-id={g.id} style={{ ["--c" as string]: accentVar(g.accent) }} aria-pressed={on} aria-label={ariaFor(g)} data-nav
                  data-tip={tip.title} data-tip-sub={tip.sub} onClick={(e) => onToggle(g.id, e.shiftKey || e.metaKey || e.ctrlKey)}>
            <svg viewBox="0 0 120 120" width="100" height="100" aria-hidden="true">
              <circle cx="60" cy="60" r={R} fill="none" stroke="var(--track)" strokeWidth="5" />
              <circle className="st-iris-arc" cx="60" cy="60" r={R} fill="none" stroke="var(--c)" strokeWidth="5" strokeLinecap="round"
                      strokeDasharray={`${Math.max(g.share * CIRC, g.n > 0 ? 1 : 0)} ${CIRC}`} transform="rotate(-90 60 60)" />
              <polygon points={polygonPoints(7, 38, 60, 60)} fill="color-mix(in srgb, var(--c) 14%, transparent)" stroke="var(--c)" strokeWidth="1.5" strokeOpacity=".7" />
              <polygon points={polygonPoints(7, irisRadius(g.fNumber), 60, 60)} fill="var(--c)" fillOpacity=".92" />
            </svg>
            <span className="st-iris-l1">{g.label}</span>
            <span className="st-iris-l2">{g.count} {"·"} {g.percent}</span>
            <span className="st-iris-l3">{g.name}</span>
          </button>
        );
      })}
    </div>
  );
}
