// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The right panel's tool cards, the Presets groups and Advanced groups follow Focus mode: with it on,
// a plain click opens a card alone (or shuts it if it is the only one open) and ⌥-click opens it beside
// the others; with it off the two are swapped.
export function togglePanelSet<T>(old: ReadonlySet<T>, id: T, alt: boolean, focusMode: boolean): Set<T> {
  const solo = focusMode ? !alt && !old.has(id) : alt;
  if (solo) return new Set(old.has(id) && old.size === 1 ? [] : [id]);
  const next = new Set(old);
  if (next.has(id)) next.delete(id); else next.add(id);
  return next;
}

// The Library panel's cards (Metadata, Keywording, Folders, Catalog, History) are always independent:
// a click toggles only that card, whatever Focus mode and ⌥ say.
export function toggleLeftSet<T>(old: ReadonlySet<T>, id: T): Set<T> {
  const next = new Set(old);
  if (next.has(id)) next.delete(id); else next.add(id);
  return next;
}
