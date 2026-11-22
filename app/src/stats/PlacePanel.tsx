// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Places side panel. Its boxes never change size: the details of the selected area (or a hint when none is),
// six photo slots, one button row and the quick-jump chips are always there, so selecting an area, or its
// numbers arriving, moves nothing. The chips are the biggest areas at the map's zoom, named by their position
// (place names need a geocoder, which Statistics does not call).
import { formatDate, t } from "../i18n";
import { nf, photosText, richParts, setupText, stars, starsAtLeast } from "../statsFormat";
import type { PhotoRef, PlaceCell, PlaceDetail } from "../statsTypes";
import { PhotoCard } from "./PhotoCard";
import { Bold } from "./Section";
import { compact, coords, degrees } from "./placesGeometry";

const SLOTS = 6;
const SHOWN_CHIPS = 12;

const day = (iso: string | null) => {
  const m = iso && /^(\d{4})-(\d{2})-(\d{2})/.exec(iso);
  return m ? new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])) : null;
};
const monthYear = (d: Date) => formatDate(d, { year: "numeric", month: "short" });

/** "Sony|ILCE-7M4" as "Sony ILCE-7M4"; a model that already starts with its maker is not said twice ("Canon EOS R5") */
export const bodyName = (key: string) => {
  const [maker, model = ""] = key.split("|");
  return model && model.toLowerCase().startsWith(maker.toLowerCase()) ? model : `${maker} ${model}`.trim();
};

/** "Oct 2023 – Dec 2024", or one month when it is the same */
export function rangeText(first: string | null, last: string | null): string {
  const a = day(first), b = day(last);
  if (!a || !b) return "";
  const [x, y] = [monthYear(a), monthYear(b)];
  return x === y ? x : `${x} – ${y}`;
}

/** The one-line summary: the usual body and lens, the usual focal length, and when. */
export function summaryText(d: PlaceDetail): string {
  const body = d.topBody ? bodyName(d.topBody.key) : null;
  const lens = d.topLens?.name ?? null;
  const gear = body && lens ? t("stats.places.sum.both", { body, lens })
    : body ? t("stats.places.sum.body", { body }) : lens ? t("stats.places.sum.lens", { lens }) : "";
  const focal = d.modalFocalMm ? t("stats.places.sum.focal", { mm: d.modalFocalMm }) : "";
  const sentence = [gear, focal].filter(Boolean).join(" ");
  return [sentence, rangeText(d.firstShot, d.lastShot)].filter(Boolean).join(". ") + (sentence || rangeText(d.firstShot, d.lastShot) ? "." : "");
}

export const chipLabel = (c: { lat: number; lng: number }) => `${degrees(c.lat, "lat", 0.1)} ${degrees(c.lng, "lng", 0.1)}`;
const radiusText = (km: number) => `${km < 10 ? Math.round(km * 10) / 10 : Math.round(km)}`;

export interface PlacePanelProps {
  detail: PlaceDetail | null;
  /** a key is selected and its numbers have not arrived */
  loading: boolean;
  error: string | null;
  selected: boolean;
  minRating: number;
  top: readonly PlaceCell[];
  activeKeys: readonly string[];
  /** photos with GPS and distinct places, for the hint */
  known: number | undefined;
  total: number | undefined;
  onJump(cell: PlaceCell): void;
  onClear(): void;
  onOpenGrid(): void;
  onOpenPhoto(photo: PhotoRef, list: PhotoRef[]): void;
  onRetry(): void;
}

