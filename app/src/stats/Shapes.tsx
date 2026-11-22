// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Series markers: a legend never relies on colour alone, so each series has its own shape too.
import type { Accent, ShapeKind } from "../statsMath";

export const accentVar = (a: Accent) => `var(--stat-${a})`;

const PATHS: Record<ShapeKind, (c: string) => React.ReactNode> = {
  circle: (c) => <circle cx="6" cy="6" r="5.5" fill={c} />,
  square: (c) => <rect x=".5" y=".5" width="11" height="11" rx="2" fill={c} />,
  diamond: (c) => <path d="M6 .2 11.8 6 6 11.8.2 6z" fill={c} />,
  triangle: (c) => <path d="M6 .5 11.6 11H.4z" fill={c} />,
  hexagon: (c) => <path d="M3 1h6l3 5-3 5H3L0 6z" fill={c} />,
  dash: (c) => <rect y="4" width="12" height="4" rx="2" fill={c} />,
};

export function ShapeMark({ kind, accent }: { kind: ShapeKind; accent: Accent }) {
  return <svg className="st-sw" viewBox="0 0 12 12" aria-hidden="true">{PATHS[kind](accentVar(accent))}</svg>;
}
