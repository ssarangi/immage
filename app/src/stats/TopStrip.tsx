// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { useEffect, useMemo, useState } from "react";
import { t } from "../i18n";
import { useStatsApi } from "../statsApi";
import { bodyName, nf } from "../statsFormat";
import { buildRequest, clearAll, type StatsState } from "../statsModel";
import { stripGroups, type StripGroup, type StripKind } from "../statsTop";
import type { PhotoRef, PlaceDetail, StatsFilter, StatsResponse } from "../statsTypes";
import { Seg } from "./Controls";
import { useThumbState } from "./TopCard";
import { chipLabel } from "./PlacePanel";
import "./top.css";

const SLOTS = 8;
const PLACE_ZOOM = 3;
const withoutPlace = (f: StatsFilter): StatsFilter => { const { place: _p, ...rest } = f; void _p; return rest as StatsFilter; };

/** The best frame of each of the biggest places, one `stats_place` per place, asked for only while the Place strip is shown. */
function usePlaceGroups(state: StatsState, on: boolean): { groups: StripGroup[]; loading: boolean; error: boolean } {
  const api = useStatsApi();
  const [out, setOut] = useState<{ key: string; groups: StripGroup[] } | null>(null);
  const [failed, setFailed] = useState<string | null>(null);
  const request = useMemo(() => { const r = buildRequest(state, ["places"]); r.filter = withoutPlace(r.filter); return r; }, [state.scope, state.filter]); // eslint-disable-line react-hooks/exhaustive-deps
  const key = JSON.stringify(request);
  useEffect(() => {
    if (!on) return;
    let alive = true;
    api.query({ ...request, places: { zoom: PLACE_ZOOM } }).then(async (res) => {
      if ("notModified" in res || !alive) return;
      const cells = (res.places?.top ?? []).slice(0, SLOTS);
      const details = await Promise.all(cells.map((c) => api.place(c.key, request.filter, request.scope).catch((): PlaceDetail | null => null)));
      if (!alive) return;
      const groups = details.flatMap((d): StripGroup[] => (d && d.photos[0] ? [{ key: d.key, name: d.name ?? chipLabel(d), n: d.n, top: d.top, photo: d.photos[0] }] : []))
        .sort((a, b) => b.top - a.top || b.n - a.n);
      setOut({ key, groups });
      setFailed(null);
    }).catch(() => { if (alive) setFailed(key); });
    return () => { alive = false; };
  }, [api, on, key]); // eslint-disable-line react-hooks/exhaustive-deps
  const have = out?.key === key;
  return { groups: have ? out!.groups : out?.groups ?? [], loading: on && !have && failed !== key, error: failed === key && !have };
}

function StripCard({ group, rank, openLabel, filterLabel, onOpen, onAll }: { group: StripGroup; rank: number; openLabel: string; filterLabel: string; onOpen(): void; onAll(): void }) {
  const { src, state } = useThumbState(group.photo, true);
  return (
    <li className="st-tcs">
      <button type="button" className="st-tcs-im" onClick={onOpen} aria-label={openLabel} data-tip={group.name}>
        {src ? <img src={src} alt="" draggable={false} /> : state === "none" ? <span className="st-tc-none">{t("stats.top.noPreview")}</span> : <span className="st-tc-blank st-sk" aria-hidden="true" />}
        <span className="st-pill st-tc-rank">{"#"}{rank}</span>
      </button>
      <span className="st-tcs-meta"><b>{group.name}</b><span>{t("stats.top.strip.count", { top: nf(group.top), n: nf(group.n) })}</span></span>
      <button type="button" className="st-link st-tcs-all" onClick={onAll} aria-label={filterLabel}>{t("stats.top.seeAll")}</button>
    </li>
  );
}

/**
 * "Top of each lens, camera, place": a horizontal strip of the best frame of each group, most top images first.
 * A frame opens in the editor with the strip as its neighbours; "See all" sets the matching filter, which the grid below follows.
 * The strip has a fixed height (cards, then a scroll bar's room), whether it holds skeletons, a message or cards.
 */
export function TopStrip({ state, response, loading, onOpen, onFilter, update }: {
  state: StatsState; response: StatsResponse | null; loading: boolean;
  onOpen(photo: PhotoRef, list: PhotoRef[]): void; onFilter(kind: StripKind, key: string): void; update(fn: (s: StatsState) => StatsState): void;
}) {
  const [kind, setKind] = useState<StripKind>("lenses");
  const place = usePlaceGroups(state, kind === "places");
  const own = useMemo(() => (kind === "places" ? [] : stripGroups(kind, response, kind === "bodies" ? bodyName : (k) => k)), [kind, response]);
  const groups = kind === "places" ? place.groups : own;
  const busy = kind === "places" ? place.loading : loading;
  const failed = kind === "places" && place.error;
  const list = groups.map((g) => g.photo);
  const nothing = !busy && !failed && groups.length === 0;
  return (
    <div className="st-card st-tstrip">
      <div className="st-card-head">
        <div><h3>{t("stats.top.strip.title")}</h3><p className="st-sub">{t("stats.top.strip.sub")}</p></div>
        <Seg<StripKind> value={kind} label={t("stats.top.strip.by")} onChange={setKind}
             options={[{ id: "lenses", label: t("stats.top.strip.lens") }, { id: "bodies", label: t("stats.top.strip.camera") }, { id: "places", label: t("stats.top.strip.place") }]} />
      </div>
      <div className="st-tstrip-scroll" tabIndex={-1}>
        {busy && !groups.length ? <ul className="st-tstrip-list" aria-hidden="true">{Array.from({ length: 6 }, (_, i) => (
          <li className="st-tcs" key={i}><span className="st-tcs-im"><span className="st-tc-blank st-sk" /></span><span className="st-tcs-meta"><i className="st-sk st-sk-line" /><i className="st-sk st-sk-line short" /></span></li>
        ))}</ul>
          : failed ? <div className="st-nomatch"><span>{t("stats.top.strip.failed")}</span></div>
          : nothing ? <div className="st-nomatch"><span>{t("stats.top.strip.none")}</span><button type="button" className="st-link" onClick={() => update(clearAll)}>{t("stats.filters.clear")}</button></div>
          : <ul className={`st-tstrip-list${busy ? " is-stale" : ""}`}>
            {groups.map((g, i) => (
              <StripCard key={g.key} group={g} rank={i + 1} openLabel={t("stats.open.photo", { name: g.name })} filterLabel={t("stats.top.strip.filter", { name: g.name })}
                         onOpen={() => onOpen(g.photo, list)} onAll={() => onFilter(kind, g.key)} />
            ))}
          </ul>}
      </div>
    </div>
  );
}
