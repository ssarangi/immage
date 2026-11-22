// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The Statistics view's state while the app is open (module-level, like findModel.ts's callers keep
// Find's), and the hook that turns it into data: one request for the sections on screen, the old
// numbers kept (dimmed) while a new request runs, a retry, the 2 s coverage poll while indexing, and
// a re-query one second after the search cache changes.
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { settings } from "./settings";
import { buildRequest, clampRating, DEFAULT_MIN_RATING, initialState, sameRequest, type StatsState } from "./statsModel";
import type { StatsApi } from "./statsApi";
import type { Coverage, StatsRequest, StatsResponse, StatsSection } from "./statsTypes";

const RATING_KEY = "immage.stats.minRating";
export function loadMinRating(): number {
  try { return clampRating(Number(settings.get(RATING_KEY) ?? DEFAULT_MIN_RATING)); } catch { return DEFAULT_MIN_RATING; }
}
export function storeMinRating(n: number) {
  try { settings.set(RATING_KEY, String(clampRating(n))); } catch { /* not kept */ }
}

let state: StatsState | null = null;
const listeners = new Set<() => void>();
const current = (): StatsState => (state ??= initialState(loadMinRating()));

export function getStatsState(): StatsState { return current(); }
export function setStatsState(next: StatsState | ((s: StatsState) => StatsState)) {
  const prev = current();
  const value = typeof next === "function" ? next(prev) : next;
  if (value === prev) return;
  if (value.filter.minRating !== prev.filter.minRating) storeMinRating(value.filter.minRating);
  state = value;
  listeners.forEach((l) => l());
}
/** For the tests and the harness: a fresh state, not the one the last visit left. */
export function resetStatsState(next?: StatsState) { state = next ?? null; listeners.forEach((l) => l()); }
export function useStatsState(): [StatsState, typeof setStatsState] {
  const s = useSyncExternalStore((cb) => { listeners.add(cb); return () => { listeners.delete(cb); }; }, current, current);
  return [s, setStatsState];
}

// where the view was scrolled to, kept while the app is open
export const viewMemory = { scrollTop: 0 };

export type Phase = "loading" | "ready" | "refreshing" | "error";
export interface StatsData {
  phase: Phase;
  /** the last answer, kept while a new request runs */
  response: StatsResponse | null;
  coverage: Coverage | null;
  error: string | null;
  reload(): void;
}

/** Sections S1 asks for; the next ones are added here as their charts arrive. */
export const SECTIONS_NOW: StatsSection[] = ["overview", "cameras", "lenses", "exposure", "time"];

const DIRTY_DELAY = 1000;
const POLL = 2000;

export function useStatsData(api: StatsApi, state: StatsState, sections: StatsSection[] = SECTIONS_NOW): StatsData {
  const [phase, setPhase] = useState<Phase>("loading");
  const [response, setResponse] = useState<StatsResponse | null>(null);
  const [coverage, setCoverage] = useState<Coverage | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [tick, setTick] = useState(0);
  const last = useRef<{ req: StatsRequest; res: StatsResponse } | null>(null);
  const req = buildRequest(state, sections);
  const reqKey = JSON.stringify(req);

  useEffect(() => {
    let alive = true;
    const request = JSON.parse(reqKey) as StatsRequest;
    const prev = last.current;
    // an unchanged request goes with the last answer's etag, so the backend can say it is current
    if (prev && sameRequest(prev.req, request)) request.etag = prev.res.etag;
    setPhase((p) => (p === "loading" && !prev ? "loading" : "refreshing"));
    setError(null);
    api.query(request).then((res) => {
      if (!alive) return;
      if ("notModified" in res) { setPhase("ready"); return; }
      last.current = { req: request, res };
      setResponse(res);
      setCoverage(res.coverage);
      setPhase("ready");
    }).catch((e) => {
      if (!alive) return;
      setError(String(e instanceof Error ? e.message : e));
      setPhase("error");
    });
    return () => { alive = false; };
  }, [api, reqKey, tick]);

  // the cache changed: ask again once things have been quiet for a second
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const off = api.onDirty(() => { clearTimeout(timer); timer = setTimeout(() => setTick((n) => n + 1), DIRTY_DELAY); });
    return () => { clearTimeout(timer); off(); };
  }, [api]);

  // indexing: the counts move every 2 s, the page itself is asked once when it is done
  const cov = coverage?.state;
  useEffect(() => {
    if (cov !== "indexing") return;
    let alive = true;
    const timer = setInterval(() => {
      api.coverage().then((c) => {
        if (!alive) return;
        setCoverage(c);
        if (c.state !== "indexing") setTick((n) => n + 1);
      }).catch(() => {});
    }, POLL);
    return () => { alive = false; clearInterval(timer); };
  }, [api, cov]);

  // a Refresh asks for fresh numbers whatever the etag says
  const reload = useCallback(() => { last.current = null; setTick((n) => n + 1); }, []);
  return { phase, response, coverage, error, reload };
}
