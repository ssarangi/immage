// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The scene view: the critique lightbox (GridCulling) showing one scene's frames,
// with a burst filmstrip and Accept / Undecided / Reject under the stage and a Scene
// tab in the side panel. GridCulling owns the photo, zoom and keys; this file gives
// it the scene's frames and the two pieces of chrome, as a SceneLightbox.
import { Check, Ban, Minus } from "lucide-react";
import { useEffect, useMemo, useState, type ComponentProps, type ReactNode } from "react";
import { GridCulling } from "./GridCulling";
import { StarIcon } from "./icons";
import type { ImageEntry } from "./library";
import {
  captureText, apertureText, focalText, filmstripOrder, frameState, isoText, scoreText, sceneStats, shutterText,
  type FrameState,
} from "./scenes";
import { preciseClock, clockText } from "./scenesFormat";
import { closeScene, openScene, scoreScene, useScenes } from "./scenesStore";
import { SceneThumb } from "./SceneThumb";
import { decide, parseClock, stateText, StateBadge, useCtx } from "./ScenesView";
import { formatNumber, t } from "./i18n";

export type SceneLightbox = {
  key: string;
  frames: ImageEntry[];
  startId: number;
  context: string;
  hasNext: boolean;
  hasPrevious: boolean;
  onClose: () => void;
  onScene: (by: 1 | -1) => void;
  decide: (current: ImageEntry, state: FrameState) => void;
  strip: (current: ImageEntry, onPick: (id: number) => void) => ReactNode;
  panel: (current: ImageEntry) => ReactNode;
};

// GridCulling with the scene view: the hook lives here, not in App, so the app shell does not render
// again for every score or key press of the timeline
export function GridCullingWithScenes({ scenesOn, ...props }: ComponentProps<typeof GridCulling> & { scenesOn: boolean }) {
  const scene = useSceneLightbox(props.allImages, props.onMeta);
  return <GridCulling {...props} scene={scenesOn ? scene : null} />;
}

const STATES: FrameState[] = ["accepted", "undecided", "rejected"];
const ICONS = { accepted: Check, undecided: Minus, rejected: Ban } as const;
const LABEL = { accepted: "cull.scenes.decision.accept", undecided: "cull.scenes.decision.undecided", rejected: "cull.scenes.decision.reject" } as const;
const HINT = { accepted: "cull.scenes.decision.acceptHint", undecided: "cull.scenes.decision.undecidedHint", rejected: "cull.scenes.decision.rejectHint" } as const;

