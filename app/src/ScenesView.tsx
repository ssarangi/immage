// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Scenes in the grid's place: a vertical timeline of time blocks, each a wrapping
// row of scene cards (the best frame, how many frames, its score and a camera line),
// grouped by day. Virtualised: every row's height follows from the counts alone
// (scenes.ts `layout`), so the scrollbar is right before a thumbnail loads and
// nothing moves when data, scores or pictures arrive. Hover, focus and selection
// only paint (outline, shadow, opacity); they never change a size.
import "./scenes.css";
import { Ban, Check, ChevronDown, Minus, Star, TriangleAlert } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactElement } from "react";
import type { ImageEntry } from "./library";
import {
  BLOCK_H, CARD_W, blockStats, cameraLine, decisionPatch, frameKey, layout, moveFocus, rowOfScene,
  sceneState, sceneStats, scoreText, timelineKey, visibleSceneIndexes, windowRows,
  type BlockInfo, type Ctx, type FrameState, type SceneState,
} from "./scenes";
import { clockText, dayText } from "./scenesFormat";
import {
  buildScenes, closeScene, loadAcceptStar, openScene, pauseScoring, rememberAccepted, setScenes, setScenesOn, toggleBlock, useScenes,
} from "./scenesStore";
import { SceneThumb } from "./SceneThumb";
import { formatNumber, t } from "./i18n";
import { keys } from "./keys";
import { EditViewIcon, GridViewIcon, ScenesViewIcon } from "./icons";

// the context every card and header reads: the library's flags, the scores, the accepts
export function useCtx(entries: readonly ImageEntry[]): { ctx: Ctx<ImageEntry>; byKey: Map<string, ImageEntry> } {
  const s = useScenes();
  const byKey = useMemo(() => new Map(entries.map((e) => [frameKey(e.path, e.version), e])), [entries]);
  const ctx = useMemo<Ctx<ImageEntry>>(() => ({
    entries: s.items.map((it) => byKey.get(frameKey(it.path, it.version))), quality: s.quality, accepted: s.accepted,
  }), [s.items, s.quality, s.accepted, byKey]);
  return { ctx, byKey };
}

// the library change of a decision, written one frame at a time as the grid's own controls do
export function decide(frames: ImageEntry[], state: FrameState, ctx: Ctx<ImageEntry>, onMeta: (e: ImageEntry, f: Partial<ImageEntry>) => void) {
  const star = loadAcceptStar();
  for (const e of frames) {
    const p = decisionPatch(state, e, ctx.accepted, star);
    onMeta(e, p.fields);
    rememberAccepted(e.path, e.version, p.accepted);
  }
}

const STATE_ICON: Record<SceneState, ReactElement> = {
  accepted: <Check size={12} aria-hidden="true" />, rejected: <Ban size={12} aria-hidden="true" />,
  undecided: <Minus size={12} aria-hidden="true" />, mixed: <Minus size={12} aria-hidden="true" />,
};
export const stateText = (s: SceneState) => t(`cull.scenes.state.${s}`);

export function StateBadge({ state }: { state: SceneState }) {
  return <span className={`scene-badge ${state}`} role="img" aria-label={stateText(state)} title={stateText(state)}>{STATE_ICON[state]}</span>;
}

function Card({ scene, ctx, active, onOpen }: {
  scene: number; ctx: Ctx<ImageEntry>; active: boolean; onOpen: (scene: number) => void;
}) {
  const s = useScenes();
  const info = s.model!.scenes[scene];
  const stats = sceneStats(info, ctx);
  const state = sceneState(stats);
  const shown = ctx.entries[stats.shown];
  const block = s.model!.blocks[info.block];
  const time = block.noTime || !shown ? "" : clockText(parseClock(shown.exif.datetime || shown.exif.file_datetime || ""));
  const score = scoreText(stats.bestQuality);
  const line = shown ? cameraLine(shown.exif) : [];
  return <button type="button" role="option" id={`scene-card-${scene}`} aria-selected={active}
    className={`scene-card${active ? " active" : ""}`} data-state={state} tabIndex={-1}
    aria-label={t("cull.scenes.card.aria", {
      n: formatNumber(info.n), frames: formatNumber(info.frames.length), time: time || t("cull.scenes.card.noTime"),
      q: score === null ? t("cull.scenes.card.unscored") : formatNumber(score), state: stateText(state),
    })}
    onClick={() => onOpen(scene)}>
    <span className="scene-thumb">
      <SceneThumb entry={shown} />
      <span className="scene-count" dir="ltr">{formatNumber(info.frames.length)}</span>
      <StateBadge state={state} />
      {stats.best !== null && <span className="scene-best" role="img" aria-label={t("cull.scenes.card.best")} title={t("cull.scenes.card.best")}>
        <Star size={12} aria-hidden="true" fill="currentColor" /></span>}
    </span>
    <span className="scene-meta">
      <span className="scene-row1">
        <bdi dir="ltr" className="scene-time">{time || "-"}</bdi>
        {score === null ? <span className="scene-q skel" title={t("cull.scenes.card.unscored")} aria-hidden="true" />
          : <span className="scene-q" title={t("cull.scenes.card.quality")}><bdi dir="ltr">Q {formatNumber(score)}</bdi></span>}
      </span>
      <span className="scene-cam" title={line.join(" · ")}><bdi dir="ltr">{line.join(" · ") || "-"}</bdi></span>
    </span>
  </button>;
}

