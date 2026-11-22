// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import type { KeyboardEvent } from "react";

/** Arrow keys move between a chart's marks (elements with data-nav); Home and End go to the ends. */
export function arrowNav(e: KeyboardEvent<Element>) {
  const keys = ["ArrowRight", "ArrowDown", "ArrowLeft", "ArrowUp", "Home", "End"];
  if (!keys.includes(e.key)) return;
  const marks = [...e.currentTarget.querySelectorAll<HTMLElement | SVGElement>("[data-nav]")];
  const i = marks.indexOf(document.activeElement as HTMLElement);
  if (i < 0 || !marks.length) return;
  e.preventDefault();
  const next = e.key === "Home" ? 0 : e.key === "End" ? marks.length - 1
    : (i + (e.key === "ArrowRight" || e.key === "ArrowDown" ? 1 : -1) + marks.length) % marks.length;
  marks[next].focus();
}

/** Enter and Space press an SVG mark, which has no click of its own on the keyboard. */
export function pressKey(e: KeyboardEvent<Element>, press: () => void) {
  if (e.key === "Enter" || e.key === " ") { e.preventDefault(); press(); }
}
