// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The pure parts of the Keywording card's Auto-keywords group: which keywords belong to it, and the
// state its header shows. The state comes from the library-wide analysis status that already exists
// (`analysis_status`, `keywords_summary`) and the photo's own result (`keywords_photo_states`):
// "analysing" is a photo with no result while the queue works, "nothing found" a photo the analysis has
// been through that got none, and a photo with no result and an idle queue has no group.

export type AutoState = "analysing" | "none" | "done" | "blocked" | "paused";

export type AutoInput = {
  enabled: boolean;     // the analysis is on and its model installed
  paused: boolean;      // the user paused background work
  busy: boolean;        // photos are being analysed or wait for it
  blocked: boolean;     // stopped: no memory, or failing
} | null;               // not read yet

// null: no group at all (off with nothing added, or the status is not in and the photo has none)
export function autoGroupState(s: AutoInput, count: number, analysed = false): AutoState | null {
  if (!s || !s.enabled) return count > 0 ? "done" : null;
  if (s.blocked) return "blocked";
  if (s.paused) return "paused";
  if (count > 0) return "done";
  if (analysed) return "none";
  return s.busy ? "analysing" : null;
}

// the keyword of one photo or of a selection: on `n` of the `total` photos
export type KeywordItem = { name: string; n: number; total: number };

// The keywords of one photo or a selection, split into the user's own and the ones Auto-keywords added.
// A keyword is Auto-keywords' when every photo that has it got it from the analysis. Own: those on every
// photo first, in the order met, then the ones on only some, most photos first. Auto: highest `rank` first
// (confidence), then by name; both orders are stable for the same input.
export function groupRows(rows: readonly { keywords: readonly string[]; ai: ReadonlySet<string> }[],
                          rank: (name: string) => number = () => 0): { own: KeywordItem[]; auto: KeywordItem[] } {
  const total = rows.length;
  const seen = new Map<string, { n: number; ai: number; at: number }>();
  for (const r of rows) for (const k of new Set(r.keywords)) {
    const e = seen.get(k) ?? { n: 0, ai: 0, at: seen.size };
    e.n++; if (r.ai.has(k)) e.ai++;
    seen.set(k, e);
  }
  const own: KeywordItem[] = [], auto: KeywordItem[] = [];
  for (const [name, e] of [...seen].sort((a, b) => a[1].at - b[1].at)) (e.ai === e.n ? auto : own).push({ name, n: e.n, total });
  own.sort((a, b) => Number(b.n === total) - Number(a.n === total) || (a.n === total ? 0 : b.n - a.n || a.name.localeCompare(b.name)));
  auto.sort((a, b) => rank(b.name) - rank(a.name) || a.name.localeCompare(b.name));
  return { own, auto };
}