// "YYYY:MM:DD HH:MM:SS" as ms since 1970 (UTC), for the card's clock
export function parseClock(text: string): number {
  const m = /^(\d{4})[:-](\d\d)[:-](\d\d)[ T](\d\d):(\d\d):(\d\d)/.exec(text);
  return m ? Date.UTC(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]) - Date.UTC(1970, 0, 1) : 0;
}

function BlockHeader({ block, ctx, folded }: { block: BlockInfo; ctx: Ctx<ImageEntry>; folded: boolean }) {
  const s = useScenes();
  const model = s.model!;
  const counts = blockStats(model, block, ctx);
  const label = block.noTime ? t("cull.scenes.noTime")
    : block.startMs === block.endMs || block.endMs === null ? t("cull.scenes.blockOne", { from: clockText(block.startMs!) })
    : t("cull.scenes.block", { from: clockText(block.startMs!), to: clockText(block.endMs) });
  return <div className="scene-block-head" role="presentation">
    <span className="scene-dot" aria-hidden="true" />
    <button type="button" className="scene-fold" aria-expanded={!folded} aria-label={t("cull.scenes.fold")} title={t("cull.scenes.fold")}
      data-block={block.index} onClick={() => toggleBlock(block.index)}>
      <ChevronDown size={14} aria-hidden="true" />
    </button>
    <strong className="scene-block-time"><bdi dir="ltr">{label}</bdi>
      {block.approx && <abbr className="scene-approx" title={t("cull.scenes.timeFromFile")} aria-label={t("cull.scenes.timeFromFile")}>~</abbr>}</strong>
    <span className="scene-block-sum">{t("cull.scenes.blockSummary", { n: block.count, count: formatNumber(block.count), frames: formatNumber(block.frames) })}</span>
    <span className="scene-block-counts">{t("cull.scenes.blockCounts", {
      accepted: formatNumber(counts.accepted), rejected: formatNumber(counts.rejected), undecided: formatNumber(counts.undecided) })}</span>
    {block.noTime && <span className="scene-block-note">{t("cull.scenes.noTimeNote")}</span>}
  </div>;
}

function Skeleton({ blocks }: { blocks: number }) {
  return <div className="scene-skeleton" aria-hidden="true">
    {Array.from({ length: blocks }, (_, b) => <div key={b} className="scene-skel-block">
      <div className="scene-block-head"><span className="scene-dot" /><span className="scene-skel-bar" /></div>
      <div className="scene-skel-cards">{Array.from({ length: 6 }, (_, c) => <span key={c} className="scene-skel-card" />)}</div>
    </div>)}
  </div>;
}

// the toolbar's Edit | Grid | Scenes switch: the three ways to look at the same photos
export type ViewMode = "edit" | "grid" | "scenes";
export function ViewSwitch({ view, onView, canScenes }: {
  view: ViewMode; onView: (v: ViewMode) => void; canScenes: boolean; labels?: boolean;
}) {
  const items: { id: ViewMode; icon: React.ReactNode; label: string; title: string; off?: boolean }[] = [
    { id: "edit", icon: <EditViewIcon />, label: t("library.toolbar.view.edit"), title: t("library.toolbar.view.editTitle") },
    { id: "grid", icon: <GridViewIcon />, label: t("cull.scenes.toggle.grid"), title: t("library.toolbar.view.gridTitle") },
    { id: "scenes", icon: <ScenesViewIcon />, label: t("cull.scenes.toggle.scenes"), title: keys(t("library.toolbar.view.scenesTitle")), off: !canScenes },
  ];
  return <span className="view-switch" role="radiogroup" aria-label={t("library.toolbar.view.label")}>
    {items.map((i) => (
      <button key={i.id} type="button" role="radio" aria-checked={view === i.id} aria-disabled={i.off || undefined} aria-label={i.label}
              title={i.title} onClick={() => { if (!i.off) onView(i.id); }}>
        {i.icon}<span className="view-label">{i.label}</span>
      </button>
    ))}
  </span>;
}

