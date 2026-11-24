// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Library column's People card (notes/faces-ux-spec.md 1.4, 3.3): the open photo's faces as chips (a crop and a
// name, or "Unnamed face"), the overlay switch and one status line. A card like the others: shut until opened, its
// body scrolls inside the card under a fixed header, and it exists only while People is on. A chip selects its face
// in the overlay (and turns the overlay on).
import { ScanFace } from "lucide-react";
import { headerKey } from "./CatalogCard";
import { faceTitle } from "./faceOverlayModel";
import { Crop } from "./PeopleGrid";
import { formatNumber, t } from "./i18n";
import { usePeople } from "./peopleStore";
import { usePhotoFaces } from "./usePhotoFaces";

export function PeopleCard({ path, open, onToggle, overlay, onOverlay, selected, onSelect }: {
  path: string | null; open: boolean; onToggle: (alt: boolean) => void;
  overlay: boolean; onOverlay: (on: boolean) => void; selected: number | null; onSelect: (face: number | null) => void;
}) {
  const { faces, loading } = usePhotoFaces(path, open || overlay);
  const scan = usePeople().scan;
  const status = scan.kind === "scanning" ? t("people.card.scanning", { done: formatNumber(scan.done) }) : t("people.card.upToDate");
  const count = faces.length;
  return (
    <div className="panel faces-panel" data-card="people">
      <div className={`panel-title${open ? " open" : ""}`} onClick={(e) => onToggle(e.altKey)} role="button" tabIndex={0} aria-expanded={open}
           onKeyDown={(e) => headerKey(e, onToggle)}>
        <span>{t("people.card.title")}</span>
        <span className="faces-head-note">{path ? t("people.card.count", { n: count, count: formatNumber(count) }) : ""}</span>
        <svg className={`chevron${open ? " open" : ""}`} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" aria-hidden="true"><path d="m6 9 6 6 6-6" /></svg>
      </div>
      {open && (
        <div className="faces-body">
          <div className="faces-chips" role="list" aria-label={t("people.card.inPhoto")} aria-busy={loading}>
            {faces.map((f) => (
              <button key={f.face} type="button" role="listitem" className={`faces-chip${selected === f.face ? " selected" : ""}`} data-state={f.state}
                      aria-pressed={selected === f.face} title={faceTitle(f, t)} onClick={() => { onSelect(f.face); onOverlay(true); }}>
                <Crop face={f.face} tile={24} className="faces-crop" />
                <span>{faceTitle(f, t)}</span>
              </button>
            ))}
            {!count && <span className="faces-empty">{!path ? t("people.card.noPhoto") : loading ? t("people.state.loading") : t("people.card.none")}</span>}
          </div>
          <label className="faces-switch">
            <ScanFace size={14} aria-hidden="true" /><span>{t("people.card.show")}</span>
            <input type="checkbox" className="switch" checked={overlay} onChange={(e) => onOverlay(e.target.checked)} aria-label={t("people.card.show")} />
          </label>
          <div className="faces-status" role="status">{status}</div>
        </div>
      )}
    </div>
  );
}
