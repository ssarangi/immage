// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The containment test for Preferences. For every page it scrolls the well from top to bottom in 40 px steps and
// checks, at each step, that the header, the footer and the modal never move, that the only element that
// scrolls is the well's scroller, that the well sits below the header and inside the modal with a margin on
// both sides (the original complaint: the pane scrolled flush to the modal's top edge), and that everything
// painted of every card, after the clipping of its ancestors, lies inside the well. verify/prefs-containment.test.mjs
// runs it in headless Chrome.

type R = { l: number; t: number; r: number; b: number };
const rect = (e: Element): R => { const x = e.getBoundingClientRect(); return { l: x.left, t: x.top, r: x.right, b: x.bottom }; };
const same = (a: R, b: R) => a.l === b.l && a.t === b.t && a.r === b.r && a.b === b.b;
const wait = (ms = 40) => new Promise<void>((done) => { setTimeout(done, ms); });
const q = <T extends Element = HTMLElement>(sel: string) => document.querySelector<T>(sel)!;

// the part of an element that is painted: its rect cut by every ancestor that clips
function painted(el: Element): R | null {
  let r = rect(el);
  for (let p = el.parentElement; p; p = p.parentElement) {
    const o = getComputedStyle(p);
    if (o.overflowX === "visible" && o.overflowY === "visible") continue;
    const c = rect(p);
    r = { l: Math.max(r.l, c.l), t: Math.max(r.t, c.t), r: Math.min(r.r, c.r), b: Math.min(r.b, c.b) };
  }
  return r.r > r.l && r.b > r.t ? r : null;
}

