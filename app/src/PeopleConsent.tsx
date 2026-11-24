// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The People view while People is off: one card in three steps, at a fixed size so a step's state replaces
// its text in place and nothing moves. 1: the model pack (the Preferences card's state and buttons).
// 2: what to scan. 3: the consent line and the button. Face data is biometric, so nothing starts before
// the box is ticked, and the box is the consent record (peopleStore.turnOn).
import { UserRound } from "lucide-react";
import { useEffect, useId, useState } from "react";
import type { PackInfo } from "./bindings";
import { packView, uiFmt, type ActionId } from "./aiPacks";
import { t } from "./i18n";
import type { Scope } from "./peopleStore";

export function PeopleConsent({ pack, folder, busy, error, onPack, onTurnOn }: {
  pack: PackInfo | null; folder: string | null; hasCatalog?: boolean; busy: boolean; error: string;
  onPack: (a: ActionId) => void; onTurnOn: (scope: Scope) => void;
}) {
  const [scope, setScope] = useState<Scope>(folder ? "folder" : "catalog");   // the open folder is the safe default: the Catalog may hold network sources
  const [touched, setTouched] = useState(false);
  useEffect(() => { if (!touched) setScope(folder ? "folder" : "catalog"); }, [folder, touched]);
  const [agreed, setAgreed] = useState(false);
  const why = useId();
  const view = pack ? packView(pack, t, uiFmt) : null;
  const folderName = folder ? folder.split(/[\\/]/).filter(Boolean).pop() ?? folder : "";
  const ready = !!view?.usable;
  // the reason the button waits, in one fixed line under it
  const reason = !ready ? t("people.consent.needModel") : !agreed ? t("people.consent.needCheck") : scope === "folder" && !folder ? t("people.consent.needFolder") : "";
  const can = ready && agreed && !(scope === "folder" && !folder) && !busy;
  const action = view?.actions[0];
  const pct = view?.bar != null ? Math.round(view.bar * 100) : 0;
  return (
    <div className="people-consent-wrap">
      <section className="people-consent" aria-labelledby="people-consent-title">
        <h2 id="people-consent-title"><UserRound size={20} aria-hidden="true" />{t("people.consent.title")}</h2>
        <p className="lead">{t("people.consent.lead")}</p>
        <p className="body">{t("people.consent.body")}</p>

        <div className="people-step" data-done={ready || undefined}>
          <i aria-hidden="true">1</i>
          <div>
            <h3>{t("people.consent.model")}</h3>
            <div className="people-slot" aria-live="polite">
              <span className="people-slot-text" title={view?.message}>{view ? `${view.pill.text}${view.message ? `. ${view.message}` : ""}` : t("people.consent.reading")}</span>
              {action && <button type="button" className={`people-btn${action.primary ? " primary" : ""}`} disabled={busy} onClick={() => onPack(action.id)}>{action.label}</button>}
            </div>
            <span className="people-bar-track" style={{ visibility: view?.bar != null ? "visible" : "hidden" }}><i style={{ width: `${pct}%` }} /></span>
          </div>
        </div>

        <div className="people-step" data-done={ready || undefined}>
          <i aria-hidden="true">2</i>
          <div role="radiogroup" aria-label={t("people.consent.scan")}>
            <h3>{t("people.consent.scan")}</h3>
            <label className="people-radio"><input type="radio" name="people-scope" checked={scope === "catalog"} onChange={() => { setTouched(true); setScope("catalog"); }} />{t("people.consent.scopeCatalog")}</label>
            <label className="people-radio" aria-disabled={!folder}>
              <input type="radio" name="people-scope" checked={scope === "folder"} disabled={!folder} onChange={() => { setTouched(true); setScope("folder"); }} />
              {folder ? t("people.consent.scopeFolder", { name: folderName }) : t("people.consent.scopeNone")}
            </label>
          </div>
        </div>

        <div className="people-step" data-done={undefined}>
          <i aria-hidden="true">3</i>
          <div>
            <h3>{t("people.consent.start")}</h3>
            <label className="check"><input type="checkbox" checked={agreed} onChange={(e) => setAgreed(e.target.checked)} aria-describedby={why} />{t("people.consent.check")}</label>
            <p className="why" id={why}>{reason}</p>
            <button type="button" className="people-btn primary big" disabled={!can} title={reason || undefined} onClick={() => onTurnOn(scope)}>{t("people.consent.turnOn")}</button>
          </div>
        </div>
        <p className="error" role="alert">{error}</p>

        <div className="foot">
          <span>{t("people.consent.eyesNote")}</span>
          <button type="button" className="people-btn" disabled title={t("people.consent.eyesSoon")}>{t("people.consent.eyesOnly")}</button>
        </div>
      </section>
    </div>
  );
}
