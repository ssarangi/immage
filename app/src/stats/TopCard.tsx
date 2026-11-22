// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { useEffect, useState } from "react";
import { formatDate, t } from "../i18n";
import { LABEL_COLORS } from "../library";
import { useStatsApi } from "../statsApi";
import { setupText } from "../statsFormat";
import { loadThumb } from "../statsThumbs";
import { LABELS } from "../statsTop";
import type { ColorLabel, PhotoRef } from "../statsTypes";
import "./top.css";

/** a thumbnail's address, and whether it is on its way (`pending`), failed or absent (`none`): a card says which */
export function useThumbState(photo: PhotoRef | null, on: boolean): { src: string | null; state: "pending" | "ready" | "none" } {
  const api = useStatsApi();
  const key = photo ? photo.path : "";
  const [got, setGot] = useState<{ key: string; src: string | null } | null>(null);
  useEffect(() => {
    if (!photo || !on || photo.thumb.kind === "none") return;
    let alive = true;
    void loadThumb(api, photo).then((src) => { if (alive) setGot({ key, src }); });
    return () => { alive = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api, key, on]);
  if (!photo || photo.thumb.kind === "none") return { src: null, state: photo ? "none" : "pending" };
  if (got?.key === key) return { src: got.src, state: got.src ? "ready" : "none" };
  return { src: null, state: "pending" };
}

const day = (iso: string | null) => {
  const m = iso && /^(\d{4})-(\d{2})-(\d{2})/.exec(iso);
  return m ? formatDate(new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3])), { year: "numeric", month: "short", day: "numeric" }) : "";
};

/** "#3" corner badge, the five-star overlay (always five slots, so a rating never changes its width) */
export function Stars({ rating }: { rating: number }) {
  return (
    <span className="st-pill amber st-tc-stars" role="img" aria-label={t("stats.top.rating", { n: rating })}>
      {[1, 2, 3, 4, 5].map((i) => <i key={i} className={i <= rating ? "on" : ""} aria-hidden="true">{"★"}</i>)}
    </span>
  );
}

export function LabelDot({ label }: { label: ColorLabel }) {
  return <i className="st-tc-dot" style={{ background: LABEL_COLORS[LABELS.indexOf(label)] }} aria-hidden="true" />;
}

/**
 * A photo in the Top images grid. One button (the image and its text) opens the photo; the image slot keeps
 * its 3:2 box whether the thumbnail is on its way, absent or shown, and everything the pointer or the keyboard
 * changes is a border or an opacity, so no card moves.
 */
export function TopCard({ photo, rank, near, tabIndex, onOpen, onFocus, index }: {
  photo: PhotoRef; rank: number; /** the card is near the view: its image loads */ near: boolean; tabIndex: number; index: number;
  onOpen(): void; onFocus(): void;
}) {
  const { src, state } = useThumbState(photo, near);
  const lens = photo.lens ?? "";
  const body = photo.body ?? "";
  const when = day(photo.taken);
  const name = lens || setupText(photo, false) || body || photo.path.split("/").pop() || "";
  const second = [body && lens ? body : "", when].filter(Boolean).join(" · ") || (lens ? "" : body);
  const label = photo.label ? t(`stats.label.${photo.label}`) : "";
  return (
    <div className="st-tc">
      <button type="button" className="st-tc-main" tabIndex={tabIndex} data-nav="top" data-index={index} onFocus={onFocus} onClick={onOpen}
              aria-label={t("stats.top.open", { rank, name, rating: photo.rating })}
              data-tip={name} data-tip-sub={[setupText(photo), label].filter(Boolean).join(" · ") || undefined}>
        <span className="st-tc-im">
          {src ? <img src={src} alt="" draggable={false} /> : state === "none"
            ? <span className="st-tc-none"><svg viewBox="0 0 24 24" aria-hidden="true"><rect x="3.5" y="5" width="17" height="14" rx="2" fill="none" stroke="currentColor" strokeWidth="1.5" /><path d="m4 17 5-5 3 3 3-3 5 5M3 3l18 18" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg>{t("stats.top.noPreview")}</span>
            : <span className="st-tc-blank st-sk" aria-hidden="true" />}
          <span className="st-pill st-tc-rank">{"#"}{rank}</span>
          <Stars rating={photo.rating} />
        </span>
        <span className="st-tc-meta">
          <b>{name}</b>
          <span>{second}</span>
          <span className="st-tc-where">
            {photo.label && <><LabelDot label={photo.label} /><span className="st-sr">{label}{photo.place ? ", " : ""}</span></>}
            {photo.place && <span className="st-tc-place">{photo.place}</span>}
          </span>
        </span>
      </button>
    </div>
  );
}

/** the skeleton of a card: the same boxes */
export function TopCardSkeleton({ more }: { more?: boolean }) {
  return (
    <div className={`st-tc${more ? " more" : ""}`} aria-hidden="true">
      <div className="st-tc-main">
        <span className="st-tc-im"><span className="st-tc-blank st-sk" /></span>
        <span className="st-tc-meta"><i className="st-sk st-sk-line" /><i className="st-sk st-sk-line short" /><i className="st-sk st-sk-line short" /></span>
      </div>
    </div>
  );
}
