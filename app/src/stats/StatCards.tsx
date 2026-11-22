// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { t } from "../i18n";
import { clearAll, setValues } from "../statsModel";
import { foldBins, share } from "../statsMath";
import { monthName, nf, pct, pct0, hourText, shutterText, starsAtLeast } from "../statsFormat";
import { FOCAL_BUCKETS, type FocalDef } from "../statsTypes";
import { MiniBars } from "./MiniBars";
import { StatCard } from "./StatCard";
import { useView } from "./ctx";

const bucketOf = (mm: number, defs: readonly FocalDef[] = FOCAL_BUCKETS) => defs.find((b) => mm >= b.fromMm && (b.toMm === null || mm <= b.toMm))?.id;

/** The six cards under the hero. Each is a button: it filters, or jumps to the section it comes from. */
export function StatCards() {
  const v = useView();
  const { state, update, response, loading } = v;
  const o = response?.overview;
  const ld = loading || !o;
  const threshold = starsAtLeast(state.filter.minRating);
  const defs = response?.defs.focal;
  const m = o?.medianSetup;
  const peak = o?.peak;
  const perYear = o?.perYear ?? [];
  const rd = o?.ratingDist;
  const bodies = response?.cameras?.bodies ?? [];
  const fav = o?.favouriteFocal;
  const years = o && o.firstYear != null && o.lastYear != null ? (o.firstYear === o.lastYear ? String(o.firstYear) : `${o.firstYear}–${o.lastYear}`) : "";
  const dash = "–";
  return (
    <div className="st-stats" role="group" aria-label={t("stats.cards")}>
      <StatCard k={t("stats.card.photos.k")} accent="cyan" loading={ld} value={o ? nf(o.photos) : ""}
                sub={o ? t("stats.card.photos.sub", { folders: t("stats.hero.nFolders", { n: o.folders, count: nf(o.folders) }), years }) : ""}
                mini={perYear.length ? <MiniBars values={perYear.map((y) => y.n)} accent="cyan" /> : null}
                title={t("stats.card.photos.tip")} onClick={() => update(clearAll)} />
      <StatCard k={t("stats.card.top.k")} accent="amber" loading={ld} value={o ? nf(o.matches.top) : ""}
                sub={o ? t("stats.card.top.sub", { rate: pct(share(o.matches.top, o.matches.n)), threshold, five: nf(rd?.[5] ?? 0) }) : ""}
                mini={rd ? <MiniBars values={rd.slice(1)} accent="amber" ramp /> : null}
                title={t("stats.card.top.tip", { threshold })} disabled={!v.has("top")} onClick={() => v.scrollTo("top")} />
      <StatCard k={t("stats.card.gear.k")} accent="green" loading={ld} value={o ? nf(o.gear.bodies) : ""} unit={o ? ` ${t("stats.card.gear.unit", { n: o.gear.bodies })}` : ""}
                sub={o ? t("stats.card.gear.sub", { lenses: t("stats.hero.nLenses", { n: o.gear.lenses, count: nf(o.gear.lenses) }), makers: t("stats.card.gear.nMakers", { n: o.gear.makers, count: nf(o.gear.makers) }) }) : ""}
                mini={bodies.length ? <MiniBars values={bodies.slice(0, 7).map((b) => b.n)} accent="green" /> : null}
                title={t("stats.card.gear.tip")} onClick={() => v.scrollTo("cameras")} />
      <StatCard k={t("stats.card.focal.k")} accent="violet" loading={ld} value={fav ? String(Math.round(fav.mm)) : dash} unit={fav ? " mm" : undefined}
                sub={fav ? t("stats.card.focal.sub", { share: pct0(fav.share), top: pct0(fav.topShare) }) : t("stats.card.none")}
                mini={response?.lenses?.focal.bins.length ? <MiniBars values={foldBins(response.lenses.focal.bins, 8)} accent="violet" /> : null}
                title={t("stats.card.focal.tip")} disabled={!fav}
                onClick={() => { const id = fav ? bucketOf(fav.mm, defs) : undefined; if (id) update((s) => setValues(s, "focalBuckets", [id])); }} />
      <StatCard k={t("stats.card.setup.k")} accent="rose" loading={ld} value={m?.fNumber != null ? `f/${Math.round(m.fNumber * 10) / 10}` : dash} unit={m?.shutterSec != null ? ` ${shutterText(m.shutterSec)}` : undefined}
                sub={m ? [m.iso != null ? `ISO ${Math.round(m.iso)}` : "", m.focalMm != null ? `${Math.round(m.focalMm)} mm` : ""].filter(Boolean).join(" · ") || t("stats.card.none") : ""}
                mini={response?.exposure ? <MiniBars values={response.exposure.aperture.map((a) => a.n)} accent="rose" /> : null}
                title={t("stats.card.setup.tip")} disabled={!v.has("exposure")} onClick={() => v.scrollTo("exposure")} />
      <StatCard k={t("stats.card.peak.k")} accent="cyan" loading={ld} value={peak?.hour != null ? hourText(peak.hour) : dash} unit={peak?.month != null ? ` ${monthName(peak.month, "short")}` : undefined}
                sub={peak?.month != null ? t("stats.card.peak.sub", { n: peak.monthN, count: nf(peak.monthN), month: monthName(peak.month) }) : t("stats.card.none")}
                mini={response?.time ? <MiniBars values={response.time.months.map((x) => x.n)} accent="cyan" /> : null}
                title={t("stats.card.peak.tip")} disabled={peak?.hour == null || peak?.month == null}
                onClick={() => { if (peak?.hour != null && peak.month != null) update((s) => setValues(setValues(s, "hours", [peak.hour!]), "months", [peak.month!])); }} />
    </div>
  );
}
