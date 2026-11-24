// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The quiet row at the top of To review: pairs the analysis thinks are one person, strongest first. It is one fixed
// row (36 px, always laid out in that section), so a suggestion arriving or leaving moves nothing; "Show N more"
// opens the rest in a list that floats over the grid. A suggestion is never acted on by itself: Merge, Add to and
// Not them are buttons, Enter on the row does the first, Escape dismisses it for this session.
import { X } from "lucide-react";
import { useState } from "react";
import type { PersonRow } from "./bindings";
import { formatNumber, t } from "./i18n";
import { capRows, type SuggestRow } from "./peopleModel";

export type SuggestAct = "merge" | "addTo" | "notThem";

export function PeopleSuggest({ rows, people, sizes, onAct, onDismiss }: {
  rows: SuggestRow[]; people: readonly PersonRow[]; sizes: ReadonlyMap<number, number>;
  onAct: (r: SuggestRow, a: SuggestAct) => void; onDismiss: (r: SuggestRow) => void;
}) {
  const [open, setOpen] = useState(false);
  const name = (id: number) => people.find((p) => p.id === id)?.name ?? "";
  const rest = capRows(rows.slice(1), 8);
  const line = (r: SuggestRow) => {
    const sa = sizes.get(r.a) ?? 0, sb = sizes.get(r.b) ?? 0;
    return r.kind === "groups" ? t("people.suggest.groups", { a: formatNumber(sa), b: formatNumber(sb) })
      : r.kind === "group_person" ? t("people.suggest.groupPerson", { name: name(r.b), n: sa, count: formatNumber(sa) })
      : t("people.suggest.people", { a: name(r.a), b: name(r.b) });
  };
  const buttons = (r: SuggestRow) => r.kind === "group_person" ? (
    <>
      <button type="button" className="people-btn" onClick={() => onAct(r, "addTo")}>{t("people.suggest.addTo", { name: name(r.b) })}</button>
      <button type="button" className="people-btn" onClick={() => onAct(r, "notThem")}>{t("people.act.notThem")}</button>
    </>
  ) : <button type="button" className="people-btn" onClick={() => onAct(r, "merge")}>{t("people.suggest.merge")}</button>;
  const top = rows[0];
  const keyDown = (e: React.KeyboardEvent) => {
    if (!top || e.target !== e.currentTarget) return;
    if (e.key === "Enter") { e.preventDefault(); onAct(top, top.kind === "group_person" ? "addTo" : "merge"); }
    else if (e.key === "Escape") { e.preventDefault(); e.stopPropagation(); onDismiss(top); }
  };
  return (
    <div className="people-suggest" role="group" tabIndex={top ? 0 : -1} aria-label={t("people.suggest.label")} onKeyDown={keyDown}>
      {top && <>
        <span className="people-suggest-text" title={line(top)} data-confidence={top.confidence}>{line(top)}</span>
        {buttons(top)}
        <button type="button" className="people-btn" aria-label={t("people.suggest.dismiss")} title={t("people.suggest.dismiss")} onClick={() => onDismiss(top)}><X size={12} aria-hidden="true" /></button>
        {rest.shown.length > 0 && <button type="button" className="people-link" aria-expanded={open} onClick={() => setOpen(!open)}>
          {t("people.suggest.more", { n: rows.length - 1, count: formatNumber(rows.length - 1) })}</button>}
      </>}
      {open && top && rest.shown.length > 0 && (
        <ul className="people-suggest-list" aria-label={t("people.suggest.label")}>
          {rest.shown.map((r) => (
            <li key={r.key}>
              <span className="people-suggest-text" title={line(r)}>{line(r)}</span>
              {buttons(r)}
              <button type="button" className="people-btn" aria-label={t("people.suggest.dismiss")} onClick={() => onDismiss(r)}><X size={12} aria-hidden="true" /></button>
            </li>
          ))}
          {rest.more > 0 && <li><span className="people-suggest-text">{t("people.suggest.moreLeft", { n: rest.more, count: formatNumber(rest.more) })}</span></li>}
        </ul>
      )}
    </div>
  );
}
