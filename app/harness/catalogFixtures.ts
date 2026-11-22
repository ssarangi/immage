// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Fixture states for the Catalog harness: made-up sources (nothing here names a real path or host),
// and one focused source whose state changes. Every number is illustrative.
import type { MetaProgress, Progress, SourceStatus, ThumbProgress, CatalogStatus } from "../src/bindings";
import type { Ended, Jobs, MetaDone, ScanDone, ThumbDone } from "../src/catalogModel";

const src = (id: number, name: string, over: Partial<SourceStatus["source"]> = {}, online = true): SourceStatus => ({
  source: {
    id, name, kind: "nas", volume_key: "smb:fake/photos", fs_type: "cifs", host: "fake-nas", host_hash: null, share: "Photos",
    mount_path: "/fake/nas", rel_root: name.replace(/ /g, "_"), enabled: true, treat_as_network: null, scan_policy: "manual",
    include_subfolders: true, dir_skip: false, state: "complete", error: null, last_scan_start: 1, last_scan_end: 2, last_ok: 2,
    created_at: 1, photos: 12060, missing: 0, ...over,
  },
  availability: online
    ? { state: "Online", detail: { mount_point: "/fake/nas", abs_root: `/fake/nas/${name.replace(/ /g, "_")}` } }
    : { state: "Offline", detail: { reason: "not mounted" } },
});

export const SOURCES = (first: Partial<SourceStatus["source"]> = {}, firstOnline = true): SourceStatus[] => [
  src(1, "Archive 2012-2019", { photos: 98200, ...first }, firstOnline),
  src(2, "2023 Japan", { photos: 4210 }),
  src(3, "Wildlife", { photos: 38771, state: "incomplete", error: "1 folder did not answer in time" }),
  src(4, "EOS_DIGITAL", { kind: "drive", volume_key: "uuid:7C3E-1A09", fs_type: "exfat", host: null, share: null, mount_path: "/fake/sd", rel_root: "", photos: 3448 }, false),
  src(5, "Family", { photos: 0, state: "new", last_scan_end: null, last_ok: null }),
];

const scan = (over: Partial<Progress> = {}): Progress => ({
  source_id: 1, job_id: 11, run_id: 1, phase: "finding", dirs_done: 310, dirs_queued: 900, found: 12400, new: 12400, changed: 0, unchanged: 0,
  errors: 0, elapsed_s: 52, rate_per_s: 4200, eta_s: null, current_dir: "2016/06", paused: false, ...over,
});
const meta = (over: Partial<MetaProgress> = {}): MetaProgress => ({
  source_id: 1, job_id: 12, phase: "reading", total: 98200, done: 41860, failed: 3, unparsed: 1, skipped: 0, bytes_read: 1 << 30, elapsed_s: 300,
  rate_per_s: 118, eta_s: [360, 720], workers: 4, p95_ms: 90, paused: false, ...over,
});
const thumbs = (over: Partial<ThumbProgress> = {}): ThumbProgress => ({
  source_id: 1, job_id: 13, phase: "making", total: 98200, done: 21300, failed: 0, deferred: 0, skipped: 0, bytes_read: 1 << 29, elapsed_s: 300,
  rate_per_s: 60, eta_s: [900, 1500], workers: 2, decoders: 2, p95_ms: 60, paused: false, yielding: false, ...over,
});
const sd = (over: Partial<ScanDone> = {}): ScanDone => ({
  source_id: 1, state: "complete", elapsed_s: 52, found: 98200, folders: 1240, new: 98200, changed: 0, unchanged: 0, missing: 0, unreadable: 0,
  incomplete_dirs: [], error: null, ...over,
});
const md = (over: Partial<MetaDone> = {}): MetaDone => ({ source_id: 1, state: "complete", elapsed_s: 700, total: 98200, done: 98106, failed: 90, unparsed: 4, error: null, ...over });
const td = (over: Partial<ThumbDone> = {}): ThumbDone => ({ source_id: 1, state: "complete", elapsed_s: 100, total: 98106, done: 98106, failed: 0, deferred: 0, thumbnail_bytes: 4 << 30, error: null, ...over });

