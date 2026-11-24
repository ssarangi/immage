// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The head of a person's page (notes/faces-ux-spec.md 3.1): a breadcrumb back to People, the cover, the name (edited in
// place), what is known about them, the actions (Open in Grid, Rename, Merge, Ignore, Cover) and the Photos / Faces
// tabs. A fixed block of 124 px, so switching tabs or a count changing moves nothing under it.
import { ChevronLeft } from "lucide-react";
import { useState } from "react";
import type { PersonRow } from "./bindings";
import { formatNumber, t } from "./i18n";
import { Crop } from "./PeopleGrid";

export type PageTab = "photos" | "faces";

export function PersonHeader({ person, tab, review, onTab, onBack, onRename, onOpenGrid, onMerge, onIgnore, onCover }: {
  person: PersonRow; tab: PageTab; review: number; onTab: (t: PageTab) => void; onBack: () => void; onRename: (name: string) => void;
  onOpenGrid?: () => void; onMerge: () => void; onIgnore: () => void; onCover: () => void;
}) {
  const [editing, setEditing] = useState(false);
  const done = (v: string | null) => { setEditing(false); if (v && v.trim() && v.trim() !== person.name) onRename(v.trim()); };
  return (
    <div className="person-head">
      <nav className="person-crumb" aria-label={t("people.page.crumb")}>
        <button type="button" className="people-link" onClick={onBack}><ChevronLeft size={12} aria-hidden="true" />{t("people.section.people")}</button>
        <span aria-hidden="true">&gt;</span>
        <span aria-current="page">{person.name}</span>
      </nav>
      <div className="person-top">
        <Crop face={person.cover_face_id} tile={72} className="pcrop person-cover" />
        <div className="person-info">
          {editing
            ? <input className="person-name-input" autoFocus defaultValue={person.name} aria-label={t("people.rename.label")} spellCheck={false}
                     onFocus={(e) => e.currentTarget.select()}
                     onKeyDown={(e) => { e.stopPropagation(); if (e.key === "Enter") done(e.currentTarget.value); else if (e.key === "Escape") done(null); }}
                     onBlur={(e) => done(e.currentTarget.value)} />
            : <h2 className="person-name" title={person.name} onDoubleClick={() => setEditing(true)}>{person.name}</h2>}
          <p className="person-facts">{t("people.page.facts", { photos: formatNumber(person.photos), faces: formatNumber(person.faces), review: formatNumber(review) })}</p>
        </div>
        <div className="person-actions">
          <button type="button" className="people-btn" disabled={!onOpenGrid} onClick={onOpenGrid}>{t("people.page.openGrid")}</button>
          <button type="button" className="people-btn" onClick={() => setEditing(true)}>{t("people.act.rename")}</button>
          <button type="button" className="people-btn" onClick={onCover}>{t("people.act.cover")}</button>
          <button type="button" className="people-btn" onClick={onMerge}>{t("people.page.mergeInto")}</button>
          <button type="button" className="people-btn" onClick={onIgnore}>{t("people.act.ignore")}</button>
        </div>
      </div>
      <div className="person-tabs" role="tablist" aria-label={t("people.page.tabs")}>
        {(["photos", "faces"] as const).map((x) => (
          <button key={x} type="button" role="tab" id={`person-tab-${x}`} aria-selected={tab === x} tabIndex={tab === x ? 0 : -1} className="person-tab" onClick={() => onTab(x)}
                  onKeyDown={(e) => { if (e.key === "ArrowRight" || e.key === "ArrowLeft") { e.preventDefault(); onTab(x === "photos" ? "faces" : "photos"); } }}>
            {x === "photos" ? t("people.page.photos") : t("people.page.faces", { n: person.faces, count: formatNumber(person.faces) })}
          </button>
        ))}
      </div>
    </div>
  );
}
