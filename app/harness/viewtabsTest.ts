// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The top strip's checks, run in the page by harness/viewtabs.tsx (?test=1) and read by
// verify/view-tabs-shift.test.mjs: the tabs never overlap the menus or leave the strip at 700 to
// 1600px (three tabs and four), nothing in the strip moves when the view changes, the labels
// become icons when the strip is narrow, the tablist is one tab stop that the arrow keys walk
// (skipping a disabled tab), every tab has a name and a shortcut tooltip, and no drag region
// covers a tab.
type H = { set: (p: { view?: "edit" | "grid" | "scenes"; canScenes?: boolean }) => void; width: (w: number) => void; four: (on: boolean) => void; badge: (on: boolean) => void };
const frames = (n = 3) => new Promise<void>((done) => { setTimeout(done, 25 * n); });
const R = (e: Element) => { const r = e.getBoundingClientRect(); return [r.left, r.top, r.width, r.height].map((n) => Math.round(n * 100) / 100); };
const q = <T extends Element>(s: string) => document.querySelector<T>(s)!;
const tabs = () => [...document.querySelectorAll<HTMLButtonElement>('[role="tab"]')];
const rtl = document.documentElement.dir === "rtl";
const fwd = rtl ? "ArrowLeft" : "ArrowRight";
const key = (k: string) => (document.activeElement as HTMLElement).dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true }));

