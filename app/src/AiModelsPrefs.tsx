// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Preferences > General > AI models: the optional model packs that run on this
// computer, how much they use, what is running, and each analysis's own switch.
// `AiModelsView` draws it from plain data (the dev harness renders it with
// fixtures); `AiModelsPrefs` connects it to the pack and analysis commands.
// Nothing here downloads or analyses without an explicit press: Download opens
// the consent dialog, and an analysis switch is its own consent.
import { useCallback, useEffect, useRef, useState } from "react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { commands } from "./commands";
import type { AnalysisId, AnalysisStatusAll, KeywordsSummary, PackId, PackInfo, RemoveReport, SearchStatus, TagCount } from "./bindings";
import { PackCard } from "./PackCard";
import { PackConsentDialog, PackRemoveDialog } from "./PackDialogs";
import { KeywordsReviewDialog, KeywordsRows } from "./KeywordsCard";
import { analysedLine, keywordsLine, packView, summaryOf, uiFmt, usePacks, workView, sizeLabel, type ActionId, type WorkInput } from "./aiPacks";
import { t } from "./i18n";
import "./aiModels.css";

export type Dialog = { kind: "consent" | "remove"; id: PackId } | { kind: "review" } | null;

export type ViewProps = {
  packs: PackInfo[];
  work: WorkInput | null;
  /** the Auto-keywords analysis (RAM++): its status, switch, results and confidence setting */
  keywordsWork: WorkInput | null;
  /** the analysis switches (stored) */
  smartOn: boolean;
  keywordsOn: boolean;
  keywords: KeywordsSummary | null;
  aiTags: TagCount[];
  flash: Partial<Record<PackId, string>>;
  busy: boolean;
  error: string;
  dialog: Dialog;
  onAction: (id: PackId, action: ActionId) => void;
  onSwitch: (id: AnalysisId, on: boolean) => void;
  onThreshold: (percent: number) => void;
  onReview: () => void;
  onRemoveTags: (names: string[]) => Promise<RemoveReport>;
  onPause: () => void;
  onShowFolder: () => void;
  onCulling: () => void;
  onConsent: (id: PackId) => void;
  onRemove: (id: PackId, keepResults: boolean) => void;
  onClose: () => void;
  license: (id: PackId) => Promise<string>;
  onTest?: () => void;
  testLine?: string;
};

const NAMES: Record<PackId, () => { name: string; does: string; photos: string; what: string; analysis: string; card: string }> = {
  clip: () => ({
    name: t("aimodels.clip.name"), does: t("aimodels.clip.does"), photos: t("aimodels.clip.photos"),
    what: t("aimodels.clip.what", { n: 7 }), analysis: t("aimodels.clip.analysisName"), card: "local-search",
  }),
  ram: () => ({
    name: t("aimodels.kw.name"), does: t("aimodels.kw.does"), photos: t("aimodels.kw.photos"),
    what: t("aimodels.kw.what", { n: 3 }), analysis: t("aimodels.kw.analysisName"), card: "ai-models/keywords",
  }),
};

