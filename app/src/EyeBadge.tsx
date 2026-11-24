// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The closed-eye glyph in the corner of a thumbnail or a scene card (notes/faces-ux-spec.md 5.2): an eye with a slash for
// closed, a half lid for half closed, nothing for open or unknown. It is absolutely placed in its own corner (like the cull
// badge), so it never changes a cell's size, and the shape differs so colour is never the only cue.
import { t } from "./i18n";
import { useEye, type EyeLabel } from "./eyesStore";

export function EyeGlyph({ state }: { state: EyeLabel }) {
  return (
    <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M2.5 12S6 5.5 12 5.5 21.500 12 21.500 12 18 18.500 12 18.500 2.500 12 2.500 12z" />
      {state === "closed" ? <path d="M4 20 20 4" /> : <path d="M3.500 12h17" />}
    </svg>
  );
}

export function EyeBadge({ path, className = "" }: { path: string; className?: string }) {
  const state = useEye(path);
  if (state !== "closed" && state !== "semi") return null;
  const label = t(state === "closed" ? "cull.eyes.closed" : "cull.eyes.semi");
  return <span className={`eye-badge ${state}${className ? ` ${className}` : ""}`} role="img" aria-label={label} title={label}><EyeGlyph state={state} /></span>;
}
