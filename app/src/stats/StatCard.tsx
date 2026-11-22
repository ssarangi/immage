// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import type { ReactNode } from "react";
import type { Accent } from "../statsMath";

/** A strip card: caption, one big numeral in an accent, a line of context, a mini chart. It is a button: it filters or jumps. */
export function StatCard({ k, value, unit, sub, accent, mini, onClick, disabled, loading, title }: {
  k: string; value: ReactNode; unit?: ReactNode; sub: ReactNode; accent: Accent; mini?: ReactNode; onClick?: () => void;
  disabled?: boolean; loading?: boolean; title?: string;
}) {
  return (
    <button type="button" className={`st-stat${loading ? " is-loading" : ""}`} onClick={onClick} disabled={disabled || loading} title={title}>
      <span className="st-stat-k">{k}</span>
      <span className={`st-stat-v c-${accent}`}>{loading ? <i className="st-sk st-sk-num" /> : <>{value}{unit && <small>{unit}</small>}</>}</span>
      <span className="st-stat-s">{loading ? <i className="st-sk st-sk-line" /> : sub}</span>
      <span className="st-stat-mini">{loading ? null : mini}</span>
    </button>
  );
}
