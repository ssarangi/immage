// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Display text for Statistics: numbers, chip labels, bucket names. Units stay Latin ("mm", "f/");
// counts, months and percentages follow the language (Intl), everything else is a catalog key.
import { formatDate, formatNumber, t } from "./i18n";
import { yearRuns } from "./statsMath";
import type { Chip } from "./statsModel";
import { FOCAL_BUCKETS, type ColorLabel, type StatsResponse } from "./statsTypes";

export const nf = (n: number) => formatNumber(n);
export const pct = (fraction: number, digits = 1) => formatNumber(fraction, { style: "percent", minimumFractionDigits: digits, maximumFractionDigits: digits });
export const pct0 = (fraction: number) => formatNumber(fraction, { style: "percent", maximumFractionDigits: 0 });
export const photosText = (n: number) => t("stats.photos", { n, count: nf(n) });
export const star = (n: number) => `★ ${n}`;
export const starsAtLeast = (n: number) => `★ ≥ ${n}`;
export const stars = (n: number) => "★".repeat(Math.max(0, Math.min(5, n)));

export const monthName = (m: number, style: "long" | "short" = "long") => formatDate(new Date(2024, m - 1, 15), { month: style });
export const hourText = (h: number) => `${String(h).padStart(2, "0")}:00`;

/** `**bold**` in a catalog string, as bold parts. */
export function richParts(text: string): { text: string; bold: boolean }[] {
  return text.split("**").map((part, i) => ({ text: part, bold: i % 2 === 1 })).filter((p) => p.text);
}

export const focalName = (id: string) => t(`stats.focal.${id}`);
export const focalRange = (id: string, defs?: StatsResponse["defs"]) => {
  const d = (defs?.focal ?? FOCAL_BUCKETS).find((b) => b.id === id);
  if (!d) return "";
  return d.toMm === null ? `${d.fromMm}+` : `${d.fromMm}–${d.toMm}`;
};

const LABEL_NAMES: Record<ColorLabel, string> = { red: "red", yellow: "yellow", green: "green", blue: "blue", purple: "purple" };

export interface ChipText { kind: string; text: string }
/** What a chip says: its facet ("Focal") and the choice ("Normal 35-69 mm"). */
export function chipText(chip: Chip, defs?: StatsResponse["defs"]): ChipText {
  if (chip.dim === "onlyTop") return { kind: t("stats.dim.rating"), text: t("stats.chip.onlyTop") };
  const v = chip.values[0];
  switch (chip.dim) {
    case "years": {
      const runs = yearRuns(chip.values.map(Number));
      return { kind: t("stats.dim.years"), text: runs.map(([a, b]) => (a === b ? String(a) : `${a}–${b}`)).join(", ") };
    }
    case "months": return { kind: t("stats.dim.months"), text: monthName(Number(v)) };
    case "hours": return { kind: t("stats.dim.hours"), text: `${hourText(Number(v))}–${String(Number(v)).padStart(2, "0")}:59` };
    case "weekdays": return { kind: t("stats.dim.weekdays"), text: formatDate(new Date(2024, 0, 1 + Number(v)), { weekday: "long" }) };
    case "focalBuckets": return { kind: t("stats.dim.focal"), text: `${focalName(String(v))} ${focalRange(String(v), defs)} mm` };
    case "apertureGroups": {
      const d = defs?.aperture.find((a) => a.id === v);
      return { kind: t("stats.dim.aperture"), text: d ? d.label : String(v) };
    }
    case "shutterRegimes": return { kind: t("stats.dim.shutter"), text: t(`stats.shutter.${v}`) };
    case "isoBins": return { kind: t("stats.dim.iso"), text: `ISO ${v}` };
    case "bodies": return { kind: t("stats.dim.body"), text: String(v).replace("|", " ") };
    case "makers": return { kind: t("stats.dim.maker"), text: String(v) };
    case "lenses": return { kind: t("stats.dim.lens"), text: String(v) };
    case "labels": return { kind: t("stats.dim.label"), text: t(`stats.label.${LABEL_NAMES[v as ColorLabel] ?? "red"}`) };
    case "place": return { kind: t("stats.dim.place"), text: String(v) };
    case "fileTypes": return { kind: t("stats.dim.type"), text: String(v) };
    case "aspect": return { kind: t("stats.dim.aspect"), text: String(v) };
    default: return { kind: String(chip.dim), text: String(v) };
  }
}

/** "Sony ILCE-7M4" from a body key "Sony|ILCE-7M4"; a model that already starts with its maker is not given it twice. */
export function bodyName(key: string): string {
  const i = key.indexOf("|");
  if (i < 0) return key;
  const maker = key.slice(0, i), model = key.slice(i + 1);
  return !maker || model.toLowerCase().startsWith(maker.toLowerCase()) ? model : `${maker} ${model}`;
}

/** 1/250 for a fast shutter, 2 s for a long one. */
export function shutterText(sec: number | null): string {
  if (sec === null || sec <= 0) return "";
  if (sec >= 0.95) return `${Math.round(sec * 10) / 10} s`;
  return `1/${Math.round(1 / sec)}`;
}
/** "35 mm · f/2 · 1/320" from what a photo carries. */
export function setupText(p: { focalMm: number | null; fNumber: number | null; shutterSec: number | null; iso: number | null }, withIso = true): string {
  const parts = [p.focalMm !== null ? `${Math.round(p.focalMm)} mm` : "", p.fNumber !== null ? `f/${Math.round(p.fNumber * 10) / 10}` : "",
    shutterText(p.shutterSec), withIso && p.iso !== null ? `ISO ${Math.round(p.iso)}` : ""].filter(Boolean);
  return parts.join(" \u00b7 ");
}
