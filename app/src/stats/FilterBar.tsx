// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { t } from "../i18n";
import type { Chip } from "../statsModel";
import { chipText } from "../statsFormat";
import { nf } from "../statsFormat";
import type { StatsResponse } from "../statsTypes";

/**
 * The 40 px filter row under the tabs: a chip per active filter (what is selected, with its x), Clear all,
 * and how many photos match. It is always there: with nothing selected it says how to start, so choosing
 * something never moves the content. Chips that do not fit fold into a "+N more" popover instead of wrapping.
 */
export function FilterBar({ chips, defs, matches, total, onRemove, onClear, loading }: {
  chips: Chip[]; defs?: StatsResponse["defs"]; matches: { n: number; top: number } | null; total: number | null;
  onRemove(key: string): void; onClear(): void; loading: boolean;
}) {
  const host = useRef<HTMLDivElement>(null);
  const [fit, setFit] = useState<number | null>(null);
  const [more, setMore] = useState(false);
  const sig = chips.map((c) => c.key).join("|");

  // measure with every chip in the row, then hide the ones past the edge (a +N chip takes room)
  useLayoutEffect(() => { setFit(null); }, [sig]);
  useLayoutEffect(() => {
    const el = host.current;
    if (!el || fit !== null) return;
    const kids = [...el.querySelectorAll<HTMLElement>("[data-chip]")];
    const limit = el.clientWidth;
    const room = 84;
    let count = kids.length;
    const lastRight = kids.length ? kids[kids.length - 1].offsetLeft + kids[kids.length - 1].offsetWidth - el.offsetLeft : 0;
    if (lastRight > limit) {
      count = 0;
      kids.forEach((k, i) => { if (k.offsetLeft - el.offsetLeft + k.offsetWidth <= limit - room) count = i + 1; });
    }
    setFit(count);
  });
  useEffect(() => {
    if (!more) return;
    const esc = (e: KeyboardEvent) => { if (e.key === "Escape") { e.stopPropagation(); setMore(false); } };
    const down = (e: PointerEvent) => { if (!(e.target as Element).closest(".st-more-wrap")) setMore(false); };
    window.addEventListener("keydown", esc, true);
    window.addEventListener("pointerdown", down, true);
    return () => { window.removeEventListener("keydown", esc, true); window.removeEventListener("pointerdown", down, true); };
  }, [more]);
  const shown = fit === null ? chips.length : Math.min(fit, chips.length);
  const hidden = chips.slice(shown);

  return (
    <div className="st-fbar" aria-live="polite">
      <span className="st-fbar-lbl">{t("stats.filters.label")}</span>
      <div className="st-chips" ref={host}>
        {chips.length === 0 && <span className="st-fbar-hint">{t("stats.filters.empty")}</span>}
        {chips.map((c, i) => {
          const tx = chipText(c, defs);
          return (
            <span key={c.key} data-chip className={`st-chip ${c.tone}`} hidden={fit !== null && i >= shown}>
              <i>{tx.kind}</i> <span className="st-chip-t">{tx.text}</span>
              <button type="button" aria-label={t("stats.filters.remove", { kind: tx.kind, text: tx.text })} onClick={() => onRemove(c.key)}>{"×"}</button>
            </span>
          );
        })}
        {hidden.length > 0 && (
          <span className="st-more-wrap">
            <button type="button" className="st-chip more" aria-expanded={more} onClick={() => setMore(!more)}>{t("stats.filters.more", { n: hidden.length })}</button>
            {more && (
              <div className="st-pop st-more-pop" role="group">
                {hidden.map((c) => {
                  const tx = chipText(c, defs);
                  return <span key={c.key} className={`st-chip ${c.tone}`}><i>{tx.kind}</i> {tx.text}
                    <button type="button" aria-label={t("stats.filters.remove", { kind: tx.kind, text: tx.text })} onClick={() => onRemove(c.key)}>{"×"}</button></span>;
                })}
              </div>
            )}
          </span>
        )}
      </div>
      {chips.length > 0 && <button type="button" className="st-link" onClick={onClear}>{t("stats.filters.clear")}</button>}
      <span className="st-grow" />
      <span className="st-fbar-count">
        {loading || !matches ? <i className="st-sk st-sk-line count" /> : <>{t("stats.filters.matches", { n: matches.n, count: nf(matches.n), total: total === null ? "–" : nf(total) })} {"·"} {t("stats.filters.top", { n: matches.top, count: nf(matches.top) })}</>}
      </span>
    </div>
  );
}
