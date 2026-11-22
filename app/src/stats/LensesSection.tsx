// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { useMemo, useState } from "react";
import { t } from "../i18n";
import { barWidths, isSmallSample, keeperRate, lensFocalInsight, MIN_SAMPLE, rankRows, share, type LensMode } from "../statsMath";
import { clearAll, selectedSet, toggleValue } from "../statsModel";
import { focalName, focalRange, nf, pct, pct0, richParts, setupText, starsAtLeast, photosText } from "../statsFormat";
import { FOCAL_BUCKETS, type FocalBucketId } from "../statsTypes";
import { Seg } from "./Controls";
import { FovFan, type FanBucket } from "./FovFan";
import { PhotoCard } from "./PhotoCard";
import { RankedBars, type RankRow } from "./RankedBars";
import { Bold, ErrorCard, Insight, KnownFact, NoMatch, SectionHead } from "./Section";
import { useView } from "./ctx";

const LENS_ROWS = 8;
type Group = "exact" | "grouped";

export function LensesSection() {
  const v = useView();
  const { state, update, response, loading, error } = v;
  const lenses = response?.lenses;
  const overview = response?.overview;
  const defs = response?.defs;
  const [mode, setMode] = useState<LensMode>("most-used");
  const [group, setGroup] = useState<Group>("exact");
  const threshold = starsAtLeast(state.filter.minRating);
  const empty = !!lenses && lenses.lenses.length === 0 && lenses.focalByBucket.every((b) => b.n === 0);
  const selLenses = selectedSet(state, "lenses");
  const selFocal = selectedSet(state, "focalBuckets");

  const rows: RankRow[] = useMemo(() => {
    if (!lenses) return [];
    if (group === "grouped") {
      const src = lenses.focalByBucket.filter((b) => b.n > 0).map((b) => ({ ...b, name: `${focalName(b.id)} ${focalRange(b.id, defs)} mm` }));
      const ranked = rankRows(src, mode).slice(0, LENS_ROWS);
      const w = barWidths(ranked, mode);
      return ranked.map((b, i) => ({ id: b.id, name: b.name, all: w[i].all, top: w[i].top, v1: nf(b.n), v2: pct(keeperRate(b) ?? 0),
        tip: { title: b.name, sub: `${photosText(b.n)} · ${t("stats.keeper.tip", { rate: pct(keeperRate(b) ?? 0), top: nf(b.top), n: nf(b.n) })}` } }));
    }
    const ranked = rankRows(lenses.lenses, mode).slice(0, LENS_ROWS);
    const w = barWidths(ranked, mode);
    return ranked.map((l, i) => {
      const rate = keeperRate(l.keeperRate) ?? 0;
      const f = l.focalRange;
      const sub = !f ? undefined : l.isPrime || f.minMm === f.maxMm ? `${Math.round(f.modalMm)} mm` : `${Math.round(f.minMm)}–${Math.round(f.maxMm)} mm`;
      const small = isSmallSample(l.n);
      return { id: l.name, name: l.name, sub, all: w[i].all, top: w[i].top, v1: nf(l.n), v2: pct(rate), small,
        tip: { title: l.name, sub: `${photosText(l.n)} · ${t("stats.keeper.tip", { rate: pct(rate), top: nf(l.top), n: nf(l.n) })}${small ? ` · ${t("stats.small.tip", { n: MIN_SAMPLE })}` : ""}` } };
    });
  }, [lenses, mode, group, defs]);

  const buckets: FanBucket[] = useMemo(() => (lenses ? lenses.focalByBucket.filter((b) => b.n > 0 && b.modalMm > 0)
    .map((b) => ({ id: b.id, label: focalName(b.id), modalMm: b.modalMm, n: b.n })) : []), [lenses]);
  const bucketTotal = lenses ? lenses.focalByBucket.reduce((a, b) => a + b.n, 0) : 0;
  const order = FOCAL_BUCKETS.map((b) => b.id as string);
  const presets = lenses ? [...lenses.focalByBucket].sort((a, b) => order.indexOf(a.id) - order.indexOf(b.id)) : [];
  const featured = lenses && (lenses.focalByBucket.find((b) => selFocal.has(b.id)) ?? [...lenses.focalByBucket].sort((a, b) => b.n - a.n)[0]);

  const fav = overview?.favouriteFocal ?? null;
  const lensIns = useMemo(() => (lenses ? lensFocalInsight(lenses.lenses, fav?.mm ?? null) : null), [lenses, fav]);
  const pick = (id: string) => update((s) => toggleValue(s, "focalBuckets", id));

  if (error && !lenses) return <section className="st-sec" id="lenses"><SectionHead eyebrow={t("stats.lenses.eyebrow")} title={t("stats.lenses.title")} em={t("stats.lenses.titleEm")} /><ErrorCard message={error} onRetry={v.retry} /></section>;

  return (
    <section className="st-sec" id="lenses">
      <SectionHead eyebrow={t("stats.lenses.eyebrow")} title={t("stats.lenses.title")} em={t("stats.lenses.titleEm")}
        aside={<>
          <KnownFact known={lenses?.known} of={overview?.photos} label={t("stats.lenses.known")} />
          <Seg<Group> value={group} label={t("stats.lenses.group")} disabled={loading} onChange={setGroup}
               options={[{ id: "exact", label: t("stats.lenses.exact") }, { id: "grouped", label: t("stats.lenses.grouped") }]} />
        </>}
        lede={<Bold parts={richParts(t("stats.lenses.lede", { threshold: `**${threshold}**`, keeper: `**${t("stats.lenses.keeperRate")}**` }))} />} />

      <div className="st-grid st-grid-lenses">
        <div className="st-card">
          <div className="st-card-head">
            <div><h3>{t("stats.lenses.ranking")}</h3><p className="st-sub">{group === "grouped" ? t("stats.lenses.rankingSubGrouped") : t("stats.lenses.rankingSub", { n: MIN_SAMPLE })}</p></div>
            <Seg<LensMode> value={mode} label={t("stats.lenses.rankBy")} disabled={loading} onChange={setMode}
                 options={[{ id: "most-used", label: t("stats.lenses.mostUsed") }, { id: "keeper", label: t("stats.lenses.keeperRate") }, { id: "top", label: t("stats.lenses.topImages") }]} />
          </div>
          {empty ? <NoMatch onClear={() => update(clearAll)} /> :
            <RankedBars rows={rows} accent="cyan" selected={group === "grouped" ? selFocal : selLenses} v2Kind="pill" loadingRows={loading ? LENS_ROWS : 0}
                        onToggle={(id, add) => update((s) => toggleValue(s, group === "grouped" ? "focalBuckets" : "lenses", id, add))}
                        ariaFor={(r, i) => t("stats.lenses.aria", { rank: i + 1, name: r.name, n: r.v1, rate: r.v2 })} />}
        </div>

        <div className="st-card">
          <h3>{t("stats.lenses.fov")}</h3>
          <p className="st-sub">{t("stats.lenses.fovSub")}</p>
          <div className="st-presets" role="group" aria-label={t("stats.lenses.focalRange")}>
            {loading ? Array.from({ length: 6 }, (_, i) => <i key={i} className="st-sk st-sk-chip wide" />)
              : presets.map((b) => (
                <button key={b.id} type="button" className="st-opt amber" aria-pressed={selFocal.has(b.id)} onClick={() => pick(b.id)}>
                  {focalName(b.id)} <span>{focalRange(b.id, defs)} {"·"} {nf(b.n)}</span>
                </button>
              ))}
          </div>
          <div className="st-fov-grid">
            <FovFan buckets={buckets} selected={selFocal} loading={loading} title={t("stats.lenses.fovTitle")}
                    onToggle={(id, add) => update((s) => toggleValue(s, "focalBuckets", id, add))}
                    ariaFor={(b, deg) => t("stats.lenses.fovAria", { name: b.label, mm: Math.round(b.modalMm), n: nf(b.n), deg: Math.round(deg) })}
                    tipFor={(b, deg) => ({ title: `${b.label} ${focalRange(b.id, defs)} mm`, sub: `${photosText(b.n)} · ${t("stats.lenses.view", { deg: Math.round(deg) })}` })} />
            <div className="st-sample">
              <PhotoCard photo={loading ? null : featured?.cover ?? null} loading={loading} className="sample"
                         pill={featured ? `${Math.round(featured.modalMm)} mm · ${pct0(share(featured.n, bucketTotal))}` : undefined}
                         title={featured?.cover ? setupText(featured.cover, false) : ""} meta="" openLabel={t("stats.open.photo", { name: featured ? focalName(featured.id) : "" })}
                         onOpen={featured?.cover ? () => v.openPhoto(featured.cover!, lenses!.focalByBucket.map((b) => b.cover).filter((c): c is NonNullable<typeof c> => !!c)) : undefined} />
              <button type="button" className="st-link st-sample-link" disabled={!featured || loading} onClick={() => featured && v.openGrid({ focalBuckets: [featured.id as FocalBucketId] })}>{t("stats.lenses.openGrid")}</button>
            </div>
          </div>
          <div className="st-insight-slot">
            {!loading && fav && (
              <Insight tone="cyan" tag={t("stats.insight.favourite")}>
                <Bold parts={richParts(t("stats.insight.focal", { mm: `**${Math.round(fav.mm)} mm**`, share: pct0(fav.share) }))} />
                {lensIns?.focalRange && <>{" "}<Bold parts={richParts(t("stats.insight.lensFocal", { lens: `**${lensIns.name}**`, mm: `**${Math.round(lensIns.focalRange.modalMm)} mm**`, share: pct0(lensIns.focalRange.modalShare) }))} /></>}
              </Insight>
            )}
          </div>
        </div>
      </div>
    </section>
  );
}
