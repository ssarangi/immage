// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { useMemo } from "react";
import { t } from "../i18n";
import { share } from "../statsMath";
import { DAYPART_ACCENT, EARLY_HOURS, LATE_HOURS, daypart, monthInsight, peakIndex } from "../statsExposure";
import { clearAll, selectedSet, toggleValue } from "../statsModel";
import { hourText, monthName, nf, pct, pct0, photosText, richParts, starsAtLeast } from "../statsFormat";
import { PolarClock } from "./PolarClock";
import { Seasonality, type MonthBar } from "./Seasonality";
import { Bold, ErrorCard, Insight, KnownFact, NoMatch, SectionHead } from "./Section";
import { ShapeMark } from "./Shapes";
import { useView } from "./ctx";
import "./exposure-time.css";

const two = (n: number) => String(n).padStart(2, "0");
const range = ([a, b]: [number, number]) => `${two(a)}–${two(b)}`;

export function TimeSection() {
  const v = useView();
  const { state, update, response, loading, error } = v;
  const tm = response?.time;
  const overview = response?.overview;
  const threshold = starsAtLeast(state.filter.minRating);
  const selH = selectedSet(state, "hours");
  const selM = selectedSet(state, "months");
  const empty = !!tm && tm.hours.every((h) => h.n === 0);
  // the clock ignores the hours chosen and the months ignore the month, so the count of photos with a time is not a share of the matches
  const ownPicked = selH.size > 0 || selM.size > 0 || (state.filter.years?.length ?? 0) > 0;

  const counts = useMemo(() => (tm ? tm.hours.map((h) => h.n) : []), [tm]);
  const total = counts.reduce((a, n) => a + n, 0);
  const months: MonthBar[] = useMemo(() => (tm ? tm.months.map((m, i) => ({ month: i + 1, label: monthName(i + 1, "short"), n: m.n, top: m.top })) : []), [tm]);
  const mTotal = months.reduce((a, m) => a + m.n, 0);
  const ins = useMemo(() => (tm ? monthInsight(tm.months) : null), [tm]);
  const dayName = (h: number) => t(`stats.daypart.${daypart(h)}`);
  const peakH = peakIndex(counts);

  if (error && !tm) return <section className="st-sec" id="time"><SectionHead eyebrow={t("stats.time.eyebrow")} title={t("stats.time.title")} em={t("stats.time.titleEm")} tail={t("stats.time.titleTail")} /><ErrorCard message={error} onRetry={v.retry} /></section>;

  return (
    <section className="st-sec" id="time">
      <SectionHead eyebrow={t("stats.time.eyebrow")} title={t("stats.time.title")} em={t("stats.time.titleEm")} tail={t("stats.time.titleTail")}
        aside={ownPicked ? <span className="st-fact"><b>{nf(tm?.known ?? 0)}</b> {t("stats.time.known")}</span> : <KnownFact known={tm?.known} of={overview?.photos} label={t("stats.time.known")} />}
        lede={t("stats.time.lede")} />

      <div className="st-grid st-grid-time">
        <div className="st-card">
          <h3>{t("stats.clock.title")}</h3>
          <p className="st-sub"><Bold parts={richParts(t("stats.clock.sub"))} /></p>
          {empty ? <NoMatch onClear={() => update(clearAll)} /> : <div className="st-clock-wrap">
            <PolarClock counts={counts} loading={loading} selected={selH} title={t("stats.clock.title")}
                        desc={peakH >= 0 ? t("stats.clock.desc", { hour: hourText(peakH), n: nf(counts[peakH]) }) : ""}
                        peakCaption={t("stats.clock.peak")} pickedCaption={t("stats.clock.picked")} hourText={hourText}
                        onToggle={(h, add) => update((s) => toggleValue(s, "hours", h, add))}
                        ariaFor={(h) => t("stats.clock.aria", { hour: hourText(h), end: `${two(h)}:59`, part: dayName(h), n: nf(counts[h] ?? 0) })}
                        tipFor={(h) => ({ title: `${hourText(h)}–${two(h)}:59`, sub: `${photosText(counts[h] ?? 0)} · ${pct(share(counts[h] ?? 0, total))} · ${dayName(h)}` })}
                        captionFor={(h) => photosText(counts[h] ?? 0)} />
          </div>}
          <div className="st-clock-key" role="list">
            <span role="listitem"><ShapeMark kind="square" accent={DAYPART_ACCENT.early} />{t("stats.clock.keyLight", { early: range(EARLY_HOURS), late: range(LATE_HOURS) })}</span>
            <span role="listitem"><ShapeMark kind="circle" accent={DAYPART_ACCENT.day} />{t("stats.daypart.day")}</span>
            <span role="listitem"><ShapeMark kind="diamond" accent={DAYPART_ACCENT.night} />{t("stats.daypart.night")}</span>
          </div>
        </div>

        <div className="st-card">
          <div className="st-card-head">
            <div>
              <h3>{t("stats.season.title")}</h3>
              <p className="st-sub">{ins ? <Bold parts={richParts(t("stats.season.sub", { month: `**${monthName(ins.month)}**`, n: `**${nf(ins.n)}**` }))} /> : (loading ? <i className="st-sk st-sk-line" /> : t("stats.season.subNone"))}</p>
            </div>
            <div className="st-legend-note"><span><i className="all" />{t("stats.season.all")}</span><span><i className="top" />{threshold}</span></div>
          </div>
          {empty ? <NoMatch onClear={() => update(clearAll)} /> :
            <Seasonality months={months} loading={loading} selected={selM} title={t("stats.season.title")}
                         desc={ins ? t("stats.season.desc", { month: monthName(ins.month), n: nf(ins.n) }) : ""}
                         onToggle={(m, add) => update((s) => toggleValue(s, "months", m, add))}
                         ariaFor={(m) => t("stats.season.aria", { month: monthName(m.month), n: nf(m.n), top: nf(m.top), threshold })}
                         tipFor={(m) => ({ title: monthName(m.month), sub: `${photosText(m.n)} · ${pct(share(m.n, mTotal))} · ${t("stats.season.topTip", { top: nf(m.top), threshold })}` })} />}
          <div className="st-insight-slot">
            {ins && !loading && !empty && (
              <Insight tone="amber" tag={t("stats.insight")} onClick={() => update((s) => toggleValue(s, "months", ins.month))}>
                <Bold parts={richParts(t("stats.season.insight", { month: `**${monthName(ins.month)}**`, ratio: `**${ins.ratio.toFixed(1)}×**`, rate: pct0(ins.topRate), threshold }))} />
              </Insight>
            )}
          </div>
        </div>
      </div>
    </section>
  );
}
