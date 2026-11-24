// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The face-box overlay on a photo that is zoomed, and the Library column's People card, on fixtures: boxes sit where the
// fractions say, at every zoom; hover, focus, selection and the name box move nothing; naming and "Not a face" reach the
// backend and the boxes follow; the card lists the same faces and a chip selects its box. verify/people-shift.test.mjs runs
// it (?view=overlay&shift=1) and asserts `failures` is empty.
import { orderFaces } from "../src/faceOverlayModel";
import type { PhotoFace } from "../src/peopleStore";

type R = { x: number; y: number; w: number; h: number };
type H = { calls: string[]; setZoom: (z: number) => void; setShow: (on: boolean) => void; fake: { photoFaces: (p: string) => Promise<PhotoFace[]> }; photo: string };
const h = () => (window as unknown as { __h: H }).__h;
const frames = (n = 3) => new Promise<void>((done) => { setTimeout(done, 25 * n); });
const rect = (r: DOMRect): R => ({ x: r.left, y: r.top, w: r.width, h: r.height });
const $ = <T extends HTMLElement>(s: string) => document.querySelector<T>(s);
const $$ = <T extends HTMLElement>(s: string) => Array.from(document.querySelectorAll<T>(s));

function snap(root: Element): Map<Element, R> {
  const out = new Map<Element, R>();
  for (const el of [root, ...root.querySelectorAll("*")]) out.set(el, rect(el.getBoundingClientRect()));
  return out;
}
function moved(a: Map<Element, R>, b: Map<Element, R>, ignore: string): string[] {
  const bad: string[] = [];
  for (const [el, r] of a) {
    const s = b.get(el);
    if (!s || el.closest(ignore)) continue;
    if (Math.max(Math.abs(r.x - s.x), Math.abs(r.y - s.y), Math.abs(r.w - s.w), Math.abs(r.h - s.h)) > 0.01) bad.push(`${el.tagName.toLowerCase()}.${String(el.className)}`);
  }
  return bad.slice(0, 6);
}
const typeInto = (input: HTMLInputElement, value: string) => {
  Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
};

export async function runOverlayTest() {
  const out: Record<string, unknown> = { failures: [] as string[] };
  const fail = (m: string) => (out.failures as string[]).push(m);
  await frames(10);
  const expected = orderFaces(await h().fake.photoFaces(h().photo));
  const layer = () => $(".face-layer")!.getBoundingClientRect();
  const boxes = () => $$<HTMLButtonElement>(".face-box");
  const checkGeometry = (where: string) => {
    const l = layer(), bs = boxes();
    if (bs.length !== expected.length) { fail(`${where}: ${bs.length} boxes for ${expected.length} faces`); return; }
    bs.forEach((b, i) => {
      const r = b.getBoundingClientRect(), f = expected[i];
      const want = { x: l.left + f.x * l.width, y: l.top + f.y * l.height, w: f.w * l.width, h: f.h * l.height };
      if (Math.max(Math.abs(r.left - want.x), Math.abs(r.top - want.y), Math.abs(r.width - want.w), Math.abs(r.height - want.h)) > 0.8) fail(`${where}: box ${i} is not where its fractions put it`);
    });
  };
  checkGeometry("zoom 1");
  out.boxes = boxes().length;
  // zoomed in the layer is twice the size and the boxes follow it
  h().setZoom(2); await frames(4);
  checkGeometry("zoom 2");
  const w2 = layer().width;
  h().setZoom(1); await frames(4);
  if (Math.abs(w2 - 2 * layer().width) > 1) fail("the layer did not follow the zoom");

  // names are accessible and the labels show on hover and focus without moving anything
  const names = boxes().map((b) => b.getAttribute("aria-label"));
  out.names = names;
  if (names.some((n) => !/^Face \d+ of \d+, /.test(n ?? ""))) fail(`face names: ${names.join(" | ")}`);
  const stage = $(".ov-stage")!;
  const base = snap(stage);
  const first = boxes()[0];
  first.focus(); await frames(3);
  if (Number(getComputedStyle(first.querySelector(".face-label")!).opacity) < 0.9) fail("the label does not show on focus");
  const m1 = moved(base, snap(stage), ".face-label");
  if (m1.length) fail(`focus moved ${m1.join(", ")}`);
  first.blur();
  $$<HTMLInputElement>(".faces-chip")[1]?.click(); await frames(3);
  const m2 = moved(base, snap(stage), ".face-label, .faces-chip");
  if (m2.length) fail(`a chip moved ${m2.join(", ")}`);
  if (!boxes()[1].classList.contains("selected")) fail("a chip did not select its box");

  // naming the face nobody has a name for
  const un = boxes().find((b) => /unnamed/i.test(b.getAttribute("aria-label") ?? ""));
  if (!un) { fail("no unnamed face to name"); return out; }
  const before = h().calls.length;
  un.click(); await frames(4);
  const input = $<HTMLInputElement>(".face-pop .people-name input");
  if (!input) { fail("clicking a face did not open the name box"); return out; }
  const m3 = moved(base, snap(stage), ".face-pop, .face-label, .faces-chip");
  if (m3.length) fail(`the name box moved ${m3.join(", ")}`);
  typeInto(input, "Zed"); await frames(2);
  input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
  await frames(12);
  if (!h().calls.slice(before).some((c) => c.startsWith("create:Zed:1"))) fail(`naming did not reach the backend (${h().calls.slice(before).join(",")})`);
  if (!boxes().some((b) => /, Zed$/.test(b.getAttribute("aria-label") ?? ""))) fail("the box did not take the name");
  if (!$$(".faces-chip").some((c) => c.textContent === "Zed")) fail("the card did not take the name");

  // "Not a face" on a suggested one removes its box
  const sug = boxes().find((b) => b.dataset.state === "suggested");
  const n0 = boxes().length;
  if (sug) {
    sug.click(); await frames(4);
    const nf = $$<HTMLButtonElement>(".face-pop .people-btn").find((b) => b.textContent === "Not a face");
    nf?.click(); await frames(12);
    if (!h().calls.some((c) => c.startsWith("notFace:"))) fail("Not a face did not reach the backend");
    if (boxes().length !== n0 - 1) fail(`Not a face left ${boxes().length} of ${n0} boxes`);
  } else fail(`no suggested face (${boxes().map((b) => b.dataset.state + ":" + b.getAttribute("aria-label")).join(" | ")})`);
  // the switch hides the boxes; faces off in the app hides them too (App renders nothing)
  h().setShow(false); await frames(3);
  if ($(".face-layer")) fail("the boxes stay with the switch off");
  out.ok = (out.failures as string[]).length === 0;
  return out;
}
