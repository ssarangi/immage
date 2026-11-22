// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Statistics harness's stand-in for the Rust commands. It computes nothing: it serves the REAL
// backend's replies, recorded by `cargo test dump_harness_fixtures -- --ignored` (stats/tests.rs) into
// stats-fixtures/{gt,big}.json by running stats.rs over the generated ground-truth library (`gt`: 400 photos)
// and over 117 copies of it (`big`: about 47,000). Each reply goes through the page's own normaliser
// (statsNormalize.ts), as a reply from the command does. A request with no recorded reply is an error
// shown on the page (and listed in window.__missingFixtures), so the set recorded is a visible limit.
// Only the coverage block is adjusted per ?cov= (indexing, paused, ready), as it is runtime state.
import type { StatsApi } from "../src/statsApi";
import { normalizeReply } from "../src/statsNormalize";
import type { Coverage, PhotoRef, StatsFilter, StatsRequest, StatsResponse } from "../src/statsTypes";
import type * as B from "../src/bindings";

type Fixture = { request: unknown; reply: B.StatsReply };

const files = import.meta.glob("./stats-fixtures/*.json", { import: "default" }) as Record<string, () => Promise<Fixture[]>>;
const thumbs = import.meta.glob("./stats-photos/*.jpg", { eager: true, query: "?url", import: "default" }) as Record<string, string>;
const real = Object.values(import.meta.glob("./stats-real/*.jpg", { eager: true, query: "?url", import: "default" }) as Record<string, string>);
// the six photos of the design mockup stand in for the library's thumbnails (the synthetic ones are procedural noise); ?photos=synthetic uses those
const hash = (s: string) => [...s].reduce((a, c) => (a * 31 + c.charCodeAt(0)) >>> 0, 7);
const useReal = new URLSearchParams(location.search).get("photos") !== "synthetic";
const thumbOf = (path: string) => useReal ? real[hash(path) % real.length] : thumbs[`./stats-photos/${path.replace("/photos-root/", "").replaceAll("/", "__")}`] ?? real[hash(path) % real.length];

/** the request as the same JSON whoever wrote it: no nulls, empty lists, false flags, etag or year mode; keys sorted */
export function canonical(v: unknown): string {
  const clean = (x: unknown): unknown => {
    if (Array.isArray(x)) return x.map(clean);
    if (x && typeof x === "object") {
      const o: Record<string, unknown> = {};
      for (const k of Object.keys(x).sort()) {
        const y = (x as Record<string, unknown>)[k];
        if (y === null || y === undefined || y === false || (Array.isArray(y) && !y.length) || k === "etag" || k === "yearMode") continue;
        o[k] = clean(y);
      }
      return o;
    }
    return x;
  };
  const r = clean(v) as { sections?: string[] };
  r.sections = [...(r.sections ?? [])].sort();
  return JSON.stringify(r);
}

export interface MockOptions { lib: string; state: string; cov: string; delayMs: number }

const cache = new Map<string, Promise<Map<string, B.StatsReply>>>();
function load(lib: string) {
  let p = cache.get(lib);
  if (!p) {
    const f = files[`./stats-fixtures/${lib}.json`];
    p = f ? f().then((list) => new Map(list.map((e) => [canonical(e.request), e.reply]))) : Promise.reject(new Error(`no fixtures for library "${lib}"`));
    cache.set(lib, p);
  }
  return p;
}

export function createMockApi(o: MockOptions): StatsApi & { lastGrid: { ids: string[] } | null } {
  const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
  const missing: string[] = [];
  (window as unknown as { __missingFixtures: string[] }).__missingFixtures = missing;
  const coverage = (c: Coverage): Coverage => {
    const total = c.total ?? c.indexed;
    if (o.state === "firstrun") return { indexed: 0, total: null, pending: 0, roots: 0, pendingRoots: [], paused: false, maxAgeSec: 0, state: "no-roots" };
    if (o.cov === "ready") return { ...c, roots: 23, maxAgeSec: 3 };
    const pending = Math.round(total * 0.028);
    const left = { ...c, total: total + pending, pending, roots: 23, maxAgeSec: 3 };
    return o.cov === "paused" ? { ...left, paused: true, state: "paused", pendingRoots: [{ folder: "/a", pending: 900 }] }
      : { ...left, state: "indexing", pendingRoots: [{ folder: "/a", pending: 900 }, { folder: "/b", pending: 400 }] };
  };
  const answer = async (req: StatsRequest): Promise<StatsResponse> => {
    const fixtures = await load(o.lib);
    const key = canonical(req);
    const raw = fixtures.get(key);
    if (!raw) { missing.push(key); throw new Error(`no recorded reply for ${key}`); }
    // a copy: the page's normaliser edits what it is given, as it does a command's fresh reply
    const reply = normalizeReply(JSON.parse(JSON.stringify(raw)) as B.StatsReply) as StatsResponse;
    return { ...reply, coverage: coverage(reply.coverage) };
  };
  const api = {
    lastGrid: null as { ids: string[] } | null,
    async query(req: StatsRequest): Promise<StatsResponse> {
      if (o.state === "loading") return new Promise(() => {});
      if (o.delayMs) await sleep(o.delayMs);
      if (o.state === "error") throw new Error("database is locked: could not read search-cache.sqlite");
      return answer(req);
    },
    async coverage(): Promise<Coverage> {
      return (await answer({ scope: { kind: "library" }, filter: { minRating: 4, onlyTop: false }, sections: ["overview", "cameras", "lenses"] })).coverage;
    },
    async openInGrid(scope: StatsRequest["scope"], filter: StatsFilter) {
      const n = (await answer({ scope, filter, sections: ["overview", "cameras", "lenses"] }).catch(() => null))?.overview?.photos ?? 0;
      const ids = Array.from({ length: Math.min(n, 20_000) }, (_, i) => String(2 ** 40 + i + 1));
      api.lastGrid = { ids };
      return { ids };
    },
    onDirty: () => () => {},
    async thumb(photo: PhotoRef) { return thumbOf(photo.path) ?? null; },
    async setPaused() {},
  };
  return api as StatsApi & { lastGrid: { ids: string[] } | null };
}
