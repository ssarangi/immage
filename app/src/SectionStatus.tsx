// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// What a folded Library card still says in its header: a ring (or pause bars), a short text and a
// warning dot. The slot is always there, empty when nothing is going on, so a job starting or ending
// moves nothing: it takes the free space between the title and the buttons, and the text shortens
// to a percent, then to the ring alone, as the panel narrows (styles.css, .section-status).
import { Glyph, Ring } from "./IndexCard";

export interface SectionStatusProps {
  kind: "indexing" | "paused" | "idle";
  /** 0..1, or null while not known (an unfilled ring) */
  fraction: number | null;
  /** the full line, and the short one for a narrow panel (a percent, or nothing) */
  text: string;
  /** the count alone, for a narrower slot */
  mid: string | null;
  short: string | null;
  /** the whole status as one phrase for assistive technology and the tooltip */
  label: string;
  /** a warning dot beside the work, with its own phrase */
  warn: string | null;
}

export function SectionStatus({ kind, fraction, text, mid, short, label, warn }: SectionStatusProps) {
  const shown = kind !== "idle" || warn !== null;
  return (
    <span className="section-status" data-kind={kind} role={shown ? "img" : undefined} aria-label={shown ? label : undefined}
          title={shown ? label : undefined}>
      {shown && <>
        {kind === "indexing" && <Ring fraction={fraction} />}
        {kind === "paused" && <Glyph state="paused" />}
        {kind !== "idle" && <span className="ss-long">{text}</span>}
        {kind !== "idle" && mid !== null && <span className="ss-mid">{mid}</span>}
        {kind !== "idle" && short !== null && <span className="ss-short">{short}</span>}
        {warn !== null && <Glyph state="attention" />}
      </>}
    </span>
  );
}