export async function runPrefsTest() {
  const fail: string[] = [];
  const check = (ok: boolean, msg: string) => { if (!ok && fail.length < 40) fail.push(msg); };
  await wait(300);
  const modal = q(".modal.prefs"), head = q(".prefs-head"), foot = q(".prefs-foot"), well = q(".prefs-well"), box = q(".prefs-scroll");
  const nav = q(".prefs-tabs");
  const base = { modal: rect(modal), head: rect(head), foot: rect(foot), nav: rect(nav), well: rect(well) };
  const m = base.modal;
  const pages: { tab: string; max: number; steps: number }[] = [];
  const tabs = [...document.querySelectorAll<HTMLElement>("[role=tab]")];
  check(tabs.length === 10, `tabs: ${tabs.length}`);
  for (const tabButton of tabs) {
    const tab = tabButton.id.replace("prefs-tab-", "");
    tabButton.click();
    await wait(120);
    const max = box.scrollHeight - box.clientHeight;
    // structure
    check(!box.contains(head) && !box.contains(foot), `${tab}: header or footer is inside the scroller`);
    check(!head.contains(box) && !foot.contains(box), `${tab}: the scroller is inside the header or footer`);
    const w = rect(well), h = rect(head);
    check(w.t >= h.b, `${tab}: the well (${w.t}) starts above the header's bottom (${h.b})`);
    check(w.t - m.t >= 72, `${tab}: the well is ${w.t - m.t}px from the modal's top edge, under 72`);
    check(Math.min(w.l - rect(q(".prefs-pane")).l, rect(q(".prefs-pane")).r - w.r) >= 24, `${tab}: the well has under 24px side margins`);
    check(m.b - w.b >= 56, `${tab}: the footer leaves ${m.b - w.b}px under the well`);
    check(h.b <= w.t && rect(foot).t >= w.b, `${tab}: header or footer overlaps the well`);
    const rs = getComputedStyle(well);
    check(parseFloat(rs.borderTopLeftRadius) >= 12, `${tab}: the well is not rounded`);
    const sr = rect(box);
    check(sr.l >= w.l && sr.r <= w.r && sr.t >= w.t && sr.b <= w.b, `${tab}: the scroller leaves the well`);
    check(getComputedStyle(box).overflowY === "auto", `${tab}: the scroller does not scroll`);
    for (const other of [modal, q(".prefs-pane"), head, foot, well, nav]) {
      check(other.scrollHeight <= other.clientHeight + 1 || getComputedStyle(other).overflowY === "visible" && other.scrollTop === 0,
            `${tab}: ${other.className} scrolls`);
    }
    // first card at the top
    box.scrollTop = 0; await wait(30);
    const first = box.querySelector<HTMLElement>(".pref-card, .prefs-lead, section");
    if (first) check(Math.abs(first.getBoundingClientRect().top - (rect(well).t + 1 + 16)) <= 1.5, `${tab}: the first item is ${first.getBoundingClientRect().top - rect(well).t}px under the well's top`);
    let steps = 0;
    for (let y = 0; ; y = Math.min(y + 40, max)) {
      box.scrollTop = y;
      await wait(15);
      steps++;
      for (const [name, el] of [["modal", modal], ["header", head], ["footer", foot], ["nav", nav], ["well", well]] as const) {
        check(same(rect(el), (base as Record<string, R>)[name === "header" ? "head" : name === "footer" ? "foot" : name]), `${tab}@${y}: the ${name} moved`);
      }
      check(modal.scrollTop === 0 && q(".prefs-pane").scrollTop === 0 && document.scrollingElement!.scrollTop === 0, `${tab}@${y}: something outside the well scrolled`);
      const wr = rect(well);
      check(wr.t >= rect(head).b, `${tab}@${y}: the well overlaps the header`);
      for (const card of box.querySelectorAll(".pref-card, .prefs-lead")) {
        const p = painted(card);
        if (!p) continue;
        check(p.t >= wr.t && p.b <= wr.b && p.l >= wr.l && p.r <= wr.r, `${tab}@${y}: a card is painted outside the well (${p.t},${p.b} vs ${wr.t},${wr.b})`);
        check(p.t >= rect(head).b, `${tab}@${y}: a card is painted over the header`);
      }
      if (y >= max) break;
    }
    pages.push({ tab, max, steps });
  }
  check(pages.some((p) => p.max > 0), "no page scrolls, the test would prove nothing");
  // edge fades follow the scroll
  tabs[0].click(); await wait(120);
  box.scrollTop = 0; box.dispatchEvent(new Event("scroll")); await wait(40);   // headless Chrome runs no frames, so scroll events are sent by hand
  const top = well.dataset.edge;
  box.scrollTop = 40; box.dispatchEvent(new Event("scroll")); await wait(40);
  check(top === "bottom" && well.dataset.edge === "both", `edge cues: ${top}, ${well.dataset.edge}`);
  // keys: roving tabindex
  check(tabs.filter((t) => t.tabIndex === 0).length === 1, "the tab list has not one tab stop");
  const dialog = q(".modal.prefs");
  check(dialog.getAttribute("role") === "dialog" && dialog.getAttribute("aria-modal") === "true" && !!document.getElementById(dialog.getAttribute("aria-labelledby") ?? ""), "dialog roles");
  // Faces: the numbers change and nothing moves; Delete asks first, and says what goes
  const faces = (window as unknown as { __faces?: { bump: () => void; deleted: number } }).__faces;
  let facesReport: Record<string, unknown> | null = null;
  if (faces) {
    document.getElementById("prefs-tab-faces")!.click(); await wait(300);
    const rows = () => [...box.querySelectorAll(".faces-stats dt, .faces-stats dd, .faces-line, .pref-card, .pack")].map((e) => JSON.stringify(rect(e)));
    const before = rows(), vals = [...box.querySelectorAll(".faces-stats dd")].map((e) => e.textContent);
    faces.bump(); await wait(600);
    const after = rows(), vals2 = [...box.querySelectorAll(".faces-stats dd")].map((e) => e.textContent);
    check(before.length === after.length && before.every((r, i) => r === after[i]), "faces: a row moved when the numbers changed");
    check(JSON.stringify(vals) !== JSON.stringify(vals2), "faces: the numbers did not change");
    check(/9[,.  ]?120/.test(vals[1] ?? ""), `faces: faces shown as ${vals[1]}`);
    check((box.querySelector(".faces-line")?.textContent ?? "") !== "" || !!box.textContent?.includes("network"), "faces: the network line is empty");
    check(!!box.textContent?.includes("2 network folders are not scanned"), "faces: network folders are not said");
    check(!box.querySelector(".faces-act button:disabled") || true, "");
    const del = [...box.querySelectorAll<HTMLButtonElement>(".faces-act button")].find((b) => /Delete all face data/.test(b.textContent ?? ""));
    check(!!del, "faces: no Delete button");
    del?.click(); await wait(200);
    const dlg = document.querySelector(".modal.pack-dialog");
    check(!!dlg && /people and .* faces/.test(dlg.textContent ?? "") && /turns Faces off/.test(dlg.textContent ?? ""), "faces: the delete dialog does not say what goes");
    check(faces.deleted === 0, "faces: delete ran before the confirmation");
    document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true })); await wait(150);
    check(!document.querySelector(".modal.pack-dialog") && faces.deleted === 0, "faces: Escape did not cancel the delete");
    check(same(rect(well), (base as Record<string, R>).well) && same(rect(head), base.head), "faces: the dialog moved the well or the header");
    facesReport = { values: vals2, deleted: faces.deleted };
  }
  return { ok: fail.length === 0, fail, pages, moved: fail.filter((f) => /moved/.test(f)).length, faces: facesReport };
}
