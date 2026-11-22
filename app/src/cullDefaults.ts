// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Preferences > Culling's default cull settings: the starting values of the Auto
// Cull dialog and the options of Apply. The parsing is cullModel.ts's.
import { settings } from "./settings";
import { parseDefaults, type CullDefaults } from "./cullModel";

export const CULL_DEFAULTS_KEY = "immage.cullDefaults";

export function loadCullDefaults(): CullDefaults {
  try { return parseDefaults(settings.get(CULL_DEFAULTS_KEY)); } catch { return parseDefaults(null); }
}
export function storeCullDefaults(v: CullDefaults) {
  try { settings.set(CULL_DEFAULTS_KEY, JSON.stringify(v)); } catch { /* not kept */ }
}
