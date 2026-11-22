// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The 24 px sparkline of a stat card: bars scaled to the largest, brighter when taller.
import { scaleBars, type Accent } from "../statsMath";
import { accentVar } from "./Shapes";

export function MiniBars({ values, accent, ramp = false }: { values: readonly number[]; accent: Accent; ramp?: boolean }) {
  const bars = scaleBars(values);
  const n = Math.max(bars.length, 1);
  const w = 120 / n;
  return (
    <svg className="st-mini-svg" viewBox="0 0 120 24" preserveAspectRatio="none" aria-hidden="true">
      {bars.map((v, i) => (
        <rect key={i} x={i * w + 1} y={24 - Math.max(v, 0.04) * 22} width={Math.max(w - 3, 1)} height={Math.max(v, 0.04) * 22} rx="2"
              fill={accentVar(accent)} opacity={ramp ? 0.45 + (i / n) * 0.55 : 0.35 + 0.65 * v} />
      ))}
    </svg>
  );
}