export interface Fixture {
  sources: SourceStatus[];
  jobs: Record<number, Jobs>;
  ended: Record<number, Ended>;
  /** the focused source's card is tall (running, done, problem) or short (offline, idle) */
  tall: boolean;
}

const base = (first: Partial<SourceStatus["source"]> = {}, online = true) => SOURCES(first, online);
const run = (j: Jobs, first: Partial<SourceStatus["source"]> = { state: "indexing", last_scan_end: null, photos: 0 }): Fixture => ({
  sources: base(first), jobs: { 1: { pipeline: true, ...j } }, ended: {}, tall: true,
});
const done = (e: Ended, first: Partial<SourceStatus["source"]> = {}, online = true, tall = true): Fixture =>
  ({ sources: base(first, online), jobs: {}, ended: { 1: { at: 1, ...e } }, tall });

export const STATES: Record<string, Fixture> = {
  finding: run({ scan: scan() }),
  findingfast: run({ scan: scan({ found: 98200, dirs_done: 1240, dirs_queued: 1240 }) }),
  reading: run({ scan: undefined, meta: meta() }, { state: "indexing", photos: 98200 }),
  reading1: run({ meta: meta({ done: 120, eta_s: null, rate_per_s: null }) }, { state: "indexing", photos: 98200 }),
  reading99: run({ meta: meta({ done: 97000, eta_s: [20, 30] }) }, { state: "indexing", photos: 98200 }),
  thumbs: run({ thumbs: thumbs() }, { state: "indexing", photos: 98200 }),
  yielding: run({ thumbs: thumbs({ yielding: true }) }, { state: "indexing", photos: 98200 }),
  paused: run({ meta: meta({ paused: true, eta_s: null }) }, { state: "indexing", photos: 98200 }),
  done: done({ scan: sd(), meta: md({ failed: 0, unparsed: 0, done: 98200 }), thumbs: td() }, { photos: 98200 }),
  doneunreadable: done({ scan: sd(), meta: md(), thumbs: td() }, { photos: 98200 }),
  offlinejob: done({ scan: sd({ state: "offline", error: "The source is not reachable." }) }, { photos: 41000 }),
  failed: done({ scan: sd({ state: "failed", error: "the catalog writer stopped" }) }, { photos: 0 }),
  partial: done({ scan: sd({ state: "incomplete", incomplete_dirs: ["Raw/2021-08", "Raw/2021-09"] }) }, { photos: 98200 }),
  diskfull: done({ scan: sd(), meta: md(), thumbs: td({ state: "disk_full" }) }, { photos: 98200 }),
  offline: { sources: base({ photos: 98200 }, false), jobs: {}, ended: {}, tall: false },
  idle: { sources: base({ photos: 98200 }), jobs: {}, ended: {}, tall: false },
  never: { sources: base({ photos: 0, state: "new", last_scan_end: null, last_ok: null }), jobs: {}, ended: {}, tall: false },
  empty: { sources: [], jobs: {}, ended: {}, tall: false },
};

// the states whose card is the tall variant, and the short ones, as the shift test groups them
export const TALL = Object.keys(STATES).filter((k) => STATES[k].tall);
export const SHORT = ["offline", "idle", "never"];

export const STATUS: CatalogStatus = {
  path: "/fake/catalog.sqlite", schema: 2, bytes: 1_150_000_000, photos: 283254, missing: 0, sources: 5, online: 4, offline: 1, jobs_running: 0,
  metadata: { done: 283000, pending: 0, failed: 200, unparsed: 54 },
  thumbnails: { done: 282000, pending: 0, failed: 0, deferred: 0, kept_missing: 0, bytes: 18_400_000_000 },
};
