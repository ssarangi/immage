// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { t } from "../i18n";
import { useThumb } from "../statsThumbs";
import { nf, richParts, stars } from "../statsFormat";
import type { PhotoRef, StatsResponse } from "../statsTypes";
import { Bold } from "./Section";

function Fig({ photo, className = "" }: { photo: PhotoRef | null; className?: string }) {
  const src = useThumb(photo);
  return (
    <figure className={className}>
      {src && <img src={src} alt="" draggable={false} />}
      {photo && photo.rating > 0 && src && <span className="st-pill amber st-star">{stars(photo.rating)}</span>}
    </figure>
  );
}

/** Up to three of the best frames from the gear rows: the page's picture of what the numbers are about. */
export function heroPhotos(r: StatsResponse | null): (PhotoRef | null)[] {
  const out: PhotoRef[] = [];
  const seen = new Set<string>();
  for (const p of [...(r?.cameras?.bodies ?? []).map((b) => b.cover), ...(r?.lenses?.lenses ?? []).map((l) => l.cover)]) {
    if (p && !seen.has(p.path)) { seen.add(p.path); out.push(p); }
    if (out.length === 3) break;
  }
  return [out[0] ?? null, out[1] ?? null, out[2] ?? null];
}

/** The page's headline, a sentence about the library, and a mosaic. Its height does not depend on the data. */
export function StatsHero({ response, loading, firstRun }: { response: StatsResponse | null; loading: boolean; firstRun?: boolean }) {
  const o = response?.overview;
  const first = o?.firstYear, last = o?.lastYear;
  const years = first != null && last != null ? Math.max(1, last - first + 1) : 0;
  const body = o && !firstRun
    ? t("stats.hero.body", { photos: t("stats.hero.nPhotos", { n: o.photos, count: nf(o.photos) }), folders: t("stats.hero.nFolders", { n: o.folders, count: nf(o.folders) }),
        bodies: t("stats.hero.nBodies", { n: o.gear.bodies, count: nf(o.gear.bodies) }), lenses: t("stats.hero.nLenses", { n: o.gear.lenses, count: nf(o.gear.lenses) }),
        years: t("stats.hero.nYears", { n: Math.max(years, 1), count: nf(Math.max(years, 1)) }), top: t("stats.hero.nTop", { n: o.matches.top, count: nf(o.matches.top) }) })
    : "";
  const photos = heroPhotos(response);
  return (
    <header className="st-hero" id="st-top">
      <div className="st-hero-text">
        <div className="st-eyebrow">{firstRun || !o ? t("stats.hero.eyebrowEmpty") : t("stats.hero.eyebrow", { n: o.photos, count: nf(o.photos) })}</div>
        <h1>{t("stats.hero.title")} <em>{t("stats.hero.titleEm")}</em></h1>
        <p className="st-hero-body">
          {firstRun ? t("stats.hero.bodyEmpty")
            : loading || !o ? <><i className="st-sk st-sk-line" /><i className="st-sk st-sk-line" /><i className="st-sk st-sk-line" /><i className="st-sk st-sk-line short" /></>
            : <Bold parts={richParts(body)} />}
        </p>
      </div>
      <div className="st-mosaic" aria-hidden={photos.every((p) => !p)}>
        <Fig photo={photos[0]} /><Fig photo={photos[1]} /><Fig photo={photos[2]} />
      </div>
    </header>
  );
}
