// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Preferences > Culling's "Grade automatically with Gemini" switch. It is
// reserved for automatic cloud grading, which does not exist yet, and has no
// effect today: neither the page nor the Rust side (gemini.rs ENABLED) refuses
// anything when it is off. Manual actions (Critique, search) need only a saved
// Gemini key. The key itself is never kept here: it lives in the system
// keychain (gemini_key_* commands).
import { settings } from "./settings";

export const AI_GRADING_KEY = "immage.aiGrading";

export function loadAiGrading(): boolean {
  try { return settings.get(AI_GRADING_KEY) === "1"; } catch { return false; }
}
export function storeAiGrading(on: boolean) {
  try { settings.set(AI_GRADING_KEY, on ? "1" : "0"); } catch { /* not kept */ }
}
