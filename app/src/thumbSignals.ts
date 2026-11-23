// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// What the page tells the Catalog's thumbnail job (catalog_thumbs.rs, `Signals`): that someone is
// using the app, so the job waits and leaves the cores and the disk to them, and which photos are on
// screen, so it makes those next. Both are sent only while a thumbnail job runs, so a library
// without one never opens the Catalog's database for them.
import { commands } from "./commands";
import { getCatalog } from "./catalogStore";

// how long the page must be left alone before the job goes on
export const QUIET_MS = 1500;
// the most often a changed set of photos on screen is sent
const HINT_MS = 300;

// replaceable, so a test needs no Catalog to run
export const probe = { running: () => Object.values(getCatalog().jobs).some((j) => !!j?.thumbs && !j.thumbs.paused) };
const running = () => probe.running();

let busy = false;
let quiet: ReturnType<typeof setTimeout> | undefined;
const send = (now: boolean) => {
  if (busy === now) return;
  busy = now;
  void commands.thumbnailsBusy(now).catch(() => {});
};

// a click, key, wheel turn or pointer move: the job waits until QUIET_MS after the last
export function noteUse() {
  if (!busy && !running()) return;
  send(true);
  clearTimeout(quiet);
  quiet = setTimeout(() => send(false), QUIET_MS);
}

// listens for the page's use, for as long as the page lives; returns what stops it
export function watchUse(target: Window = window): () => void {
  const events = ["pointerdown", "pointermove", "wheel", "keydown", "scroll"] as const;
  for (const e of events) target.addEventListener(e, noteUse, { capture: true, passive: true });
  return () => {
    for (const e of events) target.removeEventListener(e, noteUse, { capture: true });
    clearTimeout(quiet);
    send(false);
  };
}

let pending: string[] | null = null;
let hinted = "";
let hintTimer: ReturnType<typeof setTimeout> | undefined;
// the photos on screen, the most wanted first; passed on at most every HINT_MS
export function hintOnScreen(paths: string[]) {
  pending = paths;
  if (hintTimer || !running()) return;
  hintTimer = setTimeout(() => {
    hintTimer = undefined;
    const now = pending;
    pending = null;
    if (!now || !running()) return;
    const key = now.join("\n");
    if (key === hinted) return;
    hinted = key;
    void commands.thumbnailsHint(now).catch(() => {});
  }, HINT_MS);
}
