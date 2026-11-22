// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { useMemo } from "react";
import { t } from "../i18n";
import { MIN_SAMPLE, share } from "../statsMath";
import { APERTURE_ACCENT, IRIS_F, SHUTTER_ACCENT, apertureInsight, dashed, highIsoShare, shutterInsight } from "../statsExposure";
import { clearAll, selectedSet, toggleValue } from "../statsModel";
import { nf, pct, pct0, photosText, richParts, shutterText } from "../statsFormat";
import { IrisRings, type IrisGroup } from "./IrisRings";
import { IsoLadder } from "./IsoLadder";
import { ShutterBars } from "./ShutterBars";
import { Bold, ErrorCard, Insight, KnownFact, NoMatch, SectionHead } from "./Section";
import { useView } from "./ctx";
import "./exposure-time.css";

const fText = (f: number | null | undefined) => (f == null ? "–" : `f/${Math.round(f * 10) / 10}`);
const sText = (s: number | null | undefined) => { const x = s == null ? "" : shutterText(s); return !x ? "–" : x.endsWith("s") ? x : `${x} s`; };

export function ExposureSection() {
  const v = useView();
  const { state, update, response, loading, error } = v;
  const ex = response?.exposure;
  const defs = response?.defs;
  const overview = response?.overview;
  const selA = selectedSet(state, "apertureGroups");
  const selS = selectedSet(state, "shutterRegimes");
  const selI = selectedSet(state, "isoBins");
  const empty = !!ex && ex.aperture.every((a) => a.n === 0) && ex.shutter.every((s) => s.n === 0) && ex.iso.every((i) => i.n === 0);

  const apTotal = ex ? ex.aperture.reduce((a, g) => a + g.n, 0) : 0;
  const groups: IrisGroup[] = useMemo(() => (ex ? ex.aperture.map((a) => {
    const def = defs?.aperture.find((d) => d.id === a.id);
    const f = share(a.n, apTotal);
    return { id: a.id, label: dashed(def?.label ?? a.id), name: t(`stats.aperture.name.${a.id}`), fNumber: IRIS_F[a.id] ?? 4, n: a.n, share: f,
      accent: APERTURE_ACCENT[a.id] ?? "cyan", count: nf(a.n), percent: pct0(f) };
  }) : []), [ex, defs, apTotal]);

  const shTotal = ex ? ex.shutter.reduce((a, s) => a + s.n, 0) : 0;
  const isoTotal = ex ? ex.iso.reduce((a, s) => a + s.n, 0) : 0;
  const apIns = useMemo(() => (ex ? apertureInsight(ex.aperture, MIN_SAMPLE) : null), [ex]);
  const shIns = useMemo(() => (ex ? shutterInsight(ex.shutter) : null), [ex]);
  const label = (id: string) => groups.find((g) => g.id === id)?.label ?? id;
  const keeperTip = (n: number, top: number) => (n > 0 ? ` · ${t("stats.keeper.tip", { rate: pct(top / n), top: nf(top), n: nf(n) })}` : "");

  if (error && !ex) return <section className="st-sec" id="exposure"><SectionHead eyebrow={t("stats.exposure.eyebrow")} title={t("stats.exposure.title")} em={t("stats.exposure.titleEm")} /><ErrorCard message={error} onRetry={v.retry} /></section>;

  const m = ex?.median;
  const isoSub = ex ? (m?.iso != null ? t("stats.iso.sub", { iso: Math.round(m.iso), share: pct0(highIsoShare(ex.iso)) }) : t("stats.iso.subNoMedian", { share: pct0(highIsoShare(ex.iso)) })) : "";

  return (
    <section className="st-sec" id="exposure">
      <SectionHead eyebrow={t("stats.exposure.eyebrow")} title={t("stats.exposure.title")} em={t("stats.exposure.titleEm")}
        aside={<>
          {ex && m ? <span className="st-fact"><Bold parts={richParts(t("stats.exposure.median", { f: `**${fText(m.fNumber)}**`, s: `**${sText(m.shutterSec)}**`, iso: `**${m.iso != null ? Math.round(m.iso) : "–"}**` }))} /></span> : <span className="st-fact"><i className="st-sk st-sk-fact" /></span>}
          {selA.size > 0 ? <span className="st-fact"><b>{nf(ex?.known.aperture ?? 0)}</b> {t("stats.exposure.known")}</span> : <KnownFact known={ex?.known.aperture} of={overview?.photos} label={t("stats.exposure.known")} />}
        </>}
        lede={<Bold parts={richParts(t("stats.exposure.lede"))} />} />

      <div className="st-grid st-grid-exposure">
        <div className="st-card">
          <h3>{t("stats.aperture.title")}</h3>
          <p className="st-sub">{t("stats.aperture.sub")}</p>
          {empty ? <NoMatch onClear={() => update(clearAll)} /> :
            <IrisRings groups={groups} loading={loading} selected={selA} title={t("stats.aperture.title")}
                       desc={groups.map((g) => `${g.label} ${g.percent}`).join(", ")}
                       onToggle={(id, add) => update((s) => toggleValue(s, "apertureGroups", id, add))}
                       ariaFor={(g) => t("stats.aperture.aria", { label: g.label, name: g.name, n: nf(g.n), share: pct(g.share) })}
                       tipFor={(g) => ({ title: `${g.label} · ${g.name}`, sub: `${photosText(g.n)} · ${pct(g.share)}${keeperTip(g.n, ex?.aperture.find((a) => a.id === g.id)?.top ?? 0)}` })} />}
          <div className="st-insight-slot">
            {apIns && !loading && !empty && <Insight tone="cyan" tag={t("stats.insight")} onClick={() => update((s) => toggleValue(s, "apertureGroups", apIns.best ?? apIns.most))}>
              <Bold parts={richParts(apIns.best && apIns.best !== apIns.most
                ? t("stats.aperture.insight", { most: `**${label(apIns.most)}**`, share: pct0(apIns.mostShare), best: `**${label(apIns.best)}**`, rate: pct(apIns.bestRate) })
                : apIns.bestRate <= 0 ? t("stats.aperture.insightMost", { most: `**${label(apIns.most)}**`, share: pct0(apIns.mostShare) })
                : t("stats.aperture.insightSame", { most: `**${label(apIns.most)}**`, share: pct0(apIns.mostShare), rate: pct(apIns.bestRate) }))} />
            </Insight>}
          </div>
          <h3 className="st-h3-gap">{t("stats.iso.title")}</h3>
          <p className="st-sub">{loading ? <i className="st-sk st-sk-line" /> : isoSub}</p>
          {empty ? null :
            <IsoLadder loading={loading} selected={selI} title={t("stats.iso.title")} desc={ex?.iso.map((i) => `ISO ${i.id} ${pct0(share(i.n, isoTotal))}`).join(", ") ?? ""}
                       bars={ex?.iso.map((i) => ({ id: i.id, label: i.id, n: i.n })) ?? []}
                       onToggle={(id, add) => update((s) => toggleValue(s, "isoBins", id, add))}
                       ariaFor={(b) => t("stats.iso.aria", { iso: b.label, n: nf(b.n), share: pct(share(b.n, isoTotal)) })}
                       tipFor={(b) => ({ title: `ISO ${b.label}`, sub: `${photosText(b.n)} · ${pct(share(b.n, isoTotal))}${keeperTip(b.n, ex?.iso.find((i) => i.id === b.id)?.top ?? 0)}` })} />}
        </div>

        <div className="st-card">
          <h3>{t("stats.shutter.title")}</h3>
          <p className="st-sub">{t("stats.shutter.sub")}</p>
          {empty ? <NoMatch onClear={() => update(clearAll)} /> :
            <ShutterBars loading={loading} selected={selS} title={t("stats.shutter.title")}
                         desc={ex?.shutter.map((s) => `${t(`stats.shutter.${s.id}`)} ${pct0(share(s.n, shTotal))}`).join(", ") ?? ""}
                         bars={ex?.shutter.map((s) => ({ id: s.id, name: t(`stats.shutter.${s.id}`), range: t(`stats.shutter.cap.${s.id}`), n: s.n, count: nf(s.n), accent: SHUTTER_ACCENT[s.id] ?? "cyan" })) ?? []}
                         onToggle={(id, add) => update((s) => toggleValue(s, "shutterRegimes", id, add))}
                         ariaFor={(b) => t("stats.shutter.aria", { name: b.name, range: b.range, n: nf(b.n), share: pct(share(b.n, shTotal)) })}
                         tipFor={(b) => ({ title: `${b.name} ${b.range}`, sub: `${photosText(b.n)} · ${pct(share(b.n, shTotal))}${keeperTip(b.n, ex?.shutter.find((s) => s.id === b.id)?.top ?? 0)}` })} />}
          <div className="st-insight-slot">
            {shIns && !loading && !empty && (
              <Insight tone="amber" tag={t("stats.insight")} onClick={() => update((s) => toggleValue(s, "shutterRegimes", shIns.id))}>
                <Bold parts={richParts(shIns.id !== "long" && shIns.longShare > 0
                  ? t("stats.shutter.insight", { name: `**${t(`stats.shutter.${shIns.id}`)}**`, range: t(`stats.shutter.cap.${shIns.id}`), share: `**${pct0(shIns.share)}**`, long: `**${pct(shIns.longShare)}**` })
                  : t("stats.shutter.insightOnly", { name: `**${t(`stats.shutter.${shIns.id}`)}**`, range: t(`stats.shutter.cap.${shIns.id}`), share: `**${pct0(shIns.share)}**` }))} />
              </Insight>
            )}
          </div>
        </div>
      </div>
    </section>
  );
}
