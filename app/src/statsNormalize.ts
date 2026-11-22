// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The boundary between the generated command types (bindings.ts) and the Statistics view's own types
// (statsTypes.ts). specta types every f64 as `number | null` (a NaN is null in JSON) and every
// `Option` as `T | null`, while the view reads clean numbers and optional blocks. Rust never sends null
// for a number it computed, so nothing is converted at run time except the Option keys listed here,
// which become absent exactly as the contract file says (`weekdays?`, `heat?`, `apply?`).
//
// The compile-time check below keeps the two sets of types in step: if Rust adds, renames or retypes a
// field the view does not know, `Loose<..>` stops accepting the generated type and `npm run build` fails here.
import type * as B from "./bindings";
import type { PlaceDetail, StatsResponse } from "./statsTypes";

type Loose<T> = T extends number ? number | null
  : T extends string | boolean | null | undefined ? T
  : T extends readonly unknown[] ? { [K in keyof T]: Loose<T[K]> }
  : T extends object ? { [K in keyof T]: Loose<T[K]> | (undefined extends T[K] ? null : never) } : T;

/** compile-time only: the generated reply is the view's type with numbers and optionals loosened */
export type Compatible = [
  (r: B.StatsResponse) => Loose<StatsResponse>,
  (r: B.PlaceDetail) => Loose<PlaceDetail>,
];
export const compatible = (c: Compatible) => c;
compatible([(r) => r, (r) => r]);

/** keys the contract declares optional but Rust sends as null (the others it skips) */
const OPTIONAL_KEYS = new Set(["timeline", "zoomUsage", "apertureByShutter", "weekdays", "monthByHour", "heat", "apply", "solo", "name"]);
/** the exception: PlaceDetail.name is `string | null`, present always */
const KEEP_NULL = new Set(["PlaceDetail.name"]);

const strip = (v: unknown, owner = ""): unknown => {
  if (Array.isArray(v)) { for (let i = 0; i < v.length; i++) strip(v[i], owner); return v; }
  if (v && typeof v === "object") {
    const o = v as Record<string, unknown>;
    for (const k of Object.keys(o)) {
      if (o[k] === null && OPTIONAL_KEYS.has(k) && !KEEP_NULL.has(`${owner}.${k}`)) delete o[k];
      else strip(o[k], k === "apply" ? "StatsFilter" : owner);
    }
    // an insight's filter arrives with every field, most of them null: keep only the ones that are set
    if (o.apply && typeof o.apply === "object") {
      const f = o.apply as Record<string, unknown>;
      for (const k of Object.keys(f)) if (f[k] === null) delete f[k];
    }
  }
  return v;
};

export function normalizeReply(raw: B.StatsReply): StatsResponse | { notModified: true } {
  if ("notModified" in raw) return { notModified: true };
  return strip(raw) as unknown as StatsResponse;
}

export function normalizePlace(raw: B.PlaceDetail): PlaceDetail {
  const p = raw as unknown as Record<string, unknown>;
  strip(p, "PlaceDetail");
  return raw as unknown as PlaceDetail;
}
