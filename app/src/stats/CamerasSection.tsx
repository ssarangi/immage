// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { useMemo } from "react";
import { t } from "../i18n";
import { clearAll, clearDim, selectedSet, setValues, setYearMode, toggleValue, valuesOf } from "../statsModel";
import { barWidths, eraChips, groupMakers, isSmallSample, keeperBodyInsight, keeperRate, MIN_SAMPLE, OTHER_ID, rankRows, sameYears, share } from "../statsMath";
import { bodyName, nf, pct, richParts, starsAtLeast, stars, photosText } from "../statsFormat";
import { Seg } from "./Controls";
import { Donut } from "./Donut";
import { PhotoCard } from "./PhotoCard";
import { RankedBars, type RankRow } from "./RankedBars";
import { Bold, ErrorCard, Insight, KnownFact, NoMatch, SectionHead, SmallSample } from "./Section";
import { ShapeMark } from "./Shapes";
import { useView } from "./ctx";

const KEEPER_ROWS = 6;
const BODY_CARDS = 12;
const LEGEND_ROWS = 6;
const RANK_MIN = 20;

export function CamerasSection() {
  const v = useView();
  const { state, update, response, loading, error } = v;
  const cams = response?.cameras;
  const minRating = state.filter.minRating;
  const threshold = starsAtLeast(minRating);
  const empty = !!cams && cams.bodies.length === 0 && cams.makers.length === 0;

  const marks = useMemo(() => (cams ? groupMakers(cams.makers, t("stats.cameras.other")) : []), [cams]);
  const total = marks.reduce((a, m) => a + m.n, 0);
  const selMakers = selectedSet(state, "makers");
  // "Other" is selected when its members are exactly the chosen makers
  const selected = useMemo(() => {
    const set = new Set(selMakers);
    const other = marks.find((m) => m.id === OTHER_ID);
    if (other?.members && other.members.length && other.members.every((x) => set.has(x))) { other.members.forEach((x) => set.delete(x)); set.add(OTHER_ID); }
    return set;
  }, [selMakers, marks]);

  const toggleMaker = (id: string, additive: boolean) => {
    const other = marks.find((m) => m.id === OTHER_ID);
    if (id === OTHER_ID && other?.members) {
      const same = selected.has(OTHER_ID);
      update((s) => (same ? clearDim(s, "makers") : setValues(s, "makers", other.members!)));
    } else update((s) => toggleValue(s, "makers", id, additive));
  };

  const top3 = marks.slice(0, 3).reduce((a, m) => a + m.n, 0);
  const pickedMarks = marks.filter((m) => selected.has(m.id));
  const center = pickedMarks.length
    ? { value: pct(share(pickedMarks.reduce((a, m) => a + m.n, 0), total)), caption: pickedMarks.length === 1 ? pickedMarks[0].label : t("stats.cameras.pickedMakers", { n: pickedMarks.length }) }
    : { value: pct(share(top3, total)), caption: t("stats.cameras.top3") };

  const bodies = cams?.bodies ?? [];
  const keeperRows: RankRow[] = useMemo(() => {
    const ranked = rankRows(bodies, "keeper", RANK_MIN).slice(0, KEEPER_ROWS);
    const w = barWidths(ranked, "keeper");
    return ranked.map((b, i) => {
      const rate = keeperRate(b.keeperRate) ?? 0;
      return { id: b.key, name: bodyName(b.key), all: w[i].all, top: w[i].top, v1: pct(rate), v2: nf(b.n), small: isSmallSample(b.n),
        tip: { title: bodyName(b.key), sub: `${t("stats.keeper.tip", { rate: pct(rate), top: nf(b.top), n: nf(b.n) })}${isSmallSample(b.n) ? ` · ${t("stats.small.tip", { n: MIN_SAMPLE })}` : ""}` } };
    });
  }, [bodies]);

  const overview = response?.overview;
  const avg = overview && overview.matches.n ? overview.matches.top / overview.matches.n : null;
  const insight = useMemo(() => keeperBodyInsight(bodies), [bodies]);
  const era = useMemo(() => (cams ? eraChips(cams.years, state.yearMode) : []), [cams, state.yearMode]);
  const years = valuesOf(state.filter, "years").map(Number);
  const eraAll = cams ? cams.years.reduce((a, y) => a + y.n, 0) : 0;

  const bodyCards = bodies.slice(0, BODY_CARDS);
  const selBodies = selectedSet(state, "bodies");
  const covers = bodyCards.map((b) => b.cover).filter((c): c is NonNullable<typeof c> => !!c);

  if (error && !cams) return <section className="st-sec" id="cameras"><SectionHead eyebrow={t("stats.cameras.eyebrow")} title={t("stats.cameras.title")} em={t("stats.cameras.titleEm")} /><ErrorCard message={error} onRetry={v.retry} /></section>;

  return (
    <section className="st-sec" id="cameras">
      <SectionHead eyebrow={t("stats.cameras.eyebrow")} title={t("stats.cameras.title")} em={t("stats.cameras.titleEm")}
        aside={<><SmallSample n={overview?.matches.n} /><KnownFact known={cams?.known} of={overview?.photos} label={t("stats.cameras.known")} /></>}
        lede={t("stats.cameras.lede", { threshold })} />

      <div className="st-era" role="group" aria-label={t("stats.cameras.era")}>
        <span className="st-era-lbl">{t("stats.cameras.era")}</span>
        {loading ? Array.from({ length: 6 }, (_, i) => <i key={i} className="st-sk st-sk-chip" />) : <>
          <button type="button" className="st-opt" aria-pressed={years.length === 0} onClick={() => update((s) => clearDim(s, "years"))}>{t("stats.cameras.allTime")} <span>{nf(eraAll)}</span></button>
          {era.map((c) => (
            <button key={c.id} type="button" className="st-opt" aria-pressed={sameYears(years, c.years)}
                    onClick={() => update((s) => (sameYears(valuesOf(s.filter, "years").map(Number), c.years) ? clearDim(s, "years") : setValues(s, "years", c.years)))}>
              {c.upTo ? `≤ ${c.upTo}` : c.id} <span>{nf(c.n)}</span>
            </button>
          ))}
        </>}
        <span className="st-grow" />
        <Seg<"that-year" | "cumulative"> value={state.yearMode} label={t("stats.cameras.yearMode")} disabled={loading}
             options={[{ id: "that-year", label: t("stats.cameras.thatYear") }, { id: "cumulative", label: t("stats.cameras.cumulative") }]}
             onChange={(m) => update((s) => setYearMode(s, m))} />
      </div>

      <div className="st-grid st-grid-cameras">
        <div className="st-card">
          <h3>{t("stats.cameras.makerShare")}</h3>
          <p className="st-sub">{loading ? <i className="st-sk st-sk-line" /> : empty ? " " : t("stats.cameras.makerSub", { names: marks.slice(0, 3).map((m) => m.label).join(", "), share: pct(share(top3, total)) })}</p>
          <div className="st-donut-wrap">
            <Donut marks={marks} center={center} loading={loading} selected={selected} onToggle={toggleMaker}
                   title={t("stats.cameras.makerShare")} desc={marks.map((m) => `${m.label} ${pct(share(m.n, total))}`).join(", ")}
                   ariaFor={(m) => t("stats.cameras.makerAria", { name: m.label, count: nf(m.n), share: pct(share(m.n, total)) })}
                   tipFor={(m) => ({ title: m.label, sub: `${photosText(m.n)} · ${pct(share(m.n, total))}` })} />
            <div className="st-rows st-legend">
              {loading ? Array.from({ length: LEGEND_ROWS }, (_, i) => <div key={i} className="st-row st-row-sk" aria-hidden="true"><i className="st-sk st-sk-dot" /><i className="st-sk st-sk-line" /></div>)
                : empty ? <NoMatch onClear={() => update(clearAll)} />
                : marks.map((m) => (
                  <div className="st-legend-row" key={m.id}>
                    <button type="button" className="st-row" aria-pressed={selected.has(m.id)} onClick={(e) => toggleMaker(m.id, e.shiftKey || e.metaKey || e.ctrlKey)}
                            data-tip={m.label} data-tip-sub={`${photosText(m.n)} · ${pct(share(m.n, total))}`}>
                      <ShapeMark kind={m.shape} accent={m.accent} />
                      <span className="st-nm">{m.label}</span>
                      <span className="st-num">{nf(m.n)}</span>
                      <span className="st-pc-muted">{pct(share(m.n, total))}</span>
                      <span className="st-grid-spacer" />
                    </button>
                    <button type="button" className="st-grid-btn" aria-label={t("stats.cameras.gridMaker", { name: m.label })}
                            onClick={() => v.openGrid({ makers: m.id === OTHER_ID ? m.members ?? [] : [m.id] })}>{t("stats.grid")} {"↗"}</button>
                  </div>
                ))}
            </div>
          </div>
        </div>

        <div className="st-card">
          <h3>{t("stats.cameras.keeper")}</h3>
          <p className="st-sub">{loading ? <i className="st-sk st-sk-line" /> : t("stats.cameras.keeperSub", { threshold, avg: avg === null ? "–" : pct(avg) })}</p>
          {empty ? <NoMatch onClear={() => update(clearAll)} /> :
            <RankedBars rows={keeperRows} accent="green" selected={selBodies} v2Kind="muted" loadingRows={loading ? KEEPER_ROWS : 0}
                        onToggle={(id, add) => update((s) => toggleValue(s, "bodies", id, add))}
                        ariaFor={(r, i) => t("stats.keeper.aria", { rank: i + 1, name: r.name, rate: r.v1, n: r.v2 })} />}
          <div className="st-insight-slot">
            {insight && !loading && <Insight tone="amber" tag={t("stats.insight")} onClick={() => update((s) => toggleValue(s, "bodies", insight.key))}>
              <Bold parts={richParts(t("stats.insight.keeperBody", { body: insight.model, rate: pct(insight.rate), ratio: insight.ratio.toFixed(1) }))} />
            </Insight>}
          </div>
        </div>
      </div>

      <div className="st-card st-top-bodies">
        <div className="st-card-head">
          <div><h3>{t("stats.cameras.topBodies")}</h3><p className="st-sub">{t("stats.cameras.topBodiesSub")}</p></div>
          <div className="st-legend-note"><span><i style={{ background: "var(--stat-cyan)" }} />{t("stats.cameras.legendPhotos")}</span><span><i style={{ background: "var(--stat-amber)" }} />{t("stats.cameras.legendTop", { threshold })}</span></div>
        </div>
        <div className="st-photos">
          {loading ? Array.from({ length: BODY_CARDS }, (_, i) => <PhotoCard key={i} photo={null} loading title="" meta="" openLabel="" className={i >= 8 ? "more" : ""} />)
            : empty ? <NoMatch onClear={() => update(clearAll)} />
            : bodyCards.map((b, i) => {
              const rate = keeperRate(b.keeperRate) ?? 0;
              const name = bodyName(b.key);
              return (
                <PhotoCard key={b.key} photo={b.cover} className={i >= 8 ? "more" : ""} badge={t("stats.cameras.rank", { n: i + 1 })}
                           pill={photosText(b.n)} title={name}
                           meta={<><span className="st-stars">{stars(minRating)}</span> {t("stats.cameras.topShare", { rate: pct(rate) })}{b.topPlace ? ` · ${t("stats.cameras.best", { place: b.topPlace })}` : ""}</>}
                           pressed={selBodies.has(b.key)} pressLabel={t("stats.cameras.filterBody", { name })}
                           onPress={() => update((s) => toggleValue(s, "bodies", b.key))}
                           openLabel={t("stats.open.photo", { name })} onOpen={b.cover ? () => v.openPhoto(b.cover!, covers) : undefined}
                           tip={{ title: name, sub: `${photosText(b.n)} · ${t("stats.cameras.topShare", { rate: pct(rate) })}` }} />
              );
            })}
        </div>
      </div>
    </section>
  );
}
