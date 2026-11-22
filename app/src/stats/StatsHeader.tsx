// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { useEffect, useRef, useState } from "react";
import { t } from "../i18n";
import { LABEL_COLORS } from "../library";
import { clampRating, selectedSet, setMinRating, setScope, toggleValue, type StatsState } from "../statsModel";
import { starsAtLeast } from "../statsFormat";
import type { ColorLabel, StatsScope } from "../statsTypes";
import { Seg } from "./Controls";

const LABELS: ColorLabel[] = ["red", "yellow", "green", "blue", "purple"];
type ScopeKind = StatsScope["kind"];

const iso = (d: Date) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
function presets(now = new Date()): { id: string; from: string; to: string }[] {
  const y = now.getFullYear();
  const back = new Date(now); back.setFullYear(y - 1);
  return [
    { id: "thisYear", from: `${y}-01-01`, to: iso(now) },
    { id: "lastYear", from: `${y - 1}-01-01`, to: `${y - 1}-12-31` },
    { id: "last12", from: iso(back), to: iso(now) },
  ];
}

/** The header row: title, scope, the top-images threshold, label dots, Refresh and Done. */
export function StatsHeader({ state, update, folder, onRefresh, onClose, busy }: {
  state: StatsState; update(fn: (s: StatsState) => StatsState): void; folder: string | null; onRefresh(): void; onClose(): void; busy: boolean;
}) {
  const [range, setRange] = useState(false);
  const pop = useRef<HTMLDivElement>(null);
  const kind = state.scope.kind;
  const [from, setFrom] = useState(state.scope.kind === "dateRange" ? state.scope.from : presets()[2].from);
  const [to, setTo] = useState(state.scope.kind === "dateRange" ? state.scope.to : presets()[2].to);
  useEffect(() => {
    if (!range) return;
    const down = (e: PointerEvent) => { if (!pop.current?.contains(e.target as Node) && !(e.target as Element).closest("[data-range-btn]")) setRange(false); };
    const esc = (e: KeyboardEvent) => { if (e.key === "Escape") { e.stopPropagation(); setRange(false); } };
    window.addEventListener("pointerdown", down, true);
    window.addEventListener("keydown", esc, true);
    return () => { window.removeEventListener("pointerdown", down, true); window.removeEventListener("keydown", esc, true); };
  }, [range]);

  const pick = (k: ScopeKind) => {
    if (k === "library") { setRange(false); update((s) => setScope(s, { kind: "library" })); }
    else if (k === "folder") { setRange(false); if (folder) update((s) => setScope(s, { kind: "folder", folder, includeSubfolders: true })); }
    else setRange(true);
  };
  const rating = state.filter.minRating;
  const labels = selectedSet(state, "labels");
  const name = folder ? folder.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? folder : "";

  return (
    <div className="st-head">
      <h1>{t("stats.title")}</h1>
      <div className="st-scope">
        <Seg<ScopeKind> value={kind} label={t("stats.scope.label")} onChange={pick}
             options={[{ id: "library", label: t("stats.scope.library") }, { id: "folder", label: t("stats.scope.folder"), disabled: !folder },
                       { id: "dateRange", label: t("stats.scope.range") }]} />
        <button type="button" className="st-scope-range" data-range-btn hidden={kind !== "dateRange"} onClick={() => setRange(!range)} aria-expanded={range}>
          {state.scope.kind === "dateRange" ? `${state.scope.from} – ${state.scope.to}` : ""}
        </button>
        {kind === "folder" && <span className="st-scope-folder" title={state.scope.kind === "folder" ? state.scope.folder : ""}>{name}</span>}
        {range && (
          <div className="st-pop st-range-pop" ref={pop} role="dialog" aria-label={t("stats.scope.range")}>
            <div className="st-range-fields">
              <label>{t("stats.scope.from")}<input type="date" value={from} max={to} onChange={(e) => setFrom(e.target.value)} /></label>
              <label>{t("stats.scope.to")}<input type="date" value={to} min={from} onChange={(e) => setTo(e.target.value)} /></label>
            </div>
            <div className="st-range-presets">
              {presets().map((p) => <button key={p.id} type="button" className="st-opt" onClick={() => { setFrom(p.from); setTo(p.to); }}>{t(`stats.scope.${p.id}`)}</button>)}
            </div>
            <div className="st-range-actions">
              <button type="button" className="st-btn" onClick={() => setRange(false)}>{t("stats.cancel")}</button>
              <button type="button" className="st-btn primary" disabled={!from || !to || from > to}
                      onClick={() => { update((s) => setScope(s, { kind: "dateRange", from, to })); setRange(false); }}>{t("stats.apply")}</button>
            </div>
          </div>
        )}
      </div>

      <span className="st-field" title={t("stats.top.tip")}>
        {t("stats.top.label")}
        <span className="st-stepper">
          <button type="button" aria-label={t("stats.top.fewer")} disabled={rating <= 1} onClick={() => update((s) => setMinRating(s, clampRating(rating - 1)))}>{"−"}</button>
          <span aria-live="polite">{starsAtLeast(rating)}</span>
          <button type="button" aria-label={t("stats.top.more")} disabled={rating >= 5} onClick={() => update((s) => setMinRating(s, clampRating(rating + 1)))}>+</button>
        </span>
      </span>
      <span className="st-field">
        {t("stats.label.label")}
        <span className="st-dots" role="group" aria-label={t("stats.label.label")}>
          {LABELS.map((l, i) => (
            <button key={l} type="button" aria-pressed={labels.has(l)} aria-label={t(`stats.label.${l}`)} data-tip={t(`stats.label.${l}`)}
                    style={{ ["--c" as string]: LABEL_COLORS[i] }} onClick={() => update((s) => toggleValue(s, "labels", l, true))} />
          ))}
        </span>
      </span>
      <span className="st-grow" />
      <button type="button" className="st-btn" onClick={onRefresh} disabled={busy} aria-label={t("stats.refresh.tip")} title={t("stats.refresh.tip")}>{t("stats.refresh")}</button>
      <button type="button" className="st-btn primary" onClick={onClose}>{t("stats.done")}</button>
    </div>
  );
}
