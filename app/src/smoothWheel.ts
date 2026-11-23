// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// A mouse wheel over the photo grid, eased. A notch is a fixed distance applied in one frame, which on a
// grid of 200 pixel cells reads as a jump; WebKitGTK does not always ease it. The grid takes the wheel's
// distance and moves toward it a little each frame. A touchpad already scrolls smoothly and is left to
// the browser, as is a wheel with a modifier (zoom), a scrollbar drag and the keyboard: a scroll position
// that is not the one written last is somebody else's, and ends the ease.

// the ease's time constant: the remaining distance shrinks by 1/e every TAU ms
export const WHEEL_TAU = 80;
// a frame is never eased by more than this, so a stalled frame does not jump the rest of the way
const MAX_DT = 50;
// the browser's own scrolling decides for this long after a touchpad-like event
const PAD_HOLD = 400;

export type WheelLike = { deltaX: number; deltaY: number; deltaMode: number; ctrlKey: boolean; metaKey: boolean;
                          shiftKey: boolean; altKey: boolean };
export type Scroller = { scrollTop: number; clientHeight: number; scrollHeight: number };
export type Host = { now: () => number; raf: (cb: (t: number) => void) => number; cancel: (id: number) => void;
                     reduced: () => boolean; dpr: () => number };

export class SmoothWheel {
  private target = 0;
  private pos = 0;
  private written = NaN;
  private id = 0;
  private last = 0;
  private padUntil = -Infinity;
  private el: Scroller;
  private host: Host;
  constructor(el: Scroller, host: Host) { this.el = el; this.host = host; }

  // true when the wheel event is taken (the caller prevents its default); false leaves it to the browser
  handle(e: WheelLike): boolean {
    if (e.ctrlKey || e.metaKey || e.shiftKey || e.altKey || this.host.reduced()) { this.cancel(); return false; }
    const now = this.host.now();
    if (e.deltaMode === 0) {
      // a touchpad reports small or fractional pixels, often with some sideways; a notch is a whole,
      // larger step straight down. one touchpad event keeps the rest of its gesture (and fling) native
      if (e.deltaX !== 0 || !Number.isInteger(e.deltaY) || Math.abs(e.deltaY) < 4) this.padUntil = now + PAD_HOLD;
      if (now < this.padUntil) { this.cancel(); return false; }
    }
    if (e.deltaY === 0) return false;
    const unit = e.deltaMode === 1 ? 40 : e.deltaMode === 2 ? this.el.clientHeight : 1;
    const max = Math.max(0, this.el.scrollHeight - this.el.clientHeight);
    const at = this.el.scrollTop;
    // an ease still under way carries on from where it is; otherwise from wherever the view really is
    if (!this.id || Math.abs(at - this.written) > 1.5) { this.pos = at; this.target = at; }
    this.target = Math.max(0, Math.min(max, this.target + e.deltaY * unit));
    if (!this.id) { this.last = now; this.id = this.host.raf(this.tick); }
    return true;
  }

  cancel() {
    if (this.id) this.host.cancel(this.id);
    this.id = 0;
  }

  private tick = (t: number) => {
    this.id = 0;
    // moved by someone else since the last frame: stop easing
    if (Math.abs(this.el.scrollTop - this.written) > 1.5 && Number.isFinite(this.written)) { this.written = NaN; return; }
    const dt = Math.min(MAX_DT, Math.max(0, t - this.last));
    this.last = t;
    this.pos += (this.target - this.pos) * (1 - Math.exp(-dt / WHEEL_TAU));
    const done = Math.abs(this.target - this.pos) < 0.5;
    if (done) this.pos = this.target;
    // whole device pixels, as the browser's own scrolling rounds them
    const dpr = this.host.dpr();
    this.el.scrollTop = Math.round(this.pos * dpr) / dpr;
    this.written = this.el.scrollTop;
    if (!done) this.id = this.host.raf(this.tick);
  };
}
