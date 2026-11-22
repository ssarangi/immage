// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Preferences > Culling's AI grading switch. The Rust side reads the same
// settings key (gemini.rs ENABLED) and refuses a critique or search while it
// is off, so a stale page cannot upload a photo. The Gemini key itself is
// never kept here: it lives in the system keychain (gemini_key_* commands).
import { settings } from "./settings";

export const AI_GRADING_KEY = "immage.aiGrading";

export function loadAiGrading(): boolean {
  try { return settings.get(AI_GRADING_KEY) === "1"; } catch { return false; }
}
export function storeAiGrading(on: boolean) {
  try { settings.set(AI_GRADING_KEY, on ? "1" : "0"); } catch { /* not kept */ }
}
