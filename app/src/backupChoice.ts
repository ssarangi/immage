// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Which parts of a backup are ticked when the dialog opens: all of them, except the ones that must be asked for.
// People & face data holds the names given to people and biometric face data, so it is never in a backup (or restored
// from one) unless the user ticks it (backup.rs: not in the default set).
export const OPT_IN = new Set(["faces"]);

export const initialChoice = (ids: readonly string[]): Set<string> => new Set(ids.filter((id) => !OPT_IN.has(id)));

// the Preferences-style id a component's text is keyed by
export const COMPONENT_KEY: Record<string, string> = {
  settings: "settings", presets: "presets", export_presets: "exportPresets", looks: "looks", plugins: "plugins", darktable: "darktable", faces: "faces",
};
