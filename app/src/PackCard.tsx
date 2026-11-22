// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// One model pack of Preferences > General > AI models: its name, license and
// size, what it does and what happens to the user's photos (always both, always
// visible), a state slot of fixed height, a fixed action row, and the pack's
// analysis switch rows. Every state draws the same boxes, so nothing moves when
// a download starts, progresses, stops or finishes (see aiModels.css and
// verify/models-shift.test.mjs).
import type { ReactNode } from "react";
import type { PackInfo } from "./bindings";
import { sizeLabel, uiFmt, type ActionId, type PackView, type PillIcon } from "./aiPacks";
import { PrefRow, PrefSwitch } from "./PrefsUI";
import { t } from "./i18n";

function PillIconSvg({ icon }: { icon: PillIcon }) {
  const props = { width: 14, height: 14, viewBox: "0 0 14 14", "aria-hidden": true as const };
  if (icon === "dashed") return <svg {...props}><circle cx="7" cy="7" r="5.5" fill="none" stroke="currentColor" strokeWidth="1.5" strokeDasharray="2.4 2.2" /></svg>;
  if (icon === "spinner") return <svg {...props} className="pack-spin"><circle cx="7" cy="7" r="5" fill="none" stroke="currentColor" strokeOpacity="0.25" strokeWidth="2" /><path d="M7 2a5 5 0 0 1 5 5" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" /></svg>;
  if (icon === "check") return <svg {...props}><circle cx="7" cy="7" r="6" fill="none" stroke="currentColor" strokeWidth="1.5" /><path d="M4.3 7.2l1.9 1.9 3.5-3.7" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" /></svg>;
  return <svg {...props}><path d="M7 1.8 13 12.2H1z" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinejoin="round" /><path d="M7 5.5v3" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" /><circle cx="7" cy="10.3" r="0.9" fill="currentColor" /></svg>;
}

export type AnalysisRowProps = {
  label: string; hint: string; checked: boolean; onChange: (on: boolean) => void;
  /** why the switch cannot be turned on, which replaces its hint */
  reason: string | null;
  /** a standing line under the switch (analysed so far), always laid out */
  status: string;
  statusLabel: string;
  /** more fixed-height rows below the status line (Auto-keywords: confidence, speed, results) */
  extra?: ReactNode;
};

export function PackCard({ info, view, name, does, photos, card, busy, flash, onAction, analysis, test }: {
  info: PackInfo; view: PackView; name: string; does: string; photos: string; card?: string;
  busy: boolean;
  /** a result line (Check files) that stands in for the slot message for a few seconds */
  flash: string | null;
  onAction: (action: ActionId) => void;
  analysis?: AnalysisRowProps;
  test?: ReactNode;
}) {
  const message = flash ?? view.message;
  const licence = info.license_url
    ? <a className="pack-chip pack-chip-link" href={info.license_url} target="_blank" rel="noreferrer" title={t("aimodels.licence", { name: info.license })}>{info.license}</a>
    : <span className="pack-chip">{info.license}</span>;
  return (
    <section className="pack" data-pref-card={card} data-pack={info.id} data-state={info.state}>
      <div className="pack-h">
        <div className="pack-t">
          <h5>{name}</h5>
          {licence}
          <span className="pack-chip">{sizeLabel(info.state === "installed" ? info.bytes_on_disk : info.bytes, uiFmt)}</span>
        </div>
        <span className={`pack-st ${view.pill.kind}`} role="status">
          <PillIconSvg icon={view.pill.icon} /><span className="pack-st-text">{view.pill.text}</span>
        </span>
      </div>
      <dl className="pack-dl">
        <dt>{t("aimodels.whatItDoes")}</dt><dd>{does}</dd>
        <dt>{t("aimodels.whatHappens")}</dt><dd className="quiet">{photos}</dd>
      </dl>
      <div className={`pack-slot${view.bar != null ? " show" : ""}`} data-tone={flash ? "plain" : view.tone}>
        <div className="pack-msg" role={view.tone === "bad" ? "alert" : undefined} title={info.error?.detail}>
          <span className="pack-msg-text">{message}</span>
          <span className="pack-msg-right">{flash ? "" : view.right}</span>
        </div>
        <div className="pack-bar" aria-hidden={view.bar == null}><i style={{ width: `${Math.round((view.bar ?? 0) * 100)}%` }} /></div>
      </div>
      <div className="pack-acts">
        <span className="pack-acts-grow">{test}</span>
        {view.actions.map((a) => (
          <button key={a.id} className={`btn small pack-btn${a.primary ? " primary" : ""}`} disabled={busy && a.id !== "cancel"}
                  onClick={() => onAction(a.id)}>{a.label}</button>
        ))}
      </div>
      {analysis && <div className="pack-an">
        <PrefRow label={analysis.label} hint={analysis.reason ?? analysis.hint}>
          <PrefSwitch checked={analysis.checked && !analysis.reason} disabled={!!analysis.reason || busy} aria-label={analysis.label}
                      onChange={analysis.onChange} />
        </PrefRow>
        <PrefRow label={analysis.statusLabel} hint={analysis.status}><span /></PrefRow>
        {analysis.extra}
      </div>}
    </section>
  );
}