export function AiModelsView(p: ViewProps) {
  const sum = summaryOf(p.packs);
  // the strip shows whichever analysis is busy; Smart search when neither is
  const kwBusy = !!p.keywordsWork && p.keywordsWork.enabled && p.keywordsWork.installed && (p.keywordsWork.running > 0 || p.keywordsWork.active || (p.keywordsWork.pending ?? 0) > 0);
  const stripWork = kwBusy ? { ...p.keywordsWork!, analysed: p.keywords?.photos ?? null } : p.work;
  const work = workView(stripWork, t, uiFmt);
  const running = stripWork?.enabled && stripWork.installed;
  const dialogPack = p.dialog && "id" in p.dialog ? p.packs.find((x) => x.id === (p.dialog as { id: PackId }).id) : undefined;
  return (
    <div className="ai-models" data-pref-card="ai-models">
      <div className="ai-head"><h3>{t("aimodels.title")}</h3></div>
      <p className="ai-lead">{t("aimodels.lead")} {t("aimodels.leadGemini")}{" "}
        <button type="button" className="pack-link" onClick={p.onCulling}>{t("aimodels.cullingLink")}</button>.</p>
      <div className="ai-sum" role="group" aria-label={t("aimodels.title")}>
        <div>
          <span className="k">{t("aimodels.sum.installed")}</span>
          <span className="v">{t("aimodels.sum.packs", { n: sum.total, done: sum.installed })}</span>
          <span className="s">{t("aimodels.sum.nothingElse")}</span>
        </div>
        <div>
          <span className="k">{t("aimodels.sum.disk")}</span>
          <span className="v">{sizeLabel(sum.bytes, uiFmt)}</span>
          <span className="s"><button type="button" className="pack-link" disabled={sum.bytes === 0} onClick={p.onShowFolder}>{t("aimodels.sum.showFolder")}</button></span>
        </div>
        <div>
          <span className="k">{t("aimodels.sum.work")}</span>
          <span className="v" role="status">{work.line}</span>
          <span className="bw">
            <span className="pack-bar" aria-hidden="true"><i style={{ width: `${Math.round((work.bar ?? 0) * 100)}%` }} /></span>
            <span className="who">{running ? (kwBusy ? t("aimodels.kw.short") : t("aimodels.clip.short")) : ""}</span>
            <button className="btn small" disabled={!stripWork || (!running && !stripWork.paused)} onClick={p.onPause}>{work.paused ? t("aimodels.resume") : t("aimodels.pause")}</button>
          </span>
        </div>
      </div>
      {p.packs.map((info) => {
        const copy = NAMES[info.id]();
        const view = packView(info, t, uiFmt, !!p.flash[info.id]);
        return <PackCard key={info.id} info={info} view={view} name={copy.name} does={copy.does} photos={copy.photos} card={copy.card}
          busy={p.busy} flash={p.flash[info.id] ?? null} onAction={(a) => p.onAction(info.id, a)}
          test={info.id === "clip" && info.state === "installed" && p.onTest ? <><button type="button" className="pack-link" onClick={p.onTest}>{t("aimodels.clip.test")}</button>{p.testLine ? ` ${p.testLine}` : ""}</> : null}
          analysis={info.id === "clip" ? {
            label: t("aimodels.clip.switch"), hint: t("aimodels.clip.switchHint"),
            checked: p.smartOn, reason: view.usable ? null : view.reason,
            onChange: (on) => p.onSwitch("smartSearch", on), statusLabel: t("aimodels.clip.statusLabel"),
            status: analysedLine(p.work?.analysed ?? null, p.work?.pending ?? null, p.smartOn && view.usable, t, uiFmt),
          } : {
            label: t("aimodels.kw.switch"), hint: t("aimodels.kw.switchHint"),
            checked: p.keywordsOn, reason: view.usable ? null : view.reason,
            onChange: (on) => p.onSwitch("keywords", on), statusLabel: t("aimodels.clip.statusLabel"),
            status: keywordsLine(p.keywords?.photos ?? null, p.keywordsWork?.pending ?? null, p.keywordsOn && view.usable, t, uiFmt),
            extra: <KeywordsRows summary={p.keywords} busy={p.busy} onThreshold={p.onThreshold} onReview={p.onReview} />,
          }} />;
      })}
      {p.error && <p role="alert" className="prefs-error">{p.error}</p>}
      {p.dialog?.kind === "consent" && dialogPack && <PackConsentDialog info={dialogPack} name={NAMES[dialogPack.id]().name} what={NAMES[dialogPack.id]().what}
        license={() => p.license(dialogPack.id)} onConfirm={() => p.onConsent(dialogPack.id)} onClose={p.onClose} />}
      {p.dialog?.kind === "review" && <KeywordsReviewDialog list={p.aiTags} keywords={p.keywords?.keywords ?? 0} photos={p.keywords?.keyworded ?? 0}
        onRemove={p.onRemoveTags} onClose={p.onClose} />}
      {(p.dialog?.kind === "remove") && dialogPack && <PackRemoveDialog info={dialogPack} name={NAMES[dialogPack.id]().name} analysis={NAMES[dialogPack.id]().analysis}
        onConfirm={(keep) => p.onRemove(dialogPack.id, keep)} onClose={p.onClose} />}
    </div>
  );
}

const FLASH_MS = 6000;

