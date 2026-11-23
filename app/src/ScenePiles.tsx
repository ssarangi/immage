// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The piles review: what the keep rules propose, one row per scene with a Keep pile and a Reject pile side by
// side, in the grid's place while it is open. A frame moves to the other pile with a click; Keep all and Reject
// all move a whole scene. Those hand-made calls win over the rules when Apply writes (scenes_rules.rs). The
// list is virtualised by scene with one fixed row height, so the scrollbar is right from the first frame.
import { Ban, Check, X } from "lucide-react";
import { useLayoutEffect, useMemo, useRef, useState } from "react";
import type { ImageEntry } from "./library";
import { PILE_ROW_H, frameKey, pileOf, pileScenes, pileWindow, type Pile } from "./scenes";
import { forceFrames, previewRules, setScenes, useScenes } from "./scenesStore";
import { SceneThumb } from "./SceneThumb";
import { useCtx } from "./ScenesView";
import { formatNumber, t } from "./i18n";

export function ScenePiles({ entries }: { entries: ImageEntry[] }) {
  const s = useScenes();
  const { ctx } = useCtx(entries);
  const scroller = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ top: 0, height: 600 });
  const model = s.model;
  const verdicts = s.preview?.verdicts ?? [];
  const scenes = useMemo(() => pileScenes(verdicts), [verdicts]);

  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const read = () => setView((v) => v.height === el.clientHeight ? v : { ...v, height: el.clientHeight });
    read();
    const ro = new ResizeObserver(read);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);
  if (!model) return null;
  const [from, to] = pileWindow(scenes.length, view.top, view.height);

  // the verdicts are recomputed after every hand-made call, so the piles always show what Apply would do
  const move = (keys: string[], force: "keep" | "reject") => {
    forceFrames(keys, force);
    void previewRules(ctx);
  };
  const columns = ["keep", "reject"] as const;
  return <div className="scene-piles" role="dialog" aria-label={t("cull.scenes.piles.title")}>
    <div className="scene-piles-bar">
      <strong>{t("cull.scenes.piles.title")}</strong>
      <span className="scenes-summary">{t("cull.scenes.piles.count", { n: scenes.length, count: formatNumber(scenes.length) })}</span>
      <button type="button" className="btn small scene-piles-close" onClick={() => setScenes({ piles: false })} aria-label={t("cull.scenes.piles.close")} title={t("cull.scenes.piles.close")}>
        <X size={14} aria-hidden="true" /></button>
    </div>
    <div ref={scroller} className="scene-piles-scroll" onScroll={(e) => { const top = e.currentTarget.scrollTop; setView((v) => v.top === top ? v : { ...v, top }); }}>
      <div className="scene-piles-canvas" style={{ height: scenes.length * PILE_ROW_H }}>
        {scenes.slice(from, to).map((sceneIndex, i) => {
          const info = model.scenes[sceneIndex];
          const rows = info.frames.map((f, k) => ({ f, v: verdicts[sceneIndex]?.[k] })).filter((x) => x.v);
          const keysOf = (pile: Pile) => rows.filter((x) => pileOf(x.v!) === pile).map((x) => x.f);
          const movable = (f: number) => !ctx.entries[f]?.rejected;
          return <div key={sceneIndex} className="scene-pile-row" style={{ top: (from + i) * PILE_ROW_H, height: PILE_ROW_H }}>
            <div className="scene-pile-title"><bdi dir="ltr">{t("cull.scenes.piles.scene", { n: formatNumber(info.n), frames: formatNumber(info.frames.length) })}</bdi></div>
            {columns.map((pile) => {
              const mine = keysOf(pile);
              const other = pile === "keep" ? "reject" : "keep";
              const Icon = pile === "keep" ? Check : Ban;
              return <div key={pile} className={`scene-pile ${pile}`}>
                <div className="scene-pile-head">
                  <span><Icon size={12} aria-hidden="true" /> {t(`cull.scenes.piles.${pile}`)} <bdi dir="ltr">{formatNumber(mine.length)}</bdi></span>
                  <button type="button" className="btn small" disabled={rows.every((x) => pileOf(x.v!) === pile || !movable(x.f))}
                    onClick={() => move(rows.filter((x) => pileOf(x.v!) !== pile && movable(x.f)).map((x) => frameKey(s.items[x.f].path, s.items[x.f].version)), pile)}>
                    {t(`cull.scenes.piles.${pile}All`)}</button>
                </div>
                <div className="scene-pile-strip" role="list">
                  {mine.map((f) => {
                    const e = ctx.entries[f];
                    return <button key={f} type="button" role="listitem" className="scene-pile-tile" disabled={!movable(f)}
                      title={t(`cull.scenes.piles.move_${other}`, { name: e?.filename ?? "" })} aria-label={t(`cull.scenes.piles.move_${other}`, { name: e?.filename ?? "" })}
                      onClick={() => move([frameKey(s.items[f].path, s.items[f].version)], other)}>
                      <SceneThumb entry={e} />
                    </button>;
                  })}
                </div>
              </div>;
            })}
          </div>;
        })}
      </div>
    </div>
  </div>;
}
