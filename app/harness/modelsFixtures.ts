// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Fixture states of the AI models group for the dev harness (models.tsx) and its
// no-layout-shift test (modelsShift.ts). Dev only; the production build never includes harness/.
import type { KeywordsSummary, PackInfo, TagCount } from "../src/bindings";
import type { WorkInput } from "../src/aiPacks";
import type { Dialog } from "../src/AiModelsPrefs";

const CLIP_BYTES = 607_974_672;
export const CLIP: PackInfo = {
  id: "clip", name: "Smart search (CLIP)", license: "MIT", license_url: "https://opensource.org/license/mit", source_host: "huggingface.co",
  bytes: CLIP_BYTES, bytes_on_disk: 0, bytes_left: CLIP_BYTES, files: 7, files_ok: 0, state: "notInstalled", checking: false, error: null,
  version: "6b7c523703ae26e39be4b03774966f8571c7f523", update_version: null, update_bytes: null, verified_at: null, free_bytes: 212_000_000_000,
  unsupported_reason: null, analyses: ["smartSearch"], progress: null, download_consent: false, analysis_consent: false, folder: "/home/you/.local/share/immage/clip-models",
};
const verified = Date.UTC(2026, 10, 14, 12) / 1000;
const progress = (pct: number, phase: "download" | "verify" = "download") => ({
  id: "clip" as const, done: Math.ceil(CLIP_BYTES * pct / 100), total: CLIP_BYTES, bytes_per_sec: 6_200_000, eta_s: Math.round(CLIP_BYTES * (1 - pct / 100) / 6_200_000), phase,
});
const idle: WorkInput = { paused: false, enabled: true, installed: true, pending: 0, running: 0, active: false, analysed: 3402 };

export const RAM_BYTES = 1_849_925_269;
export const RAM: PackInfo = {
  id: "ram", name: "Auto-keywords (RAM++)", license: "Apache-2.0", license_url: "https://huggingface.co/xinyu1205/recognize-anything-plus-model", source_host: "huggingface.co",
  bytes: RAM_BYTES, bytes_on_disk: 0, bytes_left: RAM_BYTES, files: 3, files_ok: 0, state: "notInstalled", checking: false, error: null,
  version: "ce61dc6d6cc47769e69fc8b7ad7bfdfaf8c4e65b", update_version: null, update_bytes: null, verified_at: null, free_bytes: 212_000_000_000,
  unsupported_reason: null, analyses: ["keywords"], progress: null, download_consent: false, analysis_consent: false, folder: "/home/you/.local/share/immage/ram-models",
};
const ramInstalled: PackInfo = { ...RAM, state: "installed", bytes_on_disk: RAM_BYTES, bytes_left: 0, files_ok: 3, verified_at: verified, download_consent: true, analysis_consent: true };
const kw = (o: Partial<KeywordsSummary> = {}): KeywordsSummary => ({ photos: 1204, keyworded: 1190, keywords: 12418, threshold: 70, min_threshold: 40, max_threshold: 95,
  infer_ms: 1400, memory_mb: 2900, burst: true, copied: 212, blocked: null, health: "ok", failure: null, retry_in_s: null, failure_detail: null, failed: 0, ...o });

export type Fixture = { packs: PackInfo[]; work: WorkInput | null; smartOn: boolean; flash?: string; dialog?: Dialog;
  keywordsOn?: boolean; keywordsWork?: WorkInput | null; keywords?: KeywordsSummary | null; aiTags?: TagCount[] };
const installed: PackInfo = { ...CLIP, state: "installed", bytes_on_disk: CLIP_BYTES, bytes_left: 0, files_ok: 7, verified_at: verified, download_consent: true, analysis_consent: true };