export async function runViewTabsTest(h: H) {
 
  const fail: string[] = [];
  const geo: Record<string, unknown>[] = [];
  const strip = q(".titlebar");
  for (const four of [false, true]) {
    h.four(four); await frames();
    for (const w of [700, 900, 1280, 1600]) {
      h.width(w); await frames();
      const labels = tabs().map((t) => getComputedStyle(t.querySelector(".view-tab-label")!).display !== "none");
      const menus = document.querySelector(".menubar"), tl = q(".view-tabs"), sr = strip.getBoundingClientRect(), tr = tl.getBoundingClientRect();
      const mr = menus?.getBoundingClientRect();
      geo.push({ four, w, labels: labels[0], tabs: R(tl) });
      if (mr && tr.left < mr.right - 0.5 && tr.right > mr.left + 0.5) fail.push(`${four ? 4 : 3} tabs at ${w}: tabs (${tr.left}) overlap the menus (${mr.left}-${mr.right})`);
      if (tr.right > sr.right + 0.5 || tr.left < sr.left - 0.5) fail.push(`${four ? 4 : 3} tabs at ${w}: tabs leave the strip`);
      if (strip.scrollWidth > strip.clientWidth + 0.5 || document.documentElement.scrollWidth > innerWidth + 1) fail.push(`${w}: scrolls sideways`);
      if (w >= 1280 && !labels.every(Boolean)) fail.push(`${w}: labels should show`);
      if (w === 700 && menus && labels.some(Boolean)) fail.push(`${w}: labels should be icons`);
      // no drag region over a tab: the topmost element at a tab's centre is the tab (or its child), with no drag attribute
      for (const t of tabs()) {
        const r = t.getBoundingClientRect(), at = document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2);
        if (at?.closest('[role="tab"]') !== t) fail.push(`${w}: ${t.dataset.view} is covered by ${at?.className || at?.tagName}`);
        if (at?.closest("[data-tauri-drag-region]") && at.closest("[data-tauri-drag-region]") === at.closest('[role="tab"]')) fail.push(`${w}: ${t.dataset.view} is a drag region`);
      }
    }
  }
  h.four(false); h.width(1280); h.set({ canScenes: true }); await frames();
  // drag regions on the empty strip, the title and the tabs' box, not the tablist
  const box = q(".titlebar-tabs");
  for (const el of [strip, strip.firstElementChild!, box]) if (!el.hasAttribute("data-tauri-drag-region")) fail.push(`${el.className || el.tagName} lacks a drag region`);
  for (const el of [q(".view-tabs"), ...tabs(), ...(document.querySelector(".menubar") ? [q(".menubar")] : [])]) if (el.hasAttribute("data-tauri-drag-region")) fail.push(`${el.className || el.tagName} is a drag region`);
  // 2. nothing moves when the view changes
  const snap = () => ({ menus: document.querySelector(".menubar") ? R(q(".menubar")) : null, tabs: R(q(".view-tabs")), each: tabs().map(R), strip: R(strip) });
  h.set({ view: "edit" }); await frames();
  const base = JSON.stringify(snap());
  let shifts = 0;
  for (const view of ["grid", "scenes", "edit", "scenes", "grid"] as const) {
    h.set({ view }); await frames();
    if (JSON.stringify(snap()) !== base) { shifts++; fail.push(`the strip moved when the view became ${view}`); }
    const sel = tabs().filter((t) => t.getAttribute("aria-selected") === "true").map((t) => t.dataset.view);
    if (sel.join() !== view) fail.push(`aria-selected is ${sel} for ${view}`);
  }
  // 3. names and tooltips
  const list = q('[role="tablist"]');
  if (!list.getAttribute("aria-label")) fail.push("the tablist has no name");
  const names = tabs().map((t) => t.getAttribute("aria-label"));
  if (names.some((n) => !n) || new Set(names).size !== names.length) fail.push(`tab names: ${names}`);
  const titles = tabs().map((t) => t.title);
  if (!/G/.test(titles[1]) || !/(⇧|Shift\+)S/.test(titles[2])) fail.push(`tooltips lack the shortcut: ${titles}`);
  // 4. one tab stop, arrow keys, disabled skipped
  h.set({ view: "edit", canScenes: true }); await frames();
  const stops = () => tabs().filter((t) => t.tabIndex === 0).length;
  const tabStops = stops();
  tabs()[0].focus(); key(fwd); await frames();
  const afterRight = (document.activeElement as HTMLElement).dataset.view;
  const selectedAfter = tabs().find((t) => t.getAttribute("aria-selected") === "true")?.dataset.view;
  key("End"); await frames(); const afterEnd = (document.activeElement as HTMLElement).dataset.view;
  key("Home"); await frames(); const afterHome = (document.activeElement as HTMLElement).dataset.view;
  const hadFocusStops = stops();
  if (tabStops !== 1 || hadFocusStops !== 1) fail.push(`tab stops: ${tabStops}, ${hadFocusStops}`);
  if (afterRight !== "grid" || selectedAfter !== "grid") fail.push(`Arrow went to ${afterRight}, selected ${selectedAfter}`);
  if (afterEnd !== "scenes" || afterHome !== "edit") fail.push(`End ${afterEnd}, Home ${afterHome}`);
  h.set({ view: "grid", canScenes: false }); await frames();
  tabs()[1].focus(); key(fwd); await frames();
  const skipped = (document.activeElement as HTMLElement).dataset.view;
  if (skipped !== "grid") fail.push(`Arrow reached the disabled Scenes tab (${skipped})`);
  if (tabs()[2].getAttribute("aria-disabled") !== "true") fail.push("Scenes is not marked disabled");
  tabs()[2].click(); await frames();
  if (tabs().find((t) => t.getAttribute("aria-selected") === "true")?.dataset.view === "scenes") fail.push("a disabled tab was selected by click");
  // contrast of the idle label and the active bar against the strip, per theme (the theme in the URL)
  const lum = (c: string) => { const m = c.match(/[\d.]+/g)!.map(Number).slice(0, 3).map((v) => { v = c.startsWith("color(") ? v : v / 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }); return 0.2126 * m[0] + 0.7152 * m[1] + 0.0722 * m[2]; };
  const ratio = (a: string, b: string) => { const [x, y] = [lum(a), lum(b)].sort((p, q2) => q2 - p); return (x + 0.05) / (y + 0.05); };
  const idle = tabs().find((t) => t.getAttribute("aria-selected") !== "true" && t.getAttribute("aria-disabled") !== "true")!;
  const trackBg = getComputedStyle(q(".view-tabs")).backgroundColor, textRatio = ratio(getComputedStyle(idle).color, trackBg);
  if (textRatio < 4) fail.push(`idle label contrast ${textRatio.toFixed(2)}`);
  // the owner's rule: no underline, bottom border or bar on a tab or in the strip
  for (const el of [strip, ...strip.querySelectorAll("*")]) {
    const cs = getComputedStyle(el), after = getComputedStyle(el, "::after");
    if (cs.textDecorationLine.includes("underline") && !el.closest(".menubar")) fail.push(`${el.className || el.tagName} is underlined`);
    if (el.closest(".view-tabs") && (parseFloat(cs.borderBottomWidth) > 0 || (after.content !== "none" && after.content !== "normal" && parseFloat(after.height) > 0))) fail.push(`${el.className || el.tagName} has a bottom border or bar`);
  }
  // the active tab is a raised pill that differs from the track, inside one rounded track
  const track = q(".view-tabs"), active = tabs().find((t) => t.getAttribute("aria-selected") === "true")!;
  const tcs = getComputedStyle(track), acs = getComputedStyle(active);
  if (acs.backgroundColor === tcs.backgroundColor || acs.backgroundColor === "rgba(0, 0, 0, 0)") fail.push("the active pill does not differ from the track");
  if (acs.boxShadow === "none") fail.push("the active pill is not raised");
  if (parseFloat(tcs.borderTopLeftRadius) < 6 || tcs.backgroundColor === "rgba(0, 0, 0, 0)") fail.push("the tabs are not in one rounded track");
  if (tabs().some((t) => t !== active && t.getAttribute("aria-disabled") !== "true" && getComputedStyle(t).backgroundColor !== "rgba(0, 0, 0, 0)")) fail.push("an idle tab has a fill");
  // a badge takes no width; the label and the chip do not move
  h.four(true); await frames(); const w0 = JSON.stringify(tabs().map(R)); h.badge(true); await frames();
  if (JSON.stringify(tabs().map(R)) !== w0) fail.push("a badge changed the tabs' geometry");
  h.badge(false); h.four(false); await frames();
  // the hover fill never lands on a disabled tab, and a disabled tab does not select
  h.set({ canScenes: false }); await frames();
  const dis = tabs()[2]; dis.dispatchEvent(new MouseEvent("mouseover", { bubbles: true }));
  const duration = getComputedStyle(tabs()[0]).transitionDuration;
  h.set({ canScenes: true }); await frames();
  const at900 = geo.filter((g) => g.w === 900).map((g) => g.labels);
  return { ok: fail.length === 0, fail, compared: geo.length, at900, textRatio, duration, shifts, tabStops, geo };
}
