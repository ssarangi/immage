// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import type { ReactNode } from "react";
import { t } from "../i18n";
import { nf } from "../statsFormat";

export interface Tab { id: string; label: string; count?: number | null }

/** Sticky underline tabs (two fixed-height rows with the filter bar) and the primary Open in grid button. */
export function StatsNav({ tabs, active, onJump, openCount, onOpenGrid, busy, filterBar, refreshing, ref }: {
  ref?: React.Ref<HTMLElement>; tabs: Tab[]; active: string; onJump(id: string): void; openCount: number | null; onOpenGrid(): void; busy: boolean; filterBar: ReactNode; refreshing: boolean;
}) {
  return (
    <nav className="st-nav" ref={ref} aria-label={t("stats.nav.label")}>
      <div className="st-wrap">
        <div className="st-nav-row">
          {tabs.map((tab) => (
            <button key={tab.id} type="button" className="st-tab" aria-current={active === tab.id ? "true" : undefined} onClick={() => onJump(tab.id)}>
              {tab.label}{tab.count != null && tab.count > 0 && <span className="st-tab-n">{nf(tab.count)}</span>}
            </button>
          ))}
          <span className="st-grow" />
          <button type="button" className="st-btn primary" onClick={onOpenGrid} disabled={!openCount || busy}>
            {t("stats.openGrid", { n: openCount ?? 0, count: nf(openCount ?? 0) })}
          </button>
        </div>
        {filterBar}
      </div>
      <div className={`st-progress${refreshing ? " on" : ""}`} aria-hidden="true" />
    </nav>
  );
}
