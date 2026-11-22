// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { t } from "../i18n";

const BARS = [0.35, 0.6, 0.9, 0.55, 0.3];

/** No search roots (or nothing analysed yet): one card in place of the strip, the tabs and the sections. */
export function FirstRun({ kind, onAdd, onLearn }: { kind: "no-roots" | "empty"; onAdd(): void; onLearn(): void }) {
  return (
    <div className="st-card st-firstrun" role="region" aria-label={t("stats.firstrun.title")}>
      <svg viewBox="0 0 200 90" className="st-firstrun-art" aria-hidden="true">
        <circle cx="48" cy="45" r="30" className="line" /><polygon points="48,24 66,34 66,56 48,66 30,56 30,34" className="line" />
        <polygon points="48,36 57,41 57,51 48,56 39,51 39,41" className="line" />
        {BARS.map((b, i) => <rect key={i} x={104 + i * 18} y={80 - b * 66} width="12" height={b * 66} rx="3" className="line" />)}
      </svg>
      <h3>{kind === "empty" ? t("stats.firstrun.emptyTitle") : t("stats.firstrun.title")}</h3>
      <p className="st-sub">{kind === "empty" ? t("stats.firstrun.emptyBody") : t("stats.firstrun.body")}</p>
      <div className="st-firstrun-actions">
        <button type="button" className="st-btn primary" onClick={onAdd}>{t("stats.coverage.add")}</button>
        <button type="button" className="st-btn" onClick={onLearn}>{t("stats.firstrun.learn")}</button>
      </div>
    </div>
  );
}
