// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 the Immage authors

// The name combobox (spec 2.5): type to filter the existing people, with a Create row last when the name is
// new. Up and Down move, Enter accepts the highlighted row, Escape closes with nothing changed, Tab accepts
// and goes on to the next group. It floats over the grid under the tile it is for (flipping above near the
// bottom), so it moves nothing, and its first letter can be the key that opened it.
import { useEffect, useId, useRef, useState } from "react";
import type { PersonRow } from "./bindings";
import { formatNumber, t } from "./i18n";
import { CAPTION, nameOptions, type NameOption } from "./peopleModel";
import type { GridOverlay } from "./PeopleGrid";
import type { NameTarget } from "./peopleStore";

const W = 260, ROW = 30, HEAD = 74;

export function NamePopover({ o, index, count, group, seed, people, onPick, onClose, extra }: {
  o: GridOverlay; index: number; count: number; group: boolean; seed: string; people: PersonRow[];
  onPick: (to: NameTarget, next: boolean) => void; onClose: () => void; extra?: React.ReactNode;
}) {
  const [q, setQ] = useState(seed);
  const [hi, setHi] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const id = useId();
  const options = nameOptions(q, people);
  // what is being named, by what is selected: one group is "this group", anything else is its faces
  const title = t(group ? "people.name.group" : "people.name.title", { n: count, count: formatNumber(count) });
  useEffect(() => { input.current?.focus(); const n = input.current?.value.length ?? 0; input.current?.setSelectionRange(n, n); }, []);
  const at = Math.min(hi, Math.max(0, options.length - 1));
  const accept = (opt: NameOption | undefined, next: boolean) => {
    if (!opt) return;
    onPick(opt.kind === "create" ? { kind: "create", name: opt.name } : { kind: "person", id: opt.id, name: opt.name }, next);
  };
  const p = o.pos(index);
  const height = HEAD + Math.max(1, options.length) * ROW;
  const left = Math.min(Math.max(8, p.x), Math.max(8, o.width - W - 8));
  const below = p.y - o.scrollTop + o.g.tile + CAPTION + 4;
  const flipped = below + height > o.height ? Math.max(4, p.y - o.scrollTop - height - 4) : below;
  const top = Math.min(Math.max(4, flipped), Math.max(4, o.height - height - 4));
  return (
    <div className="people-name" style={{ left, top }} role="dialog" aria-label={title}
         onPointerDown={(e) => e.stopPropagation()}>
      <label htmlFor={id}>{title}</label>
      <input id={id} ref={input} role="combobox" aria-expanded="true" aria-controls={`${id}-list`} aria-autocomplete="list" autoComplete="off" spellCheck={false}
             aria-activedescendant={options.length ? `${id}-${at}` : undefined} value={q} placeholder={t("people.name.placeholder")}
             onChange={(e) => { setQ(e.target.value); setHi(0); }}
             onKeyDown={(e) => {
               e.stopPropagation();
               if (e.key === "ArrowDown") { e.preventDefault(); setHi(Math.min(at + 1, options.length - 1)); }
               else if (e.key === "ArrowUp") { e.preventDefault(); setHi(Math.max(at - 1, 0)); }
               else if (e.key === "Enter") { e.preventDefault(); accept(options[at], false); }
               else if (e.key === "Tab") { e.preventDefault(); accept(options[at], true); }
               else if (e.key === "Escape") { e.preventDefault(); onClose(); }
             }} />
      <ul id={`${id}-list`} role="listbox" aria-label={t("people.name.people")}>
        {options.map((opt, i) => (
          <li key={opt.kind === "create" ? "create" : opt.id} id={`${id}-${i}`} role="option" aria-selected={i === at}
              onPointerMove={() => setHi(i)} onClick={() => accept(opt, false)}>
            {opt.kind === "create" ? <b>{t("people.name.create", { q: opt.name })}</b>
              : <><b>{opt.name}</b><span>{t("people.tile.photos", { n: opt.photos, count: formatNumber(opt.photos) })}</span></>}
          </li>
        ))}
        {!options.length && <li aria-disabled="true"><span>{t("people.name.type")}</span></li>}
      </ul>
      {extra}
    </div>
  );
}
