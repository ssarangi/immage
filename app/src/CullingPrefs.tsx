// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { useEffect, useState } from "react";
import { commands } from "./commands";
import { PrefCard, PrefRow, PrefSwitch } from "./PrefsUI";
import { loadAiGrading, storeAiGrading } from "./aiGrading";
import { loadCullDefaults, storeCullDefaults } from "./cullDefaults";
import { DEFAULT_SETTINGS, RATIOS, SHOOT_TYPES, type CullDefaults } from "./cullModel";
import { LABEL_COLORS } from "./library";
import { t } from "./i18n";

const LABEL_KEYS = ["library.culling.red", "library.culling.yellow", "library.culling.green", "library.culling.blue", "library.culling.purple"];

// Preferences > Culling: AI grading with Google Gemini. The key field is
// write-only; once saved the page only learns that a key exists.
export function CullingPrefs({ onGoTo }: { onGoTo?: (tab: "general", card?: string) => void } = {}) {
  const [hasKey, setHasKey] = useState<boolean | null>(null);
  const [keychainError, setKeychainError] = useState(false);
  const [enabled, setEnabled] = useState(loadAiGrading);
  const [key, setKey] = useState("");
  const [replace, setReplace] = useState(false);
  const [busy, setBusy] = useState<"save" | "test" | "clear" | null>(null);
  const [defaults, setDefaults] = useState(loadCullDefaults);
  const [cacheBusy, setCacheBusy] = useState(false);
  const [message, setMessage] = useState<{ text: string; error?: boolean } | null>(null);
  const [cacheMessage, setCacheMessage] = useState<{ text: string; error?: boolean } | null>(null);

  async function refresh() {
    const status = await commands.geminiKeyStatus();
    setHasKey(status.hasKey);
    setKeychainError(status.keychainError);
  }
  useEffect(() => { void refresh().catch(() => setKeychainError(true)); }, []);

  async function run(action: NonNullable<typeof busy>) {
    setBusy(action);
    setMessage(null);
    try {
      if (action === "test") {
        await commands.geminiKeyTest(key.trim() || null);
        setMessage({ text: t("culling.testPassed") });
        return;
      }
      if (action === "save") await commands.geminiKeySet(key);
      else await commands.geminiKeyClear();
      setKey("");
      setReplace(false);
      await refresh();
      setMessage({ text: t(action === "save" ? "culling.saved" : "culling.cleared") });
    } catch (e) {
      // the command's message never holds the key
      setMessage({ text: t("culling.failed", { error: String(e) }), error: true });
    } finally {
      setBusy(null);
    }
  }

  function toggle(on: boolean) {
    setEnabled(on);
    storeAiGrading(on);
  }

  const set = (patch: Partial<CullDefaults>) => {
    const next = { ...defaults, ...patch };
    setDefaults(next);
    storeCullDefaults(next);
  };
  // a cleared or out-of-range number field goes back to its default
  const num = (v: string, d: number) => (v.trim() === "" || !Number.isFinite(Number(v)) ? d : Number(v));
  async function clearCache() {
    setCacheBusy(true);
    setCacheMessage(null);
    try {
      await commands.cullCacheClear();
      setCacheMessage({ text: t("culling.cache.cleared") });
    } catch (e) {
      setCacheMessage({ text: t("culling.failed", { error: String(e) }), error: true });
    } finally { setCacheBusy(false); }
  }

  const stored = hasKey === true && !replace;
  return <>
    <p className="prefs-lead">{t("culling.lead")}</p>
    <PrefCard title={t("culling.card.gemini")}>
      <PrefRow id="AI grading" label={t("culling.enable")} hint={t("culling.enable.hint")}>
        <PrefSwitch checked={enabled} onChange={toggle} aria-label={t("culling.enable")} />
      </PrefRow>
      <PrefRow id="Gemini API key" label={t("culling.key")}
               hint={keychainError ? <span className="warn">{t("culling.keychain")}</span> : t("culling.key.hint")}>
        <div className="pref-inline">
          {stored
            ? <><span className="pill ok">{t("culling.key.saved")}</span>
                <button className="btn small" disabled={!!busy} onClick={() => setReplace(true)}>{t("immich.key.replace")}</button></>
            : <input className="field key-field" type="password" autoComplete="new-password" spellCheck={false}
                     placeholder={t("culling.key.placeholder")} aria-label={t("culling.key")} disabled={!!busy} value={key}
                     onChange={(e) => { setKey(e.target.value); setMessage(null); }} />}
          {hasKey && <button className="btn small" disabled={!!busy} onClick={() => void run("clear")}>{t("immich.key.forget")}</button>}
        </div>
      </PrefRow>
      <PrefRow label={t("immich.row.connection")}>
        <div className="pref-inline">
          <button className="btn" disabled={!!busy || (!key.trim() && !hasKey)} onClick={() => void run("test")}>
            {busy === "test" ? t("immich.test.testing") : t("immich.test.run")}
          </button>
          {!stored && <button className="btn" disabled={!!busy || !key.trim()} onClick={() => void run("save")}>
            {t("morfphoto.save")}
          </button>}
        </div>
      </PrefRow>
      {message && <div role="status" className={`prefs-status ${message.error ? "warn" : "ok"}`}>{message.text}</div>}
      <p className="pref-hint culling-ai-note">{t("culling.aiNote")}{" "}
        <button type="button" className="pack-link" onClick={() => onGoTo?.("general", "ai-models")}>{t("culling.aiNoteLink")}</button></p>
    </PrefCard>
    <PrefCard title={t("culling.card.defaults")}>
      <PrefRow label={t("culling.def.shoot")} hint={t("culling.def.hint")}>
        <select className="field" value={defaults.shootType} aria-label={t("culling.def.shoot")}
                onChange={(e) => set({ shootType: e.target.value })}>
          {SHOOT_TYPES.map((k) => <option key={k} value={k}>{t(`cull.shoot.${k}`)}</option>)}
        </select>
      </PrefRow>
      <PrefRow label={t("culling.def.ratio")} hint={t("culling.def.ratioHint")}>
        <select className="field" value={defaults.duplicateRatio} aria-label={t("culling.def.ratio")}
                onChange={(e) => set({ duplicateRatio: e.target.value as CullDefaults["duplicateRatio"] })}>
          {RATIOS.map((k) => <option key={k} value={k}>{t(`cull.ratio.${k}`)}</option>)}
        </select>
      </PrefRow>
      <PrefRow label={t("culling.def.highlights")} hint={t("culling.def.highlightsHint")}>
        <input className="field" type="number" min={0} max={100} step={1} value={defaults.highlightPercent}
               aria-label={t("culling.def.highlights")}
               onChange={(e) => set({ highlightPercent: Math.min(100, Math.max(0, Math.round(num(e.target.value, DEFAULT_SETTINGS.highlightPercent)))) })} />
      </PrefRow>
      <PrefRow label={t("culling.def.blur")} hint={t("culling.def.blurHint")}>
        <input className="field" type="number" min={0} max={10000} step={5} value={defaults.blurThreshold}
               aria-label={t("culling.def.blur")}
               onChange={(e) => set({ blurThreshold: Math.min(10000, Math.max(0, num(e.target.value, DEFAULT_SETTINGS.blurThreshold))) })} />
      </PrefRow>
      <PrefRow label={t("culling.def.similarity")} hint={t("culling.def.similarityHint")}>
        <input className="field" type="number" min={0} max={512} step={1} value={defaults.similarityThreshold}
               aria-label={t("culling.def.similarity")}
               onChange={(e) => set({ similarityThreshold: Math.min(512, Math.max(0, Math.round(num(e.target.value, DEFAULT_SETTINGS.similarityThreshold)))) })} />
      </PrefRow>
      <PrefRow label={t("culling.def.reset")}>
        <button className="btn small" onClick={() => set({ ...DEFAULT_SETTINGS, highlightRating: defaults.highlightRating, highlightLabel: defaults.highlightLabel })}>
          {t("culling.def.resetButton")}
        </button>
      </PrefRow>
    </PrefCard>
    <PrefCard title={t("culling.card.apply")}>
      <PrefRow label={t("culling.apply.rating")} hint={t("culling.apply.ratingHint")}>
        <select className="field" value={defaults.highlightRating} aria-label={t("culling.apply.rating")}
                onChange={(e) => set({ highlightRating: Number(e.target.value) })}>
          <option value={0}>{t("cull.apply.noRating")}</option>
          {[1, 2, 3, 4, 5].map((n) => <option key={n} value={n}>{t("library.culling.stars", { count: n })}</option>)}
        </select>
      </PrefRow>
      <PrefRow label={t("culling.apply.label")} hint={t("culling.apply.labelHint")}>
        <select className="field" value={defaults.highlightLabel} aria-label={t("culling.apply.label")}
                onChange={(e) => set({ highlightLabel: Number(e.target.value) })}>
          <option value={0}>{t("cull.apply.noLabel")}</option>
          {LABEL_COLORS.map((c, n) => <option key={c} value={1 << n}>{t(LABEL_KEYS[n])}</option>)}
        </select>
      </PrefRow>
    </PrefCard>
    <PrefCard title={t("culling.card.cache")}>
      <PrefRow label={t("culling.cache.analysis")} hint={t("culling.cache.analysisHint")}>
        <button className="btn" disabled={cacheBusy} onClick={() => void clearCache()}>{t("culling.cache.clear")}</button>
      </PrefRow>
      {cacheMessage && <div role="status" className={`prefs-status ${cacheMessage.error ? "warn" : "ok"}`}>{cacheMessage.text}</div>}
      <PrefRow label={t("culling.model")} hint={t("culling.model.hint")}>
        <span className="pill">{t("culling.model.auto")}</span>
      </PrefRow>
    </PrefCard>
    <PrefCard title={t("culling.card.privacy")}>
      <p className="pref-hint">{t("culling.privacy.critique")}</p>
      <p className="pref-hint">{t("culling.privacy.search")}</p>
      <p className="pref-hint">{t("culling.privacy.models")}</p>
    </PrefCard>
  </>;
}
