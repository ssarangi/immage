// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Cull panel: the review of an auto cull, in Find's place in the grid's
// right-hand column. The chips filter the grid (App applies reviewOrder), the
// counts and reasons say what the cull did, and Apply writes it to the
// library (flags only; Undo puts it back). The photos themselves carry a
// badge (Filmstrip.tsx) and the lightbox the inspector (CullInspector.tsx).
import { useEffect, useState } from "react";
import { commands, type CullApplyResult, type CullChange, type CullSession } from "./bindings";
import { loadCullDefaults } from "./cullDefaults";
import { CHIPS, chipCounts, rejectionReasons, type CullChip } from "./cullModel";
import { setCull, showPlan, useCull } from "./cullStore";
import { LABEL_COLORS } from "./library";
import { X } from "lucide-react";
import { formatDate, formatNumber, t } from "./i18n";

const LABEL_KEYS = ["library.culling.red", "library.culling.yellow", "library.culling.green", "library.culling.blue", "library.culling.purple"];

export function CullPanel({ onChanges, onClose, onFresh, pin }: {
  onChanges: (changes: CullChange[]) => void;   // library entries Apply or Undo rewrote
  onClose: () => void;                          // back to Find
  onFresh: () => void;                          // start another cull
  pin?: React.ReactNode;
}) {
  const { plan, chip } = useCull();
  const [sessions, setSessions] = useState<CullSession[]>([]);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<{ text: string; error?: boolean; skipped?: string[] } | null>(null);
  const [rating, setRating] = useState(() => loadCullDefaults().highlightRating);
  const [label, setLabel] = useState(() => loadCullDefaults().highlightLabel);
  const session = plan?.session;
  const list = plan?.decisions ?? [];
  const counts = chipCounts(list);
  const reasons = rejectionReasons(list);

  const refresh = () => commands.cullSessions().then(setSessions).catch(() => {});
  useEffect(() => { void refresh(); }, [session?.id, session?.state]);

  async function load(id: number) {
    try {
      const next = await commands.cullSession(id);
      showPlan(next);
      setNote(null);
    } catch (e) { setNote({ text: String(e), error: true }); }
  }
  async function reload() { if (session) showPlan(await commands.cullSession(session.id)); }
  async function finish(r: CullApplyResult, text: string) {
    onChanges(r.changes);
    setNote({ text, skipped: r.skipped.map((s) => `${s.path.split(/[\\/]/).pop()}: ${s.error}`) });
    await reload();
  }
  async function apply() {
    if (!session || busy) return;
    setBusy(true); setNote(null);
    try {
      const r = await commands.cullApply(session.id, { reject: true, highlightRating: rating || null, highlightLabel: label || null });
      await finish(r, t("cull.panel.applied", { rejected: formatNumber(r.rejected), highlighted: formatNumber(r.highlighted) }));
    } catch (e) { setNote({ text: t("cull.panel.failed", { error: String(e) }), error: true }); }
    finally { setBusy(false); }
  }
  async function undo() {
    if (!session || busy) return;
    setBusy(true); setNote(null);
    try {
      const r = await commands.cullUndo(session.id);
      await finish(r, t("cull.panel.undone", { n: formatNumber(r.changes.length) }));
    } catch (e) { setNote({ text: t("cull.panel.failed", { error: String(e) }), error: true }); }
    finally { setBusy(false); }
  }
  async function remove() {
    if (!session || busy) return;
    try {
      await commands.cullDeleteSession(session.id);
      setNote({ text: t("cull.panel.deleted") });
      showPlan(null); setCull({ open: true });
      await refresh();
    } catch (e) { setNote({ text: String(e), error: true }); }
  }

  const when = (s: CullSession) => formatDate(new Date(s.createdAt * 1000), { dateStyle: "medium", timeStyle: "short" });
  const applied = session?.state === "applied";
  return (
    <div className="find-pop cull-panel" role="region" aria-label={t("cull.panel.title")}>
      <div className="panel-head"><h1>{t("cull.panel.title")}</h1>
        <div className="head-tools">
          {pin}
          <button type="button" className="tool" onClick={onClose} aria-label={t("cull.panel.close")} title={t("cull.panel.close")}><X size={14} aria-hidden="true" /></button>
        </div></div>
      <div className="find-scroll cull-scroll">
        <div className="cull-block">
          <label className="cull-label" htmlFor="cull-session">{t("cull.panel.session")}</label>
          <div className="cull-session-row">
            <select id="cull-session" className="field" value={session?.id ?? ""} onChange={(e) => { if (e.target.value) void load(Number(e.target.value)); }}>
              {!session && <option value="">{t("cull.panel.pick")}</option>}
              {sessions.map((s) => <option key={s.id} value={s.id}>
                {t("cull.panel.sessionName", { name: s.scopeLabel, when: when(s), state: t(`cull.state.${s.state}`) })}
              </option>)}
            </select>
            <button type="button" className="btn small" disabled={!session || busy} onClick={() => void remove()}
                    title={t("cull.panel.deleteHint")}>{t("cull.panel.delete")}</button>
          </div>
          <button type="button" className="btn small" onClick={onFresh}>{t("cull.panel.new")}</button>
        </div>
        {session ? <>
          <div className="cull-block">
            <p className="cull-summary">{t("cull.panel.summary", {
              total: formatNumber(session.totalCount), keep: formatNumber(session.keepCount),
              reject: formatNumber(session.rejectCount), groups: formatNumber(session.groupCount) })}</p>
            <div className="cull-chips" role="group" aria-label={t("cull.panel.show")}>
              {CHIPS.map((c) => <button key={c} type="button" className={`cull-chip${chip === c ? " active" : ""}`} aria-pressed={chip === c}
                onClick={() => setCull({ chip: c as CullChip })}>{t(`cull.chip.${c}`)} <span className="cull-count">{formatNumber(counts[c])}</span></button>)}
            </div>
          </div>
          {reasons.length > 0 && <div className="cull-block">
            <h2 className="cull-h">{t("cull.panel.reasons")}</h2>
            <ul className="cull-reasons">{reasons.map((r) => <li key={r.reason}><span>{t(`cull.reason.${r.reason}`)}</span><span className="cull-count">{formatNumber(r.count)}</span></li>)}</ul>
          </div>}
          {session.failed.length > 0 && <div className="cull-block"><p className="note-plain warn" role="note">{t("cull.panel.failedPhotos", { n: formatNumber(session.failed.length) })}</p></div>}
          <div className="cull-block">
            <h2 className="cull-h">{t("cull.panel.applyHeading")}</h2>
            <p className="note-plain">{t("cull.panel.applyNote")}</p>
            <div className="cull-session-row">
              <label htmlFor="cull-rating">{t("cull.panel.rating")}</label>
              <select id="cull-rating" className="field" value={rating} disabled={applied || busy} onChange={(e) => setRating(Number(e.target.value))}>
                <option value={0}>{t("cull.apply.noRating")}</option>
                {[1, 2, 3, 4, 5].map((n) => <option key={n} value={n}>{t("library.culling.stars", { count: n })}</option>)}
              </select>
            </div>
            <div className="cull-session-row">
              <label htmlFor="cull-label">{t("cull.panel.label")}</label>
              <select id="cull-label" className="field" value={label} disabled={applied || busy} onChange={(e) => setLabel(Number(e.target.value))}>
                <option value={0}>{t("cull.apply.noLabel")}</option>
                {LABEL_COLORS.map((c, n) => <option key={c} value={1 << n}>{t(LABEL_KEYS[n])}</option>)}
              </select>
            </div>
            <div className="cull-actions">
              <button type="button" className="btn primary" disabled={busy || session.state !== "planned"} onClick={() => void apply()}>{t("cull.panel.apply")}</button>
              <button type="button" className="btn" disabled={busy || !applied} onClick={() => void undo()}>{t("cull.panel.undo")}</button>
            </div>
          </div>
        </> : <p className="note-plain cull-empty">{t("cull.panel.empty")}</p>}
        {note && <div className="cull-block" role="status"><p className={`note-plain${note.error ? " warn" : ""}`}>{note.text}</p>
          {note.skipped && note.skipped.length > 0 && <details><summary>{t("cull.panel.skipped", { n: formatNumber(note.skipped.length) })}</summary>
            <ul className="cull-reasons">{note.skipped.map((s, i) => <li key={i}>{s}</li>)}</ul></details>}
        </div>}
        {session && <p className="note-plain cull-keys">{t("cull.panel.keys")}</p>}
      </div>
    </div>
  );
}
