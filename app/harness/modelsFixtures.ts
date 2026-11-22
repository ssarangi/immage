// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// Fixture states of the AI models group for the dev harness (models.tsx) and its
// no-layout-shift test (modelsShift.ts). Dev only; the production build never includes harness/.
import type { PackInfo } from "../src/bindings";
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

export type Fixture = { packs: PackInfo[]; work: WorkInput | null; smartOn: boolean; flash?: string; dialog?: Dialog };
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
  remove: { packs: [installed], work: idle, smartOn: true, dialog: { kind: "remove", id: "clip" } },
};
// the states the shift test compares card against card
export const CARD_STATES = ["notinstalled", "diskfull", "downloading1", "downloading", "downloading99", "verifying", "installed", "installedoff",
  "installedpaused", "checking", "checked", "repair", "offline", "stopped", "update", "unsupported"];
