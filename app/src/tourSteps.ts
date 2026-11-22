// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// the quick start guide's steps, and the hit test its buttons use. kept
// apart from Tour.tsx so a check can read them without a window
import { keys } from "./keys";
import { t } from "./i18n";

export type Callout = { target: string; title: string; body: string };
// `also`: further parts of the window lit with the target, each with its
// own arrow from the note (the Library button and the Folders panel)
export type Step = { target?: string; also?: string[]; title: string; body: string; callouts?: Callout[] };

export const STEPS: Step[] = [
  { title: t("dialogs.tour.welcome.title"),
    body: t("dialogs.tour.welcome.body") },
  { target: "import", title: t("dialogs.tour.import.title"),
    body: keys(t("dialogs.tour.import.body")) },
  { target: "stats", title: t("dialogs.tour.stats.title"),
    body: t("dialogs.tour.stats.body") },
  { target: "library", also: ["folders"], title: t("dialogs.tour.library.title"),
    body: keys(t("dialogs.tour.library.body")) },
  { target: "find", title: t("dialogs.tour.find.title"),
    body: t("dialogs.tour.find.body") },
  { target: "library", title: t("dialogs.tour.cull.title"),
    body: keys(t("dialogs.tour.cull.body")) },
  { target: "edit", title: t("dialogs.tour.edit.title"),
    body: keys(t("dialogs.tour.edit.body")),
    callouts: [
      { target: "adjust", title: t("dialogs.tour.adjust.title"),
        body: t("dialogs.tour.adjust.body") },
      { target: "crop", title: t("dialogs.tour.crop.title"),
        body: t("dialogs.tour.crop.body") },
      { target: "masks", title: t("dialogs.tour.masks.title"),
        body: t("dialogs.tour.masks.body") },
      { target: "heal", title: t("dialogs.tour.heal.title"),
        body: t("dialogs.tour.heal.body") },
      { target: "zones", title: t("dialogs.tour.zones.title"),
        body: t("dialogs.tour.zones.body") },
      { target: "film", title: t("dialogs.tour.film.title"),
        body: t("dialogs.tour.film.body") },
      { target: "negative", title: t("dialogs.tour.negative.title"),
        body: t("dialogs.tour.negative.body") },
      { target: "presets", title: t("dialogs.tour.presets.title"),
        body: t("dialogs.tour.presets.body") },
    ] },
  { target: "advanced-mode", title: t("dialogs.tour.advanced.title"),
    body: t("dialogs.tour.advanced.body") },
  { target: "send-out", title: t("dialogs.tour.sendOut.title"),
    body: t("dialogs.tour.sendOut.body"),
    callouts: [
      { target: "export", title: t("dialogs.tour.export.title"),
        body: keys(t("dialogs.tour.export.body")) },
      { target: "print", title: t("dialogs.tour.print.title"),
        body: keys(t("dialogs.tour.print.body")) },
    ] },
  { target: "prefs", title: t("dialogs.tour.preferences.title"),
    body: keys(t("dialogs.tour.preferences.body")) },
];

// the tour's own buttons, found by where a pointer is rather than by what
// received the event: a click that another layer, a pointer capture or a
// re-render took from its target still reaches them
export type ActBox = { act: string; left: number; top: number; right: number; bottom: number };
export function actAt(boxes: ActBox[], x: number, y: number): string | null {
  const hit = boxes.find((b) => x >= b.left && x <= b.right && y >= b.top && y <= b.bottom);
  return hit ? hit.act : null;
}