export const STATES: Record<string, Fixture> = {
  notinstalled: { packs: [CLIP], work: { ...idle, installed: false, enabled: false, pending: null, analysed: 0 }, smartOn: false },
  diskfull: { packs: [{ ...CLIP, free_bytes: 900_000_000, error: { kind: "diskFull", detail: "x" } }], work: null, smartOn: false },
  downloading: { packs: [{ ...CLIP, state: "downloading", bytes_left: 0, progress: progress(34), download_consent: true }], work: null, smartOn: false },
  downloading1: { packs: [{ ...CLIP, state: "downloading", progress: progress(1), download_consent: true }], work: null, smartOn: false },
  downloading99: { packs: [{ ...CLIP, state: "downloading", progress: progress(99), download_consent: true }], work: null, smartOn: false },
  verifying: { packs: [{ ...CLIP, state: "downloading", progress: progress(100, "verify"), download_consent: true }], work: null, smartOn: false },
  installed: { packs: [installed], work: { ...idle, pending: 616, running: 1, active: true, analysed: 1204 }, smartOn: true },
  installedoff: { packs: [installed], work: { ...idle, enabled: false, pending: null }, smartOn: false },
  installedpaused: { packs: [installed], work: { ...idle, paused: true, pending: 616, analysed: 1204 }, smartOn: true },
  installedidle: { packs: [installed], work: idle, smartOn: true },
  checking: { packs: [{ ...installed, verified_at: null, checking: true }], work: idle, smartOn: true },
  checked: { packs: [installed], work: idle, smartOn: true, flash: "All 7 files match. Checked just now." },
  repair: { packs: [{ ...installed, state: "needsRepair", files_ok: 6, verified_at: null, error: { kind: "hashMismatch", detail: "onnx/vision_model.onnx failed pinned integrity check" } }], work: { ...idle, pending: null }, smartOn: true },
  offline: { packs: [{ ...CLIP, state: "stopped", bytes: 1_700_000_000, bytes_left: 1_100_000_000, bytes_on_disk: 600_000_000, error: { kind: "offline", detail: "x" }, download_consent: true }], work: null, smartOn: false },
  stopped: { packs: [{ ...CLIP, state: "stopped", bytes_left: 300_000_000, bytes_on_disk: 307_000_000, download_consent: true }], work: null, smartOn: false },
  update: { packs: [{ ...installed, state: "updateAvailable", update_bytes: 700_000_000 }], work: idle, smartOn: true },
  unsupported: { packs: [{ ...CLIP, state: "unsupported", unsupported_reason: "This processor lacks a feature the model needs (AVX2). The files are fine, so nothing to repair. Remove them to free 608 MB.", bytes_on_disk: CLIP_BYTES }], work: null, smartOn: false },
  consent: { packs: [CLIP], work: null, smartOn: false, dialog: { kind: "consent", id: "clip" } },
  // Auto-keywords (RAM++): second card, compared card against card by modelsShift.ts
  kwnotinstalled: { packs: [installed, RAM], work: idle, smartOn: true, keywordsOn: false, keywords: kw({ photos: 0, keyworded: 0, keywords: 0, infer_ms: null, memory_mb: null }) },
  kwdownloading: { packs: [installed, { ...RAM, state: "downloading", download_consent: true,
    progress: { id: "ram", done: 640_000_000, total: RAM_BYTES, bytes_per_sec: 9_100_000, eta_s: 133, phase: "download" } }], work: idle, smartOn: true, keywordsOn: false,
    keywords: kw({ photos: 0, keyworded: 0, keywords: 0, infer_ms: null, memory_mb: null }) },
  kwoff: { packs: [installed, ramInstalled], work: idle, smartOn: true, keywordsOn: false, keywordsWork: { ...idle, enabled: false, pending: null, analysed: 0 },
    keywords: kw({ photos: 0, keyworded: 0, keywords: 0, infer_ms: null, memory_mb: null }) },
  kwworking: { packs: [installed, ramInstalled], work: idle, smartOn: true, keywordsOn: true, keywordsWork: { ...idle, pending: 616, running: 1, active: true, analysed: 1204 }, keywords: kw() },
  kwidle: { packs: [installed, ramInstalled], work: idle, smartOn: true, keywordsOn: true, keywordsWork: { ...idle, pending: 0, analysed: 1820 }, keywords: kw({ photos: 1820, keyworded: 1790, keywords: 18730 }) },
  kwpaused: { packs: [installed, ramInstalled], work: idle, smartOn: true, keywordsOn: true, keywordsWork: { ...idle, paused: true, pending: 616, analysed: 1204 }, keywords: kw({ threshold: 85 }) },
  kwrepair: { packs: [installed, { ...ramInstalled, state: "needsRepair", files_ok: 2, verified_at: null, error: { kind: "hashMismatch", detail: "model.onnx failed pinned integrity check" } }],
    work: idle, smartOn: true, keywordsOn: true, keywordsWork: { ...idle, pending: null }, keywords: kw() },
  kwblocked: { packs: [installed, ramInstalled], work: idle, smartOn: true, keywordsOn: true, keywordsWork: { ...idle, pending: 616, analysed: 1204 }, keywords: kw({ blocked: "level" }) },
  // a failure: blocked after repeated tries, backing off before the next, and photos skipped for their own fault
  kwstuck: { packs: [installed, ramInstalled], work: idle, smartOn: true, keywordsOn: true, keywordsWork: { ...idle, pending: 413, analysed: 0 },
    keywords: kw({ photos: 0, keyworded: 0, keywords: 0, health: "blocked", failure: "model", failure_detail: "RAM++ model is unavailable; verify the Auto-keywords model" }) },
  kwretry: { packs: [installed, ramInstalled], work: idle, smartOn: true, keywordsOn: true, keywordsWork: { ...idle, pending: 413, analysed: 0 },
    keywords: kw({ photos: 0, keyworded: 0, keywords: 0, health: "retrying", failure: "crash", retry_in_s: 30 }) },
  kwfailed: { packs: [installed, ramInstalled], work: idle, smartOn: true, keywordsOn: true, keywordsWork: { ...idle, pending: 616, running: 1, active: true, analysed: 1204 }, keywords: kw({ failed: 3 }) },
  kwburstoff: { packs: [installed, ramInstalled], work: idle, smartOn: true, keywordsOn: true, keywordsWork: { ...idle, pending: 0, analysed: 1820 }, keywords: kw({ burst: false, copied: 0 }) },
  kwreview: { packs: [installed, ramInstalled], work: idle, smartOn: true, keywordsOn: true, keywordsWork: { ...idle, pending: 0, analysed: 1820 }, keywords: kw(),
    dialog: { kind: "review" }, aiTags: [{ name: "sky", photos: 982 }, { name: "beach", photos: 640 }, { name: "sea", photos: 612 }, { name: "dog", photos: 188 }, { name: "sunset", photos: 91 }] },
  kwreviewempty: { packs: [installed, ramInstalled], work: idle, smartOn: true, keywordsOn: true, keywordsWork: { ...idle, pending: 0, analysed: 0 },
    keywords: kw({ photos: 0, keyworded: 0, keywords: 0 }), dialog: { kind: "review" }, aiTags: [] },
  remove: { packs: [installed], work: idle, smartOn: true, dialog: { kind: "remove", id: "clip" } },
};
// the states the shift test compares card against card
// the Auto-keywords card's states, compared with the same boxes (data-pack="ram")
export const KW_STATES = ["kwnotinstalled", "kwdownloading", "kwoff", "kwworking", "kwidle", "kwpaused", "kwrepair", "kwblocked", "kwstuck", "kwretry", "kwfailed", "kwburstoff"];
export const CARD_STATES = ["notinstalled", "diskfull", "downloading1", "downloading", "downloading99", "verifying", "installed", "installedoff",
  "installedpaused", "checking", "checked", "repair", "offline", "stopped", "update", "unsupported"];