export function AiModelsPrefs({ onGoTo }: { onGoTo: (tab: "culling", card?: string) => void }) {
  const { packs, error: listError, refresh, setPacks } = usePacks();
  const [analysis, setAnalysis] = useState<AnalysisStatusAll | null>(null);
  const [search, setSearch] = useState<SearchStatus | null>(null);
  const [smartOn, setSmartOn] = useState(false);
  const [keywordsOn, setKeywordsOn] = useState(false);
  const [keywords, setKeywords] = useState<KeywordsSummary | null>(null);
  const [aiTags, setAiTags] = useState<TagCount[]>([]);
  const [dialog, setDialog] = useState<Dialog>(null);
  const [flash, setFlash] = useState<Partial<Record<PackId, string>>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [testLine, setTestLine] = useState("");
  const timers = useRef<ReturnType<typeof setTimeout>[]>([]);
  useEffect(() => () => timers.current.forEach(clearTimeout), []);

  useEffect(() => {
    let alive = true;
    let polling = false;
    const poll = async () => {
      if (polling) return;
      polling = true;
      try {
        const [a, s, k] = await Promise.all([commands.analysisStatus(), commands.searchStatus(), commands.keywordsSummary()]);
        if (alive) { setAnalysis(a); setSearch(s); setKeywords(k); }
      } catch { /* the next poll tries again */ }
      finally { polling = false; }
    };
    void poll();
    void commands.analysisGet().then((list) => {
      if (!alive) return;
      setSmartOn(!!list.find((x) => x.id === "smartSearch")?.enabled);
      setKeywordsOn(!!list.find((x) => x.id === "keywords")?.enabled);
    }).catch(() => {});
    const timer = setInterval(() => { if (document.visibilityState === "visible") void poll(); }, 2000);
    return () => { alive = false; clearInterval(timer); };
  }, []);

  const showFlash = (id: PackId, text: string) => {
    setFlash((f) => ({ ...f, [id]: text }));
    timers.current.push(setTimeout(() => setFlash((f) => { const { [id]: _gone, ...rest } = f; return rest; }), FLASH_MS));
  };
  const guard = useCallback(async (job: () => Promise<void>) => {
    setBusy(true); setError("");
    try { await job(); } catch (e) { setError(String(e)); } finally { setBusy(false); void refresh(); }
  }, [refresh]);

  const row = analysis?.rows.find((r) => r.id === "smartSearch");
  const work: WorkInput | null = row && analysis ? {
    paused: analysis.paused, enabled: row.enabled, installed: row.installed, pending: row.pending, running: row.running, active: row.active,
    analysed: search?.counts?.analysed_photos ?? null,
  } : null;

  const kwRow = analysis?.rows.find((r) => r.id === "keywords");
  const keywordsWork: WorkInput | null = kwRow && analysis ? {
    paused: analysis.paused, enabled: kwRow.enabled, installed: kwRow.installed, pending: kwRow.pending, running: kwRow.running, active: kwRow.active,
    analysed: keywords?.photos ?? null,
  } : null;

  const onAction = (id: PackId, action: ActionId) => {
    if (action === "download") setDialog({ kind: "consent", id });
    else if (action === "remove") setDialog({ kind: "remove", id });
    else if (action === "cancel") void commands.packCancel(id).catch((e) => setError(String(e)));
    else if (action === "resume" || action === "repair") void guard(async () => { setPacks((l) => l && l.map((p) => (p.id === id ? { ...p, state: "downloading", error: null } : p))); await commands.packDownload(id, false); });
    else if (action === "removePartial") void guard(async () => { await commands.packRemove(id, true, true); });
    else if (action === "check") void guard(async () => {
      const r = await commands.packVerify(id);
      showFlash(id, r.failed.length ? t("aimodels.check.bad", { bad: r.failed.length, n: r.files }) : t("aimodels.check.ok", { n: r.files }));
    });
  };
  return <AiModelsView packs={packs ?? []} work={work} keywordsWork={keywordsWork} smartOn={smartOn} keywordsOn={keywordsOn} keywords={keywords} aiTags={aiTags} flash={flash} busy={busy} error={error || listError} dialog={dialog}
    onAction={onAction}
    onSwitch={(id, on) => void guard(async () => {
      const list = await commands.analysisSet({ id, enabled: on });
      setSmartOn(!!list.find((x) => x.id === "smartSearch")?.enabled);
      setKeywordsOn(!!list.find((x) => x.id === "keywords")?.enabled);
    })}
    onThreshold={(percent) => void guard(async () => { await commands.keywordsThresholdSet(percent); setKeywords(await commands.keywordsSummary()); })}
    onReview={() => void guard(async () => { setAiTags(await commands.keywordsAiList(5000)); setDialog({ kind: "review" }); })}
    onRemoveTags={async (names) => {
      const report = await commands.keywordsAiRemove(names);
      setAiTags(await commands.keywordsAiList(5000)); setKeywords(await commands.keywordsSummary());
      return report;
    }}
    onPause={() => void guard(async () => { await commands.analysisPause(!analysis?.paused); setAnalysis(await commands.analysisStatus()); })}
    onShowFolder={() => { const folder = packs?.find((p) => p.bytes_on_disk > 0)?.folder; if (folder) void revealItemInDir(folder).catch(() => {}); }}
    onCulling={() => onGoTo("culling")}
    onConsent={(id) => { setDialog(null); void guard(async () => { await commands.packDownload(id, true); }); }}
    onRemove={(id, keep) => { setDialog(null); void guard(async () => { await commands.packRemove(id, keep, false); if (id === "clip") setSmartOn(false); else setKeywordsOn(false); }); }}
    onClose={() => setDialog(null)}
    license={(id) => commands.packLicense(id)}
    onTest={() => void guard(async () => {
      setTestLine("");
      const r = await commands.searchModelProbe();
      setTestLine(t("aimodels.clip.tested", { ms: Math.round(r.elapsed_ms ?? 0) }));
      timers.current.push(setTimeout(() => setTestLine(""), FLASH_MS));
    })}
    testLine={testLine} />;
}
