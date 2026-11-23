// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Library panel's Metadata and Keywording cards while the next photo opens: its entry and
// keywords arrive after the open starts, and a card that renders nothing meanwhile vanishes and
// shifts Folders, Catalog and History up and back with every photo. The last photo's entry and
// keywords (none yet for the first photo) stand in until the new ones are in; the cards are read-only meanwhile (`loading`).
// With no photo open (`opening` false) there is nothing to hold and the cards go.
import { useRef } from "react";

export function useCardHold<E>(entry: E | null, opening: boolean, keywords: string[] | null) {
  const heldEntry = useRef<E | null>(null);
  const heldKeywords = useRef<string[] | null>(null);
  const shown = entry ?? (opening ? heldEntry.current : null);
  heldEntry.current = shown;
  const shownKeywords = shown ? (keywords ?? heldKeywords.current ?? []) : null;
  heldKeywords.current = shownKeywords;
  return { entry: shown, keywords: shownKeywords, loading: !entry || !keywords };
}
