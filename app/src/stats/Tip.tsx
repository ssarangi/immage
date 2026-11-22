// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// One shared tooltip: any element with data-tip (a bold line) and optionally data-tip-sub shows it,
// under the pointer or beside the focused element. It is position:fixed, so it never takes room.
import { useEffect, useRef, useState } from "react";

interface TipState { title: string; sub: string; x: number; y: number }

export function dataTip(title: string, sub?: string) {
  return sub ? { "data-tip": title, "data-tip-sub": sub } : { "data-tip": title };
}

export function TipLayer({ root }: { root: React.RefObject<HTMLElement | null> }) {
  const [tip, setTip] = useState<TipState | null>(null);
  const box = useRef<HTMLDivElement>(null);
  useEffect(() => {
    const el = root.current;
    if (!el) return;
    const place = (target: Element, x: number, y: number) => {
      const t = target.closest<HTMLElement>("[data-tip]");
      if (!t) { setTip(null); return; }
      setTip({ title: t.dataset.tip ?? "", sub: t.dataset.tipSub ?? "", x, y });
    };
    const move = (e: PointerEvent) => place(e.target as Element, e.clientX + 14, e.clientY + 14);
    const focus = (e: FocusEvent) => {
      const t = e.target as Element;
      if (!t.matches(":focus-visible")) return;
      const r = t.getBoundingClientRect();
      place(t, r.left + r.width / 2, r.bottom + 8);
    };
    const hide = () => setTip(null);
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerleave", hide);
    el.addEventListener("focusin", focus);
    el.addEventListener("focusout", hide);
    el.addEventListener("scroll", hide, true);
    return () => {
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerleave", hide);
      el.removeEventListener("focusin", focus);
      el.removeEventListener("focusout", hide);
      el.removeEventListener("scroll", hide, true);
    };
  }, [root]);
  if (!tip) return null;
  const w = box.current?.offsetWidth ?? 240;
  const left = Math.max(8, Math.min(window.innerWidth - w - 8, tip.x));
  const top = Math.min(window.innerHeight - 70, tip.y);
  return (
    <div className="st-tip" role="tooltip" ref={box} style={{ left, top }}>
      <b>{tip.title}</b>{tip.sub && <span>{tip.sub}</span>}
    </div>
  );
}
