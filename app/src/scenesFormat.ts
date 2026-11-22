// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Scenes' times and day names, in the display language. Rust sends the camera's
// wall-clock time as milliseconds "as if UTC", so every format here is told UTC:
// no zone is ever applied, whatever the computer's own.
import { formatDate } from "./i18n";

const UTC = { timeZone: "UTC" } as const;
export const clockText = (ms: number) => formatDate(ms, { ...UTC, hour: "numeric", minute: "2-digit" });
export const clockSecondsText = (ms: number) => formatDate(ms, { ...UTC, hour: "numeric", minute: "2-digit", second: "2-digit" });
export const dayText = (ms: number) => formatDate(ms, { ...UTC, weekday: "long", day: "numeric", month: "long", year: "numeric" });

// "7:10:04.3 AM": the clock with the camera's tenth of a second, when it recorded one
export function preciseClock(takenAt: string, ms: number): string {
  const base = clockSecondsText(ms);
  const fraction = /\.(\d)/.exec(takenAt.slice(19))?.[1];
  if (!fraction) return base;
  const parts = new Intl.DateTimeFormat(undefined, { ...UTC, second: "numeric" }).formatToParts(ms);
  const seconds = parts.find((p) => p.type === "second")?.value;
  return seconds ? base.replace(new RegExp(`(${seconds})(?!.*\\d${seconds})`), `$1.${fraction}`) : base;
}
