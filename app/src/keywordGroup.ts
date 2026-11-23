// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The pure parts of the Keywording card's Auto-keywords group: which keywords belong to it, and the
// state its header shows. The state comes from the library-wide analysis status that already exists
// (`analysis_status`, `keywords_summary`); there is no per-photo query yet, so "analysing" means the
// queue is working and this photo has none yet, and "nothing found" means the queue is idle.

export type AutoState = "analysing" | "none" | "done" | "blocked" | "paused";

export type AutoInput = {
  enabled: boolean;     // the analysis is on and its model installed
  paused: boolean;      // the user paused background work
  busy: boolean;        // photos are being analysed or wait for it
  blocked: boolean;     // stopped: no memory, or failing
} | null;               // not read yet

// null: no group at all (off with nothing added, or the status is not in and the photo has none)
export function autoGroupState(s: AutoInput, count: number): AutoState | null {
  if (!s || !s.enabled) return count > 0 ? "done" : null;
  if (s.blocked) return "blocked";
  if (s.paused) return "paused";
  if (count > 0) return "done";
  return s.busy ? "analysing" : "none";
}

// the photo's keywords split into the user's own, in the order given, and the ones Auto-keywords added, by name
export function splitKeywords(keywords: readonly string[], ai: ReadonlySet<string>): { own: string[]; auto: string[] } {
  const own: string[] = [], auto: string[] = [];
  for (const k of keywords) (ai.has(k) ? auto : own).push(k);
  auto.sort((a, b) => a.localeCompare(b));
  return { own, auto };
}
