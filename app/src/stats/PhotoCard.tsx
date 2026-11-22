// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import type { ReactNode } from "react";
import { useThumb } from "../statsThumbs";
import type { PhotoRef } from "../statsTypes";

/**
 * A photo in the rank grids: a 3:2 image (the thumbnail's centre), a rank badge, a count pill and two
 * lines. The card is one button (`onPress`: it filters, or opens the photo); `onOpen` adds a second,
 * separate button on the image that opens the photo when the card itself filters.
 */
export function PhotoCard({ photo, badge, pill, pillTone = "cyan", title, meta, onPress, pressed, onOpen, openLabel, pressLabel, loading, className = "", tip }: {
  photo: PhotoRef | null; badge?: string; pill?: string; pillTone?: "cyan" | "amber"; title: ReactNode; meta: ReactNode;
  onPress?: () => void; pressed?: boolean; onOpen?: () => void; openLabel: string; pressLabel?: string; loading?: boolean; className?: string;
  tip?: { title: string; sub?: string };
}) {
  const src = useThumb(photo);
  const main = onPress ?? onOpen;
  return (
    <div className={`st-pc ${pressed ? "on" : ""} ${className}`}>
      <button type="button" className="st-pc-main" onClick={main} disabled={loading || !main} aria-pressed={onPress && pressed !== undefined ? pressed : undefined}
              aria-label={onPress ? pressLabel : openLabel} data-tip={loading ? undefined : tip?.title ?? (onPress ? pressLabel : openLabel)}
              data-tip-sub={loading ? undefined : tip?.sub}>
        <span className="st-pc-im">
          {src ? <img src={src} alt="" draggable={false} /> : <span className={`st-pc-blank${loading ? " st-sk" : ""}`} aria-hidden="true" />}
          {badge && !loading && <span className="st-pill st-pc-badge">{badge}</span>}
          {pill && !loading && <span className={`st-pill ${pillTone} st-pc-win`}>{pill}</span>}
        </span>
        <span className="st-pc-meta">
          {loading ? <><i className="st-sk st-sk-line" /><i className="st-sk st-sk-line short" /></> : <><b>{title}</b><span>{meta}</span></>}
        </span>
      </button>
      {onPress && onOpen && !loading && photo && (
        <button type="button" className="st-pc-open" onClick={onOpen} aria-label={openLabel} data-tip={openLabel}>
          <svg viewBox="0 0 16 16" aria-hidden="true"><path d="M9.5 2.5h4v4M13.5 2.5 8.5 7.5M6.5 3.5h-3a1 1 0 0 0-1 1v8a1 1 0 0 0 1 1h8a1 1 0 0 0 1-1v-3" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" /></svg>
        </button>
      )}
    </div>
  );
}
