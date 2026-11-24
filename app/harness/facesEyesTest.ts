// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The closed-eye glyphs on a grid of thumbnails, on a fake backend: the 20 mounted photos are asked for after a pause in one
// call, the glyphs arrive without moving any cell, closed and half closed show (different shapes), open and unknown show
// nothing, a quick scroll past a window never asks for it, and with no eye model nothing is asked at all.
// verify/people-shift.test.mjs runs it (?view=eyes&shift=1).
type H = { eyeCalls: number[]; setFrom: (n: number) => void; setStatus: (eyes: boolean) => void };
const h = () => (window as unknown as { __h: H }).__h;
const wait = (ms: number) => new Promise<void>((done) => { setTimeout(done, ms); });
const $$ = (s: string) => Array.from(document.querySelectorAll<HTMLElement>(s));
const rects = () => $$(".thumb, .thumb-image").map((e) => JSON.stringify(e.getBoundingClientRect()));

export async function runEyesTest() {
  const out: Record<string, unknown> = { failures: [] as string[] };
  const fail = (m: string) => (out.failures as string[]).push(m);
  const before = rects();
  await wait(100);
  if (h().eyeCalls.length) fail("asked before the pause ended");
  await wait(500);
  out.firstCalls = [...h().eyeCalls];
  if (h().eyeCalls.length !== 1 || h().eyeCalls[0] !== 20) fail(`the first window was asked as ${JSON.stringify(h().eyeCalls)}`);
  const badges = $$(".eye-badge");
  const byN = (n: number) => document.querySelector(`.thumb[data-n="${n}"] .eye-badge`);
  out.badges = badges.length;
  // 3 and 6, 9, 12, 15, 18 are closed (not 9? 9 is a multiple of 3), 7 and 14 are half closed, 0 and 11 are unknown (multiples of 11)
  if (!byN(3)?.classList.contains("closed") || !byN(14)?.classList.contains("semi")) fail("closed and half closed glyphs are not where they belong");
  if (byN(1) || byN(11) || byN(0)) fail("an open or unknown photo shows a glyph");
  const shapes = new Set([byN(3)?.innerHTML, byN(14)?.innerHTML]);
  if (shapes.size !== 2) fail("closed and half closed look the same (colour must not be the only cue)");
  if (byN(3)?.getAttribute("aria-label") !== "Eyes closed" || byN(14)?.getAttribute("aria-label") !== "Eyes half closed") fail("the glyphs have no names");
  if (JSON.stringify(rects()) !== JSON.stringify(before)) fail("the glyphs moved a cell");
  // a quick scroll through three windows: only the last is asked for
  h().eyeCalls.length = 0;
  h().setFrom(20); await wait(60); h().setFrom(40); await wait(60); h().setFrom(60); await wait(700);
  out.afterScroll = [...h().eyeCalls];
  if (h().eyeCalls.length !== 1 || h().eyeCalls[0] !== 20) fail(`the scroll asked ${JSON.stringify(h().eyeCalls)}`);
  // no eye model: nothing shows and nothing is asked
  h().setStatus(false); await wait(100);
  h().eyeCalls.length = 0;
  h().setFrom(100); await wait(700);
  if ($$(".eye-badge").length || h().eyeCalls.length) fail("with no eye model something shows or is asked");
  out.ok = (out.failures as string[]).length === 0;
  return out;
}