export function ScenesView({ entries, onMeta, autoBuild = true }: {
  entries: ImageEntry[]; onMeta: (e: ImageEntry, fields: Partial<ImageEntry>) => void;
  autoBuild?: boolean;   // the harness fills the store itself
}) {
  const s = useScenes();
  const { ctx } = useCtx(entries);
  const scroller = useRef<HTMLDivElement>(null);
  const [view, setView] = useState({ top: 0, height: 600, width: 1200 });
  const [details, setDetails] = useState(false);
  const rtl = document.documentElement.dir === "rtl";
  const local = useMemo(() => entries.filter((e) => !e.remote), [entries]);

  // build when the photos listed change; a rating or a label does not
  useEffect(() => {
    if (!autoBuild) return;
    const id = window.setTimeout(() => void buildScenes(local), 120);
    return () => window.clearTimeout(id);
  }, [local]);

  useLayoutEffect(() => {
    const el = scroller.current;
    if (!el) return;
    const read = () => setView((v) => v.height === el.clientHeight && v.width === el.clientWidth && v.top === el.scrollTop ? v
      : { top: el.scrollTop, height: el.clientHeight, width: el.clientWidth });
    read();
    const ro = new ResizeObserver(read);
    ro.observe(el);
    return () => ro.disconnect();
  }, [s.status]);

  const model = s.model;
  const lay = useMemo(() => model ? layout(model, view.width, s.hideSingles, s.collapsed) : null, [model, view.width, s.hideSingles, s.collapsed]);
  const [from, to] = lay ? windowRows(lay.rows, view.top, view.height) : [0, 0];

  // keep the focused scene on screen
  const focusRow = (scene: number) => {
    const el = scroller.current;
    if (!el || !lay || !model) return;
    const row = rowOfScene(lay, model, scene, s.hideSingles);
    if (!row) return;
    if (row.top < el.scrollTop) el.scrollTop = Math.max(0, row.top - BLOCK_H);
    else if (row.top + row.height > el.scrollTop + el.clientHeight) el.scrollTop = row.top + row.height - el.clientHeight;
  };

  const frameEntries = (scene: number, whole: boolean): ImageEntry[] => {
    if (!model) return [];
    const info = model.scenes[scene];
    const stats = sceneStats(info, ctx);
    const frames = whole ? info.frames : [stats.shown];
    return frames.map((f) => ctx.entries[f]).filter((e): e is ImageEntry => !!e);
  };

  const onKey = (e: KeyboardEvent) => {
    if (!model || !lay || e.metaKey || e.ctrlKey || e.altKey) return;
    const onHeader = (e.target as HTMLElement).closest(".scene-fold") !== null;
    const action = timelineKey(e.key, e.shiftKey, onHeader, rtl);
    if (action.type === "none") return;
    e.preventDefault(); e.stopPropagation();
    if (action.type === "move") {
      const next = moveFocus(lay.visible, model, s.focus, e.key, lay.columns, rtl);
      setScenes({ focus: next }); focusRow(next);
    } else if (action.type === "open") { if (lay.visible.length) openScene(lay.visible.includes(s.focus) ? s.focus : lay.visible[0]); }
    else if (action.type === "fold") {
      const block = Number((e.target as HTMLElement).closest<HTMLElement>(".scene-fold")?.dataset.block);
      if (s.collapsed.has(block) === action.fold) toggleBlock(block);
    } else if (action.type === "decide") decide(frameEntries(s.focus, action.whole), action.state, ctx, onMeta);
    else if (action.type === "back") setScenesOn(false);
  };

  // when the lightbox closes, the scene it was on has the focus and is in view
  const wasOpen = useRef(false);
  useEffect(() => {
    if (s.open === null && wasOpen.current) { scroller.current?.focus({ preventScroll: true }); if (model) focusRow(s.focus); }
    wasOpen.current = s.open !== null;
  }, [s.open]);   // eslint-disable-line react-hooks/exhaustive-deps

  const unhashedNote = s.unhashed > 0 ? t("cull.scenes.status.timeOnly", { n: s.unhashed, count: formatNumber(s.unhashed) }) : "";
  const allNoTime = !!model && model.blocks.length > 0 && model.blocks.every((b) => b.noTime);
  const empty = s.status === "ready" && model !== null && model.frameCount === 0;
  const loading = (s.status === "building" || s.status === "idle") && !model;

  let status = "";
  if (s.progress && (s.status === "building" || s.updating))
    status = s.updating ? t("cull.scenes.status.updating") : t("cull.scenes.status.grouping", { done: formatNumber(s.progress.done), total: formatNumber(s.progress.total) });
  else if (s.scoring) status = s.scoring.paused ? t("cull.scenes.status.paused") : t("cull.scenes.status.scoring", { done: formatNumber(s.scoring.done), total: formatNumber(s.scoring.total) });

  return <div className="scenes-view" dir={rtl ? "rtl" : "ltr"}>
    <div className="scenes-bar">
      <label className="scenes-toggle"><input type="checkbox" checked={s.hideSingles} onChange={(e) => setScenes({ hideSingles: e.target.checked })} />
        <span>{t("cull.scenes.hideSingles")}</span></label>
      <span className="scenes-summary">{model ? t("cull.scenes.summary", { n: model.scenes.length, count: formatNumber(model.scenes.length), frames: formatNumber(model.frameCount) }) : ""}</span>
      <span className="scenes-status" role="status" aria-live="polite" title={unhashedNote}>
        <span className="scenes-status-text">{status}</span>
        {/* always there, so the status line's boxes do not change when scoring starts or ends */}
        <button type="button" className="scenes-pause" aria-pressed={s.scoring?.paused ?? false} hidden={false}
          style={s.scoring ? undefined : { visibility: "hidden" }} tabIndex={s.scoring ? 0 : -1} aria-hidden={s.scoring ? undefined : true}
          title={t(s.scoring?.paused ? "cull.scenes.status.resume" : "cull.scenes.status.pause")}
          aria-label={t(s.scoring?.paused ? "cull.scenes.status.resume" : "cull.scenes.status.pause")}
          onClick={() => pauseScoring(!s.scoring?.paused)}>{s.scoring?.paused ? "▶" : "❚❚"}</button>
      </span>
    </div>
    <div className="scenes-body">
      {s.status === "error" && !model && <div className="scenes-message" role="alert">
        <TriangleAlert size={20} aria-hidden="true" />
        <strong>{t("cull.scenes.error.build")}</strong>
        <span className="scenes-actions">
          <button type="button" className="cull-btn primary" onClick={() => void buildScenes(local, true)}>{t("cull.scenes.error.retry")}</button>
          <button type="button" className="cull-btn" aria-expanded={details} onClick={() => setDetails(!details)}>{t("cull.scenes.error.details")}</button>
        </span>
        <pre className="scenes-details" hidden={!details}>{s.error}</pre>
      </div>}
      {loading && <Skeleton blocks={4} />}
      {empty && <div className="scenes-message"><strong>{t("cull.scenes.empty.none")}</strong></div>}
      {allNoTime && <div className="scenes-note">{t("cull.scenes.empty.noTime")}</div>}
      {model && lay && !empty && <div ref={scroller} className="scenes-scroll" role="listbox" tabIndex={0} aria-label={t("cull.scenes.title")}
        aria-activedescendant={lay.visible.includes(s.focus) ? `scene-card-${s.focus}` : undefined}
        onScroll={(e) => { const top = e.currentTarget.scrollTop; setView((v) => v.top === top ? v : { ...v, top }); }}
        onKeyDown={onKey}>
        <div className="scenes-canvas" style={{ height: lay.total }}>
          {lay.rows.slice(from, to).map((row, i) => {
            const style = { top: row.top, height: row.height };
            const block = model.blocks[row.block];
            if (row.kind === "day") return <div key={`d${row.block}`} className="scene-day" style={style} role="presentation">
              <h2>{block.startMs === null ? t("cull.scenes.noTime") : dayText(block.startMs)}</h2></div>;
            if (row.kind === "block") return <div key={`b${row.block}`} className="scene-row" style={style} role="presentation">
              <BlockHeader block={block} ctx={ctx} folded={s.collapsed.has(row.block)} /></div>;
            const shown = visibleSceneIndexes(model, block, s.hideSingles).slice(row.first, row.first + row.count);
            return <div key={`c${row.block}.${row.first}`} className="scene-row scene-cards" style={{ ...style, gridTemplateColumns: `repeat(${lay.columns}, ${CARD_W}px)` }} role="presentation" data-row={i}>
              {shown.map((sc) => <Card key={sc} scene={sc} ctx={ctx} active={s.focus === sc} onOpen={openScene} />)}
            </div>;
          })}
        </div>
      </div>}
    </div>
  </div>;
}