export function PlacePanel(p: PlacePanelProps) {
  const d = p.detail;
  const show = p.selected;
  const shown = show && !p.loading && d;
  const photos = shown ? d.photos.slice(0, SLOTS) : [];
  return (
    <aside className="st-pl-panel" aria-label={t("stats.places.panel")}>
      <div className="st-pl-main">
        <div className="st-pl-head">
          <div className="st-eyebrow amber">{show ? t("stats.places.selected") : t("stats.places.areas")}</div>
          <button type="button" className="st-pl-x" onClick={p.onClear} disabled={!show} aria-label={t("stats.places.clear")} data-tip={show ? t("stats.places.clear") : undefined}>
            <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M4 4l8 8M12 4l-8 8" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" /></svg>
          </button>
        </div>
        {!show ? (
          <div className="st-pl-hint">
            <h3 className="st-pl-name">{t("stats.places.pickTitle")}</h3>
            <p>{t("stats.places.pick")}</p>
            <p className="st-pl-hintn">{p.known !== undefined && p.total !== undefined && <><b>{nf(p.known)}</b> {t("stats.places.known")} · <b>{t("stats.places.count", { n: p.total, count: nf(p.total) })}</b></>}</p>
          </div>
        ) : p.error && !d ? (
          <div className="st-pl-hint" role="alert">
            <h3 className="st-pl-name">{t("stats.places.errorTitle")}</h3>
            <p>{p.error}</p>
            <button type="button" className="st-btn" onClick={p.onRetry}>{t("stats.error.retry")}</button>
          </div>
        ) : (
          <>
            <h3 className="st-pl-name">{shown ? d.name ?? coords(d.lat, d.lng) : <i className="st-sk st-sk-title" />}</h3>
            <p className="st-pl-coords">{shown ? `${d.name ? `${coords(d.lat, d.lng)} · ` : ""}${t("stats.places.radius", { km: radiusText(d.radiusKm) })}` : <i className="st-sk st-sk-line short" />}</p>
            <div className="st-pl-tiles">
              <div><b>{shown ? nf(d.n) : <i className="st-sk st-sk-num" />}</b><span>{t("stats.places.k.photos")}</span></div>
              <div><b className="c-amber">{shown ? nf(d.top) : <i className="st-sk st-sk-num" />}</b><span>{starsAtLeast(p.minRating)}</span></div>
              <div><b>{shown ? radiusText(d.radiusKm) : <i className="st-sk st-sk-num" />}</b><span>{t("stats.places.k.km")}</span></div>
            </div>
            <p className="st-pl-sum">{shown ? <Bold parts={richParts(summaryText(d))} /> : <><i className="st-sk st-sk-line" /><i className="st-sk st-sk-line short" /></>}</p>
            <div className="st-pl-photos">
              {Array.from({ length: SLOTS }, (_, i) => photos[i]
                ? <PhotoCard key={photos[i].id + i} photo={photos[i]} pill={stars(photos[i].rating)} pillTone="amber" title={setupText(photos[i], false)} meta="" className="mini"
                             openLabel={t("stats.open.photo", { name: setupText(photos[i], false) || photos[i].path.split("/").pop() || "" })}
                             onOpen={() => p.onOpenPhoto(photos[i], photos)} />
                : <span key={`s${i}`} className={`st-pl-slot${show && !shown ? " st-sk" : ""}`} aria-hidden="true" />)}
            </div>
            <div className="st-pl-act">
              <button type="button" className="st-btn primary" disabled={!shown} onClick={p.onOpenGrid}>{t("stats.openGrid", { count: shown ? nf(d.n) : "…" })}</button>
            </div>
          </>
        )}
      </div>
      <div className="st-pl-jump" role="group" aria-label={t("stats.places.jump")}>
        {p.top.slice(0, SHOWN_CHIPS).map((c) => (
          <button key={c.key} type="button" className="st-pl-chip" aria-pressed={p.activeKeys.includes(c.key)} onClick={() => p.onJump(c)}
                  aria-label={t("stats.places.jumpTo", { where: coords(c.lat, c.lng), photos: photosText(c.n) })} data-tip={coords(c.lat, c.lng)} data-tip-sub={photosText(c.n)}>
            {c.name ?? chipLabel(c)} <span>{compact(c.n)}</span>
          </button>
        ))}
      </div>
    </aside>
  );
}