export function useSceneLightbox(all: ImageEntry[], onMeta: (e: ImageEntry, f: Partial<ImageEntry>) => void): SceneLightbox | null {
  const s = useScenes();
  const [bySharpness, setBySharpness] = useState(true);
  const { ctx } = useCtx(all);
  const model = s.model;
  const scene = s.open !== null ? model?.scenes[s.open] : undefined;

  // opening a scene scores its frames first: under two seconds on a local disk
  useEffect(() => { if (s.open !== null) void scoreScene(s.open); }, [s.open]);

  const order = useMemo(() => scene ? filmstripOrder(scene.frames, s.quality, bySharpness) : [], [scene, s.quality, bySharpness]);
  const frames = useMemo(() => order.map((f) => ctx.entries[f]).filter((e): e is ImageEntry => !!e), [order, ctx.entries]);
  if (!model || !scene || s.open === null || !frames.length) return null;
  const open = s.open;
  const stats = sceneStats(scene, ctx);
  const block = model.blocks[scene.block];
  const first = ctx.entries[scene.frames[0]], last = ctx.entries[scene.frames[scene.frames.length - 1]];
  const from = first ? clockText(parseClock(first.exif.datetime || first.exif.file_datetime || "")) : "";
  const to = last ? clockText(parseClock(last.exif.datetime || last.exif.file_datetime || "")) : "";
  const range = block.noTime || !first || !last ? t("cull.scenes.noTime")
    : from === to ? t("cull.scenes.blockOne", { from }) : t("cull.scenes.block", { from, to });
  const flagsOf = (e: ImageEntry) => frameState(e, ctx.accepted);

  return {
    key: String(open),
    frames,
    startId: (ctx.entries[stats.shown] ?? frames[0]).imgid,
    context: t("cull.scenes.view.context", { time: range, frames: formatNumber(scene.frames.length) }),
    hasNext: open + 1 < model.scenes.length,
    hasPrevious: open > 0,
    onClose: closeScene,
    onScene: (by) => { const to = open + by; if (to >= 0 && to < model.scenes.length) openScene(to); },
    decide: (current, state) => decide([current], state, ctx, onMeta),
    strip: (current, onPick) => <div className="scene-strip">
      <div className="scene-decide" role="radiogroup" aria-label={t("cull.scenes.decision.label")}>
        {STATES.map((st) => {
          const Icon = ICONS[st];
          return <button key={st} type="button" role="radio" aria-checked={flagsOf(current) === st} data-kind={st} title={t(HINT[st])}
            onClick={() => decide([current], st, ctx, onMeta)}><Icon size={14} aria-hidden="true" />{t(LABEL[st])}</button>;
        })}
      </div>
      <div className="scene-tiles">
        <div className="scene-tiles-head">
          <span>{t("cull.scenes.view.frames")}</span>
          <button type="button" className="scene-sort" aria-pressed={bySharpness} onClick={() => setBySharpness(!bySharpness)}>
            {t(bySharpness ? "cull.scenes.view.sortQuality" : "cull.scenes.view.sortTime")}</button>
        </div>
        <div className="scene-tile-list" role="list" aria-label={t("cull.scenes.view.frames")}>
          {frames.map((e) => {
            const q = scoreText(s.quality[s.items.findIndex((it) => it.path === e.path && it.version === (e.version ?? 0))] ?? null);
            return <button key={`${e.version ?? 0}:${e.path}`} type="button" role="listitem" className="scene-tile" aria-current={e.imgid === current.imgid}
              title={e.filename} onClick={() => onPick(e.imgid)}>
              <SceneThumb entry={e} />
              <StateBadge state={flagsOf(e)} />
              <span className="scene-tile-cap"><bdi dir="ltr">{e.filename}</bdi>
                <span>{e.rejected ? "" : "★".repeat(Math.min(5, e.rating)) || ""}{q === null ? "" : ` ${formatNumber(q)}`}</span></span>
            </button>;
          })}
        </div>
      </div>
    </div>,
    panel: (current) => {
      const idx = s.items.findIndex((it) => it.path === current.path && it.version === (current.version ?? 0));
      const q = scoreText(s.quality[idx] ?? null);
      const cap = captureText(current.exif);
      const ms = parseClock(cap.takenAt);
      const dash = "-";
      const rows: [string, string][] = [
        ["file", current.filename],
        ["quality", q === null ? dash : formatNumber(q)],
        ["camera", current.exif.model?.trim() || dash],
        ["lens", current.exif.lens?.trim() || dash],
        ["focal", focalText(current.exif.focal_length) || dash],
        ["aperture", apertureText(current.exif.aperture) || dash],
        ["shutter", shutterText(current.exif.exposure) || dash],
        ["iso", isoText(current.exif.iso).replace("ISO ", "") || dash],
        ["time", cap.takenAt ? preciseClock(cap.takenAt, ms) + (cap.fromFile ? " ~" : "") : dash],
      ];
      return <>
        <h2 className="scene-info-title">{t("cull.scenes.view.title", { n: formatNumber(scene.n), total: formatNumber(model.scenes.length) })}</h2>
        <p className="scene-info-sub">{range} · {t("cull.scenes.card.frames", { n: scene.frames.length, count: formatNumber(scene.frames.length) })}</p>
        <dl className="scene-info">
          {rows.map(([k, v]) => <div key={k} style={{ display: "contents" }}>
            <dt>{t(`cull.scenes.info.${k}`)}</dt>
            <dd><bdi dir={k === "file" || k === "camera" || k === "lens" ? "auto" : "ltr"}>{v}</bdi>
              {k === "quality" && q !== null && <span className="scene-info-stars" aria-hidden="true">{" "}
                {Array.from({ length: Math.min(5, current.rating) }, (_, i) => <StarIcon key={i} filled />)}</span>}</dd>
          </div>)}
        </dl>
        <p className="scene-info-note">
          {s.scoring === null && scene.frames.some((f) => s.quality[f] === null) ? t("cull.scenes.view.scoreHint") : ""}
        </p>
        <p className="scene-info-note" role="status" aria-live="polite">
          {t("cull.scenes.decision.announce", { n: formatNumber(Math.max(1, frames.findIndex((e) => e.imgid === current.imgid) + 1)), state: stateText(flagsOf(current)) })}
        </p>
        <div className="scene-nav">
          <button type="button" className="cull-btn" disabled={open === 0} onClick={() => openScene(open - 1)}>{t("cull.scenes.view.previous")}</button>
          <button type="button" className="cull-btn" disabled={open + 1 >= model.scenes.length} onClick={() => openScene(open + 1)}>{t("cull.scenes.view.next")}</button>
        </div>
      </>;
    },
  };
}
