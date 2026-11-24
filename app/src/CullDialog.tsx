// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Auto Cull's setup: what to cull and how hard. Run closes the dialog; the
// analysis goes on in the activity line (background.ts runCull), which names
// the photo it is on and stops it. The starting values are Preferences >
// Culling's.
import { useFacesOn, usePeople } from "./peopleStore";
import { useEffect, useRef, useState } from "react";
import type { CullSettings } from "./bindings";
import { loadCullDefaults } from "./cullDefaults";
import { clampPercent, RATIOS, SHOOT_TYPES, type CullDefaults } from "./cullModel";
import { formatNumber, t } from "./i18n";

export function CullDialog({ count, selected, label, aiReady, onRun, onClose, onOpenPrefs }: {
  count: number;              // photos in the scope
  selected: boolean;          // the scope is the selection, not the whole listing
  label: string;              // the folder's name, for the heading
  aiReady: boolean;           // a Gemini key is saved
  onRun: (settings: CullSettings) => void;
  onClose: () => void;
  onOpenPrefs: () => void;    // Preferences > Culling
}) {
  const [v, setV] = useState<CullDefaults>(loadCullDefaults);
  const facesOn = useFacesOn();
  const eyes = usePeople().status?.eyes_available ?? false;
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => { box.current?.querySelector<HTMLElement>("button.primary")?.focus(); }, []);
  const set = (p: Partial<CullDefaults>) => setV((o) => ({ ...o, ...p }));
  const num = (s: string, d: number) => (s.trim() === "" || !Number.isFinite(Number(s)) ? d : Number(s));
  const run = () => onRun({
    shootType: v.shootType, duplicateRatio: v.duplicateRatio, highlightPercent: v.highlightPercent,
    blurThreshold: v.blurThreshold, similarityThreshold: v.similarityThreshold, groupSimilar: true, filterBlurry: true,
    // faces only count while Faces is on; the backend ignores them otherwise, but the choice is kept as the user made it
    useFaces: v.useFaces, eyeMode: v.eyeMode,
    eyesRank: v.eyesRank,
  });
  return <div className="modal-backdrop" onPointerDown={(e) => { if (e.target === e.currentTarget) onClose(); }}
    onKeyDown={(e) => {
      e.stopPropagation();
      if (e.key === "Escape") { e.preventDefault(); onClose(); }
      if (e.key === "Enter" && (e.target as HTMLElement).tagName !== "SELECT" && (e.target as HTMLElement).tagName !== "SUMMARY") { e.preventDefault(); run(); }
    }}>
    <div ref={box} className="modal cull-dialog" role="dialog" aria-modal="true" aria-labelledby="cull-title">
      <div className="modal-head"><h2 id="cull-title">{t("cull.dialog.title")}</h2></div>
      <div className="modal-body">
        <p className="note-plain">{t(selected ? "cull.dialog.scopeSelection" : "cull.dialog.scopeFolder",
          { n: count, count: formatNumber(count), name: label })}</p>
        <p className="note-plain">{t("cull.dialog.promise")}</p>
        <div className="cull-field">
          <label htmlFor="cull-shoot">{t("cull.dialog.shoot")}</label>
          <select id="cull-shoot" className="field" value={v.shootType} onChange={(e) => set({ shootType: e.target.value })}>
            {SHOOT_TYPES.map((k) => <option key={k} value={k}>{t(`cull.shoot.${k}`)}</option>)}
          </select>
        </div>
        <div className="cull-field">
          <span id="cull-ratio-label">{t("cull.dialog.ratio")}</span>
          <div className="segmented small" role="group" aria-labelledby="cull-ratio-label">
            {RATIOS.map((k) => <button key={k} type="button" className={v.duplicateRatio === k ? "active" : ""}
              aria-pressed={v.duplicateRatio === k} onClick={() => set({ duplicateRatio: k })}>{t(`cull.ratio.${k}`)}</button>)}
          </div>
          <span className="pref-hint">{t(`cull.ratio.${v.duplicateRatio}Hint`)}</span>
        </div>
        <div className="cull-field">
          <label htmlFor="cull-highlights">{t("cull.dialog.highlights")}</label>
          <input id="cull-highlights" className="field" type="number" min={0} max={100} step={1} value={v.highlightPercent}
                 onChange={(e) => set({ highlightPercent: clampPercent(num(e.target.value, 15)) })} />
        </div>
        <div className="cull-field">
          <label htmlFor="cull-blur">{t("cull.dialog.blur")}</label>
          <input id="cull-blur" className="field" type="number" min={0} max={10000} step={5} value={v.blurThreshold}
                 onChange={(e) => set({ blurThreshold: Math.min(10000, Math.max(0, num(e.target.value, 100))) })} />
          <span className="pref-hint">{t("cull.dialog.blurHint")}</span>
        </div>
        <details className="cull-advanced">
          <summary>{t("cull.dialog.advanced")}</summary>
          <div className="cull-field">
            <label htmlFor="cull-similarity">{t("cull.dialog.similarity")}</label>
            <input id="cull-similarity" className="field" type="number" min={0} max={512} step={1} value={v.similarityThreshold}
                   onChange={(e) => set({ similarityThreshold: Math.min(512, Math.max(0, Math.round(num(e.target.value, 28)))) })} />
            <span className="pref-hint">{t("cull.dialog.similarityHint")}</span>
          </div>
        </details>
        {facesOn && <div className="cull-field cull-faces">
          <label className="cull-check"><input type="checkbox" checked={v.useFaces} onChange={(e) => set({ useFaces: e.target.checked })} />{t("cull.dialog.useFaces")}</label>
          <label className="cull-check"><input type="checkbox" checked={v.eyesRank} disabled={!v.useFaces} onChange={(e) => set({ eyesRank: e.target.checked })} />{t("cull.dialog.eyesRank")}</label>
          <span id="cull-eyes-label">{t("cull.dialog.eyes")}</span>
          <div className="segmented small" role="group" aria-labelledby="cull-eyes-label">
            {(["mainSubject", "anyFace"] as const).map((k) => <button key={k} type="button" className={v.eyeMode === k ? "active" : ""} disabled={!v.useFaces || !v.eyesRank}
              aria-pressed={v.eyeMode === k} onClick={() => set({ eyeMode: k })}>{t(`cull.eyes.mode.${k}`)}</button>)}
          </div>
          <span className="pref-hint">{t("cull.dialog.eyesHint")}{eyes || !v.eyesRank ? "" : ` ${t("cull.dialog.eyesNone")}`}</span>
        </div>}
        <p className="note-plain cull-ai-note" role="note">
          {aiReady ? t("cull.dialog.aiReady") : <>{t("cull.dialog.aiOff")} <button type="button" className="link-button" onClick={onOpenPrefs}>{t("cull.dialog.aiSetUp")}</button></>}
        </p>
      </div>
      <div className="modal-foot">
        <button className="btn" onClick={onClose}>{t("dialogs.common.cancel")}</button>
        <button className="btn primary" disabled={count === 0} onClick={run}>{t("cull.dialog.run")}</button>
      </div>
    </div>
  </div>;
}
