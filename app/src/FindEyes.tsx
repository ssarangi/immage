// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Find's Closed eyes row (notes/faces-ux-spec.md 5.4): keeps the photos whose main face has closed or half closed eyes. The
// verdicts are made on request, so switching it on has the app ask for the open folder's photos in chunks and says how far it is;
// only photos already checked can match, and the line says so while it runs. Shown only while Faces is on and the optional
// eye model file is there.
import { formatNumber, t } from "./i18n";
import type { FindState } from "./findModel";
import type { ImageEntry } from "./library";
import { useEyesScan } from "./eyesStore";

export function ClosedEyesEditor({ state, send }: { state: FindState; send: (next: FindState) => void; images: ImageEntry[] }) {
  const on = !!state.filter.closedEyes;
  const scan = useEyesScan();
  return (
    <div className="ff">
      <p className="fe-hint">{t("library.find.eyes.hint")}</p>
      <div className="meta-grid">
        <span className="k">{t("library.find.eyes.title")}</span>
        <input type="checkbox" className="switch" aria-label={t("library.find.eyes.title")} checked={on}
               onChange={(e) => send({ ...state, filter: { ...state.filter, closedEyes: e.target.checked } })} />
      </div>
      <p className="faces-line" role="status">{on && scan.running ? t("library.find.eyes.progress", { done: formatNumber(scan.done), total: formatNumber(scan.total) })
        : on && scan.known > 0 ? t("library.find.eyes.done", { n: scan.known, count: formatNumber(scan.known) }) : ""}</p>
    </div>
  );
}
